use super::*;
use crate::db::history_graph_schema::run_migration;
use rusqlite::Connection;

#[test]
fn publishes_repository_scoped_deterministic_structural_landmarks() {
    let connection = fixture_database();
    seed_history(&connection, "/fork-a", 20, 2);
    seed_history(&connection, "/fork-b", 20, 2);
    seed_structural_delta(&connection, "/fork-a", "extreme-00", None);

    let (generation_a, rows_a) = publish_and_read(&connection, "/fork-a", "index-a");
    let (_, rows_b) = publish_and_read(&connection, "/fork-b", "index-b");
    let (generation_again, rows_again) = publish_and_read(&connection, "/fork-a", "index-a");

    assert_eq!(generation_a, generation_again);
    assert_eq!(
        rows_a, rows_again,
        "same index rebuild is byte-deterministic"
    );
    assert_ne!(rows_a[0].0, rows_b[0].0, "forks scope landmark IDs");
    let components: serde_json::Value = serde_json::from_str(&rows_a[0].2).expect("components");
    assert_eq!(components["structural"]["node_changes"], 4);
    assert_eq!(components["structural"]["edge_changes"], 3);
    assert_eq!(components["structural"]["community_changes"], 2);
    assert_eq!(components["structural"]["hub_changes"], 1);
    assert_eq!(components["structural"]["bridge_changes"], 1);
    assert!(rows_a[0].3.contains("Persisted structural delta observed"));
    assert_eq!(rows_a[0].1, "qualified");
}

#[test]
fn persists_merge_binary_generated_vendor_release_and_structural_caveats() {
    let connection = fixture_database();
    seed_history(&connection, "/caveats", 20, 0);
    insert_revision(&connection, "/caveats", "merge-noise", 30, true);
    for index in 0..100 {
        let path = format!("release-notes/note-{index}.md");
        insert_path(
            &connection,
            "/caveats",
            "merge-noise",
            &path,
            Some(10_000),
            false,
            index < 50,
            index >= 50,
        );
    }
    seed_structural_delta(
        &connection,
        "/caveats",
        "merge-noise",
        Some("checkpoint bounded"),
    );
    insert_revision(&connection, "/caveats", "binary-large", 31, false);
    for index in 0..80 {
        insert_path(
            &connection,
            "/caveats",
            "binary-large",
            &format!("assets/{index}.bin"),
            None,
            true,
            false,
            false,
        );
    }

    let (_, rows) = publish_and_read(&connection, "/caveats", "index-caveats");
    let caveats = rows
        .iter()
        .map(|row| row.4.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for expected in [
        "generated",
        "vendored",
        "release-only",
        "merge revision",
        "binary files",
        "checkpoint bounded",
        "does not establish intent",
    ] {
        assert!(caveats.contains(expected), "missing caveat: {expected}");
    }
    assert!(rows.iter().any(|row| row.1 == "qualified_partial"));
}

#[test]
fn unavailable_baseline_publishes_an_explicit_empty_generation() {
    let connection = fixture_database();
    seed_history(&connection, "/small", 11, 0);

    let (generation, rows) = publish_and_read(&connection, "/small", "index-small");
    assert!(!generation.is_empty());
    assert!(rows.is_empty());
    let (status, count, coverage): (String, i64, String) = connection
        .query_row(
            "SELECT status, landmark_count, coverage_json
             FROM history_graph_landmark_generations WHERE repo_path = '/small'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("generation coverage");
    assert_eq!(status, "unavailable");
    assert_eq!(count, 0);
    assert!(coverage.contains("requires 12"));
}

#[test]
fn cancellation_and_failure_keep_the_previous_atomic_generation() {
    let connection = fixture_database();
    seed_history(&connection, "/atomic", 20, 1);
    let before = publish_and_read(&connection, "/atomic", "index-before");

    {
        let transaction = connection.unchecked_transaction().expect("transaction");
        assert!(publish_candidate_inflections_forced_failure(
            &transaction,
            "/atomic",
            "index-after"
        )
        .is_err());
        // Drop rolls back the delete performed before the forced failure.
    }
    assert_eq!(publish_state(&connection, "/atomic"), before);

    {
        let cancellation = StructuralGraphCancellation::default();
        cancellation.cancel();
        let transaction = connection.unchecked_transaction().expect("transaction");
        assert!(publish_candidate_inflections(
            &transaction,
            "/atomic",
            "index-cancelled",
            true,
            "cancelled",
            &cancellation
        )
        .is_err());
    }
    assert_eq!(publish_state(&connection, "/atomic"), before);
}

#[test]
fn storage_is_capped_at_the_published_landmark_bound() {
    let connection = fixture_database();
    seed_history(&connection, "/bounded", 1_200, 600);

    let (_, rows) = publish_and_read(&connection, "/bounded", "index-bounded");
    assert_eq!(rows.len(), MAX_PUBLISHED_INFLECTIONS);
    let coverage: String = connection
        .query_row(
            "SELECT coverage_json FROM history_graph_landmark_generations
             WHERE repo_path = '/bounded'",
            [],
            |row| row.get(0),
        )
        .expect("bounded coverage");
    assert!(coverage.contains("\"storage_truncated\":true"));
    assert!(coverage.contains("\"published_limit\":512"));
}

type LandmarkRows = Vec<(String, String, String, String, String)>;

fn fixture_database() -> Connection {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    run_migration(&connection).expect("history migration");
    connection
}

fn seed_history(connection: &Connection, repo: &str, normal: usize, extreme: usize) {
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES (?1, ?2, 'ready', 'fixture', 'fixture')",
            params![repo, format!("fingerprint-{repo}")],
        )
        .expect("repository");
    for index in 0..normal {
        let sha = format!("normal-{index:05}");
        insert_revision(connection, repo, &sha, index as i64, false);
        insert_path(
            connection,
            repo,
            &sha,
            "src/lib.rs",
            Some(10 + index as u64 % 5),
            false,
            false,
            false,
        );
    }
    for index in 0..extreme {
        let sha = format!("extreme-{index:02}");
        insert_revision(connection, repo, &sha, (normal + index) as i64, false);
        for path in 0..8 {
            insert_path(
                connection,
                repo,
                &sha,
                &format!("src/extreme-{index}-{path}.rs"),
                Some(10_000 + index as u64),
                false,
                false,
                false,
            );
        }
    }
}

fn insert_revision(connection: &Connection, repo: &str, sha: &str, ordinal: i64, merge: bool) {
    connection
        .execute(
            "INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject, coverage_json
             ) VALUES (?1, ?2, ?3, 'fixture', 'Fixture', 'fixture', ?4)",
            params![
                repo,
                sha,
                ordinal,
                serde_json::json!({ "facts_schema_version": 1, "merge": merge }).to_string()
            ],
        )
        .expect("revision");
}

