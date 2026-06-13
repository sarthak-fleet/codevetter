//! Repo Unpacked — whole-repository system briefs.
//!
//! Two-pass pipeline:
//!   1. Deterministic scanner builds a repo inventory (entrypoints, manifests,
//!      stack, language counts, top dirs, README/docs).
//!   2. Synthesis prompt is sent to the configured CLI agent (claude/gemini).
//!      Returns five sections — system_map, feature_catalog, behavior_traces,
//!      risk_map, agent_handoff — every claim is required to cite at least
//!      one source file path that exists in the inventory.
//!
//! Result rows live in `repo_unpacked_reports`. Inventory is stored alongside
//! the synthesised brief so the UI can re-render without re-paying LLM cost.

use crate::db::queries;
use crate::DbState;
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tauri::State;

const ALWAYS_SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".next",
    ".turbo",
    ".vercel",
    ".cache",
    "dist",
    "build",
    "out",
    "coverage",
    ".pnpm-store",
    "vendor",
    ".venv",
    "venv",
    ".gradle",
    ".idea",
    ".vscode",
    ".DS_Store",
];

const BINARY_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "tiff", "mp4", "mov", "webm", "mp3",
    "wav", "ogg", "flac", "zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "pdf", "psd", "ai",
    "sketch", "fig", "exe", "dll", "so", "dylib", "bin", "wasm", "o", "a", "lib", "ttf", "otf",
    "woff", "woff2", "eot", "lock", "min.js", "min.css",
];

const MAX_FILES: usize = 4000;
const MAX_FILE_BYTES: u64 = 1_000_000; // 1 MB — skip generated/blob-ish files
const README_PREVIEW_BYTES: usize = 8 * 1024;

