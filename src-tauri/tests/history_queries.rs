#[allow(dead_code)]
mod support;
use opengit_lib::domain::contracts::*;
use support::history::*;
#[tokio::test]
async fn sh01_depth_five_octopus_650_captured_raw_parent_edges_never_point_backwards() {
    let fixture = Fixture::shallow_native_shape("sha1", 650);
    let shallow = fixture.owner.child("shallow-five").unwrap();
    let uri = url::Url::from_file_path(std::path::Path::new(&fixture.git_path(&fixture.path)))
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
    let (backend, scope) = fixture.backend_at(&shallow, "main").await;
    let session = open(&backend, &scope, 200, filter()).await;
    let original = fixture.order_at(&shallow, &session.roots);
    let oracle = fixture.raw_parent_oracle_at(&shallow, &original);
    let captured: std::collections::BTreeSet<_> =
        session.page.commits.iter().map(|c| c.oid.clone()).collect();
    assert_eq!(captured, original.iter().cloned().collect());
    assert_dag(&session.page.commits, &oracle);
    let positions: std::collections::HashMap<_, _> = session
        .page
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid.as_str(), i))
        .collect();
    let mut backwards = Vec::new();
    for (i, commit) in session.page.commits.iter().enumerate() {
        for parent in &commit.parents {
            if let Some(rank) = positions.get(parent.oid.as_str()) {
                if *rank <= i {
                    backwards.push((i, *rank));
                }
            }
        }
    }
    println!(
        "SH01 captured={} backwards={backwards:?}",
        session.page.commits.len()
    );
    assert_eq!(session.page.commits.len(), 13);
    assert!(
        backwards.is_empty(),
        "captured raw-parent edge precedes its child"
    );
}
#[tokio::test]
async fn sh01_sha1_sha256_depth_two_five_eight_preserve_raw_dag_sets_filters_search_and_readonly_state(
) {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::shallow_native_shape(format, 40);
        for depth in [2, 5, 8] {
            let shallow = fixture.owner.child(&format!("shallow-{depth}")).unwrap();
            let uri =
                url::Url::from_file_path(std::path::Path::new(&fixture.git_path(&fixture.path)))
                    .unwrap()
                    .to_string();
            fixture.run(
                &[
                    "clone",
                    "--depth",
                    &depth.to_string(),
                    "--no-local",
                    "--",
                    &uri,
                    &fixture.git_path(&shallow),
                ],
                None,
            );
            fixture.run_at(
                &shallow,
                &["update-ref", "refs/heads/alpha", &fixture.oid("alpha")],
                None,
            );
            fixture.run_at(
                &shallow,
                &["update-ref", "refs/heads/beta", &fixture.oid("beta")],
                None,
            );
            std::fs::write(shallow.join("sentinel"), b"unchanged").unwrap();
            let before = fixture.snapshot_at(&shallow);
            let shallow_bytes = std::fs::read(shallow.join(".git/shallow")).unwrap();
            let cuts = String::from_utf8(shallow_bytes.clone()).unwrap();
            let (backend, scope) = fixture.backend_at(&shallow, "main").await;
            let baseline = open(&backend, &scope, 200, filter()).await;
            let original = fixture.order_at(&shallow, &baseline.roots);
            let oracle = fixture.raw_parent_oracle_at(&shallow, &original);
            let expected_set: std::collections::BTreeSet<_> = original.iter().cloned().collect();
            for size in [1, 2, 200] {
                let session = open(&backend, &scope, size, filter()).await;
                assert_eq!(session.roots, baseline.roots);
                let all = collect(&backend, &session, size).await;
                assert_dag(&all, &oracle);
                assert_eq!(
                    all.iter()
                        .map(|c| c.oid.clone())
                        .collect::<std::collections::BTreeSet<_>>(),
                    expected_set
                );
                assert!(all
                    .iter()
                    .filter(|c| cuts.lines().any(|id| id == c.oid))
                    .flat_map(|c| &c.parents)
                    .all(|p| p.boundary == ParentBoundary::Shallow));
                if size == 1 {
                    let request = LoadHistoryPageRequest {
                        context: session.context.clone(),
                        session_id: session.session_id.clone(),
                        cursor: session.page.next_cursor.clone().unwrap(),
                        page_size: Some(1),
                    };
                    assert_eq!(
                        backend
                            .load_history_page("main", request.clone())
                            .await
                            .unwrap(),
                        backend.load_history_page("main", request).await.unwrap()
                    );
                }
            }
            let solo = open(
                &backend,
                &scope,
                200,
                HistoryFilter {
                    hidden_refs: vec![],
                    solo_refs: vec!["refs/heads/alpha".into()],
                },
            )
            .await;
            let solo_original = fixture.order_at(&shallow, &solo.roots);
            assert_dag(
                &solo.page.commits,
                &fixture.raw_parent_oracle_at(&shallow, &solo_original),
            );
            let outside = backend
                .search_history(
                    "main",
                    SearchHistoryRequest {
                        context: solo.context.clone(),
                        session_id: solo.session_id.clone(),
                        query: fixture.oid("beta"),
                        cursor: None,
                        page_size: Some(1),
                    },
                )
                .await
                .unwrap();
            assert_eq!(outside.commits.len(), 1);
            assert!(outside.commits[0].outside_view);
            let mut cursor = None;
            let mut searched = Vec::new();
            let mut replayed = false;
            loop {
                let request = SearchHistoryRequest {
                    context: solo.context.clone(),
                    session_id: solo.session_id.clone(),
                    query: "Fixture".into(),
                    cursor: cursor.clone(),
                    page_size: Some(3),
                };
                let page = backend
                    .search_history("main", request.clone())
                    .await
                    .unwrap();
                if cursor.is_some() && !replayed {
                    assert_eq!(page, backend.search_history("main", request).await.unwrap());
                    replayed = true;
                }
                searched.extend(page.commits.into_iter().map(|c| c.commit));
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
            assert_dag(&searched, &oracle);
            let hidden = open(
                &backend,
                &scope,
                200,
                HistoryFilter {
                    hidden_refs: vec!["refs/heads/beta".into()],
                    solo_refs: vec!["refs/heads/main".into()],
                },
            )
            .await;
            assert_dag(&hidden.page.commits, &oracle);
            assert!(hidden
                .page
                .commits
                .iter()
                .all(|c| !c.decorations.contains(&"refs/heads/beta".into())));
            assert_eq!(
                std::fs::read(shallow.join(".git/shallow")).unwrap(),
                shallow_bytes
            );
            assert_eq!(
                std::fs::read(shallow.join("sentinel")).unwrap(),
                b"unchanged"
            );
            assert_eq!(fixture.snapshot_at(&shallow), before);
        }
    }
}

