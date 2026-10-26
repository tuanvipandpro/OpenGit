//! Save/Apply authority and evidence. No ordinal is ever passed to native mutation.
//! Messages and file bytes are runtime inputs only; proof contains safe OIDs/digests.
use super::{
    authority::{Target, TargetKind},
    catalog::ReadQuery,
    changes,
    journal::{ObjectFormat, SafeTargetProof},
    preflight,
};
use crate::{
    domain::{contracts::*, stash::*},
    git::{error::error, runner::argv},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const MAX_ENTRIES: usize = 4096;
const MAX_PATHS: usize = 4096;

fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn limit() -> AppError {
    error(ErrorCode::OutputLimit, Phase::Inspection)
}
pub(crate) fn supported(kind: ActionKind) -> bool {
    matches!(kind, ActionKind::StashPush | ActionKind::StashApply)
}
pub(crate) fn oid(value: &[u8], length: usize) -> Result<String, AppError> {
    if !matches!(length, 40 | 64)
        || value.len() != length
        || !value
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return Err(malformed());
    }
    String::from_utf8(value.to_vec()).map_err(|_| malformed())
}
fn charge(h: &mut Sha256, value: &[u8]) {
    h.update((value.len() as u64).to_le_bytes());
    h.update(value);
}
pub(crate) fn stack_digest(oids: &[String]) -> String {
    let mut h = Sha256::new();
    charge(&mut h, b"opengit-stash-stack-v1");
    for value in oids {
        charge(&mut h, value.as_bytes());
    }
    format!("{:x}", h.finalize())
}
pub(crate) fn parse_stack(bytes: &[u8], length: usize) -> Result<Vec<String>, AppError> {
    if bytes.is_empty() {
        return Ok(vec![]);
    }
    if !bytes.ends_with(b"\n") || bytes.len() > MAX_ENTRIES * (length + 1) {
        return Err(limit());
    }
    let mut values = Vec::new();
    for line in bytes[..bytes.len() - 1].split(|b| *b == b'\n') {
        values.push(oid(line, length)?);
    }
    if values.len() > MAX_ENTRIES {
        return Err(limit());
    }
    Ok(values)
}
pub(crate) struct Stack {
    pub length: usize,
    pub oids: Vec<String>,
    pub digest: String,
}
pub(crate) async fn stack(q: &ReadQuery<'_>) -> Result<Stack, AppError> {
    let length = match changes::run(q, &argv(&["rev-parse", "--show-object-format"]))
        .await?
        .as_slice()
    {
        b"sha1\n" => 40,
        b"sha256\n" => 64,
        _ => return Err(malformed()),
    };
    let symbolic = changes::output(q, &argv(&["symbolic-ref", "--quiet", "refs/stash"])).await?;
    if symbolic.success || symbolic.exit_code != Some(1) || !symbolic.stderr.is_empty() {
        return Err(changes::unsupported());
    }
    let tip = changes::output(
        q,
        &argv(&["rev-parse", "--verify", "--quiet", "refs/stash"]),
    )
    .await?;
    if !tip.stderr.is_empty() {
        return Err(malformed());
    }
    if !tip.success {
        if tip.exit_code != Some(1) || !tip.stdout.is_empty() {
            return Err(malformed());
        }
        return Ok(Stack {
            length,
            oids: vec![],
            digest: stack_digest(&[]),
        });
    }
    let tip = oid(
        tip.stdout.strip_suffix(b"\n").ok_or_else(malformed)?,
        length,
    )?;
    let raw = changes::run(q, &argv(&["reflog", "show", "--format=%H", "refs/stash"])).await?;
    let oids = parse_stack(&raw, length)?;
    if oids.first() != Some(&tip) {
        return Err(changes::changed());
    }
    Ok(Stack {
        length,
        digest: stack_digest(&oids),
        oids,
    })
}

