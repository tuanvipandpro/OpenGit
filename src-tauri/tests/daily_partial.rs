#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
const FILE: &str = "-partial Ω,[literal].txt";
const BASE: &[u8] = b"keep\nold\nnear\nc1\nc2\nc3\nc4\nc5\nc6\nc7\nfar-old\n";
const WORK: &[u8] = b"keep\nnewA\nnewB\nnear\nc1\nc2\nc3\nc4\nc5\nc6\nc7\nfar-new\n";
async fn commit(
    f: &support::daily::Fixture,
    message: &str,
    amend: bool,
) -> ChangeActionOperationResult {
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    execute(
        f,
        ActionIntent::Commit {
            message: message.into(),
            amend,
        },
    )
    .await
}
#[tokio::test]
async fn amend_shallow_parent_bearing_head_refuses_before_write_and_preserves_raw_topology() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    std::fs::write(f.git.path.join(FILE), b"staged shallow amendment\n").unwrap();
    f.git.run(&["add", "--", FILE], None);
    std::fs::write(
        f.git.path.join(FILE),
        b"working shallow amendment remains\n",
    )
    .unwrap();
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let request = || PrepareRepositoryActionRequest {
        scope: f.scope.clone(),
        intent: ActionIntent::Commit {
            message: "must preserve hidden parents".into(),
            amend: true,
        },
    };
    let prepared_before_shallow = f
        .backend
        .prepare_repository_action("main", request())
        .await
        .unwrap();
    let old = f.git.oid("HEAD");
    let raw = f.git.run(&["cat-file", "commit", &old], None);
    assert!(raw
        .split(|b| *b == b'\n')
        .any(|line| line.starts_with(b"parent ")));
    std::fs::write(f.git.path.join(".git/shallow"), format!("{old}\n")).unwrap();
    assert_eq!(
        f.git.run(&["rev-parse", "--is-shallow-repository"], None),
        b"true\n"
    );
    // Revision traversal deliberately hides the raw parent at this boundary.
    assert_eq!(
        f.git
            .run(&["rev-list", "--parents", "-n", "1", "HEAD"], None),
        format!("{old}\n").as_bytes()
    );
    let state = f.worktree_state();
    let working = std::fs::read(f.git.path.join(FILE)).unwrap();
    let rejected = f.backend.prepare_repository_action("main", request()).await;
    assert!(
        rejected.is_err(),
        "shallow amend must reject before enqueue: {rejected:?}"
    );
    assert!(rejected.unwrap_err().message.contains("shallow"));
    let operation = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: prepared_before_shallow.plan_id,
            },
        )
        .unwrap();
    let result = f.wait(&operation.id).await;
    assert_ne!(
        result.operation.state,
        OperationState::Succeeded,
        "{result:?}"
    );
    assert_eq!(
        result.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(f.git.oid("HEAD"), old);
    assert_eq!(f.git.run(&["cat-file", "commit", &old], None), raw);
    assert_eq!(f.worktree_state(), state);
    assert_eq!(std::fs::read(f.git.path.join(FILE)).unwrap(), working);
    assert!(!f.git.path.join(".git/index.lock").exists());
}
#[tokio::test]
async fn amend_shallow_change_after_semantic_read_rejects_before_native_commit() {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: "must not adopt shallow boundary".into(),
                    amend: true,
                },
            },
        )
        .await
        .unwrap();
    let old = f.git.oid("HEAD");
    let raw = f.git.run(&["cat-file", "commit", &old], None);
    let state = f.worktree_state();
    let working = std::fs::read(f.git.path.join(FILE)).unwrap();
    proxy.set("amend-shallow-hold");
    let operation = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    // The child's semantic read already returned false; change the boundary before
    // Rust receives it. The frozen before-resume stamp must not adopt this state.
    std::fs::write(f.git.path.join(".git/shallow"), format!("{old}\n")).unwrap();
    proxy.release();
    let result = f.wait(&operation.id).await;
    assert_ne!(
        result.operation.state,
        OperationState::Succeeded,
        "{result:?}"
    );
    assert_eq!(
        result.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(f.git.oid("HEAD"), old);
    assert_eq!(f.git.run(&["cat-file", "commit", &old], None), raw);
    assert_eq!(f.worktree_state(), state);
    assert_eq!(std::fs::read(f.git.path.join(FILE)).unwrap(), working);
    assert!(!f.git.path.join(".git/index.lock").exists());
}
#[tokio::test]
async fn amend_verified_same_oid_noop_and_configured_ssh_signer_success_are_preserved() {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    proxy.set("amend-noop");
    let old = f.git.oid("HEAD");
    let tree = f.git.run(&["rev-parse", "HEAD^{tree}"], None);
    let r = commit(&f, "partial base", true).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(f.git.oid("HEAD"), old);
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{r:?}"
    );
    assert_eq!(
        r.result.as_ref().unwrap().commit_oid.as_deref(),
        Some(old.as_str())
    );
    assert_eq!(f.git.run(&["rev-parse", "HEAD^{tree}"], None), tree);
    proxy.set("normal");
    let key = f.git.owner.child("amend-signing-key").unwrap();
    let ssh = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join("System32/OpenSSH/ssh-keygen.exe");
    assert!(ssh.is_file());
    let mut command = std::process::Command::new(&ssh);
    command
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    assert!(command.output().unwrap().status.success());
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let allowed = f.git.owner.child("amend-allowed-signers").unwrap();
    std::fs::write(
        &allowed,
        format!("fixture@example.invalid {}", public.trim()),
    )
    .unwrap();
    f.git.run(&["config", "gpg.format", "ssh"], None);
    f.git
        .run(&["config", "gpg.ssh.program", ssh.to_str().unwrap()], None);
    f.git
        .run(&["config", "user.signingkey", &f.git.git_path(&key)], None);
    f.git.run(
        &[
            "config",
            "gpg.ssh.allowedSignersFile",
            &f.git.git_path(&allowed),
        ],
        None,
    );
    f.git.run(&["config", "commit.gpgsign", "true"], None);
    let r = commit(&f, "signed amendment", true).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_ne!(f.git.oid("HEAD"), old);
    assert_eq!(f.git.run(&["rev-parse", "HEAD^{tree}"], None), tree);
    f.git.run(&["verify-commit", "HEAD"], None);
}
#[tokio::test]
async fn amend_message_only_staged_root_merge_and_detached_preserve_expected_parent_order_and_author(
) {
    for mode in ["normal", "root", "merge", "detached"] {
        let f = support::daily::Fixture::new("sha1", 6, "files").await;
        setup(&f);
        let tree = String::from_utf8(f.git.run(&["rev-parse", "HEAD^{tree}"], None)).unwrap();
        if mode == "root" {
            let oid = String::from_utf8(
                f.git
                    .run(&["commit-tree", tree.trim()], Some(b"root before\n")),
            )
            .unwrap();
            f.git
                .run(&["update-ref", "refs/heads/main", oid.trim()], None);
        }
        if mode == "merge" {
            let base = f.git.oid("HEAD");
            let a = String::from_utf8(f.git.run(
                &["commit-tree", tree.trim(), "-p", &base],
                Some(b"parent A\n"),
            ))
            .unwrap();
            let b = String::from_utf8(f.git.run(
                &["commit-tree", tree.trim(), "-p", &base],
                Some(b"parent B\n"),
            ))
            .unwrap();
            let merge = String::from_utf8(f.git.run(
                &["commit-tree", tree.trim(), "-p", b.trim(), "-p", a.trim()],
                Some(b"merge before\n"),
            ))
            .unwrap();
            f.git
                .run(&["update-ref", "refs/heads/main", merge.trim()], None);
        }
        if mode == "detached" {
            f.git.run(&["checkout", "--detach", "HEAD"], None);
        }
        let old = f.git.oid("HEAD");
        let parents = String::from_utf8(
            f.git
                .run(&["rev-list", "--parents", "-n", "1", "HEAD"], None),
        )
        .unwrap()
        .split_whitespace()
        .skip(1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let author = f
            .git
            .run(&["show", "-s", "--format=%an%x00%ae%x00%aI", "HEAD"], None);
        f.git.run(
            &["config", "user.name", "Different Current Committer"],
            None,
        );
        let result = commit(&f, "amended message only", true).await;
        assert_eq!(
            result.operation.state,
            OperationState::Succeeded,
            "{mode}: {result:?}"
        );
        let new = f.git.oid("HEAD");
        assert_ne!(new, old);
        assert_eq!(
            f.git.run(&["rev-parse", "HEAD^{tree}"], None),
            tree.as_bytes()
        );
        assert_eq!(
            f.git
                .run(&["show", "-s", "--format=%an%x00%ae%x00%aI", "HEAD"], None),
            author
        );
        let actual = String::from_utf8(
            f.git
                .run(&["rev-list", "--parents", "-n", "1", "HEAD"], None),
        )
        .unwrap()
        .split_whitespace()
        .skip(1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert_eq!(actual, parents, "{mode}");
        assert_eq!(
            result.result.unwrap().commit_oid.as_deref(),
            Some(new.as_str())
        );
        if mode == "normal" {
            std::fs::write(f.git.path.join(FILE), b"staged amendment\n").unwrap();
            f.git.run(&["add", "--", FILE], None);
            std::fs::write(f.git.path.join(FILE), b"working amendment remains\n").unwrap();
            let result = commit(&f, "amended staged", true).await;
            assert_eq!(
                result.operation.state,
                OperationState::Succeeded,
                "{result:?}"
            );
            assert_eq!(
                f.git.run(&["show", &format!("HEAD:{FILE}")], None),
                b"staged amendment\n"
            );
            assert_eq!(
                std::fs::read(f.git.path.join(FILE)).unwrap(),
                b"working amendment remains\n"
            );
            assert_eq!(
                String::from_utf8(
                    f.git
                        .run(&["rev-list", "--parents", "-n", "1", "HEAD"], None)
                )
                .unwrap()
                .split_whitespace()
                .skip(1)
                .map(str::to_owned)
                .collect::<Vec<_>>(),
                parents
            );
        }
    }
}
#[tokio::test]
async fn amend_unborn_and_stale_head_refuse_and_hook_signer_failures_keep_actual_state() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    let old = f.git.oid("HEAD");
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let p = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: "stale amendment".into(),
                    amend: true,
                },
            },
        )
        .await
        .unwrap();
    f.git
        .run(&["commit", "--allow-empty", "-m", "external head"], None);
    let external = f.git.oid("HEAD");
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    let r = f.wait(&op.id).await;
    assert_eq!(
        r.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(f.git.oid("HEAD"), external);
    assert_ne!(old, external);
    std::fs::write(f.git.path.join(".git/hooks/pre-commit"),b"#!/bin/sh\nprintf 'hook effect\\n' > amendment-hook.txt\ngit add -- amendment-hook.txt\nexit 1\n").unwrap();
    let r = commit(&f, "hook rejected amendment", true).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial,
        "{r:?}"
    );
    assert_eq!(f.git.oid("HEAD"), external);
    assert_eq!(
        f.git.run(&["show", ":amendment-hook.txt"], None),
        b"hook effect\n"
    );
    std::fs::remove_file(f.git.path.join(".git/hooks/pre-commit")).unwrap();
    let signer = f.git.owner.child("amend signer fails.cmd").unwrap();
    std::fs::write(&signer, b"@exit /b 1\r\n").unwrap();
    f.git.run(&["config", "commit.gpgsign", "true"], None);
    f.git
        .run(&["config", "gpg.program", &f.git.git_path(&signer)], None);
    let r = commit(&f, "signing rejected amendment", true).await;
    assert_ne!(r.operation.state, OperationState::Succeeded);
    assert_eq!(
        r.result.unwrap().failure_reason,
        Some(ChangeFailureReason::Signing)
    );
    assert_eq!(f.git.oid("HEAD"), external);
    f.git.run(&["switch", "--orphan", "unborn"], None);
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: "must not amend unborn".into(),
                    amend: true
                }
            }
        )
        .await
        .is_err());
}
#[tokio::test]
async fn stale_foreign_duplicate_direction_and_duplicate_file_selections_never_write_or_retarget() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    std::fs::write(f.git.path.join(FILE), WORK).unwrap();
    let d = diff(&f, FILE, ChangeSide::Unstaged).await;
    let id = d.hunks[0]
        .rows
        .iter()
        .find_map(|r| r.line_id.clone())
        .unwrap();
    for input in [
        ChangeSelectionInput::Lines { line_ids: vec![] },
        ChangeSelectionInput::Lines {
            line_ids: vec![id.clone(), id.clone()],
        },
        ChangeSelectionInput::Lines {
            line_ids: vec!["line-foreign-0".into()],
        },
        ChangeSelectionInput::Hunks {
            hunk_ids: vec!["foreign-hunk".into()],
        },
    ] {
        assert!(selection(&f, &d, input).await.is_err());
    }
    let a = selection(&f, &d, lines(&d, &["old"])).await.unwrap();
    let b = selection(&f, &d, lines(&d, &["newA"])).await.unwrap();
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::UnstageSelection {
                    selection_handles: vec![a.selection_handle.clone()]
                }
            }
        )
        .await
        .is_err());
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StageSelection {
                    selection_handles: vec![a.selection_handle, b.selection_handle]
                }
            }
        )
        .await
        .is_err());
    let before = f.git.run(&["ls-files", "--stage", "-z"], None);
    std::fs::write(f.git.path.join(FILE), b"same status externally changed\n").unwrap();
    assert!(
        selection(&f, &d, ChangeSelectionInput::Lines { line_ids: vec![id] })
            .await
            .is_err()
    );
    assert_eq!(f.git.run(&["ls-files", "--stage", "-z"], None), before);
    assert!(f
        .backend
        .scheduler
        .snapshot()
        .unwrap()
        .operations
        .is_empty());
}
#[tokio::test]
async fn partial_fallbacks_are_explicit_and_attributes_or_closed_scope_never_remint_old_selection()
{
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    let large = (0..1100).map(|i| format!("left{i}\n")).collect::<String>();
    std::fs::write(f.git.path.join("complex.txt"), large).unwrap();
    std::fs::write(f.git.path.join("mode.txt"), b"mode\n").unwrap();
    std::fs::write(f.git.path.join("old-name.txt"), b"rename\n").unwrap();
    f.git.run(&["add", "--all"], None);
    f.git.run(&["commit", "-m", "fallback base"], None);
    std::fs::write(
        f.git.path.join("complex.txt"),
        (0..1100).map(|i| format!("right{i}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(f.git.path.join("binary.bin"), [0, 255, 1]).unwrap();
    std::fs::write(f.git.path.join("encoding.txt"), [255, 254]).unwrap();
    std::fs::write(f.git.path.join("crlf.txt"), b"one\r\ntwo\r\n").unwrap();
    std::fs::write(f.git.path.join("empty.txt"), b"").unwrap();
    std::fs::write(f.git.path.join("attr.txt"), b"attributes\n").unwrap();
    std::fs::write(f.git.path.join(".gitattributes"), b"attr.txt diff=custom\n").unwrap();
    f.git
        .run(&["update-index", "--chmod=+x", "--", "mode.txt"], None);
    f.git
        .run(&["mv", "--", "old-name.txt", "new-name.txt"], None);
    for (name, side, expected) in [
        (
            "binary.bin",
            ChangeSide::Unstaged,
            PartialEligibility::Binary,
        ),
        (
            "encoding.txt",
            ChangeSide::Unstaged,
            PartialEligibility::Encoding,
        ),
        (
            "crlf.txt",
            ChangeSide::Unstaged,
            PartialEligibility::LineEndings,
        ),
        (
            "empty.txt",
            ChangeSide::Unstaged,
            PartialEligibility::FilePresenceOnly,
        ),
        (
            "attr.txt",
            ChangeSide::Unstaged,
            PartialEligibility::Attributes,
        ),
        (
            "complex.txt",
            ChangeSide::Unstaged,
            PartialEligibility::ComplexityLimit,
        ),
        ("mode.txt", ChangeSide::Staged, PartialEligibility::Mode),
        (
            "new-name.txt",
            ChangeSide::Staged,
            PartialEligibility::Rename,
        ),
    ] {
        let d = diff(&f, name, side).await;
        assert_eq!(d.partial_eligibility, expected, "{name}: {d:?}");
        assert!(d.hunks.is_empty());
        assert!(selection(
            &f,
            &d,
            ChangeSelectionInput::Lines {
                line_ids: vec![format!("line-{}-0", d.diff_id)]
            }
        )
        .await
        .is_err());
    }
    std::fs::write(f.git.path.join(FILE), WORK).unwrap();
    std::fs::write(f.git.path.join(".gitattributes"), b"*.txt -text\n").unwrap();
    f.scope.generation = "9".into();
    let d = diff(&f, FILE, ChangeSide::Unstaged).await;
    let ids = lines(&d, &["old", "newA"]);
    std::fs::write(f.git.path.join(".gitattributes"), b"*.txt text eol=lf\n").unwrap();
    assert!(selection(&f, &d, ids.clone()).await.is_err());
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    f.scope.generation = "1".into();
    let fresh = diff(&f, FILE, ChangeSide::Unstaged).await;
    let fresh_selection = selection(&f, &fresh, lines(&fresh, &["old", "newA"]))
        .await
        .unwrap();
    let prepared = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::StageSelection {
                    selection_handles: vec![fresh_selection.selection_handle],
                },
            },
        )
        .await
        .unwrap();
    assert!(f
        .backend
        .resolve_change_selection(
            "main",
            ResolveChangeSelectionRequest {
                scope: ActionScope {
                    generation: "9".into(),
                    ..f.scope.clone()
                },
                change_handle: d.change_handle,
                diff_id: d.diff_id,
                selection: ids
            }
        )
        .await
        .is_err());
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: prepared.plan_id,
            },
        )
        .unwrap();
    assert_eq!(
        f.wait(&op.id).await.operation.state,
        OperationState::Succeeded
    );
}
fn setup(f: &support::daily::Fixture) {
    f.git.run(&["config", "core.autocrlf", "false"], None);
    f.git.run(&["config", "core.eol", "lf"], None);
    f.git.run(&["config", "user.name", "Fixture"], None);
    f.git
        .run(&["config", "user.email", "fixture@example.invalid"], None);
    std::fs::write(f.git.path.join(FILE), BASE).unwrap();
    std::fs::write(f.git.path.join("unrelated.txt"), b"base unrelated\n").unwrap();
    f.git.run(&["add", "--all"], None);
    f.git.run(&["commit", "-m", "partial base"], None);
}
async fn diff(f: &support::daily::Fixture, name: &str, side: ChangeSide) -> ChangeDiff {
    let catalog = f
        .backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let entry = catalog
        .entries
        .iter()
        .find(|e| e.path_display == name)
        .unwrap();
    let r = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: catalog.catalog_id,
                targets: vec![ChangeSelection {
                    entry_id: entry.entry_id.clone(),
                    side,
                }],
            },
        )
        .await
        .unwrap();
    f.backend
        .get_change_diff(
            "main",
            GetChangeDiffRequest {
                scope: f.scope.clone(),
                change_handle: r.targets[0].change_handle.clone(),
            },
        )
        .await
        .unwrap()
}
fn lines(d: &ChangeDiff, names: &[&str]) -> ChangeSelectionInput {
    ChangeSelectionInput::Lines {
        line_ids: names
            .iter()
            .map(|name| {
                d.hunks
                    .iter()
                    .flat_map(|h| &h.rows)
                    .find(|r| r.text == *name && r.line_id.is_some())
                    .unwrap()
                    .line_id
                    .clone()
                    .unwrap()
            })
            .collect(),
    }
}
async fn selection(
    f: &support::daily::Fixture,
    d: &ChangeDiff,
    selection: ChangeSelectionInput,
) -> Result<ResolvedChangeSelection, AppError> {
    f.backend
        .resolve_change_selection(
            "main",
            ResolveChangeSelectionRequest {
                scope: f.scope.clone(),
                change_handle: d.change_handle.clone(),
                diff_id: d.diff_id.clone(),
                selection,
            },
        )
        .await
}
async fn execute(f: &support::daily::Fixture, intent: ActionIntent) -> ChangeActionOperationResult {
    let p = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent,
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: p.plan_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        f.backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest { plan_id: p.plan_id }
            )
            .unwrap()
            .id,
        op.id
    );
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let r = f
                .backend
                .get_change_action_outcome(
                    "main",
                    GetActionOutcomeRequest {
                        operation_id: op.id.clone(),
                    },
                )
                .unwrap();
            if opengit_lib::operations::state::terminal(&r.operation.state) {
                break r;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
fn check(
    f: &support::daily::Fixture,
    r: &ChangeActionOperationResult,
    expected: &[u8],
    unrelated: &[u8],
) {
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(f.git.run(&["show", &format!(":{FILE}")], None), expected);
    assert_eq!(
        f.git
            .run(&["ls-files", "--stage", "--", "unrelated.txt"], None),
        unrelated
    );
    assert_eq!(std::fs::read(f.git.path.join(FILE)).unwrap(), WORK);
    assert_eq!(
        std::fs::read(f.git.path.join("unrelated.txt")).unwrap(),
        b"working unrelated\n"
    );
    assert!(f
        .git
        .run(&["ls-files", "--stage", "--", FILE], None)
        .starts_with(b"100644 "));
    let tree = String::from_utf8(f.git.run(&["write-tree"], None)).unwrap();
    assert_eq!(
        r.result.as_ref().unwrap().tree_oid.as_deref(),
        Some(tree.trim())
    );
}
#[tokio::test]
async fn exact_mixed_line_subset_partial_unstage_and_full_hunk_preserve_unselected_bytes() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup(&f);
    std::fs::write(f.git.path.join(FILE), WORK).unwrap();
    std::fs::write(f.git.path.join("unrelated.txt"), b"staged unrelated\n").unwrap();
    f.git.run(&["add", "--", "unrelated.txt"], None);
    std::fs::write(f.git.path.join("unrelated.txt"), b"working unrelated\n").unwrap();
    let unrelated = f
        .git
        .run(&["ls-files", "--stage", "--", "unrelated.txt"], None);
    let d = diff(&f, FILE, ChangeSide::Unstaged).await;
    assert_eq!(d.partial_eligibility, PartialEligibility::Eligible, "{d:?}");
    assert_eq!(d.hunks.len(), 2);
    let selected = selection(&f, &d, lines(&d, &["old", "newA"]))
        .await
        .unwrap();
    let r = execute(
        &f,
        ActionIntent::StageSelection {
            selection_handles: vec![selected.selection_handle],
        },
    )
    .await;
    check(
        &f,
        &r,
        b"keep\nnewA\nnear\nc1\nc2\nc3\nc4\nc5\nc6\nc7\nfar-old\n",
        &unrelated,
    );
    let staged = diff(&f, FILE, ChangeSide::Staged).await;
    let selected = selection(&f, &staged, lines(&staged, &["old", "newA"]))
        .await
        .unwrap();
    let r = execute(
        &f,
        ActionIntent::UnstageSelection {
            selection_handles: vec![selected.selection_handle],
        },
    )
    .await;
    check(&f, &r, BASE, &unrelated);
    let d = diff(&f, FILE, ChangeSide::Unstaged).await;
    let selected = selection(
        &f,
        &d,
        ChangeSelectionInput::Hunks {
            hunk_ids: vec![d.hunks[0].hunk_id.clone()],
        },
    )
    .await
    .unwrap();
    let r = execute(
        &f,
        ActionIntent::StageSelection {
            selection_handles: vec![selected.selection_handle],
        },
    )
    .await;
    check(
        &f,
        &r,
        b"keep\nnewA\nnewB\nnear\nc1\nc2\nc3\nc4\nc5\nc6\nc7\nfar-old\n",
        &unrelated,
    );
}
#[tokio::test]
async fn partial_new_deleted_and_no_lf_endpoints_preserve_exact_presence_with_sha256() {
    let f = support::daily::Fixture::new("sha256", 6, "reftable").await;
    setup(&f);
    std::fs::write(f.git.path.join("new.txt"), b"one\ntwo").unwrap();
    let d = diff(&f, "new.txt", ChangeSide::Unstaged).await;
    assert_eq!(d.partial_eligibility, PartialEligibility::Eligible, "{d:?}");
    let s = selection(&f, &d, lines(&d, &["one"])).await.unwrap();
    let r = execute(
        &f,
        ActionIntent::StageSelection {
            selection_handles: vec![s.selection_handle],
        },
    )
    .await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(f.git.run(&["show", ":new.txt"], None), b"one\n");
    assert_eq!(
        std::fs::read(f.git.path.join("new.txt")).unwrap(),
        b"one\ntwo"
    );
    let d = diff(&f, "new.txt", ChangeSide::Staged).await;
    let s = selection(
        &f,
        &d,
        ChangeSelectionInput::Hunks {
            hunk_ids: vec![d.hunks[0].hunk_id.clone()],
        },
    )
    .await
    .unwrap();
    let r = execute(
        &f,
        ActionIntent::UnstageSelection {
            selection_handles: vec![s.selection_handle],
        },
    )
    .await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert!(f.git.run(&["ls-files", "--", "new.txt"], None).is_empty());
    std::fs::remove_file(f.git.path.join(FILE)).unwrap();
    let d = diff(&f, FILE, ChangeSide::Unstaged).await;
    let s = selection(&f, &d, lines(&d, &["old"])).await.unwrap();
    let r = execute(
        &f,
        ActionIntent::StageSelection {
            selection_handles: vec![s.selection_handle],
        },
    )
    .await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(
        f.git.run(&["show", &format!(":{FILE}")], None),
        b"keep\nnear\nc1\nc2\nc3\nc4\nc5\nc6\nc7\nfar-old\n"
    );
    assert!(!f.git.path.join(FILE).exists());
}
