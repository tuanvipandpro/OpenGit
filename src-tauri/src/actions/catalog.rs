//! Bounded configuration and ref catalog. Raw configuration is hashed, never persisted.
use super::{fetch, preflight};
use crate::{
    domain::contracts::*,
    git::{
        error::{error, git_failure},
        runner::{argv, Cancellation, GitOutput, GitRunner, RunOptions},
    },
    repositories::registry::RepositoryLocation,
};
use std::{
    ffi::OsString,
    sync::Arc,
    time::{Duration, Instant},
};

pub(crate) struct ReadQuery<'a> {
    pub runner: &'a GitRunner,
    pub location: &'a RepositoryLocation,
    pub lifetime: Arc<dyn Send + Sync>,
    pub cancel: Cancellation,
    pub deadline: Instant,
}
impl<'a> ReadQuery<'a> {
    pub fn new(
        runner: &'a GitRunner,
        location: &'a RepositoryLocation,
        lifetime: Arc<dyn Send + Sync>,
        cancel: Cancellation,
    ) -> Self {
        Self {
            runner,
            location,
            lifetime,
            cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    pub async fn output(&self, args: &[OsString]) -> Result<GitOutput, AppError> {
        self.output_protocol(args, "").await
    }
    pub(crate) async fn output_protocol(
        &self,
        args: &[OsString],
        protocols: &str,
    ) -> Result<GitOutput, AppError> {
        self.output_environment(args, protocols, vec![]).await
    }
    pub(crate) async fn output_environment(
        &self,
        args: &[OsString],
        protocols: &str,
        mut environment: Vec<(OsString, OsString)>,
    ) -> Result<GitOutput, AppError> {
        let mut runner = self.runner.clone();
        runner.timeout = runner.timeout.min(
            self.deadline
                .checked_duration_since(Instant::now())
                .filter(|v| !v.is_zero())
                .ok_or_else(|| error(ErrorCode::Timeout, Phase::Inspection))?,
        );
        runner.output_limit = runner.output_limit.min(8 * 1024 * 1024);
        let mut all = preflight::global_args(self.location)?;
        all.extend_from_slice(args);
        environment.extend([
            ("GIT_ALLOW_PROTOCOL".into(), protocols.into()),
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ]);
        runner
            .output_history(
                Some(&self.location.root),
                &all,
                &self.cancel,
                Phase::Inspection,
                RunOptions {
                    environment,
                    lifetime_guard: Some(self.lifetime.clone()),
                    ..Default::default()
                },
                None,
            )
            .await
    }
    pub async fn run(&self, args: &[OsString]) -> Result<Vec<u8>, AppError> {
        let output = self.output(args).await?;
        if !output.success {
            return Err(git_failure(&output.stderr, Phase::Inspection));
        }
        if !output.stderr.is_empty() {
            return Err(malformed());
        }
        Ok(output.stdout)
    }
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn limit() -> AppError {
    error(ErrorCode::OutputLimit, Phase::Inspection)
}
pub(crate) fn bounded_response<T: serde::Serialize>(value: &T) -> Result<(), AppError> {
    struct Budget(usize);
    impl std::io::Write for Budget {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("limit"))?;
            if self.0 > 8 * 1024 * 1024 {
                return Err(std::io::Error::other("limit"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[derive(serde::Serialize)]
    struct Success<'a, T> {
        ok: bool,
        value: &'a T,
    }
    serde_json::to_writer(Budget(0), &Success { ok: true, value }).map_err(|_| limit())
}
pub(crate) fn valid_oid(value: &str, length: usize) -> bool {
    matches!(length, 40 | 64)
        && value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
pub(crate) struct Branches {
    pub branches: Vec<ActionBranch>,
    pub symbolic: Vec<SymbolicBranch>,
    pub upstreams: std::collections::HashMap<String, String>,
}
pub(crate) fn parse_branches(bytes: &[u8], oid_length: usize) -> Result<Branches, AppError> {
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(malformed());
    }
    let mut result = Branches {
        branches: Vec::new(),
        symbolic: Vec::new(),
        upstreams: Default::default(),
    };
    let mut names = std::collections::HashSet::new();
    for line in bytes.split(|c| *c == b'\n').filter(|v| !v.is_empty()) {
        if names.len() >= fetch::REFS_MAX {
            return Err(limit());
        }
        let fields = line
            .split(|c| *c == 0)
            .map(|v| std::str::from_utf8(v).map_err(|_| malformed()))
            .collect::<Result<Vec<_>, _>>()?;
        if fields.len() != 5 || !fields[4].is_empty() {
            return Err(malformed());
        }
        let [name, oid, symref, upstream, _] = fields.as_slice() else {
            return Err(malformed());
        };
        if !crate::history::refs::valid_name(name)
            || !valid_oid(oid, oid_length)
            || !names.insert((*name).to_owned())
        {
            return Err(malformed());
        }
        let kind = if name.starts_with("refs/heads/") {
            ActionBranchKind::Local
        } else if name.starts_with("refs/remotes/") {
            ActionBranchKind::Remote
        } else {
            return Err(malformed());
        };
        if !upstream.is_empty() {
            if !crate::history::refs::valid_name(upstream) {
                return Err(malformed());
            }
            result.upstreams.insert((*name).into(), (*upstream).into());
        }
        if !symref.is_empty() {
            if !crate::history::refs::valid_name(symref) {
                return Err(malformed());
            }
            result.symbolic.push(SymbolicBranch {
                name: (*name).into(),
                target: (*symref).into(),
            });
        } else {
            result.branches.push(ActionBranch {
                name: (*name).into(),
                oid: (*oid).into(),
                kind,
                observation: None,
            });
        }
    }
    Ok(result)
}
pub(crate) async fn branches(
    query: &ReadQuery<'_>,
    oid_length: usize,
) -> Result<Branches, AppError> {
    parse_branches(
        &query
            .run(&argv(&[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00%(symref)%00%(upstream)%00",
                "refs/heads/",
                "refs/remotes/",
            ]))
            .await?,
        oid_length,
    )
}
pub(crate) async fn head(
    query: &ReadQuery<'_>,
    branches: &Branches,
) -> Result<(Head, usize), AppError> {
    let format = query
        .run(&argv(&["rev-parse", "--show-object-format"]))
        .await?;
    let length = match format.as_slice() {
        b"sha1\n" => 40,
        b"sha256\n" => 64,
        _ => return Err(malformed()),
    };
    let symbolic = query
        .output(&argv(&["symbolic-ref", "--quiet", "HEAD"]))
        .await?;
    if !symbolic.stderr.is_empty() {
        return Err(malformed());
    }
    if symbolic.success {
        let name = std::str::from_utf8(&symbolic.stdout)
            .map_err(|_| malformed())?
            .trim_end_matches(['\r', '\n']);
        if !name.starts_with("refs/heads/") || !crate::history::refs::valid_name(name) {
            return Err(malformed());
        }
        let oid = branches
            .branches
            .iter()
            .find(|b| b.name == name)
            .map(|b| b.oid.clone());
        Ok((
            Head {
                kind: if oid.is_some() {
                    HeadKind::Branch
                } else {
                    HeadKind::Unborn
                },
                branch: Some(name[11..].into()),
                oid,
            },
            length,
        ))
    } else {
        if symbolic.exit_code != Some(1) || !symbolic.stdout.is_empty() {
            return Err(malformed());
        }
        let oid = query.run(&argv(&["rev-parse", "--verify", "HEAD"])).await?;
        let oid = std::str::from_utf8(&oid)
            .map_err(|_| malformed())?
            .trim_end_matches(['\r', '\n']);
        if !valid_oid(oid, length) {
            return Err(malformed());
        }
        Ok((
            Head {
                kind: HeadKind::Detached,
                branch: None,
                oid: Some(oid.into()),
            },
            length,
        ))
    }
}
pub(crate) async fn sync(
    query: &ReadQuery<'_>,
    head: &Head,
    branches: &Branches,
    partial: bool,
) -> Result<SyncStatus, AppError> {
    let current_branch = head.branch.as_ref().map(|v| format!("refs/heads/{v}"));
    let upstream = current_branch
        .as_ref()
        .and_then(|name| branches.upstreams.get(name))
        .cloned();
    let mut result = SyncStatus {
        current_branch,
        upstream,
        ahead: None,
        behind: None,
        completeness: AncestryCompleteness::Unavailable,
    };
    let Some(oid) = &head.oid else {
        return Ok(result);
    };
    let Some(upstream) = result
        .upstream
        .as_ref()
        .and_then(|name| branches.branches.iter().find(|b| b.name == *name))
    else {
        return Ok(result);
    };
    if partial {
        return Ok(result);
    }
    let history = match crate::history::ActionHistoryQuery::new_action(
        query.runner,
        query.location,
        query.cancel.clone(),
        query.lifetime.clone(),
    )
    .await
    {
        Ok(value) => value,
        Err(e) if matches!(e.code, ErrorCode::Interrupted | ErrorCode::Timeout) => return Err(e),
        Err(_) => return Ok(result),
    };
    let before = match history.fingerprint().await {
        Ok(value) => value,
        Err(e) if matches!(e.code, ErrorCode::Interrupted | ErrorCode::Timeout) => return Err(e),
        Err(_) => return Ok(result),
    };
    if before.head != *head
        || before
            .refs
            .iter()
            .find(|r| r.name == upstream.name)
            .is_none_or(|r| r.oid != upstream.oid)
    {
        return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
    }
    if !before.shallow.is_empty() {
        result.completeness = AncestryCompleteness::Shallow;
        return Ok(result);
    }
    let counts = match history
        .run(
            &argv(&[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{oid}...{}", upstream.oid),
                "--",
            ]),
            None,
        )
        .await
    {
        Ok(value) => value,
        Err(e) if matches!(e.code, ErrorCode::Interrupted | ErrorCode::Timeout) => return Err(e),
        Err(_) => return Ok(result),
    };
    if history.fingerprint().await? != before {
        return Err(error(ErrorCode::ExternalChange, Phase::Inspection));
    }
    let fields = std::str::from_utf8(&counts)
        .map_err(|_| malformed())?
        .split_whitespace()
        .collect::<Vec<_>>();
    if fields.len() != 2 {
        return Err(malformed());
    }
    let parse = |value: &str| {
        value
            .parse::<u64>()
            .ok()
            .filter(|n| n.to_string() == value)
            .map(|n| n.to_string())
            .ok_or_else(malformed)
    };
    result.ahead = Some(parse(fields[0])?);
    result.behind = Some(parse(fields[1])?);
    result.completeness = AncestryCompleteness::Complete;
    Ok(result)
}

/// URLs/refspecs are process-local executor material, not capabilities or DB data.
pub(crate) struct Remote {
    pub name: String,
    pub url: Option<String>,
    pub refspecs: Vec<String>,
    pub digest: String,
    pub capability: FetchCapability,
    pub ssh: bool,
}
pub(crate) struct Configuration {
    pub remotes: Vec<Remote>,
    pub digest: String,
    pub partial: bool,
}
pub(crate) async fn configuration(query: &ReadQuery<'_>) -> Result<Configuration, AppError> {
    let bytes = query.run(&argv(&["config", "--null", "--list"])).await?;
    if bytes.len() > fetch::CONFIG_BYTES_MAX {
        return Err(limit());
    }
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(malformed());
    }
    let digest = preflight::digest(&bytes);
    let mut remotes = std::collections::BTreeMap::<String, Vec<(String, String)>>::new();
    let mut partial = false;
    for record in bytes.split(|c| *c == 0).filter(|v| !v.is_empty()) {
        let record = std::str::from_utf8(record).map_err(|_| malformed())?;
        let (key, value) = record.split_once('\n').unwrap_or((record, "true"));
        if key == "extensions.partialclone"
            || key.ends_with(".partialclonefilter")
            || (key.ends_with(".promisor")
                && !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "false" | "no" | "off" | "0"
                ))
        {
            partial = true;
        }
        if let Some(key) = key.strip_prefix("remote.") {
            if let Some((name, field)) = key.rsplit_once('.') {
                if !fetch::remote_name(name) {
                    return Err(malformed());
                }
                let fields = remotes.entry(name.into()).or_default();
                if fields.len() >= 128 {
                    return Err(limit());
                }
                fields.push((field.into(), value.into()));
            }
        }
    }
    if remotes.len() > fetch::REMOTES_MAX {
        return Err(limit());
    }
    let mut result = Vec::new();
    for (name, fields) in remotes {
        let refspecs: Vec<String> = fields
            .iter()
            .filter(|(k, _)| k == "fetch")
            .map(|(_, v)| v.clone())
            .collect();
        if refspecs.len() > fetch::REFSPECS_MAX {
            return Err(limit());
        }
        let mut capability = if refspecs.is_empty() {
            FetchCapability::NoRefspec
        } else if refspecs.iter().any(|v| !fetch::safe_refspec(&name, v))
            || !refspecs.iter().any(|v| !v.starts_with('^'))
        {
            FetchCapability::UnsupportedRefspec
        } else {
            FetchCapability::EligibleProbeRequired
        };
        if fields.iter().any(|(k, v)| {
            k == "mirror" && matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
        }) {
            capability = FetchCapability::Mirror;
        }
        // get-url expands insteadOf; the executor pins this result through its own alias.
        let output = query
            .output(&argv(&["remote", "get-url", "--all", "--", &name]))
            .await?;
        let mut url = None;
        let mut ssh = false;
        if output.success && output.stderr.is_empty() {
            let text = std::str::from_utf8(&output.stdout).map_err(|_| malformed())?;
            let urls: Vec<_> = text.lines().collect();
            if urls.len() == 1 && urls[0].len() <= 4096 {
                let value = urls[0];
                if let Ok(transport) = crate::clone::request::remote_transport(value) {
                    ssh = transport == crate::clone::request::RemoteTransport::Ssh;
                    url = Some(value.to_owned());
                } else if std::path::Path::new(value).is_absolute()
                    && !value.chars().any(char::is_control)
                    && !value.contains("::")
                {
                    url = Some(value.to_owned());
                }
            }
        }
        if url.is_none() {
            capability = FetchCapability::UnsupportedTransport;
        }
        // Whole effective configuration is conservative: endpoint/helper/trust/
        // refspec changes invalidate evidence without storing credential-bearing values.
        result.push(Remote {
            name,
            url,
            refspecs,
            digest: digest.clone(),
            capability,
            ssh,
        });
    }
    let overlaps = result
        .iter()
        .filter(|r| {
            result.iter().any(|other| {
                r.name != other.name
                    && (r.name.starts_with(&format!("{}/", other.name))
                        || other.name.starts_with(&format!("{}/", r.name)))
            })
        })
        .map(|r| r.name.clone())
        .collect::<std::collections::HashSet<_>>();
    for remote in &mut result {
        if overlaps.contains(&remote.name) {
            remote.capability = FetchCapability::UnsupportedRefspec;
        }
    }
    Ok(Configuration {
        remotes: result,
        digest,
        partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn symbolic_alias_is_separate_and_equal_oid_branch_names_are_preserved() {
        let oid = "a".repeat(40);
        let bytes = format!("refs/heads/main\0{oid}\0\0refs/remotes/origin/main\0\nrefs/heads/same\0{oid}\0\0\0\nrefs/remotes/origin/HEAD\0{oid}\0refs/remotes/origin/main\0\0\nrefs/remotes/origin/main\0{oid}\0\0\0\n");
        let parsed = parse_branches(bytes.as_bytes(), 40).unwrap();
        assert_eq!(parsed.branches.len(), 3);
        assert_eq!(parsed.symbolic[0].name, "refs/remotes/origin/HEAD");
        assert_eq!(
            parsed.upstreams["refs/heads/main"],
            "refs/remotes/origin/main"
        );
        assert_eq!(parsed.branches[0].oid, parsed.branches[1].oid);
        assert!(parse_branches(bytes.as_bytes(), 64).is_err());
    }
}
