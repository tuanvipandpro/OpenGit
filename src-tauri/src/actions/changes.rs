//! Raw paths remain backend authority; display names and entry IDs are descriptive.
use super::{
    authority::{Target, TargetKind},
    catalog::ReadQuery,
    journal::SafeTargetProof,
    preflight,
};
use crate::{
    domain::contracts::*,
    git::{error::error, runner::argv},
    repositories::registry::RepositoryLocation,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, PathBuf},
};
pub(crate) fn invalid() -> AppError {
    error(ErrorCode::InvalidHandle, Phase::Validation)
}
pub(crate) fn changed() -> AppError {
    error(ErrorCode::ExternalChange, Phase::Validation)
}
pub(crate) fn unsupported() -> AppError {
    error(ErrorCode::UnsupportedAction, Phase::Validation)
}
fn advice(args: &[std::ffi::OsString]) -> Vec<std::ffi::OsString> {
    let mut v = argv(&["-c", "advice.graftFileDeprecated=false"]);
    v.extend_from_slice(args);
    v
}
pub(crate) async fn run(
    q: &ReadQuery<'_>,
    args: &[std::ffi::OsString],
) -> Result<Vec<u8>, AppError> {
    q.run(&advice(args)).await
}
pub(crate) async fn output(
    q: &ReadQuery<'_>,
    args: &[std::ffi::OsString],
) -> Result<crate::git::runner::GitOutput, AppError> {
    q.output(&advice(args)).await
}
pub(crate) fn supported(k: ActionKind) -> bool {
    matches!(
        k,
        ActionKind::Stage
            | ActionKind::Unstage
            | ActionKind::StageSelection
            | ActionKind::UnstageSelection
            | ActionKind::Commit
    )
}
pub(crate) fn path(location: &RepositoryLocation, raw: &[u8]) -> Result<PathBuf, AppError> {
    let text = std::str::from_utf8(raw).map_err(|_| unsupported())?;
    if text.is_empty()
        || text.len() > 768
        || text.contains(['\\', ':', '\0'])
        || text.chars().any(char::is_control)
        || text.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.eq_ignore_ascii_case(".git")
                || p.ends_with(['.', ' '])
        })
    {
        return Err(unsupported());
    }
    let relative = PathBuf::from(text);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(unsupported());
    }
    let full = location.root.join(relative);
    let mut ancestor = full.parent();
    while let Some(p) = ancestor {
        if p == location.root {
            break;
        }
        safe_metadata(p)?;
        ancestor = p.parent();
    }
    Ok(full)
}
fn safe_metadata(path: &std::path::Path) -> Result<fs::Metadata, AppError> {
    let m = fs::symlink_metadata(path).map_err(|_| changed())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(unsupported());
        }
    }
    if m.file_type().is_symlink() {
        return Err(unsupported());
    }
    Ok(m)
}
pub(crate) struct CheckedFile {
    pub file: fs::File,
    #[cfg(windows)]
    _parent: crate::clone::path::StableDirectory,
}
pub(crate) fn open_regular(path: &std::path::Path) -> Result<CheckedFile, AppError> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::*;
        let parent =
            crate::clone::path::StableDirectory::open(path.parent().ok_or_else(unsupported)?)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| changed())?;
        let m = file.metadata().map_err(|_| changed())?;
        if !m.is_file() || m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(unsupported());
        }
        if crate::clone::path::info_identity(&crate::clone::path::handle_info(&file)?)
            != crate::repositories::discovery::identity(path)?
        {
            return Err(changed());
        }
        Ok(CheckedFile {
            file,
            _parent: parent,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(unsupported())
    }
}
pub(crate) fn file_stamp(location: &RepositoryLocation, raw: &[u8]) -> Result<String, AppError> {
    let p = path(location, raw)?;
    let mut h = Sha256::new();
    h.update(raw);
    match fs::symlink_metadata(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => h.update([0]),
        Err(_) => return Err(changed()),
        Ok(_) => {
            let m = safe_metadata(&p)?;
            if !m.is_file() || m.len() > 32 * 1024 * 1024 {
                return Err(unsupported());
            }
            h.update(crate::repositories::discovery::identity(&p)?);
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                h.update(m.file_attributes().to_le_bytes());
            }
            h.update(m.len().to_le_bytes());
            let mut opened = open_regular(&p)?;
            let f = &mut opened.file;
            let mut b = [0; 65536];
            let mut n = 0u64;
            loop {
                let count = f.read(&mut b).map_err(|_| changed())?;
                if count == 0 {
                    break;
                }
                n += count as u64;
                if n > 32 * 1024 * 1024 {
                    return Err(unsupported());
                }
                h.update(&b[..count]);
            }
            if n != m.len() {
                return Err(changed());
            }
        }
    }
    Ok(format!("{:x}", h.finalize()))
}
pub(crate) async fn index_supported(q: &ReadQuery<'_>) -> Result<(), AppError> {
    if q.location.bare {
        return Err(unsupported());
    }
    let shared = run(q, &argv(&["rev-parse", "--shared-index-path"])).await?;
    if !shared.iter().all(u8::is_ascii_whitespace) {
        return Err(unsupported());
    }
    let entries = run(q, &argv(&["ls-files", "--stage", "--sparse", "-z"])).await?;
    if entries
        .split(|b| *b == 0)
        .any(|e| e.starts_with(b"160000 ") || e.starts_with(b"040000 "))
    {
        return Err(unsupported());
    }
    let flags = run(q, &argv(&["ls-files", "-t", "-z"])).await?;
    if flags.split(|b| *b == 0).any(|e| e.starts_with(b"S ")) {
        return Err(unsupported());
    }
    Ok(())
}
pub(crate) struct Capture {
    pub status: crate::git::status::ParsedStatus,
    pub wire: Vec<ChangeEntry>,
    pub stamps: Vec<Option<String>>,
    pub digest: String,
    pub unsupported: bool,
}
pub(crate) async fn capture(q: &ReadQuery<'_>) -> Result<Capture, AppError> {
    if q.location.bare {
        return Ok(Capture {
            status: crate::git::status::ParsedStatus {
                head: Head {
                    kind: HeadKind::Unborn,
                    branch: None,
                    oid: None,
                },
                entries: vec![],
                raw_paths: vec![],
            },
            wire: vec![],
            stamps: vec![],
            digest: preflight::digest(b"bare"),
            unsupported: true,
        });
    }
    let status = crate::git::status::parse(
        &run(
            q,
            &argv(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ]),
        )
        .await?,
    )?;
    if status.entries.len() > 4096 {
        return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
    }
    let unsupported = match index_supported(q).await {
        Ok(()) => false,
        Err(e) if e.code == ErrorCode::UnsupportedAction => true,
        Err(e) => return Err(e),
    };
    let mut wire = Vec::new();
    let mut stamps = Vec::new();
    let mut digest = Sha256::new();
    let mut total = 0u64;
    for (e, (raw, previous)) in status.entries.iter().zip(&status.raw_paths) {
        total +=
            fs::symlink_metadata(path(q.location, raw).unwrap_or_default()).map_or(0, |m| m.len());
        if total > 128 * 1024 * 1024 {
            return Err(error(ErrorCode::OutputLimit, Phase::Inspection));
        }
        let stamp = if unsupported || e.submodule {
            None
        } else {
            selected_stamp(q.location, raw, previous.as_deref()).ok()
        };
        digest.update(raw);
        if let Some(p) = previous {
            digest.update(p);
        }
        digest.update(stamp.as_deref().unwrap_or("unsupported").as_bytes());
        let cap = |present: bool| {
            if !present {
                ChangeCapability::Unavailable
            } else if stamp.is_some() {
                ChangeCapability::Eligible
            } else {
                ChangeCapability::Unsupported
            }
        };
        let mut locator = raw.clone();
        locator.push(0);
        if let Some(previous) = previous {
            locator.extend(previous);
        }
        wire.push(ChangeEntry {
            // Stable descriptive locator, never authority without the current
            // random catalog capability and exact content/index fingerprint.
            entry_id: format!("change-{}", preflight::digest(&locator)),
            path_display: e.path_display.clone(),
            previous_path_display: e.previous_path_display.clone(),
            index_status: e.index_status.clone(),
            worktree_status: e.worktree_status.clone(),
            stage_capability: cap(e.worktree_status != PathStatus::Value0),
            unstage_capability: if e.kind == StatusEntryKind::Unmerged {
                ChangeCapability::Unsupported
            } else {
                cap(e.index_status != PathStatus::Value0 && e.kind != StatusEntryKind::Untracked)
            },
        });
        stamps.push(stamp);
    }
    Ok(Capture {
        status,
        wire,
        stamps,
        digest: format!("{:x}", digest.finalize()),
        unsupported,
    })
}
fn selected_stamp(
    l: &RepositoryLocation,
    p: &[u8],
    old: Option<&[u8]>,
) -> Result<String, AppError> {
    let mut s = file_stamp(l, p)?;
    if let Some(old) = old {
        s.push_str(&file_stamp(l, old)?);
    }
    Ok(preflight::digest(s.as_bytes()))
}
pub(crate) fn target(c: &Capture, s: &ChangeSelection) -> Result<Target, AppError> {
    let i = c
        .wire
        .iter()
        .position(|e| e.entry_id == s.entry_id)
        .ok_or_else(invalid)?;
    let e = &c.wire[i];
    if (if s.side == ChangeSide::Staged {
        e.unstage_capability
    } else {
        e.stage_capability
    }) != ChangeCapability::Eligible
    {
        return Err(unsupported());
    }
    let (p, old) = &c.status.raw_paths[i];
    let selected_status = if s.side == ChangeSide::Staged {
        &e.index_status
    } else {
        &e.worktree_status
    };
    let rename_on_selected_side = *selected_status == PathStatus::R;
    if rename_on_selected_side && old.is_none() {
        return Err(invalid());
    }
    // Keep both endpoints as snapshot metadata, but only a rename on the
    // selected side owns the previous path. A copy never owns its source.
    let mut identity = vec![
        if s.side == ChangeSide::Staged { 1 } else { 0 },
        u8::from(rename_on_selected_side),
    ];
    identity.extend(p);
    identity.push(0);
    if let Some(old) = old {
        identity.extend(old);
    }
    Ok(Target {
        kind: TargetKind::Change,
        proof: Some(SafeTargetProof::Change {
            path_digest: preflight::digest(&identity),
            index_digest: c.stamps[i].clone().ok_or_else(unsupported)?,
        }),
        identity,
    })
}
pub(crate) fn paths(t: &Target) -> Result<(ChangeSide, Vec<Vec<u8>>), AppError> {
    if t.kind == TargetKind::Selection {
        return paths(&super::patch::original_target(t)?);
    }
    let side = match t.identity.first() {
        Some(0) => ChangeSide::Unstaged,
        Some(1) => ChangeSide::Staged,
        _ => return Err(invalid()),
    };
    if t.identity.get(1).is_none_or(|flag| *flag > 1) {
        return Err(invalid());
    }
    let paths = t.identity[2..]
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(<[u8]>::to_vec)
        .collect::<Vec<_>>();
    if paths.is_empty() || paths.len() > 2 || t.identity[1] == 1 && paths.len() != 2 {
        return Err(invalid());
    }
    Ok((side, paths))
}
pub(crate) fn mutation_paths(t: &Target) -> Result<Vec<Vec<u8>>, AppError> {
    if t.kind == TargetKind::Selection {
        return mutation_paths(&super::patch::original_target(t)?);
    }
    let (_, mut paths) = paths(t)?;
    if t.identity[1] == 0 {
        paths.truncate(1);
    }
    Ok(paths)
}
pub(crate) fn validate_targets(
    l: &RepositoryLocation,
    intent: &ActionIntent,
    targets: &[Target],
) -> Result<(), AppError> {
    for t in targets {
        let original = if t.kind == TargetKind::Selection {
            super::patch::original_target(t)?
        } else {
            t.clone()
        };
        let t = &original;
        let (side, p) = paths(t)?;
        if matches!(
            intent,
            ActionIntent::Stage { .. } | ActionIntent::StageSelection { .. }
        ) != (side == ChangeSide::Unstaged)
        {
            return Err(invalid());
        }
        let Some(SafeTargetProof::Change {
            path_digest,
            index_digest,
        }) = &t.proof
        else {
            return Err(invalid());
        };
        if *path_digest != preflight::digest(&t.identity)
            || *index_digest != selected_stamp(l, &p[0], p.get(1).map(Vec::as_slice))?
        {
            return Err(changed());
        }
    }
    Ok(())
}
pub(crate) async fn diff(q: &ReadQuery<'_>, t: &Target) -> Result<String, AppError> {
    let (side, _) = paths(t)?;
    let paths = mutation_paths(t)?;
    let mut args = argv(&[
        "--literal-pathspecs",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-color",
    ]);
    if side == ChangeSide::Staged {
        args.push("--cached".into());
    }
    args.push("--".into());
    for p in &paths {
        args.push(std::str::from_utf8(p).map_err(|_| unsupported())?.into());
    }
    // A diff is opaque display text, never patch authority in S5-A. Configured
    // clean filters and CRLF conversion can emit diagnostics on success; retain
    // strict protocol reads elsewhere and show only a static notice here.
    let result = output(q, &args).await?;
    if !result.success {
        return Err(crate::git::error::git_failure(
            &result.stderr,
            Phase::Inspection,
        ));
    }
    let diagnostics = !result.stderr.is_empty();
    let bytes = result.stdout;
    if bytes.len() > 1024 * 1024 {
        return Ok("Diff exceeds the display limit. Whole-file actions remain available.".into());
    }
    if bytes.is_empty() && side == ChangeSide::Unstaged {
        let p = path(q.location, &paths[0])?;
        if p.is_file() {
            let mut raw = Vec::new();
            let opened = open_regular(&p)?;
            opened
                .file
                .take(1024 * 1024 + 1)
                .read_to_end(&mut raw)
                .map_err(|_| changed())?;
            if raw.len() > 1024 * 1024 || raw.contains(&0) {
                return Ok("Binary or large file. Whole-file staging is available.".into());
            }
            return Ok(String::from_utf8(raw).unwrap_or_else(|_| {
                "File encoding cannot be displayed. Whole-file staging is available.".into()
            }));
        }
    }
    let content = String::from_utf8(bytes).unwrap_or_else(|_| {
        "Diff encoding cannot be displayed. Whole-file actions are available.".into()
    });
    Ok(if diagnostics {
        format!("Git reported diagnostics while generating this preview. Whole-file actions still use configured filters and validation.\n\n{content}")
    } else {
        content
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_mutations_follow_selected_side_and_copy_sources_are_metadata_only() {
        for (index, worktree, side, both) in [
            ('R', 'M', ChangeSide::Unstaged, false),
            ('R', 'M', ChangeSide::Staged, true),
            ('M', 'R', ChangeSide::Unstaged, true),
            ('C', 'M', ChangeSide::Staged, false),
            ('M', 'C', ChangeSide::Unstaged, false),
        ] {
            let record=format!("# branch.head main\0{} {}{} N... 100644 100644 100644 {} {} {}100 new.txt\0old.txt\0","2",index,worktree,"a".repeat(40),"b".repeat(40),if index=='C'||worktree=='C'{"C"}else{"R"});
            let status = crate::git::status::parse(record.as_bytes()).unwrap();
            let e = &status.entries[0];
            let wire = vec![ChangeEntry {
                entry_id: "selected".into(),
                path_display: e.path_display.clone(),
                previous_path_display: e.previous_path_display.clone(),
                index_status: e.index_status.clone(),
                worktree_status: e.worktree_status.clone(),
                stage_capability: ChangeCapability::Eligible,
                unstage_capability: ChangeCapability::Eligible,
            }];
            let captured = Capture {
                status,
                wire,
                stamps: vec![Some("c".repeat(64))],
                digest: "d".repeat(64),
                unsupported: false,
            };
            let t = target(
                &captured,
                &ChangeSelection {
                    entry_id: "selected".into(),
                    side,
                },
            )
            .unwrap();
            assert_eq!(
                paths(&t).unwrap().1,
                vec![b"new.txt".to_vec(), b"old.txt".to_vec()]
            );
            assert_eq!(
                mutation_paths(&t).unwrap(),
                if both {
                    vec![b"new.txt".to_vec(), b"old.txt".to_vec()]
                } else {
                    vec![b"new.txt".to_vec()]
                },
                "{index}{worktree} {side:?}"
            );
        }
    }
    #[test]
    fn windows_paths_reject_tab_control_traversal_stream_and_git_directory_but_keep_literal_names()
    {
        let dir = tempfile::tempdir().unwrap();
        let location = RepositoryLocation {
            root: dir.path().into(),
            common: dir.path().into(),
            git_dir: dir.path().into(),
            common_identity: vec![],
            root_identity: None,
            bare: false,
        };
        for raw in [
            b"tab\tfile".as_slice(),
            b"../outside",
            b"file:stream",
            b"a\\b",
            b".Git/config",
            b"trailing.",
            b"nul\0path",
        ] {
            assert!(path(&location, raw).is_err(), "{raw:?}");
        }
        for raw in [
            b"-leading".as_slice(),
            b"[literal]",
            "space Ω.txt".as_bytes(),
        ] {
            assert!(path(&location, raw).is_ok(), "{raw:?}");
        }
    }
}