// ─── Public types (mirrored on the TS side) ─────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LanguageCount {
    pub language: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ManifestSummary {
    pub path: String,
    pub kind: String, // package.json | cargo.toml | pyproject.toml | go.mod | gemfile | composer.json | tauri.conf.json | other
    pub name: Option<String>,
    pub version: Option<String>,
    pub dependencies: Vec<String>,
    pub scripts: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EntrypointHint {
    pub path: String,
    pub kind: String, // bin | server | desktop | web | script | config | docs
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DocFile {
    pub path: String,
    pub bytes: u64,
    pub preview: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DirSummary {
    pub path: String,
    pub file_count: usize,
    pub bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QaReadinessSignal {
    pub id: String,
    pub label: String,
    pub status: String, // ready | partial | missing
    pub detail: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QaSuggestedFlow {
    pub id: String,
    pub route: String,
    pub goal: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QaReadiness {
    pub score: i64,
    pub status: String, // ready | partial | missing
    pub summary: String,
    pub signals: Vec<QaReadinessSignal>,
    pub suggested_flows: Vec<QaSuggestedFlow>,
}

impl Default for QaReadiness {
    fn default() -> Self {
        Self {
            score: 0,
            status: "missing".to_string(),
            summary: "No synthetic QA readiness signals were captured for this inventory."
                .to_string(),
            signals: Vec::new(),
            suggested_flows: Vec::new(),
        }
    }
}

fn default_qa_readiness() -> QaReadiness {
    QaReadiness::default()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoGraphNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoGraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub evidence: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoGraph {
    pub schema_version: i64,
    pub nodes: Vec<RepoGraphNode>,
    pub edges: Vec<RepoGraphEdge>,
    pub truncated: bool,
}

impl Default for RepoGraph {
    fn default() -> Self {
        Self {
            schema_version: 1,
            nodes: Vec::new(),
            edges: Vec::new(),
            truncated: false,
        }
    }
}

fn default_repo_graph() -> RepoGraph {
    RepoGraph::default()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoGraphImportResult {
    pub graph: RepoGraph,
    pub source_kind: String,
    pub node_count: usize,
    pub edge_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHistoryCommit {
    pub sha: String,
    pub date: Option<String>,
    pub subject: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHistoryDecision {
    pub marker: String,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHistoryTestHint {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoHistoryBrief {
    pub schema_version: i64,
    pub summary: String,
    pub recent_commits: Vec<RepoHistoryCommit>,
    pub decisions: Vec<RepoHistoryDecision>,
    pub test_hints: Vec<RepoHistoryTestHint>,
    pub sources: Vec<String>,
    pub truncated: bool,
}

impl Default for RepoHistoryBrief {
    fn default() -> Self {
        Self {
            schema_version: 1,
            summary: "No local history brief was captured for this inventory.".to_string(),
            recent_commits: Vec::new(),
            decisions: Vec::new(),
            test_hints: Vec::new(),
            sources: Vec::new(),
            truncated: false,
        }
    }
}

fn default_history_brief() -> RepoHistoryBrief {
    RepoHistoryBrief::default()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoInventory {
    pub repo_path: String,
    pub repo_name: String,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub bytes_scanned: u64,
    pub max_files_hit: bool,
    pub languages: Vec<LanguageCount>,
    pub manifests: Vec<ManifestSummary>,
    pub entrypoints: Vec<EntrypointHint>,
    pub top_level_dirs: Vec<DirSummary>,
    pub docs: Vec<DocFile>,
    pub config_files: Vec<String>,
    pub stack_tags: Vec<String>,
    #[serde(default = "default_qa_readiness")]
    pub qa_readiness: QaReadiness,
    #[serde(default = "default_repo_graph")]
    pub repo_graph: RepoGraph,
    #[serde(default = "default_history_brief")]
    pub history_brief: RepoHistoryBrief,
    pub all_files: Vec<String>,
    pub ignored_dirs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReportClaim {
    pub claim: String,
    pub sources: Vec<String>, // file paths (optionally with #Lstart-end)
    pub kind: Option<String>, // "evidence" | "inference"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReportSection {
    pub title: String,
    pub summary: String,
    pub claims: Vec<ReportClaim>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct UnpackReport {
    pub system_map: Option<ReportSection>,
    pub feature_catalog: Option<ReportSection>,
    pub data_flow: Option<ReportSection>,
    pub behavior_traces: Option<ReportSection>,
    pub testing_signals: Option<ReportSection>,
    pub risk_map: Option<ReportSection>,
    pub extension_points: Option<ReportSection>,
    pub agent_handoff: Option<ReportSection>,
    pub agent_prompt: Option<String>,
    pub overview: Option<String>,
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan_repo_inventory(repo_path: String) -> Result<Value, String> {
    let inv = build_inventory(&repo_path)?;
    Ok(serde_json::to_value(&inv).map_err(|e| e.to_string())?)
}

#[tauri::command]
pub async fn generate_unpack_report(
    db: State<'_, DbState>,
    repo_path: String,
    agent: Option<String>,
) -> Result<Value, String> {
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    let started = std::time::Instant::now();

    let inventory = build_inventory(&repo_path)?;
    let inventory_json = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;

    let report_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO repo_unpacked_reports
             (id, repo_path, repo_name, commit_sha, status, agent_used,
              inventory_json, files_scanned, files_skipped, bytes_scanned,
              started_at, created_at)
             VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                report_id,
                inventory.repo_path,
                inventory.repo_name,
                inventory.commit_sha,
                agent,
                inventory_json,
                inventory.files_scanned as i64,
                inventory.files_skipped as i64,
                inventory.bytes_scanned as i64,
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    let prompt = build_synthesis_prompt(&inventory);

    let cli_cmd = match agent.as_str() {
        "gemini" => "gemini",
        _ => "claude",
    };
    let cli_path = crate::commands::review::resolve_cli_path_pub(cli_cmd);

    let cli_output = StdCommand::new(&cli_path)
        .args(["-p", &prompt])
        .current_dir(&repo_path)
        .output();

    let cli_output = match cli_output {
        Ok(o) => o,
        Err(e) => {
            mark_failed(
                &db,
                &report_id,
                &format!("Failed to spawn {cli_cmd} ({cli_path}): {e}"),
                started.elapsed().as_millis() as i64,
            );
            return Err(format!("Failed to spawn {cli_cmd}: {e}"));
        }
    };

    if !cli_output.status.success() {
        let stderr = String::from_utf8_lossy(&cli_output.stderr).to_string();
        mark_failed(
            &db,
            &report_id,
            &format!("{cli_cmd} failed: {stderr}"),
            started.elapsed().as_millis() as i64,
        );
        return Err(format!("{cli_cmd} failed: {stderr}"));
    }

    let raw = String::from_utf8_lossy(&cli_output.stdout).to_string();
    let json_str = match crate::commands::review::extract_json_from_output_pub(&raw) {
        Some(s) => s,
        None => {
            mark_failed(
                &db,
                &report_id,
                "Could not find JSON in agent output",
                started.elapsed().as_millis() as i64,
            );
            return Err("Could not find JSON in agent output".to_string());
        }
    };

    let parsed: Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            mark_failed(
                &db,
                &report_id,
                &format!("Failed to parse JSON: {e}"),
                started.elapsed().as_millis() as i64,
            );
            return Err(format!("Failed to parse JSON: {e}"));
        }
    };

    let report = normalize_report(&parsed, &inventory);
    let report_json = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    let runtime_ms = started.elapsed().as_millis() as i64;
    let model = parsed
        .get("model")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| Some(format!("cli:{cli_cmd}")));

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE repo_unpacked_reports
             SET status = 'completed', report_json = ?1, runtime_ms = ?2,
                 model_used = ?3, completed_at = ?4
             WHERE id = ?5",
            rusqlite::params![
                report_json,
                runtime_ms,
                model,
                chrono::Utc::now().to_rfc3339(),
                report_id,
            ],
        )
        .map_err(|e| e.to_string())?;

        queries::log_activity(
            &conn,
            &queries::ActivityInput {
                agent_id: None,
                event_type: Some("repo_unpacked_completed".to_string()),
                summary: Some(format!(
                    "Repo Unpacked brief generated for {}: {} files",
                    inventory.repo_name, inventory.files_scanned
                )),
                metadata: Some(json!({"report_id": report_id}).to_string()),
            },
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(json!({
        "report_id": report_id,
        "status": "completed",
        "runtime_ms": runtime_ms,
        "report": report,
        "inventory": inventory,
    }))
}

#[tauri::command]
pub async fn list_repo_unpack_reports(
    db: State<'_, DbState>,
    repo_path: Option<String>,
    limit: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let limit = limit.unwrap_or(50);

    let rows: Vec<Value> = if let Some(path) = repo_path {
        let mut stmt = conn
            .prepare(
                "SELECT id, repo_path, repo_name, commit_sha, status, error_message,
                        agent_used, model_used, files_scanned, files_skipped, runtime_ms,
                        cost_usd, started_at, completed_at, created_at
                 FROM repo_unpacked_reports
                 WHERE repo_path = ?1
                 ORDER BY datetime(created_at) DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(rusqlite::params![path, limit], row_to_summary)
            .map_err(|e| e.to_string())?;
        iter.filter_map(Result::ok).collect()
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, repo_path, repo_name, commit_sha, status, error_message,
                        agent_used, model_used, files_scanned, files_skipped, runtime_ms,
                        cost_usd, started_at, completed_at, created_at
                 FROM repo_unpacked_reports
                 ORDER BY datetime(created_at) DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(rusqlite::params![limit], row_to_summary)
            .map_err(|e| e.to_string())?;
        iter.filter_map(Result::ok).collect()
    };

    Ok(json!({ "reports": rows }))
}

#[tauri::command]
pub async fn get_repo_unpack_report(db: State<'_, DbState>, id: String) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let row = conn
        .query_row(
            "SELECT id, repo_path, repo_name, commit_sha, status, error_message,
                    agent_used, model_used, inventory_json, report_json,
                    files_scanned, files_skipped, bytes_scanned, runtime_ms,
                    cost_usd, started_at, completed_at, created_at
             FROM repo_unpacked_reports
             WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok(json!({
                    "id": r.get::<_, String>(0)?,
                    "repo_path": r.get::<_, String>(1)?,
                    "repo_name": r.get::<_, String>(2)?,
                    "commit_sha": r.get::<_, Option<String>>(3)?,
                    "status": r.get::<_, String>(4)?,
                    "error_message": r.get::<_, Option<String>>(5)?,
                    "agent_used": r.get::<_, Option<String>>(6)?,
                    "model_used": r.get::<_, Option<String>>(7)?,
                    "inventory_json": r.get::<_, Option<String>>(8)?,
                    "report_json": r.get::<_, Option<String>>(9)?,
                    "files_scanned": r.get::<_, i64>(10)?,
                    "files_skipped": r.get::<_, i64>(11)?,
                    "bytes_scanned": r.get::<_, i64>(12)?,
                    "runtime_ms": r.get::<_, Option<i64>>(13)?,
                    "cost_usd": r.get::<_, Option<f64>>(14)?,
                    "started_at": r.get::<_, Option<String>>(15)?,
                    "completed_at": r.get::<_, Option<String>>(16)?,
                    "created_at": r.get::<_, String>(17)?,
                }))
            },
        )
        .map_err(|e| format!("Report not found: {e}"))?;

    Ok(row)
}

#[tauri::command]
pub async fn delete_repo_unpack_report(
    db: State<'_, DbState>,
    id: String,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let n = conn
        .execute(
            "DELETE FROM repo_unpacked_reports WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
    Ok(json!({ "deleted": n > 0 }))
}

#[tauri::command]
pub async fn export_repo_unpack_report(
    db: State<'_, DbState>,
    id: String,
    format: String,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let (repo_name, report_json, inventory_json, created_at, agent_used, model_used) = conn
        .query_row(
            "SELECT repo_name, report_json, inventory_json, created_at,
                    agent_used, model_used
             FROM repo_unpacked_reports WHERE id = ?1",
            rusqlite::params![id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|e| format!("Report not found: {e}"))?;

    let report: UnpackReport = report_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    let inventory: Option<RepoInventory> = inventory_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let body = render_markdown(
        &repo_name,
        &created_at,
        agent_used.as_deref(),
        model_used.as_deref(),
        &report,
        inventory.as_ref(),
    );

    let content = match format.as_str() {
        "html" => render_html(&repo_name, &body),
        "repo_graph_json" => {
            let Some(inventory) = inventory.as_ref() else {
                return Err("Report missing inventory graph.".to_string());
            };
            serde_json::to_string_pretty(&inventory.repo_graph).map_err(|e| e.to_string())?
        }
        "agent_context_markdown" => {
            let Some(inventory) = inventory.as_ref() else {
                return Err("Report missing inventory context.".to_string());
            };
            render_agent_context_sidecar(&repo_name, &created_at, inventory)
        }
        _ => body,
    };

    Ok(json!({ "content": content, "format": format }))
}

#[tauri::command]
pub async fn import_repo_graph_json(content: String) -> Result<RepoGraphImportResult, String> {
    let value: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Graph import must be valid JSON: {e}"))?;
    import_repo_graph_from_value(&value)
}

fn import_repo_graph_from_value(value: &Value) -> Result<RepoGraphImportResult, String> {
    let (candidate, source_kind) = if let Some(graph) = value.get("repo_graph") {
        (graph, "repo_graph")
    } else if let Some(graph) = value.get("graph") {
        (graph, "graph")
    } else if let Some(graph) = value.pointer("/data/graph") {
        (graph, "data.graph")
    } else {
        (value, "root")
    };

    let mut warnings = Vec::new();
    let graph = match serde_json::from_value::<RepoGraph>(candidate.clone()) {
        Ok(graph) => graph,
        Err(_) => import_loose_repo_graph(candidate, &mut warnings)?,
    };
    validate_imported_repo_graph(&graph)?;

    let node_count = graph.nodes.len();
    let edge_count = graph.edges.len();
    Ok(RepoGraphImportResult {
        graph,
        source_kind: source_kind.to_string(),
        node_count,
        edge_count,
        warnings,
    })
}

fn import_loose_repo_graph(value: &Value, warnings: &mut Vec<String>) -> Result<RepoGraph, String> {
    let nodes_value = value
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "Graph import needs a nodes array.".to_string())?;
    let edges_value = value
        .get("edges")
        .and_then(Value::as_array)
        .ok_or_else(|| "Graph import needs an edges array.".to_string())?;

    const MAX_IMPORTED_NODES: usize = 1_000;
    const MAX_IMPORTED_EDGES: usize = 1_500;
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for (idx, node) in nodes_value.iter().take(MAX_IMPORTED_NODES).enumerate() {
        let id = string_field(node, &["id", "key"])
            .or_else(|| string_field(node, &["label", "name"]).map(|s| graph_id("imported", &s)))
            .ok_or_else(|| format!("Node {idx} is missing id, label, or name."))?;
        let kind = string_field(node, &["kind", "type", "category"])
            .unwrap_or_else(|| "imported".to_string());
        let label = string_field(node, &["label", "name", "title"]).unwrap_or_else(|| id.clone());
        let path = string_field(node, &["path", "file_path", "source_path"]);
        let detail = string_field(node, &["detail", "description", "summary"]);
        let mut sources = string_array_field(node, "sources");
        if sources.is_empty() {
            if let Some(path) = path.as_ref() {
                sources.push(path.clone());
            }
        }
        nodes.push(RepoGraphNode {
            id,
            kind,
            label,
            path,
            detail,
            sources,
        });
    }

    for (idx, edge) in edges_value.iter().take(MAX_IMPORTED_EDGES).enumerate() {
        let from = string_field(edge, &["from", "source", "source_id", "start"])
            .ok_or_else(|| format!("Edge {idx} is missing from/source."))?;
        let to = string_field(edge, &["to", "target", "target_id", "end"])
            .ok_or_else(|| format!("Edge {idx} is missing to/target."))?;
        let kind = string_field(edge, &["kind", "type", "label"])
            .unwrap_or_else(|| "relates_to".to_string());
        let evidence = string_field(edge, &["evidence", "detail", "description"])
            .unwrap_or_else(|| "imported graph edge".to_string());
        edges.push(RepoGraphEdge {
            from,
            to,
            kind,
            evidence,
            sources: string_array_field(edge, "sources"),
        });
    }

    let truncated =
        nodes_value.len() > MAX_IMPORTED_NODES || edges_value.len() > MAX_IMPORTED_EDGES;
    if nodes_value.len() > MAX_IMPORTED_NODES {
        warnings.push(format!(
            "Imported first {MAX_IMPORTED_NODES} of {} nodes.",
            nodes_value.len()
        ));
    }
    if edges_value.len() > MAX_IMPORTED_EDGES {
        warnings.push(format!(
            "Imported first {MAX_IMPORTED_EDGES} of {} edges.",
            edges_value.len()
        ));
    }
    warnings
        .push("Loose graph JSON was normalized into CodeVetter's repo_graph schema.".to_string());

    Ok(RepoGraph {
        schema_version: 1,
        nodes,
        edges,
        truncated,
    })
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn string_array_field(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn validate_imported_repo_graph(graph: &RepoGraph) -> Result<(), String> {
    if graph.nodes.is_empty() {
        return Err("Graph import did not contain any nodes.".to_string());
    }
    let mut node_ids = HashSet::new();
    for node in &graph.nodes {
        if node.id.trim().is_empty() {
            return Err("Graph import contains a node with an empty id.".to_string());
        }
        if node.kind.trim().is_empty() {
            return Err(format!("Graph node {} has an empty kind.", node.id));
        }
        if !node_ids.insert(node.id.as_str()) {
            return Err(format!(
                "Graph import contains duplicate node id {}.",
                node.id
            ));
        }
    }
    for edge in &graph.edges {
        if edge.from.trim().is_empty() || edge.to.trim().is_empty() {
            return Err("Graph import contains an edge with an empty endpoint.".to_string());
        }
        if !node_ids.contains(edge.from.as_str()) || !node_ids.contains(edge.to.as_str()) {
            return Err(format!(
                "Graph edge {} -> {} references a missing node.",
                edge.from, edge.to
            ));
        }
    }
    Ok(())
}

// ─── Inventory builder (deterministic) ──────────────────────────────────────

pub fn build_inventory(repo_path: &str) -> Result<RepoInventory, String> {
    let root = PathBuf::from(repo_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {repo_path}"));
    }

    let repo_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo_path.to_string());

    let (commit_sha, branch, remote_url) = read_git_metadata(&root);

    let ignore_patterns = parse_gitignore(&root);

    let mut all_files: Vec<(String, u64)> = Vec::new();
    let mut files_skipped: usize = 0;
    let mut bytes_scanned: u64 = 0;
    let mut max_files_hit = false;
    let mut ignored_dirs: Vec<String> = Vec::new();

    walk(
        &root,
        &root,
        0,
        12,
        &ignore_patterns,
        &mut all_files,
        &mut files_skipped,
        &mut bytes_scanned,
        &mut max_files_hit,
        &mut ignored_dirs,
    );

    // Languages
    let mut lang_map: HashMap<&'static str, (usize, u64)> = HashMap::new();
    for (path, size) in &all_files {
        if let Some(lang) = language_for_path(path) {
            let entry = lang_map.entry(lang).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += size;
        }
    }
    let mut languages: Vec<LanguageCount> = lang_map
        .into_iter()
        .map(|(language, (files, bytes))| LanguageCount {
            language: language.to_string(),
            files,
            bytes,
        })
        .collect();
    languages.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    // Manifests
    let mut manifests: Vec<ManifestSummary> = Vec::new();
    for (path, _) in &all_files {
        if let Some(m) = parse_manifest(&root, path) {
            manifests.push(m);
        }
    }

    // Docs (README + docs/ + agents.md + AGENTS.md + CLAUDE.md + ARCHITECTURE.md)
    let mut docs: Vec<DocFile> = Vec::new();
    for (path, size) in &all_files {
        let lower = path.to_lowercase();
        let is_doc = lower == "readme.md"
            || lower == "readme"
            || lower == "agents.md"
            || lower == "claude.md"
            || lower == "architecture.md"
            || lower == "contributing.md"
            || lower == "license"
            || lower == "license.md"
            || lower.starts_with("docs/")
            || lower.starts_with("documentation/");
        if is_doc && lower.ends_with(".md") || lower == "readme" {
            let abs = root.join(path);
            let preview = read_first_bytes(&abs, README_PREVIEW_BYTES);
            docs.push(DocFile {
                path: path.clone(),
                bytes: *size,
                preview,
            });
        }
    }
    docs.sort_by(|a, b| a.path.cmp(&b.path));
    docs.truncate(40);

    // Top-level dirs
    let mut top_dir_map: HashMap<String, (usize, u64)> = HashMap::new();
    for (path, size) in &all_files {
        if let Some(top) = path.split('/').next() {
            if path.contains('/') {
                let entry = top_dir_map.entry(top.to_string()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += size;
            }
        }
    }
    let mut top_level_dirs: Vec<DirSummary> = top_dir_map
        .into_iter()
        .map(|(path, (file_count, bytes))| DirSummary {
            path,
            file_count,
            bytes,
        })
        .collect();
    top_level_dirs.sort_by(|a, b| b.file_count.cmp(&a.file_count));

    // Config files (interesting top-level)
    let config_files: Vec<String> = all_files
        .iter()
        .filter_map(|(p, _)| {
            let basename = Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let lower = basename.to_lowercase();
            let interesting = matches!(
                lower.as_str(),
                "tsconfig.json"
                    | "vite.config.ts"
                    | "vite.config.js"
                    | "next.config.js"
                    | "next.config.mjs"
                    | "tailwind.config.js"
                    | "tailwind.config.ts"
                    | "playwright.config.ts"
                    | "vitest.config.ts"
                    | "jest.config.js"
                    | "eslint.config.js"
                    | ".eslintrc.json"
                    | ".prettierrc"
                    | "dockerfile"
                    | "docker-compose.yml"
                    | "docker-compose.yaml"
                    | ".env.example"
                    | "wrangler.toml"
                    | "wrangler.jsonc"
                    | "fly.toml"
                    | "vercel.json"
                    | "netlify.toml"
                    | "tauri.conf.json"
                    | "renovate.json"
                    | "turbo.json"
                    | "pnpm-workspace.yaml"
                    | "lerna.json"
                    | ".github/workflows"
            );
            if interesting && !p.contains('/') {
                Some(p.clone())
            } else {
                None
            }
        })
        .collect();

    // Stack tags
    let stack_tags = infer_stack(&all_files, &manifests);

    // Entrypoints
    let entrypoints = infer_entrypoints(&all_files, &manifests, &stack_tags);
    let qa_readiness = build_qa_readiness(&all_files, &manifests, &entrypoints);
    let repo_graph = build_repo_graph(&root, &all_files, &manifests, &entrypoints);
    let history_brief = build_history_brief(&root, &all_files, &manifests);

    let path_strings: Vec<String> = all_files.iter().map(|(p, _)| p.clone()).collect();

    let inventory = RepoInventory {
        repo_path: repo_path.to_string(),
        repo_name,
        commit_sha,
        branch,
        remote_url,
        files_scanned: all_files.len(),
        files_skipped,
        bytes_scanned,
        max_files_hit,
        languages,
        manifests,
        entrypoints,
        top_level_dirs,
        docs,
        config_files,
        stack_tags,
        qa_readiness,
        repo_graph,
        history_brief,
        all_files: path_strings,
        ignored_dirs,
    };

    Ok(inventory)
}

fn build_qa_readiness(
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    entrypoints: &[EntrypointHint],
) -> QaReadiness {
    let file_paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();

    let browser_config_sources: Vec<String> = file_paths
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.ends_with("playwright.config.ts")
                || lower.ends_with("playwright.config.js")
                || lower.ends_with("playwright.config.mjs")
                || lower.ends_with("cypress.config.ts")
                || lower.ends_with("cypress.config.js")
                || lower.ends_with("cypress.config.mjs")
        })
        .take(8)
        .map(|path| (*path).to_string())
        .collect();

    let browser_spec_sources: Vec<String> = file_paths
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            let browserish_dir = lower.contains("/e2e/")
                || lower.contains("/playwright/")
                || lower.contains("/cypress/")
                || lower.starts_with("e2e/")
                || lower.starts_with("tests/e2e/")
                || lower.starts_with("cypress/");
            let browserish_name = lower.ends_with(".spec.ts")
                || lower.ends_with(".spec.tsx")
                || lower.ends_with(".spec.js")
                || lower.ends_with(".spec.jsx");
            browserish_dir && browserish_name
        })
        .take(12)
        .map(|path| (*path).to_string())
        .collect();

    let runnable_script_names = [
        "dev",
        "start",
        "preview",
        "serve",
        "tauri:dev",
        "desktop:dev",
    ];
    let qa_script_names = [
        "e2e",
        "test:e2e",
        "playwright",
        "test:playwright",
        "cypress",
        "test:cypress",
        "qa",
        "synthetic-qa",
        "test:synthetic-qa",
    ];

    let runnable_script_sources: Vec<String> = manifests
        .iter()
        .filter(|manifest| {
            manifest.kind == "package.json"
                && manifest
                    .scripts
                    .iter()
                    .any(|script| runnable_script_names.contains(&script.as_str()))
        })
        .map(|manifest| manifest.path.clone())
        .take(8)
        .collect();

    let qa_script_sources: Vec<String> = manifests
        .iter()
        .filter(|manifest| {
            manifest.kind == "package.json"
                && manifest.scripts.iter().any(|script| {
                    let lower = script.to_ascii_lowercase();
                    qa_script_names.contains(&lower.as_str())
                        || lower.contains("e2e")
                        || lower.contains("playwright")
                        || lower.contains("cypress")
                        || lower.contains("qa")
                })
        })
        .map(|manifest| manifest.path.clone())
        .take(8)
        .collect();

    let browser_dep_sources: Vec<String> = manifests
        .iter()
        .filter(|manifest| {
            manifest.dependencies.iter().any(|dep| {
                dep == "@playwright/test"
                    || dep == "playwright"
                    || dep == "cypress"
                    || dep == "puppeteer"
            })
        })
        .map(|manifest| manifest.path.clone())
        .take(8)
        .collect();

    let artifact_sources: Vec<String> = file_paths
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.contains("playwright-report/")
                || lower.contains("test-results/")
                || lower.contains("cypress/screenshots/")
                || lower.contains("cypress/videos/")
                || lower.ends_with("trace.zip")
                || lower.ends_with("report.html")
        })
        .take(8)
        .map(|path| (*path).to_string())
        .collect();

    let route_sources: Vec<String> = entrypoints
        .iter()
        .filter(|entry| {
            entry.kind == "web"
                || entry.reason.to_ascii_lowercase().contains("react")
                || entry.reason.to_ascii_lowercase().contains("router")
        })
        .map(|entry| entry.path.clone())
        .take(10)
        .collect();

    let docs_sources: Vec<String> = file_paths
        .iter()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.contains("qa")
                || lower.contains("playwright")
                || lower.contains("cypress")
                || lower.contains("e2e")
        })
        .filter(|path| path.ends_with(".md") || path.ends_with(".mdx"))
        .take(8)
        .map(|path| (*path).to_string())
        .collect();

    let mut score = 0;
    if !browser_config_sources.is_empty() {
        score += 20;
    } else if !browser_dep_sources.is_empty() {
        score += 12;
    }
    if !browser_spec_sources.is_empty() {
        score += 25;
    }
    if !qa_script_sources.is_empty() {
        score += 20;
    }
    if !runnable_script_sources.is_empty() {
        score += 15;
    }
    if !artifact_sources.is_empty() {
        score += 10;
    } else if !browser_config_sources.is_empty() || !browser_dep_sources.is_empty() {
        score += 5;
    }
    if !route_sources.is_empty() {
        score += 5;
    }
    if !docs_sources.is_empty() {
        score += 5;
    }
    score = score.min(100);

    let status = if score >= 75 {
        "ready"
    } else if score >= 45 {
        "partial"
    } else {
        "missing"
    }
    .to_string();

    let signal = |id: &str,
                  label: &str,
                  ready: bool,
                  partial: bool,
                  detail: String,
                  sources: Vec<String>|
     -> QaReadinessSignal {
        QaReadinessSignal {
            id: id.to_string(),
            label: label.to_string(),
            status: if ready {
                "ready"
            } else if partial {
                "partial"
            } else {
                "missing"
            }
            .to_string(),
            detail,
            sources,
        }
    };

    let mut runner_sources = browser_config_sources.clone();
    for source in &browser_dep_sources {
        push_unique_limited(&mut runner_sources, source.clone(), 8);
    }

    let signals = vec![
        signal(
            "browser_runner",
            "Browser runner",
            !browser_config_sources.is_empty(),
            !browser_dep_sources.is_empty(),
            if !browser_config_sources.is_empty() {
                format!(
                    "{} browser runner config file{} found.",
                    browser_config_sources.len(),
                    if browser_config_sources.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            } else if !browser_dep_sources.is_empty() {
                "Browser automation dependency is installed, but no runner config was found."
                    .to_string()
            } else {
                "No Playwright, Cypress, or browser runner config was found.".to_string()
            },
            runner_sources,
        ),
        signal(
            "user_flow_specs",
            "User-flow specs",
            !browser_spec_sources.is_empty(),
            false,
            if !browser_spec_sources.is_empty() {
                format!(
                    "{} browser-oriented spec file{} found.",
                    browser_spec_sources.len(),
                    if browser_spec_sources.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )
            } else {
                "No e2e/playwright/cypress spec files were found.".to_string()
            },
            browser_spec_sources.clone(),
        ),
        signal(
            "local_app_command",
            "Local app command",
            !runnable_script_sources.is_empty(),
            false,
            if !runnable_script_sources.is_empty() {
                "Package scripts expose a local dev/start/preview command.".to_string()
            } else {
                "No obvious package script for starting the app locally was found.".to_string()
            },
            runnable_script_sources.clone(),
        ),
        signal(
            "qa_script",
            "QA script",
            !qa_script_sources.is_empty(),
            false,
            if !qa_script_sources.is_empty() {
                "Package scripts expose a QA/e2e/browser test command.".to_string()
            } else {
                "No explicit QA/e2e/browser test script was found.".to_string()
            },
            qa_script_sources.clone(),
        ),
        signal(
            "artifact_trail",
            "Artifact trail",
            !artifact_sources.is_empty(),
            !browser_config_sources.is_empty() || !browser_dep_sources.is_empty(),
            if !artifact_sources.is_empty() {
                "Existing browser test artifacts or reports were found.".to_string()
            } else if !browser_config_sources.is_empty() || !browser_dep_sources.is_empty() {
                "Runner is artifact-capable, but no existing screenshot/trace/report artifacts were found in the scanned files.".to_string()
            } else {
                "No browser QA artifacts or artifact-capable runner were found.".to_string()
            },
            artifact_sources.clone(),
        ),
        signal(
            "targetable_routes",
            "Targetable surfaces",
            !route_sources.is_empty(),
            false,
            if !route_sources.is_empty() {
                "Web entrypoints or pages give Synthetic QA candidate surfaces.".to_string()
            } else {
                "No obvious web entrypoint or route file was found.".to_string()
            },
            route_sources.clone(),
        ),
    ];

    let suggested_flows = suggested_qa_flows(&file_paths);
    let summary = match status.as_str() {
        "ready" => "Repo has enough browser-runner, script, and flow evidence to seed Synthetic QA workflows from Repo Unpacked.",
        "partial" => "Repo has some Synthetic QA building blocks, but CodeVetter should ask for the missing runner/script/spec pieces before claiming runtime coverage.",
        _ => "Repo does not expose enough local browser QA structure for a reliable Synthetic QA workflow yet.",
    }
    .to_string();

    QaReadiness {
        score,
        status,
        summary,
        signals,
        suggested_flows,
    }
}

fn suggested_qa_flows(paths: &[&str]) -> Vec<QaSuggestedFlow> {
    let mut flows = Vec::new();
    let mut push_flow = |id: String, route: String, goal: String, source: String| {
        if flows.len() >= 8 {
            return;
        }
        if flows
            .iter()
            .any(|flow: &QaSuggestedFlow| flow.route == route)
        {
            return;
        }
        flows.push(QaSuggestedFlow {
            id,
            route,
            goal,
            sources: vec![source],
        });
    };

    for path in paths {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with("/app/page.tsx") || lower == "app/page.tsx" {
            push_flow(
                "app-root".to_string(),
                "/".to_string(),
                "Open the app home page and confirm the primary content renders.".to_string(),
                (*path).to_string(),
            );
            continue;
        }
        if lower.contains("/app/") && lower.ends_with("/page.tsx") {
            let route = path
                .split("/app/")
                .nth(1)
                .unwrap_or(path)
                .trim_end_matches("/page.tsx")
                .split('/')
                .filter(|part| !part.starts_with('(') && !part.starts_with('['))
                .collect::<Vec<_>>()
                .join("/");
            if !route.is_empty() {
                push_flow(
                    format!("next-{route}").replace('/', "-"),
                    format!("/{route}"),
                    format!("Open /{route} and verify the main user-visible flow."),
                    (*path).to_string(),
                );
            }
            continue;
        }
        if (lower.contains("/src/pages/") || lower.starts_with("src/pages/"))
            && (lower.ends_with(".tsx") || lower.ends_with(".jsx"))
        {
            let stem = Path::new(path)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            if stem.is_empty() {
                continue;
            }
            let route = if stem.eq_ignore_ascii_case("home") || stem.eq_ignore_ascii_case("index") {
                "/".to_string()
            } else {
                format!("/{}", camel_to_kebab(&stem))
            };
            push_flow(
                format!("page-{}", route.trim_start_matches('/')).replace('/', "-"),
                route.clone(),
                format!(
                    "Open {route} and verify the primary screen renders without console errors."
                ),
                (*path).to_string(),
            );
        }
    }

    flows
}

fn camel_to_kebab(value: &str) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if idx > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch == '_' || ch == ' ' {
            out.push('-');
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out.trim_matches('-').to_string()
}

fn push_unique_limited(values: &mut Vec<String>, value: impl Into<String>, limit: usize) {
    if values.len() >= limit {
        return;
    }
    let value = value.into();
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

const MAX_REPO_GRAPH_NODES: usize = 260;
const MAX_REPO_GRAPH_EDGES: usize = 520;

fn graph_id(kind: &str, value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{kind}:{slug}")
}

fn push_repo_graph_node(nodes: &mut Vec<RepoGraphNode>, node: RepoGraphNode) -> bool {
    if nodes.iter().any(|existing| existing.id == node.id) {
        return false;
    }
    if nodes.len() >= MAX_REPO_GRAPH_NODES {
        return false;
    }
    nodes.push(node);
    true
}

fn push_repo_graph_edge(edges: &mut Vec<RepoGraphEdge>, edge: RepoGraphEdge) -> bool {
    if edges.iter().any(|existing| {
        existing.from == edge.from && existing.to == edge.to && existing.kind == edge.kind
    }) {
        return false;
    }
    if edges.len() >= MAX_REPO_GRAPH_EDGES {
        return false;
    }
    edges.push(edge);
    true
}

fn file_graph_node(path: &str, kind: &str, detail: &str) -> RepoGraphNode {
    RepoGraphNode {
        id: graph_id("file", path),
        kind: kind.to_string(),
        label: path.to_string(),
        path: Some(path.to_string()),
        detail: Some(detail.to_string()),
        sources: vec![path.to_string()],
    }
}

fn build_repo_graph(
    root: &Path,
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    entrypoints: &[EntrypointHint],
) -> RepoGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut truncated = false;
    let file_paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();

    for entry in entrypoints.iter().take(80) {
        if !push_repo_graph_node(
            &mut nodes,
            file_graph_node(&entry.path, "file", &entry.reason),
        ) {
            truncated = true;
        }
    }

    for manifest in manifests.iter().take(40) {
        let package_label = manifest
            .name
            .clone()
            .unwrap_or_else(|| manifest.path.clone());
        let package_id = graph_id("package", &manifest.path);
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: package_id.clone(),
                kind: "package".to_string(),
                label: package_label,
                path: Some(manifest.path.clone()),
                detail: Some(format!("{} manifest", manifest.kind)),
                sources: vec![manifest.path.clone()],
            },
        ) {
            truncated = true;
        }

        for script in manifest.scripts.iter().take(18) {
            let script_id = graph_id("script", &format!("{}:{script}", manifest.path));
            if !push_repo_graph_node(
                &mut nodes,
                RepoGraphNode {
                    id: script_id.clone(),
                    kind: "script".to_string(),
                    label: script.clone(),
                    path: Some(manifest.path.clone()),
                    detail: Some("package script".to_string()),
                    sources: vec![manifest.path.clone()],
                },
            ) {
                truncated = true;
            }
            if !push_repo_graph_edge(
                &mut edges,
                RepoGraphEdge {
                    from: package_id.clone(),
                    to: script_id,
                    kind: "defines".to_string(),
                    evidence: format!("{} defines npm script `{script}`", manifest.path),
                    sources: vec![manifest.path.clone()],
                },
            ) {
                truncated = true;
            }
        }
    }

    for flow in suggested_qa_flows(&file_paths) {
        let Some(source) = flow.sources.first() else {
            continue;
        };
        let route_id = graph_id("route", &flow.route);
        let file_id = graph_id("file", source);
        let _ = push_repo_graph_node(&mut nodes, file_graph_node(source, "file", "route file"));
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: route_id.clone(),
                kind: "route".to_string(),
                label: flow.route.clone(),
                path: Some(source.clone()),
                detail: Some(flow.goal.clone()),
                sources: flow.sources.clone(),
            },
        ) {
            truncated = true;
        }
        if !push_repo_graph_edge(
            &mut edges,
            RepoGraphEdge {
                from: file_id,
                to: route_id,
                kind: "routes_to".to_string(),
                evidence: "route inferred from page file path".to_string(),
                sources: flow.sources,
            },
        ) {
            truncated = true;
        }
    }

    for path in file_paths.iter().filter(|path| is_test_path(path)).take(80) {
        let test_id = graph_id("test", path);
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: test_id.clone(),
                kind: "test".to_string(),
                label: (*path).to_string(),
                path: Some((*path).to_string()),
                detail: Some("test/spec file".to_string()),
                sources: vec![(*path).to_string()],
            },
        ) {
            truncated = true;
        }
    }

    for path in file_paths
        .iter()
        .filter(|path| should_scan_for_graph_markers(path))
        .take(300)
    {
        let abs = root.join(path);
        let content = read_first_bytes(&abs, 80 * 1024);
        if content.is_empty() {
            continue;
        }
        let file_id = graph_id("file", path);
        scan_tauri_commands(
            path,
            &content,
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
        scan_db_tables(
            path,
            &content,
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
        scan_decision_markers(
            path,
            &content,
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
    }

    nodes.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.label.cmp(&b.label)));
    edges.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });

    RepoGraph {
        schema_version: 1,
        nodes,
        edges,
        truncated,
    }
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with("_test.rs")
        || lower.contains("/tests/")
        || lower.starts_with("tests/")
}

fn should_scan_for_graph_markers(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".rs")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".sql")
        || lower.ends_with(".md")
        || lower.ends_with(".mdx")
}

