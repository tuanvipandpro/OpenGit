//! One explicit remote destination. Native FF/lease rules remain authoritative;
//! a dated advertisement is never a branch compare-and-swap promise.
use super::{
    authority::{Target, TargetKind},
    catalog::{self, ReadQuery},
    fetch,
    journal::*,
    preflight, remote, ActionContext,
};
use crate::{
    domain::contracts::*,
    git::{
        error::{error, git_failure},
        runner::{argv, RunOptions},
    },
    operations::scheduler::Completion,
};
use std::{ffi::OsString, sync::Arc};
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestHooks {
    pub before_fast_forward: std::sync::Mutex<Option<Arc<TestPause>>>,
}
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestPause {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
pub(crate) fn supported(k: ActionKind) -> bool {
    matches!(
        k,
        ActionKind::Push
            | ActionKind::RemoteTagPush
            | ActionKind::RemoteTagDelete
            | ActionKind::Pull
    )
}
fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn changed() -> AppError {
    error(ErrorCode::ExternalChange, Phase::Validation)
}
fn unsupported() -> AppError {
    error(ErrorCode::UnsupportedAction, Phase::Validation)
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Verification)
}
fn hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(crate) fn validate_proof(p: &SyncActionProof, kind: ActionKind) -> Result<(), AppError> {
    let expected = match kind {
        ActionKind::Push => vec![SyncStepKind::Push],
        ActionKind::RemoteTagPush => vec![SyncStepKind::TagPush],
        ActionKind::RemoteTagDelete => vec![SyncStepKind::TagDelete],
        ActionKind::Pull => vec![SyncStepKind::Fetch, SyncStepKind::FastForward],
        _ => return Err(unsupported()),
    };
    if serde_json::to_vec(p).map_err(|_| invalid())?.len() > 15360 {
        return Err(error(ErrorCode::OutputLimit, Phase::Validation));
    }
    if (if kind == ActionKind::Pull {
        p.role != RemoteEndpointRole::Fetch
    } else {
        p.role != RemoteEndpointRole::Push
    }) || !fetch::remote_name(&p.remote_name)
        || p.remote_name.len() > 256
        || !hex(&p.config_digest)
        || !hex(&p.before_fingerprint)
        || !crate::history::refs::valid_name(&p.destination)
        || p.destination.len() > 1024
        || p.source_ref
            .as_ref()
            .is_some_and(|s| s.len() > 1024 || !crate::history::refs::valid_name(s))
        || p.source_oid.as_ref().is_some_and(|s| !oid(s))
        || p.preview_oid.as_ref().is_some_and(|s| !oid(s))
        || p.result.steps.iter().map(|s| s.step).collect::<Vec<_>>() != expected
        || p.step_stamps.len() != expected.len()
        || p.step_stamps.iter().flatten().any(|s| !hex(s))
        || p.result.steps.iter().any(|s| {
            s.observed_at
                .as_ref()
                .is_some_and(|s| chrono::DateTime::parse_from_rfc3339(s).is_err())
        })
        || p.result.fetched_oid.as_ref().is_some_and(|s| !oid(s))
        || p.result.integrated_oid.as_ref().is_some_and(|s| !oid(s))
    {
        return Err(crate::persistence::workspace::db_error());
    }
    if let Some(e) = &p.result.publication {
        super::publication::validate(e)?;
        if e.remote_name != p.remote_name
            || e.destination != p.destination
            || e.config_digest != p.config_digest
            || e.source_ref != p.source_ref
            || e.source_oid != p.source_oid
        {
            return Err(invalid());
        }
    }
    Ok(())
}
struct Pin {
    name: String,
    alias: String,
    url: String,
}
impl Pin {
    fn new(url: &str) -> Self {
        Self {
            name: format!("opengit-sync-{}", crate::history::sessions::token()),
            alias: format!("opengit-captured-{}:", crate::history::sessions::token()),
            url: url.into(),
        }
    }
    fn args(&self) -> Vec<OsString> {
        argv(&[
            "-c",
            "advice.graftFileDeprecated=false",
            "-c",
            &format!("remote.{}.url={}", self.name, self.alias),
            "-c",
            &format!("remote.{}.pushurl={}", self.name, self.alias),
            "-c",
            &format!("remote.{}.mirror=false", self.name),
        ])
    }
    fn environment(&self) -> Result<Vec<(OsString, OsString)>, AppError> {
        fetch::alias_environment(&self.alias, &self.url)
    }
    async fn verify(&self, q: &ReadQuery<'_>) -> Result<(), AppError> {
        let mut args = self.args();
        args.extend(argv(&["remote", "-v"]));
        let output = q.output_environment(&args, "", self.environment()?).await?;
        if !output.success || !output.stderr.is_empty() {
            return Err(malformed());
        }
        let text = std::str::from_utf8(&output.stdout).map_err(|_| malformed())?;
        let prefix = format!("{}\t", self.name);
        let values = text
            .lines()
            .filter_map(|l| l.strip_prefix(&prefix))
            .collect::<Vec<_>>();
        // remote -v uses native get-push-URLs, unlike get-url which rejects a
        // command-only remote. Explicit pushurl prevents pushInsteadOf fallback.
        if values.len() != 2
            || !values.contains(&format!("{} (fetch)", self.url).as_str())
            || !values.contains(&format!("{} (push)", self.url).as_str())
        {
            return Err(unsupported());
        }
        Ok(())
    }
}
pub(crate) struct PreparedSync {
    proof: SyncActionProof,
    pin: Pin,
    pull: Option<PullSpec>,
}
struct PullSpec {
    remote_ref: String,
    tracking_ref: String,
    head_ref: String,
    head_oid: String,
    refs: fetch::RefMap,
}
fn matches_pattern(pattern: &str, name: &str) -> bool {
    if let Some((a, b)) = pattern.split_once('*') {
        name.starts_with(a) && name.ends_with(b) && name.len() >= a.len() + b.len()
    } else {
        pattern == name
    }
}
fn mapping(remote: &catalog::Remote, source: &str) -> Result<String, AppError> {
    if remote.refspecs.iter().any(|v| {
        v.strip_prefix('^')
            .is_some_and(|p| matches_pattern(p, source))
    }) {
        return Err(unsupported());
    }
    let mut result = std::collections::BTreeSet::new();
    for spec in &remote.refspecs {
        if spec.starts_with('^') {
            continue;
        }
        let spec = spec.strip_prefix('+').unwrap_or(spec);
        let (src, dst) = spec.split_once(':').ok_or_else(unsupported)?;
        if let Some((a, b)) = src.split_once('*') {
            if source.starts_with(a) && source.ends_with(b) && source.len() >= a.len() + b.len() {
                let capture = &source[a.len()..source.len() - b.len()];
                result.insert(dst.replacen('*', capture, 1));
            }
        } else if src == source {
            result.insert(dst.to_owned());
        }
    }
    if result.len() != 1 {
        return Err(unsupported());
    }
    Ok(result.into_iter().next().unwrap())
}
async fn prepare_pull(
    q: &ReadQuery<'_>,
    targets: &[Target],
    stamp: &str,
) -> Result<PreparedSync, AppError> {
    if q.location.bare {
        return Err(unsupported());
    }
    let cap = super::ref_catalog::capture(q).await?;
    if cap.head.kind != HeadKind::Branch {
        return Err(unsupported());
    }
    let branch = cap.head.branch.as_ref().ok_or_else(unsupported)?;
    let head_oid = cap.head.oid.clone().ok_or_else(unsupported)?;
    let head_ref = format!("refs/heads/{branch}");
    let raw = super::changes::run(q, &argv(&["config", "--null", "--list"])).await?;
    let mut remote_name = Vec::new();
    let mut merge = Vec::new();
    for record in raw.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let text = std::str::from_utf8(record).map_err(|_| malformed())?;
        let (key, value) = text.split_once('\n').unwrap_or((text, "true"));
        if key == format!("branch.{branch}.remote") {
            remote_name.push(value.to_owned());
        }
        if key == format!("branch.{branch}.merge") {
            merge.push(value.to_owned());
        }
    }
    if remote_name.len() != 1
        || merge.len() != 1
        || remote_name[0] == "."
        || !merge[0].starts_with("refs/heads/")
    {
        return Err(unsupported());
    }
    let config = catalog::configuration(q).await?;
    if config.partial {
        return Err(unsupported());
    }
    if super::changes::run(q, &argv(&["rev-parse", "--is-shallow-repository"])).await? != b"false\n"
    {
        return Err(unsupported());
    }
    let remote = config
        .remotes
        .iter()
        .find(|r| {
            r.name == remote_name[0] && r.capability == FetchCapability::EligibleProbeRequired
        })
        .ok_or_else(unsupported)?;
    let tracking = mapping(remote, &merge[0])?;
    let t = targets.first().ok_or_else(invalid)?;
    let (name, expected) = source(t, TargetKind::RemoteRef)?;
    if name != tracking
        || cap
            .refs
            .get(name)
            .is_none_or(|r| r.oid != expected || r.symbolic.is_some())
    {
        return Err(changed());
    }
    let branches = catalog::branches(q, head_oid.len()).await?;
    if branches.upstreams.get(&head_ref) != Some(&tracking) {
        return Err(changed());
    }
    let pin = Pin::new(remote.url.as_deref().ok_or_else(unsupported)?);
    pin.verify(q).await?;
    let proof = SyncActionProof {
        remote_name: remote.name.clone(),
        role: RemoteEndpointRole::Fetch,
        config_digest: config.digest,
        source_ref: Some(merge[0].clone()),
        source_oid: Some(head_oid.clone()),
        destination: head_ref.clone(),
        preview_oid: Some(expected.into()),
        before_fingerprint: stamp.into(),
        step_stamps: vec![None, None],
        result: SyncActionResult {
            steps: [SyncStepKind::Fetch, SyncStepKind::FastForward]
                .into_iter()
                .map(|step| SyncStepResult {
                    step,
                    phase: FetchStepPhase::Pending,
                    effects: ActionEffects::Unknown,
                    failure_reason: None,
                    observed_at: None,
                })
                .collect(),
            publication: None,
            tracking_updated: false,
            upstream_updated: false,
            tracking_refresh_required: false,
            fetched_oid: None,
            integrated_oid: None,
        },
    };
    validate_proof(&proof, ActionKind::Pull)?;
    Ok(PreparedSync {
        proof,
        pin,
        pull: Some(PullSpec {
            remote_ref: merge[0].clone(),
            tracking_ref: tracking,
            head_ref,
            head_oid,
            refs: cap.refs,
        }),
    })
}
fn source(t: &Target, kind: TargetKind) -> Result<(&str, &str), AppError> {
    let Some(SafeTargetProof::Ref {
        name,
        expected_oid: Some(oid),
        ..
    }) = &t.proof
    else {
        return Err(invalid());
    };
    if t.kind != kind || t.identity != name.as_bytes() {
        return Err(invalid());
    }
    Ok((name, oid))
}
pub(crate) async fn prepare(
    q: &ReadQuery<'_>,
    intent: &ActionIntent,
    targets: &[Target],
    stamp: &str,
) -> Result<PreparedSync, AppError> {
    if matches!(intent, ActionIntent::Pull { .. }) {
        return prepare_pull(q, targets, stamp).await;
    }
    if let ActionIntent::Push {
        set_upstream: true, ..
    } = intent
    {
        let mut e = unsupported();
        e.message="Automatic upstream setup is unavailable in this push flow. Fetch explicitly, then use Set or unset upstream in Local actions.".into();
        return Err(e);
    }
    let endpoints = remote::endpoints(q).await?;
    let endpoint = remote::select(&endpoints, targets.first().ok_or_else(invalid)?)?;
    if endpoint.role != RemoteEndpointRole::Push {
        return Err(invalid());
    }
    let destination = targets.last().ok_or_else(invalid)?;
    let Some(SafeTargetProof::RemoteDestination {
        remote_name,
        role,
        ref_name,
        expected_oid,
        object_format,
        config_digest,
    }) = &destination.proof
    else {
        return Err(invalid());
    };
    if destination.kind != TargetKind::RemoteDestination
        || *role != RemoteEndpointRole::Push
        || remote_name != &endpoint.name
        || config_digest != &endpoint.digest
        || destination.identity
            != serde_json::to_vec(destination.proof.as_ref().unwrap()).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    let (source_ref, source_oid, step) = match intent {
        ActionIntent::Push { .. } => {
            let (name, oid) = source(targets.get(1).ok_or_else(invalid)?, TargetKind::Branch)?;
            if !name.starts_with("refs/heads/") || !ref_name.starts_with("refs/heads/") {
                return Err(invalid());
            }
            (
                Some(name.to_owned()),
                Some(oid.to_owned()),
                SyncStepKind::Push,
            )
        }
        ActionIntent::RemoteTagPush { .. } => {
            let (name, oid) = source(targets.get(1).ok_or_else(invalid)?, TargetKind::Tag)?;
            if !name.starts_with("refs/tags/") || !ref_name.starts_with("refs/tags/") {
                return Err(invalid());
            }
            if expected_oid.as_ref().is_some_and(|v| v != oid) {
                let mut e = error(ErrorCode::DestinationExists, Phase::Validation);
                e.message =
                    "Remote tag already has a different object. Tag replacement is not supported."
                        .into();
                return Err(e);
            }
            (
                Some(name.to_owned()),
                Some(oid.to_owned()),
                SyncStepKind::TagPush,
            )
        }
        ActionIntent::RemoteTagDelete { .. } => {
            if !ref_name.starts_with("refs/tags/") || expected_oid.is_none() {
                return Err(invalid());
            }
            (None, expected_oid.clone(), SyncStepKind::TagDelete)
        }
        _ => return Err(unsupported()),
    };
    if let Some(s) = &source_oid {
        if let Some(format) = object_format {
            let length = if *format == ObjectFormat::Sha1 {
                40
            } else {
                64
            };
            if s.len() != length {
                return Err(unsupported());
            }
        }
    }
    if let (Some(name), Some(oid)) = (&source_ref, &source_oid) {
        let refs = fetch::read_refs(q, oid.len()).await?;
        if refs
            .get(name)
            .is_none_or(|r| r.oid != *oid || r.symbolic.is_some())
        {
            return Err(changed());
        }
        if step == SyncStepKind::Push {
            let typ = super::changes::run(q, &argv(&["cat-file", "-t", oid])).await?;
            if typ != b"commit\n" {
                return Err(unsupported());
            }
        }
    }
    let pin = Pin::new(endpoint.url.as_deref().ok_or_else(invalid)?);
    pin.verify(q).await?;
    let proof = SyncActionProof {
        remote_name: endpoint.name.clone(),
        role: RemoteEndpointRole::Push,
        config_digest: endpoint.digest.clone(),
        source_ref,
        source_oid,
        destination: ref_name.clone(),
        preview_oid: expected_oid.clone(),
        before_fingerprint: stamp.into(),
        step_stamps: vec![None],
        result: SyncActionResult {
            steps: vec![SyncStepResult {
                step,
                phase: FetchStepPhase::Pending,
                effects: ActionEffects::Unknown,
                failure_reason: None,
                observed_at: None,
            }],
            publication: None,
            tracking_updated: false,
            upstream_updated: false,
            tracking_refresh_required: false,
            fetched_oid: None,
            integrated_oid: None,
        },
    };
    validate_proof(&proof, intent.kind())?;
    Ok(PreparedSync {
        proof,
        pin,
        pull: None,
    })
}
fn parse_push(bytes: &[u8], source: &str, destination: &str) -> Result<char, AppError> {
    if bytes.len() > 8 * 1024 * 1024 || !bytes.ends_with(b"\n") {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    let mut status = None;
    for line in text.lines() {
        if line.len() >= 2 && line.as_bytes()[1] == b'\t' {
            let mut fields = line.splitn(3, '\t');
            let flag = fields.next().unwrap();
            let refs = fields.next().ok_or_else(malformed)?;
            let _summary = fields.next().ok_or_else(malformed)?;
            if flag.len() != 1
                || ![' ', '=', '*', '-', '!'].contains(&(flag.as_bytes()[0] as char))
                || (refs != format!("{source}:{destination}")
                    && !(source.is_empty()
                        && flag == "!"
                        && refs == format!("(delete):{destination}")))
                || status.is_some()
            {
                return Err(malformed());
            }
            status = Some(flag.as_bytes()[0] as char);
        } else if !(line.starts_with("To ") || line == "Done" || line.is_empty()) {
            return Err(malformed());
        }
    }
    status.ok_or_else(malformed)
}
fn failure(e: &AppError) -> SyncFailureReason {
    match e.code {
        ErrorCode::AuthenticationFailed
        | ErrorCode::AuthenticationRequired
        | ErrorCode::SshAuthentication => SyncFailureReason::Authentication,
        ErrorCode::ExternalChange | ErrorCode::InvalidHandle => SyncFailureReason::Stale,
        ErrorCode::Network | ErrorCode::Timeout => SyncFailureReason::Network,
        _ => SyncFailureReason::Recovery,
    }
}
#[cfg(windows)]
async fn native_step(
    c: &ActionContext,
    index: usize,
    args: &[OsString],
    mut options: RunOptions,
    stamp: String,
    phase: Phase,
) -> Result<crate::git::runner::GitOutput, AppError> {
    let gate = c.validate.clone();
    let location = c.location.clone();
    let previous = options.before_resume.take();
    options.before_resume = Some(Arc::new(move |job| {
        if preflight::native_stamp(&location)? != stamp {
            return Err(changed());
        }
        let digest = stamp.as_bytes().try_into().map_err(|_| invalid())?;
        if !gate(super::Gate::SyncStep(index, digest))? {
            return Err(error(ErrorCode::Interrupted, Phase::Transfer));
        }
        if let Some(callback) = &previous {
            callback(job)?;
        }
        Ok(())
    }));
    let mut options = c.run_options(options)?;
    options.environment.extend([
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
        ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ("GIT_ALLOW_PROTOCOL".into(), "file:https:ssh".into()),
    ]);
    let mut all = preflight::global_args(&c.location)?;
    all.extend_from_slice(args);
    let mut runner = c.runner.clone();
    runner.timeout = if phase == Phase::Transfer {
        c.resources.fetch_timeout
    } else {
        std::time::Duration::from_secs(120)
    };
    runner
        .output_history(
            Some(&c.location.root),
            &all,
            &c.cancel,
            phase,
            options,
            None,
        )
        .await
}
fn observe(
    c: &ActionContext,
    db: &crate::persistence::workspace::Database,
    p: &mut SyncActionProof,
    index: usize,
    effects: ActionEffects,
    reason: Option<SyncFailureReason>,
) -> Result<(), AppError> {
    p.result.steps[index].phase = FetchStepPhase::Observed;
    p.result.steps[index].effects = effects;
    p.result.steps[index].failure_reason = reason;
    p.result.steps[index].observed_at = Some(chrono::Utc::now().to_rfc3339());
    p.step_stamps = super::journal::read(db, &c.operation_id)?
        .sync_action
        .ok_or_else(invalid)?
        .step_stamps;
    c.sync_observed(p.clone())
}
#[cfg(windows)]
async fn execute_pull(
    c: ActionContext,
    db: Arc<crate::persistence::workspace::Database>,
    auth: Arc<crate::auth::AuthBroker>,
    owner: String,
    mut proof: SyncActionProof,
    pin: Pin,
    pull: PullSpec,
) -> Result<Completion, AppError> {
    let mut q = c.reader()?;
    let stable =
        preflight::capture_stable(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
    c.check_owner()?;
    c.scheduler
        .action_sync_begin(&c.operation_id, proof.clone())?;
    if !c.before_write().await? {
        return Ok(super::completed(ActionEffects::NotApplied, None));
    }
    let session = if crate::clone::request::remote_transport(&pin.url).is_ok() {
        Some(auth.session(&owner, &c.operation_id, c.cancel.clone())?)
    } else {
        None
    };
    let mut options = if let Some(s) = &session {
        s.options(
            &std::env::current_exe()
                .map_err(|_| error(ErrorCode::Internal, Phase::Authentication))?,
        )?
    } else {
        RunOptions::default()
    };
    options.environment.extend(pin.environment()?);
    let mut args = pin.args();
    args.extend(argv(&[
        "-c",
        "fetch.writeCommitGraph=false",
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "fetch",
        "--porcelain",
        "--verbose",
        "--atomic",
        "--no-write-fetch-head",
        "--no-prune",
        "--no-prune-tags",
        "--no-tags",
        "--no-recurse-submodules",
        "--no-auto-maintenance",
        "--upload-pack=git-upload-pack",
        "--",
        &pin.name,
        &format!("+{}:{}", pull.remote_ref, pull.tracking_ref),
    ]));
    let out = native_step(
        &c,
        0,
        &args,
        options,
        preflight::native_stamp(&c.location)?,
        Phase::Transfer,
    )
    .await;
    if let Some(s) = session {
        s.close();
    }
    let output = out?;
    if !output.success {
        return Err(git_failure(&output.stderr, Phase::Transfer));
    }
    let reported = fetch::parse_porcelain(&output.stdout, &proof.remote_name, pull.head_oid.len())?;
    if reported.len() != 1 || reported[0].name != pull.tracking_ref {
        return Err(malformed());
    }
    let fetched = reported[0].new_oid.clone();
    if !oid(&fetched) || fetched.bytes().all(|b| b == b'0') {
        return Err(malformed());
    }
    let expected = fetch::apply_reported(&pull.refs, &reported, pull.head_oid.len())?;
    proof.result.fetched_oid = Some(fetched.clone());
    proof.result.tracking_updated = reported[0].old_oid != reported[0].new_oid;
    observe(&c, &db, &mut proof, 0, ActionEffects::Applied, None)?;
    q.cancel = Default::default();
    // Bind the before-resume guard before checking the original semantic state.
    // Sampling later would adopt an intervening checkout as fresh authority.
    let fast_forward_stamp = preflight::native_stamp(&c.location)?;
    let check=async{let now=preflight::capture_stable(q.runner,&c.location,q.lifetime.clone(),&q.cancel).await?;
        if now.fingerprint!=stable.fingerprint||fetch::read_refs(&q,pull.head_oid.len()).await?!=expected||catalog::configuration(&q).await?.digest!=proof.config_digest{return Err(changed());}
        preflight::policy(&c.intent,&now)?;
        let ancestor=super::changes::output(&q,&argv(&["merge-base","--is-ancestor",&pull.head_oid,&fetched])).await?;
        if !ancestor.success {let mut e=unsupported();e.message=if ancestor.exit_code==Some(1){"Upstream diverged. This pull supports fast-forward only; it will not merge, rebase or stash."}else{"Ancestry could not be verified. Inspect missing or shallow history before pulling."}.into();return Err(e);}Ok::<(),AppError>(())}.await;
    if let Err(e) = check {
        observe(
            &c,
            &db,
            &mut proof,
            1,
            ActionEffects::NotApplied,
            Some(if e.code == ErrorCode::UnsupportedAction {
                SyncFailureReason::Diverged
            } else {
                failure(&e)
            }),
        )?;
        return Err(e);
    }
    if c.cancel.is_cancelled() {
        return Ok(super::completed(proof.effects(), None));
    }
    #[cfg(test)]
    {
        let pause = c
            .resources
            .sync_test
            .before_fast_forward
            .lock()
            .unwrap()
            .take();
        if let Some(pause) = pause {
            pause.entered.notify_one();
            pause.release.notified().await;
        }
    }
    c.check_owner()?;
    let merge = argv(&[
        "-c",
        "advice.graftFileDeprecated=false",
        "merge",
        "--ff-only",
        "--no-autostash",
        "--no-overwrite-ignore",
        "--no-edit",
        "--",
        &fetched,
    ]);
    let output = native_step(
        &c,
        1,
        &merge,
        RunOptions::default(),
        fast_forward_stamp,
        Phase::Publish,
    )
    .await;
    if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
        return Err(output.err().unwrap());
    }
    if let Err(e) = &output {
        let started = super::journal::read(&db, &c.operation_id)?
            .sync_action
            .ok_or_else(invalid)?
            .result
            .steps[1]
            .phase
            != FetchStepPhase::Pending;
        if !started {
            observe(
                &c,
                &db,
                &mut proof,
                1,
                ActionEffects::NotApplied,
                Some(failure(e)),
            )?;
            return Err(e.clone());
        }
    }
    let verified = async {
        let after_refs = fetch::read_refs(&q, pull.head_oid.len()).await?;
        let desired = fetch::apply_reported(
            &expected,
            &[fetch::ReportedRef {
                name: pull.head_ref.clone(),
                old_oid: pull.head_oid.clone(),
                new_oid: fetched.clone(),
                status: ' ',
            }],
            pull.head_oid.len(),
        )?;
        let head = super::changes::run(&q, &argv(&["symbolic-ref", "--quiet", "HEAD"])).await?;
        let tree = super::changes::output(
            &q,
            &argv(&[
                "diff-index",
                "--quiet",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                &fetched,
                "--",
            ]),
        )
        .await?;
        let files = super::changes::output(
            &q,
            &argv(&[
                "diff-files",
                "--quiet",
                "--no-ext-diff",
                "--no-textconv",
                "--",
            ]),
        )
        .await?;
        let config = catalog::configuration(&q).await?;
        let after_stable =
            preflight::capture_stable(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
        Ok::<_, AppError>((
            after_refs == desired
                && head == format!("{}\n", pull.head_ref).as_bytes()
                && tree.success
                && files.success,
            after_refs == expected && after_stable.fingerprint == stable.fingerprint,
            config.digest == proof.config_digest,
        ))
    }
    .await;
    let (applied, unchanged, config_unchanged) = verified.unwrap_or((false, false, false));
    let effects = if applied {
        ActionEffects::Applied
    } else if unchanged {
        ActionEffects::NotApplied
    } else {
        ActionEffects::Unknown
    };
    if applied {
        proof.result.integrated_oid = Some(fetched);
    }
    let native_ok = output.as_ref().is_ok_and(|v| v.success);
    let reason = if applied && native_ok && config_unchanged {
        None
    } else {
        Some(if !config_unchanged {
            SyncFailureReason::Stale
        } else if unchanged {
            SyncFailureReason::Native
        } else {
            SyncFailureReason::Recovery
        })
    };
    observe(&c, &db, &mut proof, 1, effects, reason)?;
    if !applied || !native_ok || !config_unchanged {
        return Err(match output {
            Err(e) => e,
            Ok(o) if !o.success => git_failure(&o.stderr, Phase::Publish),
            _ if !config_unchanged => changed(),
            _ => error(ErrorCode::Interrupted, Phase::Recovery),
        });
    }
    Ok(super::completed(proof.effects(), None))
}
#[cfg(windows)]
pub(crate) async fn execute(
    c: ActionContext,
    database: Arc<crate::persistence::workspace::Database>,
    auth: Arc<crate::auth::AuthBroker>,
    owner: String,
) -> Result<Completion, AppError> {
    let mut q = c.reader()?;
    let PreparedSync {
        mut proof,
        pin,
        pull,
    } = prepare(&q, &c.intent, &c.targets, &c.precondition.fingerprint).await?;
    if let Some(pull) = pull {
        drop(q);
        return execute_pull(c, database, auth, owner, proof, pin, pull).await;
    }
    let before_native = preflight::native_stamp(&c.location)?;
    let before_content = if c.location.bare {
        None
    } else {
        Some(super::changes::capture(&q).await?.digest)
    };
    c.check_owner()?;
    c.scheduler
        .action_sync_begin(&c.operation_id, proof.clone())?;
    if !c.before_write().await? {
        return Ok(super::completed(ActionEffects::NotApplied, None));
    }
    let session = if crate::clone::request::remote_transport(&pin.url).is_ok() {
        Some(auth.session(&owner, &c.operation_id, c.cancel.clone())?)
    } else {
        None
    };
    let mut options = if let Some(s) = &session {
        s.options(
            &std::env::current_exe()
                .map_err(|_| error(ErrorCode::Internal, Phase::Authentication))?,
        )?
    } else {
        RunOptions::default()
    };
    options.environment.extend(pin.environment()?);
    let delete = matches!(c.intent, ActionIntent::RemoteTagDelete { .. });
    let src = if delete {
        ""
    } else {
        proof.source_oid.as_deref().ok_or_else(invalid)?
    };
    let mut args = pin.args();
    args.extend(argv(&[
        "-c",
        "push.followTags=false",
        "push",
        "--porcelain",
        "--no-follow-tags",
        "--no-mirror",
        "--no-force",
        "--recurse-submodules=no",
    ]));
    if delete {
        args.push(
            format!(
                "--force-with-lease={}:{}",
                proof.destination,
                proof.preview_oid.as_deref().ok_or_else(invalid)?
            )
            .into(),
        );
    }
    args.extend(argv(&[
        "--",
        &pin.name,
        &format!("{src}:{}", proof.destination),
    ]));
    let output = native_step(&c, 0, &args, options, before_native, Phase::Transfer).await;
    if let Some(s) = session {
        s.close();
    }
    if output.as_ref().is_err_and(|e| e.phase == Phase::Recovery) {
        return Err(output.err().unwrap());
    }
    let parsed = output
        .as_ref()
        .ok()
        .and_then(|o| parse_push(&o.stdout, src, &proof.destination).ok());
    let ack = output.as_ref().is_ok_and(|o| o.success)
        && if delete {
            parsed == Some('-')
        } else {
            matches!(parsed, Some(' ' | '=' | '*'))
        };
    let rejected = parsed == Some('!');
    let current = super::journal::read(&database, &c.operation_id)?
        .sync_action
        .ok_or_else(invalid)?;
    proof.step_stamps = current.step_stamps;
    if ack {
        let publication = PublicationEvidence {
            remote_name: proof.remote_name.clone(),
            role: RemoteEndpointRole::Push,
            destination: proof.destination.clone(),
            source_ref: proof.source_ref.clone(),
            source_oid: proof.source_oid.clone(),
            acknowledged_oid: if delete {
                None
            } else {
                proof.source_oid.clone()
            },
            acknowledged_at: chrono::Utc::now().to_rfc3339(),
            config_digest: proof.config_digest.clone(),
            deleted: delete,
        };
        proof.result.publication = Some(publication);
        proof.result.tracking_refresh_required = true;
        proof.result.steps[0].phase = FetchStepPhase::Observed;
        proof.result.steps[0].effects = ActionEffects::Applied;
        proof.result.steps[0].observed_at = Some(chrono::Utc::now().to_rfc3339());
        c.sync_observed(proof.clone())?; // Preserve the ACK before any later local inspection can fail.
    }
    q.cancel = Default::default();
    let verification = async {
        let after =
            preflight::capture(q.runner, &c.location, q.lifetime.clone(), &q.cancel).await?;
        let after_content = if c.location.bare {
            None
        } else {
            Some(super::changes::capture(&q).await?.digest)
        };
        Ok::<_, AppError>((after, after_content))
    }
    .await;
    let unchanged = verification.as_ref().is_ok_and(|(after, content)| {
        after.fingerprint == c.precondition.fingerprint && *content == before_content
    });
    let effects = if ack {
        ActionEffects::Applied
    } else if rejected && unchanged {
        ActionEffects::NotApplied
    } else {
        ActionEffects::Unknown
    };
    let error = if !unchanged {
        Some(changed())
    } else if ack {
        None
    } else {
        Some(match output {
            Ok(o) => {
                if rejected {
                    git_failure(&o.stderr, Phase::Transfer)
                } else {
                    error(ErrorCode::Interrupted, Phase::Recovery)
                }
            }
            Err(e) => e,
        })
    };
    proof.result.steps[0] = SyncStepResult {
        step: proof.result.steps[0].step,
        phase: FetchStepPhase::Observed,
        effects,
        failure_reason: error.as_ref().map(|e| {
            if rejected {
                SyncFailureReason::Rejected
            } else {
                failure(e)
            }
        }),
        observed_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    c.sync_observed(proof.clone())?;
    if let Some(e) = error {
        return Err(e);
    }
    Ok(super::completed(
        proof.effects(),
        verification.ok().map(|(after, _)| after.fingerprint),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_pull01_external_switch_after_semantic_check_never_advances_the_other_branch() {
        // The combined native fixture has a large debug async frame; keep the
        // harness stack separate from the application runtime/guard policy.
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(sync_pull01_scenario())
            })
            .unwrap()
            .join()
            .unwrap();
    }
    async fn sync_pull01_scenario() {
        let (dir, b, scope) = crate::backend::action_lifecycle_tests::setup().await;
        let location = b
            .registry
            .location(&scope.repo_id, scope.worktree_id.as_deref())
            .unwrap();
        let root = &location.root;
        let remote = dir.path().join("remote.git");
        let runner = b.git().unwrap();
        async fn run(
            runner: &crate::git::runner::GitRunner,
            root: &std::path::Path,
            args: &[&str],
        ) -> Vec<u8> {
            runner
                .run(
                    Some(root),
                    &argv(args),
                    &Default::default(),
                    Phase::Inspection,
                )
                .await
                .unwrap()
        }
        run(
            runner,
            root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "base",
            ],
        )
        .await;
        run(runner, root, &["push", "origin", "main"]).await;
        run(runner, root, &["config", "branch.main.remote", "origin"]).await;
        run(
            runner,
            root,
            &["config", "branch.main.merge", "refs/heads/main"],
        )
        .await;
        run(runner, root, &["branch", "other"]).await;
        let old = String::from_utf8(run(runner, root, &["rev-parse", "HEAD"]).await)
            .unwrap()
            .trim()
            .to_owned();
        let tree =
            String::from_utf8(run(runner, root, &["rev-parse", "HEAD^{tree}"]).await).unwrap();
        let next = runner
            .output_with_input(
                Some(&remote),
                &argv(&[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit-tree",
                    tree.trim(),
                    "-p",
                    &old,
                ]),
                &Default::default(),
                Phase::Inspection,
                b"remote advance\n",
            )
            .await
            .unwrap();
        assert!(next.success);
        let next = String::from_utf8(next.stdout).unwrap().trim().to_owned();
        run(
            runner,
            &remote,
            &["update-ref", "refs/heads/main", &next, &old],
        )
        .await;
        let catalog = b
            .get_ref_actions(
                "main",
                GetRefActionsRequest {
                    scope: scope.clone(),
                },
            )
            .await
            .unwrap();
        let entry = catalog
            .refs
            .iter()
            .find(|r| r.name == "refs/remotes/origin/main")
            .unwrap();
        let target = b
            .resolve_ref_action_target(
                "main",
                ResolveRefActionTargetRequest {
                    scope: scope.clone(),
                    catalog_id: catalog.catalog_id,
                    source: RefActionSource::Ref {
                        entry_id: entry.entry_id.clone(),
                    },
                },
            )
            .await
            .unwrap();
        let plan = b
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: scope.clone(),
                    intent: ActionIntent::Pull {
                        upstream_handle: target.handle,
                    },
                },
            )
            .await
            .unwrap();
        let pause = Arc::new(TestPause::default());
        *b.actions.sync_test.before_fast_forward.lock().unwrap() = Some(pause.clone());
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
        run(runner, root, &["switch", "other"]).await;
        pause.release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let r = b
                    .get_sync_action_outcome(
                        "main",
                        GetActionOutcomeRequest {
                            operation_id: op.id.clone(),
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
        .unwrap();
        assert_eq!(
            String::from_utf8(run(runner, root, &["rev-parse", "refs/heads/other"]).await)
                .unwrap()
                .trim(),
            old,
            "SYNC-PULL01: an external checkout must not be adopted as the fast-forward target"
        );
        assert_eq!(
            String::from_utf8(run(runner, root, &["rev-parse", "refs/heads/main"]).await)
                .unwrap()
                .trim(),
            old
        );
        assert_eq!(
            run(runner, root, &["symbolic-ref", "HEAD"]).await,
            b"refs/heads/other\n"
        );
        assert_eq!(
            String::from_utf8(run(runner, root, &["rev-parse", "refs/remotes/origin/main"]).await)
                .unwrap()
                .trim(),
            next
        );
        assert_ne!(result.operation.state, OperationState::Succeeded);
        let proof = crate::actions::journal::read(&b.registry.database, &op.id)
            .unwrap()
            .sync_action
            .unwrap();
        assert_eq!(proof.result.steps[0].effects, ActionEffects::Applied);
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Partial
        );
        assert_eq!(proof.result.steps[1].effects, ActionEffects::NotApplied);
        assert_eq!(
            proof.result.steps[1].failure_reason,
            Some(SyncFailureReason::Stale)
        );
        assert!(
            proof.step_stamps[1].is_none(),
            "the FF process must never cross its before-resume step checkpoint"
        );
    }
}
