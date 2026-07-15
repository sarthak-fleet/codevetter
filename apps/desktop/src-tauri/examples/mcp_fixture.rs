//! Builds an isolated MCP benchmark fixture without writing to the protected repository.

use chrono::{Duration, SecondsFormat, TimeZone, Utc};
use codevetter_desktop::{
    commands::structural_graph::{
        storage::persist_snapshot,
        types::{
            GraphOrigin, GraphSourceAnchor, GraphTrust, LanguageCoverage, StructuralGraphCommunity,
            StructuralGraphCoverage, StructuralGraphEdge, StructuralGraphEngineInfo,
            StructuralGraphFileRecord, StructuralGraphNode, StructuralGraphSnapshot,
            STRUCTURAL_GRAPH_SCHEMA_VERSION,
        },
    },
    db::schema::run_migrations,
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const DEFAULT_EVENT_COUNT: usize = 10_000;
const RELEASE_COUNT: usize = 64;
const GRAPH_FILE_COUNT: usize = 64;
const GRAPH_NODE_COUNT: usize = 512;
const GRAPH_EDGE_COUNT: usize = 1_024;
const REPO_ID: &str = "repo_fixture0123456789abcdef";

fn main() {
    if let Err(error) = run() {
        eprintln!("mcp-fixture: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (protected_repo, database) = arguments()?;
    let protected_repo = protected_repo
        .canonicalize()
        .map_err(|error| format!("Resolve protected repository: {error}"))?;
    let database = exact_output_path(&database)?;
    if database.starts_with(&protected_repo) {
        return Err("Fixture database must be outside the protected repository".to_string());
    }
    if database.exists() {
        return Err("Fixture database already exists".to_string());
    }

    let fixture_repo = database
        .parent()
        .ok_or_else(|| "Fixture database requires a parent directory".to_string())?
        .join("repository");
    let revisions = create_fixture_repository(&fixture_repo)?;
    let repo_path = fixture_repo
        .canonicalize()
        .map_err(|error| format!("Resolve fixture repository: {error}"))?
        .to_string_lossy()
        .into_owned();
    let head = revisions
        .last()
        .map(|revision| revision.sha.clone())
        .ok_or_else(|| "Fixture repository has no commits".to_string())?;
    let event_count = configured_event_count();

    let connection = Connection::open(&database)
        .map_err(|error| format!("Open fixture database exactly: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA wal_autocheckpoint = 200;",
        )
        .map_err(|error| format!("Configure fixture database: {error}"))?;
    run_migrations(&connection).map_err(|error| format!("Migrate fixture database: {error}"))?;
    persist_history_fixture(&connection, &repo_path, &head, &revisions, event_count)?;

    let snapshot = graph_fixture(repo_path.clone(), head.clone());
    persist_snapshot(&connection, &snapshot).map_err(|error| error.to_string())?;

    let counts = fixture_counts(&connection, &snapshot.id)?;
    if counts.events != event_count
        || counts.releases < RELEASE_COUNT
        || counts.nodes != GRAPH_NODE_COUNT
        || counts.edges != GRAPH_EDGE_COUNT
    {
        return Err(format!(
            "Fixture counts are invalid: events={}, releases={}, nodes={}, edges={}",
            counts.events, counts.releases, counts.nodes, counts.edges
        ));
    }

    println!(
        "{}",
        json!({
            "database": database,
            "repository": repo_path,
            "repoId": REPO_ID,
            "head": head,
            "eventCount": counts.events,
            "revisionCount": counts.revisions,
            "releaseCount": counts.releases,
            "graphNodeCount": counts.nodes,
            "graphEdgeCount": counts.edges,
        })
    );
    Ok(())
}

fn arguments() -> Result<(PathBuf, PathBuf), String> {
    let mut arguments = std::env::args().skip(1);
    let protected_repo = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
    let database = arguments.next().map(PathBuf::from).ok_or_else(usage)?;
    if arguments.next().is_some() {
        return Err(usage());
    }
    Ok((protected_repo, database))
}

fn usage() -> String {
    "usage: mcp_fixture <protected-repository> <database>".to_string()
}

fn exact_output_path(database: &Path) -> Result<PathBuf, String> {
    let parent = database
        .parent()
        .ok_or_else(|| "Fixture database requires a parent directory".to_string())?
        .canonicalize()
        .map_err(|error| format!("Resolve fixture output directory: {error}"))?;
    let file_name = database
        .file_name()
        .ok_or_else(|| "Fixture database requires a file name".to_string())?;
    Ok(parent.join(file_name))
}

#[derive(Debug)]
struct FixtureRevision {
    sha: String,
    parent: Option<String>,
    committed_at: String,
    tag: Option<String>,
}

fn create_fixture_repository(repo: &Path) -> Result<Vec<FixtureRevision>, String> {
    fs::create_dir(repo).map_err(|error| format!("Create fixture repository: {error}"))?;
    git(repo, &["init", "--quiet"])?;
    git(repo, &["config", "user.email", "fixture@codevetter.local"])?;
    git(repo, &["config", "user.name", "CodeVetter Fixture"])?;

    let mut revisions = Vec::with_capacity(RELEASE_COUNT + 1);
    let mut parent = None;
    for index in 0..=RELEASE_COUNT {
        let committed_at = fixture_time(index);
        fs::write(
            repo.join("fixture.txt"),
            format!("deterministic fixture revision {index}\n"),
        )
        .map_err(|error| format!("Write fixture revision: {error}"))?;
        git(repo, &["add", "fixture.txt"])?;
        git_with_date(
            repo,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("Fixture revision {index:02}"),
            ],
            &committed_at,
        )?;
        let sha = git_output(repo, &["rev-parse", "HEAD"])?;
        let tag = (index < RELEASE_COUNT).then(|| format!("v1.0.{}", index + 1));
        if let Some(tag) = &tag {
            git(repo, &["tag", tag])?;
        }
        revisions.push(FixtureRevision {
            sha: sha.clone(),
            parent,
            committed_at,
            tag,
        });
        parent = Some(sha);
    }
    Ok(revisions)
}

fn fixture_time(index: usize) -> String {
    (Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
        .single()
        .expect("fixture epoch is valid")
        + Duration::hours(index as i64))
    .to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn configured_event_count() -> usize {
    std::env::var("CV_MCP_FIXTURE_EVENTS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_EVENT_COUNT)
        .clamp(1, 100_000)
}

fn persist_history_fixture(
    connection: &Connection,
    repo_path: &str,
    head: &str,
    revisions: &[FixtureRevision],
    event_count: usize,
) -> Result<(), String> {
    let created_at = revisions
        .last()
        .map(|revision| revision.committed_at.as_str())
        .ok_or_else(|| "Fixture repository has no commits".to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| format!("Start fixture transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO history_graph_repositories (
                repo_path, repository_fingerprint, indexed_head, status,
                coverage_json, created_at, updated_at
             ) VALUES (?1, 'isolated-mcp-fixture-v1', ?2, 'ready', ?3, ?4, ?4)",
            params![
                repo_path,
                head,
                json!({"coverage_complete": true, "fixture": true}).to_string(),
                created_at
            ],
        )
        .map_err(|error| format!("Insert fixture repository: {error}"))?;

    {
        let mut statement = transaction
            .prepare_cached(
                "INSERT INTO history_graph_revisions (
                    repo_path, sha, ordinal, committed_at, author_name, subject,
                    parents_json, tags_json, is_release, is_head, coverage_json
                 ) VALUES (?1, ?2, ?3, ?4, 'CodeVetter Fixture', ?5, ?6, ?7, ?8, ?9, '{}')",
            )
            .map_err(|error| format!("Prepare fixture revisions: {error}"))?;
        for (ordinal, revision) in revisions.iter().enumerate() {
            statement
                .execute(params![
                    repo_path,
                    revision.sha,
                    ordinal as i64,
                    revision.committed_at,
                    format!("Fixture revision {ordinal:02}"),
                    serde_json::to_string(
                        &revision.parent.iter().cloned().collect::<Vec<String>>()
                    )
                    .map_err(|error| error.to_string())?,
                    serde_json::to_string(&revision.tag.iter().cloned().collect::<Vec<String>>())
                        .map_err(|error| error.to_string())?,
                    i64::from(revision.tag.is_some()),
                    i64::from(ordinal + 1 == revisions.len()),
                ])
                .map_err(|error| format!("Insert fixture revision: {error}"))?;
        }
    }

    {
        let mut statement = transaction
            .prepare_cached(
                "INSERT INTO history_graph_events (
                    id, repo_path, revision_sha, event_kind, entity_id, related_entity_id,
                    relation_kind, trust, origin, source_id, source_cursor, payload_json,
                    evidence_json, recorded_at
                 ) VALUES (?1, ?2, ?3, 'verification', ?4, ?5, 'verified_by',
                           'extracted', 'fixture', ?6, ?7, ?8, '[]', ?9)",
            )
            .map_err(|error| format!("Prepare fixture events: {error}"))?;
        for index in 0..event_count {
            let id = if index == 0 {
                "fixture-evidence".to_string()
            } else {
                format!("fixture-evidence-{index:06}")
            };
            statement
                .execute(params![
                    id,
                    repo_path,
                    head,
                    format!("fixture-entity-{:04}", index % GRAPH_NODE_COUNT),
                    format!("fixture-entity-{:04}", (index + 1) % GRAPH_NODE_COUNT),
                    format!("fixture-source-{:03}", index % GRAPH_FILE_COUNT),
                    format!("event:{index:06}"),
                    json!({
                        "summary": "Fixture verification passed",
                        "sequence": index,
                    })
                    .to_string(),
                    fixture_time(index % (RELEASE_COUNT + 1)),
                ])
                .map_err(|error| format!("Insert fixture event: {error}"))?;
        }
    }

    transaction
        .execute(
            "INSERT INTO mcp_repository_scopes (
                repo_path, repo_id, enabled, created_at, updated_at
             ) VALUES (?1, ?2, 1, ?3, ?3)",
            params![repo_path, REPO_ID, created_at],
        )
        .map_err(|error| format!("Insert fixture scope: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("Commit fixture history: {error}"))
}

fn graph_fixture(repo_path: String, head: String) -> StructuralGraphSnapshot {
    let files = (0..GRAPH_FILE_COUNT)
        .map(|index| StructuralGraphFileRecord {
            path: format!("src/module_{index:03}.rs"),
            language: Some("rust".to_string()),
            content_hash: Some(format!("fixture-content-{index:03}")),
            disposition: "indexed".to_string(),
            byte_size: 4_096,
            node_count: GRAPH_NODE_COUNT / GRAPH_FILE_COUNT,
            edge_count: GRAPH_EDGE_COUNT / GRAPH_FILE_COUNT,
        })
        .collect::<Vec<_>>();
    let nodes = (0..GRAPH_NODE_COUNT)
        .map(|index| {
            let file = index % GRAPH_FILE_COUNT;
            let path = format!("src/module_{file:03}.rs");
            StructuralGraphNode {
                id: format!("fixture-node-{index:04}"),
                kind: if index % 8 == 0 { "type" } else { "function" }.to_string(),
                label: format!("FixtureHandler{index:04}"),
                qualified_name: Some(format!("fixture::module_{file:03}::handler_{index:04}")),
                path: Some(path.clone()),
                detail: Some("Deterministic benchmark graph node".to_string()),
                language: Some("rust".to_string()),
                community_id: Some(format!("fixture-community-{:02}", index / 64)),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![GraphSourceAnchor {
                    path,
                    start_line: Some((index % 200 + 1) as u32),
                    start_column: Some(1),
                    end_line: Some((index % 200 + 2) as u32),
                    end_column: Some(1),
                    excerpt: None,
                }],
            }
        })
        .collect::<Vec<_>>();
    let edges = (0..GRAPH_EDGE_COUNT)
        .map(|index| {
            let from = index % GRAPH_NODE_COUNT;
            let jump = if index < GRAPH_NODE_COUNT { 1 } else { 17 };
            let to = (from + jump) % GRAPH_NODE_COUNT;
            StructuralGraphEdge {
                id: format!("fixture-edge-{index:04}"),
                from: format!("fixture-node-{from:04}"),
                to: format!("fixture-node-{to:04}"),
                kind: if jump == 1 { "calls" } else { "imports" }.to_string(),
                evidence: "Deterministic syntax edge".to_string(),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Resolution,
                sources: vec![GraphSourceAnchor::path(format!(
                    "src/module_{:03}.rs",
                    from % GRAPH_FILE_COUNT
                ))],
                candidates: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    let communities = (0..8)
        .map(|index| StructuralGraphCommunity {
            id: format!("fixture-community-{index:02}"),
            label: format!("Fixture subsystem {}", index + 1),
            member_count: 64,
            hub_node_ids: vec![format!("fixture-node-{:04}", index * 64)],
            bridge_node_ids: vec![format!("fixture-node-{:04}", index * 64 + 63)],
            score: 1.0,
        })
        .collect();

    StructuralGraphSnapshot {
        schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
        id: "fixture-current".to_string(),
        repo_path,
        repo_head: Some(head),
        created_at: fixture_time(RELEASE_COUNT),
        engine: StructuralGraphEngineInfo {
            id: "fixture".to_string(),
            version: "1".to_string(),
            bundled: true,
            syntax_aware: true,
            supported_languages: vec!["rust".to_string()],
        },
        cursor: None,
        ignore_fingerprint: Some("fixture-ignore-v1".to_string()),
        coverage: StructuralGraphCoverage {
            discovered_files: GRAPH_FILE_COUNT,
            indexed_files: GRAPH_FILE_COUNT,
            languages: vec![LanguageCoverage {
                language: "rust".to_string(),
                supported: true,
                discovered_files: GRAPH_FILE_COUNT,
                indexed_files: GRAPH_FILE_COUNT,
                skipped_files: 0,
                error_files: 0,
            }],
            ..StructuralGraphCoverage::default()
        },
        diagnostics: Vec::new(),
        communities,
        files,
        nodes,
        edges,
        metrics: Vec::new(),
        clone_groups: Vec::new(),
        truncated: false,
    }
}

struct FixtureCounts {
    events: usize,
    revisions: usize,
    releases: usize,
    nodes: usize,
    edges: usize,
}

fn fixture_counts(connection: &Connection, snapshot_id: &str) -> Result<FixtureCounts, String> {
    let count = |sql: &str, value: &str| {
        connection
            .query_row(sql, params![value], |row| row.get::<_, i64>(0))
            .map(|count| count as usize)
            .map_err(|error| error.to_string())
    };
    Ok(FixtureCounts {
        events: count(
            "SELECT COUNT(*) FROM history_graph_events WHERE repo_path = ?1",
            &fixture_repo_path(connection)?,
        )?,
        revisions: count(
            "SELECT COUNT(*) FROM history_graph_revisions WHERE repo_path = ?1",
            &fixture_repo_path(connection)?,
        )?,
        releases: count(
            "SELECT COUNT(*) FROM history_graph_revisions WHERE repo_path = ?1 AND is_release = 1",
            &fixture_repo_path(connection)?,
        )?,
        nodes: count(
            "SELECT COUNT(*) FROM structural_graph_nodes WHERE snapshot_id = ?1",
            snapshot_id,
        )?,
        edges: count(
            "SELECT COUNT(*) FROM structural_graph_edges WHERE snapshot_id = ?1",
            snapshot_id,
        )?,
    })
}

fn fixture_repo_path(connection: &Connection) -> Result<String, String> {
    connection
        .query_row(
            "SELECT repo_path FROM mcp_repository_scopes WHERE repo_id = ?1",
            params![REPO_ID],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn git(repo: &Path, arguments: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_with_date(repo: &Path, arguments: &[&str], date: &str) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_output(repo: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
