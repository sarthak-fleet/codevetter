//! Repo Unpacked — whole-repository system briefs.
//!
//! Two-pass pipeline:
//!   1. Deterministic scanner builds a repo inventory (entrypoints, manifests,
//!      stack, language counts, top dirs, README/docs).
//!   2. Synthesis prompt is sent to the configured CLI agent (claude/gemini/codex/grok/cursor).
//!      Returns five sections — system_map, feature_catalog, behavior_traces,
//!      risk_map, agent_handoff — every claim is required to cite at least
//!      one source file path that exists in the inventory.
//!
//! Result rows live in `repo_unpacked_reports`. Inventory is stored alongside
//! the synthesised brief so the UI can re-render without re-paying LLM cost.

use crate::commands::cli_stream::{run_cli_prompt_streaming, CliStreamContext};
use crate::commands::unpack_analysis::{
    build_history_brief, build_history_brief_with_previews, build_repo_graph_with_previews,
    build_repo_health, build_repo_health_with_previews, build_source_preview_cache,
};
use crate::commands::unpack_export::{
    render_agent_context_sidecar, render_html, render_markdown, render_repo_memory_markdown,
};
use crate::commands::unpack_inventory::{
    build_workspace_units, infer_entrypoints, infer_stack, language_for_path,
    manifest_candidate_paths, parse_manifest, read_first_bytes,
};
use crate::commands::unpack_outcome::build_unpack_outcome_evidence;
use crate::commands::unpack_qa::build_qa_readiness;
use crate::commands::unpack_scan::{
    build_dir_tree_preview, parallel_walk_repo_with_progress, MAX_FILES,
};
use crate::commands::unpack_snapshot::build_snapshot_commit_range;
use crate::db::queries;
use crate::DbState;
#[allow(unused_imports)]
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tauri::{AppHandle, Emitter, State};

const README_PREVIEW_BYTES: usize = 8 * 1024;
/// Full file list stays in SQLite for synthesis; IPC to the webview is capped.
const CLIENT_ALL_FILES_LIMIT: usize = 512;

// ─── Public types (mirrored on the TS side) ─────────────────────────────────

pub use crate::commands::unpack_types::*;

// ─── Tauri commands ─────────────────────────────────────────────────────────

fn emit_unpack_progress(
    app: &AppHandle,
    report_id: &str,
    repo_path: &str,
    phase: &str,
    detail: Option<&str>,
) {
    let _ = app.emit(
        "unpack-progress",
        json!({
            "report_id": report_id,
            "repo_path": repo_path,
            "phase": phase,
            "detail": detail,
        }),
    );
}

/// Shrink inventory payloads crossing the Tauri IPC boundary (React chokes on 4k paths).
pub fn trim_inventory_for_client(mut inv: RepoInventory) -> RepoInventory {
    inv.all_files_capped = inv.files_scanned > CLIENT_ALL_FILES_LIMIT;
    // Tree preview is built in Rust; never ship the raw file list to the webview.
    inv.all_files.clear();
    inv
}

/// Run agent synthesis on an existing inventory snapshot (no re-scan).
#[tauri::command]
pub async fn synthesize_unpack_report(
    app: AppHandle,
    db: State<'_, DbState>,
    report_id: String,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    let model_trimmed = model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let started = std::time::Instant::now();

    let (inventory, repo_path) = load_report_inventory(&db, &report_id, true)?;

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        crate::db::with_busy_retry(
            || {
                conn.execute(
                    "UPDATE repo_unpacked_reports
                     SET status = 'running', agent_used = ?1, error_message = NULL, started_at = ?2
                     WHERE id = ?3",
                    rusqlite::params![agent, now, report_id],
                )
            },
            15,
        )
        .map_err(|e| e.to_string())?;
    }

    run_unpack_synthesis(
        app,
        db,
        report_id,
        repo_path,
        inventory,
        agent,
        model_trimmed,
        started,
        true,
    )
    .await
}

