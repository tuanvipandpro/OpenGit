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
            let head =
                changes::output(q, &argv(&["rev-parse", "--verify", "HEAD^{commit}"])).await?;
            if !head.success {
                let mut e = changes::unsupported();
                e.message = "Amend requires an existing HEAD commit.".into();
                return Err(e);
            }
            // Revision traversal hides parents at shallow boundaries, and native amend
            // may consequently create a root commit. Do not infer amend topology there.
            let shallow = changes::run(q, &argv(&["rev-parse", "--is-shallow-repository"])).await?;
            if shallow != b"false\n" {
                let mut e = changes::unsupported();
                e.message = "Amend is unsupported in a shallow repository. Fetch complete history before amending.".into();
                return Err(e);
            }
            return Ok(());
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
    // Freeze before semantic reads; before-resume must reject later shallow changes.
    let guard = preflight::native_stamp(&c.location)?;
    prepare(&q, &c.intent).await?;
    let before = changes::capture(&q).await?;
    if before.unsupported {
        return Err(changes::unsupported());
    }
    let amend = matches!(c.intent, ActionIntent::Commit { amend: true, .. });
    let expected_parents = if amend {
        let oid = before
            .status
            .head
            .oid
            .as_ref()
            .ok_or_else(changes::unsupported)?;
        let parents = changes::run(&q, &argv(&["rev-list", "--parents", "-n", "1", oid])).await?;
        let parents = std::str::from_utf8(&parents)
            .map_err(|_| changes::changed())?
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if parents.first() != Some(oid) {
            return Err(changes::changed());
        }
        parents[1..].to_vec()
    } else {
        before.status.head.oid.iter().cloned().collect::<Vec<_>>()
    };
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
    let ActionIntent::Commit { message, .. } = &c.intent else {
        return Err(changes::unsupported());
    };
    let mut args = argv(&["commit"]);
    if amend {
        args.push("--amend".into());
    }
    args.push("--file=-".into());
    let output = index::native(&c, &args, None, Some(message.as_bytes()), guard).await;
    if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
        p.result.failure_reason = Some(ChangeFailureReason::Recovery);
        index::checkpoint(&c, &p, &[])?;
        return Err(output.err().unwrap());
    }
    // Runner has proved quiescence. Inspect with a fresh cancellation token even after Cancel.
    q.cancel = Default::default();
    let after = changes::capture(&q).await?;
    let new = after.status.head.oid.clone();
    let native_ok = output.as_ref().is_ok_and(|o| o.success);
    if let Some(oid) = new
        .as_ref()
        .filter(|_| new != before.status.head.oid || amend && native_ok)
    {
        let parents = changes::run(&q, &argv(&["rev-list", "--parents", "-n", "1", oid])).await?;
        let line = std::str::from_utf8(&parents)
            .map_err(|_| changes::changed())?
            .split_whitespace()
            .collect::<Vec<_>>();
        let expected = expected_parents
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
            p.effects = if new != before.status.head.oid {
                ActionEffects::Applied
            } else {
                let state =
                    preflight::capture(q.runner, &c.location, q.lifetime.clone(), &q.cancel)
                        .await?;
                if state.fingerprint == c.precondition.fingerprint
                    && before.digest == after.digest
                    && index::index_digest(&c.location)? == p.before_index
                {
                    ActionEffects::NotApplied
                } else {
                    ActionEffects::Partial
                }
            };
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
