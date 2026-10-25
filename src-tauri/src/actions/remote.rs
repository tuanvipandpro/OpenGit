//! Read-only authenticated advertisements. URLs/capabilities never enter the journal.
use super::{
    authority::{ExecutionTicket, Target, TargetKind},
    catalog::{self, ReadQuery},
    journal::SafeTargetProof,
};
use crate::{
    domain::contracts::*,
    git::{
        error::{error, git_failure},
        runner::{argv, RunOptions},
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(60);
const BYTES: usize = 16 * 1024 * 1024;
// Inside BYTES: the 64-owner table can retain buckets after revoke/expiry.
// This fixed reserve also covers the manager and hash-table control/alignment.
const MANAGER_BYTES: usize = 256 * 1024;
pub(crate) fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Verification)
}
fn limit() -> AppError {
    error(ErrorCode::OutputLimit, Phase::Inspection)
}
pub(crate) struct Endpoint {
    pub name: String,
    pub role: RemoteEndpointRole,
    pub url: Option<String>,
    pub digest: String,
    pub capability: RemoteEndpointCapability,
}
impl Endpoint {
    pub fn identity(&self) -> Vec<u8> {
        format!(
            "{}:{}",
            if self.role == RemoteEndpointRole::Fetch {
                "fetch"
            } else {
                "push"
            },
            self.name
        )
        .into_bytes()
    }
    pub fn target(&self) -> Target {
        Target {
            kind: TargetKind::RemoteEndpoint,
            identity: self.identity(),
            proof: Some(SafeTargetProof::Remote {
                name: self.name.clone(),
                config_digest: self.digest.clone(),
            }),
        }
    }
}
pub(crate) async fn endpoints(q: &ReadQuery<'_>) -> Result<Vec<Endpoint>, AppError> {
    let cfg = catalog::configuration(q).await?;
    let raw = q.run(&argv(&["config", "--null", "--list"])).await?;
    if raw.len() > super::fetch::CONFIG_BYTES_MAX || super::preflight::digest(&raw) != cfg.digest {
        return Err(error(ErrorCode::ExternalChange, Phase::Validation));
    }
    let custom = std::str::from_utf8(&raw)
        .map_err(|_| malformed())?
        .split('\0')
        .filter_map(|v| v.split_once('\n'))
        .filter_map(|(k, _)| k.strip_prefix("remote.")?.rsplit_once('.'))
        .filter(|(_, field)| *field == "push")
        .map(|(n, _)| n.to_owned())
        .collect::<HashSet<_>>();
    let mut result = Vec::new();
    for remote in cfg.remotes {
        for role in [RemoteEndpointRole::Fetch, RemoteEndpointRole::Push] {
            let mut args = argv(&["remote", "get-url"]);
            if role == RemoteEndpointRole::Push {
                args.push("--push".into());
            }
            args.extend(argv(&["--all", "--", &remote.name]));
            let output = q.output(&args).await?;
            let mut capability = RemoteEndpointCapability::UnsupportedTransport;
            let mut url = None;
            if output.success && output.stderr.is_empty() {
                if output.stdout.len() > 128 * 4097 {
                    return Err(limit());
                }
                let text = std::str::from_utf8(&output.stdout).map_err(|_| malformed())?;
                if !text.ends_with('\n') {
                    return Err(malformed());
                }
                let urls = text.lines().collect::<Vec<_>>();
                if urls.len() != 1 {
                    capability = RemoteEndpointCapability::MultipleUrls;
                } else if valid_url(urls[0]) {
                    url = Some(urls[0].to_owned());
                    capability = RemoteEndpointCapability::Eligible;
                }
            }
            if remote.capability == FetchCapability::Mirror {
                capability = RemoteEndpointCapability::Mirror;
            } else if role == RemoteEndpointRole::Push && custom.contains(&remote.name) {
                capability = RemoteEndpointCapability::UnsupportedRefspec;
            }
            result.push(Endpoint {
                name: remote.name.clone(),
                role,
                url,
                digest: cfg.digest.clone(),
                capability,
            });
        }
    }
    Ok(result)
}
fn valid_url(v: &str) -> bool {
    if v.is_empty() || v.len() > 4096 || v.chars().any(char::is_control) {
        return false;
    }
    crate::clone::request::remote_transport(v).is_ok()
        || (std::path::Path::new(v).is_absolute() && !v.contains("::"))
}
pub(crate) fn select<'a>(
    endpoints: &'a [Endpoint],
    target: &Target,
) -> Result<&'a Endpoint, AppError> {
    let Some(SafeTargetProof::Remote {
        name,
        config_digest,
    }) = &target.proof
    else {
        return Err(invalid());
    };
    endpoints
        .iter()
        .find(|e| {
            target.kind == TargetKind::RemoteEndpoint
                && e.name == *name
                && e.digest == *config_digest
                && e.identity() == target.identity
                && e.capability == RemoteEndpointCapability::Eligible
        })
        .ok_or_else(|| error(ErrorCode::ExternalChange, Phase::Validation))
}
pub(crate) fn parse(bytes: &[u8]) -> Result<(Vec<RemoteRefEntry>, Option<String>), AppError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(limit());
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    let mut refs = Vec::new();
    let mut names = HashSet::new();
    let mut length = None;
    for line in text.lines() {
        if refs.len() >= 4096 {
            return Err(limit());
        }
        let (oid, name) = line.split_once('\t').ok_or_else(malformed)?;
        if !catalog::valid_oid(oid, oid.len())
            || oid.bytes().all(|b| b == b'0')
            || !["refs/heads/", "refs/tags/"]
                .iter()
                .any(|p| name.starts_with(p))
            || name.len() > 1024
            || !crate::history::refs::valid_name(name)
            || !names.insert(name)
            || length.is_some_and(|n| n != oid.len())
        {
            return Err(malformed());
        }
        length = Some(oid.len());
        refs.push(RemoteRefEntry {
            entry_id: format!("entry-{}", refs.len()),
            name: name.into(),
            oid: oid.into(),
        });
    }
    Ok((
        refs,
        length.map(|n| if n == 40 { "sha1" } else { "sha256" }.into()),
    ))
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub wire: RemoteRefSnapshot,
    pub operation_id: String,
    pub owner: String,
    pub ticket: ExecutionTicket,
    pub stamp: String,
    pub config_digest: String,
    pub expires: Instant,
}
impl Snapshot {
    fn weight(&self) -> usize {
        // Account storage plus the largest simultaneous response clone, allocator/container framing.
        let w = &self.wire;
        2 * (std::mem::size_of::<Self>()
            + w.refs.capacity() * std::mem::size_of::<RemoteRefEntry>()
            + w.refs
                .iter()
                .map(|r| r.entry_id.capacity() + r.name.capacity() + r.oid.capacity())
                .sum::<usize>()
            + w.captured_scope.repo_id.capacity()
            + w.captured_scope
                .worktree_id
                .as_ref()
                .map_or(0, String::capacity)
            + w.captured_scope.generation.capacity()
            + w.snapshot_id.capacity()
            + w.remote_name.capacity()
            + w.observed_at.capacity()
            + w.expires_at.capacity()
            + w.object_format.as_ref().map_or(0, String::capacity)
            + self.owner.capacity()
            + self.operation_id.capacity()
            + self.stamp.capacity()
            + self.config_digest.capacity()
            + self.ticket.bytes()
            + 2048)
    }
}
#[derive(Default)]
pub(crate) struct Snapshots(Mutex<HashMap<String, Snapshot>>);
impl Snapshots {
    pub fn publish(&self, value: Snapshot) -> Result<(), AppError> {
        catalog::bounded_response(&value.wire)?;
        let mut s = self.0.lock().map_err(|_| invalid())?;
        s.retain(|_, v| v.expires > Instant::now());
        let weight = s
            .iter()
            .filter(|(w, _)| **w != value.owner)
            .map(|(_, v)| v.weight())
            .sum::<usize>()
            .checked_add(MANAGER_BYTES)
            .and_then(|bytes| bytes.checked_add(value.weight()))
            .ok_or_else(limit)?;
        if (!s.contains_key(&value.owner) && s.len() >= 64) || weight > BYTES {
            return Err(limit());
        }
        s.insert(value.owner.clone(), value);
        Ok(())
    }
    pub fn get(&self, owner: &str, id: &str) -> Result<Snapshot, AppError> {
        let s = self.0.lock().map_err(|_| invalid())?;
        let v = s.get(owner).ok_or_else(invalid)?;
        if v.wire.snapshot_id != id || v.expires <= Instant::now() {
            return Err(invalid());
        }
        Ok(v.clone())
    }
    /// Inspect only the bounded owner ticket before acquiring the read lease;
    /// a full payload clone must remain behind the normal read capacity gate.
    pub fn ticket(&self, owner: &str, id: &str) -> Result<ExecutionTicket, AppError> {
        let state = self.0.lock().map_err(|_| invalid())?;
        let value = state.get(owner).ok_or_else(invalid)?;
        if value.wire.snapshot_id != id || value.expires <= Instant::now() {
            return Err(invalid());
        }
        Ok(value.ticket.clone())
    }
    pub fn validate(&self, owner: &str, id: &str) -> Result<(), AppError> {
        let state = self.0.lock().map_err(|_| invalid())?;
        let value = state.get(owner).ok_or_else(invalid)?;
        if value.wire.snapshot_id != id || value.expires <= Instant::now() {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn outcome(&self, owner: &str, op: &str) -> Option<Snapshot> {
        self.0
            .lock()
            .ok()?
            .get(owner)
            .filter(|v| v.operation_id == op && v.expires > Instant::now())
            .cloned()
    }
    pub fn revoke(&self, owner: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.remove(owner);
        }
    }
    pub fn revoke_repo(&self, repo: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.retain(|_, v| v.wire.captured_scope.repo_id != repo);
        }
    }
}
#[cfg(windows)]
pub(crate) type Publisher = Arc<dyn Fn(Snapshot) -> Result<(), AppError> + Send + Sync>;
#[cfg(windows)]
pub(crate) async fn execute(
    context: super::ActionContext,
    auth: Arc<crate::auth::AuthBroker>,
    owner: String,
    publish: Publisher,
) -> Result<crate::operations::scheduler::Completion, AppError> {
    let q = context.reader()?;
    let endpoints = endpoints(&q).await?;
    let endpoint = select(&endpoints, context.targets.first().ok_or_else(invalid)?)?;
    let alias = format!("opengit-captured-{}:", crate::history::sessions::token());
    let url = endpoint.url.as_deref().ok_or_else(invalid)?;
    let mut environment = super::fetch::alias_environment(&alias, url)?;
    let session = if crate::clone::request::remote_transport(url).is_ok() {
        Some(auth.session(&owner, &context.operation_id, context.cancel.clone())?)
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
    options.environment.append(&mut environment);
    let ssh_available = session.as_ref().is_none_or(|s| s.ssh_prompt_available());
    context.check_owner()?;
    if !context.before_remote_read(endpoint.role)? {
        return Err(error(ErrorCode::Interrupted, Phase::Transfer));
    }
    let output = context
        .remote_read_output(
            &argv(&[
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                "ls-remote",
                "--quiet",
                "--refs",
                "--upload-pack=git-upload-pack",
                "--",
                &alias,
                "refs/heads/*",
                "refs/tags/*",
            ]),
            options,
        )
        .await;
    if let Some(s) = session {
        s.close();
    }
    // A failed owned shutdown is never credited as a harmless completed read.
    if output
        .as_ref()
        .is_err_and(|e| e.code == ErrorCode::Interrupted && e.phase == Phase::Recovery)
    {
        return Err(output.err().unwrap());
    }
    let verify =
        catalog::ReadQuery::new(q.runner, q.location, q.lifetime.clone(), Default::default());
    let after = super::preflight::capture(
        verify.runner,
        verify.location,
        verify.lifetime.clone(),
        &verify.cancel,
    )
    .await?;
    if after.fingerprint != context.precondition.fingerprint
        || catalog::configuration(&verify).await?.digest != endpoint.digest
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Verification));
    }
    let result = output.and_then(|out| {
        if !out.success {
            Err(git_failure(&out.stderr, Phase::Transfer))
        } else if !out.stderr.is_empty() {
            Err(malformed())
        } else {
            parse(&out.stdout)
        }
    });
    match result {
        Err(mut e) => {
            context.remote_read_observed(None, None)?;
            if endpoint.url.as_ref().is_some_and(|u| {
                matches!(
                    crate::clone::request::remote_transport(u),
                    Ok(crate::clone::request::RemoteTransport::Ssh)
                )
            }) && !ssh_available
            {
                e = crate::git::error::ssh_prompt_unavailable(e);
            }
            Err(e)
        }
        Ok((refs, object_format)) => {
            context.check_owner()?;
            let digest =
                super::preflight::digest(&serde_json::to_vec(&refs).map_err(|_| malformed())?);
            context.remote_read_observed(Some(refs.len()), Some(digest))?;
            let now = chrono::Utc::now();
            let wire = RemoteRefSnapshot {
                captured_scope: context.scope.clone(),
                snapshot_id: crate::history::sessions::token(),
                remote_name: endpoint.name.clone(),
                role: endpoint.role,
                observed_at: now.to_rfc3339(),
                expires_at: (now + chrono::Duration::seconds(60)).to_rfc3339(),
                object_format,
                refs,
            };
            publish(Snapshot {
                wire,
                operation_id: context.operation_id.clone(),
                owner,
                ticket: context.ticket.clone(),
                stamp: after.fingerprint.clone(),
                config_digest: endpoint.digest.clone(),
                expires: Instant::now() + TTL,
            })?;
            context.check_owner()?;
            Ok(super::completed(
                ActionEffects::NotApplied,
                Some(after.fingerprint),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advertisement_is_complete_bounded_raw_oid_not_peeled_or_pseudoref() {
        for n in [40, 64] {
            let oid = "a".repeat(n);
            let (refs,format)=parse(format!("{oid}\trefs/heads/foo\n{oid}\trefs/heads/foo.bar\n{oid}\trefs/tags/annotated\n").as_bytes()).unwrap();
            assert_eq!(refs.len(), 3);
            assert_eq!(
                format.as_deref(),
                Some(if n == 40 { "sha1" } else { "sha256" })
            );
            assert_ne!(refs[0].entry_id, refs[1].entry_id);
            for raw in [
                format!("{oid}\tHEAD\n"),
                format!("{oid}\trefs/tags/tag^{{}}\n"),
                format!("{oid}\trefs/heads/main"),
                format!("{oid}\trefs/heads/main\n{oid}\trefs/heads/main\n"),
                format!("{}\trefs/heads/main\n", "0".repeat(n)),
            ] {
                assert!(parse(raw.as_bytes()).is_err());
            }
        }
        assert_eq!(parse(b"").unwrap(), (Vec::new(), None));
        assert!(parse(
            format!(
                "{}\trefs/heads/a\n{}\trefs/heads/b\n",
                "a".repeat(40),
                "b".repeat(64)
            )
            .as_bytes()
        )
        .is_err());
        let large = (0..4097)
            .map(|i| format!("{}\trefs/heads/n{i}\n", "a".repeat(40)))
            .collect::<String>();
        assert_eq!(
            parse(large.as_bytes()).unwrap_err().code,
            ErrorCode::OutputLimit
        );
    }
    #[test]
    fn transport_gate_rejects_secret_bearing_or_helper_urls() {
        for url in [
            "https://u:secret@example.invalid/r",
            "https://token@example.invalid/r",
            "https://example.invalid/r?token=s",
            "ext::anything",
            "git://example.invalid/r",
            "http://example.invalid/r",
            "ssh://-bad@example.invalid/r",
        ] {
            assert!(!valid_url(url), "{url}");
        }
        assert!(valid_url("https://example.invalid/r"));
        assert!(valid_url("ssh://git@example.invalid/r"));
    }
    fn sample(owner: &str, n: usize) -> Snapshot {
        let scope = ActionScope {
            repo_id: "repo".into(),
            worktree_id: None,
            generation: "1".into(),
        };
        let now = chrono::Utc::now();
        Snapshot {
            ticket: ExecutionTicket::fixture(&scope),
            wire: RemoteRefSnapshot {
                captured_scope: scope,
                snapshot_id: crate::history::sessions::token(),
                remote_name: "origin".into(),
                role: RemoteEndpointRole::Push,
                observed_at: now.to_rfc3339(),
                expires_at: (now + chrono::Duration::seconds(60)).to_rfc3339(),
                object_format: Some("sha1".into()),
                refs: (0..n)
                    .map(|i| RemoteRefEntry {
                        entry_id: format!("entry-{i}"),
                        name: format!("refs/heads/{}-{i}", "a".repeat(900)),
                        oid: "a".repeat(40),
                    })
                    .collect(),
            },
            owner: owner.into(),
            operation_id: "operation".into(),
            stamp: "a".repeat(64),
            config_digest: "b".repeat(64),
            expires: Instant::now() + TTL,
        }
    }
    #[test]
    fn snapshots_expire_replace_owner_and_charge_actual_container_payload_before_publication() {
        let store = Snapshots::default();
        let one = sample("main", 1);
        let id = one.wire.snapshot_id.clone();
        store.publish(one).unwrap();
        assert!(store.ticket("main", &id).is_ok());
        assert!(store.ticket("other", &id).is_err());
        assert!(store.get("other", &id).is_err());
        let two = sample("main", 1);
        let id2 = two.wire.snapshot_id.clone();
        store.publish(two).unwrap();
        assert!(store.ticket("main", &id).is_err());
        assert!(store.get("main", &id).is_err());
        store.0.lock().unwrap().get_mut("main").unwrap().expires =
            Instant::now() - Duration::from_millis(1);
        assert!(store.get("main", &id2).is_err());
        assert!(store.ticket("main", &id2).is_err());
        assert!(store.outcome("main", "operation").is_none());
        for i in 0..64 {
            store.publish(sample(&format!("window{i}"), 1)).unwrap();
        }
        assert_eq!(
            store.publish(sample("overflow", 1)).unwrap_err().code,
            ErrorCode::OutputLimit
        );
        store.revoke_repo("repo");
        assert!(store.0.lock().unwrap().is_empty());
        let large = sample("main", 4096);
        let weight = large.weight();
        assert!(weight > 8 * 1024 * 1024);
        store.publish(large).unwrap();
        let failed = sample("other", 4096);
        assert!(store.publish(failed).is_err());
        assert_eq!(store.0.lock().unwrap().len(), 1);
        assert!(
            store
                .0
                .lock()
                .unwrap()
                .values()
                .map(Snapshot::weight)
                .sum::<usize>()
                + MANAGER_BYTES
                <= BYTES
        );
        store.revoke("main");
        assert!(store.publish(sample("other", 4096)).is_ok());
    }
    #[test]
    fn retained_table_reserve_reduces_payload_budget_and_rejection_is_atomic() {
        let store = Snapshots::default();
        let mut high_water_capacity = 0;
        for cycle in 0..32 {
            for i in 0..64 {
                store
                    .publish(sample(&format!("window{cycle}-{i}"), 1))
                    .unwrap();
                high_water_capacity = high_water_capacity.max(store.0.lock().unwrap().capacity());
            }
            store.revoke_repo("repo");
        }
        assert!((64..=256).contains(&high_water_capacity));
        {
            let state = store.0.lock().unwrap();
            assert!(state.is_empty());
            assert!(state.capacity() > 0 && state.capacity() <= high_water_capacity);
            // Erased buckets can reduce reported usable capacity without
            // shrinking the allocation. Use the allocation high-water mark;
            // allow32 bytes/bucket for control/alignment plus the inline entry.
            let buckets = (high_water_capacity + 1).next_power_of_two();
            let retained = buckets * (std::mem::size_of::<(String, Snapshot)>() + 32)
                + std::mem::size_of::<Snapshots>();
            assert!(retained <= MANAGER_BYTES);
        }
        fn over_payload(owner: &str) -> Snapshot {
            let mut value = sample(owner, 1);
            let desired = BYTES - MANAGER_BYTES + 1024;
            let extra = (desired - value.weight()).div_ceil(2);
            let capacity = value.stamp.capacity() + extra;
            value.stamp.reserve_exact(capacity - value.stamp.len());
            // Wire data remains tiny; retained capacity must still be charged.
            assert!(value.weight() > BYTES - MANAGER_BYTES);
            assert!(value.weight() < BYTES - 32 * 1024);
            value
        }
        let survivor = sample("main", 1);
        let id = survivor.wire.snapshot_id.clone();
        store.publish(survivor).unwrap();
        let capacity = store.0.lock().unwrap().capacity();
        for owner in ["other", "main"] {
            assert_eq!(
                store.publish(over_payload(owner)).unwrap_err().code,
                ErrorCode::OutputLimit
            );
            assert!(store.validate("main", &id).is_ok());
            let state = store.0.lock().unwrap();
            assert_eq!(state.len(), 1);
            assert_eq!(state.capacity(), capacity);
            assert!(state.values().map(Snapshot::weight).sum::<usize>() + MANAGER_BYTES <= BYTES);
        }
        store.revoke("main");
        assert!(store.publish(over_payload("other")).is_err());
        assert!(store.0.lock().unwrap().is_empty());
        store.publish(sample("other", 1)).unwrap();
    }
}