async fn run_unpack_synthesis(
    app: AppHandle,
    db: State<'_, DbState>,
    report_id: String,
    repo_path: String,
    inventory: RepoInventory,
    agent: String,
    model_trimmed: Option<String>,
    started: std::time::Instant,
    preserve_inventory_on_failure: bool,
) -> Result<Value, String> {
    emit_unpack_progress(
        &app,
        &report_id,
        &repo_path,
        "synthesizing",
        Some(&format!("Running {agent} synthesis")),
    );

    let prompt = build_synthesis_prompt(&inventory);

    let cli_cmd = match agent.as_str() {
        "gemini" => "gemini",
        "codex" => "codex",
        "grok" => "grok",
        "cursor" => "cursor",
        "command-code" => "cmd",
        _ => "claude",
    };

    let stream_ctx = CliStreamContext {
        app: app.clone(),
        stream_id: report_id.clone(),
        repo_path: repo_path.clone(),
        agent: agent.clone(),
    };
    let repo_path_for_cli = repo_path.clone();
    let prompt_for_cli = prompt.clone();
    let model_for_cli = model_trimmed.clone();
    let raw = match tokio::task::spawn_blocking(move || {
        run_cli_prompt_streaming(
            &stream_ctx,
            &repo_path_for_cli,
            &prompt_for_cli,
            model_for_cli.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("cli task join error: {e}"))?
    {
        Ok(text) => text,
        Err(e) => {
            mark_unpack_failed(
                &db,
                &report_id,
                &e,
                started.elapsed().as_millis() as i64,
                preserve_inventory_on_failure,
            );
            return Err(e);
        }
    };
    let json_str = match crate::commands::review::extract_json_from_output_pub(&raw) {
        Some(s) => s,
        None => {
            let preview = raw.chars().take(1200).collect::<String>();
            let msg =
                format!("Could not find JSON in {cli_cmd} output. First 1200 chars:\n{preview}");
            mark_unpack_failed(
                &db,
                &report_id,
                &msg,
                started.elapsed().as_millis() as i64,
                preserve_inventory_on_failure,
            );
            return Err(msg);
        }
    };

    let parsed: Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("Failed to parse JSON: {e}");
            mark_unpack_failed(
                &db,
                &report_id,
                &msg,
                started.elapsed().as_millis() as i64,
                preserve_inventory_on_failure,
            );
            return Err(msg);
        }
    };

    emit_unpack_progress(
        &app,
        &report_id,
        &repo_path,
        "saving",
        Some("Persisting brief snapshot"),
    );
    let report = normalize_report(&parsed, &inventory);
    let report_json = serde_json::to_string(&report).map_err(|e| e.to_string())?;
    let runtime_ms = started.elapsed().as_millis() as i64;
    let model = model_trimmed
        .clone()
        .or_else(|| {
            parsed
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from)
        })
        .or_else(|| Some(format!("cli:{cli_cmd}")));

    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let completed_at = chrono::Utc::now().to_rfc3339();
        let activity = queries::ActivityInput {
            agent_id: None,
            event_type: Some("repo_unpacked_completed".to_string()),
            summary: Some(format!(
                "Repo Unpacked brief generated for {}: {} files",
                inventory.repo_name, inventory.files_scanned
            )),
            metadata: Some(json!({"report_id": report_id}).to_string()),
        };
        let repo_path_for_touch = inventory.repo_path.clone();
        crate::db::with_busy_retry(
            || {
                conn.execute(
                    "UPDATE repo_unpacked_reports
                     SET status = 'completed', report_json = ?1, runtime_ms = ?2,
                         model_used = ?3, completed_at = ?4, error_message = NULL
                     WHERE id = ?5",
                    rusqlite::params![report_json, runtime_ms, model, completed_at, report_id,],
                )?;
                queries::log_activity(&conn, &activity)?;
                conn.execute(
                    "UPDATE repo_projects SET last_unpack_at = ?2 WHERE repo_path = ?1",
                    rusqlite::params![repo_path_for_touch, completed_at],
                )?;
                Ok(())
            },
            20,
        )
        .map_err(|e| e.to_string())?;
    }

    emit_unpack_progress(&app, &report_id, &repo_path, "completed", None);
    Ok(json!({
        "report_id": report_id,
        "status": "completed",
        "runtime_ms": runtime_ms,
        "report": report,
        "inventory": trim_inventory_for_client(inventory),
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
                        cost_usd, started_at, completed_at, created_at,
                        report_json IS NOT NULL
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
                        cost_usd, started_at, completed_at, created_at,
                        report_json IS NOT NULL
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

    let mut row = conn
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

    if let Some(inv_json) = row
        .get("inventory_json")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Ok(inv) = serde_json::from_str::<RepoInventory>(inv_json) {
            if let Ok(trimmed) = serde_json::to_string(&trim_inventory_for_client(inv)) {
                row["inventory_json"] = json!(trimmed);
            }
        }
    }

    Ok(row)
}

/// Cancel an in-flight unpack CLI synthesis (by `report_id` / stream id).
#[tauri::command]
pub fn cancel_unpack_generation(report_id: String) -> bool {
    crate::commands::cli_stream::cancel_cli_stream(&report_id)
}

#[tauri::command]
pub async fn compare_unpack_snapshot_commits(
    repo_path: String,
    base_commit: String,
    head_commit: String,
) -> Result<SnapshotCommitRange, String> {
    tokio::task::spawn_blocking(move || {
        build_snapshot_commit_range(&repo_path, &base_commit, &head_commit, 24)
    })
    .await
    .map_err(|e| format!("snapshot comparison task join error: {e}"))?
}

#[tauri::command]
pub async fn get_unpack_outcome_evidence(
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<UnpackOutcomeEvidence, String> {
    let repo_path = repo_path.trim().to_string();
    if repo_path.is_empty() {
        return Err("repo_path is required".to_string());
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    build_unpack_outcome_evidence(&conn, &repo_path).map_err(|e| e.to_string())
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
        "repo_memory_markdown" => {
            let Some(inventory) = inventory.as_ref() else {
                return Err("Report missing inventory memory.".to_string());
            };
            render_repo_memory_markdown(&repo_name, &created_at, inventory, Some(&report))
        }
        _ => body,
    };

    Ok(json!({ "content": content, "format": format }))
}

// ─── Inventory builder (deterministic) ──────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryBuildProfile {
    /// Parallel walk + lightweight metadata only. No file-content scans.
    Fast,
    /// Adds deterministic graph, git history, and health analysis (hundreds of file reads).
    Full,
}

#[derive(Debug, Clone)]
pub struct InventoryBuildResult {
    pub inventory: RepoInventory,
    pub profile: super::unpack_scan_profile::UnpackScanProfile,
}

pub fn build_inventory_with_progress(
    repo_path: &str,
    progress: Option<super::unpack_scan::ScanProgressCallback>,
    profile: InventoryBuildProfile,
) -> Result<InventoryBuildResult, String> {
    let stage = if profile == InventoryBuildProfile::Fast {
        "fast_scan"
    } else {
        "full_scan"
    };
    let mut profiler = super::unpack_scan_profile::UnpackScanProfiler::new(stage);

    let root = PathBuf::from(repo_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {repo_path}"));
    }

    let repo_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo_path.to_string());

    let (commit_sha, branch, remote_url) = read_git_metadata(&root);
    profiler.step("git_metadata", "Git metadata");

    let walk = parallel_walk_repo_with_progress(&root, progress.clone());
    profiler.step("file_walk", "Parallel file walk");
    let tracked_files = walk.tracked_files;
    let all_files = walk.files;
    let files_skipped = walk.files_skipped;
    let bytes_scanned = walk.bytes_scanned;
    let max_files_hit = walk.max_files_hit;
    let walk_estimated_total_files = walk.estimated_total_files;
    let ignored_dirs = walk.ignored_dirs;
    let estimated_total_files = if max_files_hit {
        walk_estimated_total_files.or_else(|| count_git_tracked_files(&root))
    } else {
        None
    };
    if max_files_hit {
        profiler.step("coverage", "Coverage denominator");
    }

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
    profiler.step("languages", "Language breakdown");

    // Manifests (only plausible manifest paths — avoid scanning 4k basenames)
    let mut manifests: Vec<ManifestSummary> = Vec::new();
    for path in manifest_candidate_paths(&all_files, tracked_files.as_deref()) {
        if let Some(m) = parse_manifest(&root, &path) {
            manifests.push(m);
        }
    }
    profiler.step("manifests", "Manifest parsing");

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
    docs.truncate(if profile == InventoryBuildProfile::Fast {
        12
    } else {
        40
    });
    profiler.step("docs", "Doc previews");

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
    let coverage = build_inventory_coverage(
        &all_files,
        tracked_files.as_deref(),
        estimated_total_files,
        max_files_hit,
    );

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

    profiler.step("aggregates", "Dirs, configs, stack tags");

    // Stack tags
    let stack_tags = infer_stack(&all_files, &manifests);

    // Entrypoints
    let entrypoints = infer_entrypoints(&all_files, &manifests, &stack_tags);
    let qa_readiness = build_qa_readiness(&all_files, &manifests, &entrypoints);
    let workspace_units = build_workspace_units(
        &all_files,
        tracked_files.as_deref(),
        &manifests,
        &entrypoints,
    );
    profiler.step("entrypoints", "Entrypoints, workspace units & QA readiness");
    let (repo_graph, history_brief, repo_health) = if profile == InventoryBuildProfile::Full {
        if let Some(ref cb) = progress {
            cb(super::unpack_scan::ScanProgress {
                phase: "analyze",
                detail: format!("Building deterministic graph · {} files…", all_files.len()),
                files_seen: all_files.len(),
                files_skipped,
            });
        }
        let source_previews = build_source_preview_cache(&root, &all_files);
        profiler.step("source_previews", "Source preview cache");

        let repo_graph = build_repo_graph_with_previews(
            &root,
            &all_files,
            &manifests,
            &entrypoints,
            &workspace_units,
            Some(&source_previews),
        );
        profiler.step("repo_graph", "Deterministic graph scan");

        if let Some(ref cb) = progress {
            cb(super::unpack_scan::ScanProgress {
                phase: "analyze",
                detail: "Reading git history and decision markers…".to_string(),
                files_seen: all_files.len(),
                files_skipped,
            });
        }
        let history_brief = build_history_brief_with_previews(
            &root,
            &all_files,
            &manifests,
            Some(&source_previews),
        );
        profiler.step("history", "Git history & decision scan");

        if let Some(ref cb) = progress {
            cb(super::unpack_scan::ScanProgress {
                phase: "analyze",
                detail: "Scoring deterministic repo health…".to_string(),
                files_seen: all_files.len(),
                files_skipped,
            });
        }
        let repo_health =
            build_repo_health_with_previews(&root, &all_files, Some(&source_previews));
        profiler.step("repo_health", "Deterministic health scoring");

        (repo_graph, history_brief, repo_health)
    } else {
        if let Some(ref cb) = progress {
            cb(super::unpack_scan::ScanProgress {
                phase: "analyze",
                detail: format!("Finalizing snapshot · {} files", all_files.len()),
                files_seen: all_files.len(),
                files_skipped,
            });
        }
        (
            super::unpack_fast_graph::build_fast_repo_graph(
                &repo_name,
                &all_files,
                &manifests,
                &entrypoints,
                &workspace_units,
                &top_level_dirs,
                &docs,
                &config_files,
            ),
            default_history_brief(),
            default_repo_health(),
        )
    };
    if profile == InventoryBuildProfile::Fast {
        profiler.step("finalize", "Finalize snapshot (deferred analysis)");
    }

    let path_strings: Vec<String> = all_files.iter().map(|(p, _)| p.clone()).collect();
    let dir_tree_preview = build_dir_tree_preview(&path_strings, all_files.len());
    profiler.step("dir_tree", "Directory tree preview");

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
        estimated_total_files,
        languages,
        manifests,
        entrypoints,
        top_level_dirs,
        docs,
        config_files,
        stack_tags,
        workspace_units,
        qa_readiness,
        repo_graph,
        history_brief,
        repo_health,
        all_files: path_strings,
        ignored_dirs,
        coverage,
        all_files_capped: false,
        dir_tree_preview,
    };

    Ok(InventoryBuildResult {
        inventory,
        profile: profiler.finish(),
    })
}

