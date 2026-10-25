#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
async fn catalog(f: &support::daily::Fixture) -> RepositoryChanges {
    f.backend
        .get_repository_changes(
            "main",
            GetRepositoryChangesRequest {
                scope: f.scope.clone(),
            },
        )
        .await
        .unwrap()
}
async fn prepared(f: &support::daily::Fixture, names: &[&str], side: ChangeSide) -> PreparedAction {
    let c = catalog(f).await;
    let selections = names
        .iter()
        .map(|name| ChangeSelection {
            entry_id: c
                .entries
                .iter()
                .find(|e| e.path_display == *name)
                .unwrap_or_else(|| panic!("missing {name}: {:?}", c.entries))
                .entry_id
                .clone(),
            side,
        })
        .collect();
    let r = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: c.catalog_id,
                targets: selections,
            },
        )
        .await
        .unwrap();
    let handles = r.targets.into_iter().map(|t| t.change_handle).collect();
    f.backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: if side == ChangeSide::Unstaged {
                    ActionIntent::Stage {
                        change_handles: handles,
                    }
                } else {
                    ActionIntent::Unstage {
                        change_handles: handles,
                    }
                },
            },
        )
        .await
        .unwrap()
}
async fn execute(f: &support::daily::Fixture, p: PreparedAction) -> ChangeActionOperationResult {
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
async fn commit(f: &support::daily::Fixture, message: &str) -> ChangeActionOperationResult {
    catalog(f).await;
    let p = f
        .backend
        .prepare_repository_action(
            "main",
            PrepareRepositoryActionRequest {
                scope: f.scope.clone(),
                intent: ActionIntent::Commit {
                    message: message.into(),
                    amend: false,
                },
            },
        )
        .await
        .unwrap();
    execute(f, p).await
}
fn success(r: &ChangeActionOperationResult) {
    assert_eq!(r.operation.state, OperationState::Succeeded, "{r:?}");
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Applied,
        "{r:?}"
    );
    assert!(!r.result.as_ref().unwrap().artifacts_retained, "{r:?}");
}
fn setup_files(f: &support::daily::Fixture) {
    f.git.run(&["config", "user.name", "Fixture"], None);
    f.git
        .run(&["config", "user.email", "fixture@example.invalid"], None);
    for name in ["same.txt", "unrelated.txt", "deleted.txt", "renamed.txt"] {
        std::fs::write(f.git.path.join(name), format!("base {name}\n")).unwrap();
    }
    f.git.run(&["add", "--all"], None);
    f.git.run(&["commit", "-m", "fixture files"], None);
}
#[tokio::test]
async fn whole_file_stage_unstage_preserves_complete_unrelated_index_and_working_bytes_with_literal_names(
) {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 6, storage).await;
        setup_files(&f);
        std::fs::write(f.git.path.join("same.txt"), b"staged one\n").unwrap();
        std::fs::write(f.git.path.join("unrelated.txt"), b"unrelated staged\n").unwrap();
        f.git.run(&["add", "--", "same.txt", "unrelated.txt"], None);
        std::fs::write(f.git.path.join("same.txt"), b"working two\n").unwrap();
        std::fs::write(f.git.path.join("unrelated.txt"), b"unrelated working\n").unwrap();
        std::fs::remove_file(f.git.path.join("deleted.txt")).unwrap();
        std::fs::rename(
            f.git.path.join("renamed.txt"),
            f.git.path.join("renamed Ω.txt"),
        )
        .unwrap();
        for name in ["-leading.txt", "[literal].txt", "space Ω.txt"] {
            std::fs::write(f.git.path.join(name), format!("data {name}\n")).unwrap();
        }
        std::fs::write(f.git.path.join("binary.dat"), [0, 255, 1, 0, 128]).unwrap();
        let unrelated = f
            .git
            .run(&["ls-files", "--stage", "--", "unrelated.txt"], None);
        let names = [
            "same.txt",
            "deleted.txt",
            "renamed.txt",
            "renamed Ω.txt",
            "-leading.txt",
            "[literal].txt",
            "space Ω.txt",
            "binary.dat",
        ];
        let working = names
            .iter()
            .filter_map(|n| std::fs::read(f.git.path.join(n)).ok().map(|b| (*n, b)))
            .collect::<std::collections::BTreeMap<_, _>>();
        let p = prepared(&f, &names, ChangeSide::Unstaged).await;
        let r = execute(&f, p).await;
        success(&r);
        assert_eq!(
            f.git
                .run(&["ls-files", "--stage", "--", "unrelated.txt"], None),
            unrelated
        );
        for (name, b) in &working {
            assert_eq!(&std::fs::read(f.git.path.join(name)).unwrap(), b);
            assert_eq!(f.git.run(&["show", &format!(":{name}")], None), *b);
        }
        let all = f.git.run(&["ls-files", "--stage", "-z"], None);
        assert_eq!(all.split(|b| *b == 0).filter(|b| !b.is_empty()).count(), 7);
        assert!(!all
            .windows(b"\tdeleted.txt\0".len())
            .any(|w| w == b"\tdeleted.txt\0"));
        assert_eq!(
            std::fs::read(f.git.path.join("unrelated.txt")).unwrap(),
            b"unrelated working\n"
        );
        let tree = String::from_utf8(f.git.run(&["write-tree"], None)).unwrap();
        assert_eq!(r.result.unwrap().tree_oid.as_deref(), Some(tree.trim()));
        let p = prepared(&f, &["same.txt"], ChangeSide::Staged).await;
        success(&execute(&f, p).await);
        assert_eq!(f.git.run(&["show", ":same.txt"], None), b"base same.txt\n");
        assert_eq!(
            std::fs::read(f.git.path.join("same.txt")).unwrap(),
            b"working two\n"
        );
        assert_eq!(
            f.git
                .run(&["ls-files", "--stage", "--", "unrelated.txt"], None),
            unrelated
        );
    }
}
#[tokio::test]
async fn unborn_index_stage_unstage_and_real_commit_use_exact_tree_and_parent() {
    for (format, storage) in [("sha1", "files"), ("sha256", "reftable")] {
        let f = support::daily::Fixture::new(format, 6, storage).await;
        f.git.run(&["config", "user.name", "Fixture"], None);
        f.git
            .run(&["config", "user.email", "fixture@example.invalid"], None);
        f.git.run(&["switch", "--orphan", "unborn"], None);
        let index = f.git.path.join(".git/index");
        if index.exists() {
            std::fs::remove_file(&index).unwrap();
        }
        std::fs::write(f.git.path.join("new Ω.txt"), b"unborn data\n").unwrap();
        let p = prepared(&f, &["new Ω.txt"], ChangeSide::Unstaged).await;
        success(&execute(&f, p).await);
        let p = prepared(&f, &["new Ω.txt"], ChangeSide::Staged).await;
        success(&execute(&f, p).await);
        assert!(f.git.run(&["ls-files", "-z"], None).is_empty());
        assert_eq!(
            std::fs::read(f.git.path.join("new Ω.txt")).unwrap(),
            b"unborn data\n"
        );
        let p = prepared(&f, &["new Ω.txt"], ChangeSide::Unstaged).await;
        success(&execute(&f, p).await);
        let expected = f.git.run(&["write-tree"], None);
        let r = commit(&f, "real initial commit canary message").await;
        success(&r);
        let oid = f.git.oid("HEAD");
        assert_eq!(
            r.result.as_ref().unwrap().commit_oid.as_deref(),
            Some(oid.as_str())
        );
        assert_eq!(
            r.result
                .as_ref()
                .unwrap()
                .tree_oid
                .as_ref()
                .unwrap()
                .as_bytes(),
            expected.strip_suffix(b"\n").unwrap()
        );
        assert_eq!(
            String::from_utf8(
                f.git
                    .run(&["rev-list", "--parents", "-n", "1", "HEAD"], None)
            )
            .unwrap()
            .split_whitespace()
            .count(),
            1
        );
        let record =
            opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id)
                .unwrap();
        assert!(!serde_json::to_string(&record)
            .unwrap()
            .contains("real initial commit canary message"));
    }
}
#[tokio::test]
async fn stale_selected_content_and_foreign_index_lock_reject_without_changing_real_index() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup_files(&f);
    std::fs::write(f.git.path.join("same.txt"), b"version one\n").unwrap();
    let p = prepared(&f, &["same.txt"], ChangeSide::Unstaged).await;
    let before = std::fs::read(f.git.path.join(".git/index")).unwrap();
    std::fs::write(f.git.path.join("same.txt"), b"version two\n").unwrap();
    let r = execute(&f, p).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(
        std::fs::read(f.git.path.join(".git/index")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(f.git.path.join("same.txt")).unwrap(),
        b"version two\n"
    );
    let p = prepared(&f, &["same.txt"], ChangeSide::Unstaged).await;
    std::fs::write(f.git.path.join(".git/index.lock"), b"foreign lock keep").unwrap();
    let r = execute(&f, p).await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied
    );
    assert_eq!(
        r.result.unwrap().failure_reason,
        Some(ChangeFailureReason::Locked)
    );
    assert_eq!(
        std::fs::read(f.git.path.join(".git/index.lock")).unwrap(),
        b"foreign lock keep"
    );
    assert_eq!(
        std::fs::read(f.git.path.join(".git/index")).unwrap(),
        before
    );
}
#[tokio::test]
async fn native_commit_hook_index_worktree_effects_and_signing_failure_remain_visible() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup_files(&f);
    std::fs::write(f.git.path.join("same.txt"), b"commit staged\n").unwrap();
    f.git.run(&["add", "--", "same.txt"], None);
    let original = f.git.oid("HEAD");
    f.git.run(&["config", "user.name", ""], None);
    let identity_entries = f.git.run(&["ls-files", "--stage", "-z"], None);
    let identity = commit(&f, "missing identity must not bypass").await;
    assert!(matches!(
        identity.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied | ActionEffects::Partial
    ));
    assert_eq!(
        f.git.run(&["ls-files", "--stage", "-z"], None),
        identity_entries
    );
    assert_eq!(f.git.oid("HEAD"), original);
    assert_eq!(
        identity.result.as_ref().unwrap().failure_reason,
        Some(ChangeFailureReason::Identity)
    );
    f.git.run(&["config", "user.name", "Fixture"], None);
    let hooks = f.git.path.join(".git/hooks");
    std::fs::write(hooks.join("pre-commit"),b"#!/bin/sh\nprintf 'hook changed\\n' > hook-file.txt\ngit add -- hook-file.txt\nprintf 'RAW_HOOK_CANARY_NEVER_PERSIST' >&2\nexit 1\n").unwrap();
    let r = commit(&f, "MESSAGE_CANARY_NEVER_PERSIST").await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::Partial,
        "{r:?}"
    );
    assert_eq!(
        r.result.as_ref().unwrap().failure_reason,
        Some(ChangeFailureReason::HookOrNative)
    );
    assert_eq!(f.git.oid("HEAD"), original);
    assert_eq!(
        f.git.run(&["show", ":hook-file.txt"], None),
        b"hook changed\n"
    );
    let record =
        opengit_lib::actions::journal::read(&f.backend.registry.database, &r.operation.id).unwrap();
    let raw = serde_json::to_string(&record).unwrap();
    assert!(!raw.contains("MESSAGE_CANARY_NEVER_PERSIST"));
    assert!(!raw.contains("RAW_HOOK_CANARY_NEVER_PERSIST"));
    std::fs::remove_file(hooks.join("pre-commit")).unwrap();
    let expected = f.git.run(&["write-tree"], None);
    std::fs::write(
        hooks.join("post-commit"),
        b"#!/bin/sh\nprintf 'post staged\\n' > post-file.txt\ngit add -- post-file.txt\nexit 1\n",
    )
    .unwrap();
    let r = commit(&f, "accepted real commit").await;
    success(&r);
    assert_eq!(f.git.oid("HEAD^"), original);
    assert_eq!(f.git.run(&["rev-parse", "HEAD^{tree}"], None), expected);
    assert_eq!(
        f.git.run(&["show", ":post-file.txt"], None),
        b"post staged\n"
    );
    assert_ne!(f.git.run(&["write-tree"], None), expected);
    std::fs::remove_file(hooks.join("post-commit")).unwrap();
    let signer = f.git.owner.child("failed-signer.cmd").unwrap();
    std::fs::write(&signer, b"@exit /b 1\r\n").unwrap();
    f.git.run(&["config", "commit.gpgSign", "true"], None);
    f.git
        .run(&["config", "gpg.program", &f.git.git_path(&signer)], None);
    let before = f.git.oid("HEAD");
    let index = std::fs::read(f.git.path.join(".git/index")).unwrap();
    let r = commit(&f, "signing must not bypass").await;
    assert_eq!(
        r.operation.outcome.as_ref().unwrap().effects,
        ActionEffects::NotApplied,
        "{r:?}"
    );
    assert_eq!(
        r.result.unwrap().failure_reason,
        Some(ChangeFailureReason::Signing)
    );
    assert_eq!(f.git.oid("HEAD"), before);
    assert_eq!(std::fs::read(f.git.path.join(".git/index")).unwrap(), index);
}

