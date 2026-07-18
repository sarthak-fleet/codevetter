use super::*;
use std::fs;

#[test]
fn adapter_registry_is_local_only_and_external_sources_require_consent() {
    let connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let root = std::env::temp_dir();
    let descriptors = adapter_descriptors(&connection, &root).expect("descriptors");
    assert!(descriptors.iter().all(|adapter| adapter.local_only));
    assert!(descriptors.iter().all(|adapter| !adapter.network_access));
    let hosted = descriptors
        .iter()
        .find(|adapter| adapter.id == "hosted-provider")
        .expect("hosted provider");
    assert_eq!(hosted.consent, HistoryAdapterConsent::ExplicitImport);
    assert_eq!(
        hosted.availability,
        HistoryAdapterAvailability::NeedsConfiguration
    );
}

#[test]
fn evidence_ids_are_stable_and_source_scoped() {
    let first = deterministic_evidence_id("reviews", "review-1", Some("2026-01-01"));
    assert_eq!(
        first,
        deterministic_evidence_id("reviews", "review-1", Some("2026-01-01"))
    );
    assert_ne!(
        first,
        deterministic_evidence_id("synthetic-qa", "review-1", Some("2026-01-01"))
    );
}

#[test]
fn built_in_refresh_normalizes_local_records_without_network_or_duplicates() {
    let root = std::env::temp_dir().join(format!("cv-history-evidence-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join(".planning")).expect("fixture");
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .arg("init")
        .status()
        .expect("git init")
        .success());
    fs::write(root.join(".planning/decision.md"), "Keep evidence local.\n").expect("decision");
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", ".planning/decision.md"])
        .status()
        .expect("git add")
        .success());

    let canonical = root.canonicalize().expect("canonical");
    let canonical_text = canonical.to_string_lossy().to_string();
    let mut connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    connection
        .execute(
            "INSERT INTO local_reviews (
                    id, repo_path, status, summary_markdown, created_at
                 ) VALUES ('review-1', ?1, 'complete', 'Review passed',
                    '2026-01-01T00:00:00Z')",
            params![canonical_text],
        )
        .expect("review");
    connection
        .execute(
            "INSERT INTO synthetic_qa_runs (
                    id, repo_path, loop_id, runner_type, goal, pass, created_at
                 ) VALUES ('qa-1', ?1, 'loop-1', 'playwright', 'open app', 1,
                    '2026-01-02T00:00:00Z')",
            params![canonical_text],
        )
        .expect("qa");
    connection
        .execute(
            "INSERT INTO cc_projects (id, display_name, dir_path, created_at)
                 VALUES ('project-1', 'fixture', ?1, '2026-01-01T00:00:00Z')",
            params![canonical_text],
        )
        .expect("project");
    connection
        .execute(
            "INSERT INTO cc_sessions (
                    id, project_id, agent_type, message_count, indexed_at
                 ) VALUES ('session-1', 'project-1', 'codex', 12,
                    '2026-01-03T00:00:00Z')",
            [],
        )
        .expect("session");

    let first = refresh_builtin_adapters(&mut connection, &canonical).expect("refresh");
    assert_eq!(first.imported, 4);
    assert_eq!(first.network_requests, 0);
    let second = refresh_builtin_adapters(&mut connection, &canonical).expect("repeat");
    assert_eq!(second.imported, 0);
    assert_eq!(second.already_present, 4);
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn provider_export_keeps_delivery_separate_and_bounded() {
    let root = std::env::temp_dir().join(format!("cv-provider-export-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("fixture");
    let canonical = root.canonicalize().expect("canonical");
    let export = HistoryLocalEvidenceExport {
        schema_version: 1,
        source: "posthog-export".to_string(),
        cursor: Some("cursor-1".to_string()),
        records: vec![HistoryLocalEvidenceExportRecord {
            id: "delivery-1".to_string(),
            event_kind: "analytics_provider_delivery".to_string(),
            observed_at: "2026-01-04T00:00:00Z".to_string(),
            effective_at: Some("2026-01-03T23:59:00Z".to_string()),
            summary: "x".repeat(2_000),
            entity_ids: vec!["event:signup".to_string()],
            release_ids: vec!["v1.0.0".to_string()],
            source_paths: Vec::new(),
            episode_keys: vec!["deploy:production-42".to_string()],
        }],
    };
    let records = normalize_local_export(export).expect("normalize export");
    assert_eq!(records[0].summary.chars().count(), 1_000);
    assert!(records[0].redacted);
    let mut connection = Connection::open_in_memory().expect("database");
    crate::db::schema::run_migrations(&connection).expect("migrations");
    let result = persist_imported_records(
        &mut connection,
        &canonical,
        &records,
        "2026-01-04T00:00:00Z",
    )
    .expect("persist export");
    assert_eq!(result.imported, 1);
    assert_eq!(result.network_requests, 0);
    let stored: (String, String) = connection
        .query_row(
            "SELECT event_kind, entity_id FROM history_graph_events",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("stored provider event");
    assert_eq!(stored.0, "analytics_provider_delivery");
    assert_eq!(stored.1, "event:signup");
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn provider_export_rejects_unsupported_adapter_events() {
    let error = normalize_local_export(HistoryLocalEvidenceExport {
        schema_version: 1,
        source: "provider-export".to_string(),
        cursor: None,
        records: vec![HistoryLocalEvidenceExportRecord {
            id: "record-1".to_string(),
            event_kind: "unconfigured_network_probe".to_string(),
            observed_at: "2026-01-04T00:00:00Z".to_string(),
            effective_at: None,
            summary: "must not run".to_string(),
            entity_ids: Vec::new(),
            release_ids: Vec::new(),
            source_paths: Vec::new(),
            episode_keys: Vec::new(),
        }],
    })
    .expect_err("unsupported adapter event");

    assert!(error.contains("Unsupported local evidence event kind"));
}

#[test]
fn provider_export_redacts_credentials_before_persistence() {
    let records = normalize_local_export(HistoryLocalEvidenceExport {
        schema_version: 1,
        source: "provider-export".to_string(),
        cursor: Some("password=cursor-secret-value".to_string()),
        records: vec![HistoryLocalEvidenceExportRecord {
            id: "record-1".to_string(),
            event_kind: "incident".to_string(),
            observed_at: "2026-01-04T00:00:00Z".to_string(),
            effective_at: None,
            summary: "Authorization: Bearer imported-secret-token".to_string(),
            entity_ids: vec!["service:billing".to_string()],
            release_ids: Vec::new(),
            source_paths: vec![
                "secrets/provider.json".to_string(),
                "src/safe.rs".to_string(),
            ],
            episode_keys: Vec::new(),
        }],
    })
    .expect("normalize credential-bearing export");
    assert_eq!(records[0].summary, "[redacted]");
    assert!(records[0].source_cursor.is_none());
    assert!(records[0].redacted);
    assert_eq!(records[0].sources.len(), 1);
    assert_eq!(records[0].sources[0].path, "src/safe.rs");
}
