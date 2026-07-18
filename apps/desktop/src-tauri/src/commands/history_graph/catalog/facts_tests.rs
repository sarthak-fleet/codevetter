use super::*;
use crate::commands::history_graph::history_facts::{
    HistoryIdentityFact, HistoryPathFact, HistoryRevisionFact,
};
use rusqlite::Connection;

const REPO: &str = "/normalized-fixture";
const FIRST: &str = "1111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222";

#[test]
fn normalized_generation_is_private_deterministic_and_single_counts_primary_churn() {
    let connection = database();
    let build = fixture_build();
    let tags = fixture_tags();
    let first_identity = publish(&connection, &build, &tags).expect("first publish");
    let second_identity = publish(&connection, &build, &tags).expect("repeat publish");
    assert_eq!(first_identity, second_identity);

    assert_eq!(count(&connection, "history_graph_fact_tags"), 3);
    assert_eq!(count(&connection, "history_graph_revision_paths"), 3);
    assert_eq!(count(&connection, "history_graph_contributors"), 4);
    assert_eq!(role_count(&connection, "primary"), 2);
    assert_eq!(role_count(&connection, "coauthor"), 3);
    assert_eq!(
        connection
            .query_row(
                "SELECT sum(additions) FROM history_graph_revision_paths WHERE repo_path = ?1",
                [REPO],
                |row| row.get::<_, i64>(0),
            )
            .expect("single-counted churn"),
        12
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM history_graph_revision_paths
                 WHERE repo_path = ?1 AND binary = 1 AND generated = 1 AND vendored = 1",
                [REPO],
                |row| row.get::<_, i64>(0),
            )
            .expect("classified path"),
        1
    );
    let identity_kinds = query_strings(
        &connection,
        "SELECT identity_kind FROM history_graph_contributors
         WHERE repo_path = ?1 ORDER BY identity_kind",
    );
    assert_eq!(identity_kinds, ["automation", "human", "human", "unknown"]);
    let tag_names = query_strings(
        &connection,
        "SELECT tag FROM history_graph_fact_tags WHERE repo_path = ?1 ORDER BY tag",
    );
    assert_eq!(tag_names, ["nightly", "v1.0.0", "v9.9.9-divergent"]);
    assert_no_raw_emails(&connection);
}

#[test]
fn replacement_removes_stale_facts_and_forced_failure_rolls_back_every_table() {
    let connection = database();
    let build = fixture_build();
    let tags = fixture_tags();
    let initial_identity = publish(&connection, &build, &tags).expect("initial publish");
    let initial_state = state(&connection);

    let mut replacement = build.clone();
    replacement
        .facts_by_revision
        .get_mut(SECOND)
        .expect("second")
        .paths
        .clear();
    replacement
        .facts_by_revision
        .get_mut(SECOND)
        .expect("second")
        .coauthors
        .clear();
    replacement.facts_fingerprint = "facts:replacement".to_string();
    let replacement_tags = tags[..1].to_vec();

    {
        let transaction = connection
            .unchecked_transaction()
            .expect("failure transaction");
        assert!(publish_history_facts_forced_failure(
            &transaction,
            &replacement,
            &replacement_tags,
        )
        .expect_err("forced failure")
        .contains("Forced"));
        // Dropping the uncommitted transaction is the failure/cancellation boundary.
    }
    assert_eq!(state(&connection), initial_state);
    assert_eq!(catalog_identity(&connection), initial_identity);

    let mut overflow = replacement.clone();
    overflow
        .facts_by_revision
        .get_mut(FIRST)
        .expect("first")
        .paths[0]
        .additions = Some(u64::MAX);
    {
        let transaction = connection
            .unchecked_transaction()
            .expect("overflow transaction");
        assert!(publish_history_facts(
            &transaction,
            &overflow,
            &replacement_tags,
            "overflow",
            &StructuralGraphCancellation::default(),
        )
        .expect_err("overflow")
        .contains("exceed SQLite range"));
    }
    assert_eq!(state(&connection), initial_state);

    let replacement_identity =
        publish(&connection, &replacement, &replacement_tags).expect("replacement publish");
    assert_ne!(replacement_identity, initial_identity);
    assert_eq!(count(&connection, "history_graph_fact_tags"), 1);
    assert_eq!(count(&connection, "history_graph_revision_paths"), 1);
    assert_eq!(role_count(&connection, "coauthor"), 1);
    assert_eq!(count(&connection, "history_graph_contributors"), 3);
    assert_no_raw_emails(&connection);
}

