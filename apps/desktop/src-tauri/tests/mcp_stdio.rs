use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn stdio_boundary_is_json_only_scoped_and_paginated() {
    let fixture = McpFixture::new();
    let connection = &fixture.connection;
    let repo_path = fixture.repo_path.as_str();
    let head = fixture.head.as_str();
    let repo_id = fixture.repo_id;
    let mut sidecar = fixture.spawn_initialized();

    let tools = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}
    }));
    let tool_definitions = tools["result"]["tools"].as_array().expect("tools");
    assert_eq!(tool_definitions.len(), 22);
    assert!(tool_definitions.iter().any(|tool| {
        tool["name"] == "history_list_landmarks"
            && tool["inputSchema"]["additionalProperties"] == false
    }));
    assert!(tool_definitions.iter().any(|tool| {
        tool["name"] == "history_list_contributors"
            && tool["inputSchema"]["additionalProperties"] == false
    }));

    let resources = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 3, "method": "resources/list", "params": {}
    }));
    let repository_uri = resources["result"]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .find_map(|resource| {
            let uri = resource["uri"].as_str()?;
            uri.contains("/repository/").then(|| uri.to_string())
        })
        .expect("repository resource");
    assert!(!repository_uri.contains(repo_path));

    let resource = sidecar.request(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "resources/read",
        "params": {"uri": repository_uri}
    }));
    assert!(resource["result"]["contents"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("schemaVersion")));

    let first_page = sidecar.request(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {"name": "history_list_releases", "arguments": {"limit": 1}}
    }));
    let next_cursor = first_page["result"]["structuredContent"]["data"]["data"]["nextCursor"]
        .as_str()
        .expect("release cursor");
    let second_page = sidecar.request(json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "history_list_releases",
            "arguments": {"limit": 1, "cursor": next_cursor}
        }
    }));
    assert_ne!(second_page["result"]["isError"], Value::Bool(true));

    let archaeology_resource_uri = resources["result"]["resources"]
        .as_array()
        .expect("resources")
        .iter()
        .find_map(|resource| {
            let uri = resource["uri"].as_str()?;
            uri.contains("/archaeology-catalog/")
                .then(|| uri.to_string())
        })
        .expect("archaeology catalog resource");
    let archaeology_resource = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 7, "method": "resources/read",
        "params": {"uri": archaeology_resource_uri}
    }));
    assert!(archaeology_resource["result"]["contents"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("list_rules")));

    let first_rules = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 8, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {"limit": 1}}
    }));
    let first_structured = &first_rules["result"]["structuredContent"];
    assert_eq!(first_structured["repository"]["id"], repo_id);
    assert_eq!(
        first_structured["data"]["data"]["result"]["context"]["repository_id"],
        repo_id
    );
    assert!(!first_structured
        .to_string()
        .contains("archaeology-repository:internal"));
    let rule_cursor = first_structured["data"]["data"]["result"]["page"]["next_cursor"]
        .as_str()
        .expect("rule cursor");
    let second_rules = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {"limit": 1, "cursor": rule_cursor}}
    }));
    assert_ne!(second_rules["result"]["isError"], Value::Bool(true));

    let cursor_misuse = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {
            "limit": 1, "cursor": rule_cursor, "filter": {"query": "different"}
        }}
    }));
    assert_eq!(cursor_misuse["result"]["isError"], true);

    let search = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 11, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {
            "filter": {"query": "claim"}
        }}
    }));
    assert_eq!(
        search["result"]["structuredContent"]["data"]["data"]["result"]["items"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );

    let stable_rule = archaeology_digest('1');
    let detail = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 12, "method": "tools/call",
        "params": {"name": "archaeology_get_rule", "arguments": {"rule_id": stable_rule}}
    }));
    assert_eq!(
        detail["result"]["structuredContent"]["data"]["data"]["operation"],
        "get_rule"
    );
    assert!(!detail.to_string().contains(repo_path));
    assert!(!detail.to_string().contains("fixture@codevetter.local"));
    let detail_freshness =
        &detail["result"]["structuredContent"]["data"]["data"]["result"]["context"]["freshness"];
    assert_eq!(detail_freshness["human_review_decisions_present"], true);
    assert_eq!(detail_freshness["human_review_decisions_stale"], false);

    for (id, name, arguments) in [
        (13, "archaeology_list_domains", json!({"limit": 1})),
        (
            14,
            "archaeology_reverse_source",
            json!({"source": {"kind": "span", "span_id": "span:one"}}),
        ),
        (
            15,
            "archaeology_list_relations",
            json!({"rule_id": archaeology_digest('1'), "kinds": ["depends_on"]}),
        ),
        (
            16,
            "archaeology_compare_temporal",
            json!({
                "before": {"kind": "generation", "generation_id": "archaeology-generation:ready"},
                "after": {"kind": "revision", "revision_sha": head}
            }),
        ),
        (
            17,
            "archaeology_hydrate_evidence",
            json!({
                "rule_id": archaeology_digest('1'),
                "evidence": [
                    {"kind": "fact", "evidence_id": "fact:one"},
                    {"kind": "span", "evidence_id": "span:one"}
                ]
            }),
        ),
    ] {
        let response = sidecar.request(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }));
        assert_ne!(response["result"]["isError"], true, "{name}: {response}");
        if name == "archaeology_compare_temporal" {
            let value = &response["result"]["structuredContent"]["data"]["data"]["result"]["value"];
            assert_eq!(value["coverage"], "complete");
            assert_eq!(value["page"]["total_rows"], 0);
            assert_eq!(
                value["before"]["generation_id"],
                "archaeology-generation:ready"
            );
            assert_eq!(value["after"]["revision_sha"], head);
            assert!(!response.to_string().contains("content_hash"));
        }
    }

    let unknown_field = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 18, "method": "tools/call",
        "params": {"name": "archaeology_reverse_source", "arguments": {
            "source": {"kind": "span", "span_id": "span:one", "absolute_path": repo_path}
        }}
    }));
    assert!(unknown_field.get("error").is_some());

    let unknown_method = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 181, "method": "archaeology/unknown", "params": {}
    }));
    assert_eq!(unknown_method["error"]["code"], -32601);

    let malformed_call = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 182, "method": "tools/call",
        "params": {"name": 7, "arguments": []}
    }));
    assert!(matches!(
        malformed_call["error"]["code"].as_i64(),
        Some(-32601) | Some(-32602)
    ));

    let foreign_repo_id = "repo_fedcba9876543210";
    seed_foreign_scope(connection, fixture.root.path(), foreign_repo_id);
    let foreign_uri = codevetter_desktop::mcp::uri::HistoryResourceUri::new(
        foreign_repo_id,
        "archaeology-catalog",
        "overview",
    )
    .expect("foreign URI")
    .to_string();
    let cross_scope = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 183, "method": "resources/read",
        "params": {"uri": foreign_uri}
    }));
    assert!(cross_scope.get("error").is_some());
    assert!(!cross_scope.to_string().contains(foreign_repo_id));
    assert!(!cross_scope.to_string().contains(repo_path));

    let foreign_identity = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 19, "method": "tools/call",
        "params": {"name": "archaeology_get_rule", "arguments": {
            "rule_id": archaeology_digest('f')
        }}
    }));
    assert_eq!(foreign_identity["result"]["isError"], true);
    assert!(!foreign_identity.to_string().contains("internal"));

    connection
        .execute(
            "UPDATE archaeology_source_units SET classification='protected'
             WHERE generation_id='archaeology-generation:ready' AND source_unit_id='source-unit:one'",
            [],
        )
        .expect("protect source");
    let protected = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 20, "method": "tools/call",
        "params": {"name": "archaeology_hydrate_evidence", "arguments": {
            "rule_id": archaeology_digest('1'),
            "evidence": [{"kind": "span", "evidence_id": "span:one"}]
        }}
    }));
    assert_eq!(protected["result"]["isError"], true);

    seed_current_input_change(connection, head);
    let parser_stale = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 201, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {}}
    }));
    let parser_freshness = &parser_stale["result"]["structuredContent"]["data"]["data"]["result"]
        ["context"]["freshness"];
    assert_eq!(parser_freshness["stale"], true);
    assert!(parser_freshness["reasons"]
        .as_array()
        .is_some_and(|reasons| reasons
            .iter()
            .any(|reason| reason == "parser_identity_changed")));
    assert!(parser_freshness["reasons"]
        .as_array()
        .is_some_and(|reasons| reasons
            .iter()
            .any(|reason| reason == "config_identity_changed")));

    connection
        .execute(
            "UPDATE history_graph_repositories SET indexed_head='stale-history-head'
             WHERE repo_path=?1",
            [repo_path],
        )
        .expect("stale history");
    let history_stale = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 202, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {}}
    }));
    assert_eq!(
        history_stale["result"]["structuredContent"]["freshness"]["history"]["stale"],
        true
    );

    let repository = Path::new(repo_path);
    fs::write(repository.join("README.md"), "fixture changed\n").expect("change fixture");
    git(repository, &["add", "README.md"]);
    git(repository, &["commit", "-m", "change fixture"]);
    let changed_head = git_output(repository, &["rev-parse", "HEAD"]);
    connection
        .execute(
            "UPDATE archaeology_repositories SET current_revision=?2 WHERE repo_path=?1",
            params![repo_path, changed_head],
        )
        .expect("update archaeology revision");
    // Repository freshness is cached briefly so a burst of MCP reads does not
    // spawn Git for every tool call. Cross the cache boundary before asserting
    // that the sidecar observes the new on-disk HEAD.
    thread::sleep(Duration::from_millis(1_100));
    let stale = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 21, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {}}
    }));
    assert_eq!(
        stale["result"]["structuredContent"]["data"]["data"]["result"]["context"]["freshness"]
            ["stale"],
        true
    );
    assert_eq!(
        stale["result"]["structuredContent"]["data"]["data"]["result"]["context"]["freshness"]
            ["human_review_decisions_stale"],
        true
    );
    assert_eq!(
        stale["result"]["structuredContent"]["data"]["data"]["result"]["context"]["freshness"]
            ["human_review_stale_reasons"][0],
        "repository_revision_changed"
    );

    connection
        .execute(
            "UPDATE mcp_repository_scopes SET enabled=0 WHERE repo_id=?1",
            [repo_id],
        )
        .expect("revoke scope");
    let revoked = sidecar.request(json!({
        "jsonrpc": "2.0", "id": 22, "method": "tools/call",
        "params": {"name": "archaeology_list_rules", "arguments": {}}
    }));
    assert_eq!(revoked["result"]["isError"], true);
    assert_eq!(
        revoked["result"]["structuredContent"]["error"]["code"],
        "permission_denied"
    );
    sidecar.close();
}

