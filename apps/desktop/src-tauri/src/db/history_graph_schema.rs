use rusqlite::Connection;

const MIGRATION_SQL: &str = include_str!("schema/history_graph.sql");
const RELEASE_CATALOG_MIGRATION_SQL: &str =
    include_str!("schema/history_graph_release_catalog.sql");
const FACTS_MIGRATION_SQL: &str = include_str!("schema/history_graph_facts.sql");
const RELEASE_INTERVALS_MIGRATION_SQL: &str =
    include_str!("schema/history_graph_release_intervals.sql");
const LANDMARKS_MIGRATION_SQL: &str = include_str!("schema/history_graph_landmarks.sql");

pub fn run_migration(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(MIGRATION_SQL)?;
    run_additive_migrations(conn)
}

fn run_additive_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(RELEASE_CATALOG_MIGRATION_SQL)?;
    conn.execute_batch(FACTS_MIGRATION_SQL)?;
    conn.execute_batch(RELEASE_INTERVALS_MIGRATION_SQL)?;
    conn.execute_batch(LANDMARKS_MIGRATION_SQL)?;
    let _ = conn.execute(
        "ALTER TABLE history_graph_release_catalogs ADD COLUMN interval_schema_version INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_release_catalogs ADD COLUMN interval_identity TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_revision_paths ADD COLUMN binary INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_revision_paths ADD COLUMN generated INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_revision_paths ADD COLUMN vendored INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_contributors ADD COLUMN alias_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN decision TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN related_event_id TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_annotations ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_graph_events ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_history_graph_annotations_evidence
         ON history_graph_annotations(repo_path, related_event_id, created_at)",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn temporal_graph_schema_is_indexed_and_idempotent() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");

        run_migration(&conn).expect("first migration");
        run_migration(&conn).expect("idempotent migration");

        let tables = schema_objects(&conn, "table", "history_graph_%");
        assert_eq!(
            tables,
            BTreeSet::from([
                "history_graph_annotations".to_string(),
                "history_graph_checkpoints".to_string(),
                "history_graph_event_blobs".to_string(),
                "history_graph_events".to_string(),
                "history_graph_fact_catalogs".to_string(),
                "history_graph_fact_tags".to_string(),
                "history_graph_landmark_generations".to_string(),
                "history_graph_landmarks".to_string(),
                "history_graph_release_catalogs".to_string(),
                "history_graph_release_intervals".to_string(),
                "history_graph_release_tags".to_string(),
                "history_graph_repositories".to_string(),
                "history_graph_contributors".to_string(),
                "history_graph_revision_contributors".to_string(),
                "history_graph_revision_paths".to_string(),
                "history_graph_revisions".to_string(),
                "history_graph_snapshot_blobs".to_string(),
            ])
        );

        let indexes = schema_objects(&conn, "index", "idx_history_graph_%");
        for required in [
            "idx_history_graph_annotations_evidence",
            "idx_history_graph_events_entity",
            "idx_history_graph_events_revision",
            "idx_history_graph_fact_catalogs_identity",
            "idx_history_graph_fact_tags_revision",
            "idx_history_graph_landmark_generation_identity",
            "idx_history_graph_landmarks_generation_score",
            "idx_history_graph_landmarks_revision",
            "idx_history_graph_paths_path",
            "idx_history_graph_revision_contributor",
            "idx_history_graph_revision_primary",
            "idx_history_graph_release_tags_revision",
            "idx_history_graph_release_intervals_boundary",
            "idx_history_graph_release_intervals_revision",
            "idx_history_graph_revisions_time",
        ] {
            assert!(indexes.contains(required), "missing {required}");
        }
    }

    #[test]
    fn existing_history_rows_survive_release_catalog_migration_and_repeat() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        conn.execute_batch(MIGRATION_SQL).expect("legacy schema");
        conn.execute_batch(
            "INSERT INTO history_graph_repositories (
                 repo_path, repository_fingerprint, indexed_head,
                 indexed_tags_fingerprint, status, created_at, updated_at
             ) VALUES ('/fixture', 'repo-1', 'head-1', 'tags-1', 'ready',
                 '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z');
             INSERT INTO history_graph_revisions (
                 repo_path, sha, ordinal, committed_at, author_name, subject,
                 parents_json, tags_json, is_release
             ) VALUES ('/fixture', 'release-sha', 0, '2026-01-01T00:00:00Z',
                 'Fixture', 'release', '[]', '[\"v1.0.0\",\"v1.0.0-stable\"]', 1);",
        )
        .expect("existing history");

        assert!(schema_objects(&conn, "table", "history_graph_release_%").is_empty());
        run_additive_migrations(&conn).expect("add release catalog");
        conn.execute(
            "INSERT INTO history_graph_release_catalogs (
                 repo_path, index_identity, indexed_head, tags_fingerprint,
                 status, coverage_json, updated_at
             ) VALUES ('/fixture', 'catalog-1', 'head-1', 'tags-1', 'ready',
                 '{\"complete\":true}', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("catalog identity");
        conn.execute_batch(
            "INSERT INTO history_graph_release_tags (
                 repo_path, tag, revision_sha, tag_object_sha, tag_kind, tagged_at
             ) VALUES
                 ('/fixture', 'v1.0.0', 'release-sha', 'release-sha',
                  'lightweight', 1767225600),
                 ('/fixture', 'v1.0.0-stable', 'release-sha', 'tag-object-sha',
                  'annotated', 1767229200);",
        )
        .expect("coincident release tags");
        run_migration(&conn).expect("repeat migration");

        let catalog_is_normalized_and_fresh: bool = conn
            .query_row(
                "SELECT COUNT(*) = 2
                        AND COUNT(DISTINCT revision_sha) = 1
                        AND EXISTS (
                            SELECT 1 FROM history_graph_release_catalogs
                            WHERE repo_path = '/fixture'
                              AND schema_version = 1
                              AND index_identity = 'catalog-1'
                              AND indexed_head = 'head-1'
                              AND tags_fingerprint = 'tags-1'
                              AND status = 'ready'
                        )
                 FROM history_graph_release_tags
                 WHERE repo_path = '/fixture'",
                [],
                |row| row.get(0),
            )
            .expect("release catalog");
        assert!(
            catalog_is_normalized_and_fresh,
            "catalog keeps one row per tag, groups by revision, and preserves index identity"
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM history_graph_revisions
                 WHERE repo_path = '/fixture' AND sha = 'release-sha'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("legacy revision"),
            1,
            "migration preserves existing history"
        );
    }

    #[test]
    fn legacy_event_and_annotation_tables_gain_additive_columns() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch(
            "CREATE TABLE history_graph_events (id TEXT PRIMARY KEY);
             CREATE TABLE history_graph_annotations (
                 id TEXT PRIMARY KEY,
                 repo_path TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );",
        )
        .expect("legacy schema");

        run_additive_migrations(&conn).expect("upgrade legacy schema");

        let event_columns = table_columns(&conn, "history_graph_events");
        assert!(event_columns.contains("schema_version"));
        let annotation_columns = table_columns(&conn, "history_graph_annotations");
        for required in ["decision", "related_event_id", "metadata_json"] {
            assert!(annotation_columns.contains(required), "missing {required}");
        }
        assert!(
            schema_objects(&conn, "index", "idx_history_graph_annotations_evidence")
                .contains("idx_history_graph_annotations_evidence")
        );
    }

    #[test]
    fn legacy_path_rows_survive_normalized_fact_migration_and_repeat() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");
        conn.execute_batch(MIGRATION_SQL)
            .expect("legacy history schema");
        conn.execute_batch(
            "CREATE TABLE history_graph_contributors (
                repo_path TEXT NOT NULL,
                contributor_id TEXT NOT NULL,
                display_name TEXT NOT NULL,
                identity_kind TEXT NOT NULL,
                PRIMARY KEY (repo_path, contributor_id)
             );
             INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, status, created_at, updated_at
             ) VALUES ('/legacy', 'repo', 'ready', '2026-01-01', '2026-01-01');
             INSERT INTO history_graph_revisions (
                repo_path, sha, ordinal, committed_at, author_name, subject
             ) VALUES ('/legacy', 'sha', 0, '2026-01-01', 'Legacy', 'legacy');
             INSERT INTO history_graph_contributors (
                repo_path, contributor_id, display_name, identity_kind
             ) VALUES ('/legacy', 'legacy-id', 'Legacy', 'human');
             INSERT INTO history_graph_revision_paths (
                repo_path, revision_sha, path, change_kind, additions, deletions
             ) VALUES ('/legacy', 'sha', 'src/lib.rs', 'modified', 2, 1);",
        )
        .expect("legacy facts");

        run_additive_migrations(&conn).expect("normalized fact migration");
        run_additive_migrations(&conn).expect("repeat normalized fact migration");

        let columns = table_columns(&conn, "history_graph_revision_paths");
        for required in ["binary", "generated", "vendored"] {
            assert!(columns.contains(required), "missing {required}");
        }
        let preserved: (i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT additions, deletions, binary, generated, vendored
                 FROM history_graph_revision_paths
                 WHERE repo_path = '/legacy' AND revision_sha = 'sha'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("preserved legacy path");
        assert_eq!(preserved, (2, 1, 0, 0, 0));
        for table in [
            "history_graph_fact_catalogs",
            "history_graph_fact_tags",
            "history_graph_landmark_generations",
            "history_graph_landmarks",
            "history_graph_revision_contributors",
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("empty additive table"),
                0
            );
        }
        assert!(table_columns(&conn, "history_graph_contributors").contains("alias_count"));
        assert_eq!(
            conn.query_row(
                "SELECT display_name, alias_count FROM history_graph_contributors
                 WHERE repo_path = '/legacy' AND contributor_id = 'legacy-id'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("preserved legacy contributor"),
            ("Legacy".to_string(), 0)
        );
    }

    fn table_columns(conn: &Connection, table: &str) -> BTreeSet<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .expect("prepare columns");
        statement
            .query_map([], |row| row.get(1))
            .expect("query columns")
            .collect::<Result<_, _>>()
            .expect("columns")
    }

    fn schema_objects(conn: &Connection, kind: &str, pattern: &str) -> BTreeSet<String> {
        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND name LIKE ?2")
            .expect("prepare schema lookup");
        statement
            .query_map([kind, pattern], |row| row.get(0))
            .expect("query schema")
            .collect::<Result<_, _>>()
            .expect("schema objects")
    }
}
