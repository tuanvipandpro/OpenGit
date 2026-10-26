//! Exact byte alignment/candidates. Display diff text is never patch input.
use super::{
    authority::{Target, TargetKind},
    catalog::ReadQuery,
    changes,
    journal::SafeTargetProof,
    preflight,
};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::argv},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::{io::Read, path::PathBuf};
pub(crate) const VERSION: &str = "opengit-lcs-delete-first-v1";
pub(crate) const MAX_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_ROWS: usize = 16384;
const MAX_CELLS: usize = 1_048_576;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Limit {
    TooLarge,
    ComplexityLimit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Equal,
    Add,
    Delete,
}
#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub kind: Kind,
    pub bytes: Vec<u8>,
    pub ordinal: Option<u32>,
}
#[derive(Clone, Debug)]
pub(crate) struct Alignment {
    pub edits: Vec<Edit>,
    pub units: u32,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub bytes: Vec<u8>,
    pub exists: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CandidateError {
    InvalidSelection,
    Unterminated,
}
#[derive(Clone, Debug)]
pub(crate) struct AttributeGuard {
    paths: Vec<PathBuf>,
    stamp: String,
}
#[derive(Clone, Debug)]
struct Endpoint {
    bytes: Vec<u8>,
    exists: bool,
    mode: u32,
    oid: Option<String>,
}
pub(crate) struct PatchData {
    pub side: ChangeSide,
    pub diff_id: String,
    pub eligibility: PartialEligibility,
    pub alignment: Option<Alignment>,
    left: Endpoint,
    right: Endpoint,
    pub attrs: AttributeGuard,
    pub mode: u32,
}
fn invalid() -> AppError {
    changes::invalid()
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn digest_part(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}
fn attribute_stamp(paths: &[PathBuf]) -> Result<String, AppError> {
    let mut h = Sha256::new();
    let mut total = 0usize;
    for path in paths {
        digest_part(&mut h, &crate::persistence::workspace::encode(path));
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => h.update([0]),
            Err(_) => return Err(changes::changed()),
            Ok(_) => {
                let mut f = changes::open_regular(path)?;
                h.update([1]);
                digest_part(&mut h, &crate::repositories::discovery::identity(path)?);
                let mut b = [0; 65536];
                loop {
                    let n = f.file.read(&mut b).map_err(|_| changes::changed())?;
                    if n == 0 {
                        break;
                    }
                    total += n;
                    if total > 8 * 1024 * 1024 {
                        return Err(changes::unsupported());
                    }
                    h.update(&b[..n]);
                }
            }
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
pub(crate) fn validate_attributes(guards: &[AttributeGuard]) -> Result<(), AppError> {
    for g in guards {
        if attribute_stamp(&g.paths)? != g.stamp {
            return Err(changes::changed());
        }
    }
    Ok(())
}
async fn attributes(
    q: &ReadQuery<'_>,
    raw: &[u8],
) -> Result<(AttributeGuard, Vec<u8>, PartialEligibility), AppError> {
    let path = changes::path(q.location, raw)?;
    let mut paths = vec![
        q.location.common.join("info/attributes"),
        q.location.git_dir.join("info/attributes"),
    ];
    let mut parent = path.parent();
    while let Some(p) = parent {
        paths.push(p.join(".gitattributes"));
        if p == q.location.root {
            break;
        }
        parent = p.parent();
        if paths.len() > 64 {
            return Err(changes::unsupported());
        }
    }
    for key in ["GIT_ATTR_GLOBAL", "GIT_ATTR_SYSTEM"] {
        let out = changes::output(q, &argv(&["var", key])).await?;
        if !out.success {
            return Err(changes::unsupported());
        }
        let text = std::str::from_utf8(&out.stdout)
            .map_err(|_| malformed())?
            .trim_end_matches(['\r', '\n']);
        if !text.is_empty() {
            let p = PathBuf::from(text);
            paths.push(if p.is_absolute() {
                p
            } else {
                q.location.root.join(p)
            });
        }
    }
    paths.sort();
    paths.dedup();
    let guard = AttributeGuard {
        stamp: attribute_stamp(&paths)?,
        paths,
    };
    let text = std::str::from_utf8(raw).map_err(|_| changes::unsupported())?;
    let out = changes::run(
        q,
        &argv(&[
            "--literal-pathspecs",
            "check-attr",
            "--all",
            "-z",
            "--",
            text,
        ]),
    )
    .await?;
    let fields = out.split(|b| *b == 0).collect::<Vec<_>>();
    if fields.len() % 3 != 1 || fields.last() != Some(&b"".as_slice()) {
        return Err(malformed());
    }
    let (mut text_attr, mut eol) = ("unspecified".to_owned(), "unspecified".to_owned());
    let mut eligibility = PartialEligibility::Eligible;
    for f in fields[..fields.len() - 1].chunks(3) {
        if f[0] != raw {
            return Err(malformed());
        }
        let key = std::str::from_utf8(f[1]).map_err(|_| malformed())?;
        let value = std::str::from_utf8(f[2]).map_err(|_| malformed())?;
        match key {
            "text" => text_attr = value.into(),
            "eol" => eol = value.into(),
            "filter" | "ident" | "diff" | "working-tree-encoding"
                if value == "unset" || value == "unspecified" => {}
            _ => eligibility = PartialEligibility::Attributes,
        }
    }
    async fn config(q: &ReadQuery<'_>, key: &str) -> Result<String, AppError> {
        let out = changes::output(q, &argv(&["config", "--get", key])).await?;
        if !out.success && out.exit_code != Some(1) {
            return Err(changes::unsupported());
        }
        Ok(String::from_utf8(out.stdout)
            .map_err(|_| malformed())?
            .trim()
            .to_ascii_lowercase())
    }
    let autocrlf = config(q, "core.autocrlf").await?;
    let core_eol = config(q, "core.eol").await?;
    if !["unspecified", "unset", "set", "auto"].contains(&text_attr.as_str())
        || !["unspecified", "unset", "lf", "crlf"].contains(&eol.as_str())
    {
        eligibility = PartialEligibility::Attributes;
    }
    if text_attr != "unset"
        && eol != "lf"
        && (eol == "crlf"
            || matches!(autocrlf.as_str(), "true" | "yes" | "on" | "1")
            || matches!(text_attr.as_str(), "set" | "auto")
                && autocrlf != "input"
                && core_eol != "lf")
    {
        eligibility = PartialEligibility::LineEndings;
    }
    validate_attributes(std::slice::from_ref(&guard))?;
    Ok((guard, out, eligibility))
}
async fn blob(q: &ReadQuery<'_>, oid: &str, mode: u32) -> Result<Endpoint, PartialEligibility> {
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(PartialEligibility::Unsupported);
    }
    let size = changes::run(q, &argv(&["cat-file", "-s", oid]))
        .await
        .map_err(|_| PartialEligibility::Unsupported)?;
    let size = std::str::from_utf8(&size)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .ok_or(PartialEligibility::Unsupported)?;
    if size > MAX_BYTES {
        return Err(PartialEligibility::TooLarge);
    }
    let bytes = changes::run(q, &argv(&["cat-file", "blob", oid]))
        .await
        .map_err(|_| PartialEligibility::Unsupported)?;
    if bytes.len() != size {
        return Err(PartialEligibility::Unsupported);
    }
    Ok(Endpoint {
        bytes,
        exists: true,
        mode,
        oid: Some(oid.into()),
    })
}
fn absent() -> Endpoint {
    Endpoint {
        bytes: vec![],
        exists: false,
        mode: 0,
        oid: None,
    }
}
async fn index_endpoint(q: &ReadQuery<'_>, raw: &[u8]) -> Result<Endpoint, PartialEligibility> {
    let name = std::str::from_utf8(raw).map_err(|_| PartialEligibility::Unsupported)?;
    let bytes = changes::run(
        q,
        &argv(&[
            "--literal-pathspecs",
            "ls-files",
            "--stage",
            "-z",
            "--",
            name,
        ]),
    )
    .await
    .map_err(|_| PartialEligibility::Unsupported)?;
    let rows = bytes
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(absent());
    }
    if rows.len() != 1 {
        return Err(PartialEligibility::Unsupported);
    }
    let at = rows[0]
        .iter()
        .position(|b| *b == b'\t')
        .ok_or(PartialEligibility::Unsupported)?;
    if &rows[0][at + 1..] != raw {
        return Err(PartialEligibility::Unsupported);
    }
    let f = std::str::from_utf8(&rows[0][..at])
        .map_err(|_| PartialEligibility::Unsupported)?
        .split(' ')
        .collect::<Vec<_>>();
    if f.len() != 3 || f[2] != "0" {
        return Err(PartialEligibility::Unsupported);
    }
    let mode = u32::from_str_radix(f[0], 8).map_err(|_| PartialEligibility::Mode)?;
    if !matches!(mode, 0o100644 | 0o100755) {
        return Err(PartialEligibility::Mode);
    }
    blob(q, f[1], mode).await
}
async fn head_endpoint(q: &ReadQuery<'_>, raw: &[u8]) -> Result<Endpoint, PartialEligibility> {
    let head = changes::output(q, &argv(&["rev-parse", "--verify", "HEAD"]))
        .await
        .map_err(|_| PartialEligibility::Unsupported)?;
    if !head.success {
        return Ok(absent());
    }
    let name = std::str::from_utf8(raw).map_err(|_| PartialEligibility::Unsupported)?;
    let bytes = changes::run(
        q,
        &argv(&[
            "--literal-pathspecs",
            "ls-tree",
            "--full-tree",
            "-z",
            "HEAD",
            "--",
            name,
        ]),
    )
    .await
    .map_err(|_| PartialEligibility::Unsupported)?;
    let rows = bytes
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(absent());
    }
    if rows.len() != 1 {
        return Err(PartialEligibility::Unsupported);
    }
    let at = rows[0]
        .iter()
        .position(|b| *b == b'\t')
        .ok_or(PartialEligibility::Unsupported)?;
    if &rows[0][at + 1..] != raw {
        return Err(PartialEligibility::Unsupported);
    }
    let f = std::str::from_utf8(&rows[0][..at])
        .map_err(|_| PartialEligibility::Unsupported)?
        .split(' ')
        .collect::<Vec<_>>();
    if f.len() != 3 || f[1] != "blob" {
        return Err(PartialEligibility::Mode);
    }
    let mode = u32::from_str_radix(f[0], 8).map_err(|_| PartialEligibility::Mode)?;
    if !matches!(mode, 0o100644 | 0o100755) {
        return Err(PartialEligibility::Mode);
    }
    blob(q, f[2], mode).await
}
async fn working_endpoint(
    q: &ReadQuery<'_>,
    raw: &[u8],
    index: &Endpoint,
    probe_mode: bool,
) -> Result<Endpoint, PartialEligibility> {
    let path = changes::path(q.location, raw).map_err(|_| PartialEligibility::Unsupported)?;
    if !path.exists() {
        return Ok(absent());
    }
    let mut file = changes::open_regular(&path).map_err(|_| PartialEligibility::Mode)?;
    let mut bytes = Vec::new();
    (&mut file.file)
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PartialEligibility::Unsupported)?;
    if bytes.len() > MAX_BYTES {
        return Err(PartialEligibility::TooLarge);
    }
    let mut mode = if index.exists { index.mode } else { 0o100644 };
    if index.exists && probe_mode {
        let name = std::str::from_utf8(raw).map_err(|_| PartialEligibility::Unsupported)?;
        let raw_diff = changes::run(
            q,
            &argv(&[
                "--literal-pathspecs",
                "diff-files",
                "--raw",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "-z",
                "--",
                name,
            ]),
        )
        .await
        .map_err(|_| PartialEligibility::Unsupported)?;
        if let Some(row) = raw_diff.split(|b| *b == 0).find(|e| !e.is_empty()) {
            let text = std::str::from_utf8(row).map_err(|_| PartialEligibility::Unsupported)?;
            let fields = text.split(' ').collect::<Vec<_>>();
            if fields.len() < 2 || !fields[0].starts_with(':') {
                return Err(PartialEligibility::Mode);
            }
            mode = u32::from_str_radix(fields[1], 8).map_err(|_| PartialEligibility::Mode)?;
        }
    }
    if !matches!(mode, 0o100644 | 0o100755) {
        return Err(PartialEligibility::Mode);
    }
    Ok(Endpoint {
        bytes,
        exists: true,
        mode,
        oid: None,
    })
}
async fn hash_readonly(
    q: &ReadQuery<'_>,
    raw: &[u8],
    bytes: &[u8],
    filters: bool,
) -> Result<String, AppError> {
    let mut args = preflight::global_args(q.location)?;
    args.extend(argv(&[
        "-c",
        "advice.graftFileDeprecated=false",
        "hash-object",
    ]));
    if filters {
        args.push(
            format!(
                "--path={}",
                std::str::from_utf8(raw).map_err(|_| invalid())?
            )
            .into(),
        );
    } else {
        args.push("--no-filters".into());
    }
    args.push("--stdin".into());
    let output = q
        .runner
        .output_history(
            Some(&q.location.root),
            &args,
            &q.cancel,
            Phase::Inspection,
            crate::git::runner::RunOptions {
                lifetime_guard: Some(q.lifetime.clone()),
                environment: vec![
                    ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                    ("GIT_ALLOW_PROTOCOL".into(), "".into()),
                ],
                ..Default::default()
            },
            Some(bytes),
        )
        .await?;
    if !output.success {
        return Err(changes::unsupported());
    }
    let s = String::from_utf8(output.stdout).map_err(|_| malformed())?;
    let s = s.trim();
    if !matches!(s.len(), 40 | 64) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(malformed());
    }
    Ok(s.into())
}
pub(crate) async fn capture(
    q: &ReadQuery<'_>,
    target: &Target,
    source_handle: &str,
    stamp: &str,
) -> Result<PatchData, AppError> {
    let (side, paths) = changes::paths(target)?;
    let raw = &paths[0];
    let mut eligibility = if paths.len() != 1 {
        PartialEligibility::Rename
    } else {
        PartialEligibility::Eligible
    };
    let (attrs, attribute_bytes, attribute_eligibility) = attributes(q, raw).await.unwrap_or((
        AttributeGuard {
            paths: vec![],
            stamp: preflight::digest(b""),
        },
        vec![],
        PartialEligibility::Attributes,
    ));
    if eligibility == PartialEligibility::Eligible {
        eligibility = attribute_eligibility;
    }
    let index = index_endpoint(q, raw).await;
    let pair = match index {
        Ok(index) => {
            let left = if side == ChangeSide::Unstaged {
                Ok(index.clone())
            } else {
                head_endpoint(q, raw).await
            };
            let right = if side == ChangeSide::Unstaged {
                working_endpoint(q, raw, &index, eligibility == PartialEligibility::Eligible).await
            } else {
                Ok(index)
            };
            left.and_then(|l| right.map(|r| (l, r)))
        }
        Err(e) => Err(e),
    };
    let (left, right) = match pair {
        Ok(v) => v,
        Err(e) => {
            if eligibility == PartialEligibility::Eligible {
                eligibility = e;
            }
            (absent(), absent())
        }
    };
    if eligibility == PartialEligibility::Eligible {
        if left.bytes.contains(&0) || right.bytes.contains(&0) {
            eligibility = PartialEligibility::Binary;
        } else if std::str::from_utf8(&left.bytes).is_err()
            || std::str::from_utf8(&right.bytes).is_err()
        {
            eligibility = PartialEligibility::Encoding;
        } else if left.bytes.contains(&b'\r') || right.bytes.contains(&b'\r') {
            eligibility = PartialEligibility::LineEndings;
        } else if left.exists && right.exists && left.mode != right.mode {
            eligibility = PartialEligibility::Mode;
        }
    }
    if eligibility == PartialEligibility::Eligible {
        for endpoint in [&left, &right].into_iter().filter(|e| e.exists) {
            let raw_oid = hash_readonly(q, raw, &endpoint.bytes, false).await?;
            if endpoint.oid.as_ref().is_some_and(|v| v != &raw_oid) {
                eligibility = PartialEligibility::Unsupported;
                break;
            }
            if hash_readonly(q, raw, &endpoint.bytes, true).await? != raw_oid {
                eligibility = PartialEligibility::Attributes;
                break;
            }
            if let Some(oid) = &endpoint.oid {
                let name = std::str::from_utf8(raw).map_err(|_| invalid())?;
                let converted = changes::run(
                    q,
                    &argv(&["cat-file", "--filters", &format!("--path={name}"), oid]),
                )
                .await?;
                if converted != endpoint.bytes {
                    eligibility = PartialEligibility::LineEndings;
                    break;
                }
            }
        }
    }
    let alignment = if eligibility == PartialEligibility::Eligible {
        match align(&left.bytes, &right.bytes) {
            Ok(v) => {
                if v.units == 0 {
                    eligibility = if left.exists != right.exists {
                        PartialEligibility::FilePresenceOnly
                    } else {
                        PartialEligibility::Unsupported
                    };
                    None
                } else {
                    Some(v)
                }
            }
            Err(Limit::TooLarge) => {
                eligibility = PartialEligibility::TooLarge;
                None
            }
            Err(Limit::ComplexityLimit) => {
                eligibility = PartialEligibility::ComplexityLimit;
                None
            }
        }
    } else {
        None
    };
    let mut h = Sha256::new();
    for part in [
        VERSION.as_bytes(),
        source_handle.as_bytes(),
        stamp.as_bytes(),
        target.identity.as_slice(),
        attribute_bytes.as_slice(),
        attrs.stamp.as_bytes(),
    ] {
        digest_part(&mut h, part);
    }
    for endpoint in [&left, &right] {
        h.update([u8::from(endpoint.exists)]);
        h.update(endpoint.mode.to_le_bytes());
        digest_part(&mut h, &endpoint.bytes);
    }
    if !attrs.paths.is_empty() {
        validate_attributes(std::slice::from_ref(&attrs))?;
    }
    let mode = if left.exists { left.mode } else { right.mode };
    let mut data = PatchData {
        side,
        diff_id: format!("{:x}", h.finalize()),
        eligibility,
        alignment,
        left,
        right,
        attrs,
        mode,
    };
    // Leave space for the opaque display string and response framing inside
    // the existing 8MiB response cap, including escaped UTF-8/control text.
    if data.eligibility == PartialEligibility::Eligible && !structured_within_limit(&data.hunks()?)
    {
        data.eligibility = PartialEligibility::TooLarge;
        data.alignment = None;
    }
    Ok(data)
}
fn structured_within_limit(hunks: &[ChangeHunk]) -> bool {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(b.len())
                .filter(|n| *n <= 6 * 1024 * 1024)
                .ok_or_else(|| std::io::Error::other("limit"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(0), hunks).is_ok()
}
impl PatchData {
    pub(crate) fn hunks(&self) -> Result<Vec<ChangeHunk>, AppError> {
        let Some(a) = &self.alignment else {
            return Ok(vec![]);
        };
        let mut rows = Vec::new();
        let (mut old, mut new) = (1, 1);
        for e in &a.edits {
            let has_lf = e.bytes.ends_with(b"\n");
            let bytes = if has_lf {
                &e.bytes[..e.bytes.len() - 1]
            } else {
                &e.bytes
            };
            let text = std::str::from_utf8(bytes)
                .map_err(|_| malformed())?
                .to_owned();
            rows.push(ChangeDiffRow {
                line_id: e.ordinal.map(|n| format!("line-{}-{n}", self.diff_id)),
                kind: match e.kind {
                    Kind::Equal => ChangeRowKind::Context,
                    Kind::Add => ChangeRowKind::Add,
                    Kind::Delete => ChangeRowKind::Delete,
                },
                old_line: if e.kind == Kind::Add { None } else { Some(old) },
                new_line: if e.kind == Kind::Delete {
                    None
                } else {
                    Some(new)
                },
                text,
                has_lf,
            });
            if e.kind != Kind::Add {
                old += 1;
            }
            if e.kind != Kind::Delete {
                new += 1;
            }
        }
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (i, e) in a
            .edits
            .iter()
            .enumerate()
            .filter(|(_, e)| e.ordinal.is_some())
        {
            let _ = e;
            let (start, end) = (i.saturating_sub(3), (i + 4).min(rows.len()));
            if let Some(last) = spans.last_mut().filter(|last| start <= last.1) {
                last.1 = end;
            } else {
                spans.push((start, end));
            }
        }
        Ok(spans
            .into_iter()
            .map(|(start, end)| {
                let n = a.edits[start..end].iter().find_map(|e| e.ordinal).unwrap();
                ChangeHunk {
                    hunk_id: format!("hunk-{}-{n}", self.diff_id),
                    rows: rows[start..end].to_vec(),
                }
            })
            .collect())
    }
    pub(crate) fn candidate(&self, selected: &BTreeSet<u32>) -> Result<Candidate, AppError> {
        candidate(self.alignment.as_ref().ok_or_else(changes::unsupported)?,self.side==ChangeSide::Unstaged,selected,self.left.exists,self.right.exists).map_err(|_|{let mut e=error(ErrorCode::InvalidRequest,Phase::Validation);e.message="Selection is invalid or would place an unterminated line before another line. Select the complete change or use whole-file actions.".into();e})
    }
    pub(crate) fn select(&self, input: &ChangeSelectionInput) -> Result<BTreeSet<u32>, AppError> {
        if self.eligibility != PartialEligibility::Eligible {
            return Err(changes::unsupported());
        }
        let hunks = self.hunks()?;
        let mut ordinals = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let ids = match input {
            ChangeSelectionInput::Hunks { hunk_ids } => hunk_ids,
            ChangeSelectionInput::Lines { line_ids } => line_ids,
        };
        if ids.is_empty() || ids.len() > 256 {
            return Err(invalid());
        }
        for id in ids {
            if !seen.insert(id) {
                return Err(invalid());
            }
            let line_ids = match input {
                ChangeSelectionInput::Hunks { .. } => hunks
                    .iter()
                    .find(|h| h.hunk_id == *id)
                    .ok_or_else(invalid)?
                    .rows
                    .iter()
                    .filter_map(|r| r.line_id.clone())
                    .collect::<Vec<_>>(),
                ChangeSelectionInput::Lines { .. } => {
                    if !hunks
                        .iter()
                        .flat_map(|h| &h.rows)
                        .any(|r| r.line_id.as_ref() == Some(id))
                    {
                        return Err(invalid());
                    }
                    vec![id.clone()]
                }
            };
            for line in line_ids {
                let prefix = format!("line-{}-", self.diff_id);
                let n = line
                    .strip_prefix(&prefix)
                    .and_then(|v| v.parse::<u32>().ok())
                    .ok_or_else(invalid)?;
                if !ordinals.insert(n) {
                    return Err(invalid());
                }
            }
        }
        self.candidate(&ordinals)?;
        canonical_ranges(&ordinals)?;
        Ok(ordinals)
    }
}
fn canonical_ranges(selected: &BTreeSet<u32>) -> Result<Vec<(u32, u32)>, AppError> {
    if selected.is_empty() {
        return Err(invalid());
    }
    let mut ranges: Vec<(u32, u32)> = vec![];
    for n in selected {
        if let Some(last) = ranges.last_mut().filter(|r| r.1 == *n) {
            last.1 = n.checked_add(1).ok_or_else(invalid)?;
        } else {
            ranges.push((*n, n.checked_add(1).ok_or_else(invalid)?));
        }
    }
    if ranges.len() > 64 {
        return Err(error(ErrorCode::OutputLimit, Phase::Validation));
    }
    Ok(ranges)
}
pub(crate) fn selection_target(
    original: &Target,
    source_handle: &str,
    data: &PatchData,
    selected: &BTreeSet<u32>,
) -> Result<Target, AppError> {
    let Some(SafeTargetProof::Change { index_digest, .. }) = &original.proof else {
        return Err(invalid());
    };
    if source_handle.len() != 64
        || !source_handle.bytes().all(|b| b.is_ascii_hexdigit())
        || original.identity.len() > u16::MAX as usize
    {
        return Err(invalid());
    }
    let ranges = canonical_ranges(selected)?;
    let mut identity = b"PS1".to_vec();
    identity.extend((original.identity.len() as u16).to_le_bytes());
    identity.extend(&original.identity);
    identity.extend(source_handle.as_bytes());
    identity.extend(data.diff_id.as_bytes());
    identity.extend(index_digest.as_bytes());
    identity.push(ranges.len() as u8);
    for (a, b) in ranges {
        identity.extend(a.to_le_bytes());
        identity.extend(b.to_le_bytes());
    }
    if identity.len() > 2048 {
        return Err(error(ErrorCode::OutputLimit, Phase::Validation));
    }
    Ok(Target {
        kind: TargetKind::Selection,
        identity,
        proof: original.proof.clone(),
    })
}
struct Selection {
    original: Target,
    source_handle: String,
    diff_id: String,
    selected: BTreeSet<u32>,
}
fn decode(t: &Target) -> Result<Selection, AppError> {
    let b = &t.identity;
    if t.kind != TargetKind::Selection || b.len() > 2048 || b.get(..3) != Some(b"PS1".as_slice()) {
        return Err(invalid());
    }
    let len = u16::from_le_bytes(
        b.get(3..5)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ) as usize;
    let at = 5 + len;
    let original_bytes = b.get(5..at).ok_or_else(invalid)?.to_vec();
    let text = |start| -> Result<String, AppError> {
        let bytes = b.get(start..start + 64).ok_or_else(invalid)?;
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(invalid());
        }
        Ok(std::str::from_utf8(bytes).map_err(|_| invalid())?.into())
    };
    let source_handle = text(at)?;
    let diff_id = text(at + 64)?;
    let stamp = text(at + 128)?;
    let count = *b.get(at + 192).ok_or_else(invalid)? as usize;
    if count == 0 || count > 64 || b.len() != at + 193 + count * 8 {
        return Err(invalid());
    }
    let Some(SafeTargetProof::Change {
        path_digest,
        index_digest,
    }) = &t.proof
    else {
        return Err(invalid());
    };
    if stamp != *index_digest || preflight::digest(&original_bytes) != *path_digest {
        return Err(invalid());
    }
    let original = Target {
        kind: TargetKind::Change,
        identity: original_bytes,
        proof: t.proof.clone(),
    };
    if changes::paths(&original)?.1.len() != 1 || original.identity.get(1) != Some(&0) {
        return Err(invalid());
    }
    let mut selected = BTreeSet::new();
    let mut previous = None;
    for i in 0..count {
        let pos = at + 193 + i * 8;
        let a = u32::from_le_bytes(b[pos..pos + 4].try_into().unwrap());
        let z = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap());
        if a >= z || z > MAX_ROWS as u32 || previous.is_some_and(|v| a <= v) {
            return Err(invalid());
        }
        selected.extend(a..z);
        previous = Some(z);
    }
    Ok(Selection {
        original,
        source_handle,
        diff_id,
        selected,
    })
}
pub(crate) fn original_target(t: &Target) -> Result<Target, AppError> {
    Ok(decode(t)?.original)
}
pub(crate) struct PreparedCandidate {
    pub path: Vec<u8>,
    pub candidate: Candidate,
    pub mode: u32,
    pub attrs: AttributeGuard,
}
pub(crate) async fn prepare_candidates(
    q: &ReadQuery<'_>,
    intent: &ActionIntent,
    targets: &[Target],
    stamp: &str,
) -> Result<Vec<PreparedCandidate>, AppError> {
    if !matches!(
        intent,
        ActionIntent::StageSelection { .. } | ActionIntent::UnstageSelection { .. }
    ) {
        return Ok(vec![]);
    }
    let stage = matches!(intent, ActionIntent::StageSelection { .. });
    let mut files = BTreeSet::new();
    let mut result = vec![];
    for t in targets {
        let decoded = decode(t)?;
        let (side, paths) = changes::paths(&decoded.original)?;
        if stage != (side == ChangeSide::Unstaged) || !files.insert(paths[0].clone()) {
            return Err(invalid());
        }
        let data = capture(q, &decoded.original, &decoded.source_handle, stamp).await?;
        if data.diff_id != decoded.diff_id {
            return Err(changes::changed());
        }
        let candidate = data.candidate(&decoded.selected)?;
        result.push(PreparedCandidate {
            path: paths[0].clone(),
            candidate,
            mode: data.mode,
            attrs: data.attrs,
        });
    }
    Ok(result)
}
fn tokens(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').collect()
}
fn line_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|b| **b == b'\n').count()
        + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))
}
pub(crate) fn align(left: &[u8], right: &[u8]) -> Result<Alignment, Limit> {
    if left.len() > MAX_BYTES
        || right.len() > MAX_BYTES
        || line_count(left) + line_count(right) > MAX_ROWS
    {
        return Err(Limit::TooLarge);
    }
    let l = tokens(left);
    let r = tokens(right);
    let mut prefix = 0;
    while prefix < l.len().min(r.len()) && l[prefix] == r[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < l.len().min(r.len()) - prefix
        && l[l.len() - 1 - suffix] == r[r.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let a = &l[prefix..l.len() - suffix];
    let b = &r[prefix..r.len() - suffix];
    let cells = (a.len() + 1)
        .checked_mul(b.len() + 1)
        .filter(|n| *n <= MAX_CELLS)
        .ok_or(Limit::ComplexityLimit)?;
    let width = b.len() + 1;
    let mut table = vec![0u32; cells];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i] == b[j] {
                1 + table[(i + 1) * width + j + 1]
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut edits = Vec::new();
    let mut units = 0u32;
    let mut emit = |kind, bytes: &[u8]| {
        let ordinal = if kind == Kind::Equal {
            None
        } else {
            let v = units;
            units += 1;
            Some(v)
        };
        edits.push(Edit {
            kind,
            bytes: bytes.to_vec(),
            ordinal,
        });
    };
    for token in &l[..prefix] {
        emit(Kind::Equal, token);
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            emit(Kind::Equal, a[i]);
            i += 1;
            j += 1;
        } else if i < a.len()
            && (j == b.len() || table[(i + 1) * width + j] >= table[i * width + j + 1])
        {
            emit(Kind::Delete, a[i]);
            i += 1;
        } else {
            emit(Kind::Add, b[j]);
            j += 1;
        }
    }
    for token in &l[l.len() - suffix..] {
        emit(Kind::Equal, token);
    }
    Ok(Alignment { edits, units })
}
pub(crate) fn candidate(
    alignment: &Alignment,
    stage: bool,
    selected: &BTreeSet<u32>,
    left_exists: bool,
    right_exists: bool,
) -> Result<Candidate, CandidateError> {
    if selected.iter().any(|n| *n >= alignment.units) {
        return Err(CandidateError::InvalidSelection);
    }
    let mut bytes = Vec::new();
    let mut unterminated = false;
    for edit in &alignment.edits {
        let chosen = edit.ordinal.is_some_and(|n| selected.contains(&n));
        let emit = match (stage, edit.kind) {
            (_, Kind::Equal) => true,
            (true, Kind::Delete) => !chosen,
            (true, Kind::Add) => chosen,
            (false, Kind::Delete) => chosen,
            (false, Kind::Add) => !chosen,
        };
        if emit {
            if unterminated {
                return Err(CandidateError::Unterminated);
            }
            bytes.extend_from_slice(&edit.bytes);
            unterminated = !edit.bytes.ends_with(b"\n");
        }
    }
    let baseline = if stage { left_exists } else { right_exists };
    let target = if stage { right_exists } else { left_exists };
    let all = alignment.units > 0 && selected.len() == alignment.units as usize;
    let exists = if !bytes.is_empty() {
        true
    } else if all && !target {
        false
    } else {
        baseline || target && !selected.is_empty()
    };
    Ok(Candidate { bytes, exists })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_control_text_is_charged_to_structured_response_budget() {
        let mut d = data();
        let a = vec![1u8; 600_000];
        let b = vec![2u8; 600_000];
        d.alignment = Some(align(&a, &b).unwrap());
        assert!(!structured_within_limit(&d.hunks().unwrap()));
        assert!(structured_within_limit(&data().hunks().unwrap()));
    }
    fn data() -> PatchData {
        PatchData {
            side: ChangeSide::Unstaged,
            diff_id: "d".repeat(64),
            eligibility: PartialEligibility::Eligible,
            alignment: Some(align(b"old\n", b"new\n").unwrap()),
            left: Endpoint {
                bytes: b"old\n".to_vec(),
                exists: true,
                mode: 0o100644,
                oid: None,
            },
            right: Endpoint {
                bytes: b"new\n".to_vec(),
                exists: true,
                mode: 0o100644,
                oid: None,
            },
            attrs: AttributeGuard {
                paths: vec![],
                stamp: preflight::digest(b""),
            },
            mode: 0o100644,
        }
    }
    fn original() -> Target {
        let identity = b"\0\0file.txt\0".to_vec();
        Target {
            kind: TargetKind::Change,
            proof: Some(SafeTargetProof::Change {
                path_digest: preflight::digest(&identity),
                index_digest: "c".repeat(64),
            }),
            identity,
        }
    }
    #[test]
    fn source_expiry_is_inherited_without_renewal_and_reopened_window_never_adopts_selection() {
        use std::{
            sync::{Arc, Mutex},
            time::{Duration, Instant},
        };
        let now = Arc::new(Mutex::new(Instant::now()));
        let copy = now.clone();
        let authority =
            super::super::authority::Authority::with_clock(Arc::new(move || *copy.lock().unwrap()));
        let scope = ActionScope {
            repo_id: "repo".into(),
            worktree_id: Some("tree".into()),
            generation: "1".into(),
        };
        authority.open_window("main").unwrap();
        let epoch = authority.begin("main", &scope, || Ok(())).unwrap();
        let deadline = *now.lock().unwrap() + Duration::from_secs(5);
        let source = authority
            .issue_until(
                "main",
                &scope,
                epoch,
                original(),
                "e".repeat(64),
                Some(deadline),
                || Ok(()),
            )
            .unwrap();
        *now.lock().unwrap() += Duration::from_secs(2);
        let (target, expires) = authority
            .change_grant("main", &scope, epoch, &source, &"e".repeat(64))
            .unwrap();
        assert_eq!(expires, deadline);
        let selected =
            selection_target(&target, &source, &data(), &BTreeSet::from([0, 1])).unwrap();
        assert!(selected.identity.len() <= 2048);
        let handle = authority
            .issue_until(
                "main",
                &scope,
                epoch,
                selected,
                "e".repeat(64),
                Some(expires),
                || Ok(()),
            )
            .unwrap();
        *now.lock().unwrap() += Duration::from_secs(3);
        assert!(authority
            .resolved_targets(
                "main",
                &scope,
                epoch,
                &ActionIntent::StageSelection {
                    selection_handles: vec![handle.clone()]
                },
                &"e".repeat(64)
            )
            .is_err());
        assert!(authority
            .change_grant("main", &scope, epoch, &source, &"e".repeat(64))
            .is_err());
        authority.revoke("main", true);
        authority.open_window("main").unwrap();
        let fresh = authority.begin("main", &scope, || Ok(())).unwrap();
        let live = authority
            .issue("main", &scope, fresh, original(), "e".repeat(64), || Ok(()))
            .unwrap();
        assert!(authority
            .resolved_targets(
                "main",
                &scope,
                fresh,
                &ActionIntent::StageSelection {
                    selection_handles: vec![handle]
                },
                &"e".repeat(64)
            )
            .is_err());
        assert!(authority
            .change_grant("main", &scope, fresh, &live, &"e".repeat(64))
            .is_ok());
    }
    #[test]
    fn foreign_duplicate_fragmented_and_noncanonical_selection_ids_are_rejected_wholesale() {
        let data = data();
        let h = data.hunks().unwrap();
        let id = h[0].rows.iter().find_map(|r| r.line_id.clone()).unwrap();
        assert!(data
            .select(&ChangeSelectionInput::Lines {
                line_ids: vec![id.clone(), id]
            })
            .is_err());
        assert!(data
            .select(&ChangeSelectionInput::Lines {
                line_ids: vec![format!("line-{}-0", "f".repeat(64))]
            })
            .is_err());
        assert!(data
            .select(&ChangeSelectionInput::Hunks {
                hunk_ids: vec![h[0].hunk_id.clone(), h[0].hunk_id.clone()]
            })
            .is_err());
        assert!(canonical_ranges(&(0..130).step_by(2).collect()).is_err());
        let target =
            selection_target(&original(), &"a".repeat(64), &data, &BTreeSet::from([0, 1])).unwrap();
        assert_eq!(decode(&target).unwrap().selected, BTreeSet::from([0, 1]));
        let mut invalid = target.clone();
        invalid.identity.push(0);
        assert!(decode(&invalid).is_err());
        let mut invalid = target;
        let n = invalid.identity.len();
        invalid.identity[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&invalid).is_err());
    }
    #[test]
    fn every_small_edit_subset_matches_handwritten_stage_and_unstage_bytes() {
        let alignment = align(b"keep\nold\n", b"keep\nnew\nextra\n").unwrap();
        assert_eq!(alignment.units, 3);
        let stage = [
            "keep\nold\n",
            "keep\n",
            "keep\nold\nnew\n",
            "keep\nnew\n",
            "keep\nold\nextra\n",
            "keep\nextra\n",
            "keep\nold\nnew\nextra\n",
            "keep\nnew\nextra\n",
        ];
        let unstage = [
            "keep\nnew\nextra\n",
            "keep\nold\nnew\nextra\n",
            "keep\nextra\n",
            "keep\nold\nextra\n",
            "keep\nnew\n",
            "keep\nold\nnew\n",
            "keep\n",
            "keep\nold\n",
        ];
        for mask in 0..8 {
            let selected = (0..3)
                .filter(|n| mask & (1 << n) != 0)
                .collect::<BTreeSet<_>>();
            for (direction, expected) in [(true, stage[mask]), (false, unstage[mask])] {
                assert_eq!(
                    candidate(&alignment, direction, &selected, true, true).unwrap(),
                    Candidate {
                        bytes: expected.as_bytes().to_vec(),
                        exists: true
                    },
                    "mask {mask}, stage {direction}"
                );
            }
        }
    }
    #[test]
    fn repeated_line_occurrences_are_deletion_first_and_select_all_none_are_exact() {
        let a = align(b"a\nx\na\n", b"a\na\nx\n").unwrap();
        assert_eq!(
            a.edits.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![Kind::Equal, Kind::Delete, Kind::Equal, Kind::Add]
        );
        assert_eq!(
            candidate(&a, true, &BTreeSet::from([0]), true, true)
                .unwrap()
                .bytes,
            b"a\na\n"
        );
        assert_eq!(
            candidate(&a, true, &BTreeSet::from([1]), true, true)
                .unwrap()
                .bytes,
            b"a\nx\na\nx\n"
        );
        for (l, r) in [
            (b"".as_slice(), b"x".as_slice()),
            (b"a\nb", b"a\nb\nc"),
            (b"a\n", b""),
            (b"", b""),
        ] {
            let a = align(l, r).unwrap();
            let all = (0..a.units).collect();
            assert_eq!(candidate(&a, true, &all, true, true).unwrap().bytes, r);
            assert_eq!(candidate(&a, false, &all, true, true).unwrap().bytes, l);
            assert_eq!(
                candidate(&a, true, &BTreeSet::new(), true, true)
                    .unwrap()
                    .bytes,
                l
            );
            assert_eq!(
                candidate(&a, false, &BTreeSet::new(), true, true)
                    .unwrap()
                    .bytes,
                r
            );
        }
    }
    #[test]
    fn absence_is_distinct_from_empty_and_no_lf_is_never_repaired_silently() {
        let a = align(b"x\n", b"").unwrap();
        assert!(
            !candidate(&a, true, &BTreeSet::from([0]), true, false)
                .unwrap()
                .exists
        );
        assert!(
            candidate(&a, true, &BTreeSet::from([0]), true, true)
                .unwrap()
                .exists
        );
        let new = align(b"", b"x").unwrap();
        assert!(
            !candidate(&new, true, &BTreeSet::new(), false, true)
                .unwrap()
                .exists
        );
        assert!(
            !candidate(&new, false, &BTreeSet::from([0]), false, true)
                .unwrap()
                .exists
        );
        let a = align(b"a\nb", b"a\nb\nc").unwrap();
        assert_eq!(
            candidate(&a, true, &BTreeSet::from([2]), true, true),
            Err(CandidateError::Unterminated)
        );
        assert_eq!(
            candidate(&a, true, &BTreeSet::from([99]), true, true),
            Err(CandidateError::InvalidSelection)
        );
    }
    #[test]
    fn lcs_scratch_is_bounded_after_prefix_suffix_trim() {
        let l = (0..1100).map(|i| format!("L{i}\n")).collect::<String>();
        let r = (0..1100).map(|i| format!("R{i}\n")).collect::<String>();
        assert!(matches!(
            align(l.as_bytes(), r.as_bytes()),
            Err(Limit::ComplexityLimit)
        ));
        let prefix = "same\n".repeat(6000);
        assert!(align(
            format!("{prefix}x\n").as_bytes(),
            format!("{prefix}y\n").as_bytes()
        )
        .is_ok());
        assert!(matches!(
            align(&vec![b'x'; MAX_BYTES + 1], b""),
            Err(Limit::TooLarge)
        ));
    }
}
