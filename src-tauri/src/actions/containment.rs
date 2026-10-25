//! Membership concerns every captured branch, never just decorations or loaded pages.
use super::{catalog, fetch};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::argv},
    history::{sessions::Session, ActionHistoryQuery},
};
pub(crate) async fn read(
    query: &ActionHistoryQuery<'_>,
    session: &Session,
    handle: &str,
    oid: &str,
) -> Result<CommitContainment, AppError> {
    let raw = query
        .run(
            &argv(&[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(symref)%00%(upstream)%00",
                "refs/heads/",
                "refs/remotes/",
            ]),
            None,
        )
        .await?;
    let branches = catalog::parse_branches(&raw, session.fingerprint.oid_length)?;
    if branches.branches.len() + branches.symbolic.len() > fetch::REFS_MAX {
        return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
    }
    let contains = query
        .run(
            &argv(&[
                "for-each-ref",
                &format!("--contains={oid}"),
                "--format=%(refname)",
                "refs/heads/",
                "refs/remotes/",
            ]),
            None,
        )
        .await?;
    let contains = std::str::from_utf8(&contains)
        .map_err(|_| error(ErrorCode::MalformedOutput, Phase::Inspection))?;
    let mut positive = std::collections::HashSet::new();
    for name in contains.lines() {
        if !crate::history::refs::valid_name(name)
            || positive.len() >= fetch::REFS_MAX
            || !positive.insert(name)
        {
            return Err(error(ErrorCode::MalformedOutput, Phase::Inspection));
        }
    }
    if positive.iter().any(|name| {
        !branches.branches.iter().any(|b| b.name == *name)
            && !branches.symbolic.iter().any(|b| b.name == *name)
    }) {
        return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
    }
    let current_branch = session
        .head
        .branch
        .as_ref()
        .map(|v| format!("refs/heads/{v}"));
    let upstream = current_branch
        .as_ref()
        .and_then(|name| branches.upstreams.get(name))
        .cloned();
    let complete = session.fingerprint.shallow.is_empty();
    Ok(CommitContainment {
        context: session.context.clone(),
        commit_handle: handle.into(),
        oid: oid.into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
        branches: branches
            .branches
            .into_iter()
            .map(|branch| {
                let membership = if positive.contains(branch.name.as_str()) {
                    Membership::Contains
                } else if complete {
                    Membership::DoesNotContain
                } else {
                    Membership::Unknown
                };
                ContainmentBranch { branch, membership }
            })
            .collect(),
        current_branch,
        upstream,
        completeness: if complete {
            AncestryCompleteness::Complete
        } else {
            AncestryCompleteness::Shallow
        },
    })
}