#[test]
fn cancelled_generation_leaves_the_prior_ready_identity_untouched() {
    let connection = database();
    let build = fixture_build();
    let identity = publish(&connection, &build, &fixture_tags()).expect("ready generation");
    let before = state(&connection);
    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel();
    {
        let transaction = connection
            .unchecked_transaction()
            .expect("cancelled transaction");
        assert!(publish_history_facts(
            &transaction,
            &build,
            &fixture_tags(),
            "cancelled",
            &cancellation,
        )
        .expect_err("cancelled publication")
        .contains("cancelled"));
    }
    assert_eq!(state(&connection), before);
    assert_eq!(catalog_identity(&connection), identity);
}

fn fixture_build() -> HistoryTimelineBuild {
    let human = identity(
        "contributor:human",
        "Canonical Dev",
        HistoryAutomationKind::Human,
    );
    let bot = identity(
        "contributor:automation",
        "Build Bot [bot]",
        HistoryAutomationKind::Automation,
    );
    let unknown = identity(
        "contributor:unknown",
        "Unknown",
        HistoryAutomationKind::Unknown,
    );
    let reviewer = identity(
        "contributor:reviewer",
        "Reviewer",
        HistoryAutomationKind::Human,
    );
    let first = HistoryRevisionFact {
        sha: FIRST.to_string(),
        parents: Vec::new(),
        committed_at: "2026-01-01T00:00:00Z".to_string(),
        subject: "initial".to_string(),
        primary: human.clone(),
        coauthors: vec![reviewer.clone(), reviewer],
        malformed_coauthor_count: 0,
        tags: vec!["v1.0.0".to_string()],
        paths: vec![path("src/lib.rs", 5, false, false, false)],
        is_merge: false,
        is_head: false,
    };
    let second = HistoryRevisionFact {
        sha: SECOND.to_string(),
        parents: vec![FIRST.to_string()],
        committed_at: "2026-01-02T00:00:00Z".to_string(),
        subject: "generated update".to_string(),
        primary: bot,
        coauthors: vec![unknown, human],
        malformed_coauthor_count: 0,
        tags: vec!["nightly".to_string()],
        paths: vec![
            path("generated/client.ts", 7, false, true, false),
            HistoryPathFact {
                path: "vendor/blob.bin".to_string(),
                old_path: None,
                status: HistoryPathStatus::Added,
                additions: None,
                deletions: None,
                binary: true,
                generated: true,
                vendored: true,
            },
        ],
        is_merge: false,
        is_head: true,
    };
    let facts_by_revision = [(FIRST.to_string(), first), (SECOND.to_string(), second)]
        .into_iter()
        .collect();
    HistoryTimelineBuild {
        timeline: HistoryTimeline {
            schema_version: 1,
            repo_path: REPO.to_string(),
            head: SECOND.to_string(),
            generated_at: "2026-01-02T00:00:00Z".to_string(),
            revisions: Vec::new(),
            total_commits: 2,
            truncated: false,
            is_shallow: false,
            coverage_complete: true,
            release_ranges: Vec::new(),
            reachable_revisions: vec![FIRST.to_string(), SECOND.to_string()],
        },
        fact_git_process_count: 1,
        facts_by_revision,
        mailmap_fingerprint: "mailmap:fixture".to_string(),
        facts_fingerprint: "facts:fixture".to_string(),
    }
}