#[allow(clippy::too_many_arguments)]
fn insert_path(
    connection: &Connection,
    repo: &str,
    sha: &str,
    path: &str,
    churn: Option<u64>,
    binary: bool,
    generated: bool,
    vendored: bool,
) {
    connection
        .execute(
            "INSERT INTO history_graph_revision_paths (
                repo_path, revision_sha, path, change_kind, additions, deletions,
                binary, generated, vendored
             ) VALUES (?1, ?2, ?3, 'modified', ?4, ?5, ?6, ?7, ?8)",
            params![
                repo,
                sha,
                path,
                churn.map(|value| value / 2),
                churn.map(|value| value - value / 2),
                i64::from(binary),
                i64::from(generated),
                i64::from(vendored),
            ],
        )
        .expect("path");
}

fn seed_structural_delta(
    connection: &Connection,
    repo: &str,
    sha: &str,
    coverage_gap: Option<&str>,
) {
    let payload = serde_json::json!({
        "added_node_ids": ["n1", "n2"],
        "removed_node_ids": ["n3"],
        "changed_node_ids": ["n4"],
        "added_edge_ids": ["e1"],
        "removed_edge_ids": ["e2"],
        "changed_edge_ids": ["e3"],
        "added_community_ids": ["c1"],
        "removed_community_ids": ["c2"],
        "added_hub_ids": ["h1"],
        "removed_hub_ids": [],
        "added_bridge_ids": [],
        "removed_bridge_ids": ["b1"],
        "coverage_gap": coverage_gap,
    });
    connection
        .execute(
            "INSERT INTO history_graph_events (
                id, repo_path, revision_sha, event_kind, trust, origin,
                source_id, payload_json, recorded_at
             ) VALUES (?1, ?2, ?3, 'structural_delta', 'extracted', 'analysis',
                'fixture', ?4, 'fixture')",
            params![
                format!("delta-{repo}-{sha}"),
                repo,
                sha,
                payload.to_string()
            ],
        )
        .expect("structural delta");
}

fn publish_and_read(connection: &Connection, repo: &str, identity: &str) -> (String, LandmarkRows) {
    let transaction = connection.unchecked_transaction().expect("transaction");
    let generation = publish_candidate_inflections(
        &transaction,
        repo,
        identity,
        true,
        "fixture",
        &StructuralGraphCancellation::default(),
    )
    .expect("publish landmarks");
    transaction.commit().expect("commit landmarks");
    (generation, read_rows(connection, repo))
}

fn publish_state(connection: &Connection, repo: &str) -> (String, LandmarkRows) {
    let generation = connection
        .query_row(
            "SELECT generation_id FROM history_graph_landmark_generations WHERE repo_path = ?1",
            [repo],
            |row| row.get(0),
        )
        .expect("generation");
    (generation, read_rows(connection, repo))
}

fn read_rows(connection: &Connection, repo: &str) -> LandmarkRows {
    let mut statement = connection
        .prepare(
            "SELECT id, trust, components_json, reasons_json, caveats_json
             FROM history_graph_landmarks WHERE repo_path = ?1
             ORDER BY score_milli DESC, ordinal, id",
        )
        .expect("landmark rows");
    statement
        .query_map([repo], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .expect("query landmark rows")
        .collect::<Result<_, _>>()
        .expect("read landmark rows")
}