#[test]
fn stdio_archaeology_catalog_is_bounded_at_100000_rules() {
    let fixture = McpFixture::new();
    seed_scale_catalog(&fixture.connection, 100_000);
    let mut sidecar = fixture.spawn_initialized();

    let first = sidecar.call_tool(30, "archaeology_list_rules", json!({"limit": 100}));
    let result = &first["result"]["structuredContent"]["data"]["data"]["result"];
    assert_eq!(result["page"]["total_rows"], 100_000);
    assert_eq!(result["page"]["returned_rows"], 100);
    assert_eq!(result["page"]["truncated"], true);
    assert!(first.to_string().len() < 256 * 1_024);
    let cursor = result["page"]["next_cursor"]
        .as_str()
        .expect("scale cursor");

    let second = sidecar.call_tool(
        31,
        "archaeology_list_rules",
        json!({"limit": 100, "cursor": cursor}),
    );
    let second_result = &second["result"]["structuredContent"]["data"]["data"]["result"];
    assert_eq!(second_result["page"]["total_rows"], 100_000);
    assert_eq!(second_result["page"]["returned_rows"], 100);
    assert_ne!(
        result["items"][0]["rule_id"],
        second_result["items"][0]["rule_id"]
    );

    let search = sidecar.call_tool(
        32,
        "archaeology_list_rules",
        json!({"filter": {"query": "needle100000"}}),
    );
    let matches = &search["result"]["structuredContent"]["data"]["data"]["result"];
    assert_eq!(matches["page"]["total_rows"], 1);
    assert_eq!(matches["items"][0]["rule_id"], scale_rule_identity(100_000));

    let detail = sidecar.call_tool(
        33,
        "archaeology_get_rule",
        json!({"rule_id": scale_rule_identity(100_000)}),
    );
    assert_eq!(
        detail["result"]["structuredContent"]["data"]["data"]["result"]["value"]["rule_id"],
        scale_rule_identity(100_000)
    );
    sidecar.close();
}

