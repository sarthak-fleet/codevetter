use super::api::{
    automatic_release_checkpoint_revisions, fast_forward_delta_pairs, normalized_facts_are_current,
    HistoryFactCatalogProbe,
};
use super::*;
use crate::commands::history_read::{contributors::HistoryContributorScope, HistoryReadService};
use std::{collections::HashSet, fs};

#[test]
fn timeline_is_stable_and_release_aware() {
    let root = std::env::temp_dir().join(format!("cv-history-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("src")).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("src/a.rs"), "fn a() {}\n").expect("a");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: first"]);
    run_git(&root, &["tag", "v1.0.0"]);
    fs::write(root.join("src/b.rs"), "fn b() {}\n").expect("b");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: second"]);

    let timeline = build_timeline(&root, Some(20)).expect("timeline");
    assert_eq!(timeline.revisions.len(), 2);
    assert!(timeline.revisions[0].is_release);
    assert!(timeline.revisions[1].is_head);
    assert!(!timeline.is_shallow);
    assert!(timeline.coverage_complete);
    assert_eq!(timeline.release_ranges.len(), 2);
    assert_eq!(timeline.release_ranges[0].tag.as_deref(), Some("v1.0.0"));
    assert!(timeline.release_ranges[1].is_unreleased);
    assert_eq!(timeline.release_ranges[1].commit_shas.len(), 1);
    assert_eq!(
        resolve_temporal_reference(
            &root,
            &HistoryTemporalReference::Release {
                tag: "v1.0.0".to_string(),
            },
        )
        .expect("release reference"),
        timeline.revisions[0].sha
    );
    fs::write(root.join("src/a.rs"), "fn worktree_only() {}\n").expect("dirty worktree");
    let blobs = GitObjectReader::new(&root)
        .blobs_at(&timeline.revisions[0].sha)
        .expect("historical blobs");
    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs[0].path, "src/a.rs");
    assert!(String::from_utf8_lossy(&blobs[0].bytes).contains("fn a"));
    assert!(!String::from_utf8_lossy(&blobs[0].bytes).contains("worktree_only"));
    let historical_snapshot = build_snapshot_from_blobs(
        &history_storage_key(&timeline.repo_path),
        &timeline.revisions[0].sha,
        blobs,
        &StructuralGraphCancellation::default(),
        &|_: StructuralGraphProgress| {},
    )
    .expect("historical structural snapshot");
    assert!(historical_snapshot
        .nodes
        .iter()
        .any(|node| node.label == "a"));
    assert!(!historical_snapshot
        .nodes
        .iter()
        .any(|node| node.label == "worktree_only" || node.label == "b"));
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &timeline).expect("persist timeline");
    let revision_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM history_graph_revisions", [], |row| {
            row.get(0)
        })
        .expect("revision count");
    assert_eq!(revision_count, 2);
    let event_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM history_graph_events", [], |row| {
            row.get(0)
        })
        .expect("history event count");
    assert_eq!(
        event_count, 4,
        "commits, release, and coverage are ledger events"
    );
    let releases = load_history_revisions(&connection, &timeline.repo_path, None, true, 10)
        .expect("release query");
    assert_eq!(releases.revisions.len(), 1);
    let search =
        load_history_revisions(&connection, &timeline.repo_path, Some("second"), false, 10)
            .expect("history search");
    assert_eq!(search.revisions[0].subject, "feat: second");
    run_git(&root, &["tag", "v1.1.0"]);
    let retagged = build_timeline(&root, Some(20)).expect("retagged timeline");
    persist_timeline(&connection, &retagged).expect("persist retagged timeline");
    let invalidations: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_events WHERE event_kind = 'invalidation'",
            [],
            |row| row.get(0),
        )
        .expect("invalidation count");
    assert_eq!(invalidations, 1);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn fast_forward_and_tag_only_refreshes_reuse_indexed_facts() {
    let root =
        std::env::temp_dir().join(format!("cv-history-incremental-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("src")).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("src/lib.rs"), "pub fn first() {}\n").expect("first source");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: first"]);
    fs::write(root.join("src/lib.rs"), "pub fn second() {}\n").expect("second source");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: second"]);

    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let initial_tags = read_git_tags(&root).expect("initial tags");
    let initial = build_timeline_bundle_with_tags_cancellable(
        &root,
        Some(20),
        &initial_tags,
        &StructuralGraphCancellation::default(),
    )
    .expect("initial facts");
    persist_timeline(&connection, &initial.timeline).expect("persist initial timeline");
    {
        let publication = connection
            .unchecked_transaction()
            .expect("fact publication");
        publish_history_facts(
            &publication,
            &initial,
            &initial_tags,
            "initial",
            &StructuralGraphCancellation::default(),
        )
        .expect("publish initial facts");
        publication.commit().expect("commit initial facts");
    }

    fs::write(root.join("src/lib.rs"), "pub fn third() {}\n").expect("third source");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: third"]);
    let current_tags = read_git_tags(&root).expect("current tags");
    let (incremental, introduced) =
        catalog::build_incremental_timeline_bundle_with_tags_cancellable(
            &connection,
            &root,
            Some(20),
            &current_tags,
            &initial.timeline.head,
            &StructuralGraphCancellation::default(),
        )
        .expect("incremental timeline");
    let clean = build_timeline_bundle_with_tags_cancellable(
        &root,
        Some(20),
        &current_tags,
        &StructuralGraphCancellation::default(),
    )
    .expect("clean timeline");

    assert_eq!(incremental.timeline.revisions, clean.timeline.revisions);
    assert_eq!(
        incremental.timeline.reachable_revisions,
        clean.timeline.reachable_revisions
    );
    assert_eq!(
        incremental.timeline.release_ranges,
        clean.timeline.release_ranges
    );
    persist_timeline_catalog(&connection, &incremental.timeline)
        .expect("stage incremental timeline catalog");
    {
        let publication = connection
            .unchecked_transaction()
            .expect("incremental fact publication");
        publish_incremental_history_facts(
            &publication,
            &incremental,
            &current_tags,
            "incremental",
            &StructuralGraphCancellation::default(),
            &introduced,
        )
        .expect("publish only introduced facts");
        publication.commit().expect("commit incremental facts");
    }
    let path_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_revision_paths WHERE repo_path = ?1",
            [&incremental.timeline.repo_path],
            |row| row.get(0),
        )
        .expect("incremental path count");
    let primary_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_revision_contributors
             WHERE repo_path = ?1 AND role = 'primary'",
            [&incremental.timeline.repo_path],
            |row| row.get(0),
        )
        .expect("incremental primary count");
    assert_eq!(path_rows, 3);
    assert_eq!(primary_rows, 3);

    run_git(&root, &["tag", "v1.0.0"]);
    let retagged = build_indexed_timeline_bundle_with_tags(
        &connection,
        &root,
        Some(20),
        &read_git_tags(&root).expect("retagged tags"),
        &incremental.timeline.head,
    )
    .expect("tag-only indexed timeline");
    persist_timeline_catalog(&connection, &retagged.timeline)
        .expect("stage tag-only timeline catalog");
    {
        let publication = connection
            .unchecked_transaction()
            .expect("tag-only fact publication");
        publish_incremental_history_facts(
            &publication,
            &retagged,
            &read_git_tags(&root).expect("tag-only tags"),
            "tag-only",
            &StructuralGraphCancellation::default(),
            &HashSet::new(),
        )
        .expect("publish tag-only metadata");
        publication.commit().expect("commit tag-only metadata");
    }
    let tag_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_fact_tags WHERE repo_path = ?1",
            [&retagged.timeline.repo_path],
            |row| row.get(0),
        )
        .expect("tag-only tag count");
    let retained_path_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_revision_paths WHERE repo_path = ?1",
            [&retagged.timeline.repo_path],
            |row| row.get(0),
        )
        .expect("tag-only path count");
    assert_eq!(tag_rows, 1);
    assert_eq!(retained_path_rows, path_rows);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn fast_forward_structural_deltas_stay_within_new_bounded_revisions() {
    let a = "a".repeat(40);
    let b = "b".repeat(40);
    let c = "c".repeat(40);
    let timeline = HistoryTimeline {
        schema_version: 1,
        repo_path: "/fixture".to_string(),
        head: c.clone(),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        revisions: vec![
            HistoryRevision {
                sha: a.clone(),
                short_sha: "aaaaaaaa".to_string(),
                parents: Vec::new(),
                committed_at: "2026-01-01T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "base".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: false,
                ordinal: 0,
            },
            HistoryRevision {
                sha: b.clone(),
                short_sha: "bbbbbbbb".to_string(),
                parents: vec![a.clone()],
                committed_at: "2026-01-02T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "first append".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: false,
                ordinal: 1,
            },
            HistoryRevision {
                sha: c.clone(),
                short_sha: "cccccccc".to_string(),
                parents: vec![b.clone()],
                committed_at: "2026-01-03T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "second append".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: true,
                ordinal: 2,
            },
        ],
        total_commits: 3,
        truncated: false,
        is_shallow: false,
        coverage_complete: true,
        release_ranges: Vec::new(),
        reachable_revisions: vec![a.clone(), b.clone(), c.clone()],
    };
    let introduced = HashSet::from([b.clone(), c.clone()]);
    assert_eq!(
        fast_forward_delta_pairs(&timeline, &introduced),
        vec![(a, b), ("b".repeat(40), c)]
    );
    assert!(fast_forward_delta_pairs(&timeline, &HashSet::new()).is_empty());
}

