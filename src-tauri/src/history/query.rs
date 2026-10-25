use super::{
    sessions::{Cursor, Session},
    *,
};
use crate::git::runner::{argv, Cancellation, GitOutput, RunOptions};
use crate::repositories::discovery;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
    ffi::OsString,
    io::Read,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const ORDER_BATCH: usize = 4096;
const ORDER_BATCH_BYTES: usize = 4 * 1024 * 1024;
const ORDER_METADATA_BYTES: usize = 32 * 1024 * 1024;
const ORDER_PREFLIGHT_BYTES: usize = 32 * 1024 * 1024;
const ORDER_RAW_BYTES: usize = 128 * 1024 * 1024;
const ORDER_PARENT_OCCURRENCES: usize = 300_000;
#[cfg(test)]
mod tests;

struct OrderControl {
    deadline: Instant,
    cancel: Cancellation,
}
impl OrderControl {
    fn check(&self) -> Result<(), AppError> {
        if self.cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Inspection));
        }
        if Instant::now() >= self.deadline {
            return Err(error(ErrorCode::Timeout, Phase::Inspection));
        }
        Ok(())
    }
    async fn cooperate(&self, step: usize) -> Result<(), AppError> {
        if step.is_multiple_of(1024) {
            self.check()?;
            tokio::task::yield_now().await;
            self.check()?;
        }
        Ok(())
    }
}
fn charge(total: &mut usize, amount: usize, maximum: usize) -> Result<(), AppError> {
    let next = total.checked_add(amount).ok_or_else(limit)?;
    if next > maximum {
        return Err(limit());
    }
    *total = next;
    Ok(())
}
struct ParentBudget {
    semantic: usize,
    wire: usize,
    parents: usize,
    preflight: usize,
    raw: usize,
    oid_length: usize,
}
impl ParentBudget {
    fn new(nodes: usize, oid_length: usize) -> Result<Self, AppError> {
        if nodes > HISTORY_COMMITS_PER_SESSION {
            return Err(limit());
        }
        if !matches!(oid_length, 40 | 64) {
            return Err(malformed());
        }
        let semantic = nodes.checked_mul(oid_length + 1).ok_or_else(limit)?;
        if semantic > ORDER_METADATA_BYTES {
            return Err(limit());
        }
        Ok(Self {
            semantic,
            wire: 0,
            parents: 0,
            preflight: 0,
            raw: 0,
            oid_length,
        })
    }
    fn parent(&mut self) -> Result<(), AppError> {
        charge(&mut self.parents, 1, ORDER_PARENT_OCCURRENCES)?;
        charge(
            &mut self.semantic,
            self.oid_length + 1,
            ORDER_METADATA_BYTES,
        )
    }
}
fn record_parent(
    parent: &str,
    index: &HashMap<&str, u32>,
    edges: &mut Vec<u32>,
    budget: &mut ParentBudget,
) -> Result<(), AppError> {
    if !oid(parent, budget.oid_length) {
        return Err(malformed());
    }
    budget.parent()?;
    if let Some(parent) = index.get(parent) {
        edges.push(*parent);
    }
    Ok(())
}

async fn stable_induced_order(
    original: &[String],
    index: &HashMap<&str, u32>,
    parents: &[Vec<u32>],
    control: &OrderControl,
) -> Result<Vec<String>, AppError> {
    control.check()?;
    if parents.len() > HISTORY_COMMITS_PER_SESSION || original.len() > HISTORY_COMMITS_PER_SESSION {
        return Err(limit());
    }
    let mut ranks = vec![u32::MAX; parents.len()];
    let mut nodes = Vec::with_capacity(original.len());
    for (rank, oid) in original.iter().enumerate() {
        control.cooperate(rank).await?;
        let node = *index.get(oid.as_str()).ok_or_else(malformed)?;
        let slot = ranks.get_mut(node as usize).ok_or_else(malformed)?;
        if *slot != u32::MAX {
            return Err(malformed());
        }
        *slot = u32::try_from(rank).map_err(|_| limit())?;
        nodes.push(node);
    }
    let mut incoming = vec![0u32; parents.len()];
    let mut steps = 0usize;
    for node in &nodes {
        control.cooperate(steps).await?;
        steps = steps.checked_add(1).ok_or_else(limit)?;
        for parent in parents.get(*node as usize).ok_or_else(malformed)? {
            control.cooperate(steps).await?;
            steps = steps.checked_add(1).ok_or_else(limit)?;
            if *ranks.get(*parent as usize).ok_or_else(malformed)? != u32::MAX {
                let degree = incoming.get_mut(*parent as usize).ok_or_else(malformed)?;
                *degree = degree.checked_add(1).ok_or_else(limit)?;
            }
        }
    }
    let mut ready = BinaryHeap::new();
    for (rank, node) in nodes.iter().enumerate() {
        control.cooperate(rank).await?;
        if incoming[*node as usize] == 0 {
            ready.push(Reverse(rank));
        }
    }
    let mut result = Vec::with_capacity(original.len());
    while let Some(Reverse(rank)) = ready.pop() {
        control.cooperate(steps).await?;
        steps = steps.checked_add(1).ok_or_else(limit)?;
        result.push(original[rank].clone());
        for parent in &parents[nodes[rank] as usize] {
            control.cooperate(steps).await?;
            steps = steps.checked_add(1).ok_or_else(limit)?;
            let parent_rank = *ranks.get(*parent as usize).ok_or_else(malformed)?;
            if parent_rank != u32::MAX {
                let degree = incoming.get_mut(*parent as usize).ok_or_else(malformed)?;
                *degree = degree.checked_sub(1).ok_or_else(malformed)?;
                if *degree == 0 {
                    ready.push(Reverse(parent_rank as usize));
                }
            }
        }
    }
    control.check()?;
    if result.len() != original.len() {
        return Err(malformed());
    }
    Ok(result)
}