#[tokio::test]
async fn mode_clean_filter_diff_and_unsupported_split_index_are_explicit() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup_files(&f);
    std::fs::write(f.git.path.join("filter.txt"), b"base\n").unwrap();
    std::fs::write(
        f.git.path.join(".gitattributes"),
        b"filter.txt filter=fixture\n",
    )
    .unwrap();
    f.git.run(
        &["config", "filter.fixture.clean", "sed s/working/clean/g"],
        None,
    );
    f.git
        .run(&["add", "--", "filter.txt", ".gitattributes"], None);
    f.git.run(&["commit", "-m", "filter base"], None);
    f.git.run(&["config", "core.filemode", "false"], None);
    f.git
        .run(&["update-index", "--chmod=+x", "--", "same.txt"], None);
    std::fs::write(f.git.path.join("filter.txt"), b"working data\n").unwrap();
    std::fs::write(f.git.path.join("same.txt"), b"mode and content\n").unwrap();
    let c = catalog(&f).await;
    let e = c
        .entries
        .iter()
        .find(|e| e.path_display == "filter.txt")
        .unwrap();
    let t = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: c.catalog_id,
                targets: vec![ChangeSelection {
                    entry_id: e.entry_id.clone(),
                    side: ChangeSide::Unstaged,
                }],
            },
        )
        .await
        .unwrap();
    let d = f
        .backend
        .get_change_diff(
            "main",
            GetChangeDiffRequest {
                scope: f.scope.clone(),
                change_handle: t.targets[0].change_handle.clone(),
            },
        )
        .await
        .unwrap();
    assert!(d.content.contains("clean data"));
    assert!(
        d.content.contains("Git reported diagnostics"),
        "configured system CRLF conversion produces a successful preview diagnostic"
    );
    assert!(d.hunks.is_empty());
    let p = prepared(&f, &["filter.txt", "same.txt"], ChangeSide::Unstaged).await;
    success(&execute(&f, p).await);
    assert_eq!(f.git.run(&["show", ":filter.txt"], None), b"clean data\n");
    assert_eq!(
        std::fs::read(f.git.path.join("filter.txt")).unwrap(),
        b"working data\n"
    );
    assert!(f
        .git
        .run(&["ls-files", "--stage", "--", "same.txt"], None)
        .starts_with(b"100755 "));
    f.git
        .run(&["mv", "--", "renamed.txt", "renamed Ω moved.txt"], None);
    f.git.run(&["rm", "--", "deleted.txt"], None);
    let unstage = prepared(
        &f,
        &["renamed Ω moved.txt", "deleted.txt"],
        ChangeSide::Staged,
    )
    .await;
    success(&execute(&f, unstage).await);
    assert_eq!(
        f.git.run(&["show", ":renamed.txt"], None),
        b"base renamed.txt\n"
    );
    assert_eq!(
        f.git.run(&["show", ":deleted.txt"], None),
        b"base deleted.txt\n"
    );
    assert!(f.git.path.join("renamed Ω moved.txt").is_file());
    assert!(!f.git.path.join("renamed.txt").exists());
    assert!(!f.git.path.join("deleted.txt").exists());
    assert_eq!(f.git.run(&["show", ":filter.txt"], None), b"clean data\n");
    f.git.run(&["update-index", "--split-index"], None);
    let c = catalog(&f).await;
    assert!(c.unsupported);
    assert!(c
        .entries
        .iter()
        .all(|e| e.stage_capability != ChangeCapability::Eligible
            && e.unstage_capability != ChangeCapability::Eligible));
}