fn scan_tauri_commands(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    let mut pending_command_attr = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[tauri::command") {
            pending_command_attr = true;
            continue;
        }
        if !pending_command_attr {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("pub async fn ")
            .or_else(|| trimmed.strip_prefix("pub fn "))
            .or_else(|| trimmed.strip_prefix("async fn "))
            .or_else(|| trimmed.strip_prefix("fn "))
        {
            let name = rest
                .split(|ch: char| ch == '(' || ch.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                pending_command_attr = false;
                continue;
            }
            let command_id = graph_id("tauri_command", name);
            if !push_repo_graph_node(
                nodes,
                RepoGraphNode {
                    id: command_id.clone(),
                    kind: "tauri_command".to_string(),
                    label: name.to_string(),
                    path: Some(path.to_string()),
                    detail: Some("Tauri command boundary".to_string()),
                    sources: vec![path.to_string()],
                },
            ) {
                *truncated = true;
            }
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: file_id.to_string(),
                    to: command_id,
                    kind: "defines".to_string(),
                    evidence: "function has #[tauri::command] attribute".to_string(),
                    sources: vec![path.to_string()],
                },
            ) {
                *truncated = true;
            }
            pending_command_attr = false;
        }
    }
}

fn scan_db_tables(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    for line in content.lines() {
        let upper = line.to_ascii_uppercase();
        let Some(idx) = upper.find("CREATE TABLE") else {
            continue;
        };
        let after = line[idx + "CREATE TABLE".len()..]
            .trim()
            .trim_start_matches("IF NOT EXISTS")
            .trim();
        let table = after
            .split(|ch: char| ch == '(' || ch.is_whitespace())
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .trim_matches('`')
            .trim();
        if table.is_empty() {
            continue;
        }
        let table_id = graph_id("db_table", table);
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: table_id.clone(),
                kind: "db_table".to_string(),
                label: table.to_string(),
                path: Some(path.to_string()),
                detail: Some("database table".to_string()),
                sources: vec![path.to_string()],
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: table_id,
                kind: "persists_to".to_string(),
                evidence: "CREATE TABLE statement".to_string(),
                sources: vec![path.to_string()],
            },
        ) {
            *truncated = true;
        }
    }
}

