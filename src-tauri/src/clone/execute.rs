use super::{
    grants::Control,
    journal::{self, ClonePhase, CloneRecord},
    path::StableDirectory,
    staging::Stage,
};
use crate::{
    backend::Backend,
    domain::contracts::*,
    git::{
        error::error,
        runner::{argv, Cancellation, RunOptions},
    },
    operations::scheduler::Completion,
    persistence::workspace::{db_error, encode},
    repositories::discovery,
};
use std::{
    ffi::OsString,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
pub struct Prepared {
    pub parent: Arc<StableDirectory>,
    pub source: OsString,
    pub source_guard: Option<Arc<super::grants::Source>>,
    pub control: Arc<Control>,
    pub record: CloneRecord,
    pub interactive: bool,
}
pub async fn run(
    backend: Arc<Backend>,
    mut prepared: Prepared,
    cancel: Cancellation,
) -> Result<Completion, AppError> {
    // Scheduler invokes this only after all locks/network permits are held.
    // A queued job must recheck a failure that occurred while it was waiting.
    backend.clones.check_ready()?;
    prepared.parent.recheck()?;
    if let Some(source) = &prepared.source_guard {
        source.directory.recheck()?;
        if discovery::identity(&source.common)? != source.common_identity {
            return Err(error(ErrorCode::ExternalChange, Phase::Validation));
        }
    }
    if cancel.is_cancelled() {
        return Ok(Completion::Cancelled);
    }
    if prepared.parent.path.join(&prepared.record.leaf).exists() {
        return Err(error(ErrorCode::DestinationExists, Phase::Validation));
    }
    let created = Stage::create(prepared.parent.clone(), &mut prepared.record);
    let mut stage = match created {
        Ok(stage) => Some(stage),
        Err(failure) => {
            // Creation may have stopped before ownership could be proven.
            // Preserve the artifact and durable notice, never path-delete it.
            if std::fs::symlink_metadata(prepared.parent.path.join(&prepared.record.container))
                .is_ok()
            {
                prepared.record.cleanup = journal::CleanupDisposition::Retained(
                    journal::CleanupReason::OwnershipUnproven,
                );
            }
            prepared.record.phase = ClonePhase::Aborted;
            if journal::write(&backend.registry.database, &prepared.record).is_err() {
                backend.clones.fail_closed();
                return Err(error(ErrorCode::Database, Phase::Recovery));
            }
            return Err(if prepared.record.cleanup.retained() {
                crate::git::error::retained_error(Some(failure))
            } else {
                failure
            });
        }
    };
    prepared.record.phase = ClonePhase::Staged;
    if journal::write(&backend.registry.database, &prepared.record).is_err() {
        if let Some(stage) = stage.take() {
            let _ = stage.cleanup(&prepared.record);
        }
        backend.clones.fail_closed();
        return Err(error(ErrorCode::Database, Phase::Recovery));
    }
    let transfer = async {
        let mut runner = backend.git()?.clone();
        runner.timeout = Duration::from_secs(30 * 60);
        let session = if prepared.interactive {
            Some(backend.auth.session(
                &prepared.control.window,
                &prepared.record.operation_id,
                cancel.clone(),
            )?)
        } else {
            None
        };
        let options = if let Some(session) = &session {
            session.options(
                &std::env::current_exe()
                    .map_err(|_| error(ErrorCode::Internal, Phase::Authentication))?,
            )?
        } else {
            RunOptions {
                interactive: true,
                ..Default::default()
            }
        };
        let mut args = argv(&["clone", "--progress", "--no-hardlinks", "--"]);
        args.push(prepared.source.clone());
        args.push(super::path::git_path(&stage.as_ref().unwrap().repository)?);
        backend.scheduler.progress(
            &prepared.record.operation_id,
            Progress {
                completed: 1,
                total: Some(4),
                unit: ProgressUnit::Steps,
            },
            Phase::Transfer,
        )?;
        let transfer = runner
            .run_with(None, &args, &cancel, Phase::Transfer, options)
            .await;
        let transfer = transfer.map_err(|failure| {
            if session
                .as_ref()
                .is_some_and(|session| !session.ssh_prompt_available())
                && prepared.source.to_str().is_some_and(|source| {
                    super::request::remote_transport(source)
                        == Ok(super::request::RemoteTransport::Ssh)
                })
            {
                crate::git::error::ssh_prompt_unavailable(failure)
            } else {
                failure
            }
        });
        if let Some(session) = session {
            session.close();
        }
        transfer
    }
    .await;
    let result = async {
        transfer?;
        backend.scheduler.progress(
            &prepared.record.operation_id,
            Progress {
                completed: 2,
                total: Some(4),
                unit: ProgressUnit::Steps,
            },
            Phase::Verification,
        )?;
        if cancel.is_cancelled() {
            return Ok(Completion::Cancelled);
        }
        prepared.parent.recheck()?;
        let item = discovery::discover(backend.git()?, &stage.as_ref().unwrap().repository).await?;
        if item.bare || item.root_identity != prepared.record.repository_identity {
            return Err(error(ErrorCode::MalformedOutput, Phase::Verification));
        }
        backend.scheduler.progress(
            &prepared.record.operation_id,
            Progress {
                completed: 3,
                total: Some(4),
                unit: ProgressUnit::Steps,
            },
            Phase::Publish,
        )?;
        // Serialize cancellation with the entire commit boundary. Once rename
        // wins, the UI cannot relabel or clean up the published destination.
        let _gate = prepared
            .control
            .gate
            .lock()
            .map_err(|_| error(ErrorCode::Internal, Phase::Publish))?;
        if cancel.is_cancelled() {
            return Ok(Completion::Cancelled);
        }
        prepared.parent.recheck()?;
        prepared.record.phase = ClonePhase::PublishIntent;
        journal::write(&backend.registry.database, &prepared.record)?;
        let publish = stage.as_mut().unwrap().publish(&prepared.record);
        if stage.as_ref().unwrap().may_have_published() {
            // Capture the irreversible boundary before propagating any error.
            prepared.control.published.store(true, Ordering::SeqCst);
        }
        let publish = publish?;
        if publish == super::path::PublishOutcome::Uncertain {
            return Err(error(ErrorCode::Database, Phase::Recovery));
        }
        let destination = prepared.parent.path.join(&prepared.record.leaf);
        let old_root = stage.as_ref().unwrap().repository.as_path();
        let relocate = |path: &std::path::Path| {
            path.strip_prefix(old_root)
                .map(|relative| destination.join(relative))
                .map_err(|_| error(ErrorCode::ExternalChange, Phase::Recovery))
        };
        let published = discovery::Discovered {
            common: relocate(&item.common)?,
            common_identity: item.common_identity,
            root: Some(destination.clone()),
            root_identity: item.root_identity,
            git_dir: relocate(&item.git_dir)?,
            bare: false,
        };
        if discovery::identity(&destination)?
            != prepared
                .record
                .repository_identity
                .clone()
                .ok_or_else(db_error)?
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Recovery));
        }
        let cleanup = stage.take().unwrap().cleanup(&prepared.record);
        prepared.record.cleanup = cleanup.disposition.clone();
        let cleanup_error = cleanup.warning();
        let operation_id = prepared.record.operation_id.clone();
        if backend
            .scheduler
            .succeed_clone(
                &operation_id,
                &published,
                &mut prepared.record,
                cleanup_error,
            )
            .is_err()
        {
            backend.clones.fail_closed();
            return Err(db_error());
        }
        Ok(Completion::Succeeded)
    }
    .await;
    if result.as_ref().is_err_and(|failure| {
        failure.code == ErrorCode::Interrupted && failure.phase == Phase::Recovery
    }) {
        prepared.record.cleanup =
            journal::CleanupDisposition::Retained(journal::CleanupReason::IoFailure);
        backend.clones.fail_closed();
        journal::write(&backend.registry.database, &prepared.record)?;
        // A bounded job stop could not be confirmed. Drop guards, preserve
        // intent/data; cancellation cannot authorize cleanup or publication.
        return Err(crate::git::error::retained_error(Some(error(
            ErrorCode::Interrupted,
            Phase::Recovery,
        ))));
    }
    if prepared.control.published.load(Ordering::SeqCst) {
        // Any error after the OS commit is a recovery problem, never a cancel
        // or permission to remove destination data.
        if result.is_err() {
            backend.clones.fail_closed();
            return Err(error(ErrorCode::Database, Phase::Recovery));
        }
        return result;
    }
    if let Some(stage) = stage {
        let cleanup = stage.cleanup(&prepared.record);
        prepared.record.cleanup = cleanup.disposition;
    }
    prepared.record.phase = ClonePhase::Aborted;
    journal::write(&backend.registry.database, &prepared.record)?;
    result
}
pub fn record(parent: &StableDirectory, id: &str, leaf: &str) -> CloneRecord {
    CloneRecord {
        operation_id: id.into(),
        parent: encode(&parent.path),
        parent_identity: parent.identity.clone(),
        leaf: leaf.into(),
        container: format!(".opengit-stage-{}", uuid::Uuid::new_v4()),
        container_identity: None,
        nonce: format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()),
        repository_identity: None,
        phase: ClonePhase::Preparing,
        marker_identity: None,
        cleanup: Default::default(),
    }
}