/// Parse physical object headers, never traversal parents hidden by shallow state.
pub(crate) struct CommitObject {
    pub tree: String,
    pub parents: Vec<String>,
}
pub(crate) fn parse_commit(raw: &[u8], length: usize) -> Result<CommitObject, AppError> {
    if raw.len() > 1024 * 1024 {
        return Err(limit());
    }
    let end = raw
        .windows(2)
        .position(|b| b == b"\n\n")
        .ok_or_else(malformed)?;
    let mut lines = raw[..end].split(|b| *b == b'\n');
    let tree = oid(
        lines
            .next()
            .and_then(|v| v.strip_prefix(b"tree "))
            .ok_or_else(malformed)?,
        length,
    )?;
    let mut parents = Vec::new();
    let mut after_parents = false;
    for line in lines {
        if let Some(parent) = line.strip_prefix(b"parent ") {
            if after_parents || parents.len() >= 16 {
                return Err(malformed());
            }
            parents.push(oid(parent, length)?);
        } else {
            after_parents = true;
            if line.starts_with(b"tree ") || line.is_empty() || line.contains(&0) {
                return Err(malformed());
            }
        }
    }
    Ok(CommitObject { tree, parents })
}
async fn commit(q: &ReadQuery<'_>, value: &str, length: usize) -> Result<CommitObject, AppError> {
    oid(value.as_bytes(), length)?;
    parse_commit(
        &changes::run(q, &argv(&["cat-file", "commit", value])).await?,
        length,
    )
}
pub(crate) struct StashObject {
    pub oid: String,
    pub tree: String,
    pub base: String,
    pub index_tree: String,
    pub untracked_tree: Option<String>,
}
pub(crate) async fn inspect(
    q: &ReadQuery<'_>,
    value: &str,
    length: usize,
) -> Result<StashObject, AppError> {
    let work = commit(q, value, length).await?;
    if !matches!(work.parents.len(), 2 | 3) {
        return Err(changes::unsupported());
    }
    let index = commit(q, &work.parents[1], length).await?;
    if index.parents.as_slice() != &work.parents[..1] {
        return Err(changes::unsupported());
    }
    let untracked_tree = if let Some(value) = work.parents.get(2) {
        let untracked = commit(q, value, length).await?;
        if !untracked.parents.is_empty() {
            return Err(changes::unsupported());
        }
        Some(untracked.tree)
    } else {
        None
    };
    Ok(StashObject {
        oid: value.into(),
        tree: work.tree,
        base: work.parents[0].clone(),
        index_tree: index.tree,
        untracked_tree,
    })
}
pub(crate) fn entry_id(stack: &Stack, ordinal: usize) -> String {
    // Include ordinal as well as the complete ordered digest: duplicate OIDs are distinct.
    preflight::digest(format!("stash-entry-v1:{}:{ordinal}", stack.digest).as_bytes())
}
pub(crate) async fn entries(
    q: &ReadQuery<'_>,
    captured: &Stack,
) -> Result<Vec<StashEntry>, AppError> {
    let mut values = Vec::with_capacity(captured.oids.len());
    for (ordinal, value) in captured.oids.iter().enumerate() {
        let object = inspect(q, value, captured.length).await;
        let (base, untracked, available) = match object {
            Ok(object) => (Some(object.base), object.untracked_tree.is_some(), true),
            Err(e) if e.code == ErrorCode::UnsupportedAction => (None, false, false),
            Err(e) => return Err(e),
        };
        values.push(StashEntry {
            entry_id: entry_id(captured, ordinal),
            ordinal: ordinal.to_string(),
            oid: value.clone(),
            base_oid: base,
            includes_untracked: untracked,
            apply_available: available,
        });
    }
    Ok(values)
}
pub(crate) fn target(captured: &Stack, entry: &str) -> Result<Target, AppError> {
    let ordinal = (0..captured.oids.len())
        .find(|i| entry_id(captured, *i) == entry)
        .ok_or_else(changes::invalid)?;
    let value = &captured.oids[ordinal];
    let mut identity = b"SS1".to_vec();
    identity.extend_from_slice(&(ordinal as u32).to_le_bytes());
    identity.push(captured.length as u8);
    identity.extend_from_slice(value.as_bytes());
    identity.extend_from_slice(captured.digest.as_bytes());
    Ok(Target {
        kind: TargetKind::Stash,
        identity,
        proof: Some(SafeTargetProof::Stash {
            oid: value.clone(),
            reflog_digest: captured.digest.clone(),
            object_format: if captured.length == 40 {
                ObjectFormat::Sha1
            } else {
                ObjectFormat::Sha256
            },
        }),
    })
}
pub(crate) fn decode(target: &Target) -> Result<(usize, &str, &str), AppError> {
    let Some(SafeTargetProof::Stash {
        oid: value,
        reflog_digest,
        object_format,
    }) = &target.proof
    else {
        return Err(changes::invalid());
    };
    target
        .proof
        .as_ref()
        .ok_or_else(changes::invalid)?
        .validate()?;
    let length = match object_format {
        ObjectFormat::Sha1 => 40,
        ObjectFormat::Sha256 => 64,
    };
    let b = &target.identity;
    if target.kind != TargetKind::Stash
        || b.len() != 8 + length + 64
        || &b[..3] != b"SS1"
        || b[7] as usize != length
        || &b[8..8 + length] != value.as_bytes()
        || &b[8 + length..] != reflog_digest.as_bytes()
    {
        return Err(changes::invalid());
    }
    let ordinal = u32::from_le_bytes(b[3..7].try_into().map_err(|_| changes::invalid())?) as usize;
    if ordinal >= MAX_ENTRIES {
        return Err(changes::invalid());
    }
    Ok((ordinal, value, reflog_digest))
}
pub(crate) async fn validate_target(
    q: &ReadQuery<'_>,
    target: &Target,
) -> Result<StashObject, AppError> {
    let (ordinal, value, digest) = decode(target)?;
    let now = stack(q).await?;
    if now.digest != digest || now.oids.get(ordinal).map(String::as_str) != Some(value) {
        return Err(changes::changed());
    }
    inspect(q, value, now.length).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TreeEntry {
    pub mode: String,
    pub oid: String,
}
pub(crate) type Tree = BTreeMap<Vec<u8>, TreeEntry>;
pub(crate) fn parse_tree(raw: &[u8], length: usize) -> Result<Tree, AppError> {
    if !raw.is_empty() && !raw.ends_with(&[0]) {
        return Err(malformed());
    }
    let mut tree = Tree::new();
    for entry in raw.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        if tree.len() >= MAX_PATHS {
            return Err(limit());
        }
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(malformed)?;
        let header = entry[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
        if header.len() != 3 || header[1] != b"blob" || !matches!(header[0], b"100644" | b"100755")
        {
            return Err(changes::unsupported());
        }
        let path = &entry[tab + 1..];
        valid_path(path)?;
        if tree
            .insert(
                path.to_vec(),
                TreeEntry {
                    mode: String::from_utf8(header[0].to_vec()).map_err(|_| malformed())?,
                    oid: oid(header[2], length)?,
                },
            )
            .is_some()
        {
            return Err(malformed());
        }
    }
    Ok(tree)
}
pub(crate) async fn tree(q: &ReadQuery<'_>, value: &str, length: usize) -> Result<Tree, AppError> {
    oid(value.as_bytes(), length)?;
    parse_tree(
        &changes::run(q, &argv(&["ls-tree", "-r", "-z", "--full-tree", value])).await?,
        length,
    )
}
pub(crate) fn tree_digest(tree: &Tree) -> String {
    let mut h = Sha256::new();
    charge(&mut h, b"opengit-stash-tree-v1");
    for (path, entry) in tree {
        charge(&mut h, path);
        charge(&mut h, entry.mode.as_bytes());
        charge(&mut h, entry.oid.as_bytes());
    }
    format!("{:x}", h.finalize())
}
fn valid_path(raw: &[u8]) -> Result<&str, AppError> {
    let text = std::str::from_utf8(raw).map_err(|_| changes::unsupported())?;
    if text.is_empty()
        || text.len() > 768
        || text.contains(['\\', ':', '\0'])
        || text.chars().any(char::is_control)
        || text.split('/').any(|s| {
            s.is_empty()
                || s == "."
                || s == ".."
                || s.eq_ignore_ascii_case(".git")
                || s.ends_with(['.', ' '])
        })
    {
        return Err(changes::unsupported());
    }
    Ok(text)
}
fn checked_path(
    location: &crate::repositories::registry::RepositoryLocation,
    raw: &[u8],
) -> Result<std::path::PathBuf, AppError> {
    let text = valid_path(raw)?;
    let mut parent = location.root.clone();
    let parts = text.split('/').collect::<Vec<_>>();
    for part in &parts[..parts.len() - 1] {
        parent.push(part);
        match std::fs::symlink_metadata(&parent) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(changes::changed()),
            Ok(m) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(changes::unsupported());
                    }
                }
                if !m.is_dir() || m.file_type().is_symlink() {
                    return Err(collision());
                }
            }
        }
    }
    Ok(location.root.join(text))
}
pub(crate) fn parse_paths(bytes: &[u8]) -> Result<Vec<Vec<u8>>, AppError> {
    if bytes.len() > 4 * 1024 * 1024 || !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(limit());
    }
    let mut result = Vec::new();
    for path in bytes.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        valid_path(path)?;
        if result.len() >= MAX_PATHS {
            return Err(limit());
        }
        result.push(path.to_vec());
    }
    result.sort();
    if result.windows(2).any(|p| p[0] == p[1]) {
        return Err(malformed());
    }
    Ok(result)
}
#[cfg(windows)]
fn same_component(a: &str, b: &str) -> bool {
    let a = a.encode_utf16().collect::<Vec<_>>();
    let b = b.encode_utf16().collect::<Vec<_>>();
    unsafe {
        windows_sys::Win32::Globalization::CompareStringOrdinal(
            a.as_ptr(),
            a.len() as i32,
            b.as_ptr(),
            b.len() as i32,
            1,
        ) == 2
    }
}
#[cfg(not(windows))]
fn same_component(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
/// Prefix comparison is component-wise: file/dir and case aliases both collide.
pub(crate) fn paths_collide(a: &[u8], b: &[u8]) -> Result<bool, AppError> {
    let a = valid_path(a)?;
    let b = valid_path(b)?;
    for (a, b) in a.split('/').zip(b.split('/')) {
        if !same_component(a, b) {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(crate) async fn check_collisions(
    q: &ReadQuery<'_>,
    object: &StashObject,
    length: usize,
) -> Result<(), AppError> {
    let raw = changes::run(
        q,
        &argv(&[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            &object.base,
            &object.oid,
            "--",
        ]),
    )
    .await?;
    let mut touched = parse_paths(&raw)?;
    if let Some(untracked) = &object.untracked_tree {
        touched.extend(tree(q, untracked, length).await?.into_keys());
    }
    if touched.len() > MAX_PATHS {
        return Err(limit());
    }
    let mut others = parse_paths(
        &changes::run(
            q,
            &argv(&["ls-files", "--others", "--exclude-standard", "-z"]),
        )
        .await?,
    )?;
    others.extend(parse_paths(
        &changes::run(
            q,
            &argv(&[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
            ]),
        )
        .await?,
    )?);
    if others.len() > MAX_PATHS {
        return Err(limit());
    }
    for target in &touched {
        // Existing directories (including empty ignored directories) cannot be
        // silently replaced by a file. Parent pins reject reparse traversal.
        let path = checked_path(q.location, target)?;
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
            return Err(collision());
        }
        for other in &others {
            if paths_collide(target, other)? {
                return Err(collision());
            }
        }
    }
    Ok(())
}
fn collision() -> AppError {
    let mut e = error(ErrorCode::DestinationExists, Phase::Validation);
    e.message = "Stash Apply would collide with existing untracked or ignored data. Move that data before preparing Apply.".into();
    e
}
fn save_collision() -> AppError {
    let mut e = error(ErrorCode::DestinationExists, Phase::Validation);
    e.message = "Stash Save could overwrite untracked or ignored data at a path removed from the index. Move or add that data before saving.".into();
    e
}

pub(crate) fn parse_index(raw: &[u8], length: usize) -> Result<Tree, AppError> {
    if !raw.is_empty() && !raw.ends_with(&[0]) {
        return Err(malformed());
    }
    let mut result = Tree::new();
    for entry in raw.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        if result.len() >= MAX_PATHS {
            return Err(limit());
        }
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(malformed)?;
        let f = entry[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
        if f.len() != 3 || f[2] != b"0" {
            return Err(changes::unsupported());
        }
        if !matches!(f[0], b"100644" | b"100755") {
            let mut e = changes::unsupported();
            e.message = "Stash Save and Apply currently require regular tracked files; this repository contains a symlink, submodule, or another unsupported index entry. Use Git directly for this repository.".into();
            return Err(e);
        }
        let path = &entry[tab + 1..];
        valid_path(path)?;
        let value = oid(f[1], length)?;
        if value.bytes().all(|b| b == b'0') {
            return Err(changes::unsupported());
        }
        if result
            .insert(
                path.to_vec(),
                TreeEntry {
                    mode: String::from_utf8(f[0].to_vec()).map_err(|_| malformed())?,
                    oid: value,
                },
            )
            .is_some()
        {
            return Err(malformed());
        }
    }
    Ok(result)
}
fn work_modes(
    raw: &[u8],
    index: &Tree,
    length: usize,
) -> Result<BTreeMap<Vec<u8>, String>, AppError> {
    if !raw.is_empty() && !raw.ends_with(&[0]) {
        return Err(malformed());
    }
    let mut fields = raw.split(|b| *b == 0);
    let mut result = BTreeMap::new();
    while let Some(header) = fields.next().filter(|b| !b.is_empty()) {
        let f = header
            .strip_prefix(b":")
            .ok_or_else(malformed)?
            .split(|b| *b == b' ')
            .collect::<Vec<_>>();
        if f.len() != 5
            || !matches!(f[1], b"000000" | b"100644" | b"100755")
            || !matches!(f[4], b"M" | b"D")
        {
            return Err(changes::unsupported());
        }
        let path = fields.next().ok_or_else(malformed)?;
        valid_path(path)?;
        let old = index.get(path).ok_or_else(changes::changed)?;
        if old.mode.as_bytes() != f[0] || old.oid != oid(f[2], length)? {
            return Err(changes::changed());
        }
        if result
            .insert(
                path.to_vec(),
                String::from_utf8(f[1].to_vec()).map_err(|_| malformed())?,
            )
            .is_some()
        {
            return Err(malformed());
        }
    }
    Ok(result)
}
async fn clean_hash(
    q: &ReadQuery<'_>,
    path: &[u8],
    bytes: &[u8],
    length: usize,
) -> Result<String, AppError> {
    let path = valid_path(path)?;
    let mut all = preflight::global_args(q.location)?;
    all.extend(argv(&[
        "-c",
        "advice.graftFileDeprecated=false",
        "hash-object",
        &format!("--path={path}"),
        "--stdin",
    ]));
    let mut runner = q.runner.clone();
    runner.timeout = runner.timeout.min(
        q.deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| error(ErrorCode::Timeout, Phase::Inspection))?,
    );
    let output = runner
        .output_history(
            Some(&q.location.root),
            &all,
            &q.cancel,
            Phase::Inspection,
            crate::git::runner::RunOptions {
                lifetime_guard: Some(q.lifetime.clone()),
                environment: vec![
                    ("GIT_ALLOW_PROTOCOL".into(), "".into()),
                    ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                    ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
                ],
                ..Default::default()
            },
            Some(bytes),
        )
        .await?;
    if !output.success {
        return Err(crate::git::error::git_failure(
            &output.stderr,
            Phase::Inspection,
        ));
    }
    if !output.stderr.is_empty() {
        return Err(malformed());
    }
    oid(
        output.stdout.strip_suffix(b"\n").ok_or_else(malformed)?,
        length,
    )
}
fn read_working(
    q: &ReadQuery<'_>,
    raw: &[u8],
    left: &mut usize,
) -> Result<Option<Vec<u8>>, AppError> {
    use std::io::Read;
    let path = checked_path(q.location, raw)?;
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(changes::changed()),
        Ok(m) if !m.is_file() || m.len() > *left as u64 => return Err(changes::unsupported()),
        Ok(_) => {}
    }
    let mut opened = changes::open_regular(&path)?;
    let mut bytes = Vec::new();
    opened
        .file
        .by_ref()
        .take(*left as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| changes::changed())?;
    if bytes.len() > *left {
        return Err(limit());
    }
    *left -= bytes.len();
    Ok(Some(bytes))
}
pub(crate) fn path_stamp(
    location: &crate::repositories::registry::RepositoryLocation,
    raw: &[u8],
) -> Result<String, AppError> {
    let path = checked_path(location, raw)?;
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(preflight::digest(
            &[b"stash-absent:".as_slice(), raw].concat(),
        )),
        Err(_) => Err(changes::changed()),
        Ok(_) => changes::file_stamp(location, raw),
    }
}
pub(crate) struct SaveSnapshot {
    pub head: String,
    pub stack: Stack,
    pub index: Tree,
    pub working: Tree,
    pub untracked: Tree,
    pub stamps: BTreeMap<Vec<u8>, String>,
}
impl SaveSnapshot {
    pub(crate) fn validate_files(
        &self,
        location: &crate::repositories::registry::RepositoryLocation,
    ) -> Result<(), AppError> {
        for (path, stamp) in &self.stamps {
            if path_stamp(location, path)? != *stamp {
                return Err(changes::changed());
            }
        }
        Ok(())
    }
    pub(crate) fn working_digest(&self) -> String {
        let mut h = Sha256::new();
        for (path, stamp) in &self.stamps {
            charge(&mut h, path);
            charge(&mut h, stamp.as_bytes());
        }
        format!("{:x}", h.finalize())
    }
}
pub(crate) async fn save_snapshot(
    q: &ReadQuery<'_>,
    include_untracked: bool,
) -> Result<SaveSnapshot, AppError> {
    changes::index_supported(q).await?;
    let stack = stack(q).await?;
    let head = changes::output(q, &argv(&["rev-parse", "--verify", "HEAD^{commit}"])).await?;
    if !head.success {
        return Err(changes::unsupported());
    }
    let head = oid(
        head.stdout.strip_suffix(b"\n").ok_or_else(malformed)?,
        stack.length,
    )?;
    let flags = changes::run(q, &argv(&["ls-files", "-v", "-z"])).await?;
    if flags
        .split(|b| *b == 0)
        .any(|e| e.first().is_some_and(u8::is_ascii_lowercase))
    {
        return Err(changes::unsupported());
    }
    let index = parse_index(
        &changes::run(q, &argv(&["ls-files", "--stage", "-z"])).await?,
        stack.length,
    )?;
    // Git's stash push restores HEAD paths removed from the index. A file
    // recreated at one of those paths is untracked even though its pathname
    // was tracked in HEAD; Git may overwrite it during its own cleanup.
    // Guard the physical path, including Windows case aliases and file/dir
    // prefix collisions, whether or not -u was requested.
    let head_tree = Box::pin(tree(q, &head, stack.length)).await?;
    let mut removed_head_paths = Vec::new();
    for path in head_tree.keys().filter(|path| !index.contains_key(*path)) {
        let physical = checked_path(q.location, path)?;
        match std::fs::symlink_metadata(&physical) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(changes::changed()),
            Ok(_) => return Err(save_collision()),
        }
        removed_head_paths.push(path.clone());
    }
    let modes = work_modes(
        &changes::run(
            q,
            &argv(&[
                "diff-files",
                "--raw",
                "--no-abbrev",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "-z",
                "--ignore-submodules=none",
                "--",
            ]),
        )
        .await?,
        &index,
        stack.length,
    )?;
    let mut snapshot = SaveSnapshot {
        head,
        stack,
        index,
        working: Tree::new(),
        untracked: Tree::new(),
        stamps: BTreeMap::new(),
    };
    for path in removed_head_paths {
        snapshot
            .stamps
            .insert(path.clone(), path_stamp(q.location, &path)?);
    }
    let mut left = 32 * 1024 * 1024;
    for (path, entry) in &snapshot.index {
        let stamp = path_stamp(q.location, path)?;
        let mode = modes.get(path).unwrap_or(&entry.mode);
        if let Some(bytes) = read_working(q, path, &mut left)? {
            if mode == "000000" {
                return Err(changes::changed());
            }
            let hash = clean_hash(q, path, &bytes, snapshot.stack.length).await?;
            snapshot.working.insert(
                path.clone(),
                TreeEntry {
                    mode: mode.clone(),
                    oid: hash,
                },
            );
        } else if mode != "000000" {
            return Err(changes::changed());
        }
        snapshot.stamps.insert(path.clone(), stamp);
    }
    if include_untracked {
        // On Windows native core.filemode=false is the proven new-file mode.
        // Other mode policies are explicit unsupported rather than guessed.
        let mode = changes::output(
            q,
            &argv(&["config", "--type=bool", "--get", "core.filemode"]),
        )
        .await?;
        if !mode.success || mode.stdout != b"false\n" || !mode.stderr.is_empty() {
            return Err(changes::unsupported());
        }
        for path in parse_paths(
            &changes::run(
                q,
                &argv(&["ls-files", "--others", "--exclude-standard", "-z"]),
            )
            .await?,
        )? {
            let stamp = path_stamp(q.location, &path)?;
            let bytes = read_working(q, &path, &mut left)?.ok_or_else(changes::changed)?;
            let hash = clean_hash(q, &path, &bytes, snapshot.stack.length).await?;
            snapshot.untracked.insert(
                path.clone(),
                TreeEntry {
                    mode: "100644".into(),
                    oid: hash,
                },
            );
            snapshot.stamps.insert(path, stamp);
        }
    }
    snapshot.validate_files(q.location)?;
    Ok(snapshot)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StashPhase {
    Reserved,
    PinCreated,
    PushStarted,
    ApplyStarted,
    Observed,
    Conflict,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StashProof {
    pub phase: StashPhase,
    pub before_head: String,
    pub before_stack: String,
    pub before_index: String,
    pub before_working: String,
    pub expected_index_tree: String,
    pub expected_working_tree: String,
    pub expected_untracked_tree: Option<String>,
    pub recovery_ref: Option<String>,
    pub pinned_oid: Option<String>,
    pub effects: ActionEffects,
    pub result: StashActionResult,
}
impl StashProof {
    pub(crate) fn validate(&self, kind: ActionKind) -> Result<(), AppError> {
        if !supported(kind) {
            return Err(changes::invalid());
        }
        let length = self.before_head.len();
        oid(self.before_head.as_bytes(), length)?;
        for value in [
            &self.before_stack,
            &self.before_index,
            &self.before_working,
            &self.expected_index_tree,
            &self.expected_working_tree,
        ]
        .into_iter()
        .chain(self.expected_untracked_tree.iter())
        {
            if value.len() != 64
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(changes::invalid());
            }
        }
        for value in self.pinned_oid.iter().chain(self.result.stash_oid.iter()) {
            oid(value.as_bytes(), length)?;
        }
        if self.recovery_ref.as_ref().is_some_and(|name| {
            name.strip_prefix("refs/opengit/recovery/")
                .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        }) {
            return Err(changes::invalid());
        }
        if self.result.recovery_ref_retention == StashRetention::Present
            && (self.recovery_ref.is_none() || self.pinned_oid.is_none())
        {
            return Err(changes::invalid());
        }
        Ok(())
    }
}
pub(crate) async fn prepare(
    q: &ReadQuery<'_>,
    intent: &ActionIntent,
    targets: &[Target],
) -> Result<StashProof, AppError> {
    let include_untracked = match intent {
        ActionIntent::StashPush {
            include_untracked, ..
        } => *include_untracked,
        ActionIntent::StashApply { .. } => false,
        _ => {
            let mut e = changes::unsupported();
            e.message = "Stash Pop and Drop are deferred to version 0.4. Use Save or Apply; Apply keeps the stash.".into();
            return Err(e);
        }
    };
    let snapshot = Box::pin(save_snapshot(q, include_untracked)).await?;
    let head_tree = tree(q, &snapshot.head, snapshot.stack.length).await?;
    let applying = matches!(intent, ActionIntent::StashApply { .. });
    let target = if applying {
        if snapshot.index != head_tree || snapshot.working != head_tree {
            let mut e = error(ErrorCode::ExternalChange, Phase::Validation);
            e.message = "Stash Apply requires clean tracked files and index. Save or commit the tracked changes first.".into();
            return Err(e);
        }
        let target = validate_target(q, targets.first().ok_or_else(changes::invalid)?).await?;
        // Inspect every authoritative tree now; unsupported types and unsafe paths
        // must fail before the durable reservation or native pin creation.
        tree(q, &target.tree, snapshot.stack.length).await?;
        tree(q, &target.index_tree, snapshot.stack.length).await?;
        check_collisions(q, &target, snapshot.stack.length).await?;
        Some(target)
    } else {
        if !targets.is_empty() {
            return Err(changes::invalid());
        }
        if snapshot.index == head_tree
            && snapshot.working == head_tree
            && snapshot.untracked.is_empty()
        {
            let mut e = error(ErrorCode::InvalidRequest, Phase::Validation);
            e.message = "There are no selected changes to save. Untracked files require explicit opt-in; ignored files are excluded.".into();
            return Err(e);
        }
        None
    };
    let p = StashProof {
        phase: StashPhase::Reserved,
        before_head: snapshot.head.clone(),
        before_stack: snapshot.stack.digest.clone(),
        before_index: super::index::index_digest(q.location)?,
        before_working: snapshot.working_digest(),
        expected_index_tree: tree_digest(&snapshot.index),
        expected_working_tree: tree_digest(&snapshot.working),
        expected_untracked_tree: include_untracked.then(|| tree_digest(&snapshot.untracked)),
        recovery_ref: applying.then(|| {
            format!(
                "refs/opengit/recovery/{}",
                crate::history::sessions::token()
            )
        }),
        pinned_oid: target.as_ref().map(|target| target.oid.clone()),
        effects: ActionEffects::NotApplied,
        result: StashActionResult {
            stash_oid: target.map(|target| target.oid),
            stash_retention: if applying {
                StashRetention::Present
            } else {
                StashRetention::Unknown
            },
            recovery_ref_retention: StashRetention::Absent,
            resolution: StashResolution::NotRequired,
            failure_reason: None,
        },
    };
    p.validate(intent.kind())?;
    Ok(p)
}