fn scan_decision_markers(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    for (idx, line) in content.lines().enumerate().take(600) {
        let marker = ["WHY:", "DECISION:", "TRADEOFF:"]
            .iter()
            .find(|marker| line.contains(**marker));
        let Some(marker) = marker else {
            continue;
        };
        let detail = line
            .split_once(marker)
            .map(|(_, rest)| rest.trim())
            .unwrap_or(line.trim())
            .chars()
            .take(160)
            .collect::<String>();
        let source = format!("{path}#L{}", idx + 1);
        let decision_id = graph_id("decision", &source);
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: decision_id.clone(),
                kind: "decision".to_string(),
                label: marker.trim_end_matches(':').to_ascii_lowercase(),
                path: Some(path.to_string()),
                detail: Some(detail),
                sources: vec![source.clone()],
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: decision_id,
                kind: "decided_by".to_string(),
                evidence: format!("{marker} marker"),
                sources: vec![source],
            },
        ) {
            *truncated = true;
        }
    }
}

fn build_history_brief(
    root: &Path,
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
) -> RepoHistoryBrief {
    let commits = read_recent_git_commits(root, 12);
    let mut decisions = collect_history_decisions(root, files, 16);
    let mut test_hints = collect_history_test_hints(files, manifests, 16);
    let mut sources = Vec::new();
    let mut truncated = false;

    if decisions.len() > 12 {
        decisions.truncate(12);
        truncated = true;
    }
    if test_hints.len() > 12 {
        test_hints.truncate(12);
        truncated = true;
    }

    for decision in &decisions {
        push_unique_limited(&mut sources, decision.source.clone(), 24);
    }
    for hint in &test_hints {
        push_unique_limited(&mut sources, hint.path.clone(), 24);
    }
    for manifest in manifests.iter().take(4) {
        push_unique_limited(&mut sources, manifest.path.clone(), 24);
    }

    let summary = if commits.is_empty() && decisions.is_empty() && test_hints.is_empty() {
        "No recent git commits, decision markers, or test hints were available from the bounded local scan.".to_string()
    } else {
        format!(
            "Local history brief captured {} recent commit{}, {} decision marker{}, and {} test hint{} for Repo Unpacked. Treat commit subjects as leads and rely on cited files for durable constraints.",
            commits.len(),
            if commits.len() == 1 { "" } else { "s" },
            decisions.len(),
            if decisions.len() == 1 { "" } else { "s" },
            test_hints.len(),
            if test_hints.len() == 1 { "" } else { "s" },
        )
    };

    RepoHistoryBrief {
        schema_version: 1,
        summary,
        recent_commits: commits,
        decisions,
        test_hints,
        sources,
        truncated,
    }
}

fn read_recent_git_commits(root: &Path, limit: usize) -> Vec<RepoHistoryCommit> {
    let output = StdCommand::new("git")
        .args([
            "log",
            &format!("-n{limit}"),
            "--date=short",
            "--format=%H%x1f%ad%x1f%s",
        ])
        .current_dir(root)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_git_commit_line)
        .collect()
}

fn parse_git_commit_line(line: &str) -> Option<RepoHistoryCommit> {
    let mut parts = line.splitn(3, '\x1f');
    let sha = parts.next()?.trim();
    let date = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let subject = parts.next()?.trim();
    if sha.is_empty() || subject.is_empty() {
        return None;
    }

    Some(RepoHistoryCommit {
        sha: sha.chars().take(12).collect(),
        date: date.map(String::from),
        subject: subject.chars().take(180).collect(),
    })
}

fn collect_history_decisions(
    root: &Path,
    files: &[(String, u64)],
    limit: usize,
) -> Vec<RepoHistoryDecision> {
    let mut decisions = Vec::new();
    for (path, _) in files
        .iter()
        .filter(|(path, _)| should_scan_for_history_markers(path))
        .take(320)
    {
        if decisions.len() >= limit {
            break;
        }
        let content = read_first_bytes(&root.join(path), 80 * 1024);
        if content.is_empty() {
            continue;
        }
        for (idx, line) in content.lines().enumerate().take(700) {
            if decisions.len() >= limit {
                break;
            }
            let marker = ["WHY:", "DECISION:", "TRADEOFF:"]
                .iter()
                .find(|marker| line.contains(**marker));
            let Some(marker) = marker else {
                continue;
            };
            let text = line
                .split_once(marker)
                .map(|(_, rest)| rest.trim())
                .unwrap_or(line.trim())
                .chars()
                .take(180)
                .collect::<String>();
            if text.is_empty() {
                continue;
            }
            decisions.push(RepoHistoryDecision {
                marker: marker.trim_end_matches(':').to_ascii_lowercase(),
                text,
                source: format!("{path}#L{}", idx + 1),
            });
        }
    }
    decisions
}

fn should_scan_for_history_markers(path: &str) -> bool {
    should_scan_for_graph_markers(path) && !looks_sensitive_path(path)
}

fn looks_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let basename = Path::new(&lower)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    basename == ".env"
        || basename.ends_with(".pem")
        || basename.ends_with(".key")
        || basename == "id_rsa"
        || basename == "id_ed25519"
        || lower.contains("/.ssh/")
        || lower.contains("/secrets/")
        || lower.contains("/credentials/")
}

fn collect_history_test_hints(
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    limit: usize,
) -> Vec<RepoHistoryTestHint> {
    let mut hints = Vec::new();
    for manifest in manifests
        .iter()
        .filter(|manifest| !manifest.scripts.is_empty())
    {
        for script in &manifest.scripts {
            let lower = script.to_ascii_lowercase();
            if lower.contains("test")
                || lower.contains("lint")
                || lower.contains("check")
                || lower.contains("e2e")
                || lower.contains("playwright")
            {
                hints.push(RepoHistoryTestHint {
                    path: manifest.path.clone(),
                    reason: format!("package script `{script}` is a likely verification command"),
                });
                if hints.len() >= limit {
                    return hints;
                }
            }
        }
    }

    for (path, _) in files.iter().filter(|(path, _)| is_test_path(path)) {
        if hints.iter().any(|hint| hint.path == *path) {
            continue;
        }
        hints.push(RepoHistoryTestHint {
            path: path.clone(),
            reason: "test/spec file anchors expected behavior".to_string(),
        });
        if hints.len() >= limit {
            break;
        }
    }

    hints
}

