//! Snapshot-bound lazy grants: retain one small capability per window, not ref vectors.
use super::{
    authority::{Target, TargetKind},
    catalog::{self, ReadQuery},
    fetch,
    journal::{ObjectFormat, SafeTargetProof},
};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::argv},
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub(crate) struct Ticket {
    pub scope: ActionScope,
    pub epoch: u64,
    pub stamp: String,
    pub id: String,
    pub expires: Instant,
}
impl Ticket {
    fn weight(&self, window: &str) -> usize {
        std::mem::size_of::<Self>()
            + window.len() * 2
            + self.scope.repo_id.capacity()
            + self.scope.worktree_id.as_ref().map_or(0, String::capacity)
            + self.scope.generation.capacity()
            + self.stamp.capacity()
            + self.id.capacity()
            + 512
    }
}
#[derive(Default)]
pub(crate) struct Catalogs(Mutex<HashMap<String, Ticket>>);
impl Catalogs {
    pub fn publish(
        &self,
        window: &str,
        scope: ActionScope,
        epoch: u64,
        stamp: String,
    ) -> Result<Ticket, AppError> {
        let now = Instant::now();
        let mut s = self.0.lock().map_err(|_| invalid())?;
        s.retain(|_, v| v.expires > now);
        let t = Ticket {
            scope,
            epoch,
            stamp,
            id: crate::history::sessions::token(),
            expires: now + Duration::from_secs(300),
        };
        let weight = s
            .iter()
            .filter(|(k, _)| k.as_str() != window)
            .map(|(k, v)| v.weight(k))
            .sum::<usize>()
            + t.weight(window);
        if (!s.contains_key(window) && s.len() >= 64) || weight > 512 * 1024 {
            return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
        }
        s.insert(window.into(), t.clone());
        Ok(t)
    }
    pub fn revoke(&self, window: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.remove(window);
        }
    }
    pub fn revoke_repo(&self, repo: &str) {
        if let Ok(mut s) = self.0.lock() {
            s.retain(|_, t| t.scope.repo_id != repo);
        }
    }
    pub fn get(&self, window: &str, scope: &ActionScope, id: &str) -> Result<Ticket, AppError> {
        let s = self.0.lock().map_err(|_| invalid())?;
        let t = s.get(window).ok_or_else(invalid)?;
        if &t.scope != scope || t.id != id || t.expires <= Instant::now() {
            return Err(invalid());
        }
        Ok(t.clone())
    }
}
pub(crate) fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
pub(crate) fn supported(kind: ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::BranchCreate
            | ActionKind::BranchRename
            | ActionKind::BranchSwitch
            | ActionKind::BranchDelete
            | ActionKind::BranchTracking
            | ActionKind::TagCreate
            | ActionKind::TagDelete
    )
}
pub(crate) fn object_format(length: usize) -> ObjectFormat {
    if length == 40 {
        ObjectFormat::Sha1
    } else {
        ObjectFormat::Sha256
    }
}
pub(crate) struct Capture {
    pub refs: fetch::RefMap,
    pub entries: Vec<RefActionEntry>,
    pub head: Head,
    pub length: usize,
}
pub(crate) async fn capture(q: &ReadQuery<'_>) -> Result<Capture, AppError> {
    let format = q.run(&argv(&["rev-parse", "--show-object-format"])).await?;
    let length = match format.as_slice() {
        b"sha1\n" => 40,
        b"sha256\n" => 64,
        _ => return Err(error(ErrorCode::MalformedOutput, Phase::Inspection)),
    };
    let refs = fetch::read_refs(q, length).await?;
    let branches = catalog::branches(q, length).await?;
    let (head, _) = catalog::head(q, &branches).await?;
    let raw = q
        .run(&argv(&["worktree", "list", "--porcelain", "-z"]))
        .await?;
    let mut owners: HashMap<String, usize> = HashMap::new();
    if !raw.ends_with(&[0, 0]) {
        return Err(error(ErrorCode::MalformedOutput, Phase::Inspection));
    }
    for field in raw.split(|b| *b == 0) {
        if let Some(name) = field.strip_prefix(b"branch ") {
            let name = std::str::from_utf8(name).map_err(|_| invalid())?;
            if !crate::history::refs::valid_name(name) {
                return Err(invalid());
            }
            *owners.entry(name.into()).or_default() += 1;
        }
    }
    let current = head.branch.as_ref().map(|v| format!("refs/heads/{v}"));
    let types = q
        .run(&argv(&[
            "-c",
            "advice.graftFileDeprecated=false",
            "for-each-ref",
            "--format=%(refname)%00%(objecttype)%00%(*objecttype)%00",
            "refs/tags/",
        ]))
        .await?;
    let mut starts = std::collections::HashSet::new();
    for line in types.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        let f = line.split(|b| *b == 0).collect::<Vec<_>>();
        if f.len() != 4 || !f[3].is_empty() {
            return Err(error(ErrorCode::MalformedOutput, Phase::Inspection));
        }
        if f[1] == b"commit" || (f[1] == b"tag" && f[2] == b"commit") {
            starts.insert(std::str::from_utf8(f[0]).map_err(|_| invalid())?.to_owned());
        }
    }
    let mut entries = Vec::new();
    for (name, r) in &refs {
        let kind = if name.starts_with("refs/heads/") {
            RefActionKind::Local
        } else if name.starts_with("refs/remotes/") {
            RefActionKind::Remote
        } else if name.starts_with("refs/tags/") {
            RefActionKind::Tag
        } else {
            continue;
        };
        let here = current.as_ref() == Some(name);
        let elsewhere =
            owners.get(name).copied().unwrap_or(0) > usize::from(here && !q.location.bare);
        entries.push(RefActionEntry {
            entry_id: format!("entry-{}", entries.len()),
            kind,
            name: name.clone(),
            oid: Some(r.oid.clone()),
            symbolic_target: r.symbolic.clone(),
            checked_out_here: here && !q.location.bare,
            checked_out_elsewhere: elsewhere,
            can_start: r.symbolic.is_none()
                && (kind != RefActionKind::Tag || starts.contains(name)),
        });
    }
    Ok(Capture {
        refs,
        entries,
        head,
        length,
    })
}
pub(crate) fn target(
    entry: &RefActionEntry,
    length: usize,
) -> Result<(Target, RefTargetKind), AppError> {
    if entry.symbolic_target.is_some() {
        return Err(invalid());
    }
    let (kind, wire) = match entry.kind {
        RefActionKind::Local => (TargetKind::Branch, RefTargetKind::Branch),
        RefActionKind::Remote => (TargetKind::RemoteRef, RefTargetKind::RemoteRef),
        RefActionKind::Tag => (TargetKind::Tag, RefTargetKind::Tag),
    };
    Ok((
        Target {
            kind,
            identity: entry.name.as_bytes().to_vec(),
            proof: Some(SafeTargetProof::Ref {
                name: entry.name.clone(),
                expected_oid: entry.oid.clone(),
                object_format: object_format(length),
            }),
        },
        wire,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> ActionScope {
        ActionScope {
            repo_id: "repo".into(),
            worktree_id: None,
            generation: "1".into(),
        }
    }
    #[test]
    fn catalog_owner_replacement_expiry_and_bound_do_not_resurrect() {
        let catalogs = Catalogs::default();
        let first = catalogs
            .publish("main", scope(), 1, "a".repeat(64))
            .unwrap();
        assert!(catalogs.get("other", &scope(), &first.id).is_err());
        let second = catalogs
            .publish("main", scope(), 1, "a".repeat(64))
            .unwrap();
        assert_ne!(first.id, second.id);
        assert!(catalogs.get("main", &scope(), &first.id).is_err());
        catalogs.0.lock().unwrap().get_mut("main").unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(catalogs.get("main", &scope(), &second.id).is_err());
        for n in 0..64 {
            catalogs
                .publish(&format!("window{n}"), scope(), n, "a".repeat(64))
                .unwrap();
        }
        assert!(catalogs
            .publish("overflow", scope(), 1, "a".repeat(64))
            .is_err());
        let state = catalogs.0.lock().unwrap();
        assert!(state.iter().map(|(k, v)| v.weight(k)).sum::<usize>() <= 512 * 1024);
        drop(state);
        catalogs.revoke("window0");
        assert!(catalogs
            .publish("next", scope(), 100, "b".repeat(64))
            .is_ok());
        catalogs.revoke_repo("repo");
        assert!(catalogs.0.lock().unwrap().is_empty());
    }
    #[test]
    fn ref_proofs_do_not_turn_symbolic_aliases_into_mutation_authority() {
        let e = RefActionEntry {
            entry_id: "entry-0".into(),
            kind: RefActionKind::Remote,
            name: "refs/remotes/origin/HEAD".into(),
            oid: Some("a".repeat(40)),
            symbolic_target: Some("refs/remotes/origin/main".into()),
            checked_out_here: false,
            checked_out_elsewhere: false,
            can_start: false,
        };
        assert!(target(&e, 40).is_err());
    }
}
