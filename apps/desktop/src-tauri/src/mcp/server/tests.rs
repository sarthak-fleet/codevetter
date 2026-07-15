use super::*;
use crate::commands::structural_graph::{
    storage::persist_snapshot,
    types::{
        StructuralGraphCoverage, StructuralGraphEngineInfo, StructuralGraphSnapshot,
        STRUCTURAL_GRAPH_SCHEMA_VERSION,
    },
};
use rmcp::{ClientHandler, ServiceExt};
use rusqlite::params;
use std::{fs, process::Command};

#[test]
fn every_tool_is_explicitly_read_only_and_schema_bounded() {
    let tools = tool_definitions();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        vec![
            "graph_query",
            "graph_get_node",
            "graph_get_neighbors",
            "graph_path",
            "graph_impact",
            "history_list_releases",
            "history_search",
            "history_get_state",
            "history_lineage",
            "history_explain",
            "history_trace",
            "history_compare",
            "history_get_evidence",
        ]
    );
    for tool in tools {
        let annotations = tool.annotations.expect("annotations");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.open_world_hint, Some(false));
        let output = tool.output_schema.expect("output schema");
        assert!(output.get("oneOf").is_some());
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&Value::Bool(false))
        );
        if tool.name == "history_trace" {
            assert!(tool.input_schema["properties"]["selector"]
                .get("oneOf")
                .is_some());
        }
    }
}

#[test]
fn lineage_cursor_pages_cover_each_result_once() {
    let mut offset = 0;
    let mut covered = Vec::new();
    loop {
        let (start, length, next) = lineage_page_bounds(5, 7, offset, 2);
        covered.extend(start..start + length);
        let Some(next) = next else {
            break;
        };
        let encoded = McpCursor::new("repo", "history_lineage", next, "entity:one")
            .encode()
            .expect("opaque cursor");
        offset = McpCursor::decode(&encoded, "repo", "history_lineage", "entity:one")
            .expect("decode cursor")
            .offset();
    }
    assert_eq!(covered, (0..7).collect::<Vec<_>>());
    assert_eq!(lineage_page_bounds(5, 7, 99, 2), (7, 0, None));
}

#[derive(Debug, Clone, Default)]
struct TestClient;

impl ClientHandler for TestClient {}

