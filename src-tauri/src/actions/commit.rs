//! Native commit runs on the real index. Hooks/signers may change it; never replace it afterwards.
use super::{changes, index, journal::*, preflight, ActionContext};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::argv},
    operations::scheduler::Completion,
};
pub(crate) async fn prepare(
    q: &super::catalog::ReadQuery<'_>,
    intent: &ActionIntent,
) -> Result<(), AppError> {
    changes::index_supported(q).await?;
    if let ActionIntent::Commit { amend, .. } = intent {
        if *amend {
            return Err(changes::unsupported());
        }
        let staged = changes::output(
            q,
            &argv(&[
                "diff",
                "--cached",
                "--quiet",
                "--no-ext-diff",
                "--no-textconv",
                "--",
            ]),
        )
        .await?;
        if staged.success {
            let mut failure = error(ErrorCode::InvalidRequest, Phase::Validation);
            failure.message =
                "There are no staged changes to commit. Stage a file before preparing a commit."
                    .into();
            return Err(failure);
        }
        if staged.exit_code != Some(1) {
            return Err(changes::unsupported());
        }
    }
    Ok(())
}
fn failure(bytes: &[u8]) -> ChangeFailureReason {
    let has = |s: &[u8]| bytes.windows(s.len()).any(|v| v.eq_ignore_ascii_case(s));
    if has(b"identity unknown")
        || has(b"tell me who you are")
        || has(b"unable to auto-detect email")
        || has(b"empty ident name")
    {
        ChangeFailureReason::Identity
    } else if has(b"signing failed") || has(b"failed to sign") || has(b"gpg failed") {
        ChangeFailureReason::Signing
    } else if has(b"index.lock") {
        ChangeFailureReason::Locked
    } else {
        ChangeFailureReason::HookOrNative
    }
}
pub(crate) async fn execute(c: ActionContext) -> Result<Completion, AppError> {
    let mut q = c.reader()?;
    prepare(&q, &c.intent).await?;
    let before = changes::capture(&q).await?;
    if before.unsupported {
        return Err(changes::unsupported());
    }
    let guard = preflight::native_stamp(&c.location)?;
    let mut p = index::proof(
        before.status.head.oid.clone(),
        index::index_digest(&c.location)?,
        None,
    );
    if !c.before_write().await? {
        return Err(error(ErrorCode::Interrupted, Phase::Publish));
    }
    p.phase = IndexPhase::CommitStarted;
    p.result.artifacts_retained = false;
    index::checkpoint(&c, &p, &[])?;
    let ActionIntent::Commit {
        message,
        amend: false,
    } = &c.intent
    else {
        return Err(changes::unsupported());
    };
    let output = index::native(
        &c,
        &argv(&["commit", "--file=-"]),
        None,
        Some(message.as_bytes()),
        guard,
    )
    .await;
    if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
        p.result.failure_reason = Some(ChangeFailureReason::Recovery);
        index::checkpoint(&c, &p, &[])?;
        return Err(output.err().unwrap());
    }
    // Runner has proved quiescence. Inspect with a fresh cancellation token even after Cancel.
    q.cancel = Default::default();
    let after = changes::capture(&q).await?;
    let new = after.status.head.oid.clone();
    if let Some(oid) = new.as_ref().filter(|_| new != before.status.head.oid) {
        let parents = changes::run(&q, &argv(&["rev-list", "--parents", "-n", "1", oid])).await?;
        let line = std::str::from_utf8(&parents)
            .map_err(|_| changes::changed())?
            .split_whitespace()
            .collect::<Vec<_>>();
        let expected = before
            .status
            .head
            .oid
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if line.first() == Some(&oid.as_str()) && line[1..] == expected {
            let tree = changes::run(
                &q,
                &argv(&["rev-parse", "--verify", &format!("{oid}^{{tree}}")]),
            )
            .await?;
            p.result.commit_oid = new.clone();
            p.result.tree_oid = Some(
                String::from_utf8(tree)
                    .map_err(|_| changes::changed())?
                    .trim()
                    .into(),
            );
            p.effects = ActionEffects::Applied;
        }
    } else {
        let state =
            preflight::capture(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
        p.effects = if state.fingerprint == c.precondition.fingerprint
            && before.digest == after.digest
            && index::index_digest(&c.location)? == p.before_index
        {
            ActionEffects::NotApplied
        } else {
            ActionEffects::Partial
        };
    }
    let failed = match &output {
        Ok(v) if !v.success => {
            p.result.failure_reason = Some(failure(&v.stderr));
            Some(crate::git::error::git_failure(&v.stderr, Phase::Publish))
        }
        Err(e) => {
            p.result.failure_reason = Some(ChangeFailureReason::Recovery);
            Some(e.clone())
        }
        _ => None,
    };
    p.phase = IndexPhase::Observed;
    index::checkpoint(&c, &p, &[])?;
    if let Some(e) = failed {
        return Err(e);
    }
    Ok(super::completed(p.effects, None))
}