#[derive(Clone, PartialEq)]
pub(crate) struct Fingerprint {
    pub head: Head,
    pub refs: Vec<HistoryRef>,
    pub shallow: Vec<String>,
    shallow_identity: Option<Vec<u8>>,
    raw_refs: Vec<u8>,
    stash_log: Vec<u8>,
    head_bytes: Vec<u8>,
    link_bytes: Vec<u8>,
    git_identity: Vec<u8>,
    pub oid_length: usize,
}
#[cfg(windows)]
struct ShallowLease {
    file: std::fs::File,
    path: std::path::PathBuf,
    identity: Vec<u8>,
    bytes: Vec<u8>,
}
#[cfg(windows)]
struct Pins {
    directories: Vec<crate::clone::path::StableDirectory>,
    shallow: Option<ShallowLease>,
}
#[cfg(not(windows))]
struct Pins;
impl Pins {
    fn new(location: &RepositoryLocation) -> Result<Self, AppError> {
        #[cfg(windows)]
        {
            let mut pins = Vec::new();
            for path in [&location.root, &location.common, &location.git_dir] {
                if !pins
                    .iter()
                    .any(|pin: &crate::clone::path::StableDirectory| pin.path == *path)
                {
                    pins.push(crate::clone::path::StableDirectory::open(path)?);
                }
            }
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            };
            let path = location.common.join("shallow");
            let shallow = match std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(&path)
            {
                Ok(mut file) => {
                    let info = crate::clone::path::handle_info(&file)?;
                    if info.dwFileAttributes
                        & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)
                        != 0
                    {
                        return Err(external());
                    }
                    let identity = crate::clone::path::info_identity(&info);
                    if discovery::identity(&path)? != identity {
                        return Err(external());
                    }
                    let mut bytes = Vec::new();
                    (&mut file)
                        .take(HISTORY_RESPONSE_BYTES as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|_| external())?;
                    if bytes.len() > HISTORY_RESPONSE_BYTES {
                        return Err(limit());
                    }
                    // Never retain arbitrary unvalidated metadata in quarantine.
                    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                        if !matches!(line.len(), 40 | 64) || !line.iter().all(u8::is_ascii_hexdigit)
                        {
                            return Err(malformed());
                        }
                    }
                    Some(ShallowLease {
                        file,
                        path,
                        identity,
                        bytes,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => return Err(external()),
            };
            Ok(Self {
                directories: pins,
                shallow,
            })
        }
        #[cfg(not(windows))]
        {
            if location.common.join("shallow").exists() {
                return Err(AppError {
                    code: ErrorCode::InvalidRequest,
                    phase: Phase::Inspection,
                    retry: AppErrorRetry::None,
                    message: "Safe shallow history is unavailable on this platform.".into(),
                });
            }
            Ok(Self)
        }
    }
    fn recheck(&self) -> Result<(), AppError> {
        #[cfg(windows)]
        for pin in &self.directories {
            pin.recheck()?;
        }
        #[cfg(windows)]
        if let Some(lease) = &self.shallow {
            if crate::clone::path::info_identity(&crate::clone::path::handle_info(&lease.file)?)
                != lease.identity
                || discovery::identity(&lease.path)? != lease.identity
            {
                return Err(external());
            }
        }
        Ok(())
    }
    fn shallow_argument(&self) -> Result<OsString, AppError> {
        #[cfg(windows)]
        {
            if let Some(lease) = &self.shallow {
                return git_argument_path(&lease.path);
            }
            Ok("NUL".into())
        }
        #[cfg(not(windows))]
        {
            Ok("/dev/null".into())
        }
    }
    fn shallow_bytes(&self) -> &[u8] {
        #[cfg(windows)]
        {
            self.shallow
                .as_ref()
                .map(|lease| lease.bytes.as_slice())
                .unwrap_or_default()
        }
        #[cfg(not(windows))]
        {
            &[]
        }
    }
    fn shallow_identity(&self) -> Option<Vec<u8>> {
        #[cfg(windows)]
        {
            self.shallow.as_ref().map(|lease| lease.identity.clone())
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}
pub(crate) struct Query<'a> {
    runner: &'a GitRunner,
    location: RepositoryLocation,
    deadline: Instant,
    pins: Arc<Pins>,
    cancel: Cancellation,
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn missing() -> AppError {
    error(ErrorCode::NotFound, Phase::Inspection)
}
fn unsupported() -> AppError {
    AppError {
        code: ErrorCode::InvalidRequest,
        phase: Phase::Inspection,
        retry: AppErrorRetry::None,
        message: "History is unavailable for replacement refs, grafts, or partial clones.".into(),
    }
}
fn git_argument_path(path: &Path) -> Result<OsString, AppError> {
    #[cfg(windows)]
    {
        crate::clone::path::git_path(path)
    }
    #[cfg(not(windows))]
    {
        Ok(path.as_os_str().to_owned())
    }
}
pub(crate) fn oid(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
pub(crate) fn read_metadata(path: &Path, max: usize) -> Result<Vec<u8>, AppError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(error(ErrorCode::Io, Phase::Inspection)),
    };
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error(ErrorCode::Io, Phase::Inspection))?;
    if bytes.len() > max {
        Err(limit())
    } else {
        Ok(bytes)
    }
}
impl<'a> Query<'a> {
    pub async fn new(
        runner: &'a GitRunner,
        location: &RepositoryLocation,
        cancel: Cancellation,
    ) -> Result<Self, AppError> {
        let pins = Arc::new(Pins::new(location)?);
        let query = Self {
            runner,
            location: location.clone(),
            deadline: Instant::now() + Duration::from_secs(30),
            pins,
            cancel,
        };
        query.identities()?;
        // Fixed trusted --git-dir/--work-tree arguments keep every subprocess
        // scoped even if an external tool rewrites a linked worktree .git file.
        query.verify_location().await?;
        Ok(query)
    }
    async fn verify_location(&self) -> Result<usize, AppError> {
        let output = self
            .execute(
                &argv(&[
                    "rev-parse",
                    "--path-format=absolute",
                    "--absolute-git-dir",
                    "--git-common-dir",
                    "--show-object-format",
                ]),
                None,
                false,
            )
            .await?;
        if !output.success {
            return Err(external());
        }
        let text = std::str::from_utf8(&output.stdout).map_err(|_| malformed())?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() != 3 {
            return Err(malformed());
        }
        let paths = lines[..2]
            .iter()
            .map(|line| std::fs::canonicalize(line).map_err(|_| external()))
            .collect::<Result<Vec<_>, _>>()?;
        if paths.len() != 2 || paths[0] != self.location.git_dir || paths[1] != self.location.common
        {
            return Err(external());
        }
        match lines[2] {
            "sha1" => Ok(40),
            "sha256" => Ok(64),
            _ => Err(malformed()),
        }
    }
    fn identities(&self) -> Result<Vec<u8>, AppError> {
        self.pins.recheck()?;
        if discovery::identity(&self.location.common)? != self.location.common_identity
            || self
                .location
                .root_identity
                .as_ref()
                .is_some_and(|expected| {
                    discovery::identity(&self.location.root).as_ref() != Ok(expected)
                })
        {
            return Err(external());
        }
        discovery::identity(&self.location.git_dir)
    }
    pub async fn output(
        &self,
        args: &[OsString],
        input: Option<&[u8]>,
    ) -> Result<GitOutput, AppError> {
        self.execute(args, input, true).await
    }
    async fn execute(
        &self,
        args: &[OsString],
        input: Option<&[u8]>,
        scoped: bool,
    ) -> Result<GitOutput, AppError> {
        self.identities()?;
        if self.cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Inspection));
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(error(ErrorCode::Timeout, Phase::Inspection));
        }
        let mut runner = self.runner.clone();
        runner.timeout = runner.timeout.min(remaining);
        runner.output_limit = runner.output_limit.min(HISTORY_RESPONSE_BYTES);
        let mut command = argv(&[
            "--no-lazy-fetch",
            "--no-replace-objects",
            "--no-pager",
            "-c",
            "core.commitGraph=false",
            "-c",
            "advice.graftFileDeprecated=false",
        ]);
        command.push("--shallow-file".into());
        command.push(self.pins.shallow_argument()?);
        if scoped {
            let mut git_dir = OsString::from("--git-dir=");
            git_dir.push(git_argument_path(&self.location.git_dir)?);
            command.push(git_dir);
            if !self.location.bare {
                let mut root = OsString::from("--work-tree=");
                root.push(git_argument_path(&self.location.root)?);
                command.push(root);
            }
        }
        command.extend_from_slice(args);
        let cancel = Cancellation::default();
        let running = runner.output_history(
            Some(&self.location.root),
            &command,
            &cancel,
            Phase::Inspection,
            RunOptions {
                environment: vec![
                    ("GIT_ALLOW_PROTOCOL".into(), "".into()),
                    ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
                    ("GIT_NO_LAZY_FETCH".into(), "1".into()),
                ],
                lifetime_guard: Some(self.pins.clone()),
                ..Default::default()
            },
            input,
        );
        tokio::pin!(running);
        tokio::select! { value = &mut running => value, _ = async { while !self.cancel.is_cancelled() { tokio::time::sleep(Duration::from_millis(5)).await; } } => { cancel.cancel(); running.await } }
    }
    pub async fn run(&self, args: &[OsString], input: Option<&[u8]>) -> Result<Vec<u8>, AppError> {
        let output = self.output(args, input).await?;
        if output.success {
            if args.first().is_some_and(|a| a == "for-each-ref") && !output.stderr.is_empty() {
                return Err(malformed());
            }
            Ok(output.stdout)
        } else {
            let mut failure = missing();
            failure.message = match args.first().and_then(|a| a.to_str()) {
                Some("rev-list") => "Git could not traverse the requested history.",
                Some("for-each-ref") => "Git could not read repository references.",
                Some("rev-parse") => "Git could not read repository history metadata.",
                _ => "A required Git object is unavailable.",
            }
            .into();
            Err(failure)
        }
    }
    pub async fn fingerprint(&self) -> Result<Fingerprint, AppError> {
        let oid_length = self.verify_location().await?;
        let git_identity = self.identities()?;
        if !read_metadata(
            &self.location.common.join("info/grafts"),
            HISTORY_MESSAGE_BYTES,
        )?
        .is_empty()
        {
            return Err(unsupported());
        }
        for directory in [self.location.common.join("objects/pack")] {
            match std::fs::read_dir(directory) {
                Ok(entries) => {
                    for (i, item) in entries.enumerate() {
                        if i >= HISTORY_REFS_MAX {
                            return Err(limit());
                        }
                        if item
                            .map_err(|_| malformed())?
                            .path()
                            .extension()
                            .is_some_and(|v| v == "promisor")
                        {
                            return Err(unsupported());
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(_) => return Err(error(ErrorCode::Io, Phase::Inspection)),
            }
        }
        let config = self
            .output(
                &argv(&[
                    "config",
                    "--null",
                    "--get-regexp",
                    "^(extensions\\.partialclone|remote\\..*\\.(promisor|partialclonefilter))$",
                ]),
                None,
            )
            .await?;
        if !config.success && (!config.stdout.is_empty() || !config.stderr.is_empty()) {
            return Err(unsupported());
        }
        for record in config
            .stdout
            .split(|b| *b == 0)
            .filter(|part| !part.is_empty())
        {
            let text = std::str::from_utf8(record).map_err(|_| malformed())?;
            let (key, value) = text.split_once('\n').ok_or_else(malformed)?;
            if !key.ends_with(".promisor")
                || !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "false" | "no" | "off" | "0"
                )
            {
                return Err(unsupported());
            }
        }
        let raw_refs = self.run(&argv(&["for-each-ref", "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(symref)%00"]), None).await?;
        let mut refs = Vec::new();
        let mut ref_oids = HashMap::new();
        let mut count = 0;
        for line in raw_refs
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
        {
            count += 1;
            if count > HISTORY_REFS_MAX {
                return Err(limit());
            }
            let fields: Vec<_> = line.split(|b| *b == 0).collect();
            if fields.len() != 7 || !fields[6].is_empty() {
                return Err(malformed());
            }
            let fields: Vec<_> = fields[..6]
                .iter()
                .map(|v| std::str::from_utf8(v).map_err(|_| malformed()))
                .collect::<Result<_, _>>()?;
            let name = fields[0];
            if name.len() > 1024 {
                return Err(limit());
            }
            if !refs::valid_name(name) || !oid(fields[1], oid_length) {
                return Err(malformed());
            }
            if name.starts_with("refs/replace/") {
                return Err(unsupported());
            }
            ref_oids.insert(name.to_owned(), fields[1].to_owned());
            let (kind, display) = if let Some(name) = name.strip_prefix("refs/heads/") {
                (HistoryRefKind::Local, name)
            } else if let Some(name) = name.strip_prefix("refs/remotes/") {
                (HistoryRefKind::Remote, name)
            } else if let Some(name) = name.strip_prefix("refs/tags/") {
                (HistoryRefKind::Tag, name)
            } else if name == "refs/stash" {
                (HistoryRefKind::Stash, "stash")
            } else {
                continue;
            };
            let commit_oid = if fields[2] == "commit" {
                Some(fields[1].into())
            } else if fields[4] == "commit" && oid(fields[3], oid_length) {
                Some(fields[3].into())
            } else if fields[2] == "tag" && fields[4] == "tag" {
                let peeled = self
                    .run(
                        &argv(&["rev-parse", "--verify", &format!("{}^{{}}", fields[1])]),
                        None,
                    )
                    .await?;
                let peeled = std::str::from_utf8(&peeled)
                    .map_err(|_| malformed())?
                    .trim_end_matches(['\r', '\n']);
                if !oid(peeled, oid_length) {
                    return Err(malformed());
                }
                let kind = self.run(&argv(&["cat-file", "-t", peeled]), None).await?;
                if kind == b"commit\n" {
                    Some(peeled.into())
                } else {
                    None
                }
            } else {
                None
            };
            let symbolic_target = if fields[5].is_empty() {
                None
            } else if refs::valid_name(fields[5]) {
                Some(fields[5].into())
            } else {
                return Err(malformed());
            };
            refs.push(HistoryRef {
                name: name.into(),
                display_name: display.into(),
                kind,
                oid: fields[1].into(),
                commit_oid,
                symbolic_target,
                hidden: false,
            });
        }
        let stash_log = refs::append_stash_stack(self, &mut refs, oid_length).await?;
        let head_bytes = read_metadata(&self.location.git_dir.join("HEAD"), HISTORY_MESSAGE_BYTES)?;
        let link_bytes = if self.location.root.join(".git").is_file() {
            read_metadata(&self.location.root.join(".git"), HISTORY_MESSAGE_BYTES)?
        } else {
            Vec::new()
        };
        if head_bytes.is_empty() {
            return Err(missing());
        }
        // HEAD's loose file is only a compatibility stub with reftable storage.
        // Resolve its symbolic identity through Git, then use captured ref OIDs.
        let symbolic = self
            .output(&argv(&["symbolic-ref", "--quiet", "HEAD"]), None)
            .await?;
        if !symbolic.stderr.is_empty() {
            return Err(malformed());
        }
        let (branch, head_oid) = if symbolic.success {
            let reference = std::str::from_utf8(&symbolic.stdout)
                .map_err(|_| malformed())?
                .trim_end_matches(['\r', '\n']);
            if !refs::valid_name(reference) {
                return Err(malformed());
            }
            (
                Some(
                    reference
                        .strip_prefix("refs/heads/")
                        .unwrap_or(reference)
                        .to_owned(),
                ),
                ref_oids.get(reference).cloned(),
            )
        } else {
            if symbolic.exit_code != Some(1) || !symbolic.stdout.is_empty() {
                return Err(malformed());
            }
            let resolved = self
                .run(&argv(&["rev-parse", "--verify", "HEAD"]), None)
                .await?;
            let resolved = std::str::from_utf8(&resolved)
                .map_err(|_| malformed())?
                .trim_end_matches(['\r', '\n']);
            if !oid(resolved, oid_length) {
                return Err(malformed());
            }
            (None, Some(resolved.to_owned()))
        };
        let head = Head {
            kind: if head_oid.is_none() {
                HeadKind::Unborn
            } else if branch.is_some() {
                HeadKind::Branch
            } else {
                HeadKind::Detached
            },
            branch,
            oid: head_oid,
        };
        let shallow_bytes = read_metadata(
            &self.location.common.join("shallow"),
            HISTORY_RESPONSE_BYTES,
        )?;
        if shallow_bytes != self.pins.shallow_bytes() {
            return Err(external());
        }
        let mut shallow = Vec::new();
        for line in shallow_bytes
            .split(|b| *b == b'\n')
            .filter(|v| !v.is_empty())
        {
            let text = std::str::from_utf8(line).map_err(|_| malformed())?;
            if !oid(text, oid_length) {
                return Err(malformed());
            }
            shallow.push(text.into());
        }
        shallow.sort();
        shallow.dedup();
        Ok(Fingerprint {
            head,
            refs,
            shallow,
            shallow_identity: self.pins.shallow_identity(),
            raw_refs,
            stash_log,
            head_bytes,
            link_bytes,
            git_identity,
            oid_length,
        })
    }
    pub async fn order(&self, roots: &[String]) -> Result<Vec<String>, AppError> {
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let input = roots.iter().map(|s| format!("{s}\n")).collect::<String>();
        let bytes = self
            .run(
                &argv(&["rev-list", "--topo-order", "--stdin"]),
                Some(input.as_bytes()),
            )
            .await?;
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for line in bytes.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            let oid = std::str::from_utf8(line).map_err(|_| malformed())?;
            if !matches!(oid.len(), 40 | 64)
                || !self::oid(oid, oid.len())
                || !seen.insert(oid.to_owned())
            {
                return Err(malformed());
            }
            if result.len() >= HISTORY_COMMITS_PER_SESSION {
                return Err(limit());
            }
            result.push(oid.to_owned());
        }
        Ok(result)
    }
    pub async fn objects(&self, oids: &[String]) -> Result<Vec<RawCommit>, AppError> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let input = oids
            .iter()
            .map(|oid| format!("{oid}\n"))
            .collect::<String>();
        let bytes = self
            .run(&argv(&["cat-file", "--batch"]), Some(input.as_bytes()))
            .await?;
        let mut offset = 0;
        let mut result = Vec::new();
        for expected in oids {
            let end = bytes[offset..]
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(malformed)?
                + offset;
            let header = std::str::from_utf8(&bytes[offset..end]).map_err(|_| malformed())?;
            if header.ends_with(" missing") {
                return Err(missing());
            }
            let fields: Vec<_> = header.split(' ').collect();
            if fields.len() != 3 || fields[0] != expected || fields[1] != "commit" {
                return Err(malformed());
            }
            let size: usize = fields[2].parse().map_err(|_| malformed())?;
            let start = end + 1;
            let end = start.checked_add(size).ok_or_else(limit)?;
            if end >= bytes.len() || bytes[end] != b'\n' {
                return Err(malformed());
            }
            result.push(parse_commit(expected, &bytes[start..end])?);
            offset = end + 1;
        }
        if offset != bytes.len() {
            return Err(malformed());
        }
        Ok(result)
    }
    async fn ordering_output(
        &self,
        args: &[OsString],
        input: &[u8],
        control: &OrderControl,
    ) -> Result<Vec<u8>, AppError> {
        control.check()?;
        let output = self.output(args, Some(input)).await?;
        if !output.success {
            return Err(missing());
        }
        if !output.stderr.is_empty() {
            return Err(malformed());
        }
        control.check()?;
        Ok(output.stdout)
    }
    pub async fn normalize_orders(
        &self,
        visible: Vec<String>,
        all: Vec<String>,
        shallow: &[String],
        oid_length: usize,
    ) -> Result<(Vec<String>, Vec<String>), AppError> {
        let control = OrderControl {
            deadline: self.deadline,
            cancel: self.cancel.clone(),
        };
        control.check()?;
        // Preserve the ordinary rev-list fast path and its ranks exactly.
        if shallow.is_empty() {
            return Ok((visible, all));
        }
        let mut budget = ParentBudget::new(all.len(), oid_length)?;
        let mut index = HashMap::with_capacity(all.len());
        for (rank, child) in all.iter().enumerate() {
            control.cooperate(rank).await?;
            if !oid(child, oid_length)
                || index
                    .insert(child.as_str(), u32::try_from(rank).map_err(|_| limit())?)
                    .is_some()
            {
                return Err(malformed());
            }
        }
        let cuts: HashSet<_> = shallow.iter().map(String::as_str).collect();
        let mut sizes = Vec::with_capacity(all.len());
        for chunk in all.chunks(ORDER_BATCH) {
            let input = chunk.iter().map(|id| format!("{id}\n")).collect::<String>();
            let output = self
                .ordering_output(
                    &argv(&["cat-file", "--batch-check"]),
                    input.as_bytes(),
                    &control,
                )
                .await?;
            charge(&mut budget.preflight, output.len(), ORDER_PREFLIGHT_BYTES)?;
            let text = std::str::from_utf8(&output).map_err(|_| malformed())?;
            if !text.ends_with('\n') {
                return Err(malformed());
            }
            let mut lines = text.split_terminator('\n');
            for expected in chunk {
                control.cooperate(sizes.len()).await?;
                let line = lines.next().ok_or_else(malformed)?;
                if line == format!("{expected} missing") {
                    return Err(missing());
                }
                let fields: Vec<_> = line.splitn(4, ' ').collect();
                if fields.len() != 3
                    || fields[0] != expected
                    || fields[1] != "commit"
                    || fields[2].is_empty()
                    || !fields[2].bytes().all(|b| b.is_ascii_digit())
                {
                    return Err(malformed());
                }
                sizes.push(fields[2].parse::<usize>().map_err(|_| limit())?);
            }
            if lines.next().is_some() {
                return Err(malformed());
            }
        }
        let mut graph = vec![Vec::new(); all.len()];
        let mut work = 0usize;
        let mut seen = vec![false; all.len()];
        let mut start = 0usize;
        while start < all.len() {
            control.check()?;
            let mut end = start;
            let mut estimated = 0usize;
            while end < all.len() && end - start < ORDER_BATCH {
                let size = sizes[end].max(oid_length + 1);
                let next = estimated.checked_add(size).ok_or_else(limit)?;
                if end > start && next > ORDER_BATCH_BYTES {
                    break;
                }
                estimated = next;
                end = end.checked_add(1).ok_or_else(limit)?;
            }
            let input = all[start..end]
                .iter()
                .map(|id| format!("{id}\n"))
                .collect::<String>();
            let output = self
                .ordering_output(
                    &argv(&["rev-list", "--no-walk=unsorted", "--parents", "--stdin"]),
                    input.as_bytes(),
                    &control,
                )
                .await?;
            charge(&mut budget.wire, output.len(), ORDER_METADATA_BYTES)?;
            let text = std::str::from_utf8(&output).map_err(|_| malformed())?;
            if !text.ends_with('\n') {
                return Err(malformed());
            }
            for line in text.split_terminator('\n') {
                control.cooperate(work).await?;
                work = work.checked_add(1).ok_or_else(limit)?;
                let mut fields = line.split(' ');
                let child = fields.next().ok_or_else(malformed)?;
                let node = *index.get(child).ok_or_else(malformed)? as usize;
                if node < start || node >= end || seen[node] {
                    return Err(malformed());
                }
                seen[node] = true;
                let cut = cuts.contains(child);
                for parent in fields {
                    control.cooperate(work).await?;
                    work = work.checked_add(1).ok_or_else(limit)?;
                    if !oid(parent, oid_length) {
                        return Err(malformed());
                    }
                    if !cut {
                        record_parent(parent, &index, &mut graph[node], &mut budget)?;
                    }
                }
            }
            if seen[start..end].iter().any(|seen| !*seen) {
                return Err(malformed());
            }
            start = end;
        }
        // Only captured cut children are read raw. Their outside-set parents
        // count against limits but are never resolved, fetched or traversed.
        let cut_nodes: Vec<_> = all
            .iter()
            .enumerate()
            .filter_map(|(i, id)| cuts.contains(id.as_str()).then_some(i))
            .collect();
        let mut start = 0usize;
        while start < cut_nodes.len() {
            control.check()?;
            let mut end = start;
            let mut expected_bytes = 0usize;
            while end < cut_nodes.len() && end - start < ORDER_BATCH {
                let node = cut_nodes[end];
                let framed = sizes[node]
                    .checked_add(format!("{} commit {}\n", all[node], sizes[node]).len())
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(limit)?;
                if framed > HISTORY_RESPONSE_BYTES {
                    return Err(limit());
                }
                let next = expected_bytes.checked_add(framed).ok_or_else(limit)?;
                if end > start && next > ORDER_BATCH_BYTES {
                    break;
                }
                expected_bytes = next;
                end = end.checked_add(1).ok_or_else(limit)?;
            }
            if budget.raw.checked_add(expected_bytes).ok_or_else(limit)? > ORDER_RAW_BYTES {
                return Err(limit());
            }
            let input = cut_nodes[start..end]
                .iter()
                .map(|i| format!("{}\n", all[*i]))
                .collect::<String>();
            let output = self
                .ordering_output(&argv(&["cat-file", "--batch"]), input.as_bytes(), &control)
                .await?;
            charge(&mut budget.raw, output.len(), ORDER_RAW_BYTES)?;
            let mut offset = 0usize;
            for node in &cut_nodes[start..end] {
                control.cooperate(work).await?;
                work = work.checked_add(1).ok_or_else(limit)?;
                let tail = output.get(offset..).ok_or_else(malformed)?;
                let header_end = tail
                    .iter()
                    .position(|b| *b == b'\n')
                    .ok_or_else(malformed)?;
                let header = &tail[..header_end];
                if header == format!("{} missing", all[*node]).as_bytes() {
                    return Err(missing());
                }
                if header != format!("{} commit {}", all[*node], sizes[*node]).as_bytes() {
                    return Err(malformed());
                }
                let content = offset
                    .checked_add(header_end)
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(limit)?;
                let stop = content.checked_add(sizes[*node]).ok_or_else(limit)?;
                if output.get(stop) != Some(&b'\n') {
                    return Err(malformed());
                }
                let bytes = output.get(content..stop).ok_or_else(malformed)?;
                offset = stop.checked_add(1).ok_or_else(limit)?;
                let split = bytes
                    .windows(2)
                    .position(|b| b == b"\n\n")
                    .ok_or_else(malformed)?;
                let mut tree = false;
                for line in bytes[..split].split(|b| *b == b'\n') {
                    control.cooperate(work).await?;
                    work = work.checked_add(1).ok_or_else(limit)?;
                    if let Some(value) = line.strip_prefix(b"tree ") {
                        if tree || !std::str::from_utf8(value).is_ok_and(|v| oid(v, oid_length)) {
                            return Err(malformed());
                        }
                        tree = true;
                    }
                    if let Some(value) = line.strip_prefix(b"parent ") {
                        let parent = std::str::from_utf8(value).map_err(|_| malformed())?;
                        if !oid(parent, oid_length) {
                            return Err(malformed());
                        }
                        record_parent(parent, &index, &mut graph[*node], &mut budget)?;
                    }
                }
                if !tree {
                    return Err(malformed());
                }
            }
            if offset != output.len() {
                return Err(malformed());
            }
            start = end;
        }
        control.check()?;
        let normalized_visible = stable_induced_order(&visible, &index, &graph, &control).await?;
        let normalized_all = stable_induced_order(&all, &index, &graph, &control).await?;
        Ok((normalized_visible, normalized_all))
    }
    pub async fn search_batch(&self, oids: &[String]) -> Result<Vec<RawCommit>, AppError> {
        let oids = &oids[..oids.len().min(4096)];
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let input = oids
            .iter()
            .map(|oid| format!("{oid}\n"))
            .collect::<String>();
        let sizes = self
            .run(
                &argv(&["cat-file", "--batch-check"]),
                Some(input.as_bytes()),
            )
            .await?;
        let lines: Vec<_> = sizes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .collect();
        if lines.len() != oids.len() {
            return Err(malformed());
        }
        let mut bytes = 0usize;
        let mut count = 0;
        for (line, expected) in lines.into_iter().zip(oids) {
            let line = std::str::from_utf8(line).map_err(|_| malformed())?;
            if line.ends_with(" missing") {
                return Err(missing());
            }
            let fields: Vec<_> = line.split(' ').collect();
            if fields.len() != 3
                || fields[0] != expected
                || fields[1] != "commit"
                || !fields[2].bytes().all(|b| b.is_ascii_digit())
            {
                return Err(malformed());
            }
            let size: usize = fields[2].parse().map_err(|_| limit())?;
            let size = size.checked_add(line.len() + 2).ok_or_else(limit)?;
            if size > HISTORY_RESPONSE_BYTES {
                return Err(limit());
            }
            if count > 0 && bytes.saturating_add(size) > 4 * 1024 * 1024 {
                break;
            }
            bytes += size;
            count += 1;
        }
        self.objects(&oids[..count]).await
    }
    pub async fn page(
        &self,
        session: &mut Session,
        start: usize,
        size: usize,
    ) -> Result<HistoryPage, AppError> {
        if start > session.order.len() {
            return Err(invalid_handle());
        }
        let end = (start + size).min(session.order.len());
        let commits = self.objects(&session.order[start..end]).await?;
        let commits = commits
            .into_iter()
            .map(|raw| summary(session, raw, end))
            .collect::<Result<_, _>>()?;
        let next_cursor = if end < session.order.len() {
            Some(session.cursor(Cursor::Page { start: end, size })?)
        } else {
            None
        };
        Ok(HistoryPage {
            context: session.context.clone(),
            session_id: session.id.clone(),
            commits,
            next_cursor,
        })
    }
}
pub(crate) struct RawCommit {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: Person,
    pub committed_at: String,
    pub message: String,
    pub committer_name: String,
    pub committer_email: String,
}
pub(crate) struct Person {
    pub name: String,
    pub email: String,
    pub timestamp: String,
}
fn person(value: &str) -> Result<Person, AppError> {
    let (identity, date) = value.rsplit_once("> ").ok_or_else(malformed)?;
    let (name, email) = identity.rsplit_once(" <").ok_or_else(malformed)?;
    let mut date = date.split(' ');
    let unix: i64 = date
        .next()
        .ok_or_else(malformed)?
        .parse()
        .map_err(|_| malformed())?;
    let zone = date.next().ok_or_else(malformed)?;
    if date.next().is_some()
        || zone.len() != 5
        || !matches!(zone.as_bytes()[0], b'+' | b'-')
        || !zone.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(malformed());
    }
    let timestamp = chrono::DateTime::from_timestamp(unix, 0)
        .ok_or_else(malformed)?
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok(Person {
        name: name.into(),
        email: email.into(),
        timestamp,
    })
}
fn parse_commit(oid: &str, bytes: &[u8]) -> Result<RawCommit, AppError> {
    let split = bytes
        .windows(2)
        .position(|v| v == b"\n\n")
        .ok_or_else(malformed)?;
    let headers = std::str::from_utf8(&bytes[..split]).map_err(|_| malformed())?;
    let body = &bytes[split + 2..];
    if body.len() > HISTORY_MESSAGE_BYTES {
        return Err(limit());
    }
    let message = std::str::from_utf8(body)
        .map_err(|_| malformed())?
        .to_string();
    let (mut tree, mut author, mut committer) = (None, None, None);
    let mut parents = Vec::new();
    for line in headers.lines() {
        if line.starts_with(' ') {
            continue;
        }
        if let Some(value) = line.strip_prefix("tree ") {
            if tree.replace(value).is_some() || !self::oid(value, oid.len()) {
                return Err(malformed());
            }
        } else if let Some(value) = line.strip_prefix("parent ") {
            if !self::oid(value, oid.len()) {
                return Err(malformed());
            }
            parents.push(value.into());
        } else if let Some(value) = line.strip_prefix("author ") {
            if author.is_some() {
                return Err(malformed());
            }
            author = Some(person(value)?);
        } else if let Some(value) = line.strip_prefix("committer ") {
            if committer.is_some() {
                return Err(malformed());
            }
            committer = Some(person(value)?);
        }
    }
    if tree.is_none() {
        return Err(malformed());
    }
    let committer = committer.ok_or_else(malformed)?;
    Ok(RawCommit {
        oid: oid.into(),
        parents,
        author: author.ok_or_else(malformed)?,
        committed_at: committer.timestamp,
        committer_name: committer.name,
        committer_email: committer.email,
        message,
    })
}
pub(crate) fn summary(
    session: &mut Session,
    raw: RawCommit,
    fence: usize,
) -> Result<CommitSummary, AppError> {
    let parents = raw
        .parents
        .into_iter()
        .map(|oid| {
            let boundary = if session.shallow.contains(&raw.oid) {
                ParentBoundary::Shallow
            } else if let Some(position) = session.positions.get(&oid) {
                if *position < fence {
                    ParentBoundary::Loaded
                } else {
                    ParentBoundary::OffPage
                }
            } else if session.all.contains(&oid) {
                ParentBoundary::OutsideView
            } else {
                ParentBoundary::Missing
            };
            HistoryParent { oid, boundary }
        })
        .collect();
    Ok(CommitSummary {
        handle: session.handle(&raw.oid)?,
        decorations: session
            .refs
            .iter()
            .filter(|r| !r.hidden && r.commit_oid.as_ref() == Some(&raw.oid))
            .map(|r| r.name.clone())
            .collect(),
        oid: raw.oid,
        parents,
        author_name: raw.author.name,
        author_email: raw.author.email,
        authored_at: raw.author.timestamp,
        committed_at: raw.committed_at,
        subject: raw.message.lines().next().unwrap_or_default().into(),
    })
}