fn read_git_metadata(root: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let sha = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let branch = StdCommand::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    let remote = StdCommand::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());

    (sha, branch, remote)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    ignore_patterns: &[GlobPattern],
    out: &mut Vec<(String, u64)>,
    skipped: &mut usize,
    bytes_scanned: &mut u64,
    max_files_hit: &mut bool,
    ignored_dirs: &mut Vec<String>,
) {
    if depth > max_depth || *max_files_hit {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if *max_files_hit {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if ALWAYS_SKIP.contains(&name.as_str()) {
            if path.is_dir() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();
                if !ignored_dirs.contains(&rel) {
                    ignored_dirs.push(rel);
                }
            }
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if is_ignored(&rel, path.is_dir(), ignore_patterns) {
            if path.is_dir() && !ignored_dirs.contains(&rel) {
                ignored_dirs.push(rel.clone());
            } else {
                *skipped += 1;
            }
            continue;
        }

        if path.is_dir() {
            walk(
                root,
                &path,
                depth + 1,
                max_depth,
                ignore_patterns,
                out,
                skipped,
                bytes_scanned,
                max_files_hit,
                ignored_dirs,
            );
        } else if path.is_file() {
            // Skip binary/heavy files
            if is_binary_path(&rel) {
                *skipped += 1;
                continue;
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size > MAX_FILE_BYTES {
                *skipped += 1;
                continue;
            }
            *bytes_scanned += size;
            out.push((rel, size));
            if out.len() >= MAX_FILES {
                *max_files_hit = true;
                return;
            }
        }
    }
}

fn is_binary_path(rel: &str) -> bool {
    let lower = rel.to_lowercase();
    if lower.ends_with(".lock")
        || lower.ends_with("-lock.json")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("yarn.lock")
        || lower.ends_with("cargo.lock")
        || lower.ends_with("poetry.lock")
        || lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
    {
        return true;
    }
    let ext = Path::new(&lower)
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    BINARY_EXTS.contains(&ext.as_str())
}

fn language_for_path(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Some(match ext.as_str() {
        "ts" | "tsx" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "rs" => "Rust",
        "py" => "Python",
        "go" => "Go",
        "rb" => "Ruby",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cpp" | "cc" | "hpp" | "cxx" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "ex" | "exs" => "Elixir",
        "erl" => "Erlang",
        "scala" => "Scala",
        "lua" => "Lua",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "Sass",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" => "Shell",
        "md" | "mdx" => "Markdown",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        _ => return None,
    })
}

fn read_first_bytes(path: &Path, limit: usize) -> String {
    use std::io::Read;
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut buf = vec![0u8; limit];
    let n = file.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    String::from_utf8_lossy(&buf).to_string()
}

fn parse_manifest(root: &Path, rel: &str) -> Option<ManifestSummary> {
    let basename = Path::new(rel)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
        .to_lowercase();

    // Only top-level + apps/*/ + packages/*/ to avoid noise
    let depth = rel.matches('/').count();
    if depth > 3 {
        return None;
    }

    let abs = root.join(rel);
    match basename.as_str() {
        "package.json" => parse_package_json(&abs, rel),
        "cargo.toml" => parse_cargo_toml(&abs, rel),
        "pyproject.toml" => parse_pyproject(&abs, rel),
        "go.mod" => parse_go_mod(&abs, rel),
        "gemfile" => Some(ManifestSummary {
            path: rel.to_string(),
            kind: "gemfile".to_string(),
            name: None,
            version: None,
            dependencies: Vec::new(),
            scripts: Vec::new(),
        }),
        "composer.json" => Some(ManifestSummary {
            path: rel.to_string(),
            kind: "composer.json".to_string(),
            name: None,
            version: None,
            dependencies: Vec::new(),
            scripts: Vec::new(),
        }),
        "tauri.conf.json" => Some(ManifestSummary {
            path: rel.to_string(),
            kind: "tauri.conf.json".to_string(),
            name: None,
            version: None,
            dependencies: Vec::new(),
            scripts: Vec::new(),
        }),
        _ => None,
    }
}

fn parse_package_json(abs: &Path, rel: &str) -> Option<ManifestSummary> {
    let raw = fs::read_to_string(abs).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let name = v.get("name").and_then(|x| x.as_str()).map(String::from);
    let version = v.get("version").and_then(|x| x.as_str()).map(String::from);

    let mut deps: Vec<String> = Vec::new();
    for key in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = v.get(*key).and_then(|x| x.as_object()) {
            for k in map.keys() {
                deps.push(k.to_string());
            }
        }
    }
    deps.sort();
    deps.dedup();
    deps.truncate(80);

    let scripts: Vec<String> = v
        .get("scripts")
        .and_then(|x| x.as_object())
        .map(|m| m.keys().take(40).cloned().collect())
        .unwrap_or_default();

    Some(ManifestSummary {
        path: rel.to_string(),
        kind: "package.json".to_string(),
        name,
        version,
        dependencies: deps,
        scripts,
    })
}

fn parse_cargo_toml(abs: &Path, rel: &str) -> Option<ManifestSummary> {
    let raw = fs::read_to_string(abs).ok()?;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_deps = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]"
                || trimmed == "[dev-dependencies]"
                || trimmed == "[build-dependencies]"
                || trimmed.starts_with("[target.");
            if !in_deps {
                continue;
            }
            continue;
        }
        if !in_deps {
            if let Some(rest) = trimmed.strip_prefix("name") {
                if let Some(v) = parse_toml_string_value(rest) {
                    name = Some(v);
                }
            }
            if let Some(rest) = trimmed.strip_prefix("version") {
                if let Some(v) = parse_toml_string_value(rest) {
                    version = Some(v);
                }
            }
        } else {
            if let Some(eq_idx) = trimmed.find('=') {
                let dep = trimmed[..eq_idx].trim().trim_matches('"').to_string();
                if !dep.is_empty() && !dep.starts_with('#') {
                    deps.push(dep);
                }
            }
        }
    }
    deps.sort();
    deps.dedup();
    deps.truncate(80);
    Some(ManifestSummary {
        path: rel.to_string(),
        kind: "cargo.toml".to_string(),
        name,
        version,
        dependencies: deps,
        scripts: Vec::new(),
    })
}

fn parse_toml_string_value(rest: &str) -> Option<String> {
    let after_eq = rest.split_once('=')?.1.trim();
    let unquoted = after_eq.trim_matches('"').trim_matches('\'');
    if unquoted.is_empty() {
        None
    } else {
        Some(unquoted.to_string())
    }
}

fn parse_pyproject(abs: &Path, rel: &str) -> Option<ManifestSummary> {
    let raw = fs::read_to_string(abs).ok()?;
    let mut name = None;
    let mut version = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_deps = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed.contains("dependencies");
            continue;
        }
        if !in_deps {
            if let Some(rest) = trimmed.strip_prefix("name") {
                if let Some(v) = parse_toml_string_value(rest) {
                    name = Some(v);
                }
            }
            if let Some(rest) = trimmed.strip_prefix("version") {
                if let Some(v) = parse_toml_string_value(rest) {
                    version = Some(v);
                }
            }
        } else if let Some(dep) = trimmed.split_whitespace().next() {
            let cleaned = dep.trim_matches('"').trim_matches(',').to_string();
            if !cleaned.is_empty() {
                deps.push(cleaned);
            }
        }
    }
    deps.truncate(80);
    Some(ManifestSummary {
        path: rel.to_string(),
        kind: "pyproject.toml".to_string(),
        name,
        version,
        dependencies: deps,
        scripts: Vec::new(),
    })
}

fn parse_go_mod(abs: &Path, rel: &str) -> Option<ManifestSummary> {
    let raw = fs::read_to_string(abs).ok()?;
    let mut name = None;
    let mut deps: Vec<String> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            name = Some(rest.trim().to_string());
        }
        if trimmed.starts_with("require ") || trimmed.starts_with('\t') {
            if let Some(dep) = trimmed.split_whitespace().nth(0) {
                if dep != "require" && !dep.starts_with("//") {
                    deps.push(dep.to_string());
                }
            }
        }
    }
    deps.sort();
    deps.dedup();
    deps.truncate(80);
    Some(ManifestSummary {
        path: rel.to_string(),
        kind: "go.mod".to_string(),
        name,
        version: None,
        dependencies: deps,
        scripts: Vec::new(),
    })
}

fn infer_stack(files: &[(String, u64)], manifests: &[ManifestSummary]) -> Vec<String> {
    let mut tags: Vec<&'static str> = Vec::new();
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();

    let has = |needle: &str| names.iter().any(|p| p == &needle);
    let has_in = |needle: &str| names.iter().any(|p| p.contains(needle));

    if has("tauri.conf.json") || has_in("src-tauri/") {
        tags.push("Tauri");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"react".to_string()))
    {
        tags.push("React");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"vue".to_string()))
    {
        tags.push("Vue");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"svelte".to_string()))
    {
        tags.push("Svelte");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"next".to_string()))
    {
        tags.push("Next.js");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"vite".to_string()))
        || has("vite.config.ts")
        || has("vite.config.js")
    {
        tags.push("Vite");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.contains(&"tailwindcss".to_string()))
        || has("tailwind.config.ts")
        || has("tailwind.config.js")
    {
        tags.push("Tailwind");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.iter().any(|d| d == "drizzle-orm"))
    {
        tags.push("Drizzle");
    }
    if manifests.iter().any(|m| {
        m.dependencies
            .iter()
            .any(|d| d == "@cloudflare/workers-types")
    }) || has("wrangler.toml")
        || has("wrangler.jsonc")
    {
        tags.push("Cloudflare Workers");
    }
    if manifests.iter().any(|m| m.kind == "cargo.toml") {
        tags.push("Rust");
    }
    if manifests.iter().any(|m| m.kind == "go.mod") {
        tags.push("Go");
    }
    if manifests.iter().any(|m| m.kind == "pyproject.toml") {
        tags.push("Python");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.iter().any(|d| d == "@playwright/test"))
    {
        tags.push("Playwright");
    }
    if manifests
        .iter()
        .any(|m| m.dependencies.iter().any(|d| d == "vitest"))
    {
        tags.push("Vitest");
    }
    if has(".github/workflows") || has_in(".github/workflows/") {
        tags.push("GitHub Actions");
    }
    if has("Dockerfile") || has("docker-compose.yml") || has("docker-compose.yaml") {
        tags.push("Docker");
    }
    if has("vercel.json") {
        tags.push("Vercel");
    }
    if has("netlify.toml") {
        tags.push("Netlify");
    }
    if has("fly.toml") {
        tags.push("Fly.io");
    }

    tags.sort();
    tags.dedup();
    tags.into_iter().map(String::from).collect()
}

fn infer_entrypoints(
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    stack_tags: &[String],
) -> Vec<EntrypointHint> {
    let mut hits: Vec<EntrypointHint> = Vec::new();
    let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    let push_if = |hits: &mut Vec<EntrypointHint>, path: &str, kind: &str, reason: &str| {
        if names.contains(&path) {
            hits.push(EntrypointHint {
                path: path.to_string(),
                kind: kind.to_string(),
                reason: reason.to_string(),
            });
        }
    };

    push_if(&mut hits, "README.md", "docs", "Repository readme");
    push_if(&mut hits, "AGENTS.md", "docs", "Agent instructions");
    push_if(&mut hits, "agents.md", "docs", "Agent instructions");
    push_if(&mut hits, "CLAUDE.md", "docs", "Claude instructions");
    push_if(&mut hits, ".env.example", "config", "Required env vars");

    // Common code entrypoints (existence checked across full file list)
    let candidates = [
        ("src/main.rs", "bin", "Rust binary entrypoint"),
        ("src/lib.rs", "bin", "Rust library entrypoint"),
        ("src/index.ts", "web", "TS entrypoint"),
        ("src/index.tsx", "web", "TSX entrypoint"),
        ("src/main.ts", "web", "Vite/TS entrypoint"),
        ("src/main.tsx", "web", "Vite/React entrypoint"),
        ("src/App.tsx", "web", "React root component"),
        ("src/App.vue", "web", "Vue root component"),
        ("pages/_app.tsx", "web", "Next.js Pages Router"),
        ("app/page.tsx", "web", "Next.js App Router"),
        ("app/layout.tsx", "web", "Next.js root layout"),
        ("server.ts", "server", "Server entrypoint"),
        ("server.js", "server", "Server entrypoint"),
        ("worker.ts", "server", "Cloudflare worker"),
        ("workerd.ts", "server", "Cloudflare worker"),
        ("index.html", "web", "Static html shell"),
        ("manage.py", "script", "Django manage.py"),
        ("main.py", "script", "Python entrypoint"),
        ("app.py", "script", "Flask app"),
    ];
    for (path, kind, reason) in candidates {
        push_if(&mut hits, path, kind, reason);
    }

    // Walk every file looking for nested entrypoints (apps/*/src/main.tsx etc.)
    for (p, _) in files {
        if p.ends_with("src/main.rs") && p != "src/main.rs" {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "bin".to_string(),
                reason: "Rust binary entrypoint".to_string(),
            });
        }
        if p.ends_with("src-tauri/tauri.conf.json") {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "desktop".to_string(),
                reason: "Tauri config".to_string(),
            });
        }
        if p.ends_with("src/main.tsx") && p != "src/main.tsx" {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "web".to_string(),
                reason: "Vite React entrypoint".to_string(),
            });
        }
        if p.ends_with("src/App.tsx") && p != "src/App.tsx" {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "web".to_string(),
                reason: "React root".to_string(),
            });
        }
        if p.ends_with("vite.config.ts") || p.ends_with("vite.config.js") {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "config".to_string(),
                reason: "Vite config".to_string(),
            });
        }
        if p.ends_with("playwright.config.ts") {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "config".to_string(),
                reason: "Playwright e2e config".to_string(),
            });
        }
        if p.ends_with(".github/workflows/ci.yml")
            || p.ends_with(".github/workflows/release.yml")
            || (p.starts_with(".github/workflows/") && p.ends_with(".yml"))
        {
            hits.push(EntrypointHint {
                path: p.clone(),
                kind: "config".to_string(),
                reason: "GitHub Actions workflow".to_string(),
            });
        }
    }

    // Manifest-based: package.json scripts → "scripts" entrypoint
    for m in manifests {
        if m.kind == "package.json" && !m.scripts.is_empty() {
            let preview: Vec<String> = m.scripts.iter().take(8).cloned().collect();
            hits.push(EntrypointHint {
                path: m.path.clone(),
                kind: "config".to_string(),
                reason: format!("npm scripts: {}", preview.join(", ")),
            });
        }
    }

    // Stack hint nudges
    if stack_tags.contains(&"Tauri".to_string()) {
        for (p, _) in files {
            if p.ends_with("src-tauri/src/main.rs") {
                hits.push(EntrypointHint {
                    path: p.clone(),
                    kind: "desktop".to_string(),
                    reason: "Tauri Rust backend".to_string(),
                });
            }
        }
    }

    // De-dup by path
    let mut seen = std::collections::HashSet::new();
    hits.retain(|h| seen.insert(h.path.clone()));
    hits.truncate(60);
    hits
}

// ─── Synthesis prompt ───────────────────────────────────────────────────────