#[tokio::test]
async fn real_ssh_signed_commit_uses_configured_signer_and_verifies_signature() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup_files(&f);
    let key = f.git.owner.child("signing-key").unwrap();
    let ssh = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap())
        .join("System32/OpenSSH/ssh-keygen.exe");
    assert!(
        ssh.is_file(),
        "owned local signing fixture requires installed OpenSSH ssh-keygen"
    );
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
    let allowed = f.git.owner.child("allowed-signers").unwrap();
    std::fs::write(
        &allowed,
        format!("fixture@example.invalid {}", public.trim()),
    )
    .unwrap();
    f.git.run(&["config", "gpg.format", "ssh"], None);
    f.git
        .run(&["config", "gpg.ssh.program", ssh.to_str().unwrap()], None);
    f.git
        .run(&["config", "user.signingKey", &f.git.git_path(&key)], None);
    f.git.run(
        &[
            "config",
            "gpg.ssh.allowedSignersFile",
            &f.git.git_path(&allowed),
        ],
        None,
    );
    f.git.run(&["config", "commit.gpgSign", "true"], None);
    std::fs::write(f.git.path.join("same.txt"), b"signed content\n").unwrap();
    f.git.run(&["add", "--", "same.txt"], None);
    let parent = f.git.oid("HEAD");
    let r = commit(&f, "signed real commit").await;
    success(&r);
    assert_eq!(f.git.oid("HEAD^"), parent);
    f.git.run(&["verify-commit", "HEAD"], None);
}

