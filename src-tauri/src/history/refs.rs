use super::{query::Fingerprint, *};
use std::collections::BTreeSet;

fn unavailable_stash() -> AppError {
    AppError {
        code: ErrorCode::NotFound,
        phase: Phase::Inspection,
        retry: AppErrorRetry::ReopenRepository,
        message: "A stash entry is unavailable. Check the repository history before reopening it."
            .into(),
    }
}
pub(crate) async fn append_stash_stack(
    query: &query::Query<'_>,
    references: &mut Vec<HistoryRef>,
    oid_length: usize,
) -> Result<Vec<u8>, AppError> {
    let Some(current) = references
        .iter()
        .position(|r| r.kind == HistoryRefKind::Stash && r.name == "refs/stash")
    else {
        return Ok(Vec::new());
    };
    let exists = query
        .output(
            &crate::git::runner::argv(&["reflog", "exists", "refs/stash"]),
            None,
        )
        .await?;
    if !exists.stdout.is_empty() || !exists.stderr.is_empty() {
        return Err(unavailable_stash());
    }
    if exists.exit_code == Some(1) {
        references[current].display_name = "stash (current)".into();
        return Ok(Vec::new());
    }
    if !exists.success {
        return Err(unavailable_stash());
    }
    let output = query
        .output(
            &crate::git::runner::argv(&[
                "reflog",
                "show",
                "--no-patch",
                "--no-notes",
                "--no-show-signature",
                "--no-use-mailmap",
                "--no-color",
                "--encoding=UTF-8",
                "--format=%H%x00%gD%x00%gs%x00",
                &format!("--max-count={}", HISTORY_REFS_MAX + 1),
                "refs/stash",
                "--",
            ]),
            None,
        )
        .await?;
    if !output.success || !output.stderr.is_empty() {
        return Err(unavailable_stash());
    }
    let mut offset = 0usize;
    let mut entries = Vec::new();
    while offset < output.stdout.len() {
        let mut fields = Vec::new();
        for _ in 0..3 {
            let length = output.stdout[offset..]
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(unavailable_stash)?;
            fields.push(
                std::str::from_utf8(&output.stdout[offset..offset + length])
                    .map_err(|_| unavailable_stash())?,
            );
            offset += length + 1;
        }
        if output.stdout.get(offset) != Some(&b'\n') {
            return Err(unavailable_stash());
        }
        offset += 1;
        if !query::oid(fields[0], oid_length)
            || fields[1] != format!("refs/stash@{{{}}}", entries.len())
        {
            return Err(unavailable_stash());
        }
        if entries.len() >= HISTORY_REFS_MAX {
            return Err(limit());
        }
        entries.push((fields[0].to_owned(), fields[2].to_owned()));
    }
    // Git reflog show silently skips entries whose commit object is missing.
    // Contiguous selectors catch holes; this backend-generated bounded tail
    // lookup also catches omitted final entries. It never uses a UI expression.
    let tail = query
        .output(
            &crate::git::runner::argv(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/stash@{{{}}}", entries.len()),
            ]),
            None,
        )
        .await?;
    if tail.exit_code != Some(128) || !tail.stdout.is_empty() || !tail.stderr.is_empty() {
        return Err(unavailable_stash());
    }
    if let Some((oid, subject)) = entries.first() {
        if references[current].commit_oid.as_ref() != Some(oid) {
            return Err(external());
        }
        references[current].display_name = format!("stash@{{0}}: {subject}");
    } else {
        references[current].display_name = "stash (current)".into();
    }
    for (index, (oid, subject)) in entries.into_iter().enumerate().skip(1) {
        if references.len() >= HISTORY_REFS_MAX {
            return Err(limit());
        }
        references.push(HistoryRef {
            name: format!("refs/stash/{index}"),
            display_name: format!("stash@{{{index}}}: {subject}"),
            kind: HistoryRefKind::Stash,
            oid: oid.clone(),
            commit_oid: Some(oid),
            symbolic_target: None,
            hidden: false,
        });
    }
    Ok(output.stdout)
}

pub(crate) fn valid_name(name: &str) -> bool {
    name.starts_with("refs/")
        && name.len() <= 1024
        && !name.ends_with(['/', '.'])
        && !name.contains(['\\', ' ', '~', '^', ':', '?', '*', '['])
        && !name.bytes().any(|byte| byte < 32 || byte == 127)
        && !name.contains("..")
        && !name.contains("@{")
        && name
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
}
pub(crate) fn validate_filter(filter: &HistoryFilter) -> Result<(), AppError> {
    if filter
        .hidden_refs
        .iter()
        .chain(&filter.solo_refs)
        .any(|name| !valid_name(name))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
pub(crate) fn roots(
    fingerprint: &Fingerprint,
    filter: &HistoryFilter,
) -> (Vec<HistoryRef>, Vec<String>, Vec<String>) {
    let hidden: BTreeSet<_> = filter.hidden_refs.iter().collect();
    let solo: BTreeSet<_> = filter
        .solo_refs
        .iter()
        .filter(|name| fingerprint.refs.iter().any(|r| r.name == name.as_str()))
        .collect();
    let mut selected = BTreeSet::new();
    let mut all = BTreeSet::new();
    let refs = fingerprint
        .refs
        .iter()
        .cloned()
        .map(|mut reference| {
            reference.hidden = hidden.contains(&reference.name);
            if let Some(oid) = &reference.commit_oid {
                all.insert(oid.clone());
                if !reference.hidden
                    && (if solo.is_empty() {
                        reference.kind != HistoryRefKind::Stash
                    } else {
                        solo.contains(&reference.name)
                    })
                {
                    selected.insert(oid.clone());
                }
            }
            reference
        })
        .collect();
    if let Some(oid) = &fingerprint.head.oid {
        all.insert(oid.clone());
        if fingerprint.head.kind == HeadKind::Detached && solo.is_empty() {
            selected.insert(oid.clone());
        }
    }
    (
        refs,
        selected.into_iter().collect(),
        all.into_iter().collect(),
    )
}