fn read_git_metadata(root: &Path) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(metadata) = read_git_metadata_from_files(root) {
        return metadata;
    }

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

fn count_git_tracked_files(root: &Path) -> Option<usize> {
    let output = StdCommand::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .count(),
    )
}

fn build_inventory_coverage(
    sampled_files: &[(String, u64)],
    tracked_files: Option<&[String]>,
    estimated_total_files: Option<usize>,
    capped: bool,
) -> InventoryCoverageSummary {
    let total_files = estimated_total_files.or_else(|| tracked_files.map(|files| files.len()));
    let sample_percent = total_files.filter(|total| *total > 0).map(|total| {
        let pct = (sampled_files.len() as f64 / total as f64) * 100.0;
        (pct * 10.0).round() / 10.0
    });
    let strategy = if capped && tracked_files.is_some() {
        "stratified_git_sample"
    } else if capped {
        "bounded_walk_sample"
    } else {
        "full_walk"
    };

    let language_source: Vec<(&str, u64)> = if let Some(files) = tracked_files {
        files.iter().map(|path| (path.as_str(), 0)).collect()
    } else {
        sampled_files
            .iter()
            .map(|(path, size)| (path.as_str(), *size))
            .collect()
    };

    let mut lang_map: HashMap<&'static str, (usize, u64)> = HashMap::new();
    let mut dir_map: HashMap<String, (usize, u64)> = HashMap::new();
    for (path, bytes) in language_source {
        if let Some(lang) = language_for_path(path) {
            let entry = lang_map.entry(lang).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += bytes;
        }
        if let Some(top) = path.split('/').next() {
            if path.contains('/') {
                let entry = dir_map.entry(top.to_string()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += bytes;
            }
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
    languages.sort_by(|a, b| {
        b.files
            .cmp(&a.files)
            .then_with(|| a.language.cmp(&b.language))
    });
    languages.truncate(20);

    let mut top_level_dirs: Vec<DirSummary> = dir_map
        .into_iter()
        .map(|(path, (file_count, bytes))| DirSummary {
            path,
            file_count,
            bytes,
        })
        .collect();
    top_level_dirs.sort_by(|a, b| {
        b.file_count
            .cmp(&a.file_count)
            .then_with(|| a.path.cmp(&b.path))
    });
    top_level_dirs.truncate(24);

    let mut notes = Vec::new();
    if capped {
        notes.push(
            "Whole-repo metadata is based on Git tracked paths; graph and health are based on the representative deep-scan sample."
                .to_string(),
        );
    } else {
        notes.push("Full local walk covered the repo within the scan cap.".to_string());
    }

    InventoryCoverageSummary {
        schema_version: 1,
        strategy: strategy.to_string(),
        sampled_files: sampled_files.len(),
        total_files,
        sample_percent,
        languages,
        top_level_dirs,
        notes,
    }
}

fn read_git_metadata_from_files(
    root: &Path,
) -> Option<(Option<String>, Option<String>, Option<String>)> {
    let git_dir = resolve_git_dir(root)?;
    let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    let (sha, branch) = if let Some(reference) = head.strip_prefix("ref: ") {
        let reference = reference.trim();
        let branch = reference
            .strip_prefix("refs/heads/")
            .map(|value| value.to_string());
        let sha = read_git_ref(&git_dir, reference);
        (sha, branch)
    } else if is_git_sha(head) {
        (Some(head.to_string()), None)
    } else {
        (None, None)
    };

    let remote = read_origin_remote(&git_dir);
    Some((sha, branch, remote))
}

fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    for dir in root.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }

        if dot_git.is_file() {
            let raw = fs::read_to_string(&dot_git).ok()?;
            let gitdir = raw.trim().strip_prefix("gitdir:")?.trim();
            let path = PathBuf::from(gitdir);
            return if path.is_absolute() {
                Some(path)
            } else {
                Some(dir.join(path))
            };
        }
    }
    None
}

