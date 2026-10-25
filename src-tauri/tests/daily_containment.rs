#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
use support::history::{filter, open};
#[tokio::test]
async fn selected_offpage_hidden_commit_lists_every_containing_branch_and_current_upstream() {
    for format in ["sha1", "sha256"] {
        let f = support::daily::Fixture::new(format, 230, "files").await;
        let root = f.git.oid("main~229");
        f.git
            .run(&["update-ref", "refs/heads/same-tip", "main"], None);
        f.git
            .run(&["update-ref", "refs/remotes/origin/other", "main"], None);
        let before = f.git.snapshot();
        let session = open(
            &f.backend,
            &f.history_scope,
            1,
            HistoryFilter {
                hidden_refs: vec!["refs/heads/main".into()],
                solo_refs: vec!["refs/heads/orphan".into()],
            },
        )
        .await;
        assert_eq!(session.page.commits.len(), 1);
        let hit = f
            .backend
            .search_history(
                "main",
                SearchHistoryRequest {
                    context: session.context.clone(),
                    session_id: session.session_id.clone(),
                    query: root.clone(),
                    cursor: None,
                    page_size: Some(1),
                },
            )
            .await
            .unwrap();
        assert!(hit.commits[0].outside_view);
        let selected = &hit.commits[0].commit;
        let result = f
            .backend
            .get_commit_containment(
                "main",
                GetCommitContainmentRequest {
                    context: session.context.clone(),
                    commit_handle: selected.handle.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(result.oid, selected.oid);
        assert_eq!(result.completeness, AncestryCompleteness::Complete);
        assert_eq!(result.current_branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(result.upstream.as_deref(), Some("refs/remotes/origin/main"));
        assert!(!result
            .branches
            .iter()
            .any(|b| b.branch.name == "refs/remotes/origin/HEAD"));
        let names = String::from_utf8(f.git.run(
            &[
                "for-each-ref",
                &format!("--contains={}", selected.oid),
                "--format=%(refname)",
                "refs/heads/",
                "refs/remotes/",
            ],
            None,
        ))
        .unwrap();
        for branch in &result.branches {
            assert_eq!(
                branch.membership == Membership::Contains,
                names.lines().any(|n| n == branch.branch.name)
            );
        }
        assert!(
            result
                .branches
                .iter()
                .filter(|b| b.membership == Membership::Contains)
                .count()
                >= 4
        );
        let catalog = f.catalog().await;
        assert_eq!(catalog.sync.ahead.as_deref(), Some("0"));
        assert_eq!(catalog.sync.behind.as_deref(), Some("0"));
        assert_eq!(
            catalog.symbolic_branches[0].name,
            "refs/remotes/origin/HEAD"
        );
        assert_eq!(
            catalog.remotes[0].fetch,
            FetchCapability::EligibleProbeRequired
        );
        let again = f.catalog().await;
        assert_eq!(catalog.remotes[0].handle, again.remotes[0].handle);
        assert_eq!(f.git.snapshot(), before);
        let forged = f
            .backend
            .get_commit_containment(
                "main",
                GetCommitContainmentRequest {
                    context: session.context.clone(),
                    commit_handle: selected.oid.clone(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(forged.code, ErrorCode::InvalidHandle);
        f.git
            .run(&["update-ref", "refs/heads/new-external", "main"], None);
        let stale = f
            .backend
            .get_commit_containment(
                "main",
                GetCommitContainmentRequest {
                    context: session.context,
                    commit_handle: selected.handle.clone(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(stale.code, ErrorCode::ExternalChange);
    }
}
#[tokio::test]
async fn shallow_negatives_are_unknown_and_partial_catalog_does_not_require_ancestry() {
    let f = support::daily::Fixture::new("sha1", 20, "files").await;
    let shallow = f.git.owner.child("daily shallow").unwrap();
    let uri = url::Url::from_file_path(std::path::Path::new(&f.git.git_path(&f.git.path)))
        .unwrap()
        .to_string();
    f.git.run(
        &[
            "clone",
            "--depth",
            "2",
            "--no-single-branch",
            "--no-local",
            "--",
            &uri,
            &f.git.git_path(&shallow),
        ],
        None,
    );
    let (backend, scope) = f.git.backend_at(&shallow, "main").await;
    let session = open(&backend, &scope, 2, filter()).await;
    let selected = &session.page.commits[0];
    let result = backend
        .get_commit_containment(
            "main",
            GetCommitContainmentRequest {
                context: session.context,
                commit_handle: selected.handle.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(result.completeness, AncestryCompleteness::Shallow);
    assert!(result
        .branches
        .iter()
        .all(|b| b.membership != Membership::DoesNotContain));
    assert!(result
        .branches
        .iter()
        .any(|b| b.membership == Membership::Unknown));
    drop(backend);
    f.git
        .run(&["config", "remote.origin.promisor", "true"], None);
    let catalog = f.catalog().await;
    assert_eq!(catalog.sync.completeness, AncestryCompleteness::Unavailable);
    assert!(catalog.sync.ahead.is_none());
    assert_eq!(
        catalog.remotes[0].fetch,
        FetchCapability::EligibleProbeRequired
    );
}
#[tokio::test]
async fn one_shared_generation_allows_overlapping_catalog_membership_but_switch_revokes_both() {
    let mut f = support::daily::Fixture::new("sha1", 8, "files").await;
    let proxy = support::daily::Proxy::new(&f.git);
    f.rebind(proxy.runner.clone(), None).await;
    let session = open(&f.backend, &f.history_scope, 1, filter()).await;
    let request = GetCommitContainmentRequest {
        context: session.context,
        commit_handle: session.page.commits[0].handle.clone(),
    };
    let (catalog, membership) = tokio::join!(
        f.backend.get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: f.scope.clone()
            }
        ),
        f.backend.get_commit_containment("main", request)
    );
    assert!(catalog.is_ok());
    assert!(membership.is_ok());
    let other = f.git.owner.child("other worktree").unwrap();
    std::fs::create_dir(&other).unwrap();
    f.git.run_at(&other, &["init", "-b", "main"], None);
    let selection = f
        .backend
        .register_repository("main", &other)
        .await
        .unwrap()
        .active_selection
        .unwrap();
    f.backend
        .select_repository(
            "main",
            SelectRepositoryRequest {
                repo_id: f.scope.repo_id.clone(),
                worktree_id: f.scope.worktree_id.clone(),
            },
        )
        .unwrap();
    f.scope.generation = "2".into();
    f.history_scope.generation = "2".into();
    let session = open(&f.backend, &f.history_scope, 1, filter()).await;
    let request = GetCommitContainmentRequest {
        context: session.context,
        commit_handle: session.page.commits[0].handle.clone(),
    };
    proxy.set("cataloghold");
    let backend = f.backend.clone();
    let scope = f.scope.clone();
    let catalog = tokio::spawn(async move {
        backend
            .get_repository_actions("main", GetRepositoryActionsRequest { scope })
            .await
    });
    proxy.wait_ready().await;
    let backend = f.backend.clone();
    let membership =
        tokio::spawn(async move { backend.get_commit_containment("main", request).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    f.backend
        .select_repository(
            "main",
            SelectRepositoryRequest {
                repo_id: selection.repo_id.clone(),
                worktree_id: selection.worktree_id.clone(),
            },
        )
        .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(7), async {
        (catalog.await.unwrap(), membership.await.unwrap())
    })
    .await
    .unwrap();
    assert!(result.0.is_err());
    assert!(result.1.is_err());
    proxy.release();
    proxy.set("normal");
    let scope = ActionScope {
        repo_id: selection.repo_id,
        worktree_id: selection.worktree_id,
        generation: "3".into(),
    };
    assert!(f
        .backend
        .get_repository_actions(
            "main",
            GetRepositoryActionsRequest {
                scope: scope.clone()
            }
        )
        .await
        .is_ok());
    assert_eq!(
        f.backend
            .get_repository_actions(
                "main",
                GetRepositoryActionsRequest {
                    scope: f.scope.clone()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert!(f
        .backend
        .get_repository_actions("main", GetRepositoryActionsRequest { scope })
        .await
        .is_ok());
}
#[tokio::test]
async fn a_parent_object_removed_after_history_capture_never_becomes_a_definitive_negative() {
    let f = support::daily::Fixture::new("sha1", 8, "files").await;
    let original = f.git.oid("main");
    let tree = f.git.oid("main^{tree}");
    let make = |parent: &str, message: &str| {
        let raw=format!("tree {tree}\nparent {parent}\nauthor Fixture <fixture@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\n{message}\n");
        String::from_utf8(f.git.run(
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            Some(raw.as_bytes()),
        ))
        .unwrap()
        .trim()
        .to_owned()
    };
    let middle = make(&original, "Missing intermediate fixture");
    let child = make(&middle, "Captured child fixture");
    f.git.run(&["update-ref", "refs/heads/main", &child], None);
    let session = open(&f.backend, &f.history_scope, 100, filter()).await;
    let selected = session
        .page
        .commits
        .iter()
        .find(|c| c.oid == original)
        .unwrap();
    let object = f
        .git
        .path
        .join(".git/objects")
        .join(&middle[..2])
        .join(&middle[2..]);
    f.git.owner.require_owned_file(&object).unwrap();
    let saved = std::fs::read(&object).unwrap();
    std::fs::remove_file(&object).unwrap();
    let before = support::daily::metadata_tree(&f.git.path);
    let result = f
        .backend
        .get_commit_containment(
            "main",
            GetCommitContainmentRequest {
                context: session.context,
                commit_handle: selected.handle.clone(),
            },
        )
        .await;
    assert!(
        result.is_err(),
        "incomplete ancestry must not publish doesNotContain"
    );
    assert_eq!(support::daily::metadata_tree(&f.git.path), before);
    std::fs::write(object, saved).unwrap();
}