#[tokio::test]
async fn sha1_and_sha256_topological_pages_match_git_raw_parent_oracles_without_writes() {
    for format in ["sha1", "sha256"] {
        let fixture = Fixture::new(format, 40);
        let before = fixture.snapshot();
        let oracle = fixture.oracle(&["--all"]);
        let (backend, scope) = fixture.backend().await;
        for size in [1, 2, 200] {
            let session = open(&backend, &scope, size, filter()).await;
            assert_eq!(session.context.scope, scope);
            assert_eq!(session.page.context, session.context);
            assert_eq!(
                session.page.commits[0].oid.len(),
                if format == "sha1" { 40 } else { 64 }
            );
            let commits = collect(&backend, &session, size).await;
            assert_dag(&commits, &oracle);
            assert!(commits.iter().any(|c| c.parents.len() == 3));
            assert_eq!(commits.iter().filter(|c| c.parents.is_empty()).count(), 2);
            assert!(session
                .refs
                .iter()
                .any(|r| r.name == "refs/remotes/origin/HEAD"
                    && r.symbolic_target.as_deref() == Some("refs/remotes/origin/main")));
            assert!(session
                .refs
                .iter()
                .any(|r| r.name == "refs/tags/v1" && r.commit_oid.as_ref() != Some(&r.oid)));
            assert!(session
                .refs
                .iter()
                .any(|r| r.name == "refs/tags/blob" && r.commit_oid.is_none()));
            if size == 1 {
                assert!(session
                    .page
                    .commits
                    .iter()
                    .flat_map(|c| &c.parents)
                    .any(|p| p.boundary == ParentBoundary::OffPage));
            }
        }
        assert_eq!(fixture.snapshot(), before);
    }
}
#[tokio::test]
async fn hide_and_multi_solo_use_positive_union_roots_and_search_all_history_and_authors() {
    let fixture = Fixture::new("sha1", 60);
    let before = fixture.snapshot();
    let (backend, scope) = fixture.backend().await;
    let session = open(
        &backend,
        &scope,
        2,
        HistoryFilter {
            hidden_refs: vec!["refs/heads/alpha".into()],
            solo_refs: vec!["refs/heads/main".into()],
        },
    )
    .await;
    let commits = collect(&backend, &session, 2).await;
    assert_dag(&commits, &fixture.oracle(&["main"]));
    assert!(
        commits.iter().any(|c| c.oid == fixture.oid("alpha")),
        "hidden tip stays reachable as shared merge ancestry"
    );
    assert!(commits
        .iter()
        .all(|c| !c.decorations.contains(&"refs/heads/alpha".into())));
    assert!(!commits.iter().any(|c| c.oid == fixture.oid("unmerged")));
    for (query, expected, outside) in [
        ("UnmergedNeedle", fixture.oid("unmerged"), true),
        ("DeepNeedle", fixture.oid("main~60"), false),
        ("AuthorNeedle", fixture.oid("main~59"), false),
        (
            "AuthorNeedle <fixture@example.invalid>",
            fixture.oid("main~59"),
            false,
        ),
        ("[.*]", fixture.oid("main~60"), false),
    ] {
        let result = backend
            .search_history(
                "main",
                SearchHistoryRequest {
                    context: session.context.clone(),
                    session_id: session.session_id.clone(),
                    query: query.into(),
                    cursor: None,
                    page_size: Some(100),
                },
            )
            .await
            .unwrap();
        assert!(
            result
                .commits
                .iter()
                .any(|hit| hit.commit.oid == expected && hit.outside_view == outside),
            "full-history literal match: {query}"
        );
    }
    let prefix = fixture.oid("unmerged")[..12].to_owned();
    let found = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: prefix,
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(found.commits.len(), 1);
    assert!(found.commits[0].outside_view);
    let reference = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: "origin/HEAD".into(),
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(reference.refs.len(), 1);
    let union = open(
        &backend,
        &scope,
        200,
        HistoryFilter {
            hidden_refs: vec![],
            solo_refs: vec!["refs/heads/unmerged".into(), "refs/heads/alpha".into()],
        },
    )
    .await;
    assert_dag(
        &collect(&backend, &union, 200).await,
        &fixture.oracle(&["unmerged", "alpha"]),
    );
    let empty = open(
        &backend,
        &scope,
        200,
        HistoryFilter {
            hidden_refs: vec!["refs/heads/unmerged".into()],
            solo_refs: vec!["refs/heads/unmerged".into()],
        },
    )
    .await;
    assert!(empty.page.commits.is_empty());
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn opaque_page_and_query_cursors_are_replayable_and_bound_to_window_scope_generation_size_and_snapshot(
) {
    let fixture = Fixture::new("sha1", 20);
    let (backend, scope) = fixture.backend().await;
    let session = open(&backend, &scope, 2, filter()).await;
    let request = LoadHistoryPageRequest {
        context: session.context.clone(),
        session_id: session.session_id.clone(),
        cursor: session.page.next_cursor.clone().unwrap(),
        page_size: Some(2),
    };
    let first = backend
        .load_history_page("main", request.clone())
        .await
        .unwrap();
    let replay = backend
        .load_history_page("main", request.clone())
        .await
        .unwrap();
    assert_eq!(first, replay);
    backend.get_workspace("other").unwrap();
    assert_eq!(
        backend
            .load_history_page("other", request.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    for bad in [
        LoadHistoryPageRequest {
            page_size: Some(1),
            ..request.clone()
        },
        LoadHistoryPageRequest {
            context: HistoryContext {
                snapshot_id: "forged".into(),
                ..request.context.clone()
            },
            ..request.clone()
        },
        LoadHistoryPageRequest {
            cursor: "--all".into(),
            ..request.clone()
        },
    ] {
        assert_eq!(
            backend
                .load_history_page("main", bad)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
    }
    let search = SearchHistoryRequest {
        context: session.context.clone(),
        session_id: session.session_id.clone(),
        query: "History".into(),
        cursor: None,
        page_size: Some(2),
    };
    let result = backend
        .search_history("main", search.clone())
        .await
        .unwrap();
    let continuation = SearchHistoryRequest {
        cursor: result.next_cursor.clone(),
        ..search.clone()
    };
    assert_eq!(
        backend
            .search_history("main", continuation.clone())
            .await
            .unwrap(),
        backend
            .search_history("main", continuation.clone())
            .await
            .unwrap()
    );
    assert_eq!(
        backend
            .search_history(
                "main",
                SearchHistoryRequest {
                    query: "different".into(),
                    ..continuation.clone()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        backend
            .load_history_page(
                "main",
                LoadHistoryPageRequest {
                    cursor: result.next_cursor.unwrap(),
                    ..request.clone()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    let newer = HistoryScope {
        generation: "2".into(),
        ..scope.clone()
    };
    open(&backend, &newer, 2, filter()).await;
    assert_eq!(
        backend
            .load_history_page("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
    assert_eq!(
        backend
            .open_history(
                "main",
                OpenHistoryRequest {
                    scope,
                    filter: filter(),
                    page_size: None
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidHandle
    );
}

#[tokio::test]
async fn nested_commit_tags_stashes_and_literal_unicode_newlines_are_searchable_without_filter_changes(
) {
    let fixture = Fixture::new("sha1", 8);
    fixture.run(
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "tag",
            "-a",
            "inner",
            "-m",
            "inner",
            "orphan",
        ],
        None,
    );
    fixture.run(
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "tag",
            "-a",
            "outer",
            "-m",
            "outer",
            "inner",
        ],
        None,
    );
    fixture.run(&["update-ref", "-d", "refs/tags/inner"], None);
    fixture.run(&["update-ref", "-d", "refs/heads/orphan"], None);
    let tree = fixture.oid("main^{tree}");
    let message = "Unicode Ωmega\nAcross\nline needle\n";
    let raw = format!("tree {tree}\nauthor Unicode Ωuthor <unicode@example.invalid> 1700000000 +0000\ncommitter Fixture <fixture@example.invalid> 1700000000 +0000\n\n{message}");
    let stash = String::from_utf8(fixture.run(
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(raw.as_bytes()),
    ))
    .unwrap()
    .trim()
    .to_owned();
    fixture.run(&["update-ref", "refs/stash", &stash], None);
    let before = fixture.snapshot();
    let (backend, scope) = fixture.backend().await;
    let session = open(&backend, &scope, 200, filter()).await;
    assert!(session.refs.iter().any(|r| r.name == "refs/tags/outer"
        && r.commit_oid.as_deref() == Some(&fixture.oid("outer^{}"))));
    assert!(session.refs.iter().any(|r| r.kind == HistoryRefKind::Stash));
    assert!(!session.roots.contains(&stash));
    assert!(session.roots.contains(&fixture.oid("outer^{}")));
    for query in ["ωMEGA", "Across\nline", "ωUTHOR"] {
        let found = backend
            .search_history(
                "main",
                SearchHistoryRequest {
                    context: session.context.clone(),
                    session_id: session.session_id.clone(),
                    query: query.into(),
                    cursor: None,
                    page_size: None,
                },
            )
            .await
            .unwrap();
        assert!(found
            .commits
            .iter()
            .any(|c| c.commit.oid == stash && c.outside_view));
    }
    let focused = open(
        &backend,
        &scope,
        200,
        HistoryFilter {
            hidden_refs: vec![],
            solo_refs: vec!["refs/stash".into()],
        },
    )
    .await;
    assert_eq!(focused.page.commits.len(), 1);
    assert_eq!(focused.page.commits[0].oid, stash);
    assert_eq!(fixture.snapshot(), before);
}
#[tokio::test]
async fn ten_thousand_commit_search_reaches_deep_history_in_size_bounded_raw_batches() {
    let fixture = Fixture::new("sha1", 10_000);
    let before = fixture.snapshot();
    let (backend, scope) = fixture.backend().await;
    let opened_at = std::time::Instant::now();
    let session = open(&backend, &scope, 200, filter()).await;
    let open_ms = opened_at.elapsed().as_millis();
    let searched_at = std::time::Instant::now();
    let found = backend
        .search_history(
            "main",
            SearchHistoryRequest {
                context: session.context.clone(),
                session_id: session.session_id.clone(),
                query: "DeepNeedle".into(),
                cursor: None,
                page_size: None,
            },
        )
        .await
        .unwrap();
    let search_ms = searched_at.elapsed().as_millis();
    assert_eq!(found.commits.len(), 1);
    assert!(!session
        .page
        .commits
        .iter()
        .any(|c| c.oid == found.commits[0].commit.oid));
    assert_eq!(fixture.snapshot(), before);
    println!("METRIC actual backend history commits=10005 open_ms={open_ms} deep_literal_search_ms={search_ms} raw_batch_max=4MiB metadata_chunk_max=4096");
}
fn two_stashes(fixture: &Fixture) -> (String, String) {
    let tracked = fixture.path.join("tracked.txt");
    std::fs::write(&tracked, b"base").unwrap();
    fixture.run(&["add", "--", "tracked.txt"], None);
    fixture.run(&["commit", "-m", "Tracked base"], None);
    std::fs::write(&tracked, b"older change").unwrap();
    fixture.run(&["stash", "push", "-m", "StashNeedleOlder"], None);
    let older = fixture.oid("refs/stash");
    std::fs::write(&tracked, b"latest change").unwrap();
    fixture.run(&["stash", "push", "-m", "StashNeedleLatest"], None);
    (older, fixture.oid("refs/stash"))
}
#[tokio::test]
async fn full_stash_stack_is_listed_searchable_and_soloable_for_files_and_reftable() {
    for ref_format in ["files", "reftable"] {
        println!("stash storage={ref_format}");
        let fixture = Fixture::with_ref_format("sha1", 8, ref_format);
        let (older, latest) = two_stashes(&fixture);
        let before = fixture.snapshot();
        let (backend, scope) = fixture.backend().await;
        println!("stash open default");
        let session = open(&backend, &scope, 200, filter()).await;
        let stashes: Vec<_> = session
            .refs
            .iter()
            .filter(|r| r.kind == HistoryRefKind::Stash)
            .collect();
        assert_eq!(stashes.len(), 2);
        assert_eq!(stashes[0].name, "refs/stash");
        assert_eq!(stashes[1].name, "refs/stash/1");
        assert!(stashes[0].display_name.contains("stash@{0}:"));
        assert!(stashes[1].display_name.contains("stash@{1}:"));
        for oid in [&older, &latest] {
            assert!(!session.roots.contains(oid));
            assert!(!session.page.commits.iter().any(|c| &c.oid == oid));
            let found = backend
                .search_history(
                    "main",
                    SearchHistoryRequest {
                        context: session.context.clone(),
                        session_id: session.session_id.clone(),
                        query: oid[..12].into(),
                        cursor: None,
                        page_size: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(found.commits.len(), 1);
            assert_eq!(&found.commits[0].commit.oid, oid);
            assert!(found.commits[0].outside_view);
            assert_eq!(found.commits[0].commit.handle.len(), 64);
        }
        for (keys, revisions) in [
            (vec!["refs/stash/1"], vec!["refs/stash@{1}"]),
            (
                vec!["refs/stash", "refs/stash/1"],
                vec!["refs/stash", "refs/stash@{1}"],
            ),
        ] {
            println!("stash Solo={keys:?}");
            let selected = open(
                &backend,
                &scope,
                200,
                HistoryFilter {
                    hidden_refs: vec![],
                    solo_refs: keys.into_iter().map(str::to_owned).collect(),
                },
            )
            .await;
            assert_dag(&selected.page.commits, &fixture.oracle(&revisions));
            assert!(selected
                .page
                .commits
                .iter()
                .any(|c| c.oid == older && c.parents.len() == 2));
        }
        assert_eq!(fixture.snapshot(), before);
        let paged = open(&backend, &scope, 1, filter()).await;
        fixture.run(&["reflog", "delete", "--rewrite", "refs/stash@{1}"], None);
        assert_eq!(fixture.oid("refs/stash"), latest);
        let request = LoadHistoryPageRequest {
            context: paged.context,
            session_id: paged.session_id,
            cursor: paged.page.next_cursor.unwrap(),
            page_size: Some(1),
        };
        assert_eq!(
            backend
                .load_history_page("main", request.clone())
                .await
                .unwrap_err()
                .code,
            ErrorCode::ExternalChange
        );
        assert_eq!(
            backend
                .load_history_page("main", request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidHandle
        );
    }
}
#[tokio::test]
async fn missing_older_stash_object_is_an_explicit_error_and_stack_cap_never_truncates() {
    let fixture = Fixture::new("sha1", 8);
    let (older, _) = two_stashes(&fixture);
    let path = fixture
        .path
        .join(".git/objects")
        .join(&older[..2])
        .join(&older[2..]);
    fixture.owner.require_owned_file(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let (backend, scope) = fixture.backend().await;
    let request = OpenHistoryRequest {
        scope,
        filter: filter(),
        page_size: Some(1),
    };
    assert_eq!(
        backend
            .open_history("main", request.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    std::fs::write(&path, bytes).unwrap();
    let log = fixture.path.join(".git/logs/refs/stash");
    fixture.owner.require_owned_file(&log).unwrap();
    let original = std::fs::read(&log).unwrap();
    let line = original
        .split_inclusive(|b| *b == b'\n')
        .next_back()
        .unwrap();
    std::fs::write(&log, line.repeat(20_001)).unwrap();
    assert_eq!(
        backend
            .open_history("main", request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::OutputLimit
    );
}
#[tokio::test]
async fn traversal_above_one_hundred_thousand_returns_output_limit_instead_of_truncating() {
    let fixture = Fixture::new("sha1", 100_001);
    let before = fixture.snapshot();
    let (backend, scope) = fixture.backend().await;
    let result = backend
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
    assert_eq!(result.code, ErrorCode::OutputLimit);
    assert_eq!(fixture.snapshot(), before);
}