fn fixture_tags() -> Vec<GitTagRecord> {
    vec![
        GitTagRecord {
            name: "nightly".to_string(),
            object_sha: SECOND.to_string(),
            commit_sha: SECOND.to_string(),
            created_ts: 2,
        },
        GitTagRecord {
            name: "v1.0.0".to_string(),
            object_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            commit_sha: FIRST.to_string(),
            created_ts: 1,
        },
        GitTagRecord {
            name: "v9.9.9-divergent".to_string(),
            object_sha: "9999999999999999999999999999999999999999".to_string(),
            commit_sha: "9999999999999999999999999999999999999999".to_string(),
            created_ts: 3,
        },
    ]
}

fn identity(id: &str, name: &str, automation: HistoryAutomationKind) -> HistoryIdentityFact {
    HistoryIdentityFact {
        contributor_id: id.to_string(),
        display_name: name.to_string(),
        automation,
        alias_count: 0,
    }
}

fn path(
    path: &str,
    additions: u64,
    binary: bool,
    generated: bool,
    vendored: bool,
) -> HistoryPathFact {
    HistoryPathFact {
        path: path.to_string(),
        old_path: None,
        status: HistoryPathStatus::Added,
        additions: Some(additions),
        deletions: Some(0),
        binary,
        generated,
        vendored,
    }
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, 'repo', 'ready', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [REPO],
        )
        .expect("repository");
    connection
}

fn publish(
    connection: &Connection,
    build: &HistoryTimelineBuild,
    tags: &[GitTagRecord],
) -> Result<String, String> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let identity = publish_history_facts(
        &transaction,
        build,
        tags,
        "2026-01-02T00:00:00Z",
        &StructuralGraphCancellation::default(),
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(identity)
}

fn count(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(
            &format!("SELECT count(*) FROM {table} WHERE repo_path = ?1"),
            [REPO],
            |row| row.get(0),
        )
        .expect("table count")
}

fn role_count(connection: &Connection, role: &str) -> i64 {
    connection
        .query_row(
            "SELECT count(*) FROM history_graph_revision_contributors
             WHERE repo_path = ?1 AND role = ?2",
            [REPO, role],
            |row| row.get(0),
        )
        .expect("role count")
}

fn query_strings(connection: &Connection, sql: &str) -> Vec<String> {
    let mut statement = connection.prepare(sql).expect("string query");
    statement
        .query_map([REPO], |row| row.get(0))
        .expect("string rows")
        .collect::<Result<_, _>>()
        .expect("strings")
}

fn catalog_identity(connection: &Connection) -> String {
    connection
        .query_row(
            "SELECT index_identity FROM history_graph_fact_catalogs WHERE repo_path = ?1",
            [REPO],
            |row| row.get(0),
        )
        .expect("fact identity")
}

fn state(connection: &Connection) -> (String, i64, i64, i64, i64) {
    (
        catalog_identity(connection),
        count(connection, "history_graph_fact_tags"),
        count(connection, "history_graph_revision_paths"),
        count(connection, "history_graph_contributors"),
        count(connection, "history_graph_revision_contributors"),
    )
}

fn assert_no_raw_emails(connection: &Connection) {
    let mut values = Vec::new();
    for (table, columns) in [
        (
            "history_graph_fact_catalogs",
            "index_identity || indexed_head || tags_fingerprint || mailmap_fingerprint || facts_fingerprint",
        ),
        (
            "history_graph_contributors",
            "contributor_id || display_name || identity_kind",
        ),
        (
            "history_graph_revision_contributors",
            "revision_sha || contributor_id || role",
        ),
        (
            "history_graph_fact_tags",
            "tag || revision_sha || tag_object_sha || tag_kind",
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT {columns} FROM {table} WHERE repo_path = ?1"))
            .expect("privacy query");
        values.extend(
            statement
                .query_map([REPO], |row| row.get::<_, String>(0))
                .expect("privacy rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("privacy values"),
        );
    }
    let stored = values.join(" ");
    assert!(!stored.contains('@'));
    assert!(!stored.contains("example.test"));
}
