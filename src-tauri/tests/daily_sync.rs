#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
#[tokio::test]
async fn pull_ignored_collision_refuses_integration_and_post_merge_config_effects_are_not_erased() {
    for hook in [false, true] {
        let f = support::daily::Fixture::new("sha1", 6, "files").await;
        let old = f.git.oid("HEAD");
        if hook {
            std::fs::write(
                f.git.path.join(".git/hooks/post-merge"),
                b"#!/bin/sh\ngit config opengit.fixture post-merge-preserved\n",
            )
            .unwrap();
            f.advance("refs/heads/main");
        } else {
            let peer = f.git.owner.child("peer").unwrap();
            f.git.run(
                &[
                    "clone",
                    "--",
                    &f.git.git_path(&f.remote),
                    &f.git.git_path(&peer),
                ],
                None,
            );
            std::fs::write(peer.join("collision.txt"), b"remote tracked\n").unwrap();
            f.git.run_at(&peer, &["add", "--", "collision.txt"], None);
            f.git
                .run_at(&peer, &["commit", "-m", "remote collision"], None);
            f.git.run_at(&peer, &["push", "origin", "main"], None);
            std::fs::write(f.git.path.join(".git/info/exclude"), b"collision.txt\n").unwrap();
            std::fs::write(f.git.path.join("collision.txt"), b"keep ignored local\n").unwrap();
        }
        let p = pull_plan(&f).await;
        let r = execute(&f, p).await;
        assert_eq!(
            r.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Partial,
            "{r:?}"
        );
        let result = r.result.unwrap();
        assert_eq!(result.steps[0].effects, ActionEffects::Applied);
        if hook {
            assert_ne!(f.git.oid("HEAD"), old);
            assert!(result.integrated_oid.is_some());
            assert_eq!(result.steps[1].effects, ActionEffects::Applied);
            assert_eq!(
                result.steps[1].failure_reason,
                Some(SyncFailureReason::Stale)
            );
            assert_eq!(
                f.git.run(&["config", "--get", "opengit.fixture"], None),
                b"post-merge-preserved\n"
            );
        } else {
            assert_eq!(f.git.oid("HEAD"), old);
            assert_eq!(
                std::fs::read(f.git.path.join("collision.txt")).unwrap(),
                b"keep ignored local\n"
            );
            assert_eq!(result.steps[1].effects, ActionEffects::NotApplied);
            assert!(f
                .git
                .run(&["ls-files", "--", "collision.txt"], None)
                .is_empty());
        }
    }
}
#[tokio::test]
async fn pull_unborn_detached_bare_shallow_are_explicit_and_linked_worktree_updates_only_its_branch(
) {
    for mode in ["unborn", "detached", "bare", "shallow"] {
        let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
        match mode {
            "unborn" => {
                f.git.run(&["switch", "--orphan", "unborn"], None);
            }
            "detached" => {
                f.git.run(&["checkout", "--detach", "HEAD"], None);
            }
            "bare" => {
                let s = f
                    .backend
                    .register_repository("main", &f.remote)
                    .await
                    .unwrap()
                    .active_selection
                    .unwrap();
                f.scope = ActionScope {
                    repo_id: s.repo_id,
                    worktree_id: s.worktree_id,
                    generation: "1".into(),
                };
            }
            "shallow" => {
                std::fs::write(
                    f.git.path.join(".git/shallow"),
                    format!("{}\n", f.git.oid("HEAD~2")),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let handle = if mode == "bare" {
            f.backend
                .get_repository_actions(
                    "main",
                    GetRepositoryActionsRequest {
                        scope: f.scope.clone(),
                    },
                )
                .await
                .unwrap();
            "a".repeat(64)
        } else {
            local_handle(&f, "refs/remotes/origin/main").await
        };
        assert!(
            f.backend
                .prepare_repository_action(
                    "main",
                    PrepareRepositoryActionRequest {
                        scope: f.scope.clone(),
                        intent: ActionIntent::Pull {
                            upstream_handle: handle
                        }
                    }
                )
                .await
                .is_err(),
            "{mode}"
        );
        assert!(f
            .backend
            .scheduler
            .snapshot()
            .unwrap()
            .operations
            .is_empty());
    }
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    let original = f.git.oid("HEAD");
    let original_index = std::fs::read(f.git.path.join(".git/index")).ok();
    let linked = f.git.owner.child("linked pull Ω").unwrap();
    f.git.run(
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            &f.git.git_path(&linked),
            "main",
        ],
        None,
    );
    f.git
        .run(&["config", "branch.linked.remote", "origin"], None);
    f.git
        .run(&["config", "branch.linked.merge", "refs/heads/main"], None);
    let s = f
        .backend
        .register_repository("main", &linked)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    f.scope = ActionScope {
        repo_id: s.repo_id,
        worktree_id: s.worktree_id,
        generation: "1".into(),
    };
    let next = f.advance("refs/heads/main");
    let p = pull_plan(&f).await;
    let r = execute(&f, p).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(f.git.oid("refs/heads/linked"), next);
    assert_eq!(f.git.oid("refs/heads/main"), original);
    assert_eq!(f.git.oid("HEAD"), original);
    assert_eq!(
        std::fs::read(f.git.path.join(".git/index")).ok(),
        original_index
    );
}
#[tokio::test]
#[ignore = "requires owned writable LiveRemotes launcher; no provider/production credentials"]
async fn owned_https_and_ssh_push_and_pull_preserve_url_trust_helpers_and_exact_server_objects() {
    let descriptor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(std::env::var_os("OPENGIT_S4_SYNC_FIXTURE_DESCRIPTOR").unwrap())
            .unwrap(),
    )
    .unwrap();
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let server = std::path::PathBuf::from(descriptor["repository"].as_str().unwrap());
    assert!(
        server.is_absolute()
            && server
                .components()
                .any(|p| p.as_os_str().to_string_lossy().starts_with("opengit-live-"))
    );
    let helper_source = f.git.owner.child("sync credential helper.rs").unwrap();
    let helper = f.git.owner.child("sync helper Ω = '.exe").unwrap();
    std::fs::write(&helper_source,r#"fn main(){use std::io::Read;let mut input=String::new();std::io::stdin().read_to_string(&mut input).unwrap();if std::env::args().nth(1).as_deref()==Some("get"){println!("username=fixture\npassword={}\n",std::env::var("OPENGIT_S4_SYNC_SYNTHETIC_PASSWORD").unwrap());}}"#).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&helper_source)
        .args([
            "--crate-name",
            "sync_credential_helper",
            "--edition=2021",
            "-o"
        ])
        .arg(&helper)
        .status()
        .unwrap()
        .success());
    let https = descriptor["https"].as_str().unwrap();
    let ssh = descriptor["ssh"].as_str().unwrap();
    f.git
        .run(&["config", "--add", "credential.helper", ""], None);
    f.git.run(
        &[
            "config",
            "--add",
            "credential.helper",
            &format!("!\"{}\"", f.git.git_path(&helper)),
        ],
        None,
    );
    f.git.run(
        &[
            "config",
            &format!("http.{https}.sslCAInfo"),
            descriptor["ca"].as_str().unwrap(),
        ],
        None,
    );
    f.git.run(
        &[
            "config",
            &format!("http.{https}.schannelUseSSLCAInfo"),
            "true",
        ],
        None,
    );
    let runner = opengit_lib::git::runner::GitRunner::discover().unwrap();
    for (transport, url) in [("https", https), ("ssh", ssh)] {
        f.git.run(&["remote", "set-url", "origin", url], None);
        let destination = format!("owned-{transport}");
        let plan = push_plan(&f, &destination).await;
        let r = execute(&f, plan).await;
        assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
        let source = f.git.oid("HEAD");
        let server_ref = format!("refs/heads/{destination}");
        let raw = runner
            .run(
                Some(&server),
                &opengit_lib::git::runner::argv(&["rev-parse", &server_ref]),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        assert_eq!(String::from_utf8(raw).unwrap().trim(), source);
        assert_eq!(
            runner
                .run(
                    Some(&server),
                    &opengit_lib::git::runner::argv(&["cat-file", "-t", &source]),
                    &Default::default(),
                    Phase::Inspection
                )
                .await
                .unwrap(),
            b"commit\n"
        );
        let (_, fetch) = f.fetch(&["origin"], false).await;
        assert_eq!(
            fetch.operation.state,
            OperationState::Succeeded,
            "{fetch:?}"
        );
        let tracking = format!("origin/{destination}");
        f.git.run(
            &["branch", &format!("--set-upstream-to={tracking}"), "main"],
            None,
        );
        let tree = String::from_utf8(
            runner
                .run(
                    Some(&server),
                    &opengit_lib::git::runner::argv(&["rev-parse", &format!("{source}^{{tree}}")]),
                    &Default::default(),
                    Phase::Inspection,
                )
                .await
                .unwrap(),
        )
        .unwrap();
        let out = runner
            .output_with_input(
                Some(&server),
                &opengit_lib::git::runner::argv(&[
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit-tree",
                    tree.trim(),
                    "-p",
                    &source,
                ]),
                &Default::default(),
                Phase::Inspection,
                b"Owned upstream next\n",
            )
            .await
            .unwrap();
        assert!(out.success);
        let next = String::from_utf8(out.stdout).unwrap().trim().to_owned();
        runner
            .run(
                Some(&server),
                &opengit_lib::git::runner::argv(&["update-ref", &server_ref, &next, &source]),
                &Default::default(),
                Phase::Inspection,
            )
            .await
            .unwrap();
        let upstream_handle = local_handle(&f, &format!("refs/remotes/origin/{destination}")).await;
        let plan = f
            .backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: f.scope.clone(),
                    intent: ActionIntent::Pull { upstream_handle },
                },
            )
            .await
            .unwrap();
        let result = execute(&f, plan).await;
        assert_eq!(
            result.operation.state,
            OperationState::Succeeded,
            "{result:?}"
        );
        assert_eq!(f.git.oid("HEAD"), next);
        assert_eq!(
            f.git.oid(&format!("refs/remotes/origin/{destination}")),
            next
        );
    }
    let secret = std::env::var("OPENGIT_S4_SYNC_SYNTHETIC_PASSWORD").unwrap();
    let c = f.backend.registry.database.lock().unwrap();
    let text = c
        .prepare("SELECT record FROM repository_actions")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("");
    assert!(!text.contains(&secret));
    let rows = c
        .prepare("SELECT record FROM publication_observations")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("");
    assert!(!rows.contains(&secret));
    assert!(!rows.contains(https));
    assert!(!rows.contains(ssh));
}
#[tokio::test]
async fn push_lost_ack_and_cancel_after_server_apply_remain_unknown_without_replay_or_rollback() {
    for cancel in [false, true] {
        let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
        let proxy = support::daily::Proxy::new(&f.git);
        f.rebind(proxy.runner.clone(), None).await;
        f.git
            .run(&["commit", "--allow-empty", "-m", "lost ACK source"], None);
        let source = f.git.oid("HEAD");
        let p = push_plan(&f, "main").await;
        proxy.set(if cancel {
            "push-applied-hold"
        } else {
            "push-lost-ack"
        });
        let op = f
            .backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: p.plan_id.clone(),
                },
            )
            .unwrap();
        if cancel {
            proxy.wait_ready().await;
            f.backend
                .cancel_operation(
                    "main",
                    CancelOperationRequest {
                        operation_id: op.id.clone(),
                    },
                )
                .unwrap();
        }
        let r = wait_sync(&f, &op.id).await;
        assert_eq!(
            r.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Unknown,
            "{r:?}"
        );
        assert_ne!(r.operation.state, OperationState::Cancelled);
        assert_eq!(
            String::from_utf8(
                f.git
                    .run_at(&f.remote, &["rev-parse", "refs/heads/main"], None)
            )
            .unwrap()
            .trim(),
            source
        );
        assert!(r.result.unwrap().publication.is_none());
        assert_eq!(
            std::fs::read_to_string(&proxy.fetched)
                .unwrap()
                .lines()
                .count(),
            1
        );
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
        assert_eq!(
            std::fs::read_to_string(&proxy.fetched)
                .unwrap()
                .lines()
                .count(),
            1
        );
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id).unwrap();
        assert_eq!(
            record.outcome.as_ref().unwrap().effects,
            ActionEffects::Unknown
        );
    }
}
#[tokio::test]
async fn push_database_failure_after_ack_never_claims_not_applied_and_restart_keeps_unknown() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    f.git.run(
        &["commit", "--allow-empty", "-m", "ACK before DB fault"],
        None,
    );
    let source = f.git.oid("HEAD");
    let plan = push_plan(&f, "main").await;
    f.backend.registry.database.lock().unwrap().execute_batch("CREATE TRIGGER refuse_publication BEFORE INSERT ON publication_observations BEGIN SELECT RAISE(ABORT,'synthetic ACK fault'); END;").unwrap();
    let r = execute(&f, plan).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Unknown,
        "{r:?}"
    );
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/heads/main"], None)
        )
        .unwrap()
        .trim(),
        source
    );
    let path: String = f
        .backend
        .registry
        .database
        .lock()
        .unwrap()
        .query_row("PRAGMA database_list", [], |r| r.get(2))
        .unwrap();
    let id = r.operation.id.clone();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while std::sync::Arc::strong_count(&f.backend) > 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let support::daily::Fixture { backend, git, .. } = f;
    drop(backend);
    let restarted = opengit_lib::backend::Backend::with_git(
        std::path::Path::new(&path).parent().unwrap(),
        git.git.clone(),
    )
    .unwrap();
    restarted.get_workspace("main").unwrap();
    let after = restarted
        .get_sync_action_outcome("main", GetActionOutcomeRequest { operation_id: id })
        .unwrap();
    assert_eq!(
        after.operation.outcome.unwrap().effects,
        ActionEffects::Unknown
    );
    assert!(after.result.unwrap().publication.is_none());
}
#[tokio::test]
async fn push_split_endpoint_and_url_rewrite_pin_do_not_follow_extra_tags_or_touch_fetch_server() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let push = f.git.owner.child("push endpoint Ω = '.git").unwrap();
    let wrong = f.git.owner.child("wrong rewrite endpoint.git").unwrap();
    for path in [&push, &wrong] {
        f.git.run(
            &[
                "clone",
                "--bare",
                "--",
                &f.git.git_path(&f.remote),
                &f.git.git_path(path),
            ],
            None,
        );
    }
    let fetch_before = f.git.run_at(&f.remote, &["show-ref"], None);
    let wrong_before = f.git.run_at(&wrong, &["show-ref"], None);
    f.git.run(
        &[
            "config",
            &format!("url.{}.pushInsteadOf", f.git.git_path(&push)),
            &f.git.git_path(&f.remote),
        ],
        None,
    );
    f.git.run(
        &[
            "config",
            &format!("url.{}.insteadOf", f.git.git_path(&wrong)),
            &f.git.git_path(&push),
        ],
        None,
    );
    f.git.run(&["config", "push.followTags", "true"], None);
    f.git.run(
        &["commit", "--allow-empty", "-m", "split endpoint commit"],
        None,
    );
    f.git.run(
        &[
            "tag",
            "-a",
            "must-not-follow",
            "-m",
            "not requested",
            "HEAD",
        ],
        None,
    );
    let source = f.git.oid("HEAD");
    let p = push_plan(&f, "main").await;
    let r = execute(&f, p).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(
        String::from_utf8(f.git.run_at(&push, &["rev-parse", "refs/heads/main"], None))
            .unwrap()
            .trim(),
        source
    );
    assert_eq!(f.git.run_at(&f.remote, &["show-ref"], None), fetch_before);
    assert_eq!(f.git.run_at(&wrong, &["show-ref"], None), wrong_before);
    assert!(f
        .git
        .run_at(
            &push,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/tags/must-not-follow"
            ],
            None
        )
        .is_empty());
}
#[tokio::test]
async fn push_native_ff_rejects_divergence_but_dated_preview_is_not_an_old_oid_lease() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    f.git
        .run(&["commit", "--allow-empty", "-m", "local first"], None);
    let first = f.git.oid("HEAD");
    f.git
        .run(&["commit", "--allow-empty", "-m", "local second"], None);
    let source = f.git.oid("HEAD");
    let p = push_plan(&f, "main").await;
    f.git.run_at(
        &f.remote,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            &f.git.git_path(&f.git.path),
            &format!("{first}:refs/heads/main"),
        ],
        None,
    );
    let r = execute(&f, p).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/heads/main"], None)
        )
        .unwrap()
        .trim(),
        source
    );
    f.git
        .run(&["commit", "--allow-empty", "-m", "local sibling"], None);
    let local = f.git.oid("HEAD");
    let p = push_plan(&f, "main").await;
    let remote = f.advance("refs/heads/main");
    let rejected = execute(&f, p).await;
    assert_eq!(
        rejected.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{rejected:?}"
    );
    assert_eq!(
        rejected.result.unwrap().steps[0].failure_reason,
        Some(SyncFailureReason::Rejected)
    );
    assert_eq!(f.git.oid("HEAD"), local);
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/heads/main"], None)
        )
        .unwrap()
        .trim(),
        remote
    );
}
#[tokio::test]
async fn push_changed_source_or_config_and_unavailable_upstream_setup_refuse_before_remote_write() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    f.git
        .run(&["commit", "--allow-empty", "-m", "local pending"], None);
    let server = f.git.run_at(&f.remote, &["show-ref"], None);
    let p = push_plan(&f, "main").await;
    f.git.run(&["config", "core.filemode", "true"], None);
    let r = execute(&f, p).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(f.git.run_at(&f.remote, &["show-ref"], None), server);
    let p = push_plan(&f, "main").await;
    f.git.run(
        &["commit", "--allow-empty", "-m", "external source changed"],
        None,
    );
    let r = execute(&f, p).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(f.git.run_at(&f.remote, &["show-ref"], None), server);
    let (remote_handle, snapshot) = read_push(&f).await;
    let source_handle = local_handle(&f, "refs/heads/main").await;
    let entry = snapshot
        .refs
        .iter()
        .find(|e| e.name == "refs/heads/main")
        .unwrap();
    let destination = f
        .backend
        .resolve_remote_target(
            "main",
            ResolveRemoteTargetRequest {
                scope: f.scope.clone(),
                snapshot_id: snapshot.snapshot_id.clone(),
                source: RemoteTargetSource::Existing {
                    entry_id: entry.entry_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        f.backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: f.scope.clone(),
                    intent: ActionIntent::Push {
                        remote_handle,
                        source_handle,
                        destination_handle: destination.handle,
                        set_upstream: true
                    }
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedAction
    );
    assert_eq!(f.git.run_at(&f.remote, &["show-ref"], None), server);
}
async fn pull_plan(f: &support::daily::Fixture) -> PreparedAction {
    let handle = local_handle(f, "refs/remotes/origin/main").await;
    f.backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Pull {
                    upstream_handle: handle,
                },
            },
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn pull_fetches_only_configured_upstream_then_pinned_ff_without_fetch_head_tags_or_working_loss(
) {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 6, storage).await;
        std::fs::write(f.git.path.join(".git/FETCH_HEAD"), b"keep fetch head\n").unwrap();
        let before = f.git.oid("HEAD");
        let fetched = f.advance("refs/heads/main");
        f.git
            .run_at(&f.remote, &["tag", "not-pulled", &fetched], None);
        let plan = pull_plan(&f).await;
        let r = execute(&f, plan).await;
        assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
        assert_eq!(
            r.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Applied
        );
        assert_ne!(before, fetched);
        assert_eq!(f.git.oid("HEAD"), fetched);
        assert_eq!(f.git.oid("refs/remotes/origin/main"), fetched);
        assert_eq!(
            r.result.as_ref().unwrap().fetched_oid.as_deref(),
            Some(fetched.as_str())
        );
        assert_eq!(
            r.result.as_ref().unwrap().integrated_oid.as_deref(),
            Some(fetched.as_str())
        );
        assert_eq!(r.result.as_ref().unwrap().steps.len(), 2);
        assert_eq!(
            std::fs::read(f.git.path.join(".git/FETCH_HEAD")).unwrap(),
            b"keep fetch head\n"
        );
        assert!(f
            .git
            .run(
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/tags/not-pulled"
                ],
                None
            )
            .is_empty());
        assert!(f
            .git
            .run(&["status", "--porcelain=v2", "-z"], None)
            .is_empty());
        let tree = f.git.run(&["rev-parse", "HEAD^{tree}"], None);
        assert_eq!(f.git.run(&["write-tree"], None), tree);
    }
}
#[tokio::test]
async fn pull_dirty_refuses_before_network_and_diverged_after_fetch_remains_partial() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let handle = local_handle(&f, "refs/remotes/origin/main").await;
    std::fs::write(f.git.path.join("dirty.txt"), b"keep dirty\n").unwrap();
    assert!(f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Pull {
                    upstream_handle: handle
                }
            }
        )
        .await
        .is_err());
    assert!(f
        .backend
        .scheduler
        .snapshot()
        .unwrap()
        .operations
        .is_empty());
    std::fs::remove_file(f.git.path.join("dirty.txt")).unwrap();
    f.git
        .run(&["commit", "--allow-empty", "-m", "local divergence"], None);
    let local = f.git.oid("HEAD");
    let remote = f.advance("refs/heads/main");
    let p = pull_plan(&f).await;
    let r = execute(&f, p).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial,
        "{r:?}"
    );
    assert_eq!(f.git.oid("HEAD"), local);
    assert_eq!(f.git.oid("refs/remotes/origin/main"), remote);
    let result = r.result.unwrap();
    assert_eq!(result.steps[0].effects, ActionEffects::Applied);
    assert_eq!(
        result.steps[1].failure_reason,
        Some(SyncFailureReason::Diverged)
    );
    assert!(result.integrated_oid.is_none());
}
async fn tag_plan(
    f: &support::daily::Fixture,
    delete: bool,
    name: &str,
) -> Result<PreparedAction, AppError> {
    let (remote_handle, snapshot) = read_push(f).await;
    let existing = snapshot
        .refs
        .iter()
        .find(|r| r.name == format!("refs/tags/{name}"));
    let src = if let Some(e) = existing {
        RemoteTargetSource::Existing {
            entry_id: e.entry_id.clone(),
        }
    } else {
        RemoteTargetSource::New {
            namespace: RemoteRefNamespace::Tag,
            name: name.into(),
        }
    };
    let target = f
        .backend
        .resolve_remote_target(
            "main",
            ResolveRemoteTargetRequest {
                scope: f.scope.clone(),
                snapshot_id: snapshot.snapshot_id,
                source: src,
            },
        )
        .await
        .unwrap();
    let intent = if delete {
        ActionIntent::RemoteTagDelete {
            remote_handle,
            tag_handle: target.handle,
        }
    } else {
        let tag_handle = local_handle(f, "refs/tags/release").await;
        ActionIntent::RemoteTagPush {
            remote_handle,
            tag_handle,
            destination_handle: target.handle,
        }
    };
    f.backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent,
            },
        )
        .await
}
#[tokio::test]
async fn annotated_tag_push_pins_raw_object_and_stale_conditional_deletion_never_deletes_replacement(
) {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    f.git.run(
        &["tag", "-a", "release", "-m", "local annotation", "HEAD"],
        None,
    );
    let raw = f.git.oid("refs/tags/release");
    let p = tag_plan(&f, false, "release").await.unwrap();
    let r = execute(&f, p).await;
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/tags/release"], None)
        )
        .unwrap()
        .trim(),
        raw
    );
    assert_eq!(
        f.git
            .run_at(&f.remote, &["cat-file", "-t", "refs/tags/release"], None),
        b"tag\n"
    );
    f.git.run(
        &[
            "tag",
            "-f",
            "-a",
            "release",
            "-m",
            "changed tag object",
            "HEAD",
        ],
        None,
    );
    let replacement = f.git.oid("refs/tags/release");
    assert_eq!(
        tag_plan(&f, false, "release").await.unwrap_err().code,
        ErrorCode::DestinationExists
    );
    let delete = tag_plan(&f, true, "release").await.unwrap();
    f.git.run_at(
        &f.remote,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            &f.git.git_path(&f.git.path),
            "refs/tags/release:refs/tags/temporary-import",
        ],
        None,
    );
    f.git.run_at(
        &f.remote,
        &["update-ref", "refs/tags/release", &replacement, &raw],
        None,
    );
    let rejected = execute(&f, delete).await;
    assert_eq!(
        rejected.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{rejected:?}"
    );
    assert_eq!(
        String::from_utf8(
            f.git
                .run_at(&f.remote, &["rev-parse", "refs/tags/release"], None)
        )
        .unwrap()
        .trim(),
        replacement
    );
    let p = tag_plan(&f, true, "release").await.unwrap();
    let removed = execute(&f, p).await;
    assert_eq!(
        removed.operation.state,
        OperationState::Succeeded,
        "{removed:?}"
    );
    assert!(removed.result.unwrap().publication.unwrap().deleted);
    assert!(f
        .git
        .run_at(
            &f.remote,
            &["for-each-ref", "--format=%(refname)", "refs/tags/release"],
            None
        )
        .is_empty());
    assert_eq!(f.git.oid("refs/tags/release"), replacement);
}
async fn wait_sync(f: &support::daily::Fixture, id: &str) -> SyncActionOperationResult {
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let r = f
                .backend
                .get_sync_action_outcome(
                    "main",
                    GetActionOutcomeRequest {
                        operation_id: id.into(),
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
async fn read_push(f: &support::daily::Fixture) -> (String, RemoteRefSnapshot) {
    let c = f
        .backend
        .get_remote_actions(
            "main",
            GetRemoteActionsRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let h = c
        .remotes
        .iter()
        .find(|r| r.name == "origin")
        .unwrap()
        .endpoints
        .iter()
        .find(|e| e.role == RemoteEndpointRole::Push)
        .unwrap()
        .handle
        .clone()
        .unwrap();
    let p = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::ReadRemoteRefs {
                    remote_handle: h.clone(),
                },
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest { plan_id: p.plan_id },
        )
        .unwrap();
    f.wait(&op.id).await;
    let r = f
        .backend
        .get_remote_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap();
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    (h, r.snapshot.unwrap())
}
async fn local_handle(f: &support::daily::Fixture, name: &str) -> String {
    let c = f
        .backend
        .get_ref_actions(
            "main",
            GetRefActionsRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap();
    let id = c
        .refs
        .iter()
        .find(|r| r.name == name)
        .unwrap()
        .entry_id
        .clone();
    f.backend
        .resolve_ref_action_target(
            "main",
            ResolveRefActionTargetRequest {
                scope: f.scope.clone(),
                catalog_id: c.catalog_id,
                source: RefActionSource::Ref { entry_id: id },
            },
        )
        .await
        .unwrap()
        .handle
}
async fn push_plan(f: &support::daily::Fixture, destination: &str) -> PreparedAction {
    let (remote_handle, snapshot) = read_push(f).await;
    let source_handle = local_handle(f, "refs/heads/main").await;
    let source = if let Some(entry) = snapshot
        .refs
        .iter()
        .find(|e| e.name == format!("refs/heads/{destination}"))
    {
        RemoteTargetSource::Existing {
            entry_id: entry.entry_id.clone(),
        }
    } else {
        RemoteTargetSource::New {
            namespace: RemoteRefNamespace::Branch,
            name: destination.into(),
        }
    };
    let target = f
        .backend
        .resolve_remote_target(
            "main",
            ResolveRemoteTargetRequest {
                scope: f.scope.clone(),
                snapshot_id: snapshot.snapshot_id,
                source,
            },
        )
        .await
        .unwrap();
    f.backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Push {
                    remote_handle,
                    source_handle,
                    destination_handle: target.handle,
                    set_upstream: false,
                },
            },
        )
        .await
        .unwrap()
}
async fn execute(f: &support::daily::Fixture, plan: PreparedAction) -> SyncActionOperationResult {
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        f.backend
            .execute_repository_action(
                "main",
                ExecuteRepositoryActionRequest {
                    plan_id: plan.plan_id
                }
            )
            .unwrap()
            .id,
        op.id
    );
    wait_sync(f, &op.id).await
}
#[tokio::test]
async fn branch_push_exact_server_oid_keeps_tracking_dated_and_records_safe_ack_new_update_up_to_date(
) {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    std::fs::write(f.git.path.join("keep dirty.txt"), b"keep working bytes\n").unwrap();
    let tracking = f.git.oid("refs/remotes/origin/main");
    f.git
        .run(&["commit", "--allow-empty", "-m", "next local"], None);
    let source = f.git.oid("HEAD");
    for dest in ["new-target", "main", "main"] {
        let plan = push_plan(&f, dest).await;
        let r = execute(&f, plan).await;
        assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
        assert_eq!(
            r.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::Applied,
            "{r:?}"
        );
        let actual = String::from_utf8(f.git.run_at(
            &f.remote,
            &["rev-parse", &format!("refs/heads/{dest}")],
            None,
        ))
        .unwrap();
        assert_eq!(actual.trim(), source);
        assert_eq!(f.git.oid("refs/remotes/origin/main"), tracking);
        assert_eq!(
            std::fs::read(f.git.path.join("keep dirty.txt")).unwrap(),
            b"keep working bytes\n"
        );
        let result = r.result.unwrap();
        assert!(result.tracking_refresh_required);
        assert!(!result.tracking_updated);
        assert!(!result.upstream_updated);
        let ack = result.publication.unwrap();
        assert_eq!(ack.acknowledged_oid.as_deref(), Some(source.as_str()));
        assert_eq!(ack.role, RemoteEndpointRole::Push);
        let handle = local_handle(&f, "refs/heads/main").await;
        let evidence = f
            .backend
            .get_publication_evidence(
                "main",
                GetPublicationEvidenceRequest {
                    scope: f.scope.clone(),
                    source_handle: handle,
                },
            )
            .await
            .unwrap();
        assert!(evidence
            .observations
            .iter()
            .any(|e| e.destination == format!("refs/heads/{dest}")
                && e.acknowledged_oid.as_deref() == Some(&source)));
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id)
                .unwrap();
        let raw = serde_json::to_string(&record).unwrap();
        assert!(!raw.contains(&f.git.git_path(&f.remote)));
        assert!(!raw.contains("opengit-captured-"));
    }
}
