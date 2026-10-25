#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
async fn catalog(f: &support::daily::Fixture) -> RemoteActionCatalog {
    f.backend
        .get_remote_actions(
            "main",
            GetRemoteActionsRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap()
}
async fn read(
    f: &support::daily::Fixture,
    role: RemoteEndpointRole,
) -> RemoteActionOperationResult {
    let c = catalog(f).await;
    let handle = c
        .remotes
        .iter()
        .find(|r| r.name == "origin")
        .unwrap()
        .endpoints
        .iter()
        .find(|e| e.role == role)
        .unwrap()
        .handle
        .clone()
        .unwrap();
    let plan = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::ReadRemoteRefs {
                    remote_handle: handle,
                },
            },
        )
        .await
        .unwrap();
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id.clone(),
            },
        )
        .unwrap();
    let old = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    assert_eq!(old.id, op.id);
    f.wait(&op.id).await;
    f.backend
        .get_remote_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: op.id,
            },
        )
        .unwrap()
}
#[tokio::test]
async fn actual_remote_read_pins_split_push_endpoint_preserves_git_metadata_and_uses_lazy_exact_targets(
) {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 10, storage).await;
        let push = f.git.owner.child("push Ω = ' remote.git").unwrap();
        f.git.run(
            &[
                "clone",
                "--bare",
                "--",
                &f.git.git_path(&f.remote),
                &f.git.git_path(&push),
            ],
            None,
        );
        f.git.run_at(
            &push,
            &["update-ref", "refs/heads/push-only", &f.git.oid("HEAD")],
            None,
        );
        f.git.run_at(
            &push,
            &["tag", "-a", "annotated", "-m", "runtime only annotation"],
            None,
        );
        // Push endpoint rewrite is expanded once and pinned through a fresh command-local alias.
        f.git.run(
            &[
                "config",
                &format!("url.{}.pushInsteadOf", f.git.git_path(&push)),
                &f.git.git_path(&f.remote),
            ],
            None,
        );
        std::fs::write(f.git.path.join("dirty untracked"), b"keep").unwrap();
        std::fs::write(f.git.path.join(".git/FETCH_HEAD"), b"FETCH_HEAD sentinel").unwrap();
        let metadata = support::daily::metadata_tree(&f.git.path.join(".git"));
        let before = f.worktree_state();
        let refs = f.git.run(&["show-ref"], None);
        let fetch = read(&f, RemoteEndpointRole::Fetch).await;
        assert_eq!(
            fetch.operation.state,
            OperationState::Succeeded,
            "{:?}",
            fetch.operation.error
        );
        assert!(!fetch
            .snapshot
            .unwrap()
            .refs
            .iter()
            .any(|r| r.name == "refs/heads/push-only"));
        let result = read(&f, RemoteEndpointRole::Push).await;
        assert_eq!(
            result.operation.state,
            OperationState::Succeeded,
            "{:?}",
            result.operation.error
        );
        assert_eq!(
            result.operation.action_kind,
            Some(ActionKind::ReadRemoteRefs)
        );
        assert_eq!(
            result.operation.outcome.as_ref().unwrap().effects,
            ActionEffects::NotApplied
        );
        assert!(
            !result
                .operation
                .outcome
                .as_ref()
                .unwrap()
                .repository_changed
        );
        assert_eq!(result.availability, RemoteSnapshotAvailability::Available);
        let evidence = result.evidence.unwrap();
        assert!(evidence.no_git_writes);
        assert_eq!(evidence.phase, FetchStepPhase::Observed);
        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.object_format.as_deref(), Some(format));
        assert!(snapshot
            .refs
            .iter()
            .any(|r| r.name == "refs/heads/push-only"));
        let entry = snapshot
            .refs
            .iter()
            .find(|r| r.name == "refs/tags/annotated")
            .unwrap();
        assert_eq!(
            entry.oid,
            String::from_utf8(
                f.git
                    .run_at(&push, &["rev-parse", "refs/tags/annotated"], None)
            )
            .unwrap()
            .trim()
        );
        let mut current = f.scope.clone();
        current.generation = "2".into();
        let target = f
            .backend
            .resolve_remote_target(
                "main",
                ResolveRemoteTargetRequest {
                    scope: current.clone(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    source: RemoteTargetSource::Existing {
                        entry_id: entry.entry_id.clone(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(target.expected_oid.as_ref(), Some(&entry.oid));
        assert_eq!(target.scope, current);
        assert_eq!(target.namespace, RemoteRefNamespace::Tag);
        let new = f
            .backend
            .resolve_remote_target(
                "main",
                ResolveRemoteTargetRequest {
                    scope: current.clone(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    source: RemoteTargetSource::New {
                        namespace: RemoteRefNamespace::Branch,
                        name: "new-destination".into(),
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(new.expected_oid, None);
        assert!(f
            .backend
            .resolve_remote_target(
                "main",
                ResolveRemoteTargetRequest {
                    scope: current.clone(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    source: RemoteTargetSource::New {
                        namespace: RemoteRefNamespace::Tag,
                        name: "annotated".into()
                    }
                }
            )
            .await
            .is_err());
        assert!(f
            .backend
            .get_remote_action_outcome(
                "other",
                GetActionOutcomeRequest {
                    operation_id: result.operation.id.clone()
                }
            )
            .is_err());
        assert_eq!(f.worktree_state(), before);
        assert_eq!(f.git.run(&["show-ref"], None), refs);
        assert_eq!(
            std::fs::read(f.git.path.join(".git/FETCH_HEAD")).unwrap(),
            b"FETCH_HEAD sentinel"
        );
        assert_eq!(
            support::daily::metadata_tree(&f.git.path.join(".git")),
            metadata
        );
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &result.operation.id)
                .unwrap();
        assert_eq!(
            record.phase,
            opengit_lib::actions::journal::WritePhase::Observed
        );
        assert!(record.remote_read.as_ref().unwrap().no_git_writes);
        let raw = serde_json::to_string(&record).unwrap();
        for secret in [
            snapshot.snapshot_id.as_str(),
            target.handle.as_str(),
            f.git.git_path(&push).as_str(),
            "runtime only annotation",
        ] {
            assert!(!raw.contains(secret));
        }
        f.git.run(
            &[
                "config",
                "remote.origin.pushurl",
                &f.git.git_path(&f.remote),
            ],
            None,
        );
        assert_eq!(
            f.backend
                .resolve_remote_target(
                    "main",
                    ResolveRemoteTargetRequest {
                        scope: current,
                        snapshot_id: snapshot.snapshot_id,
                        source: RemoteTargetSource::Existing {
                            entry_id: entry.entry_id.clone()
                        }
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::ExternalChange
        );
    }
}
#[tokio::test]
async fn actual_empty_remote_and_unsupported_profiles_have_no_invented_object_format_or_handles() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let empty = f.git.owner.child("empty remote.git").unwrap();
    f.git
        .run(&["init", "--bare", &f.git.git_path(&empty)], None);
    f.git.run(
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            &f.git.git_path(&empty),
        ],
        None,
    );
    let result = read(&f, RemoteEndpointRole::Push).await;
    assert_eq!(
        result.operation.state,
        OperationState::Succeeded,
        "{:?}",
        result.operation.error
    );
    let snapshot = result.snapshot.unwrap();
    assert!(snapshot.refs.is_empty());
    assert_eq!(snapshot.object_format, None);
    f.git.run(
        &[
            "remote",
            "set-url",
            "--push",
            "--add",
            "origin",
            &f.git.git_path(&f.remote),
        ],
        None,
    );
    let c = catalog(&f).await;
    let p = &c.remotes[0].endpoints[1];
    assert_eq!(p.capability, RemoteEndpointCapability::MultipleUrls);
    assert!(p.handle.is_none());
    f.git.run(&["config", "remote.origin.mirror", "true"], None);
    assert!(catalog(&f).await.remotes[0]
        .endpoints
        .iter()
        .all(|e| e.capability == RemoteEndpointCapability::Mirror && e.handle.is_none()));
}

async fn prepared(f: &support::daily::Fixture) -> PreparedAction {
    let c = catalog(f).await;
    f.backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::ReadRemoteRefs {
                    remote_handle: c.remotes[0].endpoints[0].handle.clone().unwrap(),
                },
            },
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn remote_read_native_failures_are_bounded_no_write_evidence_and_cancellation_never_masks_unknown_stop(
) {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(
        proxy.runner.clone(),
        Some(std::time::Duration::from_secs(3)),
    )
    .await;
    let before = f.worktree_state();
    let metadata = support::daily::metadata_tree(&f.git.path.join(".git"));
    for (mode, code) in [
        ("remote-auth", ErrorCode::AuthenticationFailed),
        ("remote-malformed", ErrorCode::MalformedOutput),
        ("remote-output", ErrorCode::OutputLimit),
    ] {
        proxy.set(mode);
        let result = read(&f, RemoteEndpointRole::Fetch).await;
        assert_eq!(
            result.operation.state,
            OperationState::Failed,
            "{mode}: {:?}",
            result.operation.error
        );
        assert_eq!(result.operation.error.as_ref().unwrap().code, code);
        assert_eq!(
            result.operation.outcome.unwrap().effects,
            ActionEffects::NotApplied
        );
        assert_eq!(result.availability, RemoteSnapshotAvailability::Unavailable);
        assert!(result.evidence.unwrap().no_git_writes);
        assert!(result.snapshot.is_none());
    }
    proxy.set("remote-hold");
    let plan = prepared(&f).await;
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    let record = opengit_lib::actions::journal::read(&f.backend.registry.database, &op.id).unwrap();
    assert_eq!(
        record.phase,
        opengit_lib::actions::journal::WritePhase::Prepared
    );
    assert_eq!(record.remote_read.unwrap().phase, FetchStepPhase::Started);
    f.backend
        .cancel_operation(
            "main",
            CancelOperationRequest {
                operation_id: op.id.clone(),
            },
        )
        .unwrap();
    let result = f.wait(&op.id).await;
    assert_eq!(
        result.operation.state,
        OperationState::Cancelled,
        "{:?}",
        result.operation.error
    );
    assert_eq!(
        result.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    std::fs::remove_file(&proxy.ready).unwrap();
    let plan = prepared(&f).await;
    let timed = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    let timed = f.wait(&timed.id).await;
    assert_eq!(
        timed.operation.error.as_ref().unwrap().code,
        ErrorCode::Timeout
    );
    assert_eq!(
        timed.operation.outcome.unwrap().effects,
        ActionEffects::NotApplied
    );
    std::fs::remove_file(&proxy.ready).unwrap();
    let plan = prepared(&f).await;
    let closing = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: plan.plan_id,
            },
        )
        .unwrap();
    proxy.wait_ready().await;
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    let done = f.wait(&closing.id).await;
    assert_eq!(done.operation.state, OperationState::Cancelled);
    let remote = f
        .backend
        .get_remote_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: closing.id,
            },
        )
        .unwrap();
    assert!(remote.snapshot.is_none());
    assert_eq!(remote.availability, RemoteSnapshotAvailability::Unavailable);
    assert_eq!(f.worktree_state(), before);
    assert_eq!(
        support::daily::metadata_tree(&f.git.path.join(".git")),
        metadata
    );
}
#[tokio::test]
async fn b1_life01_stale_resolver_preserves_reopened_generation_catalog_grants_and_unused_plan() {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    f.scope.generation = "9".into();
    let old_scope = f.scope.clone();
    let old = read(&f, RemoteEndpointRole::Fetch).await.snapshot.unwrap();
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    f.scope.generation = "1".into();
    let fresh = catalog(&f).await;
    let handle = fresh.remotes[0].endpoints[0].handle.clone().unwrap();
    let prepare = || PrepareRepositoryActionRequest {
        scope: f.scope.clone(),
        intent: ActionIntent::ReadRemoteRefs {
            remote_handle: handle.clone(),
        },
    };
    let unused = f
        .backend
        .prepare_repository_action("main", prepare())
        .await
        .unwrap();
    assert_eq!(
        f.backend
            .resolve_remote_target(
                "main",
                ResolveRemoteTargetRequest {
                    scope: old_scope,
                    snapshot_id: old.snapshot_id,
                    source: RemoteTargetSource::Existing {
                        entry_id: old.refs[0].entry_id.clone()
                    },
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    // Check the grant and unused plan before another read can replace any state.
    f.backend
        .prepare_repository_action("main", prepare())
        .await
        .expect("stale resolver must preserve the fresh grant");
    let op = f
        .backend
        .execute_repository_action(
            "main",
            ExecuteRepositoryActionRequest {
                plan_id: unused.plan_id,
            },
        )
        .expect("stale resolver must preserve the unused generation1 plan");
    assert_eq!(
        f.wait(&op.id).await.operation.state,
        OperationState::Succeeded
    );
    let next = catalog(&f).await;
    assert_eq!(next.scope, f.scope);
    assert_eq!(next.remotes[0].endpoints[0].handle.as_ref(), Some(&handle));
}

#[tokio::test]
async fn remote_snapshot_does_not_revive_after_selection_close_or_restart() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let result = read(&f, RemoteEndpointRole::Fetch).await;
    let snap = result.snapshot.unwrap();
    let entry = snap.refs[0].entry_id.clone();
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    let result = f
        .backend
        .get_remote_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: result.operation.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(result.availability, RemoteSnapshotAvailability::Unavailable);
    assert!(result.evidence.unwrap().no_git_writes);
    assert!(f
        .backend
        .resolve_remote_target(
            "main",
            ResolveRemoteTargetRequest {
                scope: f.scope.clone(),
                snapshot_id: snap.snapshot_id,
                source: RemoteTargetSource::Existing { entry_id: entry }
            }
        )
        .await
        .is_err());
    // Reopen database through the same backend-known profile path, with no runtime cache.
    let db_path = {
        let c = f.backend.registry.database.lock().unwrap();
        c.query_row("PRAGMA database_list", [], |r| r.get::<_, String>(2))
            .unwrap()
    };
    let support::daily::Fixture { backend, git, .. } = f;
    drop(backend); // Database intentionally holds an exclusive profile lease.
    let restarted = opengit_lib::backend::Backend::with_git(
        std::path::Path::new(&db_path).parent().unwrap(),
        git.git.clone(),
    )
    .unwrap();
    restarted.get_workspace("main").unwrap();
    let after = restarted
        .get_remote_action_outcome(
            "main",
            GetActionOutcomeRequest {
                operation_id: result.operation.id,
            },
        )
        .unwrap();
    assert_eq!(after.availability, RemoteSnapshotAvailability::Unavailable);
    assert!(after.snapshot.is_none());
    assert!(after.evidence.unwrap().no_git_writes);
}

#[tokio::test]
#[ignore = "requires owned LiveRemotes launcher; no external provider or production credentials"]
async fn live_private_remote_read_preserves_url_specific_tls_helper_and_pinned_ssh_agent() {
    let descriptor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::env::var_os("OPENGIT_B1_FIXTURE_DESCRIPTOR").expect("owned fixture descriptor"),
        )
        .unwrap(),
    )
    .unwrap();
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    let source = f.git.owner.child("credential helper.rs").unwrap();
    let helper = f.git.owner.child("credential helper Ω = ' .exe").unwrap();
    std::fs::write(&source,r#"fn main(){use std::io::Read;let mut input=String::new();std::io::stdin().read_to_string(&mut input).unwrap();if std::env::args().nth(1).as_deref()==Some("get"){println!("username=fixture\npassword={}\n",std::env::var("OPENGIT_B1_SYNTHETIC_PASSWORD").unwrap());}}"#).unwrap();
    let rustc = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .filter(|p| p.is_absolute())
        .map(|p| p.join("rustc.exe"))
        .find(|p| p.is_file())
        .unwrap();
    assert!(std::process::Command::new(rustc)
        .arg(&source)
        .args([
            "--crate-name",
            "owned_credential_helper",
            "--edition=2021",
            "-o"
        ])
        .arg(&helper)
        .status()
        .unwrap()
        .success());
    let https = descriptor["https"].as_str().unwrap();
    let ssh = descriptor["ssh"].as_str().unwrap();
    f.git.run(&["remote", "set-url", "origin", https], None);
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
    let credential = f.git.run(
        &["credential", "fill"],
        Some(b"protocol=https\nhost=127.0.0.1\n\n"),
    );
    let secret = std::env::var("OPENGIT_B1_SYNTHETIC_PASSWORD").unwrap();
    assert!(
        credential
            .windows(secret.len())
            .any(|v| v == secret.as_bytes()),
        "owned helper did not return synthetic credential"
    );
    // No blanket CA setting: this specific expanded URL must retain trust matching.
    let before = support::daily::metadata_tree(&f.git.path.join(".git"));
    let result = read(&f, RemoteEndpointRole::Fetch).await;
    assert_eq!(
        result.operation.state,
        OperationState::Succeeded,
        "{:?}",
        result.operation.error
    );
    assert!(!result.snapshot.unwrap().refs.is_empty());
    assert!(result.evidence.unwrap().no_git_writes);
    assert_eq!(
        support::daily::metadata_tree(&f.git.path.join(".git")),
        before
    );
    let secret = std::env::var("OPENGIT_B1_SYNTHETIC_PASSWORD").unwrap();
    let record =
        opengit_lib::actions::journal::read(&f.backend.registry.database, &result.operation.id)
            .unwrap();
    assert!(!serde_json::to_string(&record).unwrap().contains(&secret));
    f.git
        .run(&["remote", "set-url", "--push", "origin", ssh], None);
    let before = support::daily::metadata_tree(&f.git.path.join(".git"));
    let result = read(&f, RemoteEndpointRole::Push).await;
    assert_eq!(
        result.operation.state,
        OperationState::Succeeded,
        "{:?}",
        result.operation.error
    );
    assert!(!result.snapshot.unwrap().refs.is_empty());
    assert!(result.evidence.unwrap().no_git_writes);
    assert_eq!(
        support::daily::metadata_tree(&f.git.path.join(".git")),
        before
    );
}

#[tokio::test]
async fn remote_read_is_valid_from_unborn_and_bare_repositories_without_local_git_writes() {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    let unborn = f.git.owner.child("unborn working repo").unwrap();
    f.git
        .run(&["init", "-b", "main", &f.git.git_path(&unborn)], None);
    f.git.run_at(
        &unborn,
        &["remote", "add", "origin", &f.git.git_path(&f.remote)],
        None,
    );
    for path in [unborn.clone(), f.remote.clone()] {
        let active = f
            .backend
            .register_repository("main", &path)
            .await
            .unwrap()
            .active_selection
            .unwrap();
        f.scope = ActionScope {
            repo_id: active.repo_id,
            worktree_id: active.worktree_id,
            generation: "1".into(),
        };
        let metadata = if path == unborn {
            path.join(".git")
        } else {
            path.clone()
        };
        let before = support::daily::metadata_tree(&metadata);
        let result = read(&f, RemoteEndpointRole::Fetch).await;
        assert_eq!(
            result.operation.state,
            OperationState::Succeeded,
            "{:?}",
            result.operation.error
        );
        assert!(result.evidence.unwrap().no_git_writes);
        assert_eq!(support::daily::metadata_tree(&metadata), before);
    }
}
