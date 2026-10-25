use super::{
    query::{summary, Query},
    sessions::{token, Session},
    *,
};
use crate::git::runner::argv;
use std::collections::HashSet;

#[derive(Clone, PartialEq)]
pub(crate) struct FileRecord {
    pub commit: String,
    pub base: DiffBase,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_path: Vec<u8>,
    pub new_path: Vec<u8>,
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn field<'a>(bytes: &'a [u8], offset: &mut usize) -> Result<&'a [u8], AppError> {
    let tail = bytes.get(*offset..).ok_or_else(malformed)?;
    let length = tail.iter().position(|b| *b == 0).ok_or_else(malformed)?;
    *offset += length + 1;
    Ok(&tail[..length])
}
fn mode(value: &str) -> Result<Option<String>, AppError> {
    match value {
        "000000" => Ok(None),
        "100644" | "100755" | "120000" | "160000" => Ok(Some(value.into())),
        _ => Err(malformed()),
    }
}
pub(crate) async fn read(
    query: &Query<'_>,
    session: &mut Session,
    oid: &str,
    base: DiffBase,
) -> Result<(CommitDetails, Vec<(String, FileRecord)>), AppError> {
    let mut commits = query.objects(&[oid.to_owned()]).await?;
    let raw = commits.pop().ok_or_else(malformed)?;
    let mut args = argv(&[
        "diff-tree",
        "-r",
        "--raw",
        "-z",
        "--no-commit-id",
        "--no-abbrev",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--ignore-submodules=none",
        "--submodule=short",
        "--no-relative",
        "-M",
        "-C",
        "--find-copies-harder",
        "-l1000",
    ]);
    match base {
        DiffBase::Root if raw.parents.is_empty() => args.push("--root".into()),
        DiffBase::Parent { index } => {
            args.push(raw.parents.get(index as usize).ok_or_else(invalid)?.into())
        }
        _ => return Err(invalid()),
    }
    args.push(oid.into());
    args.push("--".into());
    let output = query.output(&args, None).await?;
    if !output.success {
        return Err(error(ErrorCode::NotFound, Phase::Inspection));
    }
    // Git warns when the bounded rename/copy candidate limit prevents a full
    // classification. Reject the list instead of silently reporting A/D.
    if !output.stderr.is_empty() {
        return Err(AppError {
            code: ErrorCode::OutputLimit,
            phase: Phase::Inspection,
            retry: AppErrorRetry::None,
            message:
                "Git returned a file-list warning. The complete comparison could not be verified."
                    .into(),
        });
    }
    let mut offset = 0;
    let mut files = Vec::new();
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    while offset < output.stdout.len() {
        if files.len() >= HISTORY_DETAIL_FILES_MAX {
            return Err(limit());
        }
        let header =
            std::str::from_utf8(field(&output.stdout, &mut offset)?).map_err(|_| malformed())?;
        let fields: Vec<_> = header
            .strip_prefix(':')
            .ok_or_else(malformed)?
            .split(' ')
            .collect();
        if fields.len() != 5 {
            return Err(malformed());
        }
        let old_mode = mode(fields[0])?;
        let new_mode = mode(fields[1])?;
        let object = |value: &str, present: bool| -> Result<Option<String>, AppError> {
            if !query::oid(value, session.fingerprint.oid_length) {
                return Err(malformed());
            }
            if !present {
                if value.bytes().all(|b| b == b'0') {
                    Ok(None)
                } else {
                    Err(malformed())
                }
            } else if value.bytes().all(|b| b == b'0') {
                Err(malformed())
            } else {
                Ok(Some(value.into()))
            }
        };
        let old_oid = object(fields[2], old_mode.is_some())?;
        let new_oid = object(fields[3], new_mode.is_some())?;
        let status = fields[4];
        let kind = match status {
            "A" => CommitFileKind::Added,
            "D" => CommitFileKind::Deleted,
            "M" => CommitFileKind::Modified,
            "T" => CommitFileKind::TypeChanged,
            _ if status.len() >= 2
                && matches!(status.as_bytes()[0], b'R' | b'C')
                && status[1..].bytes().all(|b| b.is_ascii_digit())
                && status[1..].parse::<u32>().is_ok_and(|n| n <= 100) =>
            {
                if status.starts_with('R') {
                    CommitFileKind::Renamed
                } else {
                    CommitFileKind::Copied
                }
            }
            _ => return Err(malformed()),
        };
        let path = field(&output.stdout, &mut offset)?.to_vec();
        let (old_path, new_path, previous_path_display) =
            if matches!(kind, CommitFileKind::Renamed | CommitFileKind::Copied) {
                let next = field(&output.stdout, &mut offset)?.to_vec();
                let previous = Some(String::from_utf8_lossy(&path).into_owned());
                (path, next, previous)
            } else {
                (path.clone(), path, None)
            };
        if new_path.is_empty() || old_path.is_empty() || !seen.insert(new_path.clone()) {
            return Err(malformed());
        }
        let record = FileRecord {
            commit: oid.into(),
            base: base.clone(),
            old_oid,
            new_oid,
            old_mode: old_mode.clone(),
            new_mode: new_mode.clone(),
            old_path,
            new_path,
        };
        let handle = match session.files.iter().find(|(_, value)| **value == record) {
            Some((handle, _)) => handle.clone(),
            None => {
                if session.files.len() + pending.len() >= HISTORY_FILES_PER_SESSION {
                    return Err(limit());
                }
                let handle = token();
                pending.push((handle.clone(), record.clone()));
                handle
            }
        };
        files.push(CommitFile {
            handle,
            path_display: String::from_utf8_lossy(&record.new_path).into_owned(),
            previous_path_display,
            kind,
            old_mode,
            new_mode,
        });
    }
    let message = raw.message.clone();
    let committer_name = raw.committer_name.clone();
    let committer_email = raw.committer_email.clone();
    Ok((
        CommitDetails {
            context: session.context.clone(),
            commit: summary(session, raw, session.graph_fence)?,
            message,
            committer_name,
            committer_email,
            base,
            files,
        },
        pending,
    ))
}