fn checkpoint(c: &super::ActionContext, p: &StashProof, owned: bool) -> Result<(), AppError> {
    let artifacts = if owned {
        vec![super::journal::ArtifactProof {
            root: super::journal::ArtifactRoot::Common,
            kind: super::journal::ArtifactKind::RecoveryRef,
            name: p.recovery_ref.clone().ok_or_else(changes::invalid)?,
            identity: p
                .pinned_oid
                .as_ref()
                .ok_or_else(changes::invalid)?
                .as_bytes()
                .to_vec(),
        }]
    } else {
        vec![]
    };
    c.scheduler
        .action_stash_checkpoint(&c.operation_id, p.clone(), artifacts)
}
async fn native(
    c: &super::ActionContext,
    args: &[std::ffi::OsString],
    guard: String,
    files: BTreeMap<Vec<u8>, String>,
) -> Result<crate::git::runner::GitOutput, AppError> {
    use std::sync::Arc;
    let location = c.location.clone();
    let validate = c.validate.clone();
    let cancel = c.cancel.clone();
    let mut options = c.run_options(Default::default())?;
    options.before_resume = Some(Arc::new(move |_| {
        if cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Publish));
        }
        validate(super::Gate::Check)?;
        if preflight::native_stamp(&location)? != guard {
            return Err(changes::changed());
        }
        for (path, stamp) in &files {
            if path_stamp(&location, path)? != *stamp {
                return Err(changes::changed());
            }
        }
        Ok(())
    }));
    options.environment.extend([
        ("GIT_ALLOW_PROTOCOL".into(), "".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
    ]);
    let mut all = preflight::global_args(&c.location)?;
    all.extend(argv(&["-c", "advice.graftFileDeprecated=false"]));
    all.extend_from_slice(args);
    let mut runner = c.runner.clone();
    runner.timeout = std::time::Duration::from_secs(120);
    runner
        .output_history(
            Some(&c.location.root),
            &all,
            &c.cancel,
            Phase::Publish,
            options,
            None,
        )
        .await
}
async fn saved_matches(
    q: &ReadQuery<'_>,
    object: &StashObject,
    p: &StashProof,
    length: usize,
) -> Result<bool, AppError> {
    if object.base != p.before_head
        || tree_digest(&tree(q, &object.index_tree, length).await?) != p.expected_index_tree
        || tree_digest(&tree(q, &object.tree, length).await?) != p.expected_working_tree
    {
        return Ok(false);
    }
    let actual_untracked = if let Some(value) = &object.untracked_tree {
        tree(q, value, length).await?
    } else {
        Tree::new()
    };
    Ok(match &p.expected_untracked_tree {
        Some(expected) => tree_digest(&actual_untracked) == *expected,
        None => object.untracked_tree.is_none(),
    })
}
fn native_error(output: &Result<crate::git::runner::GitOutput, AppError>) -> Option<AppError> {
    match output {
        Err(e) => Some(e.clone()),
        Ok(o) if !o.success => Some(crate::git::error::git_failure(&o.stderr, Phase::Publish)),
        _ => None,
    }
}
fn read_again(q: &mut ReadQuery<'_>) {
    q.cancel = Default::default();
    q.deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
}
fn retention(stack: &Stack, value: Option<&String>) -> StashRetention {
    match value {
        Some(value) if stack.oids.contains(value) => StashRetention::Present,
        Some(_) => StashRetention::Absent,
        None => StashRetention::Unknown,
    }
}
pub(crate) async fn execute(
    c: super::ActionContext,
    database: std::sync::Arc<crate::persistence::workspace::Database>,
) -> Result<crate::operations::scheduler::Completion, AppError> {
    let mut p = super::journal::read(&database, &c.operation_id)?
        .stash_action
        .ok_or_else(changes::invalid)?;
    p.validate(c.intent.kind())?;
    if p.phase != StashPhase::Reserved {
        return Err(changes::invalid());
    }
    let mut q = c.reader()?;
    let guard = preflight::native_stamp(&c.location)?;
    let now = Box::pin(prepare(&q, &c.intent, &c.targets)).await?;
    if now.before_head != p.before_head
        || now.before_stack != p.before_stack
        || now.before_index != p.before_index
        || now.before_working != p.before_working
        || now.expected_index_tree != p.expected_index_tree
        || now.expected_working_tree != p.expected_working_tree
        || now.expected_untracked_tree != p.expected_untracked_tree
        || now.pinned_oid != p.pinned_oid
    {
        return Err(changes::changed());
    }
    let include_untracked = matches!(
        c.intent,
        ActionIntent::StashPush {
            include_untracked: true,
            ..
        }
    );
    let snapshot = save_snapshot(&q, include_untracked).await?;
    if snapshot.working_digest() != p.before_working || snapshot.stack.digest != p.before_stack {
        return Err(changes::changed());
    }
    let stable =
        preflight::capture_stable(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
    if !c.before_write().await? {
        return Err(error(ErrorCode::Interrupted, Phase::Publish));
    }
    if let ActionIntent::StashPush {
        message,
        include_untracked,
    } = &c.intent
    {
        p.phase = StashPhase::PushStarted;
        p.effects = ActionEffects::Unknown;
        checkpoint(&c, &p, false)?;
        let mut args = argv(&["stash", "push", "--message", message]);
        if *include_untracked {
            args.push("--include-untracked".into());
        }
        let output = native(&c, &args, guard, snapshot.stamps.clone()).await;
        if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
            return Err(output.err().unwrap());
        }
        read_again(&mut q);
        let after = stack(&q).await?;
        if let Some(value) = after
            .oids
            .first()
            .filter(|value| !snapshot.stack.oids.contains(value))
        {
            let object = inspect(&q, value, after.length).await?;
            if saved_matches(&q, &object, &p, after.length).await? {
                p.result.stash_oid = Some(value.clone());
                let clean = save_snapshot(&q, false).await?;
                let base = tree(&q, &snapshot.head, after.length).await?;
                let stack_exact = after.oids.len() == snapshot.stack.oids.len() + 1
                    && after.oids[1..] == snapshot.stack.oids;
                let untracked_removed = !*include_untracked
                    || snapshot.untracked.keys().all(|path| {
                        checked_path(&c.location, path).is_ok_and(|path| !path.exists())
                    });
                p.effects = if clean.head == p.before_head
                    && clean.index == base
                    && clean.working == base
                    && stack_exact
                    && untracked_removed
                {
                    ActionEffects::Applied
                } else {
                    ActionEffects::Partial
                };
            }
        }
        if p.result.stash_oid.is_none() {
            p.effects = if after.digest == snapshot.stack.digest
                && super::index::index_digest(&c.location)? == p.before_index
                && snapshot.validate_files(&c.location).is_ok()
            {
                ActionEffects::NotApplied
            } else {
                ActionEffects::Unknown
            };
        }
        p.result.stash_retention = retention(&after, p.result.stash_oid.as_ref());
        p.result.resolution =
            if matches!(p.effects, ActionEffects::Partial | ActionEffects::Unknown) {
                StashResolution::Inspect
            } else {
                StashResolution::NotRequired
            };
        p.result.failure_reason = native_error(&output).map(|_| StashFailureReason::Native);
        p.phase = StashPhase::Observed;
        checkpoint(&c, &p, false)?;
        if let Some(e) = native_error(&output) {
            return Err(e);
        }
        return Ok(super::completed(p.effects, None));
    }
    let value = p.pinned_oid.clone().ok_or_else(changes::invalid)?;
    let recovery = p.recovery_ref.clone().ok_or_else(changes::invalid)?;
    // Reservation is already durable in the consumed plan. Creation uses native
    // expected absence; a crash before ownership checkpoint never grants cleanup.
    let zero = "0".repeat(value.len());
    p.effects = ActionEffects::Unknown;
    p.result.recovery_ref_retention = StashRetention::Unknown;
    p.result.resolution = StashResolution::Inspect;
    checkpoint(&c, &p, false)?;
    let pin = native(
        &c,
        &argv(&["update-ref", "--no-deref", &recovery, &value, &zero]),
        guard,
        snapshot.stamps.clone(),
    )
    .await;
    if native_error(&pin).is_some() {
        p.effects = ActionEffects::Unknown;
        p.result.recovery_ref_retention = StashRetention::Unknown;
        p.result.resolution = StashResolution::Inspect;
        p.result.failure_reason = Some(StashFailureReason::Recovery);
        checkpoint(&c, &p, false)?;
        return Err(native_error(&pin).unwrap());
    }
    read_again(&mut q);
    let pin_value = changes::run(&q, &argv(&["rev-parse", "--verify", &recovery])).await?;
    if pin_value != format!("{value}\n").as_bytes() {
        return Err(changes::changed());
    }
    p.phase = StashPhase::PinCreated;
    p.result.recovery_ref_retention = StashRetention::Present;
    checkpoint(&c, &p, true)?;
    // Freeze BEFORE semantic recheck. Stack-only changes before this boundary
    // cannot retarget the pinned object; later unchecked state is never adopted.
    let apply_guard = preflight::native_stamp(&c.location)?;
    let after_pin =
        preflight::capture_stable(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
    if stable.fingerprint != after_pin.fingerprint {
        return Err(changes::changed());
    }
    snapshot.validate_files(&c.location)?;
    let object = inspect(&q, &value, snapshot.stack.length).await?;
    // Freeze every path Apply might write before the last semantic collision
    // read. A path created while that read is in flight must fail the native
    // before-resume check, rather than becoming the newly accepted baseline.
    let mut files = snapshot.stamps.clone();
    let touched = changes::run(
        &q,
        &argv(&[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            &object.base,
            &object.oid,
            "--",
        ]),
    )
    .await?;
    let mut touched = parse_paths(&touched)?;
    if let Some(tree_id) = &object.untracked_tree {
        touched.extend(tree(&q, tree_id, snapshot.stack.length).await?.into_keys());
    }
    for path in touched {
        if !files.contains_key(&path) {
            files.insert(path.clone(), path_stamp(&c.location, &path)?);
        }
    }
    check_collisions(&q, &object, snapshot.stack.length).await?;
    snapshot.validate_files(&c.location)?;
    p.phase = StashPhase::ApplyStarted;
    p.effects = ActionEffects::Unknown;
    checkpoint(&c, &p, true)?;
    let output = native(
        &c,
        &argv(&["-c", "stash.index=false", "stash", "apply", &value]),
        apply_guard,
        files.clone(),
    )
    .await;
    if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
        return Err(output.err().unwrap());
    }
    read_again(&mut q);
    let after = changes::capture(&q).await?;
    let unmerged = after
        .status
        .entries
        .iter()
        .any(|entry| entry.kind == StatusEntryKind::Unmerged);
    let unchanged = super::index::index_digest(&c.location)? == p.before_index
        && files
            .iter()
            .all(|(path, stamp)| path_stamp(&c.location, path).as_ref() == Ok(stamp));
    let head_unchanged = after.status.head.oid.as_deref() == Some(p.before_head.as_str());
    p.effects = if unmerged {
        ActionEffects::Conflict
    } else if !head_unchanged {
        ActionEffects::Unknown
    } else if unchanged {
        ActionEffects::NotApplied
    } else if native_error(&output).is_some() {
        ActionEffects::Partial
    } else {
        ActionEffects::Applied
    };
    p.phase = if unmerged {
        StashPhase::Conflict
    } else {
        StashPhase::Observed
    };
    p.result.resolution = if unmerged {
        StashResolution::Unmerged
    } else if matches!(p.effects, ActionEffects::Partial | ActionEffects::Unknown) {
        StashResolution::Inspect
    } else {
        StashResolution::NotRequired
    };
    p.result.failure_reason = if unmerged {
        Some(StashFailureReason::Conflict)
    } else {
        native_error(&output).map(|_| StashFailureReason::Native)
    };
    let after_stack = stack(&q).await?;
    p.result.stash_retention = retention(&after_stack, Some(&value));
    checkpoint(&c, &p, true)?;
    if !unmerged && native_error(&output).is_none() && head_unchanged && !c.cancel.is_cancelled() {
        // Success may release only our proven pin, using exact expected OID.
        // Any failure/uncertainty retains evidence and never removes another ref.
        let cleanup_guard = preflight::native_stamp(&c.location)?;
        let actual =
            changes::output(&q, &argv(&["rev-parse", "--verify", "--quiet", &recovery])).await?;
        if actual.success && actual.stdout == format!("{value}\n").as_bytes() {
            let cleanup = native(
                &c,
                &argv(&["update-ref", "--no-deref", "-d", &recovery, &value]),
                cleanup_guard,
                BTreeMap::new(),
            )
            .await;
            if cleanup.as_ref().is_ok_and(|output| output.success) {
                p.result.recovery_ref_retention = StashRetention::Absent;
                checkpoint(&c, &p, false)?;
            } else {
                p.result.recovery_ref_retention = StashRetention::Unknown;
                p.result.resolution = StashResolution::Inspect;
                checkpoint(&c, &p, true)?;
            }
        } else {
            p.result.recovery_ref_retention = StashRetention::Unknown;
            p.result.resolution = StashResolution::Inspect;
            checkpoint(&c, &p, true)?;
        }
    }
    if unmerged {
        return Ok(super::completed(ActionEffects::Conflict, None));
    }
    if let Some(e) = native_error(&output) {
        return Err(e);
    }
    Ok(super::completed(p.effects, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_duplicate_stack_entries_have_distinct_bound_identity() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let list = vec![a.clone(), a, b];
        let stack = Stack {
            length: 40,
            digest: stack_digest(&list),
            oids: list,
        };
        assert_ne!(entry_id(&stack, 0), entry_id(&stack, 1));
        for i in 0..stack.oids.len() {
            let t = target(&stack, &entry_id(&stack, i)).unwrap();
            let (ordinal, value, digest) = decode(&t).unwrap();
            assert_eq!(ordinal, i);
            assert_eq!(value, stack.oids[i]);
            assert_eq!(digest, stack.digest);
        }
        assert!(target(&stack, "stash@{0}").is_err());
        let mut t = target(&stack, &entry_id(&stack, 0)).unwrap();
        t.identity[8] = b'b';
        assert!(decode(&t).is_err());
        let mut reordered = stack.oids.clone();
        reordered.swap(1, 2);
        assert_ne!(stack_digest(&reordered), stack.digest);
    }
    #[test]
    fn physical_commit_headers_preserve_order_and_ignore_message_data() {
        let tree = "a".repeat(40);
        let a = "b".repeat(40);
        let b = "c".repeat(40);
        let object = format!("tree {tree}\nparent {b}\nparent {a}\nauthor fixture\ncommitter fixture\n\nparent message is not topology\n");
        let parsed = parse_commit(object.as_bytes(), 40).unwrap();
        assert_eq!(parsed.tree, tree);
        assert_eq!(parsed.parents, vec![b, a]);
        assert!(parse_commit(b"tree invalid\n\nmessage", 40).is_err());
        assert!(parse_stack(format!("{tree}\n\n").as_bytes(), 40).is_err());
    }
    #[test]
    fn collisions_cover_case_and_file_directory_prefix_without_sibling_false_positive() {
        for (a, b) in [
            ("new.txt", "NEW.TXT"),
            ("dir/file.txt", "DIR"),
            ("dir", "dir/ignored.txt"),
            ("Ω/file", "ω/FILE"),
        ] {
            assert!(
                paths_collide(a.as_bytes(), b.as_bytes()).unwrap(),
                "{a} {b}"
            );
        }
        assert!(!paths_collide(b"dir/a", b"dir/b").unwrap());
        assert!(!paths_collide(b"a", b"ab/x").unwrap());
        for invalid in [b"../x".as_slice(), b".git/config", b"x\\y", b"x//y"] {
            assert!(paths_collide(invalid, b"x").is_err());
        }
    }
}
