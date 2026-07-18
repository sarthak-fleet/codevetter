use super::*;
use std::fs;

#[test]
fn evidence_hydration_returns_only_selected_bounded_fields() {
    let root = std::env::temp_dir().join(format!("cv-history-read-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "fixture@local"]);
    run_git(&root, &["config", "user.name", "Fixture"]);
    fs::write(root.join("main.rs"), "fn main() {}\n").expect("file");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "initial"]);
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("schema");
    let canonical = root
        .canonicalize()
        .expect("canonical")
        .to_string_lossy()
        .to_string();
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status,
                created_at, updated_at
             ) VALUES (?1, 'fixture', 'head', 'ready', '2026-01-01T00:00:00Z',
                '2026-01-01T00:00:00Z')",
            [&canonical],
        )
        .expect("repo");
    connection
        .execute(
            "INSERT INTO history_graph_events (
                id, repo_path, event_kind, trust, origin, source_id,
                payload_json, evidence_json, recorded_at
             ) VALUES ('event', ?1, 'verification', 'extracted', 'metadata', 'test',
                '{\"summary\":\"passed\",\"secret\":\"must-not-return\"}', '[]',
                '2026-01-01T00:00:00Z')",
            [&canonical],
        )
        .expect("event");
    let evidence_json = serde_json::json!([{
        "path": "main.rs",
        "start_line": 1,
        "start_column": 1,
        "end_line": 1,
        "end_column": 10,
        "excerpt": null
    }])
    .to_string();
    connection
        .execute(
            "UPDATE history_graph_events SET evidence_json = ?1 WHERE id = 'event'",
            [&evidence_json],
        )
        .expect("relative source evidence");
    let service = HistoryReadService::new(&connection, &canonical).expect("service");
    let details = service.evidence(&["event".to_string()]).expect("evidence");
    assert!(details[0].available);
    let encoded = serde_json::to_string(&details).expect("json");
    assert!(encoded.contains("passed"));
    assert!(!encoded.contains("must-not-return"));
    let _ = fs::remove_dir_all(root);
}

fn run_git(root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success());
}
