#[allow(dead_code)]
mod support;
use opengit_lib::{
    backend::Backend,
    domain::contracts::*,
    git::runner::{Cancellation, GitRunner, INPUT_LIMIT_BYTES},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use support::history::*;

fn page_request(session: &HistorySession, size: u32) -> LoadHistoryPageRequest {
    LoadHistoryPageRequest {
        context: session.context.clone(),
        session_id: session.session_id.clone(),
        cursor: session.page.next_cursor.clone().unwrap(),
        page_size: Some(size),
    }
}
#[tokio::test]
async fn external_ref_head_shallow_and_unsupported_changes_invalidate_existing_sessions() {
    let fixture = Fixture::new("sha1", 12);
    let (backend, scope) = fixture.backend().await;
    for action in ["ref", "head", "shallow", "replace", "graft", "promisor"] {
        let session = open(&backend, &scope, 1, filter()).await;
        let request = page_request(&session, 1);
        match action {
            "ref" => {
                fixture.run(&["update-ref", "refs/heads/external", "main"], None);
            }
            "head" => {
                fixture.run(&["symbolic-ref", "HEAD", "refs/heads/alpha"], None);
            }
            "shallow" => {
                std::fs::write(
                    fixture.path.join(".git/shallow"),
                    format!("{}\n", fixture.oid("main~2")),
                )
                .unwrap();
            }
            "replace" => {
                fixture.run(&["replace", "main~2", "main~3"], None);
            }
            "graft" => {
                std::fs::write(
                    fixture.path.join(".git/info/grafts"),
                    format!("{}\n", fixture.oid("main")),
                )
                .unwrap();
            }
            "promisor" => {
                fixture.run(&["config", "remote.origin.promisor", "true"], None);
            }
            _ => unreachable!(),
        }
        assert_eq!(
            backend
                .load_history_page("main", request.clone())
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::ExternalChange,
            "{action}"
        );
        assert_eq!(
            backend
                .load_history_page("main", request)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidHandle
        );
        match action {
            "ref" => {
                fixture.run(&["update-ref", "-d", "refs/heads/external"], None);
            }
            "head" => {
                fixture.run(&["symbolic-ref", "HEAD", "refs/heads/main"], None);
            }
            "shallow" => {
                std::fs::remove_file(fixture.path.join(".git/shallow")).unwrap();
            }
            "replace" => {
                fixture.run(&["replace", "-d", "main~2"], None);
            }
            "graft" => {
                std::fs::remove_file(fixture.path.join(".git/info/grafts")).unwrap();
            }
            "promisor" => {
                fixture.run(&["config", "--unset", "remote.origin.promisor"], None);
            }
            _ => (),
        }
    }
}
#[cfg(windows)]
#[tokio::test]
async fn sh01_metadata_raw_frames_output_overflow_and_size_overflow_fail_without_publishing() {
    for (action, expected) in [
        ("sh-order-malformed", ErrorCode::MalformedOutput),
        ("sh-raw-malformed", ErrorCode::MalformedOutput),
        ("sh-order-output", ErrorCode::OutputLimit),
        ("sh-size-overflow", ErrorCode::OutputLimit),
    ] {
        let fixture = Fixture::shallow_native_shape("sha1", 40);
        let shallow = shallow_order_clone(&fixture);
        let before = fixture.snapshot_at(&shallow);
        let shallow_bytes = std::fs::read(shallow.join(".git/shallow")).unwrap();
        let (backend, scope) = proxy_at(&fixture, proxy(&fixture, action, ""), &shallow).await;
        let error = backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope,
                    filter: filter(),
                    page_size: Some(1),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, expected, "{action}");
        assert_eq!(fixture.snapshot_at(&shallow), before);
        assert_eq!(
            std::fs::read(shallow.join(".git/shallow")).unwrap(),
            shallow_bytes
        );
    }
}
#[cfg(windows)]
fn shallow_order_clone(fixture: &Fixture) -> PathBuf {
    let shallow = fixture.owner.child("shallow-order").unwrap();
    let uri = url::Url::from_file_path(Path::new(&fixture.git_path(&fixture.path)))
        .unwrap()
        .to_string();
    fixture.run(
        &[
            "clone",
            "--depth",
            "5",
            "--no-local",
            "--",
            &uri,
            &fixture.git_path(&shallow),
        ],
        None,
    );
    shallow
}
#[cfg(windows)]
async fn proxy_at(
    fixture: &Fixture,
    runner: GitRunner,
    path: &Path,
) -> (Arc<Backend>, HistoryScope) {
    let backend =
        Arc::new(Backend::with_git(&fixture.owner.child("sh-order-db").unwrap(), runner).unwrap());
    let selection = backend
        .register_repository("main", path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    let scope = HistoryScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "1".into(),
    };
    (backend, scope)
}
#[cfg(windows)]
#[tokio::test]
async fn sh01_capture_uses_shared_timeout_cancel_and_final_fingerprint_discards_changed_refs() {
    for action in ["timeout", "cancel", "refs"] {
        let fixture = Fixture::shallow_native_shape("sha1", 40);
        let shallow = shallow_order_clone(&fixture);
        let cut_file = shallow.join(".git/shallow");
        let cut_bytes = std::fs::read(&cut_file).unwrap();
        let mut runner = proxy(
            &fixture,
            if action == "refs" {
                "sh-order-ref"
            } else {
                "sh-order-block"
            },
            &format!("{}\n", fixture.oid("main")),
        );
        if action == "timeout" {
            runner.timeout = Duration::from_secs(2);
        }
        let (backend, scope) = proxy_at(&fixture, runner, &shallow).await;
        let service = backend.clone();
        let task = tokio::spawn(async move {
            service
                .open_history(
                    "main",
                    OpenHistoryRequest {
                        scope,
                        filter: filter(),
                        page_size: Some(1),
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), async {
            while !fixture.owner.root().join("sh-order-entered").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        if action == "cancel" {
            backend.close_window("main");
        }
        let error = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.code,
            match action {
                "timeout" => ErrorCode::Timeout,
                "cancel" => ErrorCode::Interrupted,
                _ => ErrorCode::ExternalChange,
            }
        );
        assert_eq!(std::fs::read(&cut_file).unwrap(), cut_bytes);
        std::fs::write(&cut_file, &cut_bytes).unwrap();
    }
}
#[cfg(windows)]
#[tokio::test]
async fn sh01_nonshallow_order_keeps_cli_rank_and_skips_parent_metadata_capture() {
    let fixture = Fixture::new("sha1", 40);
    let before = fixture.snapshot();
    let (backend, scope) = with_runner(&fixture, proxy(&fixture, "sh-order-malformed", "")).await;
    let session = open(&backend, &scope, 200, filter()).await;
    let original = fixture.order_at(&fixture.path, &session.roots);
    assert_eq!(
        session
            .page
            .commits
            .iter()
            .map(|c| c.oid.clone())
            .collect::<Vec<_>>(),
        original
    );
    assert!(!fixture.owner.root().join("sh-order-entered").exists());
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn unsupported_partial_markers_and_ancestry_overrides_fail_before_object_reads() {
    for flag in ["replace", "graft", "partial", "promisor", "pack"] {
        let fixture = Fixture::new("sha1", 8);
        let (backend, scope) = fixture.backend().await;
        match flag {
            "replace" => {
                fixture.run(&["replace", "main~1", "main~2"], None);
            }
            "graft" => {
                std::fs::write(fixture.path.join(".git/info/grafts"), fixture.oid("main")).unwrap();
            }
            "partial" => {
                fixture.run(&["config", "extensions.partialClone", "origin"], None);
            }
            "promisor" => {
                fixture.run(&["config", "remote.hidden.promisor", "true"], None);
            }
            "pack" => {
                std::fs::write(fixture.path.join(".git/objects/pack/owned.promisor"), b"").unwrap();
            }
            _ => (),
        }
        let before = fixture.snapshot();
        let failure = backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope,
                    filter: filter(),
                    page_size: None,
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(failure.code, ErrorCode::InvalidRequest, "{flag}");
        assert_eq!(fixture.snapshot(), before);
    }
}
#[tokio::test]
async fn unborn_bare_detached_linked_and_shallow_repositories_keep_scope_and_raw_parent_truth() {
    let fixture = Fixture::new("sha1", 12);
    let unborn = fixture.owner.child("unborn").unwrap();
    std::fs::create_dir(&unborn).unwrap();
    fixture.run_at(&unborn, &["init", "-b", "unborn"], None);
    let (backend, scope) = fixture.backend_at(&unborn, "main").await;
    let session = open(&backend, &scope, 1, filter()).await;
    assert_eq!(session.head.kind, HeadKind::Unborn);
    assert!(session.page.commits.is_empty());
    assert!(session.page.next_cursor.is_none());
    let bare = fixture.owner.child("bare.git").unwrap();
    fixture.run(
        &[
            "clone",
            "--bare",
            "--no-local",
            "--",
            &fixture.git_path(&fixture.path),
            &fixture.git_path(&bare),
        ],
        None,
    );
    let (backend, scope) = fixture.backend_at(&bare, "main").await;
    assert!(scope.worktree_id.is_none());
    let session = open(&backend, &scope, 200, filter()).await;
    assert_dag(
        &collect(&backend, &session, 200).await,
        &fixture.oracle(&["--all"]),
    );
    let linked = fixture.owner.child("linked").unwrap();
    fixture.run(
        &[
            "worktree",
            "add",
            "--detach",
            &fixture.git_path(&linked),
            "main~3",
        ],
        None,
    );
    let (backend, scope) = fixture.backend_at(&linked, "main").await;
    let session = open(&backend, &scope, 200, filter()).await;
    assert_eq!(session.head.kind, HeadKind::Detached);
    assert!(session.roots.contains(&fixture.oid("main~3")));
    let mut wrong = scope.clone();
    wrong.worktree_id = Some("other-worktree".into());
    assert_eq!(
        backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope: wrong,
                    filter: filter(),
                    page_size: None
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidHandle
    );
    let shallow = fixture.owner.child("shallow").unwrap();
    let uri = url::Url::from_file_path(Path::new(&fixture.git_path(&fixture.path)))
        .unwrap()
        .to_string();
    fixture.run(
        &[
            "clone",
            "--depth",
            "2",
            "--no-local",
            "--",
            &uri,
            &fixture.git_path(&shallow),
        ],
        None,
    );
    let (backend, scope) = fixture.backend_at(&shallow, "main").await;
    let session = open(&backend, &scope, 1, filter()).await;
    let commits = collect(&backend, &session, 1).await;
    let shallow_file = std::fs::read_to_string(shallow.join(".git/shallow")).unwrap();
    for commit in commits
        .iter()
        .filter(|c| shallow_file.lines().any(|oid| oid == c.oid))
    {
        let raw = fixture.run_at(&shallow, &["cat-file", "commit", &commit.oid], None);
        let parents: Vec<_> = String::from_utf8(raw)
            .unwrap()
            .lines()
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.strip_prefix("parent ").map(str::to_owned))
            .collect();
        assert!(!parents.is_empty());
        assert_eq!(
            commit
                .parents
                .iter()
                .map(|p| p.oid.clone())
                .collect::<Vec<_>>(),
            parents
        );
        assert!(commit
            .parents
            .iter()
            .all(|p| p.boundary == ParentBoundary::Shallow));
    }
    assert!(commits
        .iter()
        .flat_map(|c| &c.parents)
        .any(|p| p.boundary == ParentBoundary::Shallow));
}
#[tokio::test]
async fn window_close_forget_and_switch_revoke_handles_and_session_lru_is_bounded() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let first = open(&backend, &scope, 1, filter()).await;
    for _ in 0..8 {
        open(&backend, &scope, 1, filter()).await;
    }
    assert_eq!(
        backend
            .load_history_page("main", page_request(&first, 1))
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidHandle
    );
    let active = open(&backend, &scope, 1, filter()).await;
    backend
        .select_repository(
            "main",
            SelectRepositoryRequest {
                repo_id: scope.repo_id.clone(),
                worktree_id: scope.worktree_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        backend
            .load_history_page("main", page_request(&active, 1))
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidHandle
    );
    let active = open(&backend, &scope, 1, filter()).await;
    backend.close_window("main");
    assert_eq!(
        backend
            .load_history_page("main", page_request(&active, 1))
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidHandle
    );
    backend.get_workspace("main").unwrap();
    let active = open(&backend, &scope, 1, filter()).await;
    backend
        .forget_repository(
            "main",
            ForgetRepositoryRequest {
                repo_id: scope.repo_id,
            },
        )
        .unwrap();
    assert_eq!(
        backend
            .load_history_page("main", page_request(&active, 1))
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidHandle
    );
}
#[tokio::test]
async fn request_bounds_option_looking_refs_and_query_cursors_never_become_git_arguments() {
    let fixture = Fixture::new("sha1", 8);
    let (backend, scope) = fixture.backend().await;
    let before = fixture.snapshot();
    for (page_size, reference) in [
        (0, "refs/heads/main"),
        (501, "refs/heads/main"),
        (1, "--all"),
        (1, "refs/heads/a..b"),
        (1, "refs/heads/a\n--all"),
    ] {
        assert_eq!(
            backend
                .open_history(
                    "main",
                    OpenHistoryRequest {
                        scope: scope.clone(),
                        page_size: Some(page_size),
                        filter: HistoryFilter {
                            hidden_refs: vec![reference.into()],
                            solo_refs: vec![]
                        }
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let session = open(&backend, &scope, 1, filter()).await;
    for query in [String::new(), "a".repeat(513), "\0".into(), "Ω".repeat(257)] {
        assert_eq!(
            backend
                .search_history(
                    "main",
                    SearchHistoryRequest {
                        context: session.context.clone(),
                        session_id: session.session_id.clone(),
                        query,
                        cursor: None,
                        page_size: None
                    }
                )
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let literal = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: "--all".into(),
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    assert!(literal.commits.is_empty());
    assert_eq!(fixture.snapshot(), before);
}

#[cfg(windows)]
fn compile(root: &Path, source_text: &str) -> PathBuf {
    let source = root.join("fixture.rs");
    let executable = root.join("fixture.exe");
    std::fs::write(&source, source_text).unwrap();
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
#[cfg(windows)]
fn proxy(fixture: &Fixture, action: &str, extra: &str) -> GitRunner {
    let text = format!(
        r#"use std::{{ffi::OsString,io::Write,path::Path,process::{{Command,Stdio}},time::Duration}};
fn main(){{let args:Vec<OsString>=std::env::args_os().skip(1).collect();let root=Path::new({root:?});let action={action:?};
if action=="old" && args==[OsString::from("--version")] {{print!("git version 2.31.0");return;}}
if action=="old" && args.iter().any(|s|s=="--no-lazy-fetch"){{std::fs::OpenOptions::new().create(true).append(true).open(root.join("probes")).unwrap().write_all(b"p").unwrap();std::process::exit(129);}}
if args.iter().any(|s|s=="--no-walk=unsorted"){{std::fs::write(root.join("sh-order-entered"),b"entered").unwrap();
if action=="sh-order-malformed"{{println!("not-an-object");return;}}
if action=="sh-order-output"{{std::io::stdout().write_all(&vec![b'x';8*1024*1024+1]).unwrap();return;}}
if action=="sh-order-block"{{std::thread::sleep(Duration::from_secs(60));}}
if action=="sh-order-ref"&&std::fs::OpenOptions::new().create_new(true).write(true).open(root.join("sh-order-mutated")).is_ok(){{std::fs::write(root.join("shallow-order/.git/refs/heads/raced"),{extra:?}).unwrap();}}
}}
if action=="sh-size-overflow"&&args.iter().any(|s|s=="--batch-check"){{let mut input=String::new();std::io::Read::read_to_string(&mut std::io::stdin(),&mut input).unwrap();for id in input.lines(){{println!("{{}} commit 18446744073709551615",id);}}return;}}
if action=="tail-error"&&args.iter().any(|s|s=="rev-parse")&&args.iter().any(|s|s.to_str().is_some_and(|s|s.starts_with("refs/stash@{{"))){{std::process::exit(1);}}
if action=="tail-fatal"&&args.iter().any(|s|s=="rev-parse")&&args.iter().any(|s|s.to_str().is_some_and(|s|s.starts_with("refs/stash@{{"))){{let status=Command::new({git:?}).args(["-c","invalid=value"]).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();std::fs::write(root.join("tail-fatal-code"),status.code().unwrap_or(1).to_string()).unwrap();std::process::exit(status.code().unwrap_or(1));}}
if action=="symbolic-error"&&args.iter().any(|s|s=="--no-lazy-fetch")&&args.iter().any(|s|s=="symbolic-ref"){{std::process::exit(2);}}
if args.iter().any(|s|s=="cat-file"||s=="rev-list"){{std::fs::write(root.join("object-command"),b"seen").unwrap();}}
if (action=="shallow-aba"||action=="graft-aba")&&args.iter().any(|s|s=="rev-list")&&std::fs::OpenOptions::new().create_new(true).write(true).open(root.join("did-action")).is_ok(){{
let path=root.join(if action=="shallow-aba"{{"history Ω/.git/shallow"}}else{{"history Ω/.git/info/grafts"}});std::fs::write(&path,{extra:?}).unwrap();
let status=Command::new({git:?}).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();std::fs::remove_file(path).unwrap();std::process::exit(status.code().unwrap_or(1));}}
if args.iter().any(|s|s=="cat-file")&&args.iter().any(|s|s=="--batch")&&std::fs::OpenOptions::new().create_new(true).write(true).open(root.join("did-action")).is_ok(){{
if action=="sh-raw-malformed"{{print!("invalid raw frame");return;}}
std::fs::write(root.join("entered"),if args.iter().any(|s|s=="--no-lazy-fetch"){{b"safe".as_slice()}}else{{b"unsafe".as_slice()}}).unwrap();
if action=="ref"{{std::fs::write(root.join("history Ω/.git/refs/heads/raced"),{extra:?}).unwrap();}}
if action=="partial"{{std::fs::OpenOptions::new().append(true).open(root.join("history Ω/.git/config")).unwrap().write_all({extra:?}.as_bytes()).unwrap();let object=std::fs::read_to_string(root.join("missing-object-path")).unwrap();std::fs::remove_file(object).unwrap();}}
if action=="block"{{std::thread::sleep(Duration::from_secs(60));}}
if action=="pause"{{while !root.join("release").exists(){{std::thread::sleep(Duration::from_millis(1));}}}}
}}
let status=Command::new({git:?}).args(args).stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit()).status().unwrap();std::process::exit(status.code().unwrap_or(1));}}"#,
        root = fixture.owner.root(),
        action = action,
        extra = extra,
        git = fixture.git.executable()
    );
    GitRunner::trusted(compile(fixture.owner.root(), &text)).unwrap()
}
#[cfg(windows)]
async fn with_runner(fixture: &Fixture, runner: GitRunner) -> (Arc<Backend>, HistoryScope) {
    let data = fixture.owner.child("proxy-database").unwrap();
    let backend = Arc::new(Backend::with_git(&data, runner).unwrap());
    let selection = backend
        .register_repository("main", &fixture.path)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    (
        backend,
        HistoryScope {
            repo_id: selection.repo_id,
            worktree_id: selection.worktree_id,
            generation: "1".into(),
        },
    )
}
#[cfg(windows)]
#[tokio::test]
async fn older_git_keeps_foundation_inspection_but_history_capability_rejects_before_object_queries(
) {
    let fixture = Fixture::new("sha1", 8);
    let runner = proxy(&fixture, "old", "");
    let (backend, scope) = with_runner(&fixture, runner).await;
    let inspection = backend
        .inspect_repository(
            "main",
            InspectRepositoryRequest {
                repo_id: scope.repo_id.clone(),
                worktree_id: scope.worktree_id.clone(),
                generation: "1".into(),
            },
        )
        .await
        .unwrap();
    assert!(inspection.head.oid.is_some());
    let view = backend
        .get_repository_view(
            "main",
            GetRepositoryViewRequest {
                scope: scope.clone(),
            },
        )
        .unwrap();
    let saved = backend
        .save_repository_view(
            "main",
            SaveRepositoryViewRequest {
                scope: scope.clone(),
                expected_revision: view.view_revision,
                view: view.view,
            },
        )
        .unwrap();
    assert_eq!(saved.view_revision, "1");
    assert!(
        !fixture.owner.root().join("probes").exists(),
        "preferences must not probe Git capabilities"
    );
    for _ in 0..2 {
        let error = backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope: scope.clone(),
                    filter: filter(),
                    page_size: None,
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::UnsupportedGit);
        assert_eq!(error.phase, Phase::Inspection);
        assert!(error.message.contains("safe history reads"));
    }
    assert_eq!(
        std::fs::read(fixture.owner.root().join("probes")).unwrap(),
        b"p"
    );
    assert!(!fixture.owner.root().join("object-command").exists());
}
#[cfg(windows)]
#[tokio::test]
async fn empty_unexpected_exit_codes_are_never_credited_as_stash_eof_or_detached_head() {
    for (action, expected) in [
        ("tail-error", ErrorCode::NotFound),
        ("tail-fatal", ErrorCode::NotFound),
        ("symbolic-error", ErrorCode::MalformedOutput),
    ] {
        let fixture = Fixture::new("sha1", 8);
        fixture.run(
            &["update-ref", "--create-reflog", "refs/stash", "main"],
            None,
        );
        let before = fixture.snapshot();
        let (backend, scope) = with_runner(&fixture, proxy(&fixture, action, "")).await;
        assert_eq!(
            backend
                .open_history(
                    "main",
                    OpenHistoryRequest {
                        scope,
                        filter: filter(),
                        page_size: Some(1)
                    }
                )
                .await
                .unwrap_err()
                .code,
            expected
        );
        if action == "tail-fatal" {
            assert_eq!(
                std::fs::read(fixture.owner.root().join("tail-fatal-code")).unwrap(),
                b"128"
            );
        }
        assert!(!fixture.owner.root().join("object-command").exists());
        assert_eq!(fixture.snapshot(), before);
    }
}
#[cfg(windows)]
#[tokio::test]
async fn external_change_between_object_read_and_final_fingerprint_discards_entire_open() {
    let fixture = Fixture::new("sha1", 8);
    let runner = proxy(&fixture, "ref", &format!("{}\n", fixture.oid("main")));
    let (backend, scope) = with_runner(&fixture, runner).await;
    let failure = backend
        .open_history(
            "main",
            OpenHistoryRequest {
                scope,
                filter: filter(),
                page_size: Some(2),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(failure.code, ErrorCode::ExternalChange);
    assert!(fixture.path.join(".git/refs/heads/raced").exists());
}
#[cfg(windows)]
#[tokio::test]
async fn closing_window_cancels_inflight_stdin_history_read_and_reopen_can_recover() {
    let fixture = Fixture::new("sha1", 8);
    let runner = proxy(&fixture, "block", "");
    let (backend, scope) = with_runner(&fixture, runner).await;
    let service = backend.clone();
    let request = OpenHistoryRequest {
        scope: scope.clone(),
        filter: filter(),
        page_size: Some(2),
    };
    let task = tokio::spawn(async move { service.open_history("main", request).await });
    tokio::time::timeout(Duration::from_secs(20), async {
        while !fixture.owner.root().join("entered").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    backend.close_window("main");
    let failure = tokio::time::timeout(Duration::from_secs(6), task)
        .await
        .unwrap()
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(failure.code, ErrorCode::Interrupted);
    backend.get_workspace("main").unwrap();
    let recovered = open(&backend, &scope, 2, filter()).await;
    assert_eq!(recovered.page.commits.len(), 2);
}
#[cfg(windows)]
#[tokio::test]
async fn late_promisor_configuration_and_missing_object_cannot_trigger_lazy_network_fetch() {
    let fixture = Fixture::new("sha1", 8);
    let tree = fixture.oid("main^{tree}");
    let commit = String::from_utf8(fixture.run(
        &[
            "commit-tree",
            &tree,
            "-p",
            "main",
            "-m",
            "Loose late object",
        ],
        None,
    ))
    .unwrap()
    .trim()
    .to_owned();
    fixture.run(&["update-ref", "refs/heads/loose", &commit], None);
    let object = fixture
        .path
        .join(".git/objects")
        .join(&commit[..2])
        .join(&commit[2..]);
    fixture.owner.require_owned_file(&object).unwrap();
    let contents = std::fs::read(&object).unwrap();
    std::fs::write(
        fixture.owner.root().join("missing-object-path"),
        object.to_str().unwrap(),
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = format!(
        "\n[remote \"late\"]\n\turl = http://127.0.0.1:{}/owned\n\tpromisor = true\n",
        listener.local_addr().unwrap().port()
    );
    let runner = proxy(&fixture, "partial", &config);
    let (backend, scope) = with_runner(&fixture, runner).await;
    let result = backend
        .open_history(
            "main",
            OpenHistoryRequest {
                scope,
                filter: HistoryFilter {
                    hidden_refs: vec![],
                    solo_refs: vec!["refs/heads/loose".into()],
                },
                page_size: Some(1),
            },
        )
        .await
        .err()
        .unwrap();
    assert!(matches!(
        result.code,
        ErrorCode::NotFound | ErrorCode::ExternalChange
    ));
    assert_eq!(
        std::fs::read(fixture.owner.root().join("entered")).unwrap(),
        b"safe"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fixture.owner.guard(fixture.owner.root()).unwrap();
    std::fs::write(object, contents).unwrap();
}
#[cfg(windows)]
#[tokio::test]
async fn transient_restored_shallow_or_graft_files_cannot_rewrite_captured_positive_root_traversal()
{
    for action in ["shallow-aba", "graft-aba"] {
        let fixture = Fixture::new("sha1", 12);
        let oracle = fixture.oracle(&["--all"]);
        let before = fixture.snapshot();
        let runner = proxy(&fixture, action, &format!("{}\n", fixture.oid("main")));
        let (backend, scope) = with_runner(&fixture, runner).await;
        let session = open(&backend, &scope, 200, filter()).await;
        assert_dag(&session.page.commits, &oracle);
        assert_eq!(fixture.snapshot(), before);
        assert!(!fixture.path.join(".git/shallow").exists());
        assert!(!fixture.path.join(".git/info/grafts").exists());
    }
}
#[cfg(windows)]
#[tokio::test]
async fn existing_shallow_lease_denies_write_or_replace_during_query_and_releases_after_confirmed_stop(
) {
    let fixture = Fixture::new("sha1", 12);
    let path = fixture.path.join(".git/shallow");
    let bytes = format!("{}\n", fixture.oid("main~3"));
    std::fs::write(&path, &bytes).unwrap();
    let runner = proxy(&fixture, "pause", "");
    let (backend, scope) = with_runner(&fixture, runner).await;
    let service = backend.clone();
    let request = OpenHistoryRequest {
        scope,
        filter: filter(),
        page_size: Some(200),
    };
    let task = tokio::spawn(async move { service.open_history("main", request).await });
    tokio::time::timeout(Duration::from_secs(20), async {
        while !fixture.owner.root().join("entered").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let moved = fixture.path.join(".git/old-shallow");
    assert!(std::fs::write(&path, b"changed").is_err());
    assert!(std::fs::rename(&path, &moved).is_err());
    fixture.run(&["update-ref", "refs/heads/during-query", "main"], None);
    fixture.run(&["update-ref", "-d", "refs/heads/during-query"], None);
    std::fs::write(fixture.owner.root().join("release"), b"go").unwrap();
    let session = task.await.unwrap().unwrap();
    assert!(session
        .page
        .commits
        .iter()
        .flat_map(|c| &c.parents)
        .any(|p| p.boundary == ParentBoundary::Shallow));
    std::fs::rename(&path, &moved).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(
        path.is_file(),
        "no idle shallow lease may survive a completed query"
    );
}
#[tokio::test]
async fn missing_commit_metadata_and_large_message_or_encoded_response_return_structured_errors() {
    use std::io::Write;
    let fixture = Fixture::new("sha1", 8);
    let tree = fixture.oid("main^{tree}");
    let raw = format!("tree {tree}\nauthor Fixture <fixture@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\n{}", "m".repeat(65_537));
    let oid = String::from_utf8(fixture.run(
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(raw.as_bytes()),
    ))
    .unwrap()
    .trim()
    .to_owned();
    fixture.run(&["update-ref", "refs/heads/oversized", &oid], None);
    let (backend, scope) = fixture.backend().await;
    assert_eq!(
        backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope: scope.clone(),
                    filter: HistoryFilter {
                        hidden_refs: vec![],
                        solo_refs: vec!["refs/heads/oversized".into()]
                    },
                    page_size: Some(1)
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::OutputLimit
    );
    let path = fixture
        .path
        .join(".git/objects")
        .join(&oid[..2])
        .join(&oid[2..]);
    fixture.owner.require_owned_file(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope: scope.clone(),
                    filter: filter(),
                    page_size: None
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::NotFound
    );
    fixture.run(&["update-ref", "-d", "refs/heads/oversized"], None);
    let mut stream = Vec::new();
    let text = "\"".repeat(60_000);
    for i in 0..80 {
        write!(&mut stream, "commit refs/heads/escaped\nmark :{}\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\ndata {}\n{text}\nfrom {}\n\n", 1000 + i, text.len(), if i == 0 { fixture.oid("main") } else { format!(":{}", 999 + i) }).unwrap();
    }
    stream.extend_from_slice(b"done\n");
    fixture.run(&["fast-import", "--quiet"], Some(&stream));
    let before = fixture.snapshot();
    let result = backend
        .open_history(
            "main",
            OpenHistoryRequest {
                scope: scope.clone(),
                filter: HistoryFilter {
                    hidden_refs: vec![],
                    solo_refs: vec!["refs/heads/escaped".into()],
                },
                page_size: Some(100),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(result.code, ErrorCode::OutputLimit);
    assert_eq!(fixture.snapshot(), before);
    // A failed encoded-response budget must not make unreturned rows appear
    // loaded in a later search response.
    let base = fixture.oid("escaped");
    let mut stream = Vec::new();
    for i in 0..80 {
        write!(&mut stream, "commit refs/heads/escaped\nmark :{}\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\ndata 9\nTop small\nfrom {}\n\n", 2000 + i, if i == 0 { base.clone() } else { format!(":{}", 1999 + i) }).unwrap();
    }
    stream.extend_from_slice(b"done\n");
    fixture.run(&["fast-import", "--quiet"], Some(&stream));
    let session = open(
        &backend,
        &scope,
        80,
        HistoryFilter {
            hidden_refs: vec![],
            solo_refs: vec!["refs/heads/escaped".into()],
        },
    )
    .await;
    let failed_page = backend
        .load_history_page("main", page_request(&session, 80))
        .await
        .err()
        .unwrap();
    assert_eq!(failed_page.code, ErrorCode::OutputLimit);
    let row = session.page.commits.last().unwrap();
    let found = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: row.oid.clone(),
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        found.commits[0].commit.parents[0].boundary,
        ParentBoundary::OffPage
    );
}
#[cfg(windows)]
#[tokio::test]
async fn bounded_stdin_drains_stdout_concurrently_and_stops_on_timeout_cancel_output_or_broken_pipe(
) {
    let owner = support::OwnedFixture::new().unwrap();
    let executable = compile(
        owner.root(),
        r#"use std::{io::{Read,Write},time::Duration};fn main(){let args:Vec<_>=std::env::args_os().collect();let root=std::path::Path::new(&args[2]);std::fs::write(root.join("started"),b"yes").unwrap();match args[1].to_str().unwrap(){"block"=>std::thread::sleep(Duration::from_secs(60)),"exit"=>(),_=>{std::io::stdout().write_all(&vec![b'x';2*1024*1024]).unwrap();let mut input=Vec::new();std::io::stdin().read_to_end(&mut input).unwrap();print!("{}",input.len());}}}"#,
    );
    let mut runner = GitRunner::trusted(executable).unwrap();
    let input = vec![b'i'; 2 * 1024 * 1024];
    let args = |mode: &str| vec![mode.into(), owner.root().as_os_str().into()];
    let out = runner
        .output_with_input(
            None,
            &args("duplex"),
            &Cancellation::default(),
            Phase::Inspection,
            &input,
        )
        .await
        .unwrap();
    assert!(out.success);
    assert!(out.stdout.ends_with(b"2097152"));
    assert_eq!(out.stdout.len(), input.len() + 7);
    std::fs::remove_file(owner.root().join("started")).unwrap();
    assert_eq!(
        runner
            .output_with_input(
                None,
                &args("duplex"),
                &Cancellation::default(),
                Phase::Inspection,
                &vec![0; INPUT_LIMIT_BYTES + 1]
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(!owner.root().join("started").exists());
    runner.timeout = Duration::from_millis(200);
    assert_eq!(
        runner
            .output_with_input(
                None,
                &args("block"),
                &Cancellation::default(),
                Phase::Inspection,
                &input
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Timeout
    );
    runner.timeout = Duration::from_secs(10);
    let cancel = Cancellation::default();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    assert_eq!(
        runner
            .output_with_input(None, &args("block"), &cancel, Phase::Inspection, &input)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Interrupted
    );
    runner.output_limit = 1024;
    assert_eq!(
        runner
            .output_with_input(
                None,
                &args("duplex"),
                &Cancellation::default(),
                Phase::Inspection,
                &input
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::OutputLimit
    );
    runner.output_limit = 8 * 1024 * 1024;
    assert_eq!(
        runner
            .output_with_input(
                None,
                &args("exit"),
                &Cancellation::default(),
                Phase::Inspection,
                &input
            )
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::Io
    );
}
