use super::{details::FileRecord, query::Query, *};
use crate::git::runner::argv;

fn unavailable(reason: DiffUnavailableReason) -> DiffContent {
    DiffContent::Unavailable { reason }
}
fn malformed() -> AppError {
    error(ErrorCode::MalformedOutput, Phase::Inspection)
}
pub(crate) async fn read(query: &Query<'_>, record: &FileRecord) -> Result<DiffContent, AppError> {
    if record.old_mode.as_deref() == Some("160000") || record.new_mode.as_deref() == Some("160000")
    {
        return Ok(unavailable(DiffUnavailableReason::Gitlink));
    }
    let oids: Vec<_> = [&record.old_oid, &record.new_oid]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    let input = oids
        .iter()
        .map(|oid| format!("{oid}\n"))
        .collect::<String>();
    let sizes = query
        .run(
            &argv(&["cat-file", "--batch-check"]),
            Some(input.as_bytes()),
        )
        .await?;
    let lines: Vec<_> = sizes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() != oids.len() {
        return Err(malformed());
    }
    let mut lengths = Vec::new();
    let mut total = 0usize;
    for (line, oid) in lines.iter().zip(&oids) {
        let text = std::str::from_utf8(line).map_err(|_| malformed())?;
        if text == format!("{oid} missing") {
            return Ok(unavailable(DiffUnavailableReason::MissingObject));
        }
        let fields: Vec<_> = text.split(' ').collect();
        if fields.len() != 3
            || fields[0] != oid
            || fields[1] != "blob"
            || !fields[2].bytes().all(|b| b.is_ascii_digit())
        {
            return Err(malformed());
        }
        let length = fields[2].parse::<usize>().map_err(|_| limit())?;
        total = total.checked_add(length).ok_or_else(limit)?;
        if length > HISTORY_DIFF_BLOB_BYTES || total > HISTORY_DIFF_TOTAL_BYTES {
            return Ok(unavailable(DiffUnavailableReason::TooLarge));
        }
        lengths.push(length);
    }
    let output = query
        .run(&argv(&["cat-file", "--batch"]), Some(input.as_bytes()))
        .await?;
    let mut offset = 0usize;
    let mut contents = Vec::new();
    for (oid, length) in oids.iter().zip(lengths) {
        let tail = output.get(offset..).ok_or_else(malformed)?;
        let header_end = tail
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(malformed)?;
        let header = &tail[..header_end];
        if header == format!("{oid} missing").as_bytes() {
            return Ok(unavailable(DiffUnavailableReason::MissingObject));
        }
        if header != format!("{oid} blob {length}").as_bytes() {
            return Err(malformed());
        }
        let start = offset + header_end + 1;
        let end = start.checked_add(length).ok_or_else(limit)?;
        let bytes = output.get(start..end).ok_or_else(malformed)?;
        if output.get(end) != Some(&b'\n') {
            return Err(malformed());
        }
        offset = end + 1;
        if bytes.contains(&0) {
            return Ok(unavailable(DiffUnavailableReason::Binary));
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Ok(unavailable(DiffUnavailableReason::UnsupportedEncoding));
        };
        contents.push(text.to_owned());
    }
    if offset != output.len() {
        return Err(malformed());
    }
    let mut contents = contents.into_iter();
    let original = if record.old_oid.is_some() {
        contents.next().ok_or_else(malformed)?
    } else {
        String::new()
    };
    let modified = if record.new_oid.is_some() {
        contents.next().ok_or_else(malformed)?
    } else {
        String::new()
    };
    let extension = record
        .new_path
        .rsplit(|b| *b == b'.')
        .next()
        .and_then(|b| std::str::from_utf8(b).ok());
    let language = match extension {
        Some("rs") => Some("rust"),
        Some("ts" | "tsx") => Some("typescript"),
        Some("js" | "jsx") => Some("javascript"),
        Some("json") => Some("json"),
        Some("md") => Some("markdown"),
        Some("py") => Some("python"),
        Some("html") => Some("html"),
        Some("css") => Some("css"),
        _ => None,
    }
    .map(str::to_owned);
    Ok(DiffContent::Text {
        original,
        modified,
        language,
    })
}
