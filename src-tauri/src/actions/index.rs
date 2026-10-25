//! Whole-file index transaction. No rollback, foreign-lock cleanup, or restart resume.
use super::{changes, journal::*, preflight, ActionContext};
use crate::{
    domain::contracts::*,
    git::{
        error::error,
        runner::{argv, RunOptions, TrustedOwnedIndex},
    },
    operations::scheduler::Completion,
    repositories::{discovery::identity, registry::RepositoryLocation},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::Arc,
};
pub(crate) fn io_error() -> AppError {
    error(ErrorCode::Io, Phase::Publish)
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestHooks {
    pub after_native: std::sync::Mutex<Option<Arc<TestPause>>>,
    pub candidate: std::sync::Mutex<Option<Arc<TestPause>>>,
    pub published: std::sync::Mutex<Option<Arc<TestPause>>>,
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestPause {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
#[cfg(test)]
async fn pause(hook: &std::sync::Mutex<Option<Arc<TestPause>>>) {
    let pause = hook.lock().unwrap().take();
    if let Some(pause) = pause {
        pause.entered.notify_one();
        pause.release.notified().await;
    }
}
pub(crate) fn bytes(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(io_error()),
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err(changes::unsupported());
                }
            }
            if !m.is_file() || m.len() > 16 * 1024 * 1024 {
                return Err(changes::unsupported());
            }
            let mut b = Vec::new();
            let f = changes::open_regular(path)?;
            f.file
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut b)
                .map_err(|_| io_error())?;
            if b.len() > 16 * 1024 * 1024 {
                return Err(changes::unsupported());
            }
            if b.len() as u64 != m.len() {
                return Err(changes::changed());
            }
            Ok(Some(b))
        }
    }
}
pub(crate) fn index_digest(location: &RepositoryLocation) -> Result<String, AppError> {
    let b = bytes(&location.git_dir.join("index"))?;
    let mut h = Sha256::new();
    h.update([u8::from(b.is_some())]);
    if let Some(b) = b {
        h.update(b);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub(crate) fn proof(
    head: Option<String>,
    before: String,
    name: Option<String>,
) -> ChangeActionProof {
    ChangeActionProof {
        phase: IndexPhase::Reserved,
        alternate_name: name,
        before_index: before,
        candidate_digest: None,
        candidate_tree: None,
        before_head: head,
        effects: ActionEffects::Unknown,
        result: ChangeActionResult {
            commit_oid: None,
            tree_oid: None,
            failure_reason: None,
            artifacts_retained: true,
        },
    }
}
pub(crate) fn checkpoint(
    c: &ActionContext,
    p: &ChangeActionProof,
    a: &[ArtifactProof],
) -> Result<(), AppError> {
    c.scheduler
        .action_change_checkpoint(&c.operation_id, p.clone(), a.to_vec())
}
fn artifact(kind: ArtifactKind, name: &str, id: Vec<u8>) -> ArtifactProof {
    ArtifactProof {
        kind,
        root: ArtifactRoot::Git,
        name: name.into(),
        identity: id,
    }
}
#[cfg(windows)]
fn create_owned(path: &Path) -> Result<File, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::*;
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .access_mode(0x10000 | FILE_GENERIC_READ | FILE_GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| {
            error(
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    ErrorCode::DestinationExists
                } else {
                    ErrorCode::Io
                },
                Phase::Publish,
            )
        })
}
#[cfg(not(windows))]
fn create_owned(_path: &Path) -> Result<File, AppError> {
    Err(changes::unsupported())
}
#[cfg(windows)]
fn opened_identity(file: &File) -> Result<Vec<u8>, AppError> {
    Ok(crate::clone::path::info_identity(
        &crate::clone::path::handle_info(file)?,
    ))
}
#[cfg(not(windows))]
fn opened_identity(_file: &File) -> Result<Vec<u8>, AppError> {
    Err(changes::unsupported())
}
#[cfg(windows)]
fn remove_owned(path: &Path, expected: &[u8]) -> Result<(), AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::*;
    let f = OpenOptions::new()
        .access_mode(0x10000 | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| io_error())?;
    if opened_identity(&f)? != expected || identity(path)? != expected {
        return Err(changes::changed());
    }
    crate::clone::path::delete_empty_object(&f)
}
#[cfg(not(windows))]
fn remove_owned(_path: &Path, _expected: &[u8]) -> Result<(), AppError> {
    Err(changes::unsupported())
}
#[cfg(windows)]
fn publish(lock: &File, path: &Path, expected: &[u8]) -> Result<(), AppError> {
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    if opened_identity(lock)? != expected {
        return Err(changes::changed());
    }
    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let length = (offset + units.len() * 2 + 2).max(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buffer = vec![0usize; length.div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        (*info).Anonymous.ReplaceIfExists = true;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = (units.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            units.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            units.len(),
        );
        if SetFileInformationByHandle(
            lock.as_raw_handle() as _,
            FileRenameInfo,
            buffer.as_ptr() as _,
            length as u32,
        ) == 0
        {
            return Err(io_error());
        }
    }
    if identity(path)? != expected {
        return Err(changes::changed());
    }
    Ok(())
}
#[cfg(not(windows))]
fn publish(_lock: &File, _path: &Path, _expected: &[u8]) -> Result<(), AppError> {
    Err(changes::unsupported())
}
pub(crate) async fn native(
    c: &ActionContext,
    args: &[std::ffi::OsString],
    index: Option<TrustedOwnedIndex>,
    input: Option<&[u8]>,
    guard: String,
) -> Result<crate::git::runner::GitOutput, AppError> {
    let location = c.location.clone();
    let targets = c.targets.clone();
    let intent = c.intent.clone();
    let validate = c.validate.clone();
    let check_index = index.clone();
    let cancel = c.cancel.clone();
    let mut options = c.run_options(RunOptions {
        trusted_index: index,
        ..Default::default()
    })?;
    #[cfg(windows)]
    {
        options.before_resume = Some(Arc::new(move |_| {
            if cancel.is_cancelled() {
                return Err(error(ErrorCode::Interrupted, Phase::Publish));
            }
            validate(super::Gate::Check)?;
            if let Some(v) = &check_index {
                v.validate()?;
            }
            if preflight::native_stamp(&location)? != guard {
                return Err(changes::changed());
            }
            changes::validate_targets(&location, &intent, &targets)?;
            Ok(())
        }));
    }
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
            input,
        )
        .await
}
fn empty_index(format: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut b = b"DIRC\0\0\0\x02\0\0\0\0".to_vec();
    if format == b"sha256\n" {
        b.extend(Sha256::digest(&b));
    } else if format == b"sha1\n" {
        b.extend([
            0x39, 0xd8, 0x90, 0x13, 0x9e, 0xe5, 0x35, 0x6c, 0x7e, 0xf5, 0x72, 0x21, 0x6c, 0xeb,
            0xcd, 0x27, 0xaa, 0x41, 0xf9, 0xdf,
        ]);
    } else {
        return Err(changes::unsupported());
    }
    Ok(b)
}
pub(crate) async fn execute(c: ActionContext) -> Result<Completion, AppError> {
    let mut q = c.reader()?;
    changes::index_supported(&q).await?;
    changes::validate_targets(&c.location, &c.intent, &c.targets)?;
    let before = changes::capture(&q).await?;
    let before_native = preflight::native_stamp(&c.location)?;
    if !c.before_write().await? {
        return Err(error(ErrorCode::Interrupted, Phase::Publish));
    }
    let name = format!("action-index-{}", crate::history::sessions::token());
    let alt = c.location.git_dir.join(&name);
    let lock_path = c.location.git_dir.join("index.lock");
    let mut p = proof(
        before.status.head.oid,
        index_digest(&c.location)?,
        Some(name.clone()),
    );
    let mut artifacts = Vec::new();
    checkpoint(&c, &p, &artifacts)?; // Durable reservation before create-new; a crash gap has no cleanup authority.
    let mut publication_attempted = false;
    let result = async {
        let data = match bytes(&c.location.git_dir.join("index"))? {
            Some(b) => b,
            None => empty_index(
                &changes::run(&q, &argv(&["rev-parse", "--show-object-format"])).await?,
            )?,
        };
        let mut f = create_owned(&alt)?;
        let id = opened_identity(&f)?;
        if identity(&alt)? != id {
            return Err(changes::changed());
        }
        artifacts.push(artifact(ArtifactKind::AlternateIndex, &name, id));
        p.phase = IndexPhase::AlternateOwned;
        checkpoint(&c, &p, &artifacts)?;
        f.write_all(&data).map_err(|_| io_error())?;
        f.sync_all().map_err(|_| io_error())?;
        drop(f);
        let mut paths = Vec::new();
        for target in &c.targets {
            let v = changes::mutation_paths(target)?;
            for raw in v {
                if !paths.contains(&raw) {
                    paths.push(raw);
                }
            }
        }
        // Exactly the argv mutation set is exempt from the preservation oracle.
        let selected = paths
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut args = if matches!(c.intent, ActionIntent::Stage { .. }) {
            argv(&["--literal-pathspecs", "add", "-A", "--"])
        } else if p.before_head.is_some() {
            argv(&["--literal-pathspecs", "reset", "--quiet", "HEAD", "--"])
        } else {
            argv(&[
                "--literal-pathspecs",
                "update-index",
                "--force-remove",
                "--",
            ])
        };
        for raw in &paths {
            args.push(
                std::str::from_utf8(raw)
                    .map_err(|_| changes::unsupported())?
                    .into(),
            );
        }
        let original_entries = changes::run(&q, &argv(&["ls-files", "--stage", "-z"])).await?;
        let owned = TrustedOwnedIndex::new(alt.clone(), artifacts[0].identity.clone())?;
        let output = native(&c, &args, Some(owned), None, before_native.clone()).await?;
        #[cfg(test)]
        pause(&c.resources.index_test.after_native).await;
        if !output.success {
            return Err(crate::git::error::git_failure(
                &output.stderr,
                Phase::Publish,
            ));
        }
        // Only a successful, quiescent native replacement can be inspected as
        // a candidate. Keep cleanup ownership unchanged until it is verified.
        let actual = identity(&alt)?;
        let owned = TrustedOwnedIndex::new(alt.clone(), actual)?;
        let listed = native(
            &c,
            &argv(&["ls-files", "--stage", "-z"]),
            Some(owned.clone()),
            None,
            before_native.clone(),
        )
        .await?;
        if !listed.success {
            return Err(changes::changed());
        }
        fn untouched(
            bytes: &[u8],
            selected: &std::collections::HashSet<Vec<u8>>,
        ) -> Result<std::collections::BTreeMap<Vec<u8>, Vec<u8>>, AppError> {
            let mut result = std::collections::BTreeMap::new();
            for e in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                let at = e
                    .iter()
                    .position(|b| *b == b'\t')
                    .ok_or_else(changes::changed)?;
                let path = e[at + 1..].to_vec();
                if !selected.contains(&path) && result.insert(path, e[..at].to_vec()).is_some() {
                    return Err(changes::changed());
                }
            }
            Ok(result)
        }
        if untouched(&original_entries, &selected)? != untouched(&listed.stdout, &selected)? {
            return Err(changes::changed());
        }
        let tree = native(
            &c,
            &argv(&["write-tree"]),
            Some(owned),
            None,
            before_native.clone(),
        )
        .await?;
        if !tree.success {
            return Err(changes::unsupported());
        }
        let tree = String::from_utf8(tree.stdout)
            .map_err(|_| io_error())?
            .trim()
            .to_owned();
        if !matches!(tree.len(), 40 | 64) || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io_error());
        }
        // write-tree can refresh cache-tree metadata: copy its final candidate bytes.
        let mut candidate_file = changes::open_regular(&alt)?;
        let candidate_identity = opened_identity(&candidate_file.file)?;
        let candidate_length = candidate_file
            .file
            .metadata()
            .map_err(|_| io_error())?
            .len();
        if candidate_length > 16 * 1024 * 1024 {
            return Err(changes::unsupported());
        }
        let mut candidate_after = Vec::new();
        (&mut candidate_file.file)
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut candidate_after)
            .map_err(|_| io_error())?;
        if candidate_after.len() as u64 != candidate_length {
            return Err(changes::changed());
        }
        artifacts[0].identity = candidate_identity;
        p.candidate_digest = Some(preflight::digest(&candidate_after));
        p.candidate_tree = Some(tree.clone());
        p.phase = IndexPhase::CandidateReady;
        checkpoint(&c, &p, &artifacts)?;
        #[cfg(test)]
        pause(&c.resources.index_test.candidate).await;
        if preflight::native_stamp(&c.location)? != before_native {
            return Err(changes::changed());
        }
        changes::validate_targets(&c.location, &c.intent, &c.targets)?;
        let state =
            preflight::capture(q.runner, &c.location, q.lifetime.clone(), &c.cancel).await?;
        if state.fingerprint != c.precondition.fingerprint {
            return Err(changes::changed());
        }
        p.phase = IndexPhase::LockReserved;
        checkpoint(&c, &p, &artifacts)?;
        let mut lock = create_owned(&lock_path)?;
        let lock_id = opened_identity(&lock)?;
        if identity(&lock_path)? != lock_id {
            return Err(changes::changed());
        }
        artifacts.push(artifact(
            ArtifactKind::IndexLock,
            "index.lock",
            lock_id.clone(),
        ));
        p.phase = IndexPhase::LockOwned;
        checkpoint(&c, &p, &artifacts)?;
        lock.write_all(&candidate_after).map_err(|_| io_error())?;
        lock.sync_all().map_err(|_| io_error())?;
        if index_digest(&c.location)? != p.before_index
            || preflight::native_stamp(&c.location)? != before_native
        {
            return Err(changes::changed());
        }
        changes::validate_targets(&c.location, &c.intent, &c.targets)?;
        c.check_owner()?;
        if c.cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Publish));
        }
        p.phase = IndexPhase::PublicationStarted;
        checkpoint(&c, &p, &artifacts)?;
        if index_digest(&c.location)? != p.before_index
            || preflight::native_stamp(&c.location)? != before_native
        {
            return Err(changes::changed());
        }
        changes::validate_targets(&c.location, &c.intent, &c.targets)?;
        c.check_owner()?;
        if c.cancel.is_cancelled() {
            return Err(error(ErrorCode::Interrupted, Phase::Publish));
        }
        publication_attempted = true;
        publish(&lock, &c.location.git_dir.join("index"), &lock_id)?;
        lock.sync_all().map_err(|_| io_error())?;
        drop(lock);
        if bytes(&c.location.git_dir.join("index"))?
            .as_ref()
            .map(|b| preflight::digest(b))
            != p.candidate_digest
        {
            return Err(changes::changed());
        }
        artifacts.retain(|a| a.kind != ArtifactKind::IndexLock);
        p.phase = IndexPhase::Published;
        p.effects = ActionEffects::Applied;
        p.result.tree_oid = Some(tree);
        checkpoint(&c, &p, &artifacts)?;
        #[cfg(test)]
        pause(&c.resources.index_test.published).await;
        Ok::<(), AppError>(())
    }
    .await;
    let uncertain = result.as_ref().is_err_and(|e| e.phase == Phase::Recovery)
        || publication_attempted && p.phase == IndexPhase::PublicationStarted;
    if !uncertain {
        let mut retained = alt.with_file_name(format!("{name}.lock")).exists();
        for a in &artifacts {
            if remove_owned(&c.location.git_dir.join(&a.name), &a.identity).is_err() {
                retained = true;
            }
        }
        if !retained {
            artifacts.clear();
        }
        p.result.artifacts_retained = retained;
        if result.is_err() {
            q.cancel = Default::default();
            p.effects = if index_digest(&c.location).ok().as_ref() == Some(&p.before_index)
                && preflight::native_stamp(&c.location).ok().as_ref() == Some(&before_native)
                && changes::validate_targets(&c.location, &c.intent, &c.targets).is_ok()
            {
                ActionEffects::NotApplied
            } else {
                ActionEffects::Unknown
            };
        }
    }
    if let Err(e) = &result {
        p.result.failure_reason = Some(if e.code == ErrorCode::DestinationExists {
            ChangeFailureReason::Locked
        } else if e.code == ErrorCode::ExternalChange {
            ChangeFailureReason::Stale
        } else {
            ChangeFailureReason::Recovery
        });
    }
    if uncertain {
        p.effects = ActionEffects::Unknown;
        p.result.artifacts_retained = true;
    }
    p.phase = if uncertain {
        p.phase
    } else {
        IndexPhase::Observed
    };
    checkpoint(&c, &p, &artifacts)?;
    result?;
    Ok(super::completed(p.effects, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Backend>,
        ActionScope,
        RepositoryLocation,
    ) {
        let (dir, b, s) = crate::backend::action_lifecycle_tests::setup().await;
        let l = b
            .registry
            .location(&s.repo_id, s.worktree_id.as_deref())
            .unwrap();
        fs::write(l.root.join("target Ω.txt"), b"original selected\n").unwrap();
        (dir, b, s, l)
    }
    async fn prepared(b: &Backend, s: &ActionScope) -> PreparedAction {
        let c = b
            .get_repository_changes("main", GetRepositoryChangesRequest { scope: s.clone() })
            .await
            .unwrap();
        let e = c
            .entries
            .iter()
            .find(|e| e.path_display == "target Ω.txt")
            .unwrap();
        let t = b
            .resolve_change_targets(
                "main",
                ResolveChangeTargetsRequest {
                    scope: s.clone(),
                    catalog_id: c.catalog_id,
                    targets: vec![ChangeSelection {
                        entry_id: e.entry_id.clone(),
                        side: ChangeSide::Unstaged,
                    }],
                },
            )
            .await
            .unwrap();
        b.prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: s.clone(),
                intent: ActionIntent::Stage {
                    change_handles: vec![t.targets[0].change_handle.clone()],
                },
            },
        )
        .await
        .unwrap()
    }
    async fn held(
        b: &Arc<Backend>,
        s: &ActionScope,
        published: bool,
    ) -> (OperationSnapshot, Arc<TestPause>) {
        let p = prepared(b, s).await;
        let pause = Arc::new(TestPause::default());
        *if published {
            b.actions.index_test.published.lock().unwrap()
        } else {
            b.actions.index_test.candidate.lock().unwrap()
        } = Some(pause.clone());
        let op = b
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest { plan_id: p.plan_id },
            )
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), pause.entered.notified())
            .await
            .unwrap();
        (op, pause)
    }
    async fn finished(b: &Backend, id: &str) -> ChangeActionOperationResult {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let r = b
                    .get_change_action_outcome(
                        "main",
                        GetActionOutcomeRequest {
                            operation_id: id.into(),
                        },
                    )
                    .unwrap();
                if crate::operations::state::terminal(&r.operation.state) {
                    break r;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn git(b: &Backend, l: &RepositoryLocation, args: &[&str]) -> Vec<u8> {
        b.git()
            .unwrap()
            .run(
                Some(&l.root),
                &argv(args),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn failed_native_never_adopts_or_cleans_a_replacement_alternate_identity() {
        let (_dir, b, s, l) = fixture().await;
        fs::write(l.root.join(".gitattributes"), b"*.txt filter=fail\n").unwrap();
        git(&b, &l, &["config", "filter.fail.clean", "false"]).await;
        git(&b, &l, &["config", "filter.fail.required", "true"]).await;
        let plan = prepared(&b, &s).await;
        let pause = Arc::new(TestPause::default());
        *b.actions.index_test.after_native.lock().unwrap() = Some(pause.clone());
        let op = b
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id,
                },
            )
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(60), pause.entered.notified())
            .await
            .unwrap();
        let record = super::super::journal::read(&b.registry.database, &op.id).unwrap();
        let original = record.artifacts[0].identity.clone();
        let name = record.change_action.unwrap().alternate_name.unwrap();
        let alternate = l.git_dir.join(&name);
        let displaced = l.git_dir.join(format!("{name}.displaced"));
        fs::rename(&alternate, &displaced).unwrap();
        fs::write(&alternate, b"foreign replacement: preserve").unwrap();
        let replacement = identity(&alternate).unwrap();
        assert_ne!(replacement, original);
        pause.release.notify_one();
        let result = finished(&b, &op.id).await;
        assert_ne!(result.operation.state, OperationState::Succeeded);
        assert!(result.result.as_ref().unwrap().artifacts_retained);
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().recovery,
            ActionRecovery::Inspect
        );
        assert_eq!(
            fs::read(&alternate).unwrap(),
            b"foreign replacement: preserve"
        );
        assert_eq!(identity(&displaced).unwrap(), original);
        let record = super::super::journal::read(&b.registry.database, &op.id).unwrap();
        assert_eq!(record.artifacts[0].identity, original);
        assert_ne!(record.artifacts[0].identity, replacement);
        assert!(!l.git_dir.join("index").exists());
        assert_eq!(
            fs::read(l.root.join("target Ω.txt")).unwrap(),
            b"original selected\n"
        );
    }
    #[tokio::test]
    async fn s5a_external_file_index_head_and_config_races_never_publish_over_external_state() {
        for mode in ["file", "index", "head", "config"] {
            let (_dir, b, s, l) = fixture().await;
            let (op, pause) = held(&b, &s, false).await;
            match mode {
                "file" => fs::write(
                    l.root.join("target Ω.txt"),
                    b"external same-status content\n",
                )
                .unwrap(),
                "index" => {
                    fs::write(l.root.join("other.txt"), b"external staged\n").unwrap();
                    git(&b, &l, &["add", "--", "other.txt"]).await;
                }
                "head" => {
                    git(
                        &b,
                        &l,
                        &[
                            "-c",
                            "user.name=Fixture",
                            "-c",
                            "user.email=fixture@example.invalid",
                            "commit",
                            "--allow-empty",
                            "-m",
                            "external commit",
                        ],
                    )
                    .await;
                }
                "config" => {
                    git(&b, &l, &["config", "core.fileMode", "true"]).await;
                }
                _ => unreachable!(),
            }
            let expected = bytes(&l.git_dir.join("index")).unwrap();
            let expected_work = fs::read(l.root.join("target Ω.txt")).unwrap();
            pause.release.notify_one();
            let r = finished(&b, &op.id).await;
            assert_ne!(
                r.operation.outcome.as_ref().unwrap().effects,
                ActionEffects::Applied,
                "{mode}: {r:?}"
            );
            assert_eq!(
                r.operation.error.as_ref().unwrap().code,
                ErrorCode::ExternalChange,
                "{mode}"
            );
            assert_eq!(bytes(&l.git_dir.join("index")).unwrap(), expected, "{mode}");
            assert_eq!(
                fs::read(l.root.join("target Ω.txt")).unwrap(),
                expected_work
            );
            assert!(!l.git_dir.join("index.lock").exists());
        }
    }
    #[tokio::test]
    async fn s5a_cancel_before_publication_is_not_applied_and_after_publication_retains_actual_index(
    ) {
        for after in [false, true] {
            let (_dir, b, s, l) = fixture().await;
            let (op, pause) = held(&b, &s, after).await;
            b.cancel_operation(
                "main",
                CancelOperationRequest {
                    operation_id: op.id.clone(),
                },
            )
            .unwrap();
            pause.release.notify_one();
            let r = finished(&b, &op.id).await;
            assert!(r.operation.outcome.as_ref().unwrap().cancellation_requested);
            if after {
                assert_eq!(
                    r.operation.outcome.as_ref().unwrap().effects,
                    ActionEffects::Applied
                );
                assert_eq!(
                    git(&b, &l, &["show", ":target Ω.txt"]).await,
                    b"original selected\n"
                );
            } else {
                assert_eq!(
                    r.operation.outcome.as_ref().unwrap().effects,
                    ActionEffects::NotApplied
                );
                assert!(!l.git_dir.join("index").exists());
            }
            assert_eq!(
                fs::read(l.root.join("target Ω.txt")).unwrap(),
                b"original selected\n"
            );
            assert!(!l.git_dir.join("index.lock").exists());
        }
    }
    #[tokio::test]
    async fn s5a_database_fault_before_and_after_publication_never_erases_actual_effects() {
        for after in [false, true] {
            let (_dir, b, s, l) = fixture().await;
            let (op, pause) = held(&b, &s, after).await;
            let phase = if after {
                "observed"
            } else {
                "publicationStarted"
            };
            b.registry.database.lock().unwrap().execute_batch(&format!("CREATE TRIGGER s5_fixture_db_failure BEFORE UPDATE ON repository_actions WHEN json_extract(NEW.record,'$.changeAction.phase')='{phase}' BEGIN SELECT RAISE(FAIL,'synthetic checkpoint fault'); END;")).unwrap();
            pause.release.notify_one();
            let r = finished(&b, &op.id).await;
            if after {
                assert_ne!(
                    r.operation.outcome.as_ref().unwrap().effects,
                    ActionEffects::NotApplied
                );
                assert_eq!(
                    git(&b, &l, &["show", ":target Ω.txt"]).await,
                    b"original selected\n"
                );
            } else {
                assert_eq!(
                    r.operation.outcome.as_ref().unwrap().effects,
                    ActionEffects::NotApplied,
                    "{r:?}"
                );
                assert!(!l.git_dir.join("index").exists());
            }
            assert_eq!(
                fs::read(l.root.join("target Ω.txt")).unwrap(),
                b"original selected\n"
            );
        }
    }
    #[tokio::test]
    async fn s5a_restart_reservation_identity_gap_is_inspect_and_never_basename_cleanup_authority()
    {
        let (_dir, b, s, l) = fixture().await;
        let plan = prepared(&b, &s).await;
        let mut execution = b
            .consume_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id,
                },
            )
            .unwrap();
        let name = format!("action-index-{}", crate::history::sessions::token());
        let p = proof(None, index_digest(&l).unwrap(), Some(name.clone()));
        b.scheduler
            .action_change_checkpoint(&execution.operation.id, p, vec![])
            .unwrap();
        fs::write(l.git_dir.join(&name), b"unproven artifact ownership").unwrap();
        let mut interrupted = execution.operation.clone();
        crate::actions::recovery::reconcile(&b.registry.database, &mut interrupted).unwrap();
        assert_eq!(
            interrupted.outcome.as_ref().unwrap().effects,
            ActionEffects::Unknown
        );
        assert_eq!(
            interrupted.outcome.as_ref().unwrap().recovery,
            ActionRecovery::Inspect
        );
        assert_eq!(
            fs::read(l.git_dir.join(name)).unwrap(),
            b"unproven artifact ownership"
        );
        execution.guard.as_mut().unwrap().armed = false;
    }
}
