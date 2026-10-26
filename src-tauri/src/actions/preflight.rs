//! Bounded read-only preconditions. External Git is not universally frozen.
use crate::{
    domain::contracts::*,
    git::{
        error::error,
        operation_detection,
        runner::{argv, Cancellation, GitRunner, RunOptions},
    },
    repositories::{discovery, registry::RepositoryLocation},
};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Preflight {
    pub fingerprint: String,
    pub bare: bool,
    pub dirty: bool,
    pub unmerged: bool,
    pub busy: bool,
}
pub struct Pins {
    #[cfg(windows)]
    _directories: Vec<crate::clone::path::StableDirectory>,
}
impl Pins {
    pub fn open(location: &RepositoryLocation) -> Result<Arc<Self>, AppError> {
        #[cfg(windows)]
        {
            let mut pins = Vec::new();
            for p in [&location.common, &location.git_dir, &location.root] {
                pins.push(crate::clone::path::StableDirectory::open(p)?);
            }
            Ok(Arc::new(Self { _directories: pins }))
        }
        #[cfg(not(windows))]
        {
            let _ = location;
            Err(error(ErrorCode::UnsupportedGit, Phase::Validation))
        }
    }
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
/// Synchronous before-resume guard. Git's later native locks remain authoritative.
pub(crate) fn native_stamp(location: &RepositoryLocation) -> Result<String, AppError> {
    identities(location)?;
    let mut h = Sha256::new();
    for p in [
        location.git_dir.join("HEAD"),
        location.git_dir.join("index"),
        location.common.join("config"),
        location.git_dir.join("config.worktree"),
        location.common.join("packed-refs"),
        location.common.join("shallow"),
    ] {
        file(&mut h, &p)?;
    }
    fn walk(h: &mut Sha256, p: &Path, left: &mut usize) -> Result<(), AppError> {
        if !p.exists() {
            return Ok(());
        }
        let m = std::fs::symlink_metadata(p).map_err(|_| changed())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if m.file_attributes() & 0x400 != 0 {
                return Err(changed());
            }
        }
        if m.is_dir() {
            let mut entries = std::fs::read_dir(p)
                .map_err(|_| changed())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| changed())?;
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if *left == 0 {
                    return Err(error(ErrorCode::OutputLimit, Phase::Validation));
                }
                *left -= 1;
                charge(h, &crate::persistence::workspace::encode(&e.path()));
                walk(h, &e.path(), left)?;
            }
        } else {
            file(h, p)?;
        }
        Ok(())
    }
    let mut left = 4096;
    walk(&mut h, &location.common.join("refs"), &mut left)?;
    walk(&mut h, &location.common.join("reftable"), &mut left)?;
    Ok(format!("{:x}", h.finalize()))
}
fn changed() -> AppError {
    error(ErrorCode::ExternalChange, Phase::Validation)
}
fn charge(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes)
}
fn file(hash: &mut Sha256, path: &Path) -> Result<(), AppError> {
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let size = file.metadata().map_err(|_| changed())?.len();
            if size > 8 * 1024 * 1024 {
                return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
            }
            hash.update([1]);
            hash.update(size.to_le_bytes());
            let mut seen = 0usize;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer).map_err(|_| changed())?;
                if n == 0 {
                    break;
                }
                seen = seen.checked_add(n).ok_or_else(changed)?;
                if seen > 8 * 1024 * 1024 {
                    return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
                }
                hash.update(&buffer[..n]);
            }
            if seen as u64 != size {
                return Err(changed());
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            hash.update([0]);
            Ok(())
        }
        Err(_) => Err(changed()),
    }
}
fn identities(location: &RepositoryLocation) -> Result<(), AppError> {
    if discovery::identity(&location.common)? != location.common_identity
        || location
            .root_identity
            .as_ref()
            .is_some_and(|id| discovery::identity(&location.root).as_ref() != Ok(id))
    {
        return Err(changed());
    }
    Ok(())
}
pub fn global_args(location: &RepositoryLocation) -> Result<Vec<std::ffi::OsString>, AppError> {
    let mut args = argv(&["--no-lazy-fetch", "--no-replace-objects"]);
    let mut git = std::ffi::OsString::from("--git-dir=");
    git.push(crate::actions::path_text(&location.git_dir)?);
    args.push(git);
    if !location.bare {
        let mut root = std::ffi::OsString::from("--work-tree=");
        root.push(crate::actions::path_text(&location.root)?);
        args.push(root);
    }
    Ok(args)
}
pub async fn capture(
    runner: &GitRunner,
    location: &RepositoryLocation,
    pins: Arc<dyn Send + Sync>,
    cancel: &Cancellation,
) -> Result<Preflight, AppError> {
    capture_inner(runner, location, pins, cancel, true).await
}
/// Exclude only tracking-ref changes expected from an explicit fetch. HEAD,
/// every worktree/index/status/config/shallow/operation identity remains guarded.
pub(crate) async fn capture_stable(
    runner: &GitRunner,
    location: &RepositoryLocation,
    pins: Arc<dyn Send + Sync>,
    cancel: &Cancellation,
) -> Result<Preflight, AppError> {
    capture_inner(runner, location, pins, cancel, false).await
}
async fn capture_inner(
    runner: &GitRunner,
    location: &RepositoryLocation,
    pins: Arc<dyn Send + Sync>,
    cancel: &Cancellation,
    include_refs: bool,
) -> Result<Preflight, AppError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    identities(location)?;
    // Check the registered root still resolves to these physical Git directories.
    // Every child process retains pure pins through the runner's BQ guard.
    let mut hash = Sha256::new();
    charge(&mut hash, &discovery::identity(&location.git_dir)?);
    let mut dirty = false;
    let mut unmerged = false;
    // Execute the actual fixed capability query under the same pure read lease.
    // Unsupported daily capabilities do not change foundation Git's minimum.
    let mut child = runner.clone();
    child.timeout = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(|| error(ErrorCode::Timeout, Phase::Inspection))?;
    let mut args = global_args(location)?;
    args.extend(argv(&["worktree", "list", "--porcelain", "-z"]));
    let output = child
        .output_history(
            Some(&location.root),
            &args,
            cancel,
            Phase::Inspection,
            RunOptions {
                lifetime_guard: Some(pins.clone()),
                ..Default::default()
            },
            None,
        )
        .await?;
    if !output.success {
        if output.exit_code == Some(129) && output.stdout.is_empty() {
            return Err(super::safe_error(&error(
                ErrorCode::UnsupportedGit,
                Phase::Validation,
            )));
        }
        return Err(crate::git::error::git_failure(
            &output.stderr,
            Phase::Inspection,
        ));
    }
    if !output.stderr.is_empty()
        || !output.stdout.starts_with(b"worktree ")
        || output.stdout.len() <= 11
        || !output.stdout.ends_with(&[0, 0])
    {
        return Err(error(ErrorCode::MalformedOutput, Phase::Inspection));
    }
    charge(&mut hash, &output.stdout);
    for command in [
        vec!["rev-parse", "--path-format=absolute", "--git-common-dir"],
        vec!["rev-parse", "--absolute-git-dir"],
        vec![
            "for-each-ref",
            "--format=%(refname) %(objectname) %(symref)",
        ],
        vec!["config", "--null", "--list", "--show-origin"],
    ] {
        if cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Validation));
        }
        let mut child = runner.clone();
        child.timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or_else(|| error(ErrorCode::Timeout, Phase::Inspection))?;
        // Mapping commands deliberately discover through the worktree's .git pointer.
        let mapping = command[0] == "rev-parse";
        let mut args = if mapping {
            argv(&["--no-lazy-fetch", "--no-replace-objects"])
        } else {
            global_args(location)?
        };
        args.extend(argv(&command));
        let bytes = child
            .run_with(
                Some(&location.root),
                &args,
                cancel,
                Phase::Inspection,
                RunOptions {
                    lifetime_guard: Some(pins.clone()),
                    ..Default::default()
                },
            )
            .await?;
        if mapping {
            if bytes.len() > 131072 {
                return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
            }
            let path = String::from_utf8(bytes.clone()).map_err(|_| changed())?;
            let path = std::fs::canonicalize(path.trim_end_matches(['\r', '\n']))
                .map_err(|_| changed())?;
            if path
                != if command.contains(&"--git-common-dir") {
                    location.common.clone()
                } else {
                    location.git_dir.clone()
                }
            {
                return Err(changed());
            }
        }
        if include_refs || command[0] != "for-each-ref" {
            charge(&mut hash, &bytes);
        }
    }
    if !location.bare {
        let mut child = runner.clone();
        child.timeout = deadline
            .checked_duration_since(Instant::now())
            .filter(|v| !v.is_zero())
            .ok_or_else(|| error(ErrorCode::Timeout, Phase::Inspection))?;
        let mut args = global_args(location)?;
        args.extend(argv(&[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ]));
        if include_refs {
            args.push("--branch".into());
        }
        let bytes = child
            .run_with(
                Some(&location.root),
                &args,
                cancel,
                Phase::Inspection,
                RunOptions {
                    lifetime_guard: Some(pins.clone()),
                    ..Default::default()
                },
            )
            .await?;
        (dirty, unmerged) = state_flags(&bytes)?;
        charge(&mut hash, &bytes);
    }
    file(&mut hash, &location.common.join("shallow"))?;
    file(&mut hash, &location.common.join("info/grafts"))?;
    file(&mut hash, &location.git_dir.join("HEAD"))?;
    file(&mut hash, &location.git_dir.join("index"))?;
    let operations = operation_detection::detect(&location.git_dir);
    charge(
        &mut hash,
        &serde_json::to_vec(&operations).map_err(|_| changed())?,
    );
    identities(location)?;
    if Instant::now() >= deadline {
        return Err(error(ErrorCode::Timeout, Phase::Inspection));
    }
    Ok(Preflight {
        fingerprint: format!("{:x}", hash.finalize()),
        bare: location.bare,
        dirty,
        unmerged,
        busy: !operations.is_empty(),
    })
}
/// The authority kernel only needs flags, never per-file IDs/path objects.
/// Scan the bounded NUL stream without allocating one model per untracked file.
fn state_flags(bytes: &[u8]) -> Result<(bool, bool), AppError> {
    let malformed = || error(ErrorCode::MalformedOutput, Phase::Inspection);
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(malformed());
    }
    let mut records = bytes.split(|b| *b == 0);
    let (mut dirty, mut unmerged) = (false, false);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            continue;
        }
        if record.len() < 3 || record[1] != b' ' {
            return Err(malformed());
        }
        match record[0] {
            b'1' | b'?' | b'!' => (),
            b'u' => unmerged = true,
            b'2' => {
                if records.next().is_none_or(|v| v.is_empty()) {
                    return Err(malformed());
                }
            }
            _ => return Err(malformed()),
        }
        dirty = true;
    }
    Ok((dirty, unmerged))
}
pub fn policy(intent: &ActionIntent, state: &Preflight) -> Result<(), AppError> {
    if matches!(intent, ActionIntent::ReadRemoteRefs { .. }) {
        return Ok(());
    }
    if state.bare
        && matches!(
            intent,
            ActionIntent::BranchSwitch { .. }
                | ActionIntent::BranchCreate { checkout: true, .. }
                | ActionIntent::Pull { .. }
                | ActionIntent::Stage { .. }
                | ActionIntent::Unstage { .. }
                | ActionIntent::StageSelection { .. }
                | ActionIntent::UnstageSelection { .. }
                | ActionIntent::Commit { .. }
                | ActionIntent::StashPush { .. }
                | ActionIntent::StashApply { .. }
                | ActionIntent::StashPop { .. }
                | ActionIntent::StashDrop { .. }
        )
    {
        return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
    }
    if state.busy
        || state.unmerged
            && !matches!(
                intent,
                ActionIntent::Stage { .. } | ActionIntent::Unstage { .. }
            )
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Validation));
    }
    if state.dirty
        && matches!(
            intent,
            ActionIntent::BranchSwitch { .. }
                | ActionIntent::BranchCreate { checkout: true, .. }
                | ActionIntent::Pull { .. }
                | ActionIntent::StashApply { .. }
                | ActionIntent::StashPop { .. }
        )
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Validation));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flags_skip_rename_source_and_detect_unmerged_without_allocating_path_models() {
        assert_eq!(
            state_flags(b"# branch.head main\0? untracked\0").unwrap(),
            (true, false)
        );
        assert_eq!(
            state_flags(b"2 metadata new\0u source-name\0").unwrap(),
            (true, false)
        );
        assert_eq!(state_flags(b"u metadata conflict\0").unwrap(), (true, true));
        assert!(state_flags(b"2 metadata new\0").is_err());
        assert!(state_flags(b"? no terminator").is_err());
    }
    #[test]
    fn standard_sha256_vectors() {
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            digest(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
