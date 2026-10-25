//! Local branch/tag executor. Git is authoritative; no rollback or automatic retry.
use super::{
    authority::Target,
    catalog::{self, ReadQuery},
    fetch::{self, RefMap, RefState},
    journal::*,
    preflight, ref_catalog, ActionContext,
};
use crate::{
    domain::contracts::*,
    git::{
        error::{error, git_failure},
        runner::argv,
    },
    operations::scheduler::Completion,
};
use std::collections::BTreeMap;
fn changed() -> AppError {
    error(ErrorCode::ExternalChange, Phase::Verification)
}
fn unsupported() -> AppError {
    error(ErrorCode::UnsupportedAction, Phase::Validation)
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn target_ref(t: &Target) -> Result<(&str, Option<&str>), AppError> {
    match &t.proof {
        Some(SafeTargetProof::Ref {
            name, expected_oid, ..
        }) => Ok((name, expected_oid.as_deref())),
        _ => Err(unsupported()),
    }
}
fn target_oid(t: &Target) -> Result<&str, AppError> {
    match &t.proof {
        Some(SafeTargetProof::Ref {
            expected_oid: Some(oid),
            ..
        })
        | Some(SafeTargetProof::Commit { oid, .. }) => Ok(oid),
        _ => Err(unsupported()),
    }
}
async fn commit(q: &ReadQuery<'_>, t: &Target, length: usize) -> Result<String, AppError> {
    let raw = q
        .run(&argv(&[
            "-c",
            "advice.graftFileDeprecated=false",
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", target_oid(t)?),
        ]))
        .await?;
    let oid = std::str::from_utf8(&raw)
        .map_err(|_| malformed())?
        .trim_end_matches(['\r', '\n']);
    if !catalog::valid_oid(oid, length) {
        return Err(malformed());
    }
    Ok(oid.into())
}
pub(crate) async fn prepare(
    q: &ReadQuery<'_>,
    intent: &ActionIntent,
    targets: &[Target],
) -> Result<RefActionProof, AppError> {
    let cap = ref_catalog::capture(q).await?;
    if matches!(
        intent,
        ActionIntent::BranchCreate { .. } | ActionIntent::TagCreate { .. }
    ) && cap.refs.len() >= fetch::REFS_MAX
    {
        return Err(error(ErrorCode::OutputLimit, Phase::Validation));
    }
    let effective_config = config(q).await?;
    let local_config = config_at(q, true).await?;
    let mut steps = Vec::new();
    let mut add = |kind,
                   name: String,
                   old_name: Option<String>,
                   old_oid: Option<String>,
                   desired_oid: Option<String>,
                   upstream: Option<String>| {
        steps.push(RefStepProof {
            kind,
            name,
            old_name,
            old_oid,
            desired_oid,
            target_oid: None,
            upstream,
            failure_reason: None,
            phase: FetchStepPhase::Pending,
            effects: ActionEffects::Unknown,
            before_fingerprint: None,
            after_fingerprint: None,
        })
    };
    let first = targets.first().ok_or_else(unsupported)?;
    let entry = |name: &str| {
        cap.entries
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(changed)
    };
    match intent {
        ActionIntent::BranchCreate { name, checkout, .. } => {
            let full = format!("refs/heads/{name}");
            if cap.refs.contains_key(&full) {
                return Err(error(ErrorCode::DestinationExists, Phase::Validation));
            }
            let oid = commit(q, first, cap.length).await?;
            add(
                RefStepKind::CreateBranch,
                full.clone(),
                None,
                None,
                Some(oid.clone()),
                None,
            );
            if *checkout {
                if q.location.bare {
                    return Err(unsupported());
                }
                add(RefStepKind::SwitchBranch, full, None, None, Some(oid), None)
            }
        }
        ActionIntent::BranchRename { new_name, .. } => {
            let (old, oid) = target_ref(first)?;
            let e = entry(old)?;
            if e.kind != RefActionKind::Local || e.checked_out_elsewhere {
                return Err(unsupported());
            }
            let full = format!("refs/heads/{new_name}");
            require_local_config(&effective_config, &local_config, old)?;
            if effective_config
                .keys()
                .any(|k| branch_variable(k, new_name).is_some())
            {
                return Err(unsupported());
            }
            if cap.refs.contains_key(&full) {
                return Err(error(ErrorCode::DestinationExists, Phase::Validation));
            }
            add(
                RefStepKind::RenameBranch,
                full,
                Some(old.into()),
                oid.map(str::to_owned),
                oid.map(str::to_owned),
                None,
            )
        }
        ActionIntent::BranchSwitch { .. } => {
            let (name, oid) = target_ref(first)?;
            let e = entry(name)?;
            if q.location.bare || e.kind != RefActionKind::Local || e.checked_out_elsewhere {
                return Err(unsupported());
            }
            add(
                RefStepKind::SwitchBranch,
                name.into(),
                None,
                oid.map(str::to_owned),
                oid.map(str::to_owned),
                None,
            )
        }
        ActionIntent::BranchDelete { .. } => {
            let (name, oid) = target_ref(first)?;
            let e = entry(name)?;
            if e.kind != RefActionKind::Local
                || e.checked_out_here
                || cap
                    .head
                    .branch
                    .as_ref()
                    .is_some_and(|branch| name == format!("refs/heads/{branch}"))
                || e.checked_out_elsewhere
            {
                return Err(unsupported());
            }
            let head = cap.head.oid.as_deref().ok_or_else(unsupported)?;
            let check = q
                .output(&argv(&[
                    "-c",
                    "advice.graftFileDeprecated=false",
                    "merge-base",
                    "--is-ancestor",
                    oid.ok_or_else(unsupported)?,
                    head,
                ]))
                .await?;
            if !check.success {
                return Err(unsupported());
            }
            require_local_config(&effective_config, &local_config, name)?;
            add(
                RefStepKind::DeleteBranch,
                name.into(),
                None,
                oid.map(str::to_owned),
                None,
                None,
            );
            if local_config
                .keys()
                .any(|k| branch_variable(k, &name[11..]).is_some())
            {
                add(
                    RefStepKind::RemoveBranchConfig,
                    name.into(),
                    None,
                    None,
                    None,
                    None,
                )
            }
        }
        ActionIntent::BranchTracking {
            upstream_handle, ..
        } => {
            let (name, oid) = target_ref(first)?;
            let e = entry(name)?;
            if e.kind != RefActionKind::Local || e.checked_out_elsewhere {
                return Err(unsupported());
            }
            require_local_config(&effective_config, &local_config, name)?;
            let upstream: Option<String> = if upstream_handle.is_some() {
                let (t, _) = target_ref(targets.get(1).ok_or_else(unsupported)?)?;
                if !t.starts_with("refs/remotes/") {
                    return Err(unsupported());
                }
                Some(t.into())
            } else {
                None
            };
            if let Some(u) = &upstream {
                tracking(q, u).await?;
            }
            add(
                RefStepKind::Tracking,
                name.into(),
                None,
                oid.map(str::to_owned),
                oid.map(str::to_owned),
                upstream,
            )
        }
        ActionIntent::TagCreate {
            name, annotation, ..
        } => {
            let full = format!("refs/tags/{name}");
            if cap.refs.contains_key(&full) {
                return Err(error(ErrorCode::DestinationExists, Phase::Validation));
            }
            let oid = commit(q, first, cap.length).await?;
            let desired = if matches!(annotation, TagAnnotation::Lightweight) {
                Some(oid.clone())
            } else {
                None
            };
            add(RefStepKind::CreateTag, full, None, None, desired, None);
            steps.last_mut().ok_or_else(unsupported)?.target_oid = Some(oid)
        }
        ActionIntent::TagDelete { .. } => {
            let (name, oid) = target_ref(first)?;
            if !name.starts_with("refs/tags/") {
                return Err(unsupported());
            }
            add(
                RefStepKind::DeleteTag,
                name.into(),
                None,
                oid.map(str::to_owned),
                None,
                None,
            )
        }
        _ => return Err(unsupported()),
    }
    let proof = RefActionProof {
        object_format: ref_catalog::object_format(cap.length),
        steps,
    };
    proof.validate()?;
    Ok(proof)
}
type Config = BTreeMap<String, Vec<Vec<u8>>>;
async fn config(q: &ReadQuery<'_>) -> Result<Config, AppError> {
    config_at(q, false).await
}
async fn config_at(q: &ReadQuery<'_>, local: bool) -> Result<Config, AppError> {
    let args = if local {
        argv(&["config", "--local", "--null", "--list"])
    } else {
        argv(&["config", "--null", "--list"])
    };
    let raw = q.run(&args).await?;
    if raw.len() > 1024 * 1024 || (!raw.is_empty() && !raw.ends_with(&[0])) {
        return Err(malformed());
    }
    let mut out = Config::new();
    for line in raw.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let at = line.iter().position(|b| *b == b'\n');
        let (key, value) = at.map_or((line, b"true".as_slice()), |i| (&line[..i], &line[i + 1..]));
        let key = std::str::from_utf8(key).map_err(|_| malformed())?;
        out.entry(key.into()).or_default().push(value.to_vec());
    }
    Ok(out)
}
fn require_local_config(effective: &Config, local: &Config, name: &str) -> Result<(), AppError> {
    let branch = branch_name(name)?;
    let a = effective
        .iter()
        .filter(|(k, _)| branch_variable(k, branch).is_some())
        .collect::<Vec<_>>();
    let b = local
        .iter()
        .filter(|(k, _)| branch_variable(k, branch).is_some())
        .collect::<Vec<_>>();
    if a != b {
        return Err(unsupported());
    }
    Ok(())
}
async fn tracking(q: &ReadQuery<'_>, upstream: &str) -> Result<(String, String), AppError> {
    let config = catalog::configuration(q).await?;
    let mut found = Vec::new();
    for remote in config.remotes {
        for raw in remote.refspecs {
            if raw.starts_with('^') {
                continue;
            }
            let raw = raw.strip_prefix('+').unwrap_or(&raw);
            let Some((source, dest)) = raw.split_once(':') else {
                continue;
            };
            let source = if let Some((a, b)) = dest.split_once('*') {
                if upstream.starts_with(a)
                    && upstream.ends_with(b)
                    && upstream.len() >= a.len() + b.len()
                {
                    let middle = &upstream[a.len()..upstream.len() - b.len()];
                    source.replace('*', middle)
                } else {
                    continue;
                }
            } else if upstream == dest {
                source.into()
            } else {
                continue;
            };
            if source.starts_with("refs/heads/") && crate::history::refs::valid_name(&source) {
                found.push((remote.name.clone(), source));
            }
        }
    }
    found.sort();
    found.dedup();
    if found.len() != 1 {
        return Err(unsupported());
    }
    Ok(found.remove(0))
}
#[derive(PartialEq)]
struct State {
    refs: RefMap,
    config: Config,
    head: Head,
    status: Vec<u8>,
    index: Option<String>,
    worktrees: Vec<Vec<u8>>,
}
async fn state(q: &ReadQuery<'_>, length: usize) -> Result<State, AppError> {
    let refs = fetch::read_refs(q, length).await?;
    let bs = catalog::branches(q, length).await?;
    let (head, _) = catalog::head(q, &bs).await?;
    let status = if q.location.bare {
        vec![]
    } else {
        q.run(&argv(&[
            "-c",
            "advice.graftFileDeprecated=false",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ]))
        .await?
    };
    let raw = q
        .run(&argv(&["worktree", "list", "--porcelain", "-z"]))
        .await?;
    if !raw.ends_with(&[0, 0]) {
        return Err(malformed());
    }
    let mut worktrees = Vec::new();
    let mut local = false;
    for field in raw.split(|b| *b == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            let path = std::str::from_utf8(path).map_err(|_| malformed())?;
            local = std::fs::canonicalize(path)
                .ok()
                .is_some_and(|p| p == q.location.root);
        }
        // Our HEAD is checked separately; retain all other worktree metadata exactly.
        if !(local
            && (field.starts_with(b"HEAD ")
                || field.starts_with(b"branch ")
                || field == b"detached"))
        {
            worktrees.push(field.to_vec());
        }
    }
    let index = if q.location.bare {
        None
    } else {
        use std::io::Read;
        match std::fs::File::open(q.location.git_dir.join("index")) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(8 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| changed())?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
                }
                Some(preflight::digest(&bytes))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(changed()),
        }
    };
    Ok(State {
        refs,
        config: config(q).await?,
        head,
        status,
        index,
        worktrees,
    })
}
fn branch_name(name: &str) -> Result<&str, AppError> {
    name.strip_prefix("refs/heads/").ok_or_else(unsupported)
}
/// Git's variable is the final key component. The subsection between section
/// and variable may contain dots and is case-sensitive; the section is not.
fn branch_variable<'a>(key: &'a str, branch: &str) -> Option<&'a str> {
    let (section_subsection, variable) = key.rsplit_once('.')?;
    let (section, subsection) = section_subsection.split_once('.')?;
    (section.eq_ignore_ascii_case("branch") && subsection == branch && !variable.is_empty())
        .then_some(variable)
}
fn branch_prefix(name: &str) -> Result<String, AppError> {
    Ok(format!("branch.{}.", branch_name(name)?))
}
fn remove_branch(config: &mut Config, name: &str) -> Result<(), AppError> {
    let branch = branch_name(name)?;
    config.retain(|k, _| branch_variable(k, branch).is_none());
    Ok(())
}
fn set_tracking(
    config: &mut Config,
    name: &str,
    value: Option<(String, String)>,
) -> Result<(), AppError> {
    let prefix = branch_prefix(name)?;
    config.remove(&format!("{prefix}remote"));
    config.remove(&format!("{prefix}merge"));
    if let Some((remote, merge)) = value {
        config.insert(format!("{prefix}remote"), vec![remote.into_bytes()]);
        config.insert(format!("{prefix}merge"), vec![merge.into_bytes()]);
    }
    Ok(())
}
pub(crate) async fn execute(
    context: ActionContext,
    proof: RefActionProof,
) -> Result<Completion, AppError> {
    let length = if proof.object_format == ObjectFormat::Sha1 {
        40
    } else {
        64
    };
    let reader = context.reader()?;
    let mut expected = state(&reader, length).await?;
    if !Box::pin(context.before_write()).await? {
        return Ok(super::completed(ActionEffects::NotApplied, None));
    }
    let mut expected_fingerprint = context.precondition.fingerprint.clone();
    for (index, step) in proof.steps.iter().enumerate() {
        context.check_owner()?;
        let now = state(&reader, length).await?;
        if now != expected {
            return Err(changed());
        }
        let before = preflight::capture(
            reader.runner,
            &context.location,
            reader.lifetime.clone(),
            &context.cancel,
        )
        .await?;
        if before.fingerprint != expected_fingerprint {
            return Err(changed());
        }
        preflight::policy(&context.intent, &before)?;
        // Recheck linked-worktree ownership at every native boundary.
        let cap = ref_catalog::capture(&reader).await?;
        if cap
            .entries
            .iter()
            .find(|e| e.name == step.old_name.as_deref().unwrap_or(&step.name))
            .is_some_and(|e| e.checked_out_elsewhere)
        {
            return Err(unsupported());
        }
        let mut args;
        let mut input = None;
        let mut desired = State {
            refs: expected.refs.clone(),
            config: expected.config.clone(),
            head: expected.head.clone(),
            status: expected.status.clone(),
            index: expected.index.clone(),
            worktrees: expected.worktrees.clone(),
        };
        match step.kind {
            RefStepKind::CreateBranch => {
                args = argv(&[
                    "branch",
                    "--no-track",
                    "--",
                    &step.name[11..],
                    step.desired_oid.as_deref().ok_or_else(unsupported)?,
                ]);
                desired.refs.insert(
                    step.name.clone(),
                    RefState {
                        oid: step.desired_oid.clone().ok_or_else(unsupported)?,
                        symbolic: None,
                    },
                );
            }
            RefStepKind::RenameBranch => {
                let old = step.old_name.as_deref().ok_or_else(unsupported)?;
                args = argv(&["branch", "-m", "--", &old[11..], &step.name[11..]]);
                let r = desired.refs.remove(old).ok_or_else(changed)?;
                desired.refs.insert(step.name.clone(), r);
                let branch = branch_name(old)?;
                let new_prefix = branch_prefix(&step.name)?;
                let keys = desired
                    .config
                    .keys()
                    .filter(|k| branch_variable(k, branch).is_some())
                    .cloned()
                    .collect::<Vec<_>>();
                for key in keys {
                    let value = desired.config.remove(&key).ok_or_else(changed)?;
                    desired.config.insert(
                        format!(
                            "{new_prefix}{}",
                            branch_variable(&key, branch).ok_or_else(changed)?
                        ),
                        value,
                    );
                }
                if desired.head.branch.as_deref() == Some(&old[11..]) {
                    desired.head.branch = Some(step.name[11..].into());
                }
            }
            RefStepKind::SwitchBranch => {
                args = argv(&[
                    "switch",
                    "--no-guess",
                    "--no-overwrite-ignore",
                    "--",
                    &step.name[11..],
                ]);
                desired.head = Head {
                    kind: HeadKind::Branch,
                    branch: Some(step.name[11..].into()),
                    oid: step.desired_oid.clone(),
                };
                desired.status.clear();
            }
            RefStepKind::DeleteBranch => {
                args = argv(&["update-ref", "--stdin"]);
                let head = expected.head.oid.as_deref().ok_or_else(unsupported)?;
                input = Some(
                    format!(
                        "start\nverify HEAD {head}\ndelete {} {}\nprepare\ncommit\n",
                        step.name,
                        step.old_oid.as_deref().ok_or_else(unsupported)?
                    )
                    .into_bytes(),
                );
                desired.refs.remove(&step.name);
            }
            RefStepKind::RemoveBranchConfig => {
                args = argv(&[
                    "config",
                    "--local",
                    "--remove-section",
                    &format!("branch.{}", &step.name[11..]),
                ]);
                remove_branch(&mut desired.config, &step.name)?;
            }
            RefStepKind::Tracking => {
                let value = step.upstream.as_ref().map(|u| tracking(&reader, u));
                let value = match value {
                    Some(v) => Some(v.await?),
                    None => None,
                };
                set_tracking(&mut desired.config, &step.name, value)?;
                args = if let Some(u) = &step.upstream {
                    argv(&[
                        "branch",
                        &format!("--set-upstream-to={u}"),
                        "--",
                        &step.name[11..],
                    ])
                } else {
                    argv(&["branch", "--unset-upstream", "--", &step.name[11..]])
                };
            }
            RefStepKind::CreateTag => {
                let ActionIntent::TagCreate { annotation, .. } = &context.intent else {
                    return Err(unsupported());
                };
                args = match annotation {
                    TagAnnotation::Lightweight => argv(&["tag", "--no-sign"]),
                    TagAnnotation::Annotated { message, sign } => {
                        input = Some(message.as_bytes().to_vec());
                        argv(&[
                            "tag",
                            if *sign { "-s" } else { "-a" },
                            if *sign { "--sign" } else { "--no-sign" },
                            "--file=-",
                        ])
                    }
                };
                args.extend(argv(&[
                    "--",
                    &step.name[10..],
                    step.target_oid.as_deref().ok_or_else(unsupported)?,
                ]));
                desired.refs.insert(
                    step.name.clone(),
                    RefState {
                        oid: step.target_oid.clone().ok_or_else(unsupported)?,
                        symbolic: None,
                    },
                );
            }
            RefStepKind::DeleteTag => {
                args = argv(&[
                    "update-ref",
                    "--no-deref",
                    "-d",
                    &step.name,
                    step.old_oid.as_deref().ok_or_else(unsupported)?,
                ]);
                desired.refs.remove(&step.name);
            }
        }
        // Exact fingerprint immediately before durable per-step admission.
        let recheck = preflight::capture(
            reader.runner,
            &context.location,
            reader.lifetime.clone(),
            &context.cancel,
        )
        .await?;
        if recheck.fingerprint != before.fingerprint {
            return Err(changed());
        }
        if !context.before_ref_step(index, &before.fingerprint)? {
            return Ok(super::completed(
                if index == 0 {
                    ActionEffects::NotApplied
                } else {
                    ActionEffects::Partial
                },
                None,
            ));
        }
        let output = context.local_output(&args, input).await?;
        let after = state(&reader, length).await?;
        let mut verified = output.success;
        if step.kind == RefStepKind::SwitchBranch && output.success {
            let target = step.desired_oid.as_deref().ok_or_else(unsupported)?;
            let cached = reader
                .output(&argv(&[
                    "-c",
                    "advice.graftFileDeprecated=false",
                    "diff-index",
                    "--quiet",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--cached",
                    target,
                    "--",
                ]))
                .await?;
            let files = reader
                .output(&argv(&[
                    "-c",
                    "advice.graftFileDeprecated=false",
                    "diff-files",
                    "--quiet",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--",
                ]))
                .await?;
            verified = cached.success
                && files.success
                && cached.stderr.is_empty()
                && files.stderr.is_empty();
            if verified {
                desired.index = after.index.clone();
            }
        }
        if step.kind == RefStepKind::CreateTag
            && matches!(
                &context.intent,
                ActionIntent::TagCreate {
                    annotation: TagAnnotation::Annotated { .. },
                    ..
                }
            )
            && output.success
        {
            if let Some(tag) = after.refs.get(&step.name) {
                let typ = reader
                    .run(&argv(&[
                        "-c",
                        "advice.graftFileDeprecated=false",
                        "cat-file",
                        "-t",
                        &tag.oid,
                    ]))
                    .await?;
                let peeled = reader
                    .run(&argv(&[
                        "-c",
                        "advice.graftFileDeprecated=false",
                        "rev-parse",
                        "--verify",
                        &format!("{}^{{commit}}", tag.oid),
                    ]))
                    .await?;
                verified = typ == b"tag\n"
                    && peeled
                        == format!("{}\n", step.target_oid.as_deref().ok_or_else(unsupported)?)
                            .as_bytes();
                desired.refs.insert(step.name.clone(), tag.clone());
            } else {
                verified = false;
            }
        }
        verified &= after == desired;
        let effects = if verified {
            ActionEffects::Applied
        } else if after == expected {
            ActionEffects::NotApplied
        } else {
            ActionEffects::Unknown
        };
        let stamp = preflight::capture(
            reader.runner,
            &context.location,
            reader.lifetime.clone(),
            &context.cancel,
        )
        .await?
        .fingerprint;
        context.observe_ref_step(
            index,
            effects,
            Some(stamp.clone()),
            (!output.success).then(|| failure_reason(&output.stderr)),
        )?;
        if !verified {
            let effects = if index > 0 && effects == ActionEffects::NotApplied {
                ActionEffects::Partial
            } else {
                effects
            };
            let failure = if !output.success {
                git_failure(&output.stderr, Phase::Publish)
            } else {
                changed()
            };
            let _ = effects;
            return Err(failure);
        }
        expected = desired;
        expected_fingerprint = stamp;
    }
    let post = preflight::capture(
        reader.runner,
        &context.location,
        reader.lifetime.clone(),
        &context.cancel,
    )
    .await?
    .fingerprint;
    Ok(super::completed(ActionEffects::Applied, Some(post)))
}
pub(crate) fn effects(proof: &RefActionProof) -> ActionEffects {
    let applied = proof
        .steps
        .iter()
        .any(|s| s.phase == FetchStepPhase::Observed && s.effects == ActionEffects::Applied);
    if proof.steps.iter().any(|s| {
        s.phase == FetchStepPhase::Started
            || (s.phase == FetchStepPhase::Observed && s.effects == ActionEffects::Unknown)
    }) {
        ActionEffects::Unknown
    } else if proof
        .steps
        .iter()
        .all(|s| s.phase == FetchStepPhase::Observed && s.effects == ActionEffects::Applied)
    {
        ActionEffects::Applied
    } else if applied {
        ActionEffects::Partial
    } else {
        ActionEffects::NotApplied
    }
}

fn failure_reason(bytes: &[u8]) -> RefActionFailureReason {
    let has = |text: &[u8]| {
        bytes
            .windows(text.len())
            .any(|w| w.eq_ignore_ascii_case(text))
    };
    if [
        b"unable to auto-detect email address".as_slice(),
        b"please tell me who you are",
        b"committer identity unknown",
        b"empty ident name",
    ]
    .iter()
    .any(|v| has(v))
    {
        RefActionFailureReason::Identity
    } else if [
        b"failed to sign".as_slice(),
        b"unable to sign the tag",
        b"gpg: signing failed",
    ]
    .iter()
    .any(|v| has(v))
    {
        RefActionFailureReason::Signing
    } else {
        RefActionFailureReason::Native
    }
}