#[test]
fn indexed_timeline_reads_normalized_facts_without_git_and_keeps_old_releases_visible() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let timeline = HistoryTimeline {
        schema_version: 1,
        repo_path: "/indexed-history".to_string(),
        head: "c".repeat(40),
        generated_at: "2026-01-01T00:00:00Z".to_string(),
        revisions: vec![
            HistoryRevision {
                sha: "a".repeat(40),
                short_sha: "aaaaaaaa".to_string(),
                parents: Vec::new(),
                committed_at: "2026-01-01T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "old release".to_string(),
                tags: vec!["v1.0.0".to_string()],
                is_release: true,
                is_head: false,
                ordinal: 0,
            },
            HistoryRevision {
                sha: "b".repeat(40),
                short_sha: "bbbbbbbb".to_string(),
                parents: vec!["a".repeat(40)],
                committed_at: "2026-01-02T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "middle".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: false,
                ordinal: 1,
            },
            HistoryRevision {
                sha: "c".repeat(40),
                short_sha: "cccccccc".to_string(),
                parents: vec!["b".repeat(40)],
                committed_at: "2026-01-03T00:00:00Z".to_string(),
                author: "Fixture".to_string(),
                subject: "head".to_string(),
                tags: Vec::new(),
                is_release: false,
                is_head: true,
                ordinal: 2,
            },
        ],
        total_commits: 3,
        truncated: false,
        is_shallow: false,
        coverage_complete: true,
        release_ranges: Vec::new(),
        reachable_revisions: vec!["a".repeat(40), "b".repeat(40), "c".repeat(40)],
    };
    persist_timeline(&connection, &timeline).expect("persist indexed timeline");
    connection
        .execute(
            "INSERT INTO history_graph_fact_catalogs (
                repo_path, schema_version, classification_version, index_identity,
                indexed_head, tags_fingerprint, mailmap_fingerprint, facts_fingerprint,
                status, updated_at
             ) VALUES (?1, 1, 1, 'facts', ?2, 'tags', 'mailmap', 'facts', 'ready', ?3)",
            params![timeline.repo_path, timeline.head, timeline.generated_at],
        )
        .expect("fact catalog");

    let loaded = load_indexed_timeline(&connection, &timeline.repo_path, Some(1))
        .expect("load indexed timeline")
        .expect("indexed timeline");
    assert_eq!(loaded.total_commits, 3);
    assert!(loaded.truncated);
    assert_eq!(loaded.revisions.len(), 2);
    assert_eq!(loaded.revisions[0].sha, "a".repeat(40));
    assert_eq!(loaded.revisions[1].sha, "c".repeat(40));
    assert_eq!(loaded.revisions[0].tags, ["v1.0.0"]);
}

#[test]
fn deterministic_release_fixture_preserves_tag_kinds_and_coincident_old_releases() {
    let root = std::env::temp_dir().join(format!(
        "cv-history-release-contract-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);

    fs::write(root.join("history.txt"), "release 0\n").expect("initial release");
    run_git_at(&root, &["add", "."], "2026-01-01T00:00:00+00:00");
    run_git_at(
        &root,
        &["commit", "-m", "release: initial"],
        "2026-01-01T00:00:00+00:00",
    );
    run_git_at(&root, &["tag", "v0.1.0"], "2026-01-01T01:00:00+00:00");
    run_git_at(
        &root,
        &["tag", "-a", "v0.1.0-stable", "-m", "stable release"],
        "2026-01-01T02:00:00+00:00",
    );

    for index in 1..=4 {
        fs::write(root.join("history.txt"), format!("release {index}\n")).expect("history");
        let timestamp = format!("2026-01-0{}T00:00:00+00:00", index + 1);
        run_git_at(&root, &["add", "."], &timestamp);
        run_git_at(
            &root,
            &["commit", "-m", &format!("change: {index}")],
            &timestamp,
        );
    }

    let tags = read_git_tags(&root).expect("tag facts");
    let lightweight = tags
        .iter()
        .find(|tag| tag.name == "v0.1.0")
        .expect("lightweight tag");
    let annotated = tags
        .iter()
        .find(|tag| tag.name == "v0.1.0-stable")
        .expect("annotated tag");
    assert_eq!(lightweight.object_sha, lightweight.commit_sha);
    assert_ne!(annotated.object_sha, annotated.commit_sha);
    assert_eq!(lightweight.commit_sha, annotated.commit_sha);

    let timeline = build_timeline(&root, Some(2)).expect("bounded timeline");
    assert!(timeline.truncated);
    let old_release = timeline
        .revisions
        .iter()
        .find(|revision| revision.sha == lightweight.commit_sha)
        .expect("old release outside recent window");
    assert_eq!(
        old_release.tags,
        vec!["v0.1.0".to_string(), "v0.1.0-stable".to_string()]
    );
    assert_eq!(
        timeline
            .release_ranges
            .iter()
            .filter(|range| !range.is_unreleased)
            .count(),
        1,
        "the legacy timeline groups coincident tags at one position; the catalog preserves both"
    );

    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &timeline).expect("timeline");
    let initial_fingerprint = release_tag_fingerprint(&tags);
    let initial_identity = publish_catalog(&connection, &timeline, &tags, &initial_fingerprint)
        .expect("initial release catalog");
    let expected_initial = (
        initial_identity.clone(),
        vec![
            (
                "v0.1.0".to_string(),
                "lightweight".to_string(),
                lightweight.commit_sha.clone(),
            ),
            (
                "v0.1.0-stable".to_string(),
                "annotated".to_string(),
                annotated.commit_sha.clone(),
            ),
        ],
    );
    assert_eq!(
        release_catalog_state(&connection, &timeline.repo_path),
        expected_initial
    );

    run_git(&root, &["tag", "-d", "v0.1.0-stable"]);
    let remaining_tags = read_git_tags(&root).expect("remaining tags");
    let remaining_timeline =
        build_timeline_with_tags(&root, Some(2), &remaining_tags).expect("remaining timeline");
    let remaining_fingerprint = release_tag_fingerprint(&remaining_tags);
    persist_timeline_catalog_with_fingerprint(
        &connection,
        &remaining_timeline,
        &remaining_fingerprint,
    )
    .expect("stage tag removal");
    assert_eq!(
        release_catalog_state(&connection, &timeline.repo_path),
        expected_initial,
        "staging or cancellation leaves the prior ready catalog untouched"
    );

    let publish_remaining = || {
        publish_catalog(
            &connection,
            &remaining_timeline,
            &remaining_tags,
            &remaining_fingerprint,
        )
        .expect("remaining release catalog")
    };
    let remaining_identity = publish_remaining();
    let expected_remaining = (
        remaining_identity.clone(),
        vec![(
            "v0.1.0".to_string(),
            "lightweight".to_string(),
            lightweight.commit_sha.clone(),
        )],
    );
    assert_eq!(
        release_catalog_state(&connection, &timeline.repo_path),
        expected_remaining
    );
    assert_eq!(publish_remaining(), remaining_identity);
    assert_eq!(
        release_catalog_state(&connection, &timeline.repo_path),
        expected_remaining,
        "republication is idempotent"
    );

    let invalid_tag = GitTagRecord {
        name: "v9.0.0".to_string(),
        object_sha: "missing-revision".to_string(),
        commit_sha: "missing-revision".to_string(),
        created_ts: 1,
    };
    assert!(publish_catalog(
        &connection,
        &remaining_timeline,
        &[invalid_tag],
        "failed-fingerprint",
    )
    .is_err());
    assert_eq!(
        release_catalog_state(&connection, &timeline.repo_path),
        expected_remaining,
        "failed publication rolls back identity, deletion, and rows"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn release_navigation_contract_defaults_are_versioned_and_bounded() {
    let catalog: HistoryReleaseCatalog = serde_json::from_str("{}").expect("legacy catalog");
    assert_eq!(
        catalog.schema_version,
        HISTORY_RELEASE_CATALOG_SCHEMA_VERSION
    );
    assert!(catalog.releases.is_empty());
    assert_eq!(catalog.coverage.state, HistoryCoverageState::Unavailable);
    assert!(!catalog.truncated);
    assert!(catalog.next_cursor.is_none());

    let window: HistoryTimelineWindow = serde_json::from_str("{}").expect("legacy window");
    assert_eq!(
        window.schema_version,
        HISTORY_TIMELINE_WINDOW_SCHEMA_VERSION
    );
    assert!(window.center_revision.is_none());
    assert!(window.revisions.is_empty());
    assert_eq!(window.applied_limit, 0);
    assert!(!window.truncated);
    assert!(!window.has_older);
    assert!(!window.has_newer);

    let cursor = HistoryOpaqueCursor("opaque:v1:fixture".to_string());
    assert_eq!(
        serde_json::to_string(&cursor).expect("cursor serialization"),
        "\"opaque:v1:fixture\""
    );
    assert!(serde_json::from_str::<HistoryTimelineCenter>(
        r#"{"kind":"release","tag":"v1.0.0","extra":true}"#
    )
    .is_err());
}

#[test]
fn invalid_revision_is_rejected_before_git_option_parsing() {
    let root = std::env::temp_dir();
    assert_eq!(
        resolve_revision(&root, "--upload-pack=bad").unwrap_err(),
        "A valid Git revision is required"
    );
}

#[test]
fn historical_file_bounds_remain_explicit_in_snapshot_coverage() {
    let mut snapshot = build_snapshot_from_blobs(
        "history:test",
        "revision",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn indexed() {}\n".to_vec(),
        }],
        &StructuralGraphCancellation::default(),
        &|_: StructuralGraphProgress| {},
    )
    .expect("snapshot");
    apply_historical_file_coverage(&mut snapshot, 25_001, true);
    assert!(snapshot.truncated);
    assert_eq!(snapshot.coverage.discovered_files, 25_001);
    assert!(snapshot.coverage.skipped_files >= 25_000);
    assert!(snapshot
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "historical_file_limit"));
}

