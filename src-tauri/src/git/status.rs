use super::error::error;
use crate::domain::contracts::*;
pub struct ParsedStatus {
    pub head: Head,
    pub entries: Vec<StatusEntry>,
    pub raw_paths: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
fn path_status(byte: u8) -> Result<PathStatus, AppError> {
    Ok(match byte {
        b'.' => PathStatus::Value0,
        b'M' => PathStatus::M,
        b'T' => PathStatus::T,
        b'A' => PathStatus::A,
        b'D' => PathStatus::D,
        b'R' => PathStatus::R,
        b'C' => PathStatus::C,
        b'U' => PathStatus::U,
        b'?' => PathStatus::Value8,
        b'!' => PathStatus::Value9,
        _ => return Err(malformed()),
    })
}
pub fn parse(bytes: &[u8]) -> Result<ParsedStatus, AppError> {
    if !bytes.is_empty() && bytes.last() != Some(&0) {
        return Err(malformed());
    }
    let mut parsed = ParsedStatus {
        head: Head {
            kind: HeadKind::Unborn,
            branch: None,
            oid: None,
        },
        entries: Vec::new(),
        raw_paths: Vec::new(),
    };
    let mut records = bytes.split(|b| *b == 0).peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            if let Some(oid) = record.strip_prefix(b"# branch.oid ") {
                if oid != b"(initial)" {
                    if !oid.iter().all(u8::is_ascii_hexdigit) || oid.is_empty() {
                        return Err(malformed());
                    }
                    parsed.head.oid = Some(String::from_utf8_lossy(oid).into());
                }
            }
            if let Some(branch) = record.strip_prefix(b"# branch.head ") {
                if branch == b"(detached)" {
                    parsed.head.kind = HeadKind::Detached;
                } else {
                    parsed.head.branch = Some(String::from_utf8_lossy(branch).into());
                }
            }
            continue;
        }
        let kind = *record.first().ok_or_else(malformed)?;
        let (fields_count, entry_kind) = match kind {
            b'1' => (9, StatusEntryKind::Ordinary),
            b'2' => (10, StatusEntryKind::Rename),
            b'u' => (11, StatusEntryKind::Unmerged),
            b'?' => (2, StatusEntryKind::Untracked),
            b'!' => (2, StatusEntryKind::Ignored),
            _ => return Err(malformed()),
        };
        let fields: Vec<&[u8]> = record.splitn(fields_count, |b| *b == b' ').collect();
        if fields.len() != fields_count || fields.last().is_none_or(|v| v.is_empty()) {
            return Err(malformed());
        }
        let (index_status, worktree_status, submodule) = if fields_count == 2 {
            (path_status(kind)?, path_status(kind)?, false)
        } else {
            if fields[1].len() != 2 || fields[2].len() != 4 || !matches!(fields[2][0], b'N' | b'S')
            {
                return Err(malformed());
            }
            let modes = if kind == b'u' { 3..7 } else { 3..6 };
            let oids = if kind == b'u' { 7..10 } else { 6..8 };
            if modes.into_iter().any(|i| {
                fields[i].len() != 6 || !fields[i].iter().all(|b| (b'0'..=b'7').contains(b))
            }) || oids
                .into_iter()
                .any(|i| fields[i].is_empty() || !fields[i].iter().all(u8::is_ascii_hexdigit))
                || (kind == b'2'
                    && (fields[8].len() < 2
                        || !matches!(fields[8][0], b'R' | b'C')
                        || !fields[8][1..].iter().all(u8::is_ascii_digit)))
            {
                return Err(malformed());
            }
            (
                path_status(fields[1][0])?,
                path_status(fields[1][1])?,
                fields[2][0] == b'S',
            )
        };
        let path = fields[fields_count - 1].to_vec();
        let previous = if kind == b'2' {
            let value = records
                .next()
                .filter(|p| !p.is_empty())
                .ok_or_else(malformed)?;
            Some(value.to_vec())
        } else {
            None
        };
        parsed.entries.push(StatusEntry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: entry_kind,
            path_display: String::from_utf8_lossy(&path).into(),
            previous_path_display: previous.as_ref().map(|p| String::from_utf8_lossy(p).into()),
            index_status,
            worktree_status,
            submodule,
        });
        parsed.raw_paths.push((path, previous));
    }
    if parsed.head.kind != HeadKind::Detached {
        parsed.head.kind = if parsed.head.oid.is_some() {
            HeadKind::Branch
        } else {
            HeadKind::Unborn
        };
    }
    if parsed.head.branch.is_none() && parsed.head.kind != HeadKind::Detached {
        return Err(malformed());
    }
    Ok(parsed)
}