#[test]
fn stdio_pipelines_concurrent_requests_and_exits_cleanly_on_eof() {
    let fixture = McpFixture::new();
    let mut sidecar = fixture.spawn_initialized();
    let expected: BTreeSet<i64> = (1000..1012).collect();
    for id in &expected {
        sidecar.write(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "archaeology_list_rules", "arguments": {"limit": 1}}
        }));
    }
    let mut received = BTreeSet::new();
    for _ in 0..expected.len() {
        let response = sidecar.response();
        assert_ne!(response["result"]["isError"], true, "{response}");
        received.insert(response["id"].as_i64().expect("response id"));
    }
    assert_eq!(received, expected);
    sidecar.close();
}

struct McpFixture {
    root: tempfile::TempDir,
    repo_path: String,
    database: PathBuf,
    connection: Connection,
    head: String,
    repo_id: &'static str,
}

impl McpFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("fixture");
        let repo = root.path().join("repo");
        fs::create_dir(&repo).expect("repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "fixture@codevetter.local"]);
        git(&repo, &["config", "user.name", "CodeVetter Fixture"]);
        fs::write(repo.join("README.md"), "fixture\n").expect("file");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "fixture"]);

        let head = git_output(&repo, &["rev-parse", "HEAD"]);
        let repo_path = repo
            .canonicalize()
            .expect("canonical repo")
            .to_string_lossy()
            .to_string();
        let database = root.path().join("codevetter.db");
        let connection = Connection::open(&database).expect("database");
        codevetter_desktop::db::schema::run_migrations(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO history_graph_repositories (
                    repo_path, repository_fingerprint, indexed_head, status,
                    created_at, updated_at
                 ) VALUES (?1, 'fixture', ?2, 'ready', ?3, ?3)",
                params![repo_path, head, "2026-01-01T00:00:00Z"],
            )
            .expect("history repository");
        for (ordinal, sha, committed_at, tag) in [
            (0, "fixture-release-1", "2025-12-01T00:00:00Z", "v0.9.0"),
            (1, "fixture-release-2", "2026-01-01T00:00:00Z", "v1.0.0"),
        ] {
            connection
                .execute(
                    "INSERT INTO history_graph_revisions (
                        repo_path, sha, ordinal, committed_at, author_name, subject,
                        parents_json, tags_json, is_release, is_head, coverage_json
                     ) VALUES (?1, ?2, ?3, ?4, 'Fixture', ?5, '[]', ?6, 1, 0, '{}')",
                    params![
                        repo_path,
                        sha,
                        ordinal,
                        committed_at,
                        format!("Release {tag}"),
                        json!([tag]).to_string()
                    ],
                )
                .expect("release revision");
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
        seed_archaeology_catalog(&connection, &repo_path, &head);
        Self {
            root,
            repo_path,
            database,
            connection,
            head,
            repo_id,
        }
    }

    fn spawn_initialized(&self) -> McpProcess {
        let mut sidecar = McpProcess::spawn(&self.database, self.repo_id);
        let initialized = sidecar.request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "stdio-fixture", "version": "1"}
            }
        }));
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        sidecar.notify(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
        sidecar
    }
}

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<Result<String, String>>,
    closed: bool,
}