#[tokio::test]
async fn protocol_lifecycle_is_scoped_structured_and_live_revocable() {
    let fixture = tempfile::tempdir().expect("fixture");
    let repo = fixture.path().join("repo");
    fs::create_dir(&repo).expect("repo");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "fixture@codevetter.local"]);
    git(&repo, &["config", "user.name", "CodeVetter Fixture"]);
    fs::write(repo.join("main.rs"), "fn main() {}\n").expect("source");
    git(&repo, &["add", "main.rs"]);
    git(&repo, &["commit", "-m", "fixture release"]);
    git(&repo, &["tag", "v1.0.0"]);
    let head = git_output(&repo, &["rev-parse", "HEAD"]);
    let repo_path = repo
        .canonicalize()
        .expect("canonical repo")
        .to_string_lossy()
        .to_string();
    let database_path = fixture.path().join("codevetter.db");
    let connection = Connection::open(&database_path).expect("database");
    crate::db::schema::run_migrations(&connection).expect("schema");
    connection
        .execute(
            "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, indexed_head, status,
                    coverage_json, created_at, updated_at
                 ) VALUES (?1, 'fixture', ?2, 'ready', '{\"coverage_complete\":true}', ?3, ?3)",
            params![repo_path, head, "2026-01-01T00:00:00Z"],
        )
        .expect("history repository");
    connection
        .execute(
            "INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject,
                    parents_json, tags_json, is_release, is_head, coverage_json
                 ) VALUES (?1, ?2, 0, ?3, 'Fixture', 'fixture release', '[]',
                           '[\"v1.0.0\"]', 1, 1, '{}')",
            params![repo_path, head, "2026-01-01T00:00:00Z"],
        )
        .expect("history revision");
    connection
        .execute(
            "INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject,
                    parents_json, tags_json, is_release, is_head, coverage_json
                 ) VALUES (?1, '0000000000000000000000000000000000000001', -1, ?2,
                           'Fixture', 'older fixture release', '[]', '[\"v0.9.0\"]', 1, 0, '{}')",
            params![repo_path, "2025-01-01T00:00:00Z"],
        )
        .expect("older history revision");
    for ordinal in 2..=30 {
        connection
            .execute(
                "INSERT INTO history_graph_revisions (
                        repo_path, sha, ordinal, committed_at, author_name, subject,
                        parents_json, tags_json, is_release, is_head, coverage_json
                     ) VALUES (?1, ?2, ?3, ?4, 'Fixture', ?5, '[]', ?6, 1, 0, '{}')",
                params![
                    repo_path,
                    format!("fixture-release-{ordinal:038}"),
                    -ordinal,
                    format!("2024-01-{ordinal:02}T00:00:00Z"),
                    format!("fixture release {ordinal}"),
                    json!([format!("v0.{ordinal}.0")]).to_string(),
                ],
            )
            .expect("paginated history revision");
    }
    let repo_id = "repo_0123456789abcdef";
    connection
        .execute(
            "INSERT INTO mcp_repository_scopes (
                    repo_path, repo_id, enabled, created_at, updated_at
                 ) VALUES (?1, ?2, 1, ?3, ?3)",
            params![repo_path, repo_id, "2026-01-01T00:00:00Z"],
        )
        .expect("scope");
    persist_snapshot(
        &connection,
        &StructuralGraphSnapshot {
            id: "snapshot-fixture".to_string(),
            schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
            repo_path: repo_path.clone(),
            repo_head: Some(head.clone()),
            engine: StructuralGraphEngineInfo {
                id: "codevetter-tree-sitter".to_string(),
                version: "1".to_string(),
                bundled: true,
                syntax_aware: true,
                supported_languages: vec!["rust".to_string()],
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            cursor: None,
            ignore_fingerprint: None,
            coverage: StructuralGraphCoverage::default(),
            files: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            metrics: Vec::new(),
            clone_groups: Vec::new(),
            communities: Vec::new(),
            diagnostics: Vec::new(),
            truncated: false,
        },
    )
    .expect("snapshot");

    let server =
        CodeVetterMcpServer::new(database_path.clone(), repo_id.to_string()).expect("server");
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move {
        server
            .serve(server_transport)
            .await
            .expect("serve")
            .waiting()
            .await
            .expect("wait");
    });
    let client = TestClient.serve(client_transport).await.expect("client");
    let tools = client.list_tools(None).await.expect("tools");
    assert_eq!(tools.tools.len(), 13);
    assert!(tools.tools.iter().all(|tool| tool.output_schema.is_some()));
    assert!(client
        .call_tool(
            CallToolRequestParams::new("graph_query").with_arguments(
                json!({"unexpected": "rejected"})
                    .as_object()
                    .expect("arguments")
                    .clone(),
            ),
        )
        .await
        .is_err());
    let resources = client.list_resources(None).await.expect("resources");
    assert_eq!(resources.resources.len(), DEFAULT_PAGE_SIZE);
    let resource_cursor = resources.next_cursor.clone().expect("resource cursor");
    let second_resource_page = client
        .list_resources(Some(
            PaginatedRequestParams::default().with_cursor(Some(resource_cursor)),
        ))
        .await
        .expect("second resource page");
    assert!(!second_resource_page.resources.is_empty());
    assert!(resources
        .resources
        .iter()
        .all(|resource| !resource.uri.contains(&repo_path)));
    assert!(resources.resources.iter().all(|resource| {
        resource
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.last_modified.as_ref())
            .is_some()
    }));
    let snapshot_resource = resources
        .resources
        .iter()
        .find(|resource| resource.uri.contains("/snapshot/"))
        .expect("snapshot resource");
    let read = client
        .read_resource(ReadResourceRequestParams::new(
            snapshot_resource.uri.clone(),
        ))
        .await
        .expect("read snapshot resource");
    assert_eq!(read.contents.len(), 1);
    assert!(client
        .read_resource(ReadResourceRequestParams::new(format!(
            "codevetter-history://{repo_id}/snapshot/../evidence"
        )))
        .await
        .is_err());
    assert!(client
        .read_resource(ReadResourceRequestParams::new(
            HistoryResourceUri::new(repo_id, "evidence", "missing-evidence")
                .expect("missing evidence URI")
                .to_string(),
        ))
        .await
        .is_err());
    let result = client
        .call_tool(
            CallToolRequestParams::new("graph_query")
                .with_arguments(json!({"limit": 10}).as_object().expect("arguments").clone()),
        )
        .await
        .expect("graph query");
    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["schemaVersion"], 1);
    assert!(structured.to_string().find(&repo_path).is_none());
    let first_page = client
        .call_tool(
            CallToolRequestParams::new("history_list_releases")
                .with_arguments(json!({"limit": 1}).as_object().expect("arguments").clone()),
        )
        .await
        .expect("first release page")
        .structured_content
        .expect("first release page structured");
    let cursor = first_page["data"]["data"]["nextCursor"]
        .as_str()
        .expect("release cursor");
    let second_page = client
        .call_tool(
            CallToolRequestParams::new("history_list_releases").with_arguments(
                json!({"limit": 1, "cursor": cursor})
                    .as_object()
                    .expect("arguments")
                    .clone(),
            ),
        )
        .await
        .expect("second release page");
    assert_eq!(second_page.is_error, Some(false));
    let future_only = client
        .call_tool(
            CallToolRequestParams::new("history_list_releases").with_arguments(
                json!({
                    "history_filter": {"from": "2027-01-01T00:00:00Z"}
                })
                .as_object()
                .expect("arguments")
                .clone(),
            ),
        )
        .await
        .expect("filtered releases")
        .structured_content
        .expect("filtered releases structured");
    assert_eq!(
        future_only["data"]["data"]["result"]["revisions"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    let invalid_range = client
        .call_tool(
            CallToolRequestParams::new("history_search").with_arguments(
                json!({"query": "fixture", "history_filter": {"from": "not-a-date"}})
                    .as_object()
                    .expect("arguments")
                    .clone(),
            ),
        )
        .await;
    assert!(invalid_range.is_err());
    let (first, second, third) = tokio::join!(
        client.call_tool(CallToolRequestParams::new("graph_query")),
        client.call_tool(CallToolRequestParams::new("history_list_releases")),
        client.call_tool(
            CallToolRequestParams::new("history_get_evidence").with_arguments(
                json!({"ids": ["missing-evidence"]})
                    .as_object()
                    .expect("arguments")
                    .clone(),
            ),
        ),
    );
    assert!(first.expect("concurrent graph").is_error == Some(false));
    assert!(second.expect("concurrent releases").is_error == Some(false));
    assert!(third.expect("concurrent evidence").is_error == Some(false));

    connection
            .execute(
                "UPDATE history_graph_repositories SET indexed_head = 'stale-fixture-head' WHERE repo_path = ?1",
                [&repo_path],
            )
            .expect("stale history");
    let stale = client
        .call_tool(CallToolRequestParams::new("history_list_releases"))
        .await
        .expect("stale history response")
        .structured_content
        .expect("stale history structured");
    assert_eq!(stale["freshness"]["history"]["stale"], true);
    let repository_resource = resources
        .resources
        .iter()
        .find(|resource| resource.uri.contains("/repository/"))
        .expect("repository resource");
    let stale_resource = client
        .read_resource(ReadResourceRequestParams::new(
            repository_resource.uri.clone(),
        ))
        .await
        .expect("stale resource response");
    let stale_resource_json = serde_json::to_value(stale_resource).expect("resource JSON");
    let stale_resource_text = stale_resource_json["contents"][0]["text"]
        .as_str()
        .expect("resource text");
    let stale_resource_payload: Value =
        serde_json::from_str(stale_resource_text).expect("resource payload");
    assert_eq!(
        stale_resource_payload["freshness"]["history"]["stale"],
        true
    );
    connection
        .execute(
            "UPDATE history_graph_repositories SET indexed_head = ?2 WHERE repo_path = ?1",
            params![repo_path, head],
        )
        .expect("restore history head");

    connection
        .execute(
            "DELETE FROM structural_graph_snapshots WHERE repo_path = ?1",
            [&repo_path],
        )
        .expect("remove graph fixture");
    let missing_graph = client
        .call_tool(CallToolRequestParams::new("graph_query"))
        .await
        .expect("missing graph response");
    assert_eq!(missing_graph.is_error, Some(true));
    assert_eq!(
        missing_graph
            .structured_content
            .expect("missing graph error")["error"]["code"],
        "unavailable"
    );

    connection
        .execute(
            "UPDATE mcp_repository_scopes SET enabled = 0 WHERE repo_id = ?1",
            [repo_id],
        )
        .expect("disable");
    let disabled = client
        .call_tool(CallToolRequestParams::new("history_list_releases"))
        .await
        .expect("disabled response");
    assert_eq!(disabled.is_error, Some(true));
    assert_eq!(
        disabled.structured_content.expect("error")["error"]["code"],
        "permission_denied"
    );

    connection
        .execute(
            "UPDATE mcp_repository_scopes SET enabled = 1 WHERE repo_id = ?1",
            [repo_id],
        )
        .expect("re-enable");
    drop(connection);
    let closed_desktop = client
        .call_tool(CallToolRequestParams::new("history_list_releases"))
        .await
        .expect("closed desktop response");
    assert_eq!(closed_desktop.is_error, Some(false));

    client.cancel().await.expect("cancel");
    server_task.await.expect("server task");
}

#[test]
fn request_validation_rejects_unknown_and_out_of_bounds_arguments() {
    let mut arguments = json!({"query": "safe", "unexpected": "ignored"})
        .as_object()
        .expect("arguments")
        .clone();
    assert!(validate_tool_arguments("graph_query", &arguments)
        .unwrap_err()
        .contains("Unknown 'unexpected'"));

    arguments = json!({"limit": MAX_PAGE_SIZE + 1})
        .as_object()
        .expect("arguments")
        .clone();
    assert!(validate_tool_arguments("graph_query", &arguments).is_err());

    arguments = json!({"filter": {"node_kinds": [], "unknown": true}})
        .as_object()
        .expect("arguments")
        .clone();
    assert!(validate_tool_arguments("graph_query", &arguments).is_err());

    arguments = json!({
        "selector": {"kind": "event", "event_id": "event-1", "extra": "rejected"}
    })
    .as_object()
    .expect("arguments")
    .clone();
    assert!(validate_tool_arguments("history_trace", &arguments).is_err());

    arguments = json!({
        "selector": {"kind": "event", "event_id": "event-1"},
        "limit": 10
    })
    .as_object()
    .expect("arguments")
    .clone();
    assert!(validate_tool_arguments("history_trace", &arguments).is_ok());
}

#[test]
fn query_failures_use_stable_typed_error_codes() {
    let cases = [
        ("repository disabled", "permission_denied"),
        ("history index is stale", "stale_index"),
        ("graph is not built", "unavailable"),
        ("node not found", "not_found"),
        ("multiple candidates are ambiguous", "ambiguous"),
        ("No directed graph path connects nodes", "bounded_no_path"),
        ("request cancelled", "cancelled"),
        ("query exceeded timeout", "timeout"),
        ("query must be bounded", "invalid_input"),
        ("query worker failed", "internal"),
    ];
    for (message, code) in cases {
        assert_eq!(classify_error(message), code, "{message}");
    }
}

fn git(repo: &std::path::Path, arguments: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .status()
        .expect("git");
    assert!(status.success(), "git {}", arguments.join(" "));
}

fn git_output(repo: &std::path::Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {}", arguments.join(" "));
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}
