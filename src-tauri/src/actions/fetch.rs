//! Fetch uses captured backend configuration, never renderer refspecs or URLs.
use crate::{domain::contracts::*, git::error::error};

pub const REMOTES_MAX: usize = 32;
pub const REFS_MAX: usize = 4096;
pub const REFSPECS_MAX: usize = 64;
pub const CONFIG_BYTES_MAX: usize = 1024 * 1024;
#[derive(Default)]
struct CoverageBudget(usize);
impl CoverageBudget {
    fn charge(&mut self, count: usize) -> Result<(), AppError> {
        let next = self
            .0
            .checked_add(count)
            .filter(|n| *n <= REFS_MAX)
            .ok_or_else(|| error(ErrorCode::OutputLimit, Phase::Verification))?;
        self.0 = next;
        Ok(())
    }
}

pub(crate) fn alias_environment(
    alias: &str,
    url: &str,
) -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, AppError> {
    let count = match std::env::var("GIT_CONFIG_COUNT") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|n| *n < 128 && n.to_string() == value)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, Phase::Validation))?,
        Err(std::env::VarError::NotPresent) => 0,
        Err(_) => return Err(error(ErrorCode::InvalidRequest, Phase::Validation)),
    };
    // Append rather than replace inherited command config (which can contain
    // trust/helper settings). Exact keys also support '=' and Unicode in URLs.
    Ok(vec![
        ("GIT_CONFIG_COUNT".into(), (count + 1).to_string().into()),
        (
            format!("GIT_CONFIG_KEY_{count}").into(),
            format!("url.{url}.insteadOf").into(),
        ),
        (format!("GIT_CONFIG_VALUE_{count}").into(), alias.into()),
    ])
}
pub(crate) async fn capability(
    query: &super::catalog::ReadQuery<'_>,
    oid_length: usize,
) -> Result<(), AppError> {
    use crate::git::runner::argv;
    let namespace = format!("opengit-capability-{}", uuid::Uuid::new_v4().simple());
    let alias = format!("opengit-captured-{}:", crate::history::sessions::token());
    let source = super::path_text(&query.location.common)?
        .into_string()
        .map_err(|_| error(ErrorCode::UnsafePath, Phase::Validation))?;
    let args = argv(&[
        "-c",
        "fetch.writeCommitGraph=false",
        "-c",
        "gc.auto=0",
        "-c",
        "maintenance.auto=false",
        "fetch",
        "--porcelain",
        "--verbose",
        "--dry-run",
        "--atomic",
        "--no-write-fetch-head",
        "--no-prune",
        "--no-prune-tags",
        "--no-tags",
        "--no-recurse-submodules",
        "--no-auto-maintenance",
        "--upload-pack=git-upload-pack",
        "--",
        &alias,
        &format!("+refs/heads/*:refs/remotes/{namespace}/*"),
    ]);
    let output = query
        .output_environment(&args, "file", alias_environment(&alias, &source)?)
        .await?;
    if !output.success {
        return Err(
            if output.exit_code == Some(129) && output.stdout.is_empty() {
                super::safe_error(&error(ErrorCode::UnsupportedGit, Phase::Validation))
            } else {
                crate::git::error::git_failure(&output.stderr, Phase::Inspection)
            },
        );
    }
    parse_porcelain(&output.stdout, &namespace, oid_length)?;
    Ok(())
}

pub(crate) fn remote_name(name: &str) -> bool {
    !name.starts_with('-')
        && crate::history::refs::valid_name(&format!("refs/remotes/{name}/target"))
}