#[tokio::test]
async fn old_change_capabilities_and_prepare_cannot_poison_reopened_window_generation() {
    let mut f = support::daily::Fixture::new("sha1", 6, "files").await;
    std::fs::write(f.git.path.join("selected.txt"), b"keep selected\n").unwrap();
    f.scope.generation = "9".into();
    let old = catalog(&f).await;
    let old_scope = f.scope.clone();
    let selection = ChangeSelection {
        entry_id: old.entries[0].entry_id.clone(),
        side: ChangeSide::Unstaged,
    };
    let target = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: old_scope.clone(),
                catalog_id: old.catalog_id.clone(),
                targets: vec![selection.clone()],
            },
        )
        .await
        .unwrap()
        .targets[0]
        .change_handle
        .clone();
    f.backend.close_window("main");
    f.backend.get_workspace("main").unwrap();
    f.scope.generation = "1".into();
    let plan = prepared(&f, &["selected.txt"], ChangeSide::Unstaged).await;
    assert_eq!(
        f.backend
            .resolve_change_targets(
                "main",
                ResolveChangeTargetsRequest {
                    scope: old_scope.clone(),
                    catalog_id: old.catalog_id,
                    targets: vec![selection]
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        f.backend
            .get_change_diff(
                "main",
                GetChangeDiffRequest {
                    scope: old_scope.clone(),
                    change_handle: target.clone()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        f.backend
            .prepare_repository_action(
                "main",
                PrepareRepositoryActionRequest {
                    scope: old_scope,
                    intent: ActionIntent::Stage {
                        change_handles: vec![target]
                    }
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    success(&execute(&f, plan).await);
    assert_eq!(catalog(&f).await.scope.generation, "1");
    assert_eq!(
        std::fs::read(f.git.path.join("selected.txt")).unwrap(),
        b"keep selected\n"
    );
}
#[tokio::test]
async fn raw_change_locator_survives_catalog_reorder_without_retargeting_another_file() {
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    std::fs::write(f.git.path.join("z chosen.txt"), b"chosen payload\n").unwrap();
    let original = catalog(&f).await;
    let locator = original
        .entries
        .iter()
        .find(|e| e.path_display == "z chosen.txt")
        .unwrap()
        .entry_id
        .clone();
    std::fs::write(f.git.path.join("a earlier.txt"), b"other payload\n").unwrap();
    let fresh = catalog(&f).await;
    assert_ne!(original.catalog_id, fresh.catalog_id);
    assert_ne!(fresh.entries[0].entry_id, locator);
    assert_eq!(
        fresh
            .entries
            .iter()
            .find(|e| e.path_display == "z chosen.txt")
            .unwrap()
            .entry_id,
        locator
    );
    assert!(f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: original.catalog_id,
                targets: vec![ChangeSelection {
                    entry_id: locator.clone(),
                    side: ChangeSide::Unstaged
                }]
            }
        )
        .await
        .is_err());
    let resolved = f
        .backend
        .resolve_change_targets(
            "main",
            ResolveChangeTargetsRequest {
                scope: f.scope.clone(),
                catalog_id: fresh.catalog_id,
                targets: vec![ChangeSelection {
                    entry_id: locator,
                    side: ChangeSide::Unstaged,
                }],
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
                intent: ActionIntent::Stage {
                    change_handles: vec![resolved.targets[0].change_handle.clone()],
                },
            },
        )
        .await
        .unwrap();
    success(&execute(&f, plan).await);
    assert_eq!(f.git.run(&["ls-files", "-z"], None), b"z chosen.txt\0");
    assert_eq!(
        f.git.run(&["show", ":z chosen.txt"], None),
        b"chosen payload\n"
    );
    assert_eq!(
        std::fs::read(f.git.path.join("a earlier.txt")).unwrap(),
        b"other payload\n"
    );
}

#[tokio::test]
async fn s5a_rename01_unstaged_destination_never_stages_recreated_source_and_unstage_restores_only_index_pair(
) {
    const ORIGINAL: &[u8] =
        b"rename original line1\nrename original line2\nrename original line3\n";
    const EDITED:&[u8]=b"rename original line1\nrename original line2\nrename original line3\nselected destination edit\n";
    let f = support::daily::Fixture::new("sha1", 6, "files").await;
    setup_files(&f);
    std::fs::write(f.git.path.join("old.txt"), ORIGINAL).unwrap();
    f.git.run(&["add", "--", "old.txt"], None);
    f.git.run(&["commit", "-m", "rename fixture"], None);
    f.git.run(&["mv", "--", "old.txt", "new.txt"], None);
    std::fs::write(f.git.path.join("old.txt"), b"unrelated recreated source\n").unwrap();
    std::fs::write(f.git.path.join("new.txt"), EDITED).unwrap();
    std::fs::write(f.git.path.join("unrelated.txt"), b"keep unrelated staged\n").unwrap();
    f.git.run(&["add", "--", "unrelated.txt"], None);
    std::fs::write(
        f.git.path.join("unrelated.txt"),
        b"keep unrelated working\n",
    )
    .unwrap();
    fn entries(bytes: Vec<u8>) -> std::collections::BTreeMap<Vec<u8>, Vec<u8>> {
        bytes
            .split(|b| *b == 0)
            .filter(|e| !e.is_empty())
            .map(|e| {
                let at = e.iter().position(|b| *b == b'\t').unwrap();
                (e[at + 1..].to_vec(), e[..at].to_vec())
            })
            .collect()
    }
    let mut expected = entries(f.git.run(&["ls-files", "--stage", "-z"], None));
    assert!(!expected.contains_key(b"old.txt".as_slice()));
    let working = [
        "old.txt",
        "new.txt",
        "unrelated.txt",
        "same.txt",
        "deleted.txt",
        "renamed.txt",
    ]
    .into_iter()
    .map(|p| (p, std::fs::read(f.git.path.join(p)).unwrap()))
    .collect::<std::collections::BTreeMap<_, _>>();
    let c = catalog(&f).await;
    let renamed = c
        .entries
        .iter()
        .find(|e| e.path_display == "new.txt")
        .unwrap();
    assert_eq!(renamed.index_status, PathStatus::R);
    assert_eq!(renamed.worktree_status, PathStatus::M);
    assert_eq!(renamed.previous_path_display.as_deref(), Some("old.txt"));
    let p = prepared(&f, &["new.txt"], ChangeSide::Unstaged).await;
    success(&execute(&f, p).await);
    let actual = entries(f.git.run(&["ls-files", "--stage", "-z"], None));
    assert!(
        !actual.contains_key(b"old.txt".as_slice()),
        "S5A-RENAME01: selecting the unstaged destination must leave recreated old.txt untracked"
    );
    let new_oid = String::from_utf8(f.git.run(&["hash-object", "--stdin"], Some(EDITED))).unwrap();
    expected.insert(
        b"new.txt".to_vec(),
        format!("100644 {} 0", new_oid.trim()).into_bytes(),
    );
    assert_eq!(
        actual, expected,
        "only the selected destination's complete index entry may change"
    );
    for (name, bytes) in &working {
        assert_eq!(&std::fs::read(f.git.path.join(name)).unwrap(), bytes);
    }
    let still_renamed = catalog(&f).await;
    assert_eq!(
        still_renamed
            .entries
            .iter()
            .find(|e| e.path_display == "new.txt")
            .unwrap()
            .index_status,
        PathStatus::R,
        "the pair oracle requires Git to retain index-rename classification after staging the edit"
    );
    let p = prepared(&f, &["new.txt"], ChangeSide::Staged).await;
    success(&execute(&f, p).await);
    expected.remove(b"new.txt".as_slice());
    let old_oid = String::from_utf8(f.git.run(&["rev-parse", "HEAD:old.txt"], None)).unwrap();
    expected.insert(
        b"old.txt".to_vec(),
        format!("100644 {} 0", old_oid.trim()).into_bytes(),
    );
    assert_eq!(
        entries(f.git.run(&["ls-files", "--stage", "-z"], None)),
        expected,
        "unstaging an index rename must restore the HEAD pair only in the index"
    );
    for (name, bytes) in &working {
        assert_eq!(&std::fs::read(f.git.path.join(name)).unwrap(), bytes);
    }
}
