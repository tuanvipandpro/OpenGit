#[allow(dead_code)]
mod support;
use opengit_lib::{backend::Backend, domain::contracts::*};
use support::history::*;
#[cfg(windows)]
fn compile(fixture: &Fixture, text: &str) -> std::path::PathBuf {
    let source = fixture.owner.child("helper.rs").unwrap();
    let executable = fixture.owner.child("helper.exe").unwrap();
    std::fs::write(&source, text).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap()
        .status
        .success());
    executable
}

fn object(fixture: &Fixture, content: &[u8]) -> String {
    String::from_utf8(fixture.run(&["hash-object", "-w", "--stdin"], Some(content)))
        .unwrap()
        .trim()
        .into()
}
fn commit(
    fixture: &Fixture,
    files: &[(&[u8], &str, String)],
    parents: &[&str],
    message: &str,
) -> String {
    let mut input = Vec::new();
    for (path, mode, oid) in files {
        input.extend_from_slice(
            format!(
                "{mode} {} {oid}\t",
                if *mode == "160000" { "commit" } else { "blob" }
            )
            .as_bytes(),
        );
        input.extend_from_slice(path);
        input.push(0);
    }
    let tree = String::from_utf8(fixture.run(&["mktree", "-z"], Some(&input)))
        .unwrap()
        .trim()
        .to_owned();
    let mut args = vec!["commit-tree", &tree];
    for parent in parents {
        args.extend(["-p", parent]);
    }
    String::from_utf8(fixture.run(&args, Some(message.as_bytes())))
        .unwrap()
        .trim()
        .into()
}
async fn selected(fixture: &Fixture, oid: &str) -> (Backend, HistorySession) {
    fixture.run(&["update-ref", "refs/heads/details", oid], None);
    let (backend, scope) = fixture.backend().await;
    let session = open(
        &backend,
        &scope,
        200,
        HistoryFilter {
            hidden_refs: vec![],
            solo_refs: vec!["refs/heads/details".into()],
        },
    )
    .await;
    (backend, session)
}
async fn details(
    backend: &Backend,
    session: &HistorySession,
    oid: &str,
    base: DiffBase,
) -> Result<CommitDetails, AppError> {
    backend
        .get_commit_details(
            "main",
            GetCommitDetailsRequest {
                context: session.context.clone(),
                commit_handle: session
                    .page
                    .commits
                    .iter()
                    .find(|c| c.oid == oid)
                    .unwrap()
                    .handle
                    .clone(),
                base,
            },
        )
        .await
}
async fn diff(backend: &Backend, session: &HistorySession, handle: &str) -> DiffContent {
    backend
        .get_commit_diff(
            "main",
            GetCommitDiffRequest {
                context: session.context.clone(),
                file_handle: handle.into(),
            },
        )
        .await
        .unwrap()
        .content
}
#[tokio::test]
async fn root_merge_parent_rename_copy_mode_and_delete_lists_match_actual_blobs_without_writes() {
    let fixture = Fixture::new("sha1", 8);
    let old = object(&fixture, b"old\n");
    let new = object(&fixture, b"new\n");
    let renamed = object(&fixture, b"rename payload\n");
    let copied = object(&fixture, b"copy payload\n");
    let base = commit(
        &fixture,
        &[
            (b"modify.txt", "100644", old.clone()),
            (b"gone.txt", "100644", old.clone()),
            (b"rename-old.txt", "100644", renamed.clone()),
            (b"copy-src.txt", "100644", copied.clone()),
            (b"executable", "100644", old.clone()),
            (b"kind.txt", "100644", old.clone()),
        ],
        &[],
        "Root message\n\ncomplete body\n",
    );
    let changed = commit(
        &fixture,
        &[
            (b"modify.txt", "100644", new.clone()),
            (b"rename-new.txt", "100644", renamed.clone()),
            (b"copy-src.txt", "100644", copied.clone()),
            (b"copied.txt", "100644", copied.clone()),
            (b"executable", "100755", old.clone()),
            (b"kind.txt", "120000", new.clone()),
        ],
        &[&base],
        "Changed\n",
    );
    let other = commit(
        &fixture,
        &[(b"modify.txt", "100644", object(&fixture, b"other parent\n"))],
        &[&base],
        "Other parent\n",
    );
    let merged = commit(
        &fixture,
        &[(b"modify.txt", "100644", new.clone())],
        &[&changed, &other],
        "Merge\n",
    );
    let (backend, session) = selected(&fixture, &merged).await;
    let before = fixture.snapshot();
    let root = details(&backend, &session, &base, DiffBase::Root)
        .await
        .unwrap();
    assert_eq!(root.message, "Root message\n\ncomplete body\n");
    assert_eq!(root.committer_name, "Fixture");
    assert_eq!(root.files.len(), 6);
    assert!(root.files.iter().all(|f| f.kind == CommitFileKind::Added));
    assert!(
        matches!(diff(&backend,&session,&root.files.iter().find(|f|f.path_display=="modify.txt").unwrap().handle).await, DiffContent::Text { original,modified,.. } if original.is_empty() && modified=="old\n")
    );
    let list = details(&backend, &session, &changed, DiffBase::Parent { index: 0 })
        .await
        .unwrap();
    for (name, kind, original, modified) in [
        ("modify.txt", CommitFileKind::Modified, "old\n", "new\n"),
        ("gone.txt", CommitFileKind::Deleted, "old\n", ""),
        (
            "rename-new.txt",
            CommitFileKind::Renamed,
            "rename payload\n",
            "rename payload\n",
        ),
        (
            "copied.txt",
            CommitFileKind::Copied,
            "copy payload\n",
            "copy payload\n",
        ),
        ("executable", CommitFileKind::Modified, "old\n", "old\n"),
        ("kind.txt", CommitFileKind::TypeChanged, "old\n", "new\n"),
    ] {
        let file = list.files.iter().find(|f| f.path_display == name).unwrap();
        assert_eq!(file.kind, kind);
        assert!(
            matches!(diff(&backend,&session,&file.handle).await,DiffContent::Text { original:a,modified:b,.. } if a==original&&b==modified)
        );
    }
    assert_eq!(
        details(&backend, &session, &changed, DiffBase::Parent { index: 0 })
            .await
            .unwrap(),
        list
    );
    let first = details(&backend, &session, &merged, DiffBase::Parent { index: 0 })
        .await
        .unwrap();
    let second = details(&backend, &session, &merged, DiffBase::Parent { index: 1 })
        .await
        .unwrap();
    let file = second
        .files
        .iter()
        .find(|f| f.path_display == "modify.txt")
        .unwrap();
    assert!(
        matches!(diff(&backend,&session,&file.handle).await,DiffContent::Text {original,modified,..} if original=="other parent\n"&&modified=="new\n")
    );
    assert!(!first.files.iter().any(|f| f.path_display == "modify.txt"));
    assert_eq!(
        details(&backend, &session, &merged, DiffBase::Root)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        details(&backend, &session, &merged, DiffBase::Parent { index: 2 })
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        details(&backend, &session, &base, DiffBase::Parent { index: 0 })
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn raw_unusual_paths_and_symlink_blobs_never_roundtrip_display_names_or_dereference() {
    let fixture = Fixture::new("sha1", 8);
    let oid = commit(
        &fixture,
        &[
            (b"\xff.txt", "100644", object(&fixture, b"first")),
            (b"\xfe.txt", "100644", object(&fixture, b"second")),
            (
                b"--output=canary\t\n\".txt",
                "100644",
                object(&fixture, b"unusual"),
            ),
            (
                b"link",
                "120000",
                object(&fixture, b"../../outside-sensitive-target"),
            ),
        ],
        &[],
        "Paths\n",
    );
    let (backend, session) = selected(&fixture, &oid).await;
    let before = fixture.snapshot();
    let result = details(&backend, &session, &oid, DiffBase::Root)
        .await
        .unwrap();
    assert_eq!(result.files.len(), 4);
    let ambiguous: Vec<_> = result
        .files
        .iter()
        .filter(|f| f.path_display == "\u{fffd}.txt")
        .collect();
    assert_eq!(ambiguous.len(), 2);
    assert_ne!(ambiguous[0].handle, ambiguous[1].handle);
    let mut texts = Vec::new();
    for file in ambiguous {
        if let DiffContent::Text { modified, .. } = diff(&backend, &session, &file.handle).await {
            texts.push(modified)
        } else {
            panic!("expected text")
        }
    }
    texts.sort();
    assert_eq!(texts, ["first", "second"]);
    let link = result
        .files
        .iter()
        .find(|f| f.path_display == "link")
        .unwrap();
    assert_eq!(link.new_mode.as_deref(), Some("120000"));
    assert!(
        matches!(diff(&backend,&session,&link.handle).await,DiffContent::Text {modified,..} if modified=="../../outside-sensitive-target")
    );
    assert_eq!(fixture.snapshot(), before);
    assert!(!fixture.path.join("canary").exists());
}
#[tokio::test]
async fn binary_encoding_gitlink_large_and_missing_blobs_have_typed_complete_results() {
    let fixture = Fixture::new("sha256", 8);
    fixture.run(&["config", "diff.ignoreSubmodules", "all"], None);
    fixture.run(&["config", "diff.submodule", "diff"], None);
    let missing = object(&fixture, b"will disappear");
    let oid = commit(
        &fixture,
        &[
            (b"binary", "100644", object(&fixture, b"a\0b")),
            (b"encoding", "100644", object(&fixture, b"\xff\xfe")),
            (b"submodule", "160000", fixture.oid("main")),
            (b"large", "100644", object(&fixture, &vec![b'x'; 1_048_577])),
            (b"exact", "100644", object(&fixture, &vec![b'x'; 1_048_576])),
            (b"missing", "100644", missing.clone()),
        ],
        &[],
        "Bounds\n",
    );
    let (backend, session) = selected(&fixture, &oid).await;
    let result = details(&backend, &session, &oid, DiffBase::Root)
        .await
        .unwrap();
    let path = fixture
        .path
        .join(".git/objects")
        .join(&missing[..2])
        .join(&missing[2..]);
    fixture.owner.require_owned_file(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    for (name, reason) in [
        ("binary", DiffUnavailableReason::Binary),
        ("encoding", DiffUnavailableReason::UnsupportedEncoding),
        ("submodule", DiffUnavailableReason::Gitlink),
        ("large", DiffUnavailableReason::TooLarge),
        ("missing", DiffUnavailableReason::MissingObject),
    ] {
        let file = result
            .files
            .iter()
            .find(|f| f.path_display == name)
            .unwrap();
        assert_eq!(
            diff(&backend, &session, &file.handle).await,
            DiffContent::Unavailable { reason }
        );
    }
    assert!(
        matches!(diff(&backend,&session,&result.files.iter().find(|f|f.path_display=="exact").unwrap().handle).await,DiffContent::Text {modified,..} if modified.len()==1_048_576)
    );
}
#[tokio::test]
async fn paired_raw_blob_limits_still_enforce_the_encoded_json_response_budget() {
    let fixture = Fixture::new("sha1", 8);
    let blob = object(&fixture, &vec![1u8; 1_048_576]);
    let base = commit(
        &fixture,
        &[(b"escaped", "100644", blob.clone())],
        &[],
        "Base\n",
    );
    let changed = commit(
        &fixture,
        &[(b"escaped", "100755", blob)],
        &[&base],
        "Mode\n",
    );
    let (backend, session) = selected(&fixture, &changed).await;
    let result = details(&backend, &session, &changed, DiffBase::Parent { index: 0 })
        .await
        .unwrap();
    let before = fixture.snapshot();
    let request = GetCommitDiffRequest {
        context: session.context.clone(),
        file_handle: result.files[0].handle.clone(),
    };
    assert_eq!(
        backend
            .get_commit_diff("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn handles_reject_other_windows_snapshots_expired_scope_and_ref_changes() {
    let fixture = Fixture::new("sha1", 8);
    let oid = commit(
        &fixture,
        &[(b"a", "100644", object(&fixture, b"text"))],
        &[],
        "Handle\n",
    );
    let (backend, session) = selected(&fixture, &oid).await;
    let result = details(&backend, &session, &oid, DiffBase::Root)
        .await
        .unwrap();
    let request = GetCommitDiffRequest {
        context: session.context.clone(),
        file_handle: result.files[0].handle.clone(),
    };
    backend.get_workspace("other").unwrap();
    assert_eq!(
        backend
            .get_commit_diff("other", request.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let other = open(&backend, &session.context.scope, 200, filter()).await;
    let mut bad = request.clone();
    bad.context = other.context;
    assert_eq!(
        backend.get_commit_diff("main", bad).await.unwrap_err().code,
        ErrorCode::InvalidHandle
    );
    fixture.run(&["update-ref", "refs/heads/changed", "main"], None);
    assert_eq!(
        backend
            .get_commit_diff("main", request.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::ExternalChange
    );
    assert_eq!(
        backend
            .get_commit_diff("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.close_window("main");
    assert_eq!(
        details(&backend, &session, &oid, DiffBase::Root)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
}
#[tokio::test]
async fn outside_filter_search_handles_open_details_and_empty_or_text_blobs_without_changing_filters(
) {
    let fixture = Fixture::new("sha1", 8);
    let outside = commit(
        &fixture,
        &[
            (b"out.txt", "100644", object(&fixture, b"outside content")),
            (b"empty.txt", "100644", object(&fixture, b"")),
        ],
        &[],
        "Outside detail needle\n",
    );
    fixture.run(
        &["update-ref", "refs/heads/outside-details", &outside],
        None,
    );
    let inside = commit(
        &fixture,
        &[(b"in.txt", "100644", object(&fixture, b"inside"))],
        &[],
        "Inside\n",
    );
    let (backend, session) = selected(&fixture, &inside).await;
    let before = fixture.snapshot();
    assert!(!session.roots.contains(&outside));
    let found = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: outside.clone(),
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(found.commits.len(), 1);
    assert!(found.commits[0].outside_view);
    let result = backend
        .get_commit_details(
            "main",
            GetCommitDetailsRequest {
                context: session.context.clone(),
                commit_handle: found.commits[0].commit.handle.clone(),
                base: DiffBase::Root,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.commit.oid, outside);
    assert_eq!(result.files.len(), 2);
    for file in result.files {
        let expected = if file.path_display == "empty.txt" {
            ""
        } else {
            "outside content"
        };
        assert!(
            matches!(diff(&backend,&session,&file.handle).await,DiffContent::Text {original,modified,..} if original.is_empty()&&modified==expected)
        );
    }
    assert_eq!(fixture.snapshot(), before);
    assert!(!session.roots.contains(&outside));
}
#[tokio::test]
async fn unpageable_file_lists_above_ten_thousand_return_output_limit() {
    let fixture = Fixture::new("sha1", 8);
    let blob = object(&fixture, b"x");
    let paths: Vec<_> = (0..10_001).map(|i| format!("file-{i}")).collect();
    let files: Vec<_> = paths
        .iter()
        .map(|p| (p.as_bytes(), "100644", blob.clone()))
        .collect();
    let oid = commit(&fixture, &files, &[], "Many files\n");
    let (backend, session) = selected(&fixture, &oid).await;
    let before = fixture.snapshot();
    assert_eq!(
        details(&backend, &session, &oid, DiffBase::Root)
            .await
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn actual_git_rename_candidate_warning_refuses_the_complete_list_without_partial_classification(
) {
    let fixture = Fixture::new("sha1", 8);
    let old = object(&fixture, b"old shared content\n");
    let new = object(&fixture, b"new shared content\n");
    let old_names: Vec<_> = (0..1001).map(|i| format!("old-{i}.txt")).collect();
    let new_names: Vec<_> = (0..1001).map(|i| format!("new-{i}.txt")).collect();
    let old_files: Vec<_> = old_names
        .iter()
        .map(|p| (p.as_bytes(), "100644", old.clone()))
        .collect();
    let new_files: Vec<_> = new_names
        .iter()
        .map(|p| (p.as_bytes(), "100644", new.clone()))
        .collect();
    let base = commit(&fixture, &old_files, &[], "Candidate base\n");
    let changed = commit(&fixture, &new_files, &[&base], "Candidate overflow\n");
    let output = fixture
        .command(&fixture.path)
        .args([
            "diff-tree",
            "-r",
            "--raw",
            "-z",
            "--no-commit-id",
            "--no-abbrev",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "-C",
            "--find-copies-harder",
            "-l1000",
            &base,
            &changed,
            "--",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        !output.stderr.is_empty(),
        "actual Git must exercise the warning branch"
    );
    assert!(!output.stdout.is_empty());
    let (backend, session) = selected(&fixture, &changed).await;
    let before = fixture.snapshot();
    let error = details(&backend, &session, &changed, DiffBase::Parent { index: 0 })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputLimit);
    assert!(!error.message.contains("exhaustive"));
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(
        details(&backend, &session, &base, DiffBase::Root)
            .await
            .unwrap()
            .files
            .len(),
        1001
    );
}
#[cfg(windows)]
#[tokio::test]
async fn configured_external_diff_and_textconv_positive_canaries_are_never_invoked_by_api_reads() {
    let fixture = Fixture::new("sha1", 8);
    let source = fixture.owner.child("canary.rs").unwrap();
    let executable = fixture.owner.child("canary.exe").unwrap();
    let marker = fixture.owner.child("invoked").unwrap();
    std::fs::write(
        &source,
        format!(
            "fn main() {{ std::fs::write({:?},b\"invoked\").unwrap(); println!(\"converted\"); }}",
            marker
        ),
    )
    .unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap()
        .status
        .success());
    let command = format!("\"{}\"", fixture.git_path(&executable).replace('\\', "/"));
    fixture.run(&["config", "diff.external", &command], None);
    fixture.run(&["config", "diff.fixture.textconv", &command], None);
    std::fs::write(fixture.path.join(".gitattributes"), b"*.txt diff=fixture\n").unwrap();
    let base = commit(
        &fixture,
        &[(b"a.txt", "100644", object(&fixture, b"old"))],
        &[],
        "Base\n",
    );
    let changed = commit(
        &fixture,
        &[(b"a.txt", "100644", object(&fixture, b"new"))],
        &[&base],
        "Changed\n",
    );
    fixture.run(&["diff", "--ext-diff", &base, &changed], None);
    assert!(marker.is_file());
    std::fs::remove_file(&marker).unwrap();
    fixture.run(
        &[
            "-c",
            "diff.external=",
            "diff",
            "--no-ext-diff",
            "--textconv",
            &base,
            &changed,
        ],
        None,
    );
    assert!(marker.is_file());
    std::fs::remove_file(&marker).unwrap();
    let (backend, session) = selected(&fixture, &changed).await;
    let before = fixture.snapshot();
    let details = details(&backend, &session, &changed, DiffBase::Parent { index: 0 })
        .await
        .unwrap();
    assert!(
        matches!(diff(&backend,&session,&details.files[0].handle).await,DiffContent::Text {original,modified,..} if original=="old"&&modified=="new")
    );
    assert!(!marker.exists());
    assert_eq!(fixture.snapshot(), before);
}
#[cfg(windows)]
#[tokio::test]
async fn unrelated_success_stderr_refuses_list_with_accurate_static_warning_instead_of_claiming_overflow(
) {
    let fixture = Fixture::new("sha1", 8);
    let oid = commit(
        &fixture,
        &[(b"a", "100644", object(&fixture, b"content"))],
        &[],
        "Warning\n",
    );
    fixture.run(&["update-ref", "refs/heads/details", &oid], None);
    let text = format!(
        r#"use std::process::{{Command,Stdio}};fn main(){{let args:Vec<_>=std::env::args_os().skip(1).collect();let warning=args.iter().any(|s|s=="diff-tree");let status=Command::new({:?}).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();if warning&&status.success(){{eprintln!("fixture unrelated success notice");}}std::process::exit(status.code().unwrap_or(1));}}"#,
        fixture.git.executable()
    );
    let backend = Backend::with_git(
        &fixture.owner.child("warning-db").unwrap(),
        opengit_lib::git::runner::GitRunner::trusted(compile(&fixture, &text)).unwrap(),
    )
    .unwrap();
    let selection = backend
        .register_repository("main", &fixture.path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = HistoryScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    let session = open(&backend, &scope, 200, filter()).await;
    let error = details(&backend, &session, &oid, DiffBase::Root)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::OutputLimit);
    assert!(error.message.contains("file-list warning"));
    assert!(!error.message.contains("candidate"));
    assert!(!error.message.contains("fixture"));
}