impl McpProcess {
    fn spawn(database: &std::path::Path, repo_id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codevetter-mcp"))
            .args([
                "--database",
                database.to_str().expect("database path"),
                "--repo-id",
                repo_id,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("all_proxy")
            .env_remove("no_proxy")
            .spawn()
            .expect("spawn sidecar");
        let stdout = child.stdout.take().expect("stdout");
        let stderr = child.stderr.take().expect("stderr");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let message = line.map_err(|error| error.to_string());
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut stderr = BufReader::new(stderr);
            let mut sink = Vec::new();
            let _ = stderr.read_to_end(&mut sink);
        });
        let stdin = child.stdin.take();
        Self {
            child,
            stdin,
            stdout: receiver,
            closed: false,
        }
    }

    fn request(&mut self, message: Value) -> Value {
        self.write(message);
        self.response()
    }

    fn call_tool(&mut self, id: i64, name: &str, arguments: Value) -> Value {
        self.request(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }))
    }

    fn response(&mut self) -> Value {
        let line = self
            .stdout
            .recv_timeout(RESPONSE_TIMEOUT)
            .expect("sidecar response timed out")
            .expect("read sidecar stdout");
        serde_json::from_str(&line).expect("sidecar stdout must contain JSON only")
    }

    fn notify(&mut self, message: Value) {
        self.write(message);
    }

    fn write(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("sidecar stdin");
        writeln!(stdin, "{message}").expect("write request");
        stdin.flush().expect("flush request");
    }

    fn close(mut self) {
        self.stdin.take();
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll sidecar") {
                assert!(status.success(), "sidecar exited with {status}");
                self.closed = true;
                return;
            }
            assert!(Instant::now() < deadline, "sidecar did not exit after EOF");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
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