/// Restrict destinations to this remote's tracking namespace. In particular,
/// tags, local branches, mirror mappings and implicit FETCH_HEAD-only fetches
/// do not become an unreviewed mutation mode.
pub(crate) fn safe_refspec(remote: &str, value: &str) -> bool {
    if value.len() > 2048 || !remote_name(remote) {
        return false;
    }
    let valid = |name: &str| {
        name.matches('*').count() <= 1
            && crate::history::refs::valid_name(&name.replace('*', "probe"))
    };
    if let Some(source) = value.strip_prefix('^') {
        return source.starts_with("refs/heads/") && valid(source);
    }
    let value = value.strip_prefix('+').unwrap_or(value);
    let Some((source, destination)) = value.split_once(':') else {
        return false;
    };
    source.starts_with("refs/heads/")
        && destination.starts_with(&format!("refs/remotes/{remote}/"))
        && valid(source)
        && valid(destination)
        && source.matches('*').count() == destination.matches('*').count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportedRef {
    pub name: String,
    pub old_oid: String,
    pub new_oid: String,
    pub status: char,
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Verification)
}
fn valid_oid(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Porcelain is parsed only after the process has a proved exit. Error output
/// never becomes a ref observation; successful '=' records do count as coverage.
pub(crate) fn parse_porcelain(
    bytes: &[u8],
    remote: &str,
    oid_length: usize,
) -> Result<Vec<ReportedRef>, AppError> {
    if !matches!(oid_length, 40 | 64) || (!bytes.is_empty() && !bytes.ends_with(b"\n")) {
        return Err(malformed());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    let mut refs = Vec::new();
    let mut names = std::collections::HashSet::new();
    for line in text.lines() {
        if refs.len() >= REFS_MAX {
            return Err(error(ErrorCode::OutputLimit, Phase::Verification));
        }
        let status = line.as_bytes().first().copied().ok_or_else(malformed)? as char;
        if line.as_bytes().get(1) != Some(&b' ') || !matches!(status, '*' | '+' | '-' | '=' | ' ') {
            return Err(malformed());
        }
        let fields: Vec<_> = line[2..].split(' ').collect();
        if fields.len() != 3
            || !valid_oid(fields[0], oid_length)
            || !valid_oid(fields[1], oid_length)
            || !fields[2].starts_with(&format!("refs/remotes/{remote}/"))
            || !crate::history::refs::valid_name(fields[2])
            || !names.insert(fields[2].to_owned())
        {
            return Err(malformed());
        }
        let zero = "0".repeat(oid_length);
        if (status == '=' && fields[0] != fields[1])
            || (status == '*' && fields[0] != zero)
            || (status == '-' && fields[1] != zero)
            || (status != '*' && fields[0] == zero)
            || (status != '-' && fields[1] == zero)
        {
            return Err(malformed());
        }
        refs.push(ReportedRef {
            name: fields[2].into(),
            old_oid: fields[0].into(),
            new_oid: fields[1].into(),
            status,
        });
    }
    Ok(refs)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct RefState {
    pub oid: String,
    pub symbolic: Option<String>,
}
pub(crate) type RefMap = std::collections::BTreeMap<String, RefState>;
pub(crate) async fn read_refs(
    query: &super::catalog::ReadQuery<'_>,
    oid_length: usize,
) -> Result<RefMap, AppError> {
    let raw = query
        .run(&crate::git::runner::argv(&[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)%00",
        ]))
        .await?;
    if !raw.is_empty() && !raw.ends_with(b"\n") {
        return Err(malformed());
    }
    let mut result = RefMap::new();
    for line in raw.split(|c| *c == b'\n').filter(|l| !l.is_empty()) {
        if result.len() >= REFS_MAX {
            return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
        }
        let fields = line
            .split(|c| *c == 0)
            .map(|v| std::str::from_utf8(v).map_err(|_| malformed()))
            .collect::<Result<Vec<_>, _>>()?;
        if fields.len() != 4
            || !fields[3].is_empty()
            || !crate::history::refs::valid_name(fields[0])
            || !valid_oid(fields[1], oid_length)
            || (!fields[2].is_empty() && !crate::history::refs::valid_name(fields[2]))
        {
            return Err(malformed());
        }
        if result
            .insert(
                fields[0].into(),
                RefState {
                    oid: fields[1].into(),
                    symbolic: (!fields[2].is_empty()).then(|| fields[2].into()),
                },
            )
            .is_some()
        {
            return Err(malformed());
        }
    }
    super::catalog::bounded_response(&result)?;
    Ok(result)
}
fn refs_digest(refs: &RefMap) -> Result<String, AppError> {
    Ok(super::preflight::digest(
        &serde_json::to_vec(refs).map_err(|_| malformed())?,
    ))
}
pub(crate) fn apply_reported(
    before: &RefMap,
    reported: &[ReportedRef],
    oid_length: usize,
) -> Result<RefMap, AppError> {
    let mut expected = before.clone();
    let zero = "0".repeat(oid_length);
    for item in reported {
        if expected
            .get(&item.name)
            .is_some_and(|r| r.symbolic.is_some())
        {
            return Err(malformed());
        }
        let old = expected
            .get(&item.name)
            .map(|r| r.oid.as_str())
            .unwrap_or(&zero);
        if old != item.old_oid {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        if item.new_oid == zero {
            expected.remove(&item.name);
        } else {
            expected.insert(
                item.name.clone(),
                RefState {
                    oid: item.new_oid.clone(),
                    symbolic: None,
                },
            );
        }
    }
    // Symbolic aliases follow a reported target update; they are not an extra
    // fetched branch and must not receive an independent freshness timestamp.
    let aliases = expected
        .iter()
        .filter_map(|(n, v)| v.symbolic.as_ref().map(|s| (n.clone(), s.clone())))
        .collect::<Vec<_>>();
    for (name, mut target) in aliases {
        let mut seen = std::collections::HashSet::new();
        let oid = loop {
            if !seen.insert(target.clone()) || seen.len() > REFS_MAX {
                return Err(malformed());
            }
            match expected.get(&target) {
                Some(RefState {
                    symbolic: Some(next),
                    ..
                }) => target = next.clone(),
                Some(value) => break Some(value.oid.clone()),
                None => break None,
            }
        };
        if let Some(oid) = oid {
            expected.get_mut(&name).ok_or_else(malformed)?.oid = oid;
        } else {
            expected.remove(&name);
        }
    }
    Ok(expected)
}

#[cfg(windows)]
pub(crate) async fn execute(
    context: super::ActionContext,
    database: std::sync::Arc<crate::persistence::workspace::Database>,
    auth: std::sync::Arc<crate::auth::AuthBroker>,
    window: String,
) -> Result<crate::operations::scheduler::Completion, AppError> {
    use crate::git::runner::{argv, RunOptions};
    let prune = match context.intent {
        ActionIntent::Fetch { prune, .. } => prune,
        _ => return Err(error(ErrorCode::UnsupportedAction, Phase::Validation)),
    };
    let reader = context.reader()?;
    let format = reader
        .run(&argv(&["rev-parse", "--show-object-format"]))
        .await?;
    let oid_length = match format.as_slice() {
        b"sha1\n" => 40,
        b"sha256\n" => 64,
        _ => return Err(malformed()),
    };
    let configuration = super::catalog::configuration(&reader).await?;
    let mut remotes = Vec::new();
    for target in &context.targets {
        let Some(super::journal::SafeTargetProof::Remote {
            name,
            config_digest,
        }) = &target.proof
        else {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        };
        if target.kind != super::authority::TargetKind::Remote || target.identity != name.as_bytes()
        {
            return Err(error(ErrorCode::InvalidHandle, Phase::Validation));
        }
        let index = configuration
            .remotes
            .iter()
            .position(|r| {
                r.name == *name
                    && r.digest == *config_digest
                    && r.capability == FetchCapability::EligibleProbeRequired
            })
            .ok_or_else(|| error(ErrorCode::ExternalChange, Phase::Validation))?;
        remotes.push(index);
    }
    if remotes.is_empty() || remotes.len() > REMOTES_MAX {
        return Err(error(ErrorCode::InvalidRequest, Phase::Validation));
    }
    let stable = super::preflight::capture_stable(
        reader.runner,
        reader.location,
        reader.lifetime.clone(),
        &context.cancel,
    )
    .await?;
    let mut expected = read_refs(&reader, oid_length).await?;
    let digest = refs_digest(&expected)?;
    let proofs = remotes
        .iter()
        .map(|i| super::observations::RemoteProof {
            name: configuration.remotes[*i].name.clone(),
            config_digest: configuration.remotes[*i].digest.clone(),
            refs_digest: digest.clone(),
        })
        .collect();
    context.check_owner()?;
    super::observations::begin(
        &database,
        &context.operation_id,
        &context_scope_repo(&database, &context.operation_id)?,
        prune,
        proofs,
    )?;
    drop(reader);
    if !context.before_write().await? {
        return Ok(super::completed(ActionEffects::NotApplied, None));
    }
    let mut coverage = CoverageBudget::default();
    for (step, remote_index) in remotes.into_iter().enumerate() {
        if context.cancel.is_cancelled() {
            break;
        }
        let remote = &configuration.remotes[remote_index];
        let reader = context.reader()?;
        let now = super::preflight::capture_stable(
            reader.runner,
            reader.location,
            reader.lifetime.clone(),
            &context.cancel,
        )
        .await?;
        if now.fingerprint != stable.fingerprint
            || read_refs(&reader, oid_length).await? != expected
            || super::catalog::configuration(&reader).await?.digest != configuration.digest
        {
            return Err(error(ErrorCode::ExternalChange, Phase::Verification));
        }
        context.check_owner()?;
        drop(reader);
        let alias = format!("opengit-captured-{}:", crate::history::sessions::token());
        let mut environment =
            alias_environment(&alias, remote.url.as_deref().ok_or_else(malformed)?)?;
        let session = if remote
            .url
            .as_ref()
            .is_some_and(|v| crate::clone::request::remote_transport(v).is_ok())
        {
            Some(auth.session(&window, &context.operation_id, context.cancel.clone())?)
        } else {
            None
        };
        let mut options = if let Some(session) = &session {
            session.options(
                &std::env::current_exe()
                    .map_err(|_| error(ErrorCode::Internal, Phase::Authentication))?,
            )?
        } else {
            RunOptions::default()
        };
        options.environment.append(&mut environment);
        let ssh_available = session.as_ref().is_none_or(|s| s.ssh_prompt_available());
        let mut args = argv(&[
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
            if prune { "--prune" } else { "--no-prune" },
            "--no-prune-tags",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-auto-maintenance",
            "--upload-pack=git-upload-pack",
            "--",
            &alias,
        ]);
        args.extend(remote.refspecs.iter().map(std::ffi::OsString::from));
        if !context.before_fetch_step(step, &refs_digest(&expected)?)? {
            break;
        }
        let output = context.output(&args, Phase::Transfer, options).await;
        if let Some(session) = session {
            session.close();
        }
        let verified = async {
            let output = output?;
            if !output.success {
                return Err(crate::git::error::git_failure(
                    &output.stderr,
                    Phase::Transfer,
                ));
            }
            let reported = parse_porcelain(&output.stdout, &remote.name, oid_length)?;
            coverage.charge(reported.len())?;
            let candidate = apply_reported(&expected, &reported, oid_length)?;
            let reader = context.reader()?;
            let after = super::preflight::capture_stable(
                reader.runner,
                reader.location,
                reader.lifetime.clone(),
                &crate::git::runner::Cancellation::default(),
            )
            .await?;
            // Verification uses a fresh cancellation token after an acknowledged
            // successful fetch: a late user cancel cannot erase applied effects.
            let verification = super::catalog::ReadQuery::new(
                reader.runner,
                reader.location,
                reader.lifetime.clone(),
                Default::default(),
            );
            if after.fingerprint != stable.fingerprint
                || read_refs(&verification, oid_length).await? != candidate
                || super::catalog::configuration(&verification).await?.digest
                    != configuration.digest
            {
                return Err(error(ErrorCode::ExternalChange, Phase::Verification));
            }
            expected = candidate;
            Ok(reported
                .into_iter()
                .map(|r| FetchRefChange {
                    name: r.name,
                    old_oid: (r.old_oid != "0".repeat(oid_length)).then_some(r.old_oid),
                    new_oid: (r.new_oid != "0".repeat(oid_length)).then_some(r.new_oid),
                    status: match r.status {
                        '*' => FetchRefStatus::Created,
                        '+' => FetchRefStatus::Forced,
                        '-' => FetchRefStatus::Deleted,
                        '=' => FetchRefStatus::UpToDate,
                        _ => FetchRefStatus::Updated,
                    },
                })
                .collect::<Vec<_>>())
        }
        .await;
        let error = verified.as_ref().err().map(|e| {
            if remote.ssh && !ssh_available {
                crate::git::error::ssh_prompt_unavailable(e.clone())
            } else {
                e.clone()
            }
        });
        let succeeded = verified.is_ok();
        super::observations::observe(
            &database,
            &context.operation_id,
            step,
            FetchRemoteResult {
                name: remote.name.clone(),
                phase: FetchStepPhase::Observed,
                effects: if succeeded {
                    ActionEffects::Applied
                } else {
                    ActionEffects::Unknown
                },
                error,
                reported_refs: verified.unwrap_or_default(),
                observed_at: Some(chrono::Utc::now().to_rfc3339()),
            },
        )?;
        if !succeeded {
            break;
        }
    }
    let connection = database.lock()?;
    let record = super::observations::read_from(&connection, &context.operation_id)?
        .ok_or_else(crate::persistence::workspace::db_error)?;
    let effects = super::observations::effects(&record);
    let postcondition = if matches!(effects, ActionEffects::Applied | ActionEffects::NotApplied) {
        Some(refs_digest(&expected)?)
    } else {
        None
    };
    Ok(super::completed(effects, postcondition))
}
#[cfg(windows)]
fn context_scope_repo(
    database: &crate::persistence::workspace::Database,
    id: &str,
) -> Result<String, AppError> {
    Ok(super::journal::read(database, id)?.repo_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_action_coverage_bound_is_checked_and_does_not_publish_a_prefix() {
        let mut budget = CoverageBudget::default();
        budget.charge(REFS_MAX).unwrap();
        assert_eq!(budget.charge(1).unwrap_err().code, ErrorCode::OutputLimit);
        assert_eq!(budget.0, REFS_MAX);
        assert_eq!(
            CoverageBudget(1).charge(usize::MAX).unwrap_err().code,
            ErrorCode::OutputLimit
        );
        let oid = "a".repeat(40);
        let bytes = (0..=REFS_MAX)
            .map(|i| format!("= {oid} {oid} refs/remotes/origin/r{i}\n"))
            .collect::<String>();
        assert_eq!(
            parse_porcelain(bytes.as_bytes(), "origin", 40)
                .unwrap_err()
                .code,
            ErrorCode::OutputLimit
        );
    }
    #[test]
    fn tracking_only_refspecs_cannot_write_local_heads_tags_or_another_remote() {
        for value in [
            "+refs/heads/*:refs/remotes/origin/*",
            "refs/heads/main:refs/remotes/origin/narrow",
            "^refs/heads/private/*",
        ] {
            assert!(safe_refspec("origin", value), "{value}");
        }
        for value in [
            "+refs/*:refs/*",
            "refs/heads/main:refs/heads/main",
            "refs/tags/*:refs/tags/*",
            "refs/heads/*:refs/remotes/other/*",
            "refs/heads/main",
            "--upload-pack=evil",
            "refs/heads/*:refs/remotes/origin/main",
        ] {
            assert!(!safe_refspec("origin", value), "{value}");
        }
    }
    #[test]
    fn porcelain_counts_actual_coverage_and_rejects_ambiguous_records() {
        for n in [40, 64] {
            let oid = "a".repeat(n);
            let initial = format!(
                "* {} {oid} refs/remotes/origin/main\n= {oid} {oid} refs/remotes/origin/topic\n",
                "0".repeat(n)
            );
            let parsed = parse_porcelain(initial.as_bytes(), "origin", n).unwrap();
            assert_eq!(parsed.len(), 2);
            assert_eq!(parsed[1].status, '=');
            for bad in [
                format!("= {oid} {oid} refs/heads/main\n"),
                format!("= {oid} {} refs/remotes/origin/main\n", "b".repeat(n)),
                format!("{initial}{initial}"),
                format!("= {oid} {oid} refs/remotes/origin/main"),
            ] {
                assert_eq!(
                    parse_porcelain(bad.as_bytes(), "origin", n)
                        .unwrap_err()
                        .code,
                    ErrorCode::MalformedOutput
                );
            }
        }
    }
}