fn read_git_ref(git_dir: &Path, reference: &str) -> Option<String> {
    let loose = git_dir.join(reference);
    if let Ok(value) = fs::read_to_string(loose) {
        let sha = value.trim();
        if is_git_sha(sha) {
            return Some(sha.to_string());
        }
    }

    let packed = fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let sha = parts.next()?;
        let name = parts.next()?;
        if name == reference && is_git_sha(sha) {
            return Some(sha.to_string());
        }
    }
    None
}

fn read_origin_remote(git_dir: &Path) -> Option<String> {
    let config = fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_origin = trimmed == r#"[remote "origin"]"#;
            continue;
        }
        if in_origin {
            if let Some(url) = trimmed.strip_prefix("url") {
                let url = url.trim_start();
                if let Some(value) = url.strip_prefix('=') {
                    let value = value.trim();
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    None
}

fn is_git_sha(value: &str) -> bool {
    value.len() >= 7 && value.len() <= 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

// ─── Inventory loading + prompts ────────────────────────────────────────────

pub(crate) fn inventory_needs_enrichment(inventory: &RepoInventory) -> bool {
    let default_history_summary = default_history_brief().summary;
    let default_health_summary = default_repo_health().summary;
    inventory.repo_health.files_analyzed == 0
        || inventory.repo_health.summary == default_health_summary
        || inventory.history_brief.summary == default_history_summary
}

pub fn try_enrich_stored_unpack_inventory(
    app: &tauri::AppHandle,
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    report_id: &str,
    progress: Option<super::unpack_scan::ScanProgressCallback>,
) -> Result<RepoInventory, String> {
    enrich_stored_unpack_inventory_inner(app, db, report_id, progress, true)
}

fn enrich_stored_unpack_inventory_inner(
    app: &tauri::AppHandle,
    db: &std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    report_id: &str,
    progress: Option<super::unpack_scan::ScanProgressCallback>,
    opportunistic: bool,
) -> Result<RepoInventory, String> {
    let mut profiler = super::unpack_scan_profile::UnpackScanProfiler::new("background_enrich");

    let (mut inventory, repo_path) = {
        let conn = lock_unpack_db(db, opportunistic)?;
        let row: (Option<String>, String) = conn
            .query_row(
                "SELECT inventory_json, repo_path FROM repo_unpacked_reports WHERE id = ?1",
                rusqlite::params![report_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("Report not found: {e}"))?;
        let inv_json = row
            .0
            .ok_or_else(|| "Snapshot has no inventory.".to_string())?;
        let inventory: RepoInventory =
            serde_json::from_str(&inv_json).map_err(|e| format!("Invalid inventory JSON: {e}"))?;
        (inventory, row.1)
    };
    profiler.step("load_db", "Load snapshot from DB");

    if !inventory_needs_enrichment(&inventory) {
        return Ok(inventory);
    }

    let root = PathBuf::from(&repo_path);
    use rayon::prelude::*;
    let files: Vec<(String, u64)> = inventory
        .all_files
        .par_iter()
        .map(|path| {
            let size = fs::metadata(root.join(path))
                .map(|meta| meta.len())
                .unwrap_or(0);
            (path.clone(), size)
        })
        .collect();
    profiler.step("file_stats", "File size metadata");

    if let Some(ref cb) = progress {
        cb(super::unpack_scan::ScanProgress {
            phase: "analyze",
            detail: "Building code graph from source files…".to_string(),
            files_seen: files.len(),
            files_skipped: 0,
        });
    }
    inventory.repo_graph = build_repo_graph_with_previews(
        &root,
        &files,
        &inventory.manifests,
        &inventory.entrypoints,
        &inventory.workspace_units,
        None,
    );
    profiler.step("repo_graph", "Code graph scan");

    if let Some(ref cb) = progress {
        cb(super::unpack_scan::ScanProgress {
            phase: "analyze",
            detail: "Reading git history and decision markers…".to_string(),
            files_seen: files.len(),
            files_skipped: 0,
        });
    }
    inventory.history_brief = build_history_brief(&root, &files, &inventory.manifests);
    profiler.step("history", "Git history & decisions");

    if let Some(ref cb) = progress {
        cb(super::unpack_scan::ScanProgress {
            phase: "analyze",
            detail: "Scoring repo health signals…".to_string(),
            files_seen: files.len(),
            files_skipped: 0,
        });
    }
    inventory.repo_health = build_repo_health(&root, &files);
    profiler.step("repo_health", "Health scoring");

    let inventory_json = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
    profiler.step("serialize", "JSON serialize");
    {
        let conn = lock_unpack_db(db, opportunistic)?;
        let update = || {
            conn.execute(
                "UPDATE repo_unpacked_reports SET inventory_json = ?1 WHERE id = ?2",
                rusqlite::params![inventory_json, report_id],
            )
        };
        if opportunistic {
            update().map_err(|e| e.to_string())?;
        } else {
            crate::db::with_busy_retry(update, 15).map_err(|e| e.to_string())?;
        }
    }
    profiler.step("db_update", "SQLite update");

    let profile = profiler.finish();
    super::unpack_scan_profile::emit_unpack_scan_profile(app, report_id, &repo_path, &profile);

    let _ = app.emit(
        "unpack-inventory-enriched",
        serde_json::json!({
            "report_id": report_id,
            "repo_path": repo_path,
            "inventory": trim_inventory_for_client(inventory.clone()),
            "graph_nodes": inventory.repo_graph.nodes.len(),
            "health_files": inventory.repo_health.files_analyzed,
            "profile": profile,
        }),
    );

    Ok(inventory)
}

fn lock_unpack_db<'a>(
    db: &'a std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    opportunistic: bool,
) -> Result<std::sync::MutexGuard<'a, rusqlite::Connection>, String> {
    if opportunistic {
        db.try_lock()
            .map_err(|_| "background enrich skipped: database busy".to_string())
    } else {
        db.lock().map_err(|e| e.to_string())
    }
}

fn load_report_inventory(
    db: &State<'_, DbState>,
    report_id: &str,
    block_if_running: bool,
) -> Result<(RepoInventory, String), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let row: (Option<String>, String, String) = conn
        .query_row(
            "SELECT inventory_json, repo_path, status
             FROM repo_unpacked_reports WHERE id = ?1",
            rusqlite::params![report_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| format!("Report not found: {e}"))?;
    let (inv_json, repo_path, status) = row;
    let inv_json = inv_json
        .ok_or_else(|| "This snapshot has no inventory. Unpack the repo first.".to_string())?;
    if inv_json.trim().is_empty() {
        return Err("This snapshot has no inventory. Unpack the repo first.".to_string());
    }
    if block_if_running && status == "running" {
        return Err("An AI task is already in progress for this snapshot.".to_string());
    }
    let inventory: RepoInventory =
        serde_json::from_str(&inv_json).map_err(|e| format!("Invalid inventory JSON: {e}"))?;
    Ok((inventory, repo_path))
}

/// Default AI operation: full evidence-backed system brief (summary).
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

    append_inventory_context(&mut buf, inv);
    buf
}

fn build_unpack_ask_prompt(inv: &RepoInventory, question: &str) -> String {
    let mut buf = String::new();
    buf.push_str(
        "You are CodeVetter Repo Unpacked. Answer the user's question about the repo below. \
Use your file-read and search tools to investigate before answering. \
Cite concrete file paths from the inventory. Be direct and practical.\n\n",
    );
    buf.push_str(&format!("User question:\n{question}\n\n"));
    buf.push_str(
        "Answer in plain text (markdown OK). Do not return JSON unless the question explicitly asks for structured data.\n\n",
    );
    append_inventory_context(&mut buf, inv);
    buf
}

fn append_inventory_context(buf: &mut String, inv: &RepoInventory) {
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

    buf.push_str("\nDeterministic repo health:\n");
    buf.push_str(&format!(
        "  - schema={} analyzed={} average={:.1}/10 hotspots={}{} — {}\n",
        inv.repo_health.schema_version,
        inv.repo_health.files_analyzed,
        inv.repo_health.average_score,
        inv.repo_health.hotspot_count,
        if inv.repo_health.truncated {
            " truncated"
        } else {
            ""
        },
        inv.repo_health.summary
    ));
    for file in inv.repo_health.top_files.iter().take(10) {
        buf.push_str(&format!(
            "  - {} score={:.1}/10 bucket={} lines={} churn={} test_signal={}\n",
            file.path, file.score, file.bucket, file.lines, file.churn, file.has_test_signal
        ));
        for finding in file.findings.iter().take(4) {
            buf.push_str(&format!(
                "      - {} [{}:{}] {}\n",
                finding.label, finding.dimension, finding.severity, finding.detail
            ));
        }
        for target in file.refactoring_targets.iter().take(2) {
            buf.push_str(&format!("      - refactor lead: {target}\n"));
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
    if !inv.history_brief.temporal_couplings.is_empty() {
        buf.push_str("  Co-change clusters:\n");
        for coupling in inv.history_brief.temporal_couplings.iter().take(8) {
            buf.push_str(&format!(
                "    - {} — {} commit{}{}; {}\n",
                coupling.files.join(" + "),
                coupling.commit_count,
                if coupling.commit_count == 1 { "" } else { "s" },
                coupling
                    .last_commit
                    .as_deref()
                    .map(|commit| format!("; latest {commit}"))
                    .unwrap_or_default(),
                coupling.reason
            ));
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
}

/// Ask a custom question against an existing unpack snapshot (no re-scan, no brief overwrite).
#[tauri::command]
pub async fn ask_unpack_report(
    app: AppHandle,
    db: State<'_, DbState>,
    report_id: String,
    stream_id: String,
    question: String,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("Question is empty.".to_string());
    }
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    let model_trimmed = model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);

    let (inventory, repo_path) = load_report_inventory(&db, &report_id, true)?;
    let prompt = build_unpack_ask_prompt(&inventory, &question);

    let stream_ctx = CliStreamContext {
        app: app.clone(),
        stream_id: stream_id.clone(),
        repo_path: repo_path.clone(),
        agent: agent.clone(),
    };

    let repo_path_for_cli = repo_path.clone();
    let prompt_for_cli = prompt.clone();
    let model_for_cli = model_trimmed.clone();
    let raw = tokio::task::spawn_blocking(move || {
        run_cli_prompt_streaming(
            &stream_ctx,
            &repo_path_for_cli,
            &prompt_for_cli,
            model_for_cli.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("cli task join error: {e}"))??;

    Ok(json!({
        "report_id": report_id,
        "question": question,
        "answer": raw.trim(),
        "agent": agent,
    }))
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

// ─── DB helpers ─────────────────────────────────────────────────────────────

fn mark_unpack_failed(
    db: &State<'_, DbState>,
    id: &str,
    msg: &str,
    runtime_ms: i64,
    preserve_inventory: bool,
) {
    if let Ok(conn) = db.0.lock() {
        let completed_at = chrono::Utc::now().to_rfc3339();
        let sql = if preserve_inventory {
            "UPDATE repo_unpacked_reports
             SET status = CASE
                    WHEN report_json IS NULL OR trim(report_json) = '' THEN 'scan_only'
                    ELSE status
                  END,
                 error_message = ?1,
                 runtime_ms = ?2,
                 completed_at = ?3
             WHERE id = ?4"
        } else {
            "UPDATE repo_unpacked_reports
             SET status = 'failed', error_message = ?1, runtime_ms = ?2, completed_at = ?3
             WHERE id = ?4"
        };
        let _ = crate::db::with_busy_retry(
            || conn.execute(sql, rusqlite::params![msg, runtime_ms, completed_at, id]),
            15,
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
        "analysis_ready": r.get::<_, bool>(15)?,
    }))
}

#[cfg(test)]
#[path = "unpack_tests.rs"]
mod tests;
