use rusqlite::Connection;

const MIGRATION_SQL: &str = include_str!("schema/structural_graph.sql");

pub fn run_migration(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(MIGRATION_SQL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn canonical_structural_graph_schema_is_indexed_and_idempotent() {
        let conn = Connection::open_in_memory().expect("database");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys");

        run_migration(&conn).expect("first migration");
        run_migration(&conn).expect("idempotent migration");

        let tables = schema_objects(&conn, "table", "structural_graph_%");
        assert_eq!(
            tables,
            BTreeSet::from([
                "structural_graph_clone_groups".to_string(),
                "structural_graph_communities".to_string(),
                "structural_graph_diagnostics".to_string(),
                "structural_graph_edges".to_string(),
                "structural_graph_file_cursors".to_string(),
                "structural_graph_metric_facts".to_string(),
                "structural_graph_nodes".to_string(),
                "structural_graph_snapshot_files".to_string(),
                "structural_graph_snapshots".to_string(),
                "structural_graph_sources".to_string(),
            ])
        );

        let indexes = schema_objects(&conn, "index", "idx_structural_graph_%");
        for required in [
            "idx_structural_graph_edges_from",
            "idx_structural_graph_edges_to",
            "idx_structural_graph_nodes_path",
            "idx_structural_graph_snapshot_files_disposition",
            "idx_structural_graph_snapshots_repo_created",
            "idx_structural_graph_sources_path",
        ] {
            assert!(indexes.contains(required), "missing {required}");
        }
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