fn build_synthesis_prompt(inv: &RepoInventory) -> String {
    let mut buf = String::new();
    buf.push_str(
        "You are CodeVetter Repo Unpacked. You will produce a deep, evidence-backed system brief \
for the repo described below. The inventory I've assembled is only the skeleton — your job is to \
INVESTIGATE the repo using your file-read and search tools, then synthesise a rich brief grounded \
in what you actually read. Return ONLY valid JSON (no markdown fences, no commentary).\n\n",
    );

    buf.push_str("Investigation requirements (do these before writing claims):\n");
    buf.push_str("- Open and read at least 12 source files. Prioritise: every listed entrypoint, the top 3 manifests, the largest source files in the top dirs, all notable configs, and any docs that describe architecture.\n");
    buf.push_str("- Walk at least 3 user-visible flows end-to-end (e.g. \"startup\", \"primary action\", \"persistence path\") by reading the relevant files in sequence.\n");
    buf.push_str("- Inspect tests if present. Note framework, what is covered, what isn't.\n");
    buf.push_str("- Look for security-sensitive code paths (auth, secrets, IPC, shell-out, network, file IO outside repo root).\n");
    buf.push_str("- Look for extension points (registries, plugin systems, command tables, routers, factory functions).\n\n");

    buf.push_str("Required JSON shape:\n");
    buf.push_str(r#"{
  "overview": "2-4 sentence elevator pitch grounded in what you actually read — what the system does, who it's for, what's distinctive.",
  "system_map": {
    "summary": "3-6 sentences naming entrypoints, the request/event flow at the highest level, runtime boundaries, storage, and key external integrations.",
    "claims": [{"claim":"...","sources":["src/main.rs","apps/desktop/src/App.tsx"],"kind":"evidence"}]
  },
  "feature_catalog":   { "summary": "...", "claims": [...] },
  "data_flow":         { "summary": "...", "claims": [...] },
  "behavior_traces":   { "summary": "...", "claims": [...] },
  "testing_signals":   { "summary": "...", "claims": [...] },
  "risk_map":          { "summary": "...", "claims": [...] },
  "extension_points":  { "summary": "...", "claims": [...] },
  "agent_handoff":     { "summary": "...", "claims": [...] },
  "agent_prompt": "Reusable prompt block (300-700 words) future agents can paste in to onboard. Include stack, key files, conventions, danger zones, and a short 'how to make a safe change here' recipe."
}"#);
    buf.push_str("\n\nRules:\n");
    buf.push_str("- Every claim MUST list at least one `sources` file path that EXISTS in the file list below. Multi-file claims are encouraged — cite 2-4 sources where appropriate.\n");
    buf.push_str("- You may append `#Lstart-end` to a source path to point at a specific line range you read (e.g. `src/main.rs#L42-58`).\n");
    buf.push_str("- Use `kind: \"evidence\"` when sources directly support the claim. Use `kind: \"inference\"` only when reading between the lines; mark such claims clearly and use them sparingly (<20% of claims).\n");
    buf.push_str("- Do not invent files. If you cannot cite a file, omit the claim.\n");
    buf.push_str("- Target 8-15 claims per section. Each claim should be concrete and load-bearing — name functions, commands, files, env vars, types. Avoid vague restatements.\n");
    buf.push_str("- Each section summary should be 3-6 sentences. Do not pad — say something an experienced engineer wouldn't already know from skimming the repo for 30 seconds.\n\n");
    buf.push_str("Section briefs:\n");
    buf.push_str("- system_map: entrypoints, modules, runtime boundaries (process/thread/IPC), storage layer (schema names, table names if you read them), external integrations, build/test commands, deployment shape.\n");
    buf.push_str("- feature_catalog: every user-facing feature — routes, screens, CLI subcommands, Tauri/Rust commands, jobs, APIs, provider integrations. For each: where it's implemented (path), and any flag/toggle gating it.\n");
    buf.push_str("- data_flow: how data moves through the system end-to-end. Input boundaries → transforms → state owners → output boundaries. Where state lives (memory, SQLite tables, files, KV). Sync vs async hops.\n");
    buf.push_str("- behavior_traces: ordered walk-throughs of important flows (startup, primary action, persistence, settings load, update/release). Name the functions called in order.\n");
    buf.push_str("- testing_signals: test framework(s), which directories hold tests, what's covered vs uncovered, fixtures/mocks used, CI integration. If there are no tests, say so plainly and point at the highest-leverage missing test.\n");
    buf.push_str("- risk_map: security-sensitive paths, untested critical flows, fragile coupling, dead/legacy code, hidden flags, stale docs, blast-radius hotspots, places where a small change would silently break something else.\n");
    buf.push_str("- extension_points: where new code is meant to plug in — registries, command tables, plugin/provider interfaces, route lists, factory functions, config schemas. For each, name the file and the shape of the contract.\n");
    buf.push_str("- agent_handoff: conventions (naming, lint rules, formatting), safe edit boundaries (\"changing X almost always also requires Y\"), important files an agent must read before making changes, recommended tests to run, known traps.\n");
    buf.push_str("- agent_prompt: a copy-pasteable handoff prompt summarising the project for future agents. Should let a fresh agent be productive without re-reading the repo.\n");
    buf.push_str("\n");

    buf.push_str(&format!("Repo: {}\n", inv.repo_name));
    if let Some(sha) = &inv.commit_sha {
        buf.push_str(&format!("Commit: {}\n", sha));
    }
    if let Some(branch) = &inv.branch {
        buf.push_str(&format!("Branch: {}\n", branch));
    }
    if let Some(remote) = &inv.remote_url {
        buf.push_str(&format!("Remote: {}\n", remote));
    }
    buf.push_str(&format!(
        "Files scanned: {} (skipped {} binary/oversized/ignored)\n",
        inv.files_scanned, inv.files_skipped
    ));
    if inv.max_files_hit {
        buf.push_str(&format!(
            "(file walk stopped at MAX_FILES={MAX_FILES} — large repo)\n"
        ));
    }
    buf.push_str(&format!("Stack tags: {}\n", inv.stack_tags.join(", ")));

    buf.push_str("\nLanguages (top 10 by bytes):\n");
    for l in inv.languages.iter().take(10) {
        buf.push_str(&format!(
            "  - {} — {} files, {} bytes\n",
            l.language, l.files, l.bytes
        ));
    }

    buf.push_str("\nTop-level dirs:\n");
    for d in inv.top_level_dirs.iter().take(20) {
        buf.push_str(&format!(
            "  - {}/ — {} files, {} bytes\n",
            d.path, d.file_count, d.bytes
        ));
    }

    buf.push_str("\nManifests:\n");
    for m in &inv.manifests {
        buf.push_str(&format!(
            "  - {} ({}{}{})\n",
            m.path,
            m.name.as_deref().unwrap_or(""),
            m.version
                .as_deref()
                .map(|v| format!(" v{v}"))
                .unwrap_or_default(),
            if !m.scripts.is_empty() {
                format!(" scripts={}", m.scripts.join(","))
            } else {
                String::new()
            }
        ));
        if !m.dependencies.is_empty() {
            buf.push_str(&format!(
                "      deps: {}\n",
                m.dependencies
                    .iter()
                    .take(40)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    buf.push_str("\nLikely entrypoints:\n");
    for e in &inv.entrypoints {
        buf.push_str(&format!("  - {} [{}] — {}\n", e.path, e.kind, e.reason));
    }

    if !inv.config_files.is_empty() {
        buf.push_str("\nNotable configs:\n");
        for c in &inv.config_files {
            buf.push_str(&format!("  - {}\n", c));
        }
    }

    buf.push_str("\nSynthetic QA readiness:\n");
    buf.push_str(&format!(
        "  - status={} score={} — {}\n",
        inv.qa_readiness.status, inv.qa_readiness.score, inv.qa_readiness.summary
    ));
    for signal in &inv.qa_readiness.signals {
        let sources = if signal.sources.is_empty() {
            "no sources".to_string()
        } else {
            signal.sources.join(", ")
        };
        buf.push_str(&format!(
            "  - {} [{}]: {} ({})\n",
            signal.label, signal.status, signal.detail, sources
        ));
    }
    if !inv.qa_readiness.suggested_flows.is_empty() {
        buf.push_str("  Suggested local QA flows:\n");
        for flow in &inv.qa_readiness.suggested_flows {
            buf.push_str(&format!(
                "    - {} — {} (sources: {})\n",
                flow.route,
                flow.goal,
                flow.sources.join(", ")
            ));
        }
    }

    buf.push_str("\nRepo memory graph:\n");
    buf.push_str(&format!(
        "  - schema={} nodes={} edges={}{}\n",
        inv.repo_graph.schema_version,
        inv.repo_graph.nodes.len(),
        inv.repo_graph.edges.len(),
        if inv.repo_graph.truncated {
            " truncated"
        } else {
            ""
        }
    ));
    for node in inv.repo_graph.nodes.iter().take(20) {
        buf.push_str(&format!(
            "  - node {} [{}]{}{}\n",
            node.label,
            node.kind,
            node.path
                .as_deref()
                .map(|path| format!(" path={path}"))
                .unwrap_or_default(),
            node.detail
                .as_deref()
                .map(|detail| format!(" detail={detail}"))
                .unwrap_or_default()
        ));
    }
    for edge in inv.repo_graph.edges.iter().take(20) {
        buf.push_str(&format!(
            "  - edge {} -> {} [{}] ({})\n",
            edge.from, edge.to, edge.kind, edge.evidence
        ));
    }

    buf.push_str("\nCodebase history brief:\n");
    buf.push_str(&format!(
        "  - schema={}{} — {}\n",
        inv.history_brief.schema_version,
        if inv.history_brief.truncated {
            " truncated"
        } else {
            ""
        },
        inv.history_brief.summary
    ));
    if !inv.history_brief.recent_commits.is_empty() {
        buf.push_str("  Recent commits:\n");
        for commit in inv.history_brief.recent_commits.iter().take(8) {
            buf.push_str(&format!(
                "    - {}{} — {}\n",
                commit.sha,
                commit
                    .date
                    .as_deref()
                    .map(|date| format!(" {date}"))
                    .unwrap_or_default(),
                commit.subject
            ));
        }
    }
    if !inv.history_brief.decisions.is_empty() {
        buf.push_str("  Decision markers:\n");
        for decision in inv.history_brief.decisions.iter().take(8) {
            buf.push_str(&format!(
                "    - {} at {} — {}\n",
                decision.marker, decision.source, decision.text
            ));
        }
    }
    if !inv.history_brief.test_hints.is_empty() {
        buf.push_str("  Verification hints:\n");
        for hint in inv.history_brief.test_hints.iter().take(8) {
            buf.push_str(&format!("    - {} — {}\n", hint.path, hint.reason));
        }
    }

    if !inv.docs.is_empty() {
        buf.push_str("\nDocs (truncated previews):\n");
        for d in inv.docs.iter().take(8) {
            buf.push_str(&format!("---- {} ----\n", d.path));
            buf.push_str(d.preview.as_str());
            buf.push_str("\n");
        }
    }

    buf.push_str("\nFile list (truncated to fit):\n");
    let max_files_in_prompt = 1500usize;
    for p in inv.all_files.iter().take(max_files_in_prompt) {
        buf.push_str(p);
        buf.push('\n');
    }
    if inv.all_files.len() > max_files_in_prompt {
        buf.push_str(&format!(
            "... ({} more files omitted from prompt — they exist in the inventory)\n",
            inv.all_files.len() - max_files_in_prompt
        ));
    }

    buf
}

// ─── Report normalization (validate citations) ──────────────────────────────

fn normalize_report(parsed: &Value, inv: &RepoInventory) -> UnpackReport {
    let known_paths: std::collections::HashSet<&str> =
        inv.all_files.iter().map(|s| s.as_str()).collect();

    let take_section = |key: &str, title: &str| -> Option<ReportSection> {
        let v = parsed.get(key)?;
        let summary = v
            .get("summary")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let claims = v
            .get("claims")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| {
                        let claim = c.get("claim").and_then(|x| x.as_str())?.to_string();
                        let kind = c.get("kind").and_then(|x| x.as_str()).map(String::from);
                        let sources = c
                            .get("sources")
                            .and_then(|x| x.as_array())
                            .map(|src| {
                                src.iter()
                                    .filter_map(|s| s.as_str())
                                    .filter(|s| {
                                        let path_only = s.split('#').next().unwrap_or(s);
                                        known_paths.contains(path_only)
                                    })
                                    .map(String::from)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        if sources.is_empty() {
                            return None;
                        }
                        Some(ReportClaim {
                            claim,
                            sources,
                            kind,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if summary.is_empty() && claims.is_empty() {
            None
        } else {
            Some(ReportSection {
                title: title.to_string(),
                summary,
                claims,
            })
        }
    };

    UnpackReport {
        system_map: take_section("system_map", "System Map"),
        feature_catalog: take_section("feature_catalog", "Feature Catalog"),
        data_flow: take_section("data_flow", "Data Flow"),
        behavior_traces: take_section("behavior_traces", "Behavior Traces"),
        testing_signals: take_section("testing_signals", "Testing Signals"),
        risk_map: take_section("risk_map", "Risk Map"),
        extension_points: take_section("extension_points", "Extension Points"),
        agent_handoff: take_section("agent_handoff", "Agent Handoff Pack"),
        agent_prompt: parsed
            .get("agent_prompt")
            .and_then(|v| v.as_str())
            .map(String::from),
        overview: parsed
            .get("overview")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

// ─── Export helpers ─────────────────────────────────────────────────────────

fn render_markdown(
    repo_name: &str,
    created_at: &str,
    agent: Option<&str>,
    model: Option<&str>,
    report: &UnpackReport,
    inventory: Option<&RepoInventory>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Repo Unpacked — {}\n\n", repo_name));
    out.push_str(&format!("_Generated: {}", created_at));
    if let Some(a) = agent {
        out.push_str(&format!(" · agent: {}", a));
    }
    if let Some(m) = model {
        out.push_str(&format!(" · model: {}", m));
    }
    out.push_str("_\n\n");

    if let Some(o) = &report.overview {
        out.push_str(&format!("> {}\n\n", o));
    }

    if let Some(inv) = inventory {
        out.push_str(&format!(
            "**Stack:** {}\n\n",
            if inv.stack_tags.is_empty() {
                "—".to_string()
            } else {
                inv.stack_tags.join(", ")
            }
        ));
        out.push_str(&format!(
            "**Files scanned:** {} ({} skipped, {} bytes)\n\n",
            inv.files_scanned, inv.files_skipped, inv.bytes_scanned
        ));
        out.push_str(&format!(
            "**Synthetic QA readiness:** {} / 100 ({}) — {}\n\n",
            inv.qa_readiness.score, inv.qa_readiness.status, inv.qa_readiness.summary
        ));
        if !inv.qa_readiness.signals.is_empty() {
            out.push_str("### Synthetic QA Signals\n\n");
            for signal in &inv.qa_readiness.signals {
                out.push_str(&format!(
                    "- **{}:** {} — {}\n",
                    signal.label, signal.status, signal.detail
                ));
                if !signal.sources.is_empty() {
                    let srcs: Vec<String> =
                        signal.sources.iter().map(|s| format!("`{s}`")).collect();
                    out.push_str(&format!("  - sources: {}\n", srcs.join(", ")));
                }
            }
            out.push('\n');
        }
        if !inv.qa_readiness.suggested_flows.is_empty() {
            out.push_str("### Suggested Synthetic QA Flows\n\n");
            for flow in &inv.qa_readiness.suggested_flows {
                let srcs: Vec<String> = flow.sources.iter().map(|s| format!("`{s}`")).collect();
                out.push_str(&format!(
                    "- `{}` — {}{}{}\n",
                    flow.route,
                    flow.goal,
                    if srcs.is_empty() { "" } else { " (sources: " },
                    if srcs.is_empty() {
                        String::new()
                    } else {
                        format!("{})", srcs.join(", "))
                    }
                ));
            }
            out.push('\n');
        }
        if !inv.repo_graph.nodes.is_empty() {
            out.push_str(&format!(
                "### Repo Memory Graph\n\nSchema v{} · {} nodes · {} edges{}\n\n",
                inv.repo_graph.schema_version,
                inv.repo_graph.nodes.len(),
                inv.repo_graph.edges.len(),
                if inv.repo_graph.truncated {
                    " · truncated"
                } else {
                    ""
                }
            ));
            for node in inv.repo_graph.nodes.iter().take(20) {
                out.push_str(&format!("- **{}** `{}`", node.kind, node.label));
                if let Some(path) = &node.path {
                    out.push_str(&format!(" — `{path}`"));
                }
                if let Some(detail) = &node.detail {
                    out.push_str(&format!(" — {detail}"));
                }
                out.push('\n');
            }
            for edge in inv.repo_graph.edges.iter().take(20) {
                out.push_str(&format!(
                    "- `{}` -> `{}` ({}) — {}\n",
                    edge.from, edge.to, edge.kind, edge.evidence
                ));
            }
            out.push('\n');
        }
        if !inv.history_brief.recent_commits.is_empty()
            || !inv.history_brief.decisions.is_empty()
            || !inv.history_brief.test_hints.is_empty()
        {
            out.push_str(&format!(
                "### Codebase History Brief\n\nSchema v{}{} · {}\n\n",
                inv.history_brief.schema_version,
                if inv.history_brief.truncated {
                    " · truncated"
                } else {
                    ""
                },
                inv.history_brief.summary
            ));
            if !inv.history_brief.recent_commits.is_empty() {
                out.push_str("**Recent commits**\n\n");
                for commit in inv.history_brief.recent_commits.iter().take(8) {
                    out.push_str(&format!(
                        "- `{}`{} — {}\n",
                        commit.sha,
                        commit
                            .date
                            .as_deref()
                            .map(|date| format!(" {date}"))
                            .unwrap_or_default(),
                        commit.subject
                    ));
                }
                out.push('\n');
            }
            if !inv.history_brief.decisions.is_empty() {
                out.push_str("**Decision markers**\n\n");
                for decision in inv.history_brief.decisions.iter().take(10) {
                    out.push_str(&format!(
                        "- **{}** `{}` — {}\n",
                        decision.marker, decision.source, decision.text
                    ));
                }
                out.push('\n');
            }
            if !inv.history_brief.test_hints.is_empty() {
                out.push_str("**Verification hints**\n\n");
                for hint in inv.history_brief.test_hints.iter().take(10) {
                    out.push_str(&format!("- `{}` — {}\n", hint.path, hint.reason));
                }
                out.push('\n');
            }
        }
    }

    let render_section = |out: &mut String, sec: &Option<ReportSection>| {
        let Some(sec) = sec else { return };
        out.push_str(&format!("## {}\n\n", sec.title));
        if !sec.summary.is_empty() {
            out.push_str(&format!("{}\n\n", sec.summary));
        }
        for c in &sec.claims {
            let kind_marker = match c.kind.as_deref() {
                Some("inference") => " _(inference)_",
                _ => "",
            };
            out.push_str(&format!("- {}{}\n", c.claim, kind_marker));
            if !c.sources.is_empty() {
                let srcs: Vec<String> = c.sources.iter().map(|s| format!("`{}`", s)).collect();
                out.push_str(&format!("  - sources: {}\n", srcs.join(", ")));
            }
        }
        out.push('\n');
    };

    render_section(&mut out, &report.system_map);
    render_section(&mut out, &report.feature_catalog);
    render_section(&mut out, &report.data_flow);
    render_section(&mut out, &report.behavior_traces);
    render_section(&mut out, &report.testing_signals);
    render_section(&mut out, &report.risk_map);
    render_section(&mut out, &report.extension_points);
    render_section(&mut out, &report.agent_handoff);

    if let Some(prompt) = &report.agent_prompt {
        out.push_str("## Agent Handoff Prompt\n\n");
        out.push_str("```text\n");
        out.push_str(prompt);
        out.push_str("\n```\n");
    }

    out
}

fn render_agent_context_sidecar(
    repo_name: &str,
    created_at: &str,
    inventory: &RepoInventory,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Agent Context Sidecar — {repo_name}\n\n"));
    out.push_str(&format!(
        "_Generated: {created_at} · schema: repo_graph.v{} / history_brief.v{}_\n\n",
        inventory.repo_graph.schema_version, inventory.history_brief.schema_version
    ));
    out.push_str("## Use This For\n\n");
    out.push_str("- Paste into Hunk, Graphify, or an agent session as local context.\n");
    out.push_str("- Treat graph edges as navigation leads, not proof by themselves.\n");
    out.push_str("- Prefer cited files and decision markers when resolving conflicts.\n\n");

    out.push_str("## Repo\n\n");
    out.push_str(&format!("- path: `{}`\n", inventory.repo_path));
    if let Some(branch) = &inventory.branch {
        out.push_str(&format!("- branch: `{branch}`\n"));
    }
    if let Some(sha) = &inventory.commit_sha {
        out.push_str(&format!("- commit: `{}`\n", sha));
    }
    if !inventory.stack_tags.is_empty() {
        out.push_str(&format!("- stack: {}\n", inventory.stack_tags.join(", ")));
    }
    out.push('\n');

    if !inventory.history_brief.summary.is_empty() {
        out.push_str("## History Brief\n\n");
        out.push_str(&format!("{}\n\n", inventory.history_brief.summary));
        for decision in inventory.history_brief.decisions.iter().take(12) {
            out.push_str(&format!(
                "- **{}** `{}` — {}\n",
                decision.marker, decision.source, decision.text
            ));
        }
        for hint in inventory.history_brief.test_hints.iter().take(12) {
            out.push_str(&format!("- `{}` — {}\n", hint.path, hint.reason));
        }
        if !inventory.history_brief.recent_commits.is_empty() {
            out.push_str("\nRecent commits:\n");
            for commit in inventory.history_brief.recent_commits.iter().take(8) {
                out.push_str(&format!(
                    "- `{}`{} — {}\n",
                    commit.sha,
                    commit
                        .date
                        .as_deref()
                        .map(|date| format!(" {date}"))
                        .unwrap_or_default(),
                    commit.subject
                ));
            }
        }
        out.push('\n');
    }

    if !inventory.repo_graph.nodes.is_empty() {
        out.push_str("## Repo Graph Nodes\n\n");
        for node in inventory.repo_graph.nodes.iter().take(80) {
            out.push_str(&format!("- **{}** `{}`", node.kind, node.label));
            if let Some(path) = &node.path {
                out.push_str(&format!(" — `{path}`"));
            }
            if let Some(detail) = &node.detail {
                out.push_str(&format!(" — {detail}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if !inventory.repo_graph.edges.is_empty() {
        out.push_str("## Repo Graph Edges\n\n");
        for edge in inventory.repo_graph.edges.iter().take(120) {
            out.push_str(&format!(
                "- `{}` -> `{}` ({}) — {}",
                edge.from, edge.to, edge.kind, edge.evidence
            ));
            if !edge.sources.is_empty() {
                let sources: Vec<String> = edge.sources.iter().map(|s| format!("`{s}`")).collect();
                out.push_str(&format!("; sources: {}", sources.join(", ")));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if inventory.repo_graph.truncated || inventory.history_brief.truncated {
        out.push_str(
            "> This sidecar was truncated by CodeVetter's bounded local inventory scan.\n",
        );
    }

    out
}

fn render_html(repo_name: &str, markdown_body: &str) -> String {
    // Minimal static HTML — no external assets so the export is self-contained.
    let escaped = markdown_body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Repo Unpacked — {repo_name}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 920px; margin: 2.5rem auto; padding: 0 1.5rem; color: #1f2128; background: #fafafa; }}
  pre {{ background: #f1f3f5; padding: 1rem; overflow-x: auto; font-size: 0.85rem; border-radius: 4px; white-space: pre-wrap; }}
  code {{ background: #eef0f3; padding: 0.05rem 0.35rem; border-radius: 3px; font-size: 0.85rem; }}
  h1, h2, h3 {{ font-weight: 600; }}
</style>
</head>
<body>
<pre>{escaped}</pre>
</body>
</html>"#
    )
}

// ─── DB helpers ─────────────────────────────────────────────────────────────

fn mark_failed(db: &State<'_, DbState>, id: &str, msg: &str, runtime_ms: i64) {
    if let Ok(conn) = db.0.lock() {
        let _ = conn.execute(
            "UPDATE repo_unpacked_reports
             SET status='failed', error_message=?1, runtime_ms=?2,
                 completed_at=?3
             WHERE id=?4",
            rusqlite::params![msg, runtime_ms, chrono::Utc::now().to_rfc3339(), id],
        );
    }
}

fn row_to_summary(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": r.get::<_, String>(0)?,
        "repo_path": r.get::<_, String>(1)?,
        "repo_name": r.get::<_, String>(2)?,
        "commit_sha": r.get::<_, Option<String>>(3)?,
        "status": r.get::<_, String>(4)?,
        "error_message": r.get::<_, Option<String>>(5)?,
        "agent_used": r.get::<_, Option<String>>(6)?,
        "model_used": r.get::<_, Option<String>>(7)?,
        "files_scanned": r.get::<_, i64>(8)?,
        "files_skipped": r.get::<_, i64>(9)?,
        "runtime_ms": r.get::<_, Option<i64>>(10)?,
        "cost_usd": r.get::<_, Option<f64>>(11)?,
        "started_at": r.get::<_, Option<String>>(12)?,
        "completed_at": r.get::<_, Option<String>>(13)?,
        "created_at": r.get::<_, String>(14)?,
    }))
}

// ─── Gitignore patterns (mirrors files.rs but kept local) ───────────────────

struct GlobPattern {
    pattern: String,
    negated: bool,
    dir_only: bool,
}

fn parse_gitignore(root: &Path) -> Vec<GlobPattern> {
    let path = root.join(".gitignore");
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut pattern = line.to_string();
            let negated = pattern.starts_with('!');
            if negated {
                pattern = pattern[1..].to_string();
            }
            let dir_only = pattern.ends_with('/');
            if dir_only {
                pattern = pattern.trim_end_matches('/').to_string();
            }
            Some(GlobPattern {
                pattern,
                negated,
                dir_only,
            })
        })
        .collect()
}

fn is_ignored(rel: &str, is_dir: bool, patterns: &[GlobPattern]) -> bool {
    let mut ignored = false;
    let name = Path::new(rel)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    for pat in patterns {
        if pat.dir_only && !is_dir {
            continue;
        }
        if simple_glob_match(&pat.pattern, rel, &name) {
            ignored = !pat.negated;
        }
    }
    ignored
}

fn simple_glob_match(pattern: &str, rel: &str, name: &str) -> bool {
    if pattern.contains('/') {
        let pattern = pattern.trim_start_matches('/');
        return path_match(pattern, rel);
    }
    path_match(pattern, name)
}

fn path_match(pattern: &str, text: &str) -> bool {
    if pattern == "**" {
        return true;
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return text.ends_with(&format!(".{ext}"));
    }
    if pattern.starts_with('*') && !pattern.contains('/') {
        return text.ends_with(&pattern[1..]);
    }
    if pattern == text {
        return true;
    }
    if text.starts_with(pattern) && text[pattern.len()..].starts_with('/') {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package_manifest(path: &str, scripts: &[&str], deps: &[&str]) -> ManifestSummary {
        ManifestSummary {
            path: path.to_string(),
            kind: "package.json".to_string(),
            name: Some("demo".to_string()),
            version: None,
            dependencies: deps.iter().map(|dep| (*dep).to_string()).collect(),
            scripts: scripts.iter().map(|script| (*script).to_string()).collect(),
        }
    }

    fn minimal_inventory() -> RepoInventory {
        RepoInventory {
            repo_path: "/tmp/demo".to_string(),
            repo_name: "demo".to_string(),
            commit_sha: Some("1234567890abcdef".to_string()),
            branch: Some("main".to_string()),
            remote_url: None,
            files_scanned: 2,
            files_skipped: 0,
            bytes_scanned: 200,
            max_files_hit: false,
            languages: Vec::new(),
            manifests: Vec::new(),
            entrypoints: Vec::new(),
            top_level_dirs: Vec::new(),
            docs: Vec::new(),
            config_files: Vec::new(),
            stack_tags: vec!["React".to_string(), "Rust".to_string()],
            qa_readiness: QaReadiness::default(),
            repo_graph: RepoGraph {
                schema_version: 1,
                nodes: vec![RepoGraphNode {
                    id: "file:src-review-ts".to_string(),
                    kind: "file".to_string(),
                    label: "src/review.ts".to_string(),
                    path: Some("src/review.ts".to_string()),
                    detail: Some("review surface".to_string()),
                    sources: vec!["src/review.ts".to_string()],
                }],
                edges: vec![RepoGraphEdge {
                    from: "file:src-review-ts".to_string(),
                    to: "decision:src-review-ts-l1".to_string(),
                    kind: "decided_by".to_string(),
                    evidence: "DECISION marker".to_string(),
                    sources: vec!["src/review.ts#L1".to_string()],
                }],
                truncated: false,
            },
            history_brief: RepoHistoryBrief {
                schema_version: 1,
                summary: "History summary".to_string(),
                recent_commits: vec![RepoHistoryCommit {
                    sha: "1234567890ab".to_string(),
                    date: Some("2026-06-12".to_string()),
                    subject: "Add history brief".to_string(),
                }],
                decisions: vec![RepoHistoryDecision {
                    marker: "decision".to_string(),
                    text: "review keeps proof local".to_string(),
                    source: "src/review.ts#L1".to_string(),
                }],
                test_hints: vec![RepoHistoryTestHint {
                    path: "package.json".to_string(),
                    reason: "package script `test` is a likely verification command".to_string(),
                }],
                sources: vec!["src/review.ts#L1".to_string()],
                truncated: false,
            },
            all_files: vec!["src/review.ts".to_string(), "package.json".to_string()],
            ignored_dirs: Vec::new(),
        }
    }

    #[test]
    fn qa_readiness_scores_playwright_repo_with_flows() {
        let files = vec![
            ("package.json".to_string(), 200),
            ("playwright.config.ts".to_string(), 300),
            ("src/pages/Home.tsx".to_string(), 200),
            ("src/pages/Checkout.tsx".to_string(), 200),
            ("tests/e2e/checkout.spec.ts".to_string(), 500),
            ("docs/qa.md".to_string(), 100),
        ];
        let manifests = vec![package_manifest(
            "package.json",
            &["dev", "test:synthetic-qa", "test:e2e"],
            &["@playwright/test", "react"],
        )];
        let entrypoints = infer_entrypoints(&files, &manifests, &["React".to_string()]);

        let readiness = build_qa_readiness(&files, &manifests, &entrypoints);

        assert_eq!(readiness.status, "ready");
        assert!(readiness.score >= 90);
        assert!(readiness
            .signals
            .iter()
            .any(|signal| signal.id == "browser_runner" && signal.status == "ready"));
        assert!(readiness
            .suggested_flows
            .iter()
            .any(|flow| flow.route == "/checkout"));
    }

    #[test]
    fn qa_readiness_marks_missing_repo_without_browser_runner() {
        let files = vec![("src/main.rs".to_string(), 200)];
        let manifests = vec![ManifestSummary {
            path: "Cargo.toml".to_string(),
            kind: "cargo.toml".to_string(),
            name: Some("demo".to_string()),
            version: None,
            dependencies: Vec::new(),
            scripts: Vec::new(),
        }];
        let entrypoints = infer_entrypoints(&files, &manifests, &["Rust".to_string()]);

        let readiness = build_qa_readiness(&files, &manifests, &entrypoints);

        assert_eq!(readiness.status, "missing");
        assert!(readiness.score < 45);
        assert!(readiness.suggested_flows.is_empty());
    }

    #[test]
    fn repo_graph_contains_core_repo_relationships_deterministically() {
        let root =
            std::env::temp_dir().join(format!("codevetter-graph-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src-tauri/src/commands")).expect("commands dir");
        std::fs::create_dir_all(root.join("src-tauri/src/db")).expect("db dir");
        std::fs::create_dir_all(root.join("src/pages")).expect("pages dir");
        std::fs::create_dir_all(root.join("tests/e2e")).expect("tests dir");
        std::fs::write(
            root.join("src-tauri/src/commands/review.rs"),
            r#"
#[tauri::command]
pub async fn run_review() -> Result<(), String> {
    Ok(())
}
"#,
        )
        .expect("command file");
        std::fs::write(
            root.join("src-tauri/src/db/schema.rs"),
            r##"
const MIGRATION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS local_reviews (
    id TEXT PRIMARY KEY
);
"#;
"##,
        )
        .expect("schema file");
        std::fs::write(
            root.join("src/pages/Review.tsx"),
            "// DECISION: review page owns the primary user flow\nexport default function Review() { return null; }\n",
        )
        .expect("page file");
        std::fs::write(
            root.join("tests/e2e/review.spec.ts"),
            "test('review', () => {});\n",
        )
        .expect("test file");

        let files = vec![
            ("package.json".to_string(), 200),
            ("src-tauri/src/commands/review.rs".to_string(), 200),
            ("src-tauri/src/db/schema.rs".to_string(), 200),
            ("src/pages/Review.tsx".to_string(), 200),
            ("tests/e2e/review.spec.ts".to_string(), 200),
        ];
        let manifests = vec![package_manifest(
            "package.json",
            &["dev", "test:e2e"],
            &["@playwright/test", "react"],
        )];
        let entrypoints = infer_entrypoints(&files, &manifests, &["React".to_string()]);

        let graph = build_repo_graph(&root, &files, &manifests, &entrypoints);
        let graph_again = build_repo_graph(&root, &files, &manifests, &entrypoints);

        assert_eq!(
            serde_json::to_string(&graph).expect("graph json"),
            serde_json::to_string(&graph_again).expect("graph json")
        );
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == "script" && node.label == "test:e2e"));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == "route" && node.label == "/review"));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == "tauri_command" && node.label == "run_review"));
        assert!(graph
            .nodes
            .iter()
            .any(|node| node.kind == "db_table" && node.label == "local_reviews"));
        assert!(graph.nodes.iter().any(|node| node.kind == "test"));
        assert!(graph.nodes.iter().any(|node| node.kind == "decision"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "defines"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "routes_to"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "persists_to"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "decided_by"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn history_brief_collects_decisions_and_verification_hints_deterministically() {
        let root =
            std::env::temp_dir().join(format!("codevetter-history-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).expect("src dir");
        std::fs::create_dir_all(root.join("tests")).expect("tests dir");
        std::fs::write(
            root.join("src/review.ts"),
            "// DECISION: review keeps proof local\nexport const proof = true;\n",
        )
        .expect("source file");
        std::fs::write(
            root.join("tests/review.test.ts"),
            "test('proof', () => {});\n",
        )
        .expect("test file");

        let files = vec![
            ("package.json".to_string(), 200),
            ("src/review.ts".to_string(), 200),
            ("tests/review.test.ts".to_string(), 200),
        ];
        let manifests = vec![package_manifest(
            "package.json",
            &["lint", "test:review-proof"],
            &["react"],
        )];

        let brief = build_history_brief(&root, &files, &manifests);
        let brief_again = build_history_brief(&root, &files, &manifests);

        assert_eq!(
            serde_json::to_string(&brief).expect("history brief json"),
            serde_json::to_string(&brief_again).expect("history brief json")
        );
        assert_eq!(brief.schema_version, 1);
        assert!(brief.summary.contains("decision marker"));
        assert!(brief
            .decisions
            .iter()
            .any(|decision| decision.source == "src/review.ts#L1"));
        assert!(brief
            .test_hints
            .iter()
            .any(|hint| hint.path == "package.json" && hint.reason.contains("lint")));
        assert!(brief
            .test_hints
            .iter()
            .any(|hint| hint.path == "tests/review.test.ts"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_recent_git_commit_line() {
        let commit = parse_git_commit_line(
            "1234567890abcdef\x1f2026-06-12\x1fAdd Repo Unpacked history brief",
        )
        .expect("commit line");

        assert_eq!(commit.sha, "1234567890ab");
        assert_eq!(commit.date.as_deref(), Some("2026-06-12"));
        assert_eq!(commit.subject, "Add Repo Unpacked history brief");
        assert!(parse_git_commit_line("bad").is_none());
    }

    #[test]
    fn agent_context_sidecar_exports_graph_and_history() {
        let inventory = minimal_inventory();
        let sidecar = render_agent_context_sidecar("demo", "2026-06-12T00:00:00Z", &inventory);

        assert!(sidecar.contains("# Agent Context Sidecar"));
        assert!(sidecar.contains("repo_graph.v1 / history_brief.v1"));
        assert!(sidecar.contains("review keeps proof local"));
        assert!(sidecar.contains("src/review.ts#L1"));
        assert!(sidecar.contains("file:src-review-ts"));
        assert!(sidecar.contains("decided_by"));
    }

    #[test]
    fn imports_codevetter_repo_graph_from_wrapper_json() {
        let raw = serde_json::json!({
            "repo_graph": {
                "schema_version": 1,
                "nodes": [
                    {
                        "id": "file:src-review-ts",
                        "kind": "file",
                        "label": "src/review.ts",
                        "path": "src/review.ts",
                        "detail": "changed file",
                        "sources": ["src/review.ts"]
                    }
                ],
                "edges": [],
                "truncated": false
            }
        });

        let result = import_repo_graph_from_value(&raw).expect("imported graph");

        assert_eq!(result.source_kind, "repo_graph");
        assert_eq!(result.node_count, 1);
        assert_eq!(result.edge_count, 0);
        assert!(result.warnings.is_empty());
        assert_eq!(result.graph.nodes[0].label, "src/review.ts");
    }

    #[test]
    fn imports_loose_graph_json_with_source_target_edges() {
        let raw = serde_json::json!({
            "graph": {
                "nodes": [
                    {
                        "id": "a",
                        "type": "file",
                        "name": "src/a.ts",
                        "file_path": "src/a.ts"
                    },
                    {
                        "id": "b",
                        "type": "test",
                        "name": "tests/a.test.ts"
                    }
                ],
                "edges": [
                    {
                        "source": "a",
                        "target": "b",
                        "type": "tests",
                        "description": "test covers file"
                    }
                ]
            }
        });

        let result = import_repo_graph_from_value(&raw).expect("loose graph");

        assert_eq!(result.source_kind, "graph");
        assert_eq!(result.node_count, 2);
        assert_eq!(result.edge_count, 1);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("normalized")));
        assert_eq!(result.graph.nodes[0].kind, "file");
        assert_eq!(result.graph.nodes[0].path.as_deref(), Some("src/a.ts"));
        assert_eq!(result.graph.edges[0].kind, "tests");
        assert_eq!(result.graph.edges[0].evidence, "test covers file");
    }

    #[test]
    fn repo_inventory_deserializes_old_reports_without_qa_readiness_or_repo_graph() {
        let raw = serde_json::json!({
            "repo_path": "/tmp/demo",
            "repo_name": "demo",
            "commit_sha": null,
            "branch": null,
            "remote_url": null,
            "files_scanned": 0,
            "files_skipped": 0,
            "bytes_scanned": 0,
            "max_files_hit": false,
            "languages": [],
            "manifests": [],
            "entrypoints": [],
            "top_level_dirs": [],
            "docs": [],
            "config_files": [],
            "stack_tags": [],
            "all_files": [],
            "ignored_dirs": []
        });

        let inventory: RepoInventory = serde_json::from_value(raw).expect("legacy inventory");

        assert_eq!(inventory.qa_readiness.status, "missing");
        assert_eq!(inventory.qa_readiness.score, 0);
        assert_eq!(inventory.repo_graph.schema_version, 1);
        assert!(inventory.repo_graph.nodes.is_empty());
        assert_eq!(inventory.history_brief.schema_version, 1);
        assert!(inventory.history_brief.recent_commits.is_empty());
    }
}