fn archaeology_digest(value: char) -> String {
    format!("sha256:{}", value.to_string().repeat(64))
}

fn archaeology_coverage() -> String {
    json!({
        "state": "complete",
        "parser_coverage": "complete",
        "repository_coverage": "complete",
        "temporal_coverage": "unavailable",
        "discovered_source_units": 1,
        "indexed_source_units": 1,
        "discovered_bytes": 100,
        "indexed_bytes": 100,
        "reasons": []
    })
    .to_string()
}

fn scale_rule_identity(ordinal: usize) -> String {
    format!("sha256:{ordinal:064x}")
}

fn seed_scale_catalog(connection: &Connection, rule_count: usize) {
    assert!(rule_count >= 2);
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("scale begin");
    connection
        .execute(
            "WITH RECURSIVE sequence(ordinal) AS (
               SELECT 3 UNION ALL SELECT ordinal + 1 FROM sequence WHERE ordinal < ?1
             )
             INSERT INTO archaeology_rules
               (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                identity_schema_version,stable_rule_identity,evidence_identity,
                contradiction_identity,description_identity,continuity_identity,
                parser_compatibility_identity,identity_provenance_json)
             SELECT 'archaeology-generation:ready',printf('occurrence:scale:%06d',ordinal),
                    'archaeology-repository:internal',?2,'validation',
                    printf('Scale claim rule %06d',ordinal),'candidate','deterministic','high',
                    ?3,?4,?5,'2026-01-01T00:00:00Z',2,
                    'sha256:' || printf('%064x',ordinal),?6,?7,?8,?9,?10,'{}'
             FROM sequence",
            params![
                rule_count,
                "scale-revision",
                archaeology_digest('b'),
                archaeology_digest('c'),
                archaeology_coverage(),
                archaeology_digest('3'),
                archaeology_digest('4'),
                archaeology_digest('5'),
                archaeology_digest('6'),
                archaeology_digest('7')
            ],
        )
        .expect("scale rules");
    connection
        .execute(
            "WITH RECURSIVE sequence(ordinal) AS (
               SELECT 3 UNION ALL SELECT ordinal + 1 FROM sequence WHERE ordinal < ?1
             )
             INSERT INTO archaeology_rule_search_manifest
               (generation_id,rule_id,title,clause_text,domain_text)
             SELECT 'archaeology-generation:ready',printf('occurrence:scale:%06d',ordinal),
                    printf('Scale claim rule %06d',ordinal),
                    printf('Bounded catalog qualification needle%06d',ordinal),'Scale'
             FROM sequence",
            [rule_count],
        )
        .expect("scale search manifest");
    connection
        .execute(
            "INSERT INTO archaeology_rule_clauses
               (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
             VALUES ('archaeology-generation:ready',printf('occurrence:scale:%06d',?1),
                     'clause:scale-detail',0,'The bounded scale rule is inspectable.',
                     'deterministic','high','[]')",
            [rule_count],
        )
        .expect("scale detail clause");
    connection.execute_batch("COMMIT").expect("scale commit");
}