#[test]
fn repository_without_tags_has_one_explicit_unreleased_range() {
    let root = std::env::temp_dir().join(format!("cv-history-no-tags-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("main.rs"), "fn main() {}\n").expect("main");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "initial"]);
    let timeline = build_timeline(&root, Some(20)).expect("timeline");
    assert_eq!(timeline.release_ranges.len(), 1);
    assert!(timeline.release_ranges[0].is_unreleased);
    assert_eq!(
        timeline.release_ranges[0].commit_shas,
        vec![timeline.head.clone()]
    );
    assert_eq!(
        resolve_temporal_reference(
            &root,
            &HistoryTemporalReference::Date {
                at: timeline.revisions[0].committed_at.clone(),
            },
        )
        .expect("date reference"),
        timeline.head
    );
    assert!(resolve_temporal_reference(
        &root,
        &HistoryTemporalReference::Date {
            at: "not-a-date".to_string(),
        },
    )
    .unwrap_err()
    .contains("RFC3339"));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn divergent_release_tags_join_only_after_their_branch_is_merged() {
    let root = std::env::temp_dir().join(format!("cv-history-divergent-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("base.rs"), "fn base() {}\n").expect("base");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "base"]);
    run_git(&root, &["tag", "v1.0.0"]);
    let main_branch = git_text(&root, &["branch", "--show-current"]).expect("branch");

    run_git(&root, &["checkout", "-b", "release-side"]);
    fs::write(root.join("side.rs"), "fn side() {}\n").expect("side");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "side release"]);
    run_git(&root, &["tag", "v2.0.0-side"]);
    let side_sha = git_text(&root, &["rev-parse", "HEAD"]).expect("side sha");

    run_git(&root, &["checkout", &main_branch]);
    fs::write(root.join("main.rs"), "fn main_line() {}\n").expect("main");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "main work"]);
    let before_merge = reachable_release_revisions(&root).expect("before merge releases");
    assert!(!before_merge.contains(&side_sha));

    run_git(
        &root,
        &[
            "merge",
            "--no-ff",
            "release-side",
            "-m",
            "merge release side",
        ],
    );
    let after_merge = reachable_release_revisions(&root).expect("after merge releases");
    assert!(after_merge.contains(&side_sha));
    let timeline = build_timeline(&root, Some(20)).expect("merged timeline");
    assert_eq!(timeline.revisions.last().expect("head").parents.len(), 2);
    assert!(timeline
        .release_ranges
        .iter()
        .any(|range| range.tag.as_deref() == Some("v2.0.0-side")));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn merge_reconstruction_follows_the_recorded_first_parent_chain() {
    let root = std::env::temp_dir().join(format!("cv-history-merge-dag-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("base.rs"), "fn base() {}\n").expect("base");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "base"]);
    let main_branch = git_text(&root, &["branch", "--show-current"]).expect("main branch");
    run_git(&root, &["checkout", "-b", "feature"]);
    fs::write(root.join("feature.rs"), "fn feature() {}\n").expect("feature");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feature"]);
    run_git(&root, &["checkout", &main_branch]);
    fs::write(root.join("main.rs"), "fn main_line() {}\n").expect("main line");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "main line"]);
    run_git(
        &root,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
    );

    let timeline = build_timeline(&root, Some(20)).expect("timeline");
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let cancellation = StructuralGraphCancellation::default();
    let mut snapshots = HashMap::new();
    for revision in &timeline.revisions {
        let mut snapshot = build_snapshot_from_blobs(
            &storage_key,
            &revision.sha,
            GitObjectReader::new(&root)
                .blobs_at(&revision.sha)
                .expect("revision blobs"),
            &cancellation,
            &|_: StructuralGraphProgress| {},
        )
        .expect("revision snapshot");
        compact_history_snapshot(&mut snapshot);
        snapshots.insert(revision.sha.clone(), snapshot);
    }

    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &timeline).expect("persist timeline");
    let root_revision = timeline
        .revisions
        .iter()
        .find(|revision| revision.parents.is_empty())
        .expect("root revision");
    let root_snapshot = snapshots.get(&root_revision.sha).expect("root snapshot");
    persist_history_snapshot_blob(&connection, &canonical, &root_revision.sha, root_snapshot)
        .expect("persist root snapshot");
    connection
        .execute(
            "INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, status, coverage_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', '{}', ?7)",
            params![
                canonical,
                root_revision.sha,
                root_snapshot.id,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
                timeline.generated_at,
            ],
        )
        .expect("root checkpoint");
    for revision in timeline
        .revisions
        .iter()
        .filter(|revision| !revision.parents.is_empty())
    {
        let parent = revision.parents.first().expect("first parent");
        compute_and_persist_structural_delta(
            &connection,
            &root,
            &canonical,
            parent,
            &revision.sha,
            snapshots.get(parent).expect("parent snapshot"),
            snapshots.get(&revision.sha).expect("child snapshot"),
        )
        .expect("parent-aware delta");
    }

    let reconstructed =
        reconstruct_history_as_of(&connection, &canonical, &storage_key, &timeline.head)
            .expect("reconstruct merge")
            .expect("complete first-parent chain");
    let expected = snapshots.get(&timeline.head).expect("head snapshot");
    let mut reconstructed_files = reconstructed
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let mut expected_files = expected
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    reconstructed_files.sort();
    expected_files.sort();
    assert_eq!(reconstructed_files, expected_files);
    let mut reconstructed_nodes = reconstructed.nodes.clone();
    let mut expected_nodes = expected.nodes.clone();
    reconstructed_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    expected_nodes.sort_by(|left, right| left.id.cmp(&right.id));
    let mut reconstructed_edges = reconstructed.edges.clone();
    let mut expected_edges = expected.edges.clone();
    reconstructed_edges.sort_by(|left, right| left.id.cmp(&right.id));
    expected_edges.sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(reconstructed_nodes, expected_nodes);
    assert_eq!(reconstructed_edges, expected_edges);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn rolling_timeline_windows_keep_global_ordinals_and_old_releases() {
    let root = std::env::temp_dir().join(format!("cv-history-window-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    for index in 0..6 {
        fs::write(
            root.join("history.rs"),
            format!("fn version_{index}() {{}}\n"),
        )
        .expect("history");
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", &format!("commit {index}")]);
        if index == 0 {
            run_git(&root, &["tag", "v1.0.0"]);
        }
    }
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let first = build_timeline(&root, Some(3)).expect("first window");
    persist_timeline(&connection, &first).expect("persist first window");
    for index in 6..8 {
        fs::write(
            root.join("history.rs"),
            format!("fn version_{index}() {{}}\n"),
        )
        .expect("history");
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", &format!("commit {index}")]);
    }
    let second = build_timeline(&root, Some(3)).expect("second window");
    persist_timeline(&connection, &second).expect("persist second window");

    let global_ordinals = revision_ordinals(&root).expect("global ordinals");
    let mut statement = connection
        .prepare("SELECT sha, ordinal FROM history_graph_revisions WHERE repo_path = ?1")
        .expect("ordinal query");
    let rows = statement
        .query_map([second.repo_path.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .expect("ordinal rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read ordinals");
    assert!(rows.iter().all(|(sha, ordinal)| {
        global_ordinals.get(sha).copied() == Some(*ordinal) && *ordinal >= 0
    }));
    let releases = load_history_revisions(&connection, &second.repo_path, None, true, 10)
        .expect("release query");
    assert_eq!(releases.revisions.len(), 1);
    assert_eq!(releases.revisions[0].tags, vec!["v1.0.0"]);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn catalog_staging_does_not_publish_freshness_before_backfill_success() {
    let root = std::env::temp_dir().join(format!("cv-history-publish-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("history.rs"), "fn first() {}\n").expect("first");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "first"]);
    let first = build_timeline(&root, Some(20)).expect("first timeline");
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &first).expect("publish first timeline");

    fs::write(root.join("history.rs"), "fn second() {}\n").expect("second");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "second"]);
    let second = build_timeline(&root, Some(20)).expect("second timeline");
    persist_timeline_catalog(&connection, &second).expect("stage second catalog");
    let (indexed_head, status): (Option<String>, String) = connection
        .query_row(
            "SELECT indexed_head, status FROM history_graph_repositories WHERE repo_path = ?1",
            [second.repo_path.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("published freshness");
    assert_eq!(indexed_head.as_deref(), Some(first.head.as_str()));
    assert_eq!(status, "ready");
    assert_ne!(indexed_head.as_deref(), Some(second.head.as_str()));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn shallow_history_reports_partial_coverage() {
    let origin = std::env::temp_dir().join(format!("cv-history-origin-{}", uuid::Uuid::new_v4()));
    let shallow = std::env::temp_dir().join(format!("cv-history-shallow-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&origin).expect("origin");
    run_git(&origin, &["init"]);
    run_git(&origin, &["config", "user.email", "fixture@local"]);
    run_git(&origin, &["config", "user.name", "Fixture"]);
    for index in 0..3 {
        fs::write(origin.join("history.txt"), format!("{index}\n")).expect("history");
        run_git(&origin, &["add", "."]);
        run_git(&origin, &["commit", "-m", &format!("commit {index}")]);
    }
    let source = format!("file://{}", origin.display());
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &source])
        .arg(&shallow)
        .status()
        .expect("clone");
    assert!(status.success());

    let timeline = build_timeline(&shallow, Some(20)).expect("shallow timeline");
    assert!(timeline.is_shallow);
    assert!(!timeline.coverage_complete);
    assert_eq!(timeline.revisions.len(), 1);
    fs::remove_dir_all(origin).expect("remove origin");
    fs::remove_dir_all(shallow).expect("remove shallow");
}

#[test]
fn path_history_preserves_rename_copy_and_delete_leads() {
    let root = std::env::temp_dir().join(format!("cv-history-paths-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("src")).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("src/old.rs"), "fn carried() {}\n").expect("old");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "add old"]);
    run_git(&root, &["mv", "src/old.rs", "src/new.rs"]);
    run_git(&root, &["commit", "-m", "rename old"]);
    let rename_head = git_text(&root, &["rev-parse", "HEAD"]).expect("rename head");
    let rename = changed_path_records(&root, &rename_head).expect("rename changes");
    assert!(rename.iter().any(|change| {
        change.change_kind == "renamed"
            && change.old_path.as_deref() == Some("src/old.rs")
            && change.path == "src/new.rs"
    }));

    fs::copy(root.join("src/new.rs"), root.join("src/copy.rs")).expect("copy");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "copy new"]);
    let copy_head = git_text(&root, &["rev-parse", "HEAD"]).expect("copy head");
    let copy = changed_path_records(&root, &copy_head).expect("copy changes");
    assert!(copy.iter().any(|change| {
        change.change_kind == "copied"
            && change.old_path.as_deref() == Some("src/new.rs")
            && change.path == "src/copy.rs"
    }));

    fs::remove_file(root.join("src/copy.rs")).expect("delete");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "delete copy"]);
    let delete_head = git_text(&root, &["rev-parse", "HEAD"]).expect("delete head");
    assert!(changed_path_records(&root, &delete_head)
        .expect("delete changes")
        .iter()
        .any(|change| change.change_kind == "deleted" && change.path == "src/copy.rs"));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn structural_lineage_tracks_renames_and_preserves_split_ambiguity() {
    let cancellation = StructuralGraphCancellation::default();
    let progress = |_: StructuralGraphProgress| {};
    let before = build_snapshot_from_blobs(
        "history:test",
        "before",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn old_name() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("before");
    let renamed = build_snapshot_from_blobs(
        "history:test",
        "renamed",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn new_name() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("renamed");
    let rename_lineage = derive_lineage(&before, &renamed, &[], "renamed");
    assert!(rename_lineage.iter().any(|edge| {
        edge.relation == "renamed_to"
            && edge.trust == GraphTrust::Inferred
            && renamed
                .nodes
                .iter()
                .any(|node| node.id == edge.to_entity_id && node.label == "new_name")
    }));

    let split = build_snapshot_from_blobs(
        "history:test",
        "split",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn first() {} fn second() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("split");
    let split_lineage = derive_lineage(&before, &split, &[], "split");
    assert!(split_lineage.iter().any(|edge| {
        edge.relation == "split_into"
            && edge.trust == GraphTrust::Ambiguous
            && !edge.candidates.is_empty()
    }));

    let merge_before = build_snapshot_from_blobs(
        "history:test",
        "merge-before",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn first() {} fn second() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("merge before");
    let merge_after = build_snapshot_from_blobs(
        "history:test",
        "merge-after",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn combined() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("merge after");
    assert!(derive_lineage(&merge_before, &merge_after, &[], "merged")
        .iter()
        .any(|edge| {
            edge.relation == "merged_from"
                && edge.trust == GraphTrust::Ambiguous
                && !edge.candidates.is_empty()
        }));

    let stable_before = build_snapshot_from_blobs(
        "history:test",
        "stable-before",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn stable(value: i32) {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("stable before");
    let stable_after = build_snapshot_from_blobs(
        "history:test",
        "stable-after",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn stable(value: i64) {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("stable after");
    assert!(derive_lineage(&stable_before, &stable_after, &[], "stable")
        .iter()
        .any(|edge| edge.relation == "same_as"));

    let cross_language_before = build_snapshot_from_blobs(
        "history:test",
        "cross-language-before",
        vec![HistoricalFileBlob {
            path: "src/handler.rs".to_string(),
            bytes: b"fn carried() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("cross-language before");
    let cross_language_after = build_snapshot_from_blobs(
        "history:test",
        "cross-language-after",
        vec![HistoricalFileBlob {
            path: "src/handler.ts".to_string(),
            bytes: b"function carried() {}\n".to_vec(),
        }],
        &cancellation,
        &progress,
    )
    .expect("cross-language after");
    let cross_language = derive_lineage(
        &cross_language_before,
        &cross_language_after,
        &[HistoryPathChange {
            path: "src/handler.ts".to_string(),
            change_kind: "renamed".to_string(),
            old_path: Some("src/handler.rs".to_string()),
            additions: None,
            deletions: None,
        }],
        "cross-language-after",
    );
    assert!(cross_language.iter().any(|edge| {
        edge.relation == "moved_to"
            && edge.trust == GraphTrust::Extracted
            && cross_language_after.nodes.iter().any(|node| {
                node.id == edge.to_entity_id
                    && node.label == "carried"
                    && node.language.as_deref() == Some("typescript")
            })
    }));
}

#[test]
fn outcome_evidence_requires_an_explicit_local_observation() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES ('/fixture', 'fixture', 'ready', '2026-01-01T00:00:00Z',
                '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("repository");

    assert!(load_outcome_events(&connection, "/fixture", "event:signup")
        .expect("empty outcomes")
        .is_empty());

    connection
        .execute(
            "INSERT INTO history_graph_events (
                id, repo_path, event_kind, entity_id, trust, origin, source_id,
                payload_json, evidence_json, recorded_at
             ) VALUES
                ('code-change', '/fixture', 'structural_delta', 'event:signup',
                 'extracted', 'syntax', 'git', '{}', '[]', '2026-01-01T00:00:00Z'),
                ('provider-delivery', '/fixture', 'analytics_provider_delivery',
                 'event:signup', 'extracted', 'metadata', 'provider-export', '{}', '[]',
                 '2026-01-02T00:00:00Z')",
            [],
        )
        .expect("events");

    let outcomes = load_outcome_events(&connection, "/fixture", "event:signup").expect("outcomes");
    assert_eq!(outcomes.len(), 1, "code presence is not provider delivery");
    assert_eq!(outcomes[0].0, "provider-delivery");
    assert_eq!(outcomes[0].1, "analytics_provider_delivery");
    assert_eq!(outcomes[0].2, GraphTrust::Extracted);

    connection
        .execute(
            "INSERT INTO history_graph_annotations (
                id, repo_path, entity_id, author, body, decision, source, created_at
             ) VALUES ('reject-1', '/fixture', 'event:signup', 'owner',
                'Provider export belongs to another environment', 'reject', 'user',
                '2026-01-03T00:00:00Z')",
            [],
        )
        .expect("annotation");
    let contradictions =
        load_entity_annotation_contradictions(&connection, "/fixture", "event:signup")
            .expect("contradictions");
    assert_eq!(contradictions.len(), 1);
    assert!(contradictions[0].contains("another environment"));
}

#[test]
fn lineage_queries_preserve_candidates_and_report_repository_freshness() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status, coverage_json,
                created_at, updated_at
             ) VALUES ('/fixture', 'fixture', 'head-2', 'ready',
                '{\"coverage_complete\":true}', '2026-01-01T00:00:00Z',
                '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("repository");
    let edge = HistoryLineageEdge {
        id: "lineage-1".to_string(),
        from_entity_id: "old".to_string(),
        to_entity_id: "new-a".to_string(),
        relation: "split_into".to_string(),
        trust: GraphTrust::Ambiguous,
        evidence: "two compatible successors".to_string(),
        sources: Vec::new(),
        candidates: vec!["new-b".to_string()],
    };
    connection
        .execute(
            "INSERT INTO history_graph_events (
                id, repo_path, event_kind, entity_id, related_entity_id, relation_kind,
                trust, origin, source_id, payload_json, evidence_json, recorded_at
             ) VALUES (?1, '/fixture', 'entity_lineage', ?2, ?3, ?4,
                'ambiguous', 'analysis', 'fixture', ?5, '[]', '2026-01-01T00:00:00Z')",
            params![
                edge.id,
                edge.from_entity_id,
                edge.to_entity_id,
                edge.relation,
                serde_json::to_string(&edge).expect("lineage json")
            ],
        )
        .expect("lineage event");

    let (lineage, family, truncated) =
        load_lineage_family(&connection, "/fixture", "old", 20).expect("lineage family");
    assert!(!truncated);
    assert_eq!(lineage, vec![edge]);
    assert!(family.contains("old"));
    assert!(family.contains("new-a"));
    assert!(family.contains("new-b"));

    let (indexed_head, stale, coverage) =
        history_index_freshness(&connection, "/fixture", "head-2").expect("freshness");
    assert_eq!(indexed_head, "head-2");
    assert!(!stale);
    assert_eq!(coverage["coverage_complete"], true);
    assert!(
        history_index_freshness(&connection, "/fixture", "head-3")
            .expect("stale freshness")
            .1
    );
    connection
        .execute(
            "UPDATE history_graph_repositories
             SET status = 'partial',
                 coverage_json = '{\"coverage_complete\":false,\"cancelled\":true,\"adapter_coverage\":\"partial\"}'
             WHERE repo_path = '/fixture'",
            [],
        )
        .expect("partial coverage");
    let (_, stale, partial) =
        history_index_freshness(&connection, "/fixture", "head-2").expect("partial query");
    assert!(
        !stale,
        "partial adapter coverage is separate from Git freshness"
    );
    assert_eq!(partial["coverage_complete"], false);
    assert_eq!(partial["cancelled"], true);
    assert_eq!(partial["adapter_coverage"], "partial");
}

#[test]
fn prior_removal_produces_an_explicit_reintroduction_edge() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES ('/fixture', 'fixture', 'ready', '2026-01-01T00:00:00Z',
                '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("repository");
    let cancellation = StructuralGraphCancellation::default();
    let snapshot = build_snapshot_from_blobs(
        "history:test",
        "returned",
        vec![HistoricalFileBlob {
            path: "src/lib.rs".to_string(),
            bytes: b"fn returned() {}\n".to_vec(),
        }],
        &cancellation,
        &|_: StructuralGraphProgress| {},
    )
    .expect("snapshot");
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.label == "returned")
        .expect("returned node");
    let removal = HistoryLineageEdge {
        id: "removed-1".to_string(),
        from_entity_id: node.id.clone(),
        to_entity_id: "old-revision".to_string(),
        relation: "removed_in".to_string(),
        trust: GraphTrust::Extracted,
        evidence: "absent".to_string(),
        sources: Vec::new(),
        candidates: Vec::new(),
    };
    connection
        .execute(
            "INSERT INTO history_graph_events (
                id, repo_path, event_kind, entity_id, related_entity_id, relation_kind,
                trust, origin, source_id, payload_json, evidence_json, recorded_at
             ) VALUES (?1, '/fixture', 'entity_lineage', ?2, ?3, 'removed_in',
                'extracted', 'analysis', 'fixture', ?4, '[]',
                '2026-01-01T00:00:00Z')",
            params![
                removal.id,
                removal.from_entity_id,
                removal.to_entity_id,
                serde_json::to_string(&removal).expect("removal json")
            ],
        )
        .expect("removal event");
    let reintroduced = derive_reintroductions(
        &connection,
        "/fixture",
        &snapshot,
        std::slice::from_ref(&node.id),
        "new-revision",
    )
    .expect("reintroduction");
    assert_eq!(reintroduced.len(), 1);
    assert_eq!(reintroduced[0].relation, "reintroduced_in");
    assert_eq!(reintroduced[0].trust, GraphTrust::Extracted);
}

#[test]
fn refresh_classification_prioritizes_rewrites_and_engine_repairs() {
    assert_eq!(
        classify_history_refresh(None, false, false, false, false),
        "initial"
    );
    assert_eq!(
        classify_history_refresh(Some("old"), true, true, false, true),
        "rewritten_history"
    );
    assert_eq!(
        classify_history_refresh(Some("head"), false, true, false, true),
        "engine_repair"
    );
    assert_eq!(
        classify_history_refresh(Some("old"), false, false, true, true),
        "fast_forward"
    );
    assert_eq!(
        classify_history_refresh(Some("head"), false, false, false, true),
        "tag_metadata"
    );
    assert_eq!(
        classify_history_refresh(Some("head"), false, false, false, false),
        "no_op"
    );
}

#[test]
fn automatic_release_checkpoints_are_bounded_and_prefer_recent_releases() {
    let releases = (0..40)
        .rev()
        .map(|index| format!("release-{index:02}"))
        .collect::<Vec<_>>();
    let selected = automatic_release_checkpoint_revisions(&releases);
    assert_eq!(selected.len(), 24);
    assert_eq!(selected.first().map(String::as_str), Some("release-39"));
    assert_eq!(selected.last().map(String::as_str), Some("release-16"));
    assert_eq!(
        automatic_release_checkpoint_revisions(&releases[..3]),
        releases[..3]
    );
}

#[test]
fn checkpoint_compatibility_invalidates_changed_structural_ignore_policy() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let repo = "/ignore-policy-fixture";
    let revision = "a".repeat(40);
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, 'fixture', 'ready', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [repo],
        )
        .expect("repository");
    connection
        .execute(
            "INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject
             ) VALUES (?1, ?2, 0, '2026-01-01T00:00:00Z', 'Fixture', 'fixture')",
            params![repo, revision],
        )
        .expect("revision");
    connection
        .execute(
            "INSERT INTO structural_graph_snapshots (
                id, repo_path, repo_head, schema_version, engine_id, engine_version,
                engine_json, ignore_fingerprint, coverage_json, created_at
             ) VALUES ('snapshot', ?1, ?2, ?3, ?4, ?5, '{}', ?6, '{}', '2026-01-01T00:00:00Z')",
            params![
                repo,
                revision,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                crate::commands::structural_graph::extract::current_ignore_fingerprint(),
            ],
        )
        .expect("snapshot");
    connection
        .execute(
            "INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, status, coverage_json, created_at
             ) VALUES (?1, ?2, 'snapshot', ?3, ?4, ?5, 'ready', '{}', '2026-01-01T00:00:00Z')",
            params![
                repo,
                revision,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
        )
        .expect("checkpoint");
    assert!(!has_incompatible_history_checkpoints(&connection, repo).expect("compatible"));
    connection
        .execute(
            "UPDATE structural_graph_snapshots SET ignore_fingerprint = 'old-policy' WHERE id = 'snapshot'",
            [],
        )
        .expect("change ignore policy");
    assert!(has_incompatible_history_checkpoints(&connection, repo).expect("incompatible"));
}

#[test]
fn normalized_fact_probe_skips_all_history_scan_only_for_exactly_matching_inputs() {
    let probe = HistoryFactCatalogProbe::ready_for_test(
        "a".repeat(40),
        "tags".to_string(),
        "mailmap".to_string(),
    );
    assert!(normalized_facts_are_current(
        &probe,
        &"a".repeat(40),
        "tags",
        "mailmap",
        false,
    ));
    assert!(!normalized_facts_are_current(
        &probe,
        &"b".repeat(40),
        "tags",
        "mailmap",
        false,
    ));
    assert!(!normalized_facts_are_current(
        &probe,
        &"a".repeat(40),
        "tags",
        "changed-mailmap",
        false,
    ));
    assert!(!normalized_facts_are_current(
        &probe,
        &"a".repeat(40),
        "tags",
        "mailmap",
        true,
    ));
}

#[test]
fn exact_as_of_reconstructs_from_nearest_checkpoint_and_ordered_deltas() {
    let root = std::env::temp_dir().join(format!("cv-as-of-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("src")).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("src/lib.rs"), "fn first() {}\n").expect("first");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: first"]);
    fs::write(root.join("src/lib.rs"), "fn first() {}\nfn second() {}\n").expect("second");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "feat: second"]);
    let timeline = build_timeline(&root, Some(20)).expect("timeline");
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let cancellation = StructuralGraphCancellation::default();
    let build = |revision: &str| {
        let mut snapshot = build_snapshot_from_blobs(
            &storage_key,
            revision,
            GitObjectReader::new(&root)
                .blobs_at(revision)
                .expect("historical blobs"),
            &cancellation,
            &|_: StructuralGraphProgress| {},
        )
        .expect("snapshot");
        compact_history_snapshot(&mut snapshot);
        snapshot
    };
    let before = build(&timeline.revisions[0].sha);
    let after = build(&timeline.revisions[1].sha);
    let path_changes =
        changed_path_records(&root, &timeline.revisions[1].sha).expect("path changes");
    let changed_paths = path_changes
        .iter()
        .filter(|change| change.change_kind != "deleted")
        .map(|change| change.path.clone())
        .collect::<Vec<_>>();
    let mut incremental_after = build_snapshot_from_blob_delta(
        &storage_key,
        &timeline.revisions[1].sha,
        &before,
        GitObjectReader::new(&root)
            .blobs_for_paths(&timeline.revisions[1].sha, &changed_paths)
            .expect("changed blobs"),
        &[],
        &cancellation,
        &|_: StructuralGraphProgress| {},
    )
    .expect("incremental snapshot");
    compact_history_snapshot(&mut incremental_after);
    let normalize = |snapshot: &mut StructuralGraphSnapshot| {
        snapshot.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        snapshot.edges.sort_by(|left, right| left.id.cmp(&right.id));
    };
    let mut expected_after = after.clone();
    incremental_after.created_at = expected_after.created_at.clone();
    normalize(&mut incremental_after);
    normalize(&mut expected_after);
    assert_eq!(
        incremental_after, expected_after,
        "path-scoped historical extraction must equal a full revision build"
    );
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &timeline).expect("timeline persistence");
    persist_history_snapshot_blob(&connection, &canonical, &timeline.revisions[0].sha, &before)
        .expect("compressed before snapshot");
    connection
        .execute(
            "INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, status, coverage_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', '{}', ?7)",
            params![
                canonical,
                timeline.revisions[0].sha,
                before.id,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
                timeline.generated_at,
            ],
        )
        .expect("checkpoint");
    connection
        .execute(
            "INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, status, coverage_json, created_at
             ) VALUES (?1, ?2, ?3, 'obsolete-engine', '0', 1, 'ready', '{}', ?4)",
            params![
                canonical,
                timeline.revisions[1].sha,
                after.id,
                timeline.generated_at,
            ],
        )
        .expect("incompatible checkpoint");
    let delta = compute_and_persist_structural_delta(
        &connection,
        &root,
        &canonical,
        &timeline.revisions[0].sha,
        &timeline.revisions[1].sha,
        &before,
        &after,
    )
    .expect("delta");
    assert!(!delta.added_node_ids.is_empty());
    assert!(delta
        .path_changes
        .iter()
        .any(|change| change.path == "src/lib.rs"));

    let mut reconstructed = reconstruct_history_as_of(
        &connection,
        &canonical,
        &storage_key,
        &timeline.revisions[1].sha,
    )
    .expect("as-of reconstruction")
    .expect("complete delta chain");
    let mut expected = after.clone();
    normalize(&mut reconstructed);
    normalize(&mut expected);
    assert_eq!(
        reconstructed, expected,
        "delta application must preserve exact graph content"
    );
    assert_eq!(
        reconstructed.repo_head.as_deref(),
        Some(timeline.revisions[1].sha.as_str())
    );
    assert!(reconstructed
        .nodes
        .iter()
        .any(|node| node.label == "second"));
    connection
        .execute(
            "DELETE FROM history_graph_events WHERE event_kind = 'structural_delta'",
            [],
        )
        .expect("remove delta");
    assert!(reconstruct_history_as_of(
        &connection,
        &canonical,
        &storage_key,
        &timeline.revisions[1].sha,
    )
    .expect("bounded missing chain")
    .is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rewritten_history_repair_preserves_imports_annotations_and_adapter_cursors() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute_batch(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status,
                created_at, updated_at
             ) VALUES ('/fixture', 'fixture', 'old-head', 'ready',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
             INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject,
                parents_json, tags_json
             ) VALUES ('/fixture', 'old-head', 0, '2026-01-01T00:00:00Z',
                'Fixture', 'old commit', '[]', '[]');
             INSERT INTO structural_graph_snapshots (
                id, repo_path, repo_head, schema_version, engine_id, engine_version,
                engine_json, coverage_json, created_at
             ) VALUES ('old-snapshot', 'history:fixture', 'old-head', 1,
                'old-engine', '0', '{}', '{}', '2026-01-01T00:00:00Z');
             INSERT INTO history_graph_checkpoints (
                repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                schema_version, created_at
             ) VALUES ('/fixture', 'old-head', 'old-snapshot', 'old-engine', '0', 1,
                '2026-01-01T00:00:00Z');
             INSERT INTO history_graph_events (
                id, repo_path, revision_sha, event_kind, trust, origin, source_id,
                source_cursor, payload_json, evidence_json, recorded_at
             ) VALUES
                ('derived', '/fixture', 'old-head', 'structural_delta', 'extracted',
                 'analysis', 'codevetter-structural-history', 'old-head', '{}', '[]',
                 '2026-01-01T00:00:00Z'),
                ('imported', '/fixture', NULL, 'analytics_provider_delivery', 'extracted',
                 'metadata', 'provider-export', 'provider:42', '{}', '[]',
                 '2026-01-02T00:00:00Z');
             INSERT INTO history_graph_annotations (
                id, repo_path, author, body, decision, source, created_at
             ) VALUES ('annotation', '/fixture', 'owner', 'keep this correction',
                'correct', 'user', '2026-01-03T00:00:00Z');",
        )
        .expect("fixture data");

    let invalidated =
        repair_derived_history(&connection, "/fixture", true, true, "2026-01-04T00:00:00Z")
            .expect("repair");
    assert!(invalidated >= 4);
    for table in [
        "history_graph_checkpoints",
        "history_graph_revisions",
        "structural_graph_snapshots",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("derived count");
        assert_eq!(count, 0, "{table} should be invalidated");
    }
    let imported: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_events WHERE id = 'imported'",
            [],
            |row| row.get(0),
        )
        .expect("imported evidence");
    assert_eq!(imported, 1);
    let annotations: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_annotations",
            [],
            |row| row.get(0),
        )
        .expect("annotations");
    assert_eq!(annotations, 1);
    persist_history_adapter_cursors(&connection, "/fixture", "new-head").expect("adapter cursors");
    let cursor_json: String = connection
        .query_row(
            "SELECT cursor_json FROM history_graph_repositories WHERE repo_path = '/fixture'",
            [],
            |row| row.get(0),
        )
        .expect("cursor json");
    let cursor: Value = serde_json::from_str(&cursor_json).expect("cursor payload");
    assert_eq!(cursor["head"], "new-head");
    assert_eq!(cursor["adapters"]["provider-export"], "provider:42");
}

#[test]
#[ignore = "performance benchmark; run explicitly with --ignored --nocapture"]
fn bench_history_backfill_incremental_and_as_of_real_repo() {
    let process_usage = || {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        assert_eq!(status, 0, "getrusage");
        unsafe { usage.assume_init() }
    };
    let timeval_seconds =
        |value: libc::timeval| value.tv_sec as f64 + value.tv_usec as f64 / 1_000_000.0;
    let usage_before = process_usage();
    let root = std::env::var("CV_GRAPH_BENCH_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()
                .expect("repo root")
        });
    let limit = std::env::var("CV_HISTORY_BENCH_COMMITS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(24)
        .clamp(4, 100);
    let total_started = std::time::Instant::now();
    let cancellation = StructuralGraphCancellation::default();
    let tag_records = read_git_tags(&root).expect("tags");
    let history_build = build_timeline_bundle_with_tags_cancellable(
        &root,
        Some(limit),
        &tag_records,
        &cancellation,
    )
    .expect("history facts");
    assert_eq!(
        history_build.fact_git_process_count, 1,
        "a full normalized history read must use exactly one batched Git process"
    );
    let timeline = history_build.timeline.clone();
    let canonical = root.to_string_lossy().to_string();
    let storage_key = history_storage_key(&canonical);
    let db_path = std::env::temp_dir().join(format!(
        "codevetter-temporal-bench-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let connection = Connection::open(&db_path).expect("benchmark database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    persist_timeline(&connection, &timeline).expect("timeline persistence");
    let release_revisions_newest_first = timeline
        .revisions
        .iter()
        .rev()
        .filter(|revision| revision.is_release)
        .map(|revision| revision.sha.clone())
        .collect::<Vec<_>>();
    let automatic_release_checkpoints =
        automatic_release_checkpoint_revisions(&release_revisions_newest_first);
    let automatic_release_checkpoint_set = automatic_release_checkpoints
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut build_samples = Vec::with_capacity(timeline.revisions.len());
    let build_snapshot = |revision: &HistoryRevision| {
        let started = std::time::Instant::now();
        let mut snapshot = build_snapshot_from_blobs(
            &storage_key,
            &revision.sha,
            GitObjectReader::new(&root)
                .blobs_at(&revision.sha)
                .expect("historical blobs"),
            &cancellation,
            &|_: StructuralGraphProgress| {},
        )
        .expect("historical snapshot");
        compact_history_snapshot(&mut snapshot);
        (snapshot, started.elapsed().as_secs_f64() * 1000.0)
    };
    let persist_benchmark_checkpoint =
        |revision: &HistoryRevision, snapshot: &StructuralGraphSnapshot| {
            persist_history_snapshot_blob(&connection, &canonical, &revision.sha, snapshot)
                .expect("compressed snapshot persistence");
            connection
                .execute(
                    "INSERT INTO history_graph_checkpoints (
                        repo_path, revision_sha, snapshot_id, engine_id, engine_version,
                        schema_version, status, coverage_json, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ready', ?7, ?8)",
                    params![
                        canonical,
                        revision.sha,
                        snapshot.id,
                        snapshot.engine.id,
                        snapshot.engine.version,
                        snapshot.schema_version,
                        serde_json::to_string(&snapshot.coverage).expect("coverage"),
                        snapshot.created_at,
                    ],
                )
                .expect("checkpoint");
        };
    let first_revision = timeline.revisions.first().expect("benchmark revision");
    let (mut previous_snapshot, first_build_ms) = build_snapshot(first_revision);
    build_samples.push(first_build_ms);
    persist_benchmark_checkpoint(first_revision, &previous_snapshot);
    let mut checkpointed_revisions = HashSet::from([first_revision.sha.clone()]);
    let mut checkpoint_count = 1usize;
    let mut delta_samples = Vec::with_capacity(timeline.revisions.len().saturating_sub(1));
    let mut delta_node_changes = 0usize;
    let mut delta_edge_changes = 0usize;
    let mut one_commit_refresh_ms = 0.0;
    // Qualification mirrors production: one representative append delta proves
    // the incremental path, while the initial index stores only bounded
    // checkpoints instead of manufacturing a delta for every old revision.
    for index in 1..timeline.revisions.len().min(2) {
        let revision = &timeline.revisions[index];
        let path_changes = changed_path_records(&root, &revision.sha).expect("path changes");
        let changed_paths = path_changes
            .iter()
            .filter(|change| change.change_kind != "deleted")
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        let deleted_paths = path_changes
            .iter()
            .filter(|change| change.change_kind == "deleted")
            .map(|change| change.path.clone())
            .chain(
                path_changes
                    .iter()
                    .filter(|change| change.change_kind == "renamed")
                    .filter_map(|change| change.old_path.clone()),
            )
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let mut after_snapshot = build_snapshot_from_blob_delta(
            &storage_key,
            &revision.sha,
            &previous_snapshot,
            GitObjectReader::new(&root)
                .blobs_for_paths(&revision.sha, &changed_paths)
                .expect("changed blobs"),
            &deleted_paths,
            &cancellation,
            &|_: StructuralGraphProgress| {},
        )
        .expect("incremental historical snapshot");
        compact_history_snapshot(&mut after_snapshot);
        let incremental_snapshot_ms = started.elapsed().as_secs_f64() * 1000.0;
        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
        build_samples.push(build_ms);
        if (index + 1 == timeline.revisions.len()
            || automatic_release_checkpoint_set.contains(revision.sha.as_str()))
            && checkpointed_revisions.insert(revision.sha.clone())
        {
            persist_benchmark_checkpoint(revision, &after_snapshot);
            checkpoint_count += 1;
        }
        let started = std::time::Instant::now();
        let delta = compute_and_persist_structural_delta_with_paths(
            &connection,
            &canonical,
            &timeline.revisions[index - 1].sha,
            &revision.sha,
            &previous_snapshot,
            &after_snapshot,
            path_changes,
        )
        .expect("structural delta");
        delta_node_changes += delta.added_node_ids.len()
            + delta.removed_node_ids.len()
            + delta.changed_node_ids.len();
        delta_edge_changes += delta.added_edge_ids.len()
            + delta.removed_edge_ids.len()
            + delta.changed_edge_ids.len();
        let delta_ms = started.elapsed().as_secs_f64() * 1000.0;
        delta_samples.push(delta_ms);
        one_commit_refresh_ms = incremental_snapshot_ms + delta_ms;
        previous_snapshot = after_snapshot;
        if index % 4 == 0 {
            release_history_allocator_pressure();
        }
    }
    for revision in timeline.revisions.iter().filter(|revision| {
        revision.sha == timeline.head
            || automatic_release_checkpoint_set.contains(revision.sha.as_str())
    }) {
        if !checkpointed_revisions.insert(revision.sha.clone()) {
            continue;
        }
        let (snapshot, build_ms) = build_snapshot(revision);
        build_samples.push(build_ms);
        persist_benchmark_checkpoint(revision, &snapshot);
        checkpoint_count += 1;
        if revision.sha == timeline.head {
            previous_snapshot = snapshot;
        }
    }
    release_history_allocator_pressure();
    let tag_fingerprint = release_tag_fingerprint(&tag_records);
    let published_at = chrono::Utc::now().to_rfc3339();
    {
        let publication = connection
            .unchecked_transaction()
            .expect("benchmark publication transaction");
        let fact_index_identity = publish_history_facts(
            &publication,
            &history_build,
            &tag_records,
            &published_at,
            &cancellation,
        )
        .expect("publish history facts");
        publish_release_catalog(
            &publication,
            &timeline,
            &tag_records,
            &tag_fingerprint,
            timeline.coverage_complete,
        )
        .expect("publish release catalog");
        publish_release_intervals(&publication, &history_build, &tag_records)
            .expect("publish release intervals");
        publish_candidate_inflections(
            &publication,
            &canonical,
            &fact_index_identity,
            timeline.coverage_complete,
            &published_at,
            &cancellation,
        )
        .expect("publish candidate inflections");
        publication.commit().expect("commit benchmark publication");
    }
    let backfill_ms = total_started.elapsed().as_secs_f64() * 1000.0;
    let uncached_target_index = (timeline.revisions.len() * 3 / 4)
        .min(timeline.revisions.len().saturating_sub(2))
        .max(1);
    let uncached_target_revision = &timeline.revisions[uncached_target_index];
    let mut uncached_snapshot_samples = Vec::with_capacity(20);
    for _ in 0..20 {
        let (_, elapsed_ms) = build_snapshot(uncached_target_revision);
        uncached_snapshot_samples.push(elapsed_ms);
    }
    let cached_target_revision = automatic_release_checkpoints
        .last()
        .map(String::as_str)
        .unwrap_or(timeline.head.as_str());
    let mut as_of_samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = std::time::Instant::now();
        std::hint::black_box(
            reconstruct_history_as_of(
                &connection,
                &canonical,
                &storage_key,
                cached_target_revision,
            )
            .expect("as-of query")
            .expect("automatic checkpoint is readable"),
        );
        as_of_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let mut no_op_samples = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let started = std::time::Instant::now();
        std::hint::black_box(classify_history_refresh(
            Some(&timeline.head),
            false,
            false,
            false,
            false,
        ));
        no_op_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let history =
        HistoryReadService::new_with_current_head(&connection, root.clone(), timeline.head.clone())
            .expect("history read service");
    let mut release_query_samples = Vec::with_capacity(100);
    for _ in 0..100 {
        let started = std::time::Instant::now();
        std::hint::black_box(
            history
                .release_catalog(Some(100), None)
                .expect("release catalog query"),
        );
        release_query_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let mut contributor_query_samples = Vec::with_capacity(100);
    let contributor_scope = HistoryContributorScope::ExactInterval {
        from_exclusive: None,
        to_inclusive: timeline.head.clone(),
    };
    for _ in 0..100 {
        let started = std::time::Instant::now();
        std::hint::black_box(
            history
                .contributor_summary_page(contributor_scope.clone(), Some(20), None)
                .expect("contributor summary query"),
        );
        contributor_query_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let percentile = |samples: &mut Vec<f64>, percentile: usize| {
        samples.sort_by(f64::total_cmp);
        samples[samples.len() * percentile / 100]
    };
    let build_p50 = percentile(&mut build_samples, 50);
    let build_p95 = percentile(&mut build_samples, 95);
    let delta_p50 = percentile(&mut delta_samples, 50);
    let delta_p95 = percentile(&mut delta_samples, 95);
    let as_of_p50 = percentile(&mut as_of_samples, 50);
    let as_of_p95 = percentile(&mut as_of_samples, 95);
    let as_of_max = as_of_samples.last().copied().unwrap_or_default();
    let uncached_snapshot_p95 = percentile(&mut uncached_snapshot_samples, 95);
    let uncached_snapshot_max = uncached_snapshot_samples
        .last()
        .copied()
        .unwrap_or_default();
    let no_op_p50 = percentile(&mut no_op_samples, 50);
    let no_op_p95 = percentile(&mut no_op_samples, 95);
    let release_query_p50 = percentile(&mut release_query_samples, 50);
    let release_query_p95 = percentile(&mut release_query_samples, 95);
    let release_query_max = release_query_samples.last().copied().unwrap_or_default();
    let contributor_query_p50 = percentile(&mut contributor_query_samples, 50);
    let contributor_query_p95 = percentile(&mut contributor_query_samples, 95);
    let contributor_query_max = contributor_query_samples
        .last()
        .copied()
        .unwrap_or_default();
    let database_bytes = fs::metadata(&db_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    let snapshot_blob_bytes: i64 = connection
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(payload)), 0) FROM history_graph_snapshot_blobs",
            [],
            |row| row.get(0),
        )
        .expect("snapshot blob bytes");
    let delta_blob_bytes: i64 = connection
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(payload)), 0) FROM history_graph_event_blobs",
            [],
            |row| row.get(0),
        )
        .expect("delta blob bytes");
    let contributor_fact_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_contributors WHERE repo_path = ?1",
            [canonical.as_str()],
            |row| row.get(0),
        )
        .expect("contributor fact count");
    let landmark_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_graph_landmarks WHERE repo_path = ?1",
            [canonical.as_str()],
            |row| row.get(0),
        )
        .expect("landmark count");
    let rss_kib = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or_default();
    let usage_after = process_usage();
    let user_cpu = timeval_seconds(usage_after.ru_utime) - timeval_seconds(usage_before.ru_utime);
    let system_cpu = timeval_seconds(usage_after.ru_stime) - timeval_seconds(usage_before.ru_stime);
    let input_blocks = usage_after
        .ru_inblock
        .saturating_sub(usage_before.ru_inblock);
    let output_blocks = usage_after
        .ru_oublock
        .saturating_sub(usage_before.ru_oublock);

    eprintln!("\n=== bench_history_backfill_incremental_and_as_of_real_repo ===");
    eprintln!("repo:                  {}", root.display());
    eprintln!(
        "history:               {} commits · {} releases · {} auto-release checkpoints · {checkpoint_count} total checkpoints",
        timeline.revisions.len(),
        release_revisions_newest_first.len(),
        automatic_release_checkpoints.len(),
    );
    eprintln!(
        "history Git processes:  {} bounded batched log reader",
        history_build.fact_git_process_count
    );
    eprintln!(
        "graph:                 {} files · {} nodes · {} edges",
        previous_snapshot.coverage.indexed_files,
        previous_snapshot.nodes.len(),
        previous_snapshot.edges.len()
    );
    eprintln!("backfill total:         {backfill_ms:.2} ms");
    eprintln!("checkpoint p50/p95:     {build_p50:.2} / {build_p95:.2} ms");
    eprintln!("delta p50/p95:          {delta_p50:.2} / {delta_p95:.2} ms");
    eprintln!(
        "delta avg changes:       {:.0} nodes · {:.0} edges",
        delta_node_changes as f64 / delta_samples.len().max(1) as f64,
        delta_edge_changes as f64 / delta_samples.len().max(1) as f64
    );
    eprintln!("one-commit refresh:     {one_commit_refresh_ms:.2} ms");
    eprintln!(
        "uncached old snapshot p95/max: {uncached_snapshot_p95:.2} / {uncached_snapshot_max:.2} ms"
    );
    eprintln!("cached as-of p50/p95/max: {as_of_p50:.3} / {as_of_p95:.3} / {as_of_max:.3} ms");
    eprintln!("no-op p50/p95:          {no_op_p50:.6} / {no_op_p95:.6} ms");
    eprintln!(
        "release query p50/p95/max: {release_query_p50:.3} / {release_query_p95:.3} / {release_query_max:.3} ms"
    );
    eprintln!(
        "contributor query p50/p95/max: {contributor_query_p50:.3} / {contributor_query_p95:.3} / {contributor_query_max:.3} ms"
    );
    eprintln!(
        "checkpoint hit ratio:   {:.1}%",
        checkpoint_count as f64 / timeline.revisions.len() as f64 * 100.0
    );
    eprintln!(
        "database:               {:.2} MiB ({:.1} KiB/commit)",
        database_bytes as f64 / 1_048_576.0,
        database_bytes as f64 / 1024.0 / timeline.revisions.len() as f64
    );
    eprintln!(
        "normalized facts:       {contributor_fact_count} contributors · {landmark_count} landmarks"
    );
    eprintln!(
        "compressed payloads:    {:.2} MiB checkpoints · {:.2} MiB deltas",
        snapshot_blob_bytes as f64 / 1_048_576.0,
        delta_blob_bytes as f64 / 1_048_576.0
    );
    eprintln!(
        "process RSS:            {:.1} MiB\n",
        rss_kib as f64 / 1024.0
    );
    eprintln!("CPU user/system:        {user_cpu:.2} / {system_cpu:.2} s");
    eprintln!("filesystem block ops:   {input_blocks} read · {output_blocks} write\n");

    if let Ok(report_path) = std::env::var("CV_HISTORY_BENCH_REPORT") {
        let report = serde_json::json!({
            "schema_version": 1,
            "repo": root,
            "history": {
                "revisions": timeline.revisions.len(),
                "releases": release_revisions_newest_first.len(),
                "automatic_release_checkpoints": automatic_release_checkpoints.len(),
                "total_checkpoints": checkpoint_count,
                "fact_git_process_count": history_build.fact_git_process_count,
            },
            "graph": {
                "indexed_files": previous_snapshot.coverage.indexed_files,
                "nodes": previous_snapshot.nodes.len(),
                "edges": previous_snapshot.edges.len(),
            },
            "timing_ms": {
                "cold_index_total": backfill_ms,
                "checkpoint_p50": build_p50,
                "checkpoint_p95": build_p95,
                "delta_p50": delta_p50,
                "delta_p95": delta_p95,
                "one_commit_refresh": one_commit_refresh_ms,
                "uncached_old_snapshot_p95": uncached_snapshot_p95,
                "uncached_old_snapshot_max": uncached_snapshot_max,
                "as_of_p50": as_of_p50,
                "as_of_p95": as_of_p95,
                "as_of_max": as_of_max,
                "no_op_p50": no_op_p50,
                "no_op_p95": no_op_p95,
                "release_catalog_p50": release_query_p50,
                "release_catalog_p95": release_query_p95,
                "release_catalog_max": release_query_max,
                "contributor_summary_p50": contributor_query_p50,
                "contributor_summary_p95": contributor_query_p95,
                "contributor_summary_max": contributor_query_max,
            },
            "storage": {
                "database_bytes": database_bytes,
                "database_bytes_per_revision": database_bytes as f64 / timeline.revisions.len() as f64,
                "contributor_facts": contributor_fact_count,
                "landmarks": landmark_count,
                "database_bytes_per_contributor": (contributor_fact_count > 0)
                    .then(|| database_bytes as f64 / contributor_fact_count as f64),
                "database_bytes_per_landmark": (landmark_count > 0)
                    .then(|| database_bytes as f64 / landmark_count as f64),
                "snapshot_blob_bytes": snapshot_blob_bytes,
                "delta_blob_bytes": delta_blob_bytes,
            },
            "process": {
                "rss_kib": rss_kib,
                "cpu_user_seconds": user_cpu,
                "cpu_system_seconds": system_cpu,
                "input_blocks": input_blocks,
                "output_blocks": output_blocks,
            },
        });
        fs::write(
            &report_path,
            serde_json::to_vec_pretty(&report).expect("report JSON"),
        )
        .expect("write history benchmark report");
        eprintln!("benchmark report:      {report_path}");
    }

    drop(connection);
    let _ = fs::remove_file(db_path);
}

fn run_git(root: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?}");
}

fn run_git_at(root: &Path, arguments: &[&str], timestamp: &str) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp)
        .status()
        .expect("git");
    assert!(status.success(), "git {arguments:?} at {timestamp}");
}

fn release_catalog_state(
    connection: &Connection,
    repo_path: &str,
) -> (String, Vec<(String, String, String)>) {
    let identity = connection
        .query_row(
            "SELECT index_identity FROM history_graph_release_catalogs WHERE repo_path = ?1",
            [repo_path],
            |row| row.get(0),
        )
        .expect("catalog identity");
    let mut statement = connection
        .prepare(
            "SELECT tag, tag_kind, revision_sha FROM history_graph_release_tags
             WHERE repo_path = ?1 ORDER BY tag",
        )
        .expect("release rows");
    let rows = statement
        .query_map([repo_path], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query release rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("read release rows");
    (identity, rows)
}

fn publish_catalog(
    connection: &Connection,
    timeline: &HistoryTimeline,
    tags: &[GitTagRecord],
    fingerprint: &str,
) -> Result<String, String> {
    let publication = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let identity = publish_release_catalog(&publication, timeline, tags, fingerprint, true)?;
    publication.commit().map_err(|error| error.to_string())?;
    Ok(identity)
}
