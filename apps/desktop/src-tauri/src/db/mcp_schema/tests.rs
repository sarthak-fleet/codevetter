use super::*;
use std::collections::BTreeSet;

#[test]
fn scope_and_audit_schema_are_local_metadata_only_and_idempotent() {
    let connection = Connection::open_in_memory().expect("database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE history_graph_repositories (repo_path TEXT PRIMARY KEY);",
        )
        .expect("history prerequisite");

    run_migration(&connection).expect("first migration");
    run_migration(&connection).expect("idempotent migration");

    assert_eq!(
        table_columns(&connection, "mcp_repository_scopes"),
        BTreeSet::from([
            "created_at".to_string(),
            "enabled".to_string(),
            "repo_id".to_string(),
            "repo_path".to_string(),
            "updated_at".to_string(),
        ])
    );
    let audit = table_columns(&connection, "mcp_access_audit");
    assert_eq!(
        audit,
        BTreeSet::from([
            "created_at".to_string(),
            "duration_ms".to_string(),
            "id".to_string(),
            "operation".to_string(),
            "repo_id".to_string(),
            "response_bytes".to_string(),
            "result_count".to_string(),
            "server_session".to_string(),
            "status".to_string(),
        ])
    );
    for forbidden in ["arguments", "query", "prompt", "content", "evidence"] {
        assert!(!audit.iter().any(|column| column.contains(forbidden)));
    }
}

fn table_columns(connection: &Connection, table: &str) -> BTreeSet<String> {
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("prepare columns");
    statement
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query columns")
        .collect::<Result<_, _>>()
        .expect("columns")
}