fn seed_foreign_scope(connection: &Connection, root: &Path, repo_id: &str) {
    let repo = root.join("foreign-repo");
    fs::create_dir(&repo).expect("foreign repo");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.email", "foreign@codevetter.local"]);
    git(&repo, &["config", "user.name", "Foreign Fixture"]);
    fs::write(repo.join("README.md"), "foreign\n").expect("foreign file");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "foreign fixture"]);
    let head = git_output(&repo, &["rev-parse", "HEAD"]);
    let path = repo
        .canonicalize()
        .expect("foreign canonical repo")
        .to_string_lossy()
        .to_string();
    connection
        .execute(
            "INSERT INTO history_graph_repositories
               (repo_path,repository_fingerprint,indexed_head,status,created_at,updated_at)
             VALUES (?1,'foreign-fixture',?2,'ready',?3,?3)",
            params![path, head, "2026-01-01T00:00:00Z"],
        )
        .expect("foreign history repository");
    connection
        .execute(
            "INSERT INTO mcp_repository_scopes
               (repo_path,repo_id,enabled,created_at,updated_at)
             VALUES (?1,?2,1,?3,?3)",
            params![path, repo_id, "2026-01-01T00:00:00Z"],
        )
        .expect("foreign scope");
}

fn seed_current_input_change(connection: &Connection, revision: &str) {
    connection
        .execute(
            "INSERT INTO archaeology_generations
               (generation_id,repository_id,schema_version,revision_sha,source_identity,
                parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
             VALUES ('archaeology-generation:staging','archaeology-repository:internal',2,
                     ?1,?2,?3,?4,?5,'staging',?6,'2026-01-01T00:00:03Z')",
            params![
                revision,
                archaeology_digest('a'),
                archaeology_digest('8'),
                archaeology_digest('c'),
                archaeology_digest('9'),
                archaeology_coverage()
            ],
        )
        .expect("staging generation");
    connection
        .execute(
            "INSERT INTO archaeology_jobs
               (job_id,repository_id,generation_id,owner_id,stage,state,updated_at)
             VALUES ('archaeology-job:stale-inputs','archaeology-repository:internal',
                     'archaeology-generation:staging','owner:fixture','parse','running',
                     '2026-01-01T00:00:04Z')",
            [],
        )
        .expect("active archaeology job");
}

fn seed_archaeology_catalog(connection: &Connection, repo_path: &str, revision: &str) {
    let repository = "archaeology-repository:internal";
    let generation = "archaeology-generation:ready";
    connection
        .execute(
            "INSERT INTO archaeology_repositories
             (repository_id,repo_path,source_identity,current_revision,ready_generation_id,
              created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![
                repository,
                repo_path,
                archaeology_digest('a'),
                revision,
                generation,
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("archaeology repository");
    connection
        .execute(
            "INSERT INTO archaeology_generations
             (generation_id,repository_id,schema_version,revision_sha,source_identity,
              parser_identity,algorithm_identity,config_identity,status,coverage_json,
              created_at,published_at)
             VALUES (?1,?2,2,?3,?4,?5,?6,?7,'ready',?8,?9,?9)",
            params![
                generation,
                repository,
                revision,
                archaeology_digest('a'),
                archaeology_digest('b'),
                archaeology_digest('c'),
                archaeology_digest('d'),
                archaeology_coverage(),
                "2026-01-01T00:00:00Z"
            ],
        )
        .expect("archaeology generation");
    connection
        .execute(
            "INSERT INTO archaeology_source_units
             (generation_id,source_unit_id,path_identity,relative_path,content_hash,
              hash_algorithm,language,dialect,parser_id,parser_version,classification,
              byte_count,line_count,coverage_json)
             VALUES (?1,'source-unit:one','source-path:one','src/claims.cbl',?2,
                     'sha256','cobol','fixed','parser:cobol','1','source',100,10,?3)",
            params![generation, "e".repeat(64), archaeology_coverage()],
        )
        .expect("archaeology source");
    connection
        .execute(
            "INSERT INTO archaeology_source_spans
             (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
              start_line,start_column,end_line,end_column)
             VALUES (?1,'span:one','source-unit:one',?2,10,30,2,1,3,4)",
            params![generation, revision],
        )
        .expect("archaeology span");
    connection
        .execute(
            "INSERT INTO archaeology_facts
             (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
             VALUES (?1,'fact:one','predicate','Claim amount is positive','parser:cobol',
                     'extracted','high','[]')",
            [generation],
        )
        .expect("archaeology fact");
    connection
        .execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES (?1,'fact','fact:one','span','span:one','supporting')",
            [generation],
        )
        .expect("archaeology fact span");

    for (occurrence, stable, title, clause) in [
        (
            "occurrence:one",
            archaeology_digest('1'),
            "Eligible claims are scheduled",
            "clause:one",
        ),
        (
            "occurrence:two",
            archaeology_digest('2'),
            "Positive claims require review",
            "clause:two",
        ),
    ] {
        connection
            .execute(
                "INSERT INTO archaeology_rules
                 (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
                  confidence,parser_identity,algorithm_identity,coverage_json,created_at,
                  identity_schema_version,stable_rule_identity,evidence_identity,
                  contradiction_identity,description_identity,continuity_identity,
                  parser_compatibility_identity,identity_provenance_json)
                 VALUES (?1,?2,?3,?4,'validation',?5,'candidate','deterministic','high',
                         ?6,?7,?8,?9,2,?10,?11,?12,?13,?14,?15,'{}')",
                params![
                    generation,
                    occurrence,
                    repository,
                    revision,
                    title,
                    archaeology_digest('b'),
                    archaeology_digest('c'),
                    archaeology_coverage(),
                    "2026-01-01T00:00:00Z",
                    stable,
                    archaeology_digest('3'),
                    archaeology_digest('4'),
                    archaeology_digest('5'),
                    archaeology_digest('6'),
                    archaeology_digest('7')
                ],
            )
            .expect("archaeology rule");
        connection
            .execute(
                "INSERT INTO archaeology_rule_search_manifest
                 (generation_id,rule_id,title,clause_text,domain_text)
                 VALUES (?1,?2,?3,'A claim is handled when its amount is positive.','Claims')",
                params![generation, occurrence, title],
            )
            .expect("archaeology search manifest");
        connection
            .execute(
                "INSERT INTO archaeology_rule_clauses
                 (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
                 VALUES (?1,?2,?3,0,'A claim is handled when its amount is positive.',
                         'deterministic','high','[]')",
                params![generation, occurrence, clause],
            )
            .expect("archaeology clause");
        connection
            .execute(
                "INSERT INTO archaeology_evidence_links
                 (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
                 VALUES (?1,'rule_clause',?2,'fact','fact:one','supporting'),
                        (?1,'rule_clause',?2,'span','span:one','supporting')",
                params![generation, clause],
            )
            .expect("archaeology clause evidence");
        connection
            .execute(
                "INSERT INTO archaeology_rule_domains
                 (generation_id,rule_id,domain_id,domain_label)
                 VALUES (?1,?2,'domain:claims','Claims')",
                params![generation, occurrence],
            )
            .expect("archaeology domain");
    }
    connection
        .execute(
            "INSERT INTO archaeology_rule_relations
             (generation_id,relation_id,from_rule_id,to_rule_id,kind,trust,summary)
             VALUES (?1,'relation:dependency','occurrence:one','occurrence:two',
                     'depends_on','deterministic','Uses the reviewed claim rule')",
            [generation],
        )
        .expect("archaeology relation");
    let candidate_event = archaeology_digest('e');
    connection
        .execute(
            "INSERT INTO archaeology_rule_review_events
             (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,
              evidence_identity,created_at,event_schema_version,event_stream_identity,
              logical_sequence,stable_rule_identity,contradiction_identity,
              description_identity,continuity_identity,parser_identity,actor_kind,
              reviewer_provenance_json,legacy_stale)
             VALUES (?1,?2,'occurrence:one',?3,'candidate','codevetter:local',?4,
                     '2026-01-01T00:00:01Z',2,?5,1,?6,?7,?8,?9,?10,
                     'deterministic_policy','{}',0)",
            params![
                candidate_event,
                repository,
                generation,
                archaeology_digest('3'),
                archaeology_digest('0'),
                archaeology_digest('1'),
                archaeology_digest('4'),
                archaeology_digest('5'),
                archaeology_digest('6'),
                archaeology_digest('b')
            ],
        )
        .expect("candidate review event");
    connection
        .execute(
            "INSERT INTO archaeology_rule_review_events
             (event_id,repository_id,rule_id,generation_id,decision,reviewer_id,
              evidence_identity,created_at,event_schema_version,event_stream_identity,
              logical_sequence,stable_rule_identity,contradiction_identity,
              description_identity,continuity_identity,parser_identity,prior_event_id,
              actor_kind,reviewer_provenance_json,legacy_stale)
             VALUES (?1,?2,'occurrence:one',?3,'accepted','reviewer:local',?4,
                     '2026-01-01T00:00:02Z',2,?5,2,?6,?7,?8,?9,?10,?11,
                     'human','{}',0)",
            params![
                archaeology_digest('f'),
                repository,
                generation,
                archaeology_digest('3'),
                archaeology_digest('0'),
                archaeology_digest('1'),
                archaeology_digest('4'),
                archaeology_digest('5'),
                archaeology_digest('6'),
                archaeology_digest('b'),
                candidate_event
            ],
        )
        .expect("accepted review event");
    connection
        .execute(
            "INSERT INTO archaeology_temporal_generations
             (temporal_generation_identity,repository_id,generation_id,revision_sha,
              source_schema_version,catalog_identity,rule_count,coverage_state,
              coverage_reasons_json,created_at)
             VALUES (?1,?2,?3,?4,2,?5,2,'complete','[]','2026-01-01T00:00:00Z')",
            params![
                archaeology_digest('8'),
                repository,
                generation,
                revision,
                archaeology_digest('9')
            ],
        )
        .expect("archaeology temporal generation");
}
