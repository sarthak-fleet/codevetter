use crate::commands::session_adapters::{
    ClaudeCodeAdapter, CodexAdapter, CursorAdapter, RawSessionAdapterSummary, SessionSourceAdapter,
};
use crate::db::queries;
use crate::DbState;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::sync::{LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

static FULL_INDEX_LOCK: Mutex<()> = Mutex::new(());

pub const LIVE_TRANSCRIPT_INITIAL_DELAY_SECS: u64 = 20;
pub const LIVE_TRANSCRIPT_INTERVAL_SECS: u64 = 10;
pub const LIVE_SECONDARY_ADAPTER_INTERVAL_SECS: u64 = 60;
pub const FULL_INDEX_RECOVERY_INTERVAL_SECS: u64 = 6 * 60 * 60;
const LIVE_TRANSCRIPT_SESSION_BYTE_BUDGET: usize = 64 * 1024;
const LIVE_TRANSCRIPT_DELIMITER_WINDOW_BYTES: usize = 4 * 1024;
const LIVE_TRANSCRIPT_TICK_BUDGET_MS: u64 = 150;
const LIVE_CODEX_DISCOVERY_SESSION_BUDGET: usize = 1;
const LIVE_DEFERRED_JSONL_MAX_ENTRIES: usize = 256;

static LIVE_DEFERRED_JSONL_ROWS: LazyLock<Mutex<HashMap<String, (i64, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
pub struct LiveSessionEvidencePolicy {
    pub schema_version: i64,
    pub mode: String,
    pub supported_incremental_adapters: Vec<String>,
    pub incremental_interval_secs: u64,
    pub secondary_adapter_interval_secs: u64,
    pub recovery: String,
    pub full_index_recovery_interval_secs: u64,
    pub update_event: String,
    pub local_only: bool,
    pub last_full_indexed_at: Option<String>,
}

pub fn live_session_evidence_policy(
    conn: &rusqlite::Connection,
) -> Result<LiveSessionEvidencePolicy, String> {
    Ok(LiveSessionEvidencePolicy {
        schema_version: 1,
        mode: "incremental_jsonl_poll".to_string(),
        supported_incremental_adapters: vec!["claude-code".to_string(), "codex".to_string()],
        incremental_interval_secs: LIVE_TRANSCRIPT_INTERVAL_SECS,
        secondary_adapter_interval_secs: LIVE_SECONDARY_ADAPTER_INTERVAL_SECS,
        recovery: "persisted_byte_cursor_plus_scheduled_or_manual_full_index".to_string(),
        full_index_recovery_interval_secs: FULL_INDEX_RECOVERY_INTERVAL_SECS,
        update_event: "session_archive_updated".to_string(),
        local_only: true,
        last_full_indexed_at: queries::get_preference(conn, "last_indexed_at")
            .map_err(|error| error.to_string())?,
    })
}

#[tauri::command]
pub async fn get_live_session_evidence_policy(
    db: State<'_, DbState>,
) -> Result<LiveSessionEvidencePolicy, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    live_session_evidence_policy(&conn)
}

#[derive(Debug, Clone)]
struct IndexedAdapterSession {
    session_id: String,
    source_ref: String,
    messages_indexed: u64,
    parse_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProductionAdapterRunStats {
    adapter_id: String,
    agent_type: String,
    source_roots: Vec<String>,
    sample_source_paths: Vec<String>,
    sample_session_ids: Vec<String>,
    parse_warnings: Vec<String>,
    sessions_indexed: i64,
    messages_indexed: i64,
    supports_incremental: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FullIndexSummary {
    pub indexed_sessions: u64,
    pub indexed_messages: u64,
    pub skipped_sessions: u64,
    pub archive_search_rows_indexed: i64,
    pub indexed_at: String,
}

impl FullIndexSummary {
    pub fn log_message(&self) -> String {
        format!(
            "sessions={}, messages={}, skipped={}, archive_search_rows_indexed={}",
            self.indexed_sessions,
            self.indexed_messages,
            self.skipped_sessions,
            self.archive_search_rows_indexed
        )
    }
}

impl ProductionAdapterRunStats {
    fn new(
        adapter_id: &str,
        agent_type: &str,
        source_roots: Vec<String>,
        supports_incremental: bool,
    ) -> Self {
        Self {
            adapter_id: adapter_id.to_string(),
            agent_type: agent_type.to_string(),
            source_roots,
            sample_source_paths: Vec::new(),
            sample_session_ids: Vec::new(),
            parse_warnings: Vec::new(),
            sessions_indexed: 0,
            messages_indexed: 0,
            supports_incremental,
        }
    }

    fn record_session(&mut self, session: &IndexedAdapterSession) {
        self.sessions_indexed += 1;
        self.messages_indexed += session.messages_indexed as i64;
        push_unique_limited(&mut self.sample_source_paths, session.source_ref.clone(), 3);
        push_unique_limited(&mut self.sample_session_ids, session.session_id.clone(), 3);
        for warning in &session.parse_warnings {
            self.record_warning(&session.source_ref, warning);
        }
    }

    fn record_warning(&mut self, source_ref: &str, warning: &str) {
        push_unique_limited(
            &mut self.parse_warnings,
            format!("{source_ref}: {warning}"),
            8,
        );
    }
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

fn persist_production_adapter_run(
    conn: &rusqlite::Connection,
    run: &ProductionAdapterRunStats,
    last_indexed_at: &str,
) -> Result<String, String> {
    queries::insert_session_adapter_run(
        conn,
        &queries::SessionAdapterRunInput {
            project: None,
            adapter_id: run.adapter_id.clone(),
            agent_type: Some(run.agent_type.clone()),
            source_roots: run.source_roots.clone(),
            sample_source_paths: run.sample_source_paths.clone(),
            evidence_archive: "sqlite:cc_sessions".to_string(),
            sessions_indexed: run.sessions_indexed,
            messages_indexed: run.messages_indexed,
            last_indexed_at: Some(last_indexed_at.to_string()),
            sample_session_ids: run.sample_session_ids.clone(),
            parse_warnings: run.parse_warnings.clone(),
            supports_incremental: run.supports_incremental,
        },
    )
    .map(|row| row.id)
    .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Public Tauri commands
// ─────────────────────────────────────────────────────────────────

/// Manually trigger a re-index of all Claude Code session files.
///
/// Walks `~/.claude/projects/` looking for JSONL session files, parses each
/// one with the real Claude Code JSONL format, and upserts project / session /
/// message rows into the database.
///
/// Supports **incremental indexing**: files whose mtime has not changed since
/// the last index are skipped entirely.  Files that have grown (append-only)
/// are read starting from the previously stored byte offset so that only new
/// lines are parsed.
/// Run the full index directly with a connection reference.
/// Used by the startup background thread.
pub fn run_full_index_summary_with_conn(
    conn: &rusqlite::Connection,
) -> Result<FullIndexSummary, String> {
    let _index_guard = FULL_INDEX_LOCK
        .lock()
        .map_err(|e| format!("full index lock poisoned: {e}"))?;
    run_full_index_unlocked(conn)
}

/// Run the full index only if no other full index is active.
///
/// Background schedulers use this so they never queue a low-priority full scan
/// behind another index pass while foreground commands are trying to use SQLite.
pub fn try_run_full_index_summary_with_conn(
    conn: &rusqlite::Connection,
) -> Result<Option<FullIndexSummary>, String> {
    let _index_guard = match FULL_INDEX_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(None),
    };
    run_full_index_unlocked(conn).map(Some)
}

fn run_full_index_unlocked(conn: &rusqlite::Connection) -> Result<FullIndexSummary, String> {
    let (indexed_sessions, indexed_messages, skipped_sessions) = full_index_impl(conn)?;
    let archive_search_rows_indexed =
        queries::sync_session_message_archive_fts(conn).map_err(|e| e.to_string())?;

    // Store the last indexed timestamp
    let now = chrono::Utc::now().to_rfc3339();
    let _ = queries::set_preference(conn, "last_indexed_at", &now);

    // v1.1.84: truncate the WAL after every index pass. Without this, writes
    // from the 5-minute re-indexer accumulate over hours/days into a multi-
    // hundred-megabyte `codevetter.db-wal`, bloating both disk and the mmap.
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");

    Ok(FullIndexSummary {
        indexed_sessions,
        indexed_messages,
        skipped_sessions,
        archive_search_rows_indexed,
        indexed_at: now,
    })
}

pub fn emit_session_archive_updated(app: &AppHandle, summary: &FullIndexSummary) {
    if let Err(error) = app.emit("session_archive_updated", summary.clone()) {
        log::warn!("Failed to emit session_archive_updated: {error}");
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptTailSummary {
    pub sessions_tailed: u64,
    pub messages_indexed: u64,
    pub tailed_at: String,
}

/// Incrementally re-index recently active transcript files between full index passes.
pub fn tail_live_transcript_sessions_with_conn(
    conn: &rusqlite::Connection,
) -> Result<TranscriptTailSummary, String> {
    tail_live_transcript_sessions_inner(conn, true)
}

fn tail_live_transcript_sessions_inner(
    conn: &rusqlite::Connection,
    discover_new_codex_sessions: bool,
) -> Result<TranscriptTailSummary, String> {
    let _index_guard = match FULL_INDEX_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Ok(TranscriptTailSummary {
                sessions_tailed: 0,
                messages_indexed: 0,
                tailed_at: chrono::Utc::now().to_rfc3339(),
            });
        }
    };
    let since = (chrono::Utc::now() - chrono::Duration::minutes(120)).to_rfc3339();
    let sources =
        queries::list_live_session_sources(conn, &since, 16).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let tick_started = std::time::Instant::now();
    let mut sessions_tailed = 0u64;
    let mut messages_indexed = 0u64;
    let mut seen_paths = HashSet::new();

    for source in sources {
        if tick_started.elapsed()
            >= std::time::Duration::from_millis(LIVE_TRANSCRIPT_TICK_BUDGET_MS)
        {
            break;
        }
        let path = std::path::Path::new(&source.jsonl_path);
        seen_paths.insert(source.jsonl_path.clone());
        if !path.exists() {
            continue;
        }
        let result = match source.agent_type.as_str() {
            "claude-code" => index_adapter_session_bounded(
                &ClaudeCodeAdapter,
                path,
                conn,
                &source.project_id,
                &now,
                LIVE_TRANSCRIPT_SESSION_BYTE_BUDGET,
            ),
            "codex" => index_adapter_session_bounded(
                &CodexAdapter,
                path,
                conn,
                &source.project_id,
                &now,
                LIVE_TRANSCRIPT_SESSION_BYTE_BUDGET,
            ),
            _ => continue,
        };
        match result {
            Ok(indexed) => {
                if indexed.messages_indexed > 0 {
                    sessions_tailed += 1;
                    messages_indexed += indexed.messages_indexed;
                }
            }
            Err(error) => {
                log::debug!("Transcript tail skipped {}: {error}", source.jsonl_path);
            }
        }
    }

    // New Codex sessions are not present in `cc_sessions` yet, so the DB-backed
    // live-source query above cannot see them until a full index discovers them.
    // Scan recently touched Codex roots directly and index any fresh file now.
    let recent_codex_files = if discover_new_codex_sessions
        && tick_started.elapsed() < std::time::Duration::from_millis(LIVE_TRANSCRIPT_TICK_BUDGET_MS)
    {
        recent_codex_session_files(chrono::Duration::hours(48), 80)
    } else {
        Vec::new()
    };
    let mut discovery_sessions_indexed = 0usize;
    for path in recent_codex_files {
        if tick_started.elapsed()
            >= std::time::Duration::from_millis(LIVE_TRANSCRIPT_TICK_BUDGET_MS)
        {
            break;
        }
        let path_str = path.to_string_lossy().to_string();
        if seen_paths.contains(&path_str) {
            continue;
        }
        let file_size = std::fs::metadata(&path)
            .map(|m| m.len() as i64)
            .unwrap_or_default();
        if let Ok(Some(existing)) = queries::get_session_by_jsonl_path(conn, &path_str) {
            if session_fully_indexed(&existing, file_size) {
                continue;
            }
        }
        let project_id = match ensure_codex_project_for_jsonl(conn, &path, &now) {
            Ok(project_id) => project_id,
            Err(error) => {
                log::debug!("Codex live discovery skipped {path_str}: {error}");
                continue;
            }
        };
        match index_adapter_session_bounded(
            &CodexAdapter,
            &path,
            conn,
            &project_id,
            &now,
            LIVE_TRANSCRIPT_SESSION_BYTE_BUDGET,
        ) {
            Ok(indexed) => {
                discovery_sessions_indexed += 1;
                if indexed.messages_indexed > 0 {
                    sessions_tailed += 1;
                    messages_indexed += indexed.messages_indexed;
                }
            }
            Err(error) => log::debug!("Codex live discovery failed {path_str}: {error}"),
        }
        if discovery_sessions_indexed >= LIVE_CODEX_DISCOVERY_SESSION_BUDGET {
            break;
        }
    }

    Ok(TranscriptTailSummary {
        sessions_tailed,
        messages_indexed,
        tailed_at: now,
    })
}

/// Refresh Grok + Cursor sessions outside the full index. They aren't
/// transcript-tailable via `list_live_session_sources` (Grok is a session
/// directory, Cursor is a SQLite DB), so without this they only refreshed on
/// the 5-minute full index and visibly lagged Claude/Codex (which tail every
/// 10s) — reading to the user as "Grok/Cursor not updating". Each indexer skips
/// unchanged sessions cheaply via mtime, so this is light to call on a short
/// sub-cadence. Runs every indexer independently so one failure doesn't block
/// the others.
pub fn refresh_secondary_agents_with_conn(
    conn: &rusqlite::Connection,
) -> Result<TranscriptTailSummary, String> {
    let _index_guard = match FULL_INDEX_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Ok(TranscriptTailSummary {
                sessions_tailed: 0,
                messages_indexed: 0,
                tailed_at: chrono::Utc::now().to_rfc3339(),
            });
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let mut sessions_tailed = 0u64;
    let mut messages_indexed = 0u64;

    for result in [
        index_grok_sessions(conn),
        index_cursor_sessions(conn),
        index_cursor_agent_sessions(conn),
        index_devin_sessions(conn),
    ] {
        match result {
            Ok((indexed, messages, _skipped)) => {
                sessions_tailed += indexed;
                messages_indexed += messages;
            }
            Err(error) => log::debug!("Secondary-agent refresh skipped one source: {error}"),
        }
    }

    Ok(TranscriptTailSummary {
        sessions_tailed,
        messages_indexed,
        tailed_at: now,
    })
}

#[tauri::command]
pub async fn trigger_index(app: AppHandle) -> Result<Value, String> {
    // Index against a private WAL connection on a blocking thread — exactly the
    // pattern the periodic/startup indexer already uses (main.rs:run_full_index).
    // This keeps the full (cold) index off both the async runtime worker AND the
    // shared DbState connection lock, so other DB-backed commands stay responsive
    // during a manual re-index. FULL_INDEX_LOCK (held inside
    // run_full_index_summary_with_conn) still serializes against the periodic run.
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    let summary = tokio::task::spawn_blocking(move || {
        let conn = crate::db::init_db(app_data_dir).map_err(|e| e.to_string())?;
        run_full_index_summary_with_conn(&conn)
    })
    .await
    .map_err(|e| format!("index task join error: {e}"))??;
    emit_session_archive_updated(&app, &summary);

    Ok(json!({
        "indexed_sessions": summary.indexed_sessions,
        "indexed_messages": summary.indexed_messages,
        "skipped_sessions": summary.skipped_sessions,
        "archive_search_rows_indexed": summary.archive_search_rows_indexed,
        "projects_scanned": 0,
    }))
}

/// Shared implementation for the full indexer.
/// Whether the indexer can skip a session file without re-parsing it: it has
/// messages and the byte cursor has consumed *exactly* the file's current size,
/// so nothing has been appended.
///
/// This keys on byte offset, NOT the file mtime. The old skip compared stored
/// vs freshly-recomputed mtime strings, but their sub-microsecond nanoseconds
/// drift between reads of the same unchanged inode — so the skip silently failed
/// and hundreds of large sessions were fully re-parsed (and their archive rows
/// DELETE+re-INSERTed) on every 5-minute pass, pegging one core. Byte offset is
/// exact: equal ⇒ nothing new; smaller ⇒ file grew (parse the tail); larger ⇒
/// file shrank/rotated (full re-parse).
fn session_fully_indexed(meta: &queries::SessionMeta, file_size: i64) -> bool {
    meta.message_count > 0
        && meta.last_indexed_byte_offset > 0
        && meta.last_indexed_byte_offset == file_size
}

fn full_index_impl(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let all_bases = resolve_all_claude_projects_dirs();
    let index_started_at = chrono::Utc::now().to_rfc3339();
    let mut claude_run = ProductionAdapterRunStats::new(
        "claude-code",
        "claude-code",
        all_bases
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        true,
    );

    let mut indexed_sessions = 0u64;
    let mut indexed_messages = 0u64;
    let mut skipped_sessions = 0u64;

    // Collect project directories from all Claude profile directories.
    let project_dirs: Vec<_> = all_bases
        .iter()
        .filter(|b| b.exists())
        .flat_map(|b| std::fs::read_dir(b).ok().into_iter())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect();

    for project_entry in &project_dirs {
        let project_path = project_entry.path();
        // Per-project IIFE so a failure in one project (bad JSONL row, locked
        // file, etc.) only skips that project instead of aborting the whole
        // re-index. Previously a single `?` here meant new project dirs added
        // late in the iteration order (e.g. after a repo move) were never
        // scanned, freezing token-usage stats.
        let project_result: Result<(), String> = (|| {
            let project_dir_name = project_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let display_name = resolve_project_display_name(&project_dir_name);
            let dir_path_str = project_path.to_string_lossy().to_string();

            // Re-use existing project ID if the dir_path already exists, otherwise
            // create a new one.  This avoids generating a fresh UUID on every
            // re-index which would orphan sessions.
            let project_id = queries::get_project_id_by_dir(conn, &dir_path_str)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            let now = chrono::Utc::now().to_rfc3339();

            queries::upsert_project(
                conn,
                &queries::ProjectInput {
                    id: project_id.clone(),
                    display_name: display_name.clone(),
                    dir_path: dir_path_str,
                    session_count: None,
                    last_activity: Some(now.clone()),
                    created_at: now.clone(),
                },
            )
            .map_err(|e| e.to_string())?;

            // Look for JSONL files inside the project directory (recursively).
            let jsonl_files: Vec<_> = walkdir(&project_path, "jsonl");

            for jsonl_path in &jsonl_files {
                let jsonl_path_str = jsonl_path.to_string_lossy().to_string();

                // ── Incremental check ────────────────────────────────
                let file_meta = std::fs::metadata(jsonl_path).ok();
                let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);

                let existing = queries::get_session_by_jsonl_path(conn, &jsonl_path_str)
                    .map_err(|e| e.to_string())?;

                // Skip when the indexer has already consumed the whole file: the
                // cursor reached EOF (offset == current size) and the file has
                // not grown. Byte offset is an EXACT signal — the old mtime-string
                // check silently failed because stored vs recomputed nanoseconds
                // drift, re-parsing 100s of MB every pass and pegging the CPU.
                if let Some(ref meta) = existing {
                    if session_fully_indexed(meta, file_size) {
                        skipped_sessions += 1;
                        continue;
                    }
                }

                match parse_claude_session(jsonl_path, conn, &project_id, &now) {
                    Ok(session) => {
                        indexed_sessions += 1;
                        indexed_messages += session.messages_indexed;
                        claude_run.record_session(&session);
                    }
                    Err(error) => {
                        claude_run.record_warning(&jsonl_path_str, &error);
                        continue;
                    }
                }
            }

            // Update project session count.
            let session_count = jsonl_files.len() as i64;
            conn.execute(
                "UPDATE cc_projects SET session_count = ?2 WHERE id = ?1",
                rusqlite::params![project_id, session_count],
            )
            .map_err(|e: rusqlite::Error| e.to_string())?;

            // Update display name from session cwd if available (more reliable
            // than decoding the encoded directory name).
            let cwd_name: Option<String> = conn
            .query_row(
                "SELECT cwd FROM cc_sessions WHERE project_id = ?1 AND cwd IS NOT NULL AND cwd != '' LIMIT 1",
                rusqlite::params![project_id],
                |row| row.get::<_, String>(0),
            )
            .ok();

            if let Some(ref cwd) = cwd_name {
                let better_name = std::path::Path::new(cwd)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| display_name.clone());
                let _ = conn.execute(
                    "UPDATE cc_projects SET display_name = ?2 WHERE id = ?1",
                    rusqlite::params![project_id, better_name],
                );
            }

            Ok(())
        })();
        if let Err(e) = project_result {
            claude_run.record_warning(&project_path.to_string_lossy(), &e);
            log::error!("Skipping project {project_path:?}: {e}");
        }
    }
    persist_production_adapter_run(conn, &claude_run, &index_started_at)?;

    // ── Phase 2: Scan Codex sessions ─────────────────────────
    let codex_roots = resolve_codex_session_roots();
    let mut codex_run = ProductionAdapterRunStats::new(
        "codex",
        "codex",
        codex_roots
            .iter()
            .map(|root| root.to_string_lossy().to_string())
            .collect(),
        true,
    );
    let mut codex_indexed = 0u64;
    let mut codex_messages = 0u64;

    for codex_base in codex_roots.iter().filter(|root| root.exists()) {
        let codex_files: Vec<_> = walkdir(codex_base, "jsonl");

        for jsonl_path in &codex_files {
            let jsonl_path_str = jsonl_path.to_string_lossy().to_string();

            // ── Incremental check ────────────────────────────
            // Skip fully-consumed unchanged files via exact byte offset (see the
            // Claude phase above for why mtime strings are unreliable).
            let file_meta = std::fs::metadata(jsonl_path).ok();
            let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);

            let existing = queries::get_session_by_jsonl_path(conn, &jsonl_path_str)
                .map_err(|e| e.to_string())?;

            if let Some(ref meta) = existing {
                if session_fully_indexed(meta, file_size) {
                    skipped_sessions += 1;
                    continue;
                }
            }

            // Read the first line to get session_meta and determine the project
            let first_line = match std::fs::File::open(jsonl_path) {
                Ok(f) => {
                    let mut rdr = std::io::BufReader::new(f);
                    let mut buf = String::new();
                    let _ = rdr.read_line(&mut buf);
                    buf
                }
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error.to_string());
                    continue;
                }
            };

            let meta_parsed: Value = match serde_json::from_str(first_line.trim()) {
                Ok(v) => v,
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error.to_string());
                    continue;
                }
            };

            let meta_type = meta_parsed
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if meta_type != "session_meta" {
                codex_run.record_warning(&jsonl_path_str, "first JSONL row is not session_meta");
                continue;
            }

            let payload = match meta_parsed.get("payload") {
                Some(p) => p,
                None => {
                    codex_run
                        .record_warning(&jsonl_path_str, "session_meta row is missing payload");
                    continue;
                }
            };

            let codex_cwd = payload
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if codex_cwd.is_empty() {
                codex_run.record_warning(&jsonl_path_str, "session_meta row is missing cwd");
                continue;
            }

            let now = chrono::Utc::now().to_rfc3339();

            // Resolve or create the project for this Codex session's cwd
            let project_id = queries::get_project_id_by_dir(conn, &codex_cwd)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| {
                    let pid = uuid::Uuid::new_v4().to_string();
                    let display = std::path::Path::new(&codex_cwd)
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| codex_cwd.clone());
                    let _ = queries::upsert_project(
                        conn,
                        &queries::ProjectInput {
                            id: pid.clone(),
                            display_name: display,
                            dir_path: codex_cwd.clone(),
                            session_count: None,
                            last_activity: Some(now.clone()),
                            created_at: now.clone(),
                        },
                    );
                    pid
                });

            match parse_codex_session(jsonl_path, conn, &project_id, &now) {
                Ok(session) => {
                    codex_indexed += 1;
                    codex_messages += session.messages_indexed;
                    codex_run.record_session(&session);
                }
                Err(error) => {
                    codex_run.record_warning(&jsonl_path_str, &error);
                    continue;
                }
            }
        }
    }
    persist_production_adapter_run(conn, &codex_run, &index_started_at)?;

    indexed_sessions += codex_indexed;
    indexed_messages += codex_messages;

    // ── Phase 3: Scan Cursor AI sessions ─────────────────────
    match index_cursor_sessions(conn) {
        Ok((cursor_indexed, cursor_messages, cursor_skipped)) => {
            indexed_sessions += cursor_indexed;
            indexed_messages += cursor_messages;
            skipped_sessions += cursor_skipped;
        }
        Err(error) => {
            log::warn!("Cursor session index failed; continuing with archive backfill: {error}");
        }
    }

    // ── Phase 4: Scan Grok CLI sessions (~/.grok/sessions) ───
    match index_grok_sessions(conn) {
        Ok((grok_indexed, grok_messages, grok_skipped)) => {
            indexed_sessions += grok_indexed;
            indexed_messages += grok_messages;
            skipped_sessions += grok_skipped;
        }
        Err(error) => {
            log::warn!("Grok session index failed; continuing with archive backfill: {error}");
        }
    }

    // ── Phase 5: Scan Cursor Agent CLI sessions (~/.cursor/chats) ───
    match index_cursor_agent_sessions(conn) {
        Ok((ca_indexed, ca_messages, ca_skipped)) => {
            indexed_sessions += ca_indexed;
            indexed_messages += ca_messages;
            skipped_sessions += ca_skipped;
        }
        Err(error) => {
            log::warn!("Cursor Agent session index failed; continuing: {error}");
        }
    }

    // ── Phase 6: Scan Devin CLI sessions (~/.local/share/devin/cli/sessions.db)
    match index_devin_sessions(conn) {
        Ok((devin_indexed, devin_messages, devin_skipped)) => {
            indexed_sessions += devin_indexed;
            indexed_messages += devin_messages;
            skipped_sessions += devin_skipped;
        }
        Err(error) => {
            log::warn!("Devin session index failed; continuing: {error}");
        }
    }

    let backfilled_archives = backfill_missing_session_archives(conn)?;
    if backfilled_archives > 0 {
        log::info!("Backfilled normalized session archive for {backfilled_archives} sessions");
    }

    Ok((indexed_sessions, indexed_messages, skipped_sessions))
}

/// Token usage stats: today / week / month / year totals + 30-day daily series
/// + 12-week weekly series. Windows use the user's local timezone.
#[tauri::command]
pub async fn get_token_usage_stats(
    db: State<'_, DbState>,
) -> Result<queries::TokenUsageStats, String> {
    let conn = conn_lock(&db)?;
    queries::get_token_usage_stats(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_usage_breakdown(
    db: State<'_, DbState>,
) -> Result<Vec<queries::AgentUsageRow>, String> {
    let conn = conn_lock(&db)?;
    queries::get_agent_usage_breakdown(&conn).map_err(|e| e.to_string())
}

/// Per-day, per-agent generated/cache tokens for the last `days` days (day-wise
/// drill-down behind the daily chart). Defaults to 30 days when omitted.
#[tauri::command]
pub async fn get_agent_usage_by_day(
    db: State<'_, DbState>,
    days: Option<i64>,
) -> Result<Vec<queries::AgentDayUsage>, String> {
    let conn = conn_lock(&db)?;
    queries::get_agent_usage_by_day(&conn, days.unwrap_or(30)).map_err(|e| e.to_string())
}

/// Generated/cache tokens grouped by model (per-message attribution where a
/// session_model_usage breakdown exists; costs priced live from the current
/// table so the split never inherits stale per-session costs). `days` limits
/// to a rolling window ending today (session activity prorated per day, same
/// attribution as the daily chart); omitted = all time. `day_start` +
/// `day_end` (exclusive) slice a single day or week for chart/rhythm drill-down.
#[tauri::command]
pub async fn get_usage_by_model(
    db: State<'_, DbState>,
    days: Option<i64>,
    exclude_agents: Option<Vec<String>>,
    day_start: Option<String>,
    day_end: Option<String>,
) -> Result<Vec<queries::ModelUsage>, String> {
    use chrono::{Duration, Local};
    let day_range = match (day_start, day_end) {
        (Some(s), Some(e)) if !s.trim().is_empty() && !e.trim().is_empty() => (Some(s), Some(e)),
        _ => (None, None),
    };
    let since = if day_range.0.is_some() {
        None
    } else {
        days.map(|d| {
            (Local::now().date_naive() - Duration::days(d.max(1) - 1))
                .format("%Y-%m-%d")
                .to_string()
        })
    };
    let conn = conn_lock(&db)?;
    let exclude = exclude_agents.unwrap_or_default();
    queries::get_usage_by_model(
        &conn,
        estimate_cost,
        since.as_deref(),
        day_range.0.as_deref(),
        day_range.1.as_deref(),
        &exclude,
    )
    .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Content text extraction
// ─────────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn extract_content_text(parsed: &Value) -> Option<String> {
    let message = parsed.get("message")?;
    let content = message.get("content")?;
    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");

    // String content (common for user messages).
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    // Array content.
    if let Some(blocks) = content.as_array() {
        match role {
            "assistant" => {
                // Concatenate text blocks, skip thinking blocks.
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        if block_type == "text" {
                            block.get("text")?.as_str()
                        } else {
                            None
                        }
                    })
                    .collect();

                if texts.is_empty() {
                    // If no text blocks, check for tool_use blocks and
                    // produce a summary so the message is not blank.
                    let tool_names: Vec<&str> = blocks
                        .iter()
                        .filter_map(|block| {
                            let bt = block.get("type")?.as_str()?;
                            if bt == "tool_use" {
                                block.get("name")?.as_str()
                            } else {
                                None
                            }
                        })
                        .collect();
                    if tool_names.is_empty() {
                        None
                    } else {
                        Some(format!("[tool_use: {}]", tool_names.join(", ")))
                    }
                } else {
                    Some(texts.join("\n\n"))
                }
            }
            "user" => {
                // User array content is typically tool_result blocks.
                let summaries: Vec<String> = blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        if block_type == "tool_result" {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            Some(format!("[tool_result for {tool_use_id}]"))
                        } else if block_type == "text" {
                            block.get("text").and_then(|v| v.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect();

                if summaries.is_empty() {
                    None
                } else {
                    Some(summaries.join("\n"))
                }
            }
            _ => {
                // Unknown role with array content -- store raw JSON.
                Some(content.to_string())
            }
        }
    } else {
        // Content is some other JSON value (object, number, etc.).
        Some(content.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────
// Project name resolution
// ─────────────────────────────────────────────────────────────────

/// Convert a Claude Code project directory name like
/// `-Users-sarthakagrawal-Desktop-code-reviewer` into a human-friendly
/// display name.
///
/// Strategy: use the known home directory to strip the encoded prefix, then
/// greedily match intermediate path segments by checking which sub-segments
/// correspond to real directories on disk.  Everything after the last matched
/// directory is the project name (preserving real hyphens).
///
/// Example: `-Users-sarthakagrawal-Desktop-code-reviewer`
///   home = `/Users/sarthakagrawal`  →  encoded = `Users-sarthakagrawal`
///   remainder = `Desktop-code-reviewer`
///   `~/Desktop` is a dir → consume  →  project name = `code-reviewer`
fn resolve_project_display_name(dir_name: &str) -> String {
    let trimmed = dir_name.trim_start_matches('-');
    if trimmed.is_empty() {
        return dir_name.to_string();
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    if !home.is_empty() {
        // Encode the home path the same way Claude Code encodes directory names
        let home_encoded = home.trim_start_matches('/').replace('/', "-");

        if let Some(remainder) = trimmed.strip_prefix(&home_encoded) {
            let remainder = remainder.trim_start_matches('-');
            if remainder.is_empty() {
                return dir_name.to_string();
            }

            // Greedily match intermediate path segments from the home dir.
            // e.g., remainder = "Desktop-code-reviewer"
            // Check: is ~/Desktop a dir? Yes → consume.  Is ~/Desktop/code a
            // dir? No → "code-reviewer" is the project name.
            let parts: Vec<&str> = remainder.split('-').collect();
            let mut current_dir = std::path::PathBuf::from(&home);
            let mut consumed = 0usize;

            for start in 0..parts.len() {
                let candidate = parts[start];
                let test_path = current_dir.join(candidate);
                // Only consume this segment as a directory if there are more
                // segments after it (the last segment must be part of the
                // project name).
                if test_path.is_dir() && start + 1 < parts.len() {
                    current_dir = test_path;
                    consumed = start + 1;
                } else {
                    break;
                }
            }

            let project_name = parts[consumed..].join("-");
            if !project_name.is_empty() {
                return project_name;
            }
        }
    }

    // Fallback: replace `-` with `/` and take last component
    let reconstructed = trimmed.replace('-', "/");
    std::path::Path::new(&reconstructed)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| dir_name.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Cost estimation
// ─────────────────────────────────────────────────────────────────

fn estimate_cost(
    model: &str,
    total_input: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_creation: i64,
) -> f64 {
    // Per-million-token pricing (input, output, cache-read, cache-write), USD.
    // Cache-read ≈ 0.1× input, cache-write (5-min) ≈ 1.25× input. Bump
    // PRICING_REV in recompute_all_session_costs whenever these change so the
    // stored estimated_cost_usd is refreshed for already-indexed sessions.
    let (input_price, output_price, cache_read_price, cache_write_price) = match model {
        // Claude Code's internal non-API marker — no tokens are billed.
        m if m.contains("synthetic") => (0.0, 0.0, 0.0, 0.0),
        // Claude Fable 5 / Mythos 5 are $10/$50 (above Opus tier); cache-read
        // 0.1× input, cache-write 1.25× input.
        m if m.contains("fable") || m.contains("mythos") => (10.0, 50.0, 1.0, 12.5),
        // Opus 4.1 / 4.0 (claude-opus-4-2025…) / Claude 3 Opus were $15/$75;
        // Opus 4.5+ dropped to $5/$25.
        m if m.contains("opus-4-1") || m.contains("opus-4-2025") || m.contains("3-opus") => {
            (15.0, 75.0, 1.5, 18.75)
        }
        m if m.contains("opus") => (5.0, 25.0, 0.50, 6.25),
        m if m.contains("sonnet") => (3.0, 15.0, 0.30, 3.75),
        // Haiku 4.5 is $1/$5 (older Haiku 3.5 was $0.25/$1.25).
        m if m.contains("haiku") => (1.0, 5.0, 0.10, 1.25),
        // OpenAI GPT-5.4 mini (standard API): $0.75/$4.50, cached $0.075.
        // Match before base GPT-5.4 so the mini suffix never bills as full.
        m if m.contains("gpt-5.4") && m.contains("mini") => (0.75, 4.5, 0.075, 0.0),
        // GPT-5 mini class before 5.4: $0.25/$2, cached $0.025.
        m if m.contains("gpt-5") && m.contains("mini") => (0.25, 2.0, 0.025, 0.0),
        // GPT-5.5 (Codex CLI default, mid-2026): $5/$30, cached input $0.50.
        m if m.contains("gpt-5.5") => (5.0, 30.0, 0.50, 0.0),
        // GPT-5.6 family (Jul 2026): Sol flagship $5/$30, Terra $2.50/$15,
        // Luna $1/$6; cached input 90% off, cache writes 1.25× input.
        m if m.contains("gpt-5.6") && m.contains("terra") => (2.5, 15.0, 0.25, 3.125),
        m if m.contains("gpt-5.6") && m.contains("luna") => (1.0, 6.0, 0.10, 1.25),
        m if m.contains("gpt-5.6") => (5.0, 30.0, 0.50, 6.25),
        // OpenAI GPT-5.4 standard API: $2.50/$15, cached $0.25.
        m if m.contains("gpt-5.4") => (2.5, 15.0, 0.25, 0.0),
        // Codex specialized model, if logs expose the exact model id.
        m if m.contains("gpt-5.3-codex") => (1.75, 14.0, 0.175, 0.0),
        // GPT-5 family fallback (gpt-5, gpt-5.1, …): $1.25/$10.
        m if m.contains("gpt-5") => (1.25, 10.0, 0.125, 1.25),
        m if m.contains("gpt-4o") => (2.5, 10.0, 1.25, 2.5),
        m if m.contains("gpt-4.1") => (2.0, 8.0, 0.5, 2.0),
        // OpenAI o3 repriced to $2/$8 (cached input $0.50).
        m if m.contains("o3") || m.contains("o4-mini") => (2.0, 8.0, 0.50, 2.0),
        // Grok Build code API (current Grok CLI default): $1/$2, cached $0.20.
        // Token counts are local estimates from session logs, not exact xAI
        // billing rows.
        m if m.contains("grok-build") => (1.0, 2.0, 0.20, 1.0),
        // Legacy Grok code/composer fast model IDs.
        m if m.contains("grok-code") || m.contains("grok-composer") => (0.2, 1.5, 0.02, 0.2),
        // Cursor Composer (non-Grok) — local token estimates; fast-tier pricing.
        m if m.contains("composer") => (0.2, 1.5, 0.02, 0.2),
        // Current xAI chat/API models.
        m if m.contains("grok-4.5") => (2.0, 6.0, 0.50, 2.0),
        m if m.contains("grok-4.3") || m.contains("grok-4.20") => (1.25, 2.5, 0.20, 1.25),
        m if m.contains("grok") => (2.0, 6.0, 0.50, 2.0),
        // GLM-5.2 (Z.ai): $1.40/$4.40, cached $0.26 (verified Jun 2026). Cache
        // creation storage is limited-time free → 0. Devin's internal models
        // (compactor, swe-*, MODEL_PRIVATE_*) are assumed GLM-based.
        m if m.contains("glm")
            || m.contains("compactor")
            || m.contains("swe")
            || m.contains("MODEL_PRIVATE") =>
        {
            (1.4, 4.4, 0.26, 0.0)
        }
        _ => (3.0, 15.0, 0.30, 3.75), // default ≈ sonnet pricing
    };

    // total_input already includes cache_read + cache_creation tokens (added
    // during indexing), so subtract them to get the base input token count
    // that is billed at the full input rate.
    let base_input = (total_input - cache_read - cache_creation).max(0);

    let cost = (base_input as f64 * input_price
        + output_tokens as f64 * output_price
        + cache_read as f64 * cache_read_price
        + cache_creation as f64 * cache_write_price)
        / 1_000_000.0;
    (cost * 100.0).round() / 100.0 // round to cents
}

/// Bump this whenever the `estimate_cost` price table changes so already-indexed
/// sessions get their stored `estimated_cost_usd` refreshed (otherwise mtime-skip
/// keeps the old cost). Rev 2 = Opus $5/$25 + Haiku 4.5 $1/$5 + o3 $2/$8.
/// Rev 3 = GLM-5.2 $1.40/$4.40 + Devin-internal models.
/// Rev 4 = Fable/Mythos 5 $10/$50 (was falling to the sonnet default) + session
/// costs now sum per-model rows when a `session_model_usage` breakdown exists.
/// Rev 5 = `<synthetic>` prices to $0 (was sonnet default), Opus 4.1/4.0/3
/// restored to $15/$75, Grok CLI fast models (grok-code/build/composer) at
/// grok-code-fast pricing instead of grok-4.
/// Rev 6 = GPT-5.5 $5/$30 (cached $0.50) + GPT-5 family $1.25/$10 — paired
/// with the codex model backfill that relabels o3-defaulted sessions to the
/// real model recorded on their turn_context rows.
/// Rev 7 = GPT-5 mini class ($0.25/$2) split out of the family fallback.
/// Rev 8 = current xAI Grok pricing: grok-build $1/$2 cached $0.20,
/// grok-4.5 $2/$6 cached $0.50, grok-4.3/4.20 $1.25/$2.50 cached $0.20.
/// Rev 9 = official OpenAI GPT-5.4 / GPT-5.4-mini prices and gpt-5.3-codex.
/// Rev 10 = GPT-5.6 tiers (Sol $5/$30, Terra $2.50/$15, Luna $1/$6, cached 90%
/// off) — 5.6-sol previously fell through to the GPT-5 family fallback and
/// booked at ~1/4 of its real price.
const PRICING_REV: &str = "10";

/// Estimated cost for one session: per-model when a breakdown exists (correct
/// for multi-model Claude sessions), else session-level `model_used` pricing.
fn estimate_session_cost(
    conn: &rusqlite::Connection,
    session_id: &str,
    model_used: &str,
    total_input: i64,
    output_tokens: i64,
    cache_read: i64,
    cache_creation: i64,
) -> f64 {
    if let Ok(rows) = queries::get_session_model_usage(conn, session_id) {
        if !rows.is_empty() {
            let cost: f64 = rows
                .iter()
                .map(|u| {
                    estimate_cost(
                        &u.model,
                        u.input_tokens,
                        u.output_tokens,
                        u.cache_read_tokens,
                        u.cache_creation_tokens,
                    )
                })
                .sum();
            return (cost * 100.0).round() / 100.0;
        }
    }
    estimate_cost(
        model_used,
        total_input,
        output_tokens,
        cache_read,
        cache_creation,
    )
}

/// Recompute `estimated_cost_usd` for every session from its stored token counts
/// and model, using the current price table — a pure DB pass, no file re-read.
/// Runs once per `PRICING_REV` (gated by a preference) so a price change is
/// reflected immediately without forcing a full re-index.
pub fn recompute_all_session_costs(conn: &rusqlite::Connection) {
    if let Ok(Some(rev)) = queries::get_preference(conn, "pricing_rev") {
        if rev == PRICING_REV {
            return;
        }
    }
    let mut stmt = match conn.prepare(
        "SELECT id, model_used, total_input_tokens, total_output_tokens,
                cache_read_tokens, cache_creation_tokens
         FROM cc_sessions",
    ) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("cost recompute prepare failed: {e}");
            return;
        }
    };
    let mapped = match stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    }) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("cost recompute query failed: {e}");
            return;
        }
    };
    let rows: Vec<(String, Option<String>, i64, i64, i64, i64)> =
        mapped.filter_map(Result::ok).collect();
    drop(stmt);
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(_) => return,
    };
    for (id, model, total_input, output, cache_read, cache_creation) in rows {
        let cost = estimate_session_cost(
            conn,
            &id,
            model.as_deref().unwrap_or(""),
            total_input,
            output,
            cache_read,
            cache_creation,
        );
        let _ = tx.execute(
            "UPDATE cc_sessions SET estimated_cost_usd = ?2 WHERE id = ?1",
            rusqlite::params![id, cost],
        );
    }
    if tx.commit().is_ok() {
        let _ = queries::set_preference(conn, "pricing_rev", PRICING_REV);
        log::info!("Recomputed session costs for pricing rev {PRICING_REV}");
    }
}

/// Bump to re-run `backfill_session_model_usage` (gated by a preference).
const MODEL_USAGE_BACKFILL_REV: &str = "1";

/// One-time backfill of `session_model_usage` for already-indexed Claude
/// sessions (v1.1.100). Session-level `model_used` is last-model-wins, so a
/// session that switched models mid-way (e.g. opus→fable) booked ALL its
/// tokens/cost to the final model — the by-model panel was misattributed for
/// every multi-model session. Streams each Claude JSONL once, extracting only
/// per-message model + usage (no archive rows), then replaces that session's
/// breakdown rows and refreshes its stored cost. Runs in the background
/// storage-cleanup thread; sessions whose transcript file no longer exists
/// keep the session-level fallback attribution.
pub fn backfill_session_model_usage(conn: &rusqlite::Connection) {
    if let Ok(Some(rev)) = queries::get_preference(conn, "model_usage_backfill_rev") {
        if rev == MODEL_USAGE_BACKFILL_REV {
            return;
        }
    }
    let sessions: Vec<(String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, jsonl_path FROM cc_sessions
             WHERE agent_type = 'claude-code' AND jsonl_path IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("model-usage backfill prepare failed: {e}");
                return;
            }
        };
        let mapped =
            match stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                Ok(mapped) => mapped.filter_map(Result::ok).collect(),
                Err(e) => {
                    log::warn!("model-usage backfill query failed: {e}");
                    return;
                }
            };
        mapped
    };
    let total = sessions.len();
    let mut filled = 0usize;
    for (id, path) in sessions {
        let map = match scan_claude_model_usage(std::path::Path::new(&path)) {
            Ok(map) => map,
            Err(_) => continue, // file gone/unreadable → keep session-level fallback
        };
        if map.is_empty() {
            continue;
        }
        let deltas = model_usage_deltas(&map);
        if queries::replace_session_model_usage(conn, &id, &deltas).is_err() {
            continue;
        }
        // Refresh the stored cost now that the split exists, so ordering with
        // recompute_all_session_costs doesn't matter.
        if let Ok((ti, to, cr, cc, model)) = queries::get_session_token_totals(conn, &id) {
            let cost =
                estimate_session_cost(conn, &id, model.as_deref().unwrap_or(""), ti, to, cr, cc);
            let _ = queries::set_session_cost(conn, &id, cost);
        }
        filled += 1;
    }
    let _ = queries::set_preference(conn, "model_usage_backfill_rev", MODEL_USAGE_BACKFILL_REV);
    log::info!("Model-usage backfill filled {filled}/{total} Claude sessions (rev {MODEL_USAGE_BACKFILL_REV})");
}

/// Bump to re-run `backfill_codex_session_models` (gated by a preference).
/// Rev 2 = also refreshes each relabelled session's stored cost in place, so
/// the repricing no longer depends on recompute_all_session_costs running
/// afterwards (its pricing_rev gate may already be satisfied).
const CODEX_MODEL_BACKFILL_REV: &str = "2";

/// One-time repair of `model_used` for already-indexed Codex sessions. Newer
/// Codex CLIs stopped writing `model` on session_meta (it only carries
/// model_provider), so the adapter's o3-era fallback labelled every OpenAI
/// session "o3" even when the real model — recorded on per-turn
/// `turn_context` rows — was gpt-5.5. Streams each transcript, takes the last
/// turn_context model, and relabels the session. Sessions whose file has
/// rotated away keep the o3 fallback. Must run before
/// `recompute_all_session_costs` so the pricing pass books corrected models.
pub fn backfill_codex_session_models(conn: &rusqlite::Connection) {
    if let Ok(Some(rev)) = queries::get_preference(conn, "codex_model_backfill_rev") {
        if rev == CODEX_MODEL_BACKFILL_REV {
            return;
        }
    }
    let sessions: Vec<(String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, jsonl_path FROM cc_sessions
             WHERE agent_type = 'codex' AND jsonl_path IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("codex model backfill prepare failed: {e}");
                return;
            }
        };
        let rows =
            match stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                Ok(mapped) => mapped.filter_map(Result::ok).collect(),
                Err(e) => {
                    log::warn!("codex model backfill query failed: {e}");
                    return;
                }
            };
        rows
    };
    let total = sessions.len();
    let mut relabelled = 0usize;
    for (id, path) in sessions {
        let Some(model) = scan_codex_turn_context_model(std::path::Path::new(&path)) else {
            continue; // file gone or no turn_context rows → keep fallback label
        };
        if let Ok(n) = conn.execute(
            "UPDATE cc_sessions SET model_used = ?2
             WHERE id = ?1 AND COALESCE(model_used, '') <> ?2",
            rusqlite::params![id, model],
        ) {
            relabelled += usize::from(n > 0);
        }
        // Reprice in place regardless of whether the label just changed — an
        // interrupted earlier pass can leave sessions relabelled but still
        // priced under the old model, and recompute_all_session_costs won't
        // re-run once its pricing_rev gate is satisfied.
        if let Ok((ti, to, cr, cc, m)) = queries::get_session_token_totals(conn, &id) {
            let cost = estimate_session_cost(conn, &id, m.as_deref().unwrap_or(""), ti, to, cr, cc);
            let _ = queries::set_session_cost(conn, &id, cost);
        }
    }
    let _ = queries::set_preference(conn, "codex_model_backfill_rev", CODEX_MODEL_BACKFILL_REV);
    log::info!(
        "Codex model backfill relabelled {relabelled}/{total} sessions (rev {CODEX_MODEL_BACKFILL_REV})"
    );
}

/// Stream one Codex JSONL and return the model recorded on its last
/// `turn_context` row. Reads line by line (codex logs reach 100+ MB) with a
/// cheap substring pre-filter so non-matching lines are never JSON-parsed.
fn scan_codex_turn_context_model(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    let mut model = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if !line.contains("\"turn_context\"") {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if parsed.get("type").and_then(|v| v.as_str()) != Some("turn_context") {
            continue;
        }
        if let Some(m) = parsed
            .get("payload")
            .and_then(|p| p.get("model"))
            .and_then(|v| v.as_str())
        {
            model = Some(m.to_string());
        }
    }
    model
}

/// Stream one Claude JSONL and accumulate per-message model→usage. Reads line
/// by line (no whole-file allocation — transcripts reach 200+ MB) and parses
/// only the fields needed, mirroring the ClaudeCodeAdapter attribution rule:
/// tokens go to `message.model`, with `<synthetic>`/missing → "unknown".
fn scan_claude_model_usage(
    path: &std::path::Path,
) -> Result<
    std::collections::BTreeMap<String, crate::commands::session_adapters::ModelTokenUsage>,
    String,
> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    let mut map: std::collections::BTreeMap<
        String,
        crate::commands::session_adapters::ModelTokenUsage,
    > = std::collections::BTreeMap::new();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // torn tail / invalid UTF-8 → stop at last clean line
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let message = parsed.get("message");
        let usage = message.and_then(|m| m.get("usage"));
        let get = |key: &str| {
            usage
                .and_then(|u| u.get(key))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        };
        let input = get("input_tokens");
        let cache_creation = get("cache_creation_input_tokens");
        let cache_read = get("cache_read_input_tokens");
        let output = get("output_tokens");
        if input + cache_creation + cache_read + output == 0 {
            continue;
        }
        let model_key = message
            .and_then(|m| m.get("model"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "<synthetic>")
            .unwrap_or("unknown");
        let entry = map.entry(model_key.to_string()).or_default();
        entry.message_count += 1;
        entry.input_tokens += input + cache_creation + cache_read;
        entry.output_tokens += output;
        entry.cache_read_tokens += cache_read;
        entry.cache_creation_tokens += cache_creation;
    }
    Ok(map)
}

/// One-time repair of Codex token totals (v1.1.99). Codex reports SESSION-CUMULATIVE
/// token counts in every `token_count` event; the incremental indexer used to ADD
/// that running total on each pass, inflating some sessions ~150x — one reached
/// 61.5B input tokens / $35k versus a true 391M / ~$220. The indexer now SETs
/// cumulative tokens (see `tokens_absolute`), but rows already stored are still
/// wrong. Re-read each Codex file, take the correct final cumulative from the
/// adapter, and overwrite ONLY the token columns + cost — the archive/message
/// counts were never corrupted, so this skips re-archiving and is far cheaper than
/// a full re-index. Gated by a preference so it runs exactly once.
pub fn fix_codex_token_totals(conn: &rusqlite::Connection) {
    if let Ok(Some(rev)) = queries::get_preference(conn, "codex_token_fix_rev") {
        if rev == "1" {
            return;
        }
    }
    let rows: Vec<(String, String, Option<String>)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, jsonl_path, model_used FROM cc_sessions
             WHERE agent_type = 'codex' AND jsonl_path IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("codex token fix prepare failed: {e}");
                return;
            }
        };
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        });
        match mapped {
            Ok(m) => m.filter_map(Result::ok).collect(),
            Err(e) => {
                log::warn!("codex token fix query failed: {e}");
                return;
            }
        }
    };

    let tx = match conn.unchecked_transaction() {
        Ok(t) => t,
        Err(_) => return,
    };
    let mut fixed = 0u64;
    for (id, path, stored_model) in rows {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let summary = CodexAdapter.parse_raw(&path, &raw);
        // Only overwrite when we actually parsed a cumulative token_count; a
        // truncated/odd file shouldn't zero a possibly-valid stored value.
        if !summary.tokens_are_cumulative {
            continue;
        }
        let model = summary.model_used.or(stored_model);
        let cost = estimate_cost(
            model.as_deref().unwrap_or(""),
            summary.total_input_tokens,
            summary.total_output_tokens,
            summary.cache_read_tokens,
            summary.cache_creation_tokens,
        );
        let _ = tx.execute(
            "UPDATE cc_sessions SET
                total_input_tokens = ?2,
                total_output_tokens = ?3,
                cache_read_tokens = ?4,
                cache_creation_tokens = ?5,
                estimated_cost_usd = ?6
             WHERE id = ?1",
            rusqlite::params![
                id,
                summary.total_input_tokens,
                summary.total_output_tokens,
                summary.cache_read_tokens,
                summary.cache_creation_tokens,
                cost,
            ],
        );
        fixed += 1;
    }
    if tx.commit().is_ok() {
        let _ = queries::set_preference(conn, "codex_token_fix_rev", "1");
        log::info!("Fixed Codex cumulative token totals for {fixed} sessions");
    }
}

/// Streaming full-file scan of one Claude JSONL with usage dedup. Returns the
/// deduped totals + per-model map, the last usage key, and the exact cursor
/// (bytes/lines of complete lines consumed) so incremental tailing continues
/// cleanly from where this scan stopped. Mirrors ClaudeCodeAdapter's
/// attribution rules without materialising the (200+ MB) file in memory.
struct ClaudeDedupScan {
    total_input: i64,
    total_output: i64,
    cache_read: i64,
    cache_creation: i64,
    model_usage:
        std::collections::BTreeMap<String, crate::commands::session_adapters::ModelTokenUsage>,
    last_usage_key: Option<String>,
    consumed_bytes: i64,
    consumed_lines: i64,
}

fn scan_claude_usage_dedup(path: &std::path::Path) -> Result<ClaudeDedupScan, String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut reader = std::io::BufReader::new(file);
    let mut scan = ClaudeDedupScan {
        total_input: 0,
        total_output: 0,
        cache_read: 0,
        cache_creation: 0,
        model_usage: std::collections::BTreeMap::new(),
        last_usage_key: None,
        consumed_bytes: 0,
        consumed_lines: 0,
    };
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break, // torn tail → stop at last clean line
        };
        // Only complete lines advance the cursor, matching complete_lines_prefix.
        if buf.last() != Some(&b'\n') {
            break;
        }
        scan.consumed_bytes += n as i64;
        scan.consumed_lines += 1;
        let line = match std::str::from_utf8(&buf) {
            Ok(s) => s.trim(),
            Err(_) => continue,
        };
        if line.is_empty() {
            continue;
        }
        let parsed: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let message = parsed.get("message");
        let usage = message.and_then(|m| m.get("usage"));
        let Some(usage) = usage else { continue };
        let usage_key = message
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
            .map(|id| {
                let request_id = parsed
                    .get("requestId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                format!("{id}:{request_id}")
            });
        if usage_key.is_some() && usage_key.as_deref() == scan.last_usage_key.as_deref() {
            continue; // repeated content-block line of an already-counted message
        }
        if let Some(key) = usage_key {
            scan.last_usage_key = Some(key);
        }
        let get = |key: &str| usage.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
        let input = get("input_tokens");
        let cache_creation = get("cache_creation_input_tokens");
        let cache_read = get("cache_read_input_tokens");
        let output = get("output_tokens");
        scan.total_input += input + cache_creation + cache_read;
        scan.total_output += output;
        scan.cache_read += cache_read;
        scan.cache_creation += cache_creation;
        if input + cache_creation + cache_read + output > 0 {
            let model_key = message
                .and_then(|m| m.get("model"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "<synthetic>")
                .unwrap_or("unknown");
            let entry = scan.model_usage.entry(model_key.to_string()).or_default();
            entry.message_count += 1;
            entry.input_tokens += input + cache_creation + cache_read;
            entry.output_tokens += output;
            entry.cache_read_tokens += cache_read;
            entry.cache_creation_tokens += cache_creation;
        }
    }
    Ok(scan)
}

/// One-time repair of Claude token totals. The indexer used to sum the usage
/// object of EVERY JSONL line, but Claude Code writes one line per content
/// block of an assistant message, each repeating the same final usage — ~50%+
/// of usage lines in real transcripts are such repeats, inflating all Claude
/// token/cost numbers ~2.2× (measured 103–134% per month on this machine).
/// Re-scan each on-disk Claude file with dedup and overwrite the token columns,
/// per-model rows, cost, dedup key, and index cursor. Sessions whose JSONL has
/// rotated away cannot be recomputed and are left untouched. Gated by a
/// preference so it runs exactly once.
pub fn fix_claude_usage_dedup(conn: &rusqlite::Connection) {
    if let Ok(Some(rev)) = queries::get_preference(conn, "claude_dedup_fix_rev") {
        if rev == "1" {
            return;
        }
    }
    let rows: Vec<(String, String, Option<String>)> = {
        let mut stmt = match conn.prepare(
            "SELECT id, jsonl_path, model_used FROM cc_sessions
             WHERE agent_type = 'claude-code' AND jsonl_path IS NOT NULL",
        ) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("claude dedup fix prepare failed: {e}");
                return;
            }
        };
        match stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .and_then(|rows| rows.collect())
        {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("claude dedup fix query failed: {e}");
                return;
            }
        }
    };
    let mut fixed = 0u64;
    let mut skipped_missing = 0u64;
    for (id, path, stored_model) in rows {
        let path_ref = std::path::Path::new(&path);
        if !path_ref.exists() {
            skipped_missing += 1;
            continue;
        }
        let scan = match scan_claude_usage_dedup(path_ref) {
            Ok(scan) => scan,
            Err(e) => {
                log::debug!("claude dedup scan failed for {path}: {e}");
                continue;
            }
        };
        let deltas = model_usage_deltas(&scan.model_usage);
        let cost = if deltas.is_empty() {
            estimate_cost(
                stored_model.as_deref().unwrap_or(""),
                scan.total_input,
                scan.total_output,
                scan.cache_read,
                scan.cache_creation,
            )
        } else {
            let cost: f64 = deltas
                .iter()
                .map(|u| {
                    estimate_cost(
                        &u.model,
                        u.input_tokens,
                        u.output_tokens,
                        u.cache_read_tokens,
                        u.cache_creation_tokens,
                    )
                })
                .sum();
            (cost * 100.0).round() / 100.0
        };
        let updated = conn.execute(
            "UPDATE cc_sessions SET
                total_input_tokens = ?2,
                total_output_tokens = ?3,
                cache_read_tokens = ?4,
                cache_creation_tokens = ?5,
                estimated_cost_usd = ?6,
                last_usage_key = ?7,
                last_indexed_byte_offset = ?8,
                last_indexed_line_count = ?9
             WHERE id = ?1",
            rusqlite::params![
                id,
                scan.total_input,
                scan.total_output,
                scan.cache_read,
                scan.cache_creation,
                cost,
                scan.last_usage_key,
                scan.consumed_bytes,
                scan.consumed_lines,
            ],
        );
        if updated.is_ok() {
            let _ = queries::replace_session_model_usage(conn, &id, &deltas);
            fixed += 1;
        }
    }
    let _ = queries::set_preference(conn, "claude_dedup_fix_rev", "1");
    log::info!(
        "Claude usage dedup backfill: rewrote {fixed} sessions, {skipped_missing} skipped (file rotated away)"
    );
}

fn upsert_adapter_summary_session(
    conn: &rusqlite::Connection,
    project_id: &str,
    summary: RawSessionAdapterSummary,
    file_size: i64,
    file_mtime: Option<String>,
    now: &str,
    existing_session_id: Option<&str>,
) -> Result<IndexedAdapterSession, String> {
    let sid = existing_session_id
        .map(String::from)
        .or_else(|| summary.stable_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let adapter_id = summary.adapter_id.clone();
    let agent_type = summary.agent_type.clone();
    let source_ref = summary.source_ref.clone();
    let message_count = summary.message_count.max(0) as u64;
    let day_counts = summary.day_counts.clone();
    let archive_messages = summary.archive_messages.clone();
    let parse_warnings = summary.parse_warnings.clone();

    for warning in &summary.parse_warnings {
        log::warn!(
            "{} session adapter warning for {}: {}",
            summary.adapter_id,
            source_ref,
            warning
        );
    }

    // Adapter-backed writes reparse the source summary as a whole, so replace
    // old day buckets instead of incrementing on top of stale counts.
    let _ = queries::reset_session_days(conn, &sid);

    // Multi-model sessions cost the sum of their per-model parts; fall back to
    // session-level model_used pricing when the adapter has no breakdown.
    let usage_deltas = model_usage_deltas(&summary.model_usage);
    let estimated_cost = if usage_deltas.is_empty() {
        estimate_cost(
            summary.model_used.as_deref().unwrap_or(""),
            summary.total_input_tokens,
            summary.total_output_tokens,
            summary.cache_read_tokens,
            summary.cache_creation_tokens,
        )
    } else {
        let cost: f64 = usage_deltas
            .iter()
            .map(|u| {
                estimate_cost(
                    &u.model,
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_read_tokens,
                    u.cache_creation_tokens,
                )
            })
            .sum();
        (cost * 100.0).round() / 100.0
    };

    queries::upsert_session(
        conn,
        &queries::SessionInput {
            id: sid.clone(),
            project_id: project_id.to_string(),
            agent_type: Some(agent_type.clone()),
            jsonl_path: Some(source_ref.clone()),
            git_branch: summary.git_branch,
            cwd: summary.cwd,
            cli_version: summary.cli_version,
            first_message: summary.first_timestamp,
            last_message: summary.last_timestamp,
            message_count: Some(summary.message_count),
            total_input_tokens: Some(summary.total_input_tokens),
            total_output_tokens: Some(summary.total_output_tokens),
            model_used: summary.model_used,
            slug: summary.slug,
            file_size_bytes: Some(file_size),
            indexed_at: Some(now.to_string()),
            file_mtime,
            cache_read_tokens: Some(summary.cache_read_tokens),
            cache_creation_tokens: Some(summary.cache_creation_tokens),
            compaction_count: Some(summary.compaction_count),
            estimated_cost_usd: Some(estimated_cost),
        },
    )
    .map_err(|e| e.to_string())?;

    for (day, n) in &day_counts {
        let _ = queries::bump_session_day(conn, &sid, day, *n);
    }

    replace_archive_messages(
        conn,
        &sid,
        &adapter_id,
        &agent_type,
        &source_ref,
        archive_messages,
    )?;

    // Full reparse → replace (not add) the per-model breakdown, mirroring how
    // archive rows are replaced. Empty for non-Claude adapters, which clears
    // nothing since such sessions never had rows.
    queries::replace_session_model_usage(conn, &sid, &usage_deltas).map_err(|e| e.to_string())?;

    Ok(IndexedAdapterSession {
        session_id: sid,
        source_ref,
        messages_indexed: message_count,
        parse_warnings,
    })
}

fn model_usage_deltas(
    map: &std::collections::BTreeMap<String, crate::commands::session_adapters::ModelTokenUsage>,
) -> Vec<queries::SessionModelUsageDelta> {
    map.iter()
        .map(|(model, u)| queries::SessionModelUsageDelta {
            model: model.clone(),
            message_count: u.message_count,
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_creation_tokens: u.cache_creation_tokens,
        })
        .collect()
}

fn replace_archive_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
    adapter_id: &str,
    agent_type: &str,
    source_ref: &str,
    archive_messages: Vec<crate::commands::session_adapters::RawSessionArchiveMessage>,
) -> Result<(), String> {
    let archive_inputs: Vec<_> = archive_messages
        .into_iter()
        .enumerate()
        .map(|(idx, message)| queries::SessionMessageArchiveInput {
            adapter_id: adapter_id.to_string(),
            agent_type: agent_type.to_string(),
            source_ref: source_ref.to_string(),
            source_line: message.source_line,
            message_index: idx as i64,
            role: message.role,
            kind: message.kind,
            timestamp: message.timestamp,
            content_text: message.content_text,
            tool_name: message.tool_name,
            tool_call_id: message.tool_call_id,
            raw_type: message.raw_type,
        })
        .collect();
    queries::replace_session_message_archive(conn, session_id, &archive_inputs)
        .map_err(|e| e.to_string())
}

// ─────────────────────────────────────────────────────────────────
// Claude Code session parsing
// ─────────────────────────────────────────────────────────────────

/// Parse a Claude Code JSONL session file with the shared raw adapter and
/// upsert the normalized session summary.
fn parse_claude_session(
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
) -> Result<IndexedAdapterSession, String> {
    index_adapter_session(&ClaudeCodeAdapter, jsonl_path, conn, project_id, now)
}

/// Prefix of `text` up to and including the last newline, plus its byte length and
/// complete-line count. A half-written trailing line (no newline yet) is excluded,
/// so the indexer never parses a partially-flushed event and always resumes on a
/// clean line boundary. Returns ("", 0, 0) when there is no newline at all.
fn complete_lines_prefix(text: &str) -> (&str, i64, i64) {
    match text.rfind('\n') {
        Some(pos) => {
            let end = pos + 1; // include the '\n'
            let prefix = &text[..end];
            let line_count = prefix.lines().count() as i64;
            (prefix, end as i64, line_count)
        }
        None => ("", 0, 0),
    }
}

#[derive(Debug)]
struct JsonlChunk {
    text: String,
    consumed_bytes: i64,
    line_count: i64,
    inspected_bytes: usize,
    deferred_oversized_offset: Option<i64>,
}

fn live_jsonl_row_is_deferred(path: &str, offset: i64, file_size: i64) -> bool {
    let Ok(mut deferred) = LIVE_DEFERRED_JSONL_ROWS.lock() else {
        return false;
    };
    match deferred.get(path).copied() {
        Some((saved_offset, saved_size)) if saved_offset == offset && file_size >= saved_size => {
            true
        }
        Some(_) => {
            deferred.remove(path);
            false
        }
        None => false,
    }
}

fn defer_live_jsonl_row(path: &str, offset: i64, file_size: i64) {
    if let Ok(mut deferred) = LIVE_DEFERRED_JSONL_ROWS.lock() {
        insert_bounded_deferred_row(&mut deferred, path.to_string(), (offset, file_size));
    }
}

fn insert_bounded_deferred_row(
    deferred: &mut HashMap<String, (i64, i64)>,
    path: String,
    marker: (i64, i64),
) {
    if deferred.len() >= LIVE_DEFERRED_JSONL_MAX_ENTRIES && !deferred.contains_key(&path) {
        if let Some(oldest) = deferred.keys().next().cloned() {
            deferred.remove(&oldest);
        }
    }
    deferred.insert(path, marker);
}

fn clear_deferred_live_jsonl_row(path: &str) {
    if let Ok(mut deferred) = LIVE_DEFERRED_JSONL_ROWS.lock() {
        deferred.remove(path);
    }
}

/// Read complete JSONL rows without reading or allocating an unbounded row.
/// Rows larger than the budget plus the delimiter window are left for the
/// unbounded maintenance index and remembered so live ticks do not rescan them.
fn read_complete_jsonl_chunk(
    path: &std::path::Path,
    offset: i64,
    byte_budget: usize,
) -> Result<JsonlChunk, String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(offset.max(0) as u64))
        .map_err(|e| e.to_string())?;
    let mut reader = std::io::BufReader::new(file);
    let mut chunk = String::new();
    let mut line_count = 0i64;
    let budget = byte_budget.max(1);
    let hard_limit = budget.saturating_add(LIVE_TRANSCRIPT_DELIMITER_WINDOW_BYTES);
    let mut inspected_bytes = 0usize;

    while chunk.len() < budget {
        let remaining = hard_limit.saturating_sub(inspected_bytes);
        if remaining == 0 {
            break;
        }
        let mut line = Vec::new();
        let bytes_read = std::io::Read::by_ref(&mut reader)
            .take(remaining as u64)
            .read_until(b'\n', &mut line)
            .map_err(|e| e.to_string())?;
        inspected_bytes += bytes_read;
        if bytes_read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            // A short read is just the current EOF (usually a writer between
            // flushes). Defer only when the hard read limit was exhausted.
            let deferred_oversized_offset = (chunk.is_empty() && bytes_read == remaining)
                .then_some(offset + chunk.len() as i64);
            return Ok(JsonlChunk {
                consumed_bytes: chunk.len() as i64,
                text: chunk,
                line_count,
                inspected_bytes,
                deferred_oversized_offset,
            });
        }
        let line = std::str::from_utf8(&line).map_err(|e| e.to_string())?;
        chunk.push_str(line);
        line_count += 1;
    }

    Ok(JsonlChunk {
        consumed_bytes: chunk.len() as i64,
        text: chunk,
        line_count,
        inspected_bytes,
        deferred_oversized_offset: None,
    })
}

/// Index one agent session file, incrementally when possible.
///
/// If the session was indexed before and the file only grew, seek to the saved
/// byte offset, parse just the appended tail, and merge the deltas — turning the
/// per-append cost from O(file size) into O(bytes appended). Otherwise (first
/// index, a legacy session with no cursor, or a file that shrank/rotated) do a
/// full parse. See docs/development/performance.md.
fn index_adapter_session<A: SessionSourceAdapter>(
    adapter: &A,
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
) -> Result<IndexedAdapterSession, String> {
    index_adapter_session_with_budget(adapter, jsonl_path, conn, project_id, now, None)
}

fn index_adapter_session_bounded<A: SessionSourceAdapter>(
    adapter: &A,
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
    byte_budget: usize,
) -> Result<IndexedAdapterSession, String> {
    index_adapter_session_with_budget(
        adapter,
        jsonl_path,
        conn,
        project_id,
        now,
        Some(byte_budget.max(1)),
    )
}

fn index_adapter_session_with_budget<A: SessionSourceAdapter>(
    adapter: &A,
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
    byte_budget: Option<usize>,
) -> Result<IndexedAdapterSession, String> {
    let path_str = jsonl_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(jsonl_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let file_mtime = file_meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

    let existing =
        queries::get_session_by_jsonl_path(conn, &path_str).map_err(|e| e.to_string())?;

    if let Some(meta) = &existing {
        let (offset, line_count) =
            queries::get_session_index_cursor(conn, &meta.id).map_err(|e| e.to_string())?;
        if byte_budget.is_some() && live_jsonl_row_is_deferred(&path_str, offset, file_size) {
            return Ok(IndexedAdapterSession {
                session_id: meta.id.clone(),
                source_ref: path_str,
                messages_indexed: 0,
                parse_warnings: Vec::new(),
            });
        }
        // Incremental only when we have a real cursor and the file didn't shrink.
        if offset > 0 && file_size >= offset {
            return index_session_incremental(
                adapter,
                conn,
                &path_str,
                meta,
                offset,
                line_count,
                file_size,
                file_mtime,
                now,
                byte_budget,
            );
        }
        // offset == 0 (never cursored) or file shrank → fall through to full parse.
    }

    // Full maintenance indexing remains unbounded. The live watcher bootstraps
    // only a small complete-line chunk, persists its cursor, and resumes on a
    // later tick instead of parsing a multi-hundred-MB transcript in one loop.
    if byte_budget.is_some() && live_jsonl_row_is_deferred(&path_str, 0, file_size) {
        return Err("oversized JSONL row deferred to maintenance index".to_string());
    }
    let chunk = match byte_budget {
        Some(limit) => read_complete_jsonl_chunk(jsonl_path, 0, limit)?,
        None => {
            let raw = std::fs::read_to_string(jsonl_path).map_err(|e| e.to_string())?;
            let (prefix, byte_len, line_count) = complete_lines_prefix(&raw);
            JsonlChunk {
                text: prefix.to_string(),
                consumed_bytes: byte_len,
                line_count,
                inspected_bytes: raw.len(),
                deferred_oversized_offset: None,
            }
        }
    };
    if let Some(offset) = chunk.deferred_oversized_offset {
        log::debug!(
            "Deferred oversized live JSONL row at {path_str}:{offset} after inspecting {} bytes",
            chunk.inspected_bytes
        );
        defer_live_jsonl_row(&path_str, offset, file_size);
    }
    if chunk.text.is_empty() {
        return Err("session has no complete JSONL row yet".to_string());
    }
    let summary = adapter.parse_raw(&path_str, &chunk.text);
    let last_usage_key = summary.last_usage_key.clone();
    let existing_id = existing.as_ref().map(|m| m.id.as_str());
    let result = upsert_adapter_summary_session(
        conn,
        project_id,
        summary,
        file_size,
        file_mtime,
        now,
        existing_id,
    )?;
    queries::set_session_index_cursor(
        conn,
        &result.session_id,
        chunk.consumed_bytes,
        chunk.line_count,
    )
    .map_err(|e| e.to_string())?;
    queries::set_session_last_usage_key(conn, &result.session_id, last_usage_key.as_deref())
        .map_err(|e| e.to_string())?;
    if byte_budget.is_none() {
        clear_deferred_live_jsonl_row(&path_str);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn index_session_incremental<A: SessionSourceAdapter>(
    adapter: &A,
    conn: &rusqlite::Connection,
    path_str: &str,
    meta: &queries::SessionMeta,
    offset: i64,
    line_count: i64,
    file_size: i64,
    file_mtime: Option<String>,
    now: &str,
    byte_budget: Option<usize>,
) -> Result<IndexedAdapterSession, String> {
    use std::io::{Read, Seek, SeekFrom};

    let chunk = match byte_budget {
        Some(limit) if file_size > offset => {
            read_complete_jsonl_chunk(std::path::Path::new(path_str), offset, limit)?
        }
        _ => {
            let mut tail = String::new();
            if file_size > offset {
                let mut f = std::fs::File::open(path_str).map_err(|e| e.to_string())?;
                f.seek(SeekFrom::Start(offset as u64))
                    .map_err(|e| e.to_string())?;
                f.read_to_string(&mut tail).map_err(|e| e.to_string())?;
            }
            let (prefix, new_bytes, new_lines) = complete_lines_prefix(&tail);
            JsonlChunk {
                text: prefix.to_string(),
                consumed_bytes: new_bytes,
                line_count: new_lines,
                inspected_bytes: tail.len(),
                deferred_oversized_offset: None,
            }
        }
    };
    if let Some(deferred_offset) = chunk.deferred_oversized_offset {
        log::debug!(
            "Deferred oversized live JSONL row at {path_str}:{deferred_offset} after inspecting {} bytes",
            chunk.inspected_bytes
        );
        defer_live_jsonl_row(path_str, deferred_offset, file_size);
    }

    // No complete new line yet (e.g. a half-flushed event, or only mtime changed).
    // Refresh size/mtime so the mtime-skip works next pass; nothing new to index.
    if chunk.text.is_empty() {
        if chunk.deferred_oversized_offset.is_some() {
            return Ok(IndexedAdapterSession {
                session_id: meta.id.clone(),
                source_ref: path_str.to_string(),
                messages_indexed: 0,
                parse_warnings: Vec::new(),
            });
        }
        let _ = queries::apply_session_append_delta(
            conn,
            &zero_delta(meta, offset, line_count, file_size, file_mtime, now),
        );
        return Ok(IndexedAdapterSession {
            session_id: meta.id.clone(),
            source_ref: path_str.to_string(),
            messages_indexed: 0,
            parse_warnings: Vec::new(),
        });
    }

    let summary =
        adapter.parse_raw_with_state(path_str, &chunk.text, meta.last_usage_key.as_deref());

    // Append archive rows, continuing message_index / source_line past what is stored.
    let start_index = meta.archived_message_count;
    let inputs: Vec<queries::SessionMessageArchiveInput> = summary
        .archive_messages
        .iter()
        .enumerate()
        .map(|(i, m)| queries::SessionMessageArchiveInput {
            adapter_id: summary.adapter_id.clone(),
            agent_type: summary.agent_type.clone(),
            source_ref: path_str.to_string(),
            source_line: m.source_line.map(|sl| sl + line_count),
            message_index: start_index + i as i64,
            role: m.role.clone(),
            kind: m.kind.clone(),
            timestamp: m.timestamp.clone(),
            content_text: m.content_text.clone(),
            tool_name: m.tool_name.clone(),
            tool_call_id: m.tool_call_id.clone(),
            raw_type: m.raw_type.clone(),
        })
        .collect();
    queries::append_session_message_archive(conn, &meta.id, &inputs).map_err(|e| e.to_string())?;

    // Per-day counts are additive — bump, never reset.
    for (day, n) in &summary.day_counts {
        let _ = queries::bump_session_day(conn, &meta.id, day, *n);
    }

    let messages_indexed = summary.message_count.max(0) as u64;
    let parse_warnings = summary.parse_warnings.clone();
    queries::apply_session_append_delta(
        conn,
        &queries::SessionAppendDelta {
            session_id: meta.id.clone(),
            add_message_count: summary.message_count,
            add_input_tokens: summary.total_input_tokens,
            add_output_tokens: summary.total_output_tokens,
            add_cache_read_tokens: summary.cache_read_tokens,
            add_cache_creation_tokens: summary.cache_creation_tokens,
            add_compaction_count: summary.compaction_count,
            // Codex reports session-cumulative token totals, so SET rather than
            // add them on each incremental pass (otherwise they compound to
            // billions). Claude reports per-message deltas → add.
            tokens_absolute: summary.tokens_are_cumulative,
            last_message: summary.last_timestamp.clone(),
            first_message: summary.first_timestamp.clone(),
            model_used: summary.model_used.clone(),
            cli_version: summary.cli_version.clone(),
            git_branch: summary.git_branch.clone(),
            cwd: summary.cwd.clone(),
            slug: summary.slug.clone(),
            file_size_bytes: file_size,
            file_mtime,
            indexed_at: now.to_string(),
            new_byte_offset: offset + chunk.consumed_bytes,
            new_line_count: line_count + chunk.line_count,
            last_usage_key: summary.last_usage_key.clone(),
        },
    )
    .map_err(|e| e.to_string())?;

    // Claude reports per-message deltas, so per-model tail usage is additive.
    // Cumulative-total adapters (Codex) leave model_usage empty → no-op.
    queries::add_session_model_usage(conn, &meta.id, &model_usage_deltas(&summary.model_usage))
        .map_err(|e| e.to_string())?;

    // Recompute cost from the NEW totals so it matches a one-shot full re-index
    // exactly (estimate_cost rounds to cents — a per-delta round would drift).
    let (ti, to, cr, cc, model) =
        queries::get_session_token_totals(conn, &meta.id).map_err(|e| e.to_string())?;
    let cost = estimate_session_cost(
        conn,
        &meta.id,
        model.as_deref().unwrap_or(""),
        ti,
        to,
        cr,
        cc,
    );
    queries::set_session_cost(conn, &meta.id, cost).map_err(|e| e.to_string())?;

    if byte_budget.is_none() {
        clear_deferred_live_jsonl_row(path_str);
    }

    Ok(IndexedAdapterSession {
        session_id: meta.id.clone(),
        source_ref: path_str.to_string(),
        messages_indexed,
        parse_warnings,
    })
}

/// A content-free delta that only refreshes file size/mtime/indexed_at and keeps
/// the cursor where it is — used when an append carried no complete new line.
fn zero_delta(
    meta: &queries::SessionMeta,
    offset: i64,
    line_count: i64,
    file_size: i64,
    file_mtime: Option<String>,
    now: &str,
) -> queries::SessionAppendDelta {
    queries::SessionAppendDelta {
        session_id: meta.id.clone(),
        add_message_count: 0,
        add_input_tokens: 0,
        add_output_tokens: 0,
        add_cache_read_tokens: 0,
        add_cache_creation_tokens: 0,
        add_compaction_count: 0,
        tokens_absolute: false,
        last_message: None,
        first_message: None,
        model_used: None,
        cli_version: None,
        git_branch: None,
        cwd: None,
        slug: None,
        file_size_bytes: file_size,
        file_mtime,
        indexed_at: now.to_string(),
        new_byte_offset: offset,
        new_line_count: line_count,
        last_usage_key: None,
    }
}

// ─────────────────────────────────────────────────────────────────
// Codex session parsing
// ─────────────────────────────────────────────────────────────────

fn resolve_codex_base_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".codex")
}

fn resolve_codex_session_roots() -> Vec<std::path::PathBuf> {
    let base = resolve_codex_base_dir();
    vec![base.join("sessions"), base.join("archived_sessions")]
}

fn recent_codex_session_files(max_age: chrono::Duration, limit: usize) -> Vec<std::path::PathBuf> {
    let cutoff = chrono::Utc::now() - max_age;
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = resolve_codex_session_roots()
        .into_iter()
        .filter(|root| root.exists())
        .flat_map(|root| walkdir(&root, "jsonl"))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            let modified_utc = chrono::DateTime::<chrono::Utc>::from(modified);
            (modified_utc >= cutoff).then_some((modified, path))
        })
        .collect();
    files.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    files
        .into_iter()
        .take(limit)
        .map(|(_, path)| path)
        .collect()
}

fn ensure_codex_project_for_jsonl(
    conn: &rusqlite::Connection,
    jsonl_path: &std::path::Path,
    now: &str,
) -> Result<String, String> {
    let first_line = {
        let file = std::fs::File::open(jsonl_path).map_err(|e| e.to_string())?;
        let mut rdr = std::io::BufReader::new(file);
        let mut buf = String::new();
        let _ = rdr.read_line(&mut buf);
        buf
    };
    let meta_parsed: Value = serde_json::from_str(first_line.trim()).map_err(|e| e.to_string())?;
    if meta_parsed.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return Err("first JSONL row is not session_meta".to_string());
    }
    let payload = meta_parsed
        .get("payload")
        .ok_or_else(|| "session_meta row is missing payload".to_string())?;
    let codex_cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| "session_meta row is missing cwd".to_string())?;

    let existing = queries::get_project_id_by_dir(conn, codex_cwd).map_err(|e| e.to_string())?;
    if let Some(project_id) = existing {
        return Ok(project_id);
    }

    let project_id = uuid::Uuid::new_v4().to_string();
    let display_name = std::path::Path::new(codex_cwd)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| codex_cwd.to_string());
    queries::upsert_project(
        conn,
        &queries::ProjectInput {
            id: project_id.clone(),
            display_name,
            dir_path: codex_cwd.to_string(),
            session_count: None,
            last_activity: Some(now.to_string()),
            created_at: now.to_string(),
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(project_id)
}

/// Parse a Codex JSONL session file and upsert the session + messages.
/// Returns (sessions_indexed, messages_indexed).
fn parse_codex_session(
    jsonl_path: &std::path::Path,
    conn: &rusqlite::Connection,
    project_id: &str,
    now: &str,
) -> Result<IndexedAdapterSession, String> {
    index_adapter_session(&CodexAdapter, jsonl_path, conn, project_id, now)
}

fn backfill_missing_session_archives(conn: &rusqlite::Connection) -> Result<u64, String> {
    let candidates =
        queries::list_sessions_needing_archive_backfill(conn, 5_000).map_err(|e| e.to_string())?;
    let mut backfilled = 0u64;

    for candidate in candidates {
        let path = std::path::Path::new(&candidate.jsonl_path);
        if !path.exists() {
            continue;
        }
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) => {
                log::warn!(
                    "Archive backfill could not read {}: {}",
                    candidate.jsonl_path,
                    error
                );
                continue;
            }
        };
        let summary = match candidate.agent_type.as_str() {
            "claude-code" => ClaudeCodeAdapter.parse_raw(&candidate.jsonl_path, &raw),
            "codex" => CodexAdapter.parse_raw(&candidate.jsonl_path, &raw),
            _ => continue,
        };
        if summary.archive_messages.is_empty() {
            continue;
        }
        replace_archive_messages(
            conn,
            &candidate.id,
            &summary.adapter_id,
            &summary.agent_type,
            &summary.source_ref,
            summary.archive_messages,
        )?;
        backfilled += 1;
    }

    Ok(backfilled)
}

// ─────────────────────────────────────────────────────────────────
// Cursor AI session detection & indexing
// ─────────────────────────────────────────────────────────────────

/// Resolve the Cursor data directory (platform-specific).
pub fn resolve_cursor_data_dir() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Cursor")
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join(".config")
            .join("Cursor")
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(appdata).join("Cursor")
    }
}

/// Resolve Cursor's global `state.vscdb`, where modern builds store all
/// AI conversations (composers + bubbles) in the `cursorDiskKV` table.
pub fn resolve_cursor_global_db() -> std::path::PathBuf {
    resolve_cursor_data_dir()
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

/// Look up a value in Cursor's global `ItemTable`. Returns `None` if the
/// DB is missing, the row doesn't exist, or anything goes wrong — caller
/// should treat absence as "feature unavailable" rather than an error.
pub fn read_cursor_item_table(key: &str) -> Option<String> {
    let db_path = resolve_cursor_global_db();
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Index Cursor AI sessions from the global `cursorDiskKV` table.
///
/// Modern Cursor (Anysphere/Glass) stores every AI conversation in:
///   ~/Library/Application Support/Cursor/User/globalStorage/state.vscdb
///
/// Each conversation lives across two key shapes inside `cursorDiskKV`:
///   * `composerData:<composerId>` — JSON with `name`, `createdAt`,
///     `lastUpdatedAt`, `modelConfig.modelName`, `contextTokensUsed`,
///     `workspaceIdentifier.uri.fsPath`, and an ordered
///     `fullConversationHeadersOnly: [{bubbleId, type, ...}]` array.
///   * `bubbleId:<composerId>:<bubbleId>` — JSON for a single message with
///     `type` (1 = user, 2 = assistant), optional `text`, and an ISO
///     `createdAt` timestamp.
///
/// Older workspace-storage `state.vscdb` ItemTable entries (composerData,
/// workbench.panel.aichat, etc.) are no longer produced and have been
/// dropped from this indexer.
fn resolve_grok_sessions_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".grok")
        .join("sessions")
}

/// Decode the percent-encoded cwd Grok uses as a session project dir name
/// (e.g. `%2FUsers%2Fsarthak%2Fproj` -> `/Users/sarthak/proj`).
fn percent_decode_path(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Recursively find the first i64 value for `key` anywhere in a JSON value.
/// Grok's updates.jsonl nests token fields under JSON-RPC `params.update`, so a
/// flat top-level lookup misses them.
fn json_find_i64(value: &Value, key: &str) -> Option<i64> {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key).and_then(|v| v.as_i64()) {
                return Some(found);
            }
            map.values().find_map(|v| json_find_i64(v, key))
        }
        Value::Array(arr) => arr.iter().find_map(|v| json_find_i64(v, key)),
        _ => None,
    }
}

fn record_turn_local_day(
    turn_days: &mut std::collections::BTreeMap<i64, String>,
    turn_millis: i64,
) {
    if let std::collections::btree_map::Entry::Vacant(entry) = turn_days.entry(turn_millis) {
        if let Some(timestamp) = chrono::DateTime::<chrono::Utc>::from_timestamp(
            turn_millis / 1000,
            ((turn_millis % 1000) * 1_000_000) as u32,
        ) {
            entry.insert(
                timestamp
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string(),
            );
        }
    }
}

/// Estimate a Grok session's usage from its on-disk logs. Grok records only a
/// per-turn *context-window size* (`totalTokens` in updates.jsonl), not
/// cumulative billing — so summing the peak context per turn approximates the
/// cumulative input burn (each turn re-sends ~its whole context), the same way
/// Codex's cumulative `total_token_usage` accrues. Output tokens are estimated
/// from `chat_history.jsonl` assistant content (chars÷4 heuristic, same as the
/// Cursor Agent CLI adapter). Cache tokens aren't logged. Day attribution uses
/// the per-turn `agentTimestampMs` from `updates.jsonl` so multi-day sessions
/// spread across the days they actually occurred (not all on day 1).
fn parse_grok_session_dir(
    sess_dir: &std::path::Path,
    cwd: &str,
) -> Result<RawSessionAdapterSummary, String> {
    let source_ref = sess_dir.to_string_lossy().to_string();
    let summary_raw = std::fs::read_to_string(sess_dir.join("summary.json"))
        .map_err(|e| format!("cannot read summary.json: {e}"))?;
    let meta: Value =
        serde_json::from_str(&summary_raw).map_err(|e| format!("bad summary.json: {e}"))?;

    let stable_id = meta
        .get("info")
        .and_then(|i| i.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let model_used = meta
        .get("current_model_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let slug = meta
        .get("generated_title")
        .or_else(|| meta.get("session_summary"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let git_branch = meta
        .get("head_branch")
        .and_then(|v| v.as_str())
        .map(String::from);
    let first_ts = meta
        .get("created_at")
        .and_then(|v| v.as_str())
        .map(String::from);
    let last_ts = meta
        .get("last_active_at")
        .or_else(|| meta.get("updated_at"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let message_count = meta
        .get("num_chat_messages")
        .or_else(|| meta.get("num_messages"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Peak context size per turn (keyed by turn start), summed = input estimate.
    // Also collect per-turn timestamps for day attribution.
    // Token fields are nested under JSON-RPC params, so search recursively.
    let mut per_turn: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    let mut turn_days: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
    if let Ok(file) = std::fs::File::open(sess_dir.join("updates.jsonl")) {
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                let total = json_find_i64(&v, "totalTokens");
                let turn = json_find_i64(&v, "turnStartMs")
                    .or_else(|| json_find_i64(&v, "agentTimestampMs"));
                if let (Some(total), Some(turn)) = (total, turn) {
                    let slot = per_turn.entry(turn).or_insert(0);
                    if total > *slot {
                        *slot = total;
                    }
                    // Record the day for this turn (local timezone, matching
                    // the cc_session_days convention used by other adapters).
                    record_turn_local_day(&mut turn_days, turn);
                }
            }
        }
    }
    let mut estimated_input: i64 = per_turn.values().sum();

    // Fallback: if updates.jsonl yielded nothing, use the peak context size
    // from signals.json (a floor — single snapshot, not cumulative).
    if estimated_input == 0 {
        if let Ok(sig_raw) = std::fs::read_to_string(sess_dir.join("signals.json")) {
            if let Ok(sig) = serde_json::from_str::<Value>(&sig_raw) {
                estimated_input = json_find_i64(&sig, "contextTokensUsed").unwrap_or(0);
            }
        }
    }

    // Output token estimate: sum assistant message content chars ÷ 4 from
    // chat_history.jsonl. Grok doesn't log output tokens, so this is the same
    // chars-per-token heuristic the Cursor Agent CLI adapter uses. Rough but
    // magnitude-aware — without it the "Fresh tokens" bar shows 0 for Grok.
    let mut estimated_output: i64 = 0;
    if let Ok(file) = std::fs::File::open(sess_dir.join("chat_history.jsonl")) {
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                if grok_chat_role(&v) == Some("assistant") {
                    if let Some(content) = grok_chat_content(&v) {
                        estimated_output += content_char_count(content) / 4;
                    }
                }
            }
        }
    }

    // Day attribution: count distinct turns per day from updates.jsonl. Falls
    // back to the old single-day attribution (all on created_at's day) when
    // updates.jsonl has no turn timestamps.
    let mut day_counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    if !turn_days.is_empty() {
        for day in turn_days.values() {
            *day_counts.entry(day.clone()).or_insert(0) += 1;
        }
    } else if let Some(ts) = first_ts.as_deref() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
            let day = dt
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string();
            day_counts.insert(day, message_count.max(1));
        }
    }

    Ok(RawSessionAdapterSummary {
        adapter_id: "grok".to_string(),
        agent_type: "grok".to_string(),
        stable_id,
        source_ref,
        cwd: Some(cwd.to_string()),
        git_branch,
        cli_version: None,
        model_used,
        first_timestamp: first_ts,
        last_timestamp: last_ts,
        message_count,
        total_input_tokens: estimated_input,
        total_output_tokens: estimated_output,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        compaction_count: 0,
        slug,
        day_counts,
        archive_messages: Vec::new(),
        parse_warnings: Vec::new(),
        // Grok token counts are summed per-turn estimates, not a running total.
        tokens_are_cumulative: false,
        model_usage: std::collections::BTreeMap::new(),
        last_usage_key: None,
    })
}

/// Count characters in a chat_history.jsonl `content` field, which can be either
/// a plain string or an array of `{type: "text", text: "..."}` content blocks.
fn content_char_count(content: &Value) -> i64 {
    match content {
        Value::String(s) => s.chars().count() as i64,
        Value::Array(arr) => arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .map(|s| s.chars().count() as i64)
            .sum(),
        _ => 0,
    }
}

fn grok_chat_role(value: &Value) -> Option<&str> {
    value
        .get("type")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("role").and_then(|v| v.as_str()))
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(|v| v.as_str())
        })
}

fn grok_chat_content(value: &Value) -> Option<&Value> {
    value.get("content").or_else(|| {
        value
            .get("message")
            .and_then(|message| message.get("content"))
    })
}

/// Phase 4: index Grok CLI sessions from ~/.grok/sessions. Token counts are
/// per-turn-context estimates (see `parse_grok_session_dir`), not exact billing.
fn index_grok_sessions(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let base = resolve_grok_sessions_dir();
    if !base.exists() {
        return Ok((0, 0, 0));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let (mut indexed, mut messages, mut skipped) = (0u64, 0u64, 0u64);

    let project_dirs = match std::fs::read_dir(&base) {
        Ok(rd) => rd,
        Err(_) => return Ok((0, 0, 0)),
    };
    for proj_entry in project_dirs.filter_map(|e| e.ok()) {
        let proj_dir = proj_entry.path();
        if !proj_dir.is_dir() {
            continue;
        }
        let encoded = proj_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let cwd = percent_decode_path(&encoded);
        let project_id = queries::get_project_id_by_dir(conn, &cwd)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| {
                let pid = uuid::Uuid::new_v4().to_string();
                let display = std::path::Path::new(&cwd)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cwd.clone());
                let _ = queries::upsert_project(
                    conn,
                    &queries::ProjectInput {
                        id: pid.clone(),
                        display_name: display,
                        dir_path: cwd.clone(),
                        session_count: None,
                        last_activity: Some(now.clone()),
                        created_at: now.clone(),
                    },
                );
                pid
            });

        let session_dirs = match std::fs::read_dir(&proj_dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for sess_entry in session_dirs.filter_map(|e| e.ok()) {
            let sess_dir = sess_entry.path();
            if !sess_dir.is_dir() {
                continue; // skip per-project prompt_history.jsonl etc.
            }
            let summary_path = sess_dir.join("summary.json");
            if !summary_path.exists() {
                continue;
            }
            let source_ref = sess_dir.to_string_lossy().to_string();
            let file_meta = std::fs::metadata(&summary_path).ok();
            let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
            let file_mtime = file_meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

            let existing =
                queries::get_session_by_jsonl_path(conn, &source_ref).map_err(|e| e.to_string())?;
            if let Some(ref meta) = existing {
                // Re-parse 0-token rows: earlier builds estimated Grok at 0
                // (token fields were nested), so don't let the mtime skip pin them.
                // Also re-parse rows with 0 output tokens — the old parser didn't
                // estimate output from chat_history.jsonl (added in a later rev).
                if meta.file_mtime.as_deref() == file_mtime.as_deref()
                    && meta.message_count > 0
                    && meta.total_input_tokens > 0
                    && meta.total_output_tokens > 0
                {
                    skipped += 1;
                    continue;
                }
            }

            match parse_grok_session_dir(&sess_dir, &cwd) {
                Ok(summary) => {
                    let msg = summary.message_count.max(0) as u64;
                    match upsert_adapter_summary_session(
                        conn,
                        &project_id,
                        summary,
                        file_size,
                        file_mtime,
                        &now,
                        existing.as_ref().map(|m| m.id.as_str()),
                    ) {
                        Ok(_) => {
                            indexed += 1;
                            messages += msg;
                        }
                        Err(e) => log::warn!("grok upsert failed for {source_ref}: {e}"),
                    }
                }
                Err(e) => log::warn!("grok parse failed for {source_ref}: {e}"),
            }
        }
    }

    Ok((indexed, messages, skipped))
}

pub(crate) fn resolve_devin_sessions_db() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("devin")
        .join("cli")
        .join("sessions.db")
}

/// Phase 6: index Devin CLI sessions from ~/.local/share/devin/cli/sessions.db.
///
/// Devin stores sessions in a single SQLite DB (not JSONL). Each `sessions` row
/// has a `working_directory`, `model`, unix-second `created_at`/`last_activity_at`,
/// and a `title`. Token metrics live inside `message_nodes.chat_message` JSON,
/// under `metadata.metrics` (input/output/cache_read/cache_creation tokens) and
/// `metadata.generation_model`.
///
/// IMPORTANT: Devin writes duplicate `message_nodes` rows per logical message
/// (one with `extensions`, one without) sharing the same `message_id` and
/// identical token metrics. Summing all rows would ~2x the real token burn, so
/// we dedupe by `message_id` before aggregating (verified: 14.6k rows → 6.8k
/// distinct message_ids on the dev machine).
///
/// `total_input_tokens` follows the cc_sessions convention of including cache
/// read + cache creation (estimate_cost subtracts them back out to bill the
/// base input at the full rate).
fn index_devin_sessions(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let db_path = resolve_devin_sessions_db();
    index_devin_sessions_from_path(conn, &db_path)
}

fn index_devin_sessions_from_path(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
) -> Result<(u64, u64, u64), String> {
    if !db_path.exists() {
        return Ok((0, 0, 0));
    }

    let file_meta = std::fs::metadata(db_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);

    let dconn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("failed to open devin sessions.db: {e}"))?;

    // Read cheap session watermarks first. Devin chat_message values can make
    // sessions.db very large, so aggregating every message before checking
    // last_activity_at turns every steady-state refresh into a full DB scan.
    let mut sess_stmt = dconn
        .prepare(
            "SELECT s.id,
                    s.working_directory,
                    s.model,
                    s.backend_type,
                    s.created_at,
                    s.last_activity_at,
                    s.title
             FROM sessions s
             ORDER BY s.last_activity_at DESC",
        )
        .map_err(|e| format!("devin session query prepare failed: {e}"))?;

    let session_rows = sess_stmt
        .query_map([], |r| {
            Ok(DevinSessionHeader {
                id: r.get(0)?,
                working_directory: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                model: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                backend_type: r.get::<_, Option<String>>(3)?,
                created_at: r.get(4)?,
                last_activity_at: r.get(5)?,
                title: r.get::<_, Option<String>>(6)?,
            })
        })
        .map_err(|e| format!("devin session query failed: {e}"))?;
    let sessions: Vec<DevinSessionHeader> = session_rows.filter_map(Result::ok).collect();
    drop(sess_stmt);

    // Only changed sessions reach these JSON aggregates. Duplicate assistant
    // rows are still deduped by message_id exactly as in the original query.
    let mut metrics_stmt = dconn
        .prepare(
            "SELECT COUNT(*),
                    COALESCE(SUM(in_t), 0),
                    COALESCE(SUM(out_t), 0),
                    COALESCE(SUM(cr), 0),
                    COALESCE(SUM(cc), 0)
               FROM (
                    SELECT json_extract(chat_message, '$.message_id') AS message_id,
                           MAX(json_extract(chat_message, '$.metadata.metrics.input_tokens'))          AS in_t,
                           MAX(json_extract(chat_message, '$.metadata.metrics.output_tokens'))         AS out_t,
                           MAX(json_extract(chat_message, '$.metadata.metrics.cache_read_tokens'))     AS cr,
                           MAX(json_extract(chat_message, '$.metadata.metrics.cache_creation_tokens')) AS cc
                      FROM message_nodes
                     WHERE session_id = ?1
                       AND json_extract(chat_message, '$.role') = 'assistant'
                       AND json_extract(chat_message, '$.metadata.metrics.input_tokens') IS NOT NULL
                     GROUP BY message_id
               )",
        )
        .map_err(|e| format!("devin metrics query prepare failed: {e}"))?;
    let mut model_stmt = dconn
        .prepare(
            "SELECT (SELECT json_extract(chat_message, '$.metadata.generation_model')
                       FROM message_nodes
                      WHERE session_id = ?1
                        AND json_extract(chat_message, '$.role') = 'assistant'
                        AND json_extract(chat_message, '$.metadata.generation_model') IS NOT NULL
                      ORDER BY created_at DESC LIMIT 1)",
        )
        .map_err(|e| format!("devin model query prepare failed: {e}"))?;
    let mut day_stmt = dconn
        .prepare(
            "SELECT date(created_at, 'unixepoch', 'localtime') AS day,
                    COUNT(DISTINCT json_extract(chat_message, '$.message_id')) AS n
               FROM message_nodes
              WHERE session_id = ?1
                AND json_extract(chat_message, '$.role') = 'assistant'
                AND json_extract(chat_message, '$.metadata.metrics.input_tokens') IS NOT NULL
              GROUP BY day",
        )
        .map_err(|e| format!("devin day query prepare failed: {e}"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut indexed = 0u64;
    let mut messages = 0u64;
    let mut skipped = 0u64;

    for s in &sessions {
        let source_ref = format!("devin:{}", s.id);
        let last_activity_rfc =
            chrono::DateTime::<chrono::Utc>::from_timestamp(s.last_activity_at, 0)
                .map(|dt| dt.to_rfc3339());
        let created_rfc = chrono::DateTime::<chrono::Utc>::from_timestamp(s.created_at, 0)
            .map(|dt| dt.to_rfc3339());

        // Per-session incremental skip: the devin session's last_activity_at is
        // stored as file_mtime, so an unchanged value + non-zero tokens means
        // nothing new has happened for this session.
        let existing =
            queries::get_session_by_jsonl_path(conn, &source_ref).map_err(|e| e.to_string())?;
        if let Some(ref meta) = existing {
            if meta.file_mtime.as_deref() == last_activity_rfc.as_deref()
                && meta.total_input_tokens > 0
            {
                skipped += 1;
                continue;
            }
        }

        let (msg_count, input_toks, output_toks, cache_read, cache_creation): (
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = metrics_stmt
            .query_row(rusqlite::params![s.id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(|e| format!("devin metrics query failed for {source_ref}: {e}"))?;

        // Skip sessions with no token activity (e.g. freshly created, no
        // assistant turns yet) — they'd insert zero-token rows that clutter
        // the dashboard without contributing usage.
        if msg_count == 0 && input_toks == 0 {
            skipped += 1;
            continue;
        }

        let gen_model = model_stmt
            .query_row(rusqlite::params![s.id], |r| r.get::<_, Option<String>>(0))
            .map_err(|e| format!("devin model query failed for {source_ref}: {e}"))?;
        let day_rows = day_stmt
            .query_map(rusqlite::params![s.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("devin day query failed for {source_ref}: {e}"))?;
        let day_counts = day_rows.flatten().collect();

        let cwd = if s.working_directory.is_empty() {
            None
        } else {
            Some(s.working_directory.clone())
        };

        // Resolve or create the project for this session's cwd.
        let project_id = if let Some(ref cwd) = cwd {
            queries::get_project_id_by_dir(conn, cwd)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| {
                    let pid = uuid::Uuid::new_v4().to_string();
                    let display = std::path::Path::new(cwd)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| cwd.clone());
                    let _ = queries::upsert_project(
                        conn,
                        &queries::ProjectInput {
                            id: pid.clone(),
                            display_name: display,
                            dir_path: cwd.clone(),
                            session_count: None,
                            last_activity: Some(now.clone()),
                            created_at: now.clone(),
                        },
                    );
                    pid
                })
        } else {
            // No cwd — attribute to a synthetic "Devin" project.
            let dir = "devin://unknown";
            queries::get_project_id_by_dir(conn, dir)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| {
                    let pid = uuid::Uuid::new_v4().to_string();
                    let _ = queries::upsert_project(
                        conn,
                        &queries::ProjectInput {
                            id: pid.clone(),
                            display_name: "Devin".to_string(),
                            dir_path: dir.to_string(),
                            session_count: None,
                            last_activity: Some(now.clone()),
                            created_at: now.clone(),
                        },
                    );
                    pid
                })
        };

        // cc_sessions.total_input_tokens includes cache_read + cache_creation
        // (estimate_cost subtracts them back out to bill base input at full rate).
        let total_input = input_toks + cache_read + cache_creation;
        let model_used = gen_model.or_else(|| {
            if s.model.is_empty() {
                None
            } else {
                Some(s.model.clone())
            }
        });

        let summary = RawSessionAdapterSummary {
            adapter_id: "devin".to_string(),
            agent_type: "devin".to_string(),
            stable_id: Some(s.id.clone()),
            source_ref: source_ref.clone(),
            cwd,
            git_branch: None,
            cli_version: s.backend_type.clone(),
            model_used,
            first_timestamp: created_rfc.clone(),
            last_timestamp: last_activity_rfc.clone(),
            message_count: msg_count,
            total_input_tokens: total_input,
            total_output_tokens: output_toks,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            compaction_count: 0,
            slug: s.title.clone(),
            day_counts,
            archive_messages: Vec::new(),
            parse_warnings: Vec::new(),
            tokens_are_cumulative: false,
            model_usage: std::collections::BTreeMap::new(),
            last_usage_key: None,
        };

        match upsert_adapter_summary_session(
            conn,
            &project_id,
            summary,
            file_size,
            last_activity_rfc.clone(),
            &now,
            existing.as_ref().map(|m| m.id.as_str()),
        ) {
            Ok(session) => {
                indexed += 1;
                messages += session.messages_indexed;
            }
            Err(e) => log::warn!("devin upsert failed for {source_ref}: {e}"),
        }
    }

    Ok((indexed, messages, skipped))
}

#[derive(Debug, Clone)]
struct DevinSessionHeader {
    id: String,
    working_directory: String,
    model: String,
    backend_type: Option<String>,
    created_at: i64,
    last_activity_at: i64,
    title: Option<String>,
}

fn resolve_cursor_agent_chats_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".cursor").join("chats")
}

/// Estimate a Cursor Agent CLI session's usage from its `store.db` message
/// blobs. Cursor logs no token counts and no per-turn context — only role +
/// content text. We approximate cumulative input as the re-sent context: at
/// each assistant turn the model re-reads everything before it, so input grows
/// by `prior_context_chars / 4`. Output is the assistant text / 4. Rough by
/// nature (chars-per-token heuristic), but magnitude-aware like the other
/// estimates. Returns (input_est, output_est, message_count, model).
fn estimate_cursor_agent_session(
    store_db: &std::path::Path,
) -> Result<(i64, i64, i64, Option<String>), String> {
    let conn =
        rusqlite::Connection::open_with_flags(store_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT data FROM blobs ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(|e| e.to_string())?;

    let mut context_chars: i64 = 0;
    let (mut input_est, mut output_est, mut message_count) = (0i64, 0i64, 0i64);
    let mut model: Option<String> = None;

    for data in rows.flatten() {
        // Blobs are a mix of JSON messages and opaque binary; skip non-JSON.
        let text = match std::str::from_utf8(&data) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let role = value.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if role.is_empty() {
            continue;
        }
        if model.is_none() {
            model = value
                .get("model")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
        let chars = content.chars().count() as i64;
        if role == "assistant" {
            input_est += context_chars / 4; // context re-sent for this turn
            output_est += chars / 4;
        }
        context_chars += chars;
        message_count += 1;
    }

    Ok((input_est, output_est, message_count, model))
}

/// Phase 5: index Cursor Agent CLI sessions from ~/.cursor/chats/<ws>/<uuid>/
/// store.db. Token counts are content-length estimates (see
/// `estimate_cursor_agent_session`), the roughest of the adapters.
fn index_cursor_agent_sessions(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let base = resolve_cursor_agent_chats_dir();
    if !base.exists() {
        return Ok((0, 0, 0));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let (mut indexed, mut messages, mut skipped) = (0u64, 0u64, 0u64);

    // All Cursor Agent sessions share one synthetic project — store.db has no
    // reliable cwd (only embedded in user-message text).
    let chats_dir = base.to_string_lossy().to_string();
    let project_id = queries::get_project_id_by_dir(conn, &chats_dir)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| {
            let pid = uuid::Uuid::new_v4().to_string();
            let _ = queries::upsert_project(
                conn,
                &queries::ProjectInput {
                    id: pid.clone(),
                    display_name: "Cursor Agent".to_string(),
                    dir_path: chats_dir.clone(),
                    session_count: None,
                    last_activity: Some(now.clone()),
                    created_at: now.clone(),
                },
            );
            pid
        });

    // Find every store.db two levels deep: <chats>/<workspace>/<uuid>/store.db
    for ws_entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let ws_dir = ws_entry.path();
        if !ws_dir.is_dir() {
            continue;
        }
        for sess_entry in std::fs::read_dir(&ws_dir).into_iter().flatten().flatten() {
            let store_db = sess_entry.path().join("store.db");
            if !store_db.exists() {
                continue;
            }
            let source_ref = store_db.to_string_lossy().to_string();
            let file_meta = std::fs::metadata(&store_db).ok();
            let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
            let file_mtime = file_meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

            let existing =
                queries::get_session_by_jsonl_path(conn, &source_ref).map_err(|e| e.to_string())?;
            if let Some(ref meta) = existing {
                if meta.file_mtime.as_deref() == file_mtime.as_deref()
                    && meta.message_count > 0
                    && meta.total_input_tokens > 0
                {
                    skipped += 1;
                    continue;
                }
            }

            let (input_est, output_est, msg_count, model) =
                match estimate_cursor_agent_session(&store_db) {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!("cursor-agent estimate failed for {source_ref}: {e}");
                        continue;
                    }
                };
            if msg_count == 0 {
                continue;
            }

            let stable_id = sess_entry
                .path()
                .file_name()
                .map(|s| s.to_string_lossy().to_string());
            let mut day_counts: std::collections::BTreeMap<String, i64> =
                std::collections::BTreeMap::new();
            if let Some(ts) = file_mtime.as_deref() {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
                    let day = dt
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d")
                        .to_string();
                    day_counts.insert(day, msg_count.max(1));
                }
            }

            let summary = RawSessionAdapterSummary {
                adapter_id: "cursor-agent".to_string(),
                agent_type: "cursor".to_string(),
                stable_id,
                source_ref: source_ref.clone(),
                cwd: None,
                git_branch: None,
                cli_version: None,
                model_used: model,
                first_timestamp: file_mtime.clone(),
                last_timestamp: file_mtime.clone(),
                message_count: msg_count,
                total_input_tokens: input_est,
                total_output_tokens: output_est,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                compaction_count: 0,
                slug: None,
                day_counts,
                archive_messages: Vec::new(),
                parse_warnings: Vec::new(),
                tokens_are_cumulative: false,
                model_usage: std::collections::BTreeMap::new(),
                last_usage_key: None,
            };

            match upsert_adapter_summary_session(
                conn,
                &project_id,
                summary,
                file_size,
                file_mtime,
                &now,
                existing.as_ref().map(|m| m.id.as_str()),
            ) {
                Ok(_) => {
                    indexed += 1;
                    messages += msg_count.max(0) as u64;
                }
                Err(e) => log::warn!("cursor-agent upsert failed for {source_ref}: {e}"),
            }
        }
    }

    Ok((indexed, messages, skipped))
}

fn index_cursor_sessions(conn: &rusqlite::Connection) -> Result<(u64, u64, u64), String> {
    let db_path = resolve_cursor_global_db();
    let index_started_at = chrono::Utc::now().to_rfc3339();
    let mut cursor_run = ProductionAdapterRunStats::new(
        "cursor",
        "cursor",
        vec![db_path.to_string_lossy().to_string()],
        true,
    );
    if !db_path.exists() {
        persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
        return Ok((0, 0, 0));
    }

    let db_path_str = db_path.to_string_lossy().to_string();
    let file_meta = std::fs::metadata(&db_path).ok();
    let file_size = file_meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);

    let cursor_db = match rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(e) => {
            cursor_run.record_warning(&db_path_str, &e.to_string());
            persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
            log::warn!("Failed to open Cursor global db {}: {}", db_path_str, e);
            return Ok((0, 0, 0));
        }
    };

    // Collect every composer up-front so we can drop the statement and then
    // re-use the connection to query bubbles in the loop below.
    let composers: Vec<(String, String)> = {
        let mut stmt = match cursor_db
            .prepare("SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%'")
        {
            Ok(s) => s,
            Err(error) => {
                cursor_run.record_warning(&db_path_str, &error.to_string());
                persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
                return Ok((0, 0, 0));
            }
        };
        let mut out = Vec::new();
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for r in rows.flatten() {
                if let Some(cid) = r.0.strip_prefix("composerData:") {
                    if cid == "empty-state-draft" || cid.is_empty() {
                        continue;
                    }
                    out.push((cid.to_string(), r.1));
                }
            }
        }
        out
    };

    if composers.is_empty() {
        persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
        return Ok((0, 0, 0));
    }

    let mut bubble_stmt = match cursor_db.prepare("SELECT value FROM cursorDiskKV WHERE key = ?1") {
        Ok(s) => s,
        Err(error) => {
            cursor_run.record_warning(&db_path_str, &error.to_string());
            persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
            return Ok((0, 0, 0));
        }
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut indexed_sessions = 0u64;
    let mut indexed_messages = 0u64;
    let mut skipped_sessions = 0u64;

    for (composer_id, composer_json) in composers {
        let composer: Value = match serde_json::from_str(&composer_json) {
            Ok(v) => v,
            Err(error) => {
                cursor_run.record_warning(
                    &format!("{db_path_str}#cursor-{composer_id}"),
                    &error.to_string(),
                );
                continue;
            }
        };

        let headers = composer
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array());
        let header_count = headers.map(|h| h.len()).unwrap_or(0);
        if header_count == 0 {
            cursor_run.record_warning(
                &format!("{db_path_str}#cursor-{composer_id}"),
                "composer has no conversation headers",
            );
            continue;
        }

        let composer_mtime: Option<String> = composer
            .get("lastUpdatedAt")
            .and_then(|v| v.as_i64())
            .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.to_rfc3339()));

        let session_id = format!("cursor-{}", composer_id);
        let composite_path = format!("{}#{}", db_path_str, session_id);

        let existing =
            queries::get_session_by_jsonl_path(conn, &composite_path).map_err(|e| e.to_string())?;
        if let Some(ref existing) = existing {
            if existing.file_mtime.as_deref() == composer_mtime.as_deref()
                && existing.message_count > 0
                && existing.archived_message_count > 0
            {
                skipped_sessions += 1;
                continue;
            }
        }

        let mut bubbles = Vec::new();
        if let Some(hdrs) = headers {
            for hdr in hdrs {
                let Some(bid) = hdr.get("bubbleId").and_then(|v| v.as_str()) else {
                    continue;
                };
                let key = format!("bubbleId:{}:{}", composer_id, bid);
                let bubble_json: Option<String> = bubble_stmt
                    .query_row(rusqlite::params![key], |row| row.get::<_, String>(0))
                    .ok();
                let Some(bubble_json) = bubble_json else {
                    continue;
                };
                if let Ok(bubble) = serde_json::from_str::<Value>(&bubble_json) {
                    bubbles.push(bubble);
                } else {
                    cursor_run.record_warning(&composite_path, "bubble row is not valid JSON");
                }
            }
        }

        let raw = json!({
            "composer_id": composer_id,
            "composer": composer,
            "bubbles": bubbles,
        })
        .to_string();
        let mut summary = CursorAdapter.parse_raw(&composite_path, &raw);

        if summary.message_count == 0 {
            for warning in &summary.parse_warnings {
                cursor_run.record_warning(&composite_path, warning);
            }
            continue;
        }

        let cwd = summary
            .cwd
            .clone()
            .unwrap_or_else(|| "Cursor (no workspace)".to_string());
        let project_id = queries::get_project_id_by_dir(conn, &cwd)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| {
                let pid = uuid::Uuid::new_v4().to_string();
                let display = std::path::Path::new(&cwd)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| cwd.clone());
                let _ = queries::upsert_project(
                    conn,
                    &queries::ProjectInput {
                        id: pid.clone(),
                        display_name: display,
                        dir_path: cwd.clone(),
                        session_count: None,
                        last_activity: Some(now.clone()),
                        created_at: now.clone(),
                    },
                );
                pid
            });

        // `contextTokensUsed` is the *last* conversation context size, not a
        // cumulative billing figure — using it as a token total understates
        // Cursor's real burn by orders of magnitude (every assistant turn
        // re-sends the whole context). The live `api2.cursor.sh` call below
        // is the source of truth for usage. We deliberately don't fabricate
        // a token count locally.
        summary.total_input_tokens = 0;
        summary.total_output_tokens = 0;
        summary.cache_read_tokens = 0;
        summary.cache_creation_tokens = 0;
        summary.compaction_count = 0;

        // Cursor doesn't ship per-message token counts in local storage and
        // the composer's `contextTokensUsed` is a snapshot, not a cumulative
        // total — so we store 0 here and rely on the live API in
        // `check_live_usage_cursor` for actual usage figures.
        let session = upsert_adapter_summary_session(
            conn,
            &project_id,
            summary,
            file_size,
            composer_mtime,
            &now,
            existing.as_ref().map(|m| m.id.as_str()),
        )?;

        indexed_sessions += 1;
        indexed_messages += session.messages_indexed;
        cursor_run.record_session(&session);
    }

    persist_production_adapter_run(conn, &cursor_run, &index_started_at)?;
    Ok((indexed_sessions, indexed_messages, skipped_sessions))
}

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

fn conn_lock<'a>(
    db: &'a State<'a, DbState>,
) -> Result<std::sync::MutexGuard<'a, rusqlite::Connection>, String> {
    db.0.lock().map_err(|e| e.to_string())
}

/// Collect Claude profile project directories.
/// Scans ccusage defaults plus any ~/.claude-*/projects/ profiles.
fn resolve_all_claude_projects_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let home_path = std::path::PathBuf::from(&home);
    let mut dirs = Vec::new();

    if let Ok(config_dirs) = std::env::var("CLAUDE_CONFIG_DIR") {
        for raw in config_dirs
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let projects_dir = std::path::PathBuf::from(raw).join("projects");
            if projects_dir.exists() && !dirs.contains(&projects_dir) {
                dirs.push(projects_dir);
            }
        }
        return dirs;
    }

    let default_dirs = [
        home_path.join(".config").join("claude").join("projects"),
        home_path.join(".claude").join("projects"),
    ];

    for projects_dir in default_dirs {
        if projects_dir.exists() && !dirs.contains(&projects_dir) {
            dirs.push(projects_dir);
        }
    }

    if let Ok(entries) = std::fs::read_dir(&home_path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(".claude-") && entry.path().is_dir() {
                let projects_dir = entry.path().join("projects");
                if projects_dir.exists() && !dirs.contains(&projects_dir) {
                    dirs.push(projects_dir);
                }
            }
        }
    }

    dirs
}

/// Recursively collect files with the given extension.
fn walkdir(dir: &std::path::Path, ext: &str) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path, ext));
            } else if path.extension().map(|e| e == ext).unwrap_or(false) {
                results.push(path);
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::{params, Connection};

    fn memory_conn_with_project() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        schema::run_migrations(&conn).expect("schema");
        queries::upsert_project(
            &conn,
            &queries::ProjectInput {
                id: "project".to_string(),
                display_name: "CodeVetter".to_string(),
                dir_path: "/repo/codevetter".to_string(),
                session_count: None,
                last_activity: Some("2026-06-12T16:00:00Z".to_string()),
                created_at: "2026-06-12T16:00:00Z".to_string(),
            },
        )
        .expect("project");
        conn
    }

    // N deterministic, newline-terminated Claude events. Splitting these bytes at
    // any line boundary yields a valid indexed prefix + a valid appended tail.
    fn synth_claude_events(n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            let day = 10 + (i % 3);
            s.push_str(&format!(
                "{{\"type\":\"{role}\",\"sessionId\":\"S\",\"version\":\"1.2.3\",\"gitBranch\":\"main\",\"cwd\":\"/p\",\"timestamp\":\"2026-06-{day:02}T0{hour}:00:00Z\",\"message\":{{\"role\":\"{role}\",\"model\":\"claude-sonnet-4\",\"content\":\"line {i}\",\"usage\":{{\"input_tokens\":{inp},\"output_tokens\":{out},\"cache_read_input_tokens\":3,\"cache_creation_input_tokens\":2}}}}}}\n",
                role = role, day = day, hour = i % 9, i = i, inp = (i as i64) + 1, out = (i as i64) * 2
            ));
        }
        s
    }

    fn synth_codex_events(n: usize) -> String {
        let mut rows = vec![json!({
            "timestamp": "2026-06-12T08:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "codex-bounded-session",
                "cwd": "/repo/codevetter",
                "cli_version": "1.0.0",
                "model_provider": "openai"
            }
        })
        .to_string()];
        for i in 0..n {
            rows.push(
                json!({
                    "timestamp": format!("2026-06-12T08:{:02}:00Z", i % 60),
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "x".repeat(384)}]
                    }
                })
                .to_string(),
            );
            rows.push(
                json!({
                    "timestamp": format!("2026-06-12T08:{:02}:01Z", i % 60),
                    "type": "event_msg",
                    "payload": {
                        "type": "token_count",
                        "info": {"total_token_usage": {
                            "input_tokens": (i + 1) * 100,
                            "output_tokens": (i + 1) * 20,
                            "cached_input_tokens": (i + 1) * 10
                        }}
                    }
                })
                .to_string(),
            );
        }
        rows.join("\n") + "\n"
    }

    type IndexSnapshot = (
        Vec<i64>,
        Vec<(i64, Option<i64>, Option<String>, String, Option<String>)>,
        Vec<(String, i64)>,
    );

    fn index_snapshot(conn: &Connection, path: &str) -> IndexSnapshot {
        let (sid, totals) = conn
            .query_row(
                "SELECT id, message_count, total_input_tokens, total_output_tokens,
                        cache_read_tokens, cache_creation_tokens, compaction_count,
                        last_indexed_byte_offset, last_indexed_line_count,
                        CAST(ROUND(estimated_cost_usd * 100) AS INTEGER)
                 FROM cc_sessions WHERE jsonl_path = ?1",
                params![path],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        vec![
                            r.get::<_, i64>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, i64>(6)?,
                            r.get::<_, i64>(7)?,
                            r.get::<_, i64>(8)?,
                            r.get::<_, i64>(9)?,
                        ],
                    ))
                },
            )
            .expect("session row");

        let mut stmt = conn
            .prepare(
                "SELECT message_index, source_line, role, kind, content_text
                 FROM session_message_archive WHERE session_id = ?1 ORDER BY message_index",
            )
            .unwrap();
        let archive = stmt
            .query_map(params![sid], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();

        let mut dstmt = conn
            .prepare(
                "SELECT day, msg_count FROM cc_session_days WHERE session_id = ?1 ORDER BY day",
            )
            .unwrap();
        let days = dstmt
            .query_map(params![sid], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();

        (totals, archive, days)
    }

    #[test]
    fn incremental_index_matches_full_reindex_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("cv_inc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let events = synth_claude_events(40);
        let split = events
            .match_indices('\n')
            .nth(16)
            .map(|(i, _)| i + 1)
            .unwrap();

        // (A) full index of the whole 40-event file.
        let conn_a = memory_conn_with_project();
        let path_a = dir.join("a.jsonl");
        std::fs::write(&path_a, &events).unwrap();
        parse_claude_session(&path_a, &conn_a, "project", "2026-06-12T16:03:00Z").unwrap();

        // (B) index 17 events, then append the rest and index incrementally.
        let conn_b = memory_conn_with_project();
        let path_b = dir.join("b.jsonl");
        std::fs::write(&path_b, &events[..split]).unwrap();
        parse_claude_session(&path_b, &conn_b, "project", "2026-06-12T16:03:00Z").unwrap();
        let mid: i64 = conn_b
            .query_row(
                "SELECT message_count FROM cc_sessions WHERE jsonl_path = ?1",
                params![path_b.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            mid, 17,
            "first pass should index exactly the 17 complete lines"
        );
        std::fs::write(&path_b, &events).unwrap();
        parse_claude_session(&path_b, &conn_b, "project", "2026-06-12T16:04:00Z").unwrap();

        let a = index_snapshot(&conn_a, path_a.to_string_lossy().as_ref());
        let b = index_snapshot(&conn_b, path_b.to_string_lossy().as_ref());
        assert_eq!(a.0, b.0, "session totals/cursor/cost diverged");
        assert_eq!(a.1, b.1, "archive rows diverged");
        assert_eq!(a.2, b.2, "day buckets diverged");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_live_bootstrap_is_bounded_and_matches_full_index() {
        let dir = tempfile::tempdir().expect("session directory");
        let events = synth_codex_events(24);

        let full_path = dir.path().join("full.jsonl");
        std::fs::write(&full_path, &events).expect("full fixture");
        let full_conn = memory_conn_with_project();
        index_adapter_session(
            &CodexAdapter,
            &full_path,
            &full_conn,
            "project",
            "2026-06-12T09:00:00Z",
        )
        .expect("full index");

        let bounded_path = dir.path().join("bounded.jsonl");
        std::fs::write(&bounded_path, &events).expect("bounded fixture");
        let bounded_conn = memory_conn_with_project();
        index_adapter_session_bounded(
            &CodexAdapter,
            &bounded_path,
            &bounded_conn,
            "project",
            "2026-06-12T09:00:00Z",
            512,
        )
        .expect("first bounded pass");

        let file_size = events.len() as i64;
        let first_cursor: i64 = bounded_conn
            .query_row(
                "SELECT last_indexed_byte_offset FROM cc_sessions WHERE jsonl_path = ?1",
                params![bounded_path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("first cursor");
        assert!(first_cursor > 0, "bootstrap must make progress");
        assert!(
            first_cursor < file_size,
            "bootstrap must leave large transcripts for later passes"
        );

        for pass in 1..=100 {
            index_adapter_session_bounded(
                &CodexAdapter,
                &bounded_path,
                &bounded_conn,
                "project",
                &format!("2026-06-12T09:{:02}:00Z", pass % 60),
                512,
            )
            .expect("bounded continuation");
            let cursor: i64 = bounded_conn
                .query_row(
                    "SELECT last_indexed_byte_offset FROM cc_sessions WHERE jsonl_path = ?1",
                    params![bounded_path.to_string_lossy().as_ref()],
                    |row| row.get(0),
                )
                .expect("continuation cursor");
            if cursor == file_size {
                break;
            }
            assert!(pass < 100, "bounded index did not converge");
        }

        let full = index_snapshot(&full_conn, full_path.to_string_lossy().as_ref());
        let bounded = index_snapshot(&bounded_conn, bounded_path.to_string_lossy().as_ref());
        assert_eq!(bounded.0, full.0, "bounded totals/cursor/cost diverged");
        assert_eq!(bounded.1, full.1, "bounded archive rows diverged");
        assert_eq!(bounded.2, full.2, "bounded day buckets diverged");
    }

    #[test]
    fn oversized_live_row_is_hard_bounded_deferred_and_recovered() {
        let dir = tempfile::tempdir().expect("session directory");
        let path = dir.path().join("oversized.jsonl");
        let meta = json!({
            "timestamp": "2026-06-12T08:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "codex-oversized-session",
                "cwd": "/repo/codevetter",
                "model_provider": "openai"
            }
        })
        .to_string()
            + "\n";
        let oversized = json!({
            "timestamp": "2026-06-12T08:01:00Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "x".repeat(3 * 1024 * 1024)}]
            }
        })
        .to_string()
            + "\n";
        let usage = json!({
            "timestamp": "2026-06-12T08:01:01Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {"total_token_usage": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cached_input_tokens": 10
                }}
            }
        })
        .to_string()
            + "\n";
        std::fs::write(&path, format!("{meta}{oversized}{usage}")).expect("fixture");
        let file_size = std::fs::metadata(&path).expect("metadata").len() as i64;

        let inspected = read_complete_jsonl_chunk(&path, meta.len() as i64, 1024)
            .expect("bounded oversized read");
        assert!(inspected.text.is_empty());
        assert_eq!(inspected.consumed_bytes, 0);
        assert!(
            inspected.inspected_bytes <= 1024 + LIVE_TRANSCRIPT_DELIMITER_WINDOW_BYTES,
            "reader exceeded its hard allocation/read limit"
        );
        assert_eq!(inspected.deferred_oversized_offset, Some(meta.len() as i64));

        let conn = memory_conn_with_project();
        index_adapter_session_bounded(
            &CodexAdapter,
            &path,
            &conn,
            "project",
            "2026-06-12T09:00:00Z",
            1024,
        )
        .expect("metadata bootstrap");
        index_adapter_session_bounded(
            &CodexAdapter,
            &path,
            &conn,
            "project",
            "2026-06-12T09:00:10Z",
            1024,
        )
        .expect("oversized deferral");
        let cursor_after_deferral: i64 = conn
            .query_row(
                "SELECT last_indexed_byte_offset FROM cc_sessions WHERE jsonl_path = ?1",
                params![path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("deferred cursor");
        assert_eq!(cursor_after_deferral, meta.len() as i64);
        assert!(live_jsonl_row_is_deferred(
            path.to_string_lossy().as_ref(),
            cursor_after_deferral,
            file_size
        ));

        // A later live tick returns from the marker without moving the cursor.
        index_adapter_session_bounded(
            &CodexAdapter,
            &path,
            &conn,
            "project",
            "2026-06-12T09:00:20Z",
            1024,
        )
        .expect("remembered deferral");
        let cursor_after_retry: i64 = conn
            .query_row(
                "SELECT last_indexed_byte_offset FROM cc_sessions WHERE jsonl_path = ?1",
                params![path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .expect("retry cursor");
        assert_eq!(cursor_after_retry, cursor_after_deferral);

        // Unbounded maintenance ignores the live marker and completes exactly.
        index_adapter_session(
            &CodexAdapter,
            &path,
            &conn,
            "project",
            "2026-06-12T10:00:00Z",
        )
        .expect("maintenance recovery");
        let recovered: (i64, i64, i64) = conn
            .query_row(
                "SELECT last_indexed_byte_offset, message_count, total_input_tokens
                   FROM cc_sessions WHERE jsonl_path = ?1",
                params![path.to_string_lossy().as_ref()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("recovered session");
        assert_eq!(recovered, (file_size, 1, 100));
        assert!(!live_jsonl_row_is_deferred(
            path.to_string_lossy().as_ref(),
            file_size,
            file_size
        ));
    }

    #[test]
    fn short_partial_live_row_is_not_deferred() {
        let dir = tempfile::tempdir().expect("session directory");
        let path = dir.path().join("partial.jsonl");
        let partial = "{\"type\":\"response_item\"";
        std::fs::write(&path, partial).expect("partial fixture");
        let chunk = read_complete_jsonl_chunk(&path, 0, 1024).expect("partial read");
        assert!(chunk.text.is_empty());
        assert_eq!(chunk.inspected_bytes, partial.len());
        assert_eq!(chunk.deferred_oversized_offset, None);
    }

    #[test]
    fn deferred_live_row_cache_is_bounded() {
        let mut deferred = HashMap::new();
        for index in 0..(LIVE_DEFERRED_JSONL_MAX_ENTRIES + 32) {
            insert_bounded_deferred_row(
                &mut deferred,
                format!("/fixture/session-{index}.jsonl"),
                (index as i64, index as i64 + 1),
            );
        }
        assert_eq!(deferred.len(), LIVE_DEFERRED_JSONL_MAX_ENTRIES);
    }

    #[test]
    fn live_policy_is_versioned_local_and_recoverable() {
        let conn = memory_conn_with_project();
        queries::set_preference(&conn, "last_indexed_at", "2026-07-12T12:00:00Z").unwrap();
        let policy = live_session_evidence_policy(&conn).expect("policy");
        assert_eq!(policy.schema_version, 1);
        assert_eq!(policy.incremental_interval_secs, 10);
        assert_eq!(policy.full_index_recovery_interval_secs, 6 * 60 * 60);
        assert_eq!(
            policy.supported_incremental_adapters,
            ["claude-code", "codex"]
        );
        assert!(policy.local_only);
        assert!(policy.recovery.contains("byte_cursor"));
        assert_eq!(
            policy.last_full_indexed_at.as_deref(),
            Some("2026-07-12T12:00:00Z")
        );
    }

    #[test]
    fn live_transcript_catch_up_has_a_conservative_tick_budget() {
        const { assert!(LIVE_TRANSCRIPT_SESSION_BYTE_BUDGET <= 64 * 1024) };
        const { assert!(LIVE_TRANSCRIPT_TICK_BUDGET_MS <= 200) };
        assert_eq!(LIVE_CODEX_DISCOVERY_SESSION_BUDGET, 1);
    }

    #[test]
    fn partial_tail_is_preserved_until_the_line_completes() {
        let dir = std::env::temp_dir().join(format!("cv_partial_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let events = synth_claude_events(6);
        let path = dir.join("partial.jsonl");
        std::fs::write(&path, &events[..events.len() - 1]).unwrap();
        let conn = memory_conn_with_project();
        parse_claude_session(&path, &conn, "project", "2026-07-12T12:00:00Z").unwrap();
        let first = index_snapshot(&conn, path.to_string_lossy().as_ref());
        assert_eq!(
            first.0[0], 5,
            "unterminated sixth line must remain unconsumed"
        );

        std::fs::write(&path, &events).unwrap();
        parse_claude_session(&path, &conn, "project", "2026-07-12T12:00:10Z").unwrap();
        let completed = index_snapshot(&conn, path.to_string_lossy().as_ref());
        assert_eq!(completed.0[0], 6);
        assert_eq!(completed.1.len(), 6);
    }

    #[test]
    fn lock_skipped_tail_recovers_exactly_once_on_next_pass() {
        let dir = std::env::temp_dir().join(format!("cv_tail_lock_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let events = synth_claude_events(3);
        let split = events
            .match_indices('\n')
            .nth(1)
            .map(|(i, _)| i + 1)
            .unwrap();
        let path = dir.join("live.jsonl");
        std::fs::write(&path, &events[..split]).unwrap();
        let conn = memory_conn_with_project();
        parse_claude_session(&path, &conn, "project", &chrono::Utc::now().to_rfc3339()).unwrap();
        conn.execute(
            "UPDATE cc_sessions SET last_message = ?2 WHERE jsonl_path = ?1",
            params![
                path.to_string_lossy().as_ref(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
        std::fs::write(&path, &events).unwrap();

        let guard = FULL_INDEX_LOCK.lock().unwrap();
        let skipped = tail_live_transcript_sessions_inner(&conn, false).expect("non-blocking skip");
        assert_eq!(skipped.messages_indexed, 0);
        drop(guard);

        let recovered = tail_live_transcript_sessions_inner(&conn, false).expect("recovered tail");
        assert_eq!(recovered.sessions_tailed, 1);
        assert_eq!(recovered.messages_indexed, 1);
        let settled = tail_live_transcript_sessions_inner(&conn, false).expect("settled tail");
        assert_eq!(settled.messages_indexed, 0);
        let snapshot = index_snapshot(&conn, path.to_string_lossy().as_ref());
        assert_eq!(snapshot.1.len(), 3, "archive rows must remain exact-once");
    }

    #[test]
    fn file_shrink_falls_back_to_full_reparse() {
        let dir = std::env::temp_dir().join(format!("cv_shrink_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let events = synth_claude_events(20);
        let conn = memory_conn_with_project();
        let path = dir.join("s.jsonl");

        std::fs::write(&path, &events).unwrap();
        parse_claude_session(&path, &conn, "project", "2026-06-12T16:03:00Z").unwrap();

        // Rotate/truncate: rewrite a shorter file. Indexer must NOT append onto
        // stale rows — it falls back to a clean full reparse.
        let smaller = synth_claude_events(5);
        std::fs::write(&path, &smaller).unwrap();
        parse_claude_session(&path, &conn, "project", "2026-06-12T16:05:00Z").unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT message_count FROM cc_sessions WHERE jsonl_path = ?1",
                params![path.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        let arch: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_message_archive a
                 JOIN cc_sessions s ON s.id = a.session_id WHERE s.jsonl_path = ?1",
                params![path.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 5,
            "shrunk file should reflect 5 events, not appended"
        );
        assert_eq!(arch, 5, "archive should be rebuilt to 5 rows");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "perf bench; run with --ignored --nocapture"]
    fn bench_incremental_reindex_vs_full() {
        use std::time::Instant;
        let dir = std::env::temp_dir().join(format!("cv_incbench_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = synth_claude_events(80_000); // ~24 MB
        let path = dir.join("big.jsonl");
        std::fs::write(&path, &big).unwrap();

        let conn = memory_conn_with_project();
        let t0 = Instant::now();
        parse_claude_session(&path, &conn, "project", "2026-06-12T16:03:00Z").unwrap();
        let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;

        // Append ~4 KB and incrementally re-index (the new per-append cost).
        let mut grown = big.clone();
        grown.push_str(&synth_claude_events(12));
        std::fs::write(&path, &grown).unwrap();
        let t1 = Instant::now();
        parse_claude_session(&path, &conn, "project", "2026-06-12T16:04:00Z").unwrap();
        let inc_ms = t1.elapsed().as_secs_f64() * 1000.0;

        // Contrast: a fresh full reparse of the same file (the OLD per-append cost).
        let conn2 = memory_conn_with_project();
        let t2 = Instant::now();
        parse_claude_session(&path, &conn2, "project", "2026-06-12T16:05:00Z").unwrap();
        let full_ms = t2.elapsed().as_secs_f64() * 1000.0;

        eprintln!("\n=== incremental re-index vs full reparse (real indexer) ===");
        eprintln!(
            "file:                {:.1} MB",
            big.len() as f64 / 1_048_576.0
        );
        eprintln!("cold full index:     {cold_ms:.1} ms");
        eprintln!("full reparse:        {full_ms:.1} ms   (old behavior, every append)");
        eprintln!("incremental append:  {inc_ms:.3} ms   (new behavior, 4 KB tail)");
        eprintln!(
            "speedup:             {:.0}x\n",
            full_ms / inc_ms.max(f64::MIN_POSITIVE)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claude_indexer_uses_adapter_summary_for_session_upsert() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/claude-code.jsonl"
        ));
        let session = parse_claude_session(fixture, &conn, "project", "2026-06-12T16:03:00Z")
            .expect("claude session");

        assert_eq!(session.session_id, "claude-session-1");
        assert_eq!(session.messages_indexed, 3);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, git_branch, model_used, message_count,
                        total_input_tokens, total_output_tokens, cache_read_tokens,
                        cache_creation_tokens, compaction_count
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params![fixture.to_string_lossy().as_ref()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "claude-session-1");
        assert_eq!(session.1, "claude-code");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("main"));
        assert_eq!(session.4.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(session.5, 3);
        assert_eq!(session.6, 135);
        assert_eq!(session.7, 40);
        assert_eq!(session.8, 25);
        assert_eq!(session.9, 10);
        assert_eq!(session.10, 1);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["claude-session-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 3);

        let archived =
            queries::list_session_message_archive(&conn, "claude-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 3);
        assert_eq!(archived[0].adapter_id, "claude-code");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[2].kind, "compaction");
    }

    #[test]
    fn codex_indexer_uses_adapter_summary_for_session_upsert() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/codex.jsonl"
        ));
        let session = parse_codex_session(fixture, &conn, "project", "2026-06-12T16:03:00Z")
            .expect("codex session");

        assert_eq!(session.session_id, "codex-session-1");
        assert_eq!(session.messages_indexed, 2);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, git_branch, model_used, message_count,
                        total_input_tokens, total_output_tokens, cache_read_tokens
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params![fixture.to_string_lossy().as_ref()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "codex-session-1");
        assert_eq!(session.1, "codex");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("feature/adapter"));
        assert_eq!(session.4.as_deref(), Some("o3"));
        assert_eq!(session.5, 2);
        assert_eq!(session.6, 500);
        assert_eq!(session.7, 150);
        assert_eq!(session.8, 100);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["codex-session-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 2);

        let archived =
            queries::list_session_message_archive(&conn, "codex-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "codex");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[1].raw_type.as_deref(), Some("response_item"));
    }

    #[test]
    fn archive_backfill_repairs_existing_codex_session_rows() {
        let conn = memory_conn_with_project();
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/codex.jsonl"
        ));

        queries::upsert_session(
            &conn,
            &queries::SessionInput {
                id: "codex-session-1".to_string(),
                project_id: "project".to_string(),
                agent_type: Some("codex".to_string()),
                jsonl_path: Some(fixture.to_string_lossy().to_string()),
                git_branch: None,
                cwd: Some("/repo/codevetter".to_string()),
                cli_version: None,
                first_message: None,
                last_message: Some("2026-06-12T16:01:00Z".to_string()),
                message_count: Some(2),
                total_input_tokens: Some(500),
                total_output_tokens: Some(150),
                model_used: Some("o3".to_string()),
                slug: None,
                file_size_bytes: Some(123),
                indexed_at: Some("2026-06-12T16:03:00Z".to_string()),
                file_mtime: Some("2026-06-12T16:03:00Z".to_string()),
                cache_read_tokens: Some(100),
                cache_creation_tokens: Some(0),
                compaction_count: Some(0),
                estimated_cost_usd: Some(0.0),
            },
        )
        .expect("existing codex session");

        let candidates =
            queries::list_sessions_needing_archive_backfill(&conn, 10).expect("candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, "codex-session-1");

        let backfilled = backfill_missing_session_archives(&conn).expect("backfill");
        assert_eq!(backfilled, 1);

        let archived =
            queries::list_session_message_archive(&conn, "codex-session-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "codex");
        assert_eq!(archived[0].role.as_deref(), Some("user"));
        assert_eq!(archived[1].role.as_deref(), Some("assistant"));
    }

    #[test]
    fn cursor_adapter_summary_upserts_session_and_day_bucket() {
        let conn = memory_conn_with_project();
        let raw = include_str!("../../tests/fixtures/session_adapters/cursor.json");
        let summary = CursorAdapter.parse_raw("/cursor/state.vscdb#cursor-composer-1", raw);
        let indexed = upsert_adapter_summary_session(
            &conn,
            "project",
            summary,
            123,
            Some("2026-06-12T16:02:00Z".to_string()),
            "2026-06-12T16:03:00Z",
            None,
        )
        .expect("cursor upsert");

        assert_eq!(indexed.session_id, "cursor-composer-1");
        assert_eq!(indexed.messages_indexed, 2);
        let session = conn
            .query_row(
                "SELECT id, agent_type, cwd, model_used, slug, message_count,
                        total_input_tokens, total_output_tokens
                 FROM cc_sessions
                 WHERE jsonl_path = ?1",
                params!["/cursor/state.vscdb#cursor-composer-1"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .expect("session row");
        assert_eq!(session.0, "cursor-composer-1");
        assert_eq!(session.1, "cursor");
        assert_eq!(session.2.as_deref(), Some("/repo/codevetter"));
        assert_eq!(session.3.as_deref(), Some("cursor-small"));
        assert_eq!(session.4.as_deref(), Some("Fix checkout test"));
        assert_eq!(session.5, 2);
        assert_eq!(session.6, 0);
        assert_eq!(session.7, 0);

        let day_count: i64 = conn
            .query_row(
                "SELECT msg_count FROM cc_session_days WHERE session_id = ?1 AND day = ?2",
                params!["cursor-composer-1", "2026-06-12"],
                |row| row.get(0),
            )
            .expect("session day bucket");
        assert_eq!(day_count, 2);

        let archived =
            queries::list_session_message_archive(&conn, "cursor-composer-1", 10).expect("archive");
        assert_eq!(archived.len(), 2);
        assert_eq!(archived[0].adapter_id, "cursor");
        assert_eq!(
            archived[0].content_text.as_deref(),
            Some("Fix checkout test")
        );
        assert_eq!(archived[1].role.as_deref(), Some("assistant"));
    }

    #[test]
    fn production_adapter_run_stats_persist_source_health_row() {
        let conn = memory_conn_with_project();
        let mut stats = ProductionAdapterRunStats::new(
            "codex",
            "codex",
            vec!["/Users/me/.codex/sessions".to_string()],
            true,
        );
        stats.record_session(&IndexedAdapterSession {
            session_id: "session-a".to_string(),
            source_ref: "/Users/me/.codex/sessions/a.jsonl".to_string(),
            messages_indexed: 7,
            parse_warnings: vec!["missing cwd fallback used".to_string()],
        });
        stats.record_warning("/Users/me/.codex/sessions/b.jsonl", "not valid JSON");

        let id = persist_production_adapter_run(&conn, &stats, "2026-06-12T16:03:00Z")
            .expect("adapter run");
        assert!(!id.is_empty());

        let rows = queries::list_session_adapter_runs(&conn, None, 10).expect("adapter runs");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].adapter_id, "codex");
        assert_eq!(rows[0].agent_type.as_deref(), Some("codex"));
        assert_eq!(rows[0].source_roots, vec!["/Users/me/.codex/sessions"]);
        assert_eq!(
            rows[0].sample_source_paths,
            vec!["/Users/me/.codex/sessions/a.jsonl"]
        );
        assert_eq!(rows[0].sample_session_ids, vec!["session-a"]);
        assert_eq!(rows[0].sessions_indexed, 1);
        assert_eq!(rows[0].messages_indexed, 7);
        assert_eq!(
            rows[0].last_indexed_at.as_deref(),
            Some("2026-06-12T16:03:00Z")
        );
        assert!(rows[0]
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("missing cwd fallback used")));
        assert!(rows[0]
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("not valid JSON")));
        assert!(rows[0].supports_incremental);
    }

    #[test]
    fn grok_session_estimates_input_from_per_turn_context() {
        let dir = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/session_adapters/grok-session"
        ));
        let summary = parse_grok_session_dir(dir, "/repo/codevetter").expect("grok session parses");

        assert_eq!(summary.agent_type, "grok");
        assert_eq!(summary.adapter_id, "grok");
        assert_eq!(summary.stable_id.as_deref(), Some("grok-session-1"));
        assert_eq!(summary.model_used.as_deref(), Some("grok-build"));
        assert_eq!(summary.message_count, 4);
        // Input estimate = sum of the peak context size per turn:
        // turn 100 -> 1000, turn 200 -> 3000, turn 300 -> 5200 (max of 5200/5000).
        assert_eq!(summary.total_input_tokens, 9200);
        // Grok logs no cumulative output tokens; estimate from chat_history chars / 4.
        assert_eq!(summary.total_output_tokens, 4);
        assert_eq!(summary.cache_read_tokens, 0);
    }

    #[test]
    fn cursor_agent_estimates_input_from_resent_context() {
        let dir = std::env::temp_dir().join(format!("cv-cursor-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("store.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute("CREATE TABLE blobs (id TEXT, data BLOB)", [])
                .unwrap();
            let put = |s: Vec<u8>| {
                conn.execute("INSERT INTO blobs (id, data) VALUES ('x', ?1)", params![s])
                    .unwrap();
            };
            put(format!(r#"{{"role":"user","content":"{}"}}"#, "u".repeat(100)).into_bytes());
            put(format!(r#"{{"role":"assistant","content":"{}"}}"#, "a".repeat(40)).into_bytes());
            put(format!(r#"{{"role":"user","content":"{}"}}"#, "u".repeat(60)).into_bytes());
            put(format!(r#"{{"role":"assistant","content":"{}"}}"#, "a".repeat(40)).into_bytes());
            put(vec![0u8, 159, 146, 150]); // opaque binary blob — must be skipped
        }

        let (input_est, output_est, msgs, _model) =
            estimate_cursor_agent_session(&db_path).expect("estimate");
        std::fs::remove_dir_all(&dir).ok();

        // Re-sent context: turn 1 sees 100 prior chars (25 tok), turn 2 sees
        // 100+40+60=200 (50 tok) -> 75. Output: 40/4 + 40/4 = 20. Binary skipped.
        assert_eq!(input_est, 75);
        assert_eq!(output_est, 20);
        assert_eq!(msgs, 4);
    }

    #[test]
    fn percent_decode_path_restores_cwd() {
        assert_eq!(
            percent_decode_path("%2FUsers%2Fsarthak%2FDesktop%2Ffleet%2Freader"),
            "/Users/sarthak/Desktop/fleet/reader"
        );
        // Non-encoded input is returned unchanged.
        assert_eq!(percent_decode_path("plain-name"), "plain-name");
    }

    // Diagnostic (not a CI eval): runs the real indexer against a COPY of the
    // live DB twice. Pass 2 is steady state — it must skip ~everything and be
    // fast. Run with: cargo test --bin codevetter-desktop diag_live_index -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diag_codex_fix_cost() {
        // Runs the Codex token repair against a copy of the live DB and reports
        // before/after totals. Run with:
        // cargo test --bin codevetter-desktop diag_codex_fix_cost -- --ignored --nocapture
        let path = "/tmp/cv_live_copy.db";
        if !std::path::Path::new(path).exists() {
            eprintln!("SKIP: {path} not present");
            return;
        }
        let conn = Connection::open(path).expect("open");
        let total = |c: &Connection| -> f64 {
            c.query_row(
                "SELECT COALESCE(SUM(estimated_cost_usd),0) FROM cc_sessions",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0.0)
        };
        let codex = |c: &Connection| -> f64 {
            c.query_row("SELECT COALESCE(SUM(estimated_cost_usd),0) FROM cc_sessions WHERE agent_type='codex'", [], |r| r.get(0)).unwrap_or(0.0)
        };
        eprintln!(
            "BEFORE: total=${:.0} codex=${:.0}",
            total(&conn),
            codex(&conn)
        );
        fix_codex_token_totals(&conn);
        eprintln!(
            "AFTER:  total=${:.0} codex=${:.0}",
            total(&conn),
            codex(&conn)
        );
        let mut stmt = conn.prepare("SELECT agent_type, ROUND(estimated_cost_usd,2), total_input_tokens FROM cc_sessions ORDER BY estimated_cost_usd DESC LIMIT 5").unwrap();
        let rows: Vec<(String, f64, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for (a, c, t) in rows {
            eprintln!("  top: {a} ${c} ({t} input tok)");
        }
    }

    #[test]
    #[ignore]
    fn diag_mtime_skip_mismatch() {
        // For every session in the live copy, compare the STORED file_mtime with
        // what the indexer recomputes from the file on disk. Any mismatch on an
        // unchanged file means the mtime-skip can never fire → perpetual reparse.
        let path = "/tmp/cv_live_copy.db";
        if !std::path::Path::new(path).exists() {
            eprintln!("SKIP: {path} not present");
            return;
        }
        let conn = Connection::open(path).expect("open");
        let mut stmt = conn
            .prepare("SELECT jsonl_path, file_mtime FROM cc_sessions WHERE jsonl_path IS NOT NULL AND file_mtime IS NOT NULL")
            .unwrap();
        let rows: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        let mut checked = 0;
        let mut mismatch = 0;
        let mut shown = 0;
        for (p, stored) in &rows {
            let meta = match std::fs::metadata(p) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let recomputed = meta
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            checked += 1;
            if recomputed.as_deref() != Some(stored.as_str()) {
                mismatch += 1;
                if shown < 8 {
                    eprintln!("MISMATCH stored={stored:?} recomputed={recomputed:?}");
                    shown += 1;
                }
            }
        }
        eprintln!("checked={checked} mismatch={mismatch}");
    }

    #[test]
    #[ignore]
    fn diag_claude_dedup_dry_run() {
        // Dry-run the Claude usage-dedup backfill against a copy of the live DB
        // (cp codevetter.db /tmp/cv_dedup_dryrun.db) and print before/after.
        let path = "/tmp/cv_dedup_dryrun.db";
        if !std::path::Path::new(path).exists() {
            eprintln!("SKIP: {path} not present");
            return;
        }
        let conn = Connection::open(path).expect("open dry-run copy");
        schema::run_migrations(&conn).expect("migrate");
        let claude = |c: &Connection| -> (f64, i64) {
            c.query_row(
                "SELECT COALESCE(SUM(estimated_cost_usd),0), COALESCE(SUM(total_output_tokens),0)
                 FROM cc_sessions WHERE agent_type='claude-code'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        let (cost0, out0) = claude(&conn);
        let t0 = std::time::Instant::now();
        fix_claude_usage_dedup(&conn);
        let (cost1, out1) = claude(&conn);
        eprintln!(
            "claude BEFORE: ${cost0:.0} / {out0} out-tokens\nclaude AFTER:  ${cost1:.0} / {out1} out-tokens\nelapsed {:.1}s",
            t0.elapsed().as_secs_f64()
        );
    }

    #[test]
    #[ignore]
    fn diag_live_index_steady_state() {
        let path = "/tmp/cv_live_copy.db";
        if !std::path::Path::new(path).exists() {
            eprintln!("SKIP: {path} not present");
            return;
        }
        let conn = Connection::open(path).expect("open live copy");
        schema::run_migrations(&conn).expect("migrate");

        for pass in 1..=2 {
            let t0 = std::time::Instant::now();
            let s = run_full_index_summary_with_conn(&conn).expect("index pass");
            let dt = t0.elapsed();
            eprintln!(
                "PASS {pass}: {:?} in {:.1}s — indexed={} skipped={} msgs={} fts_rows={}",
                "ok",
                dt.as_secs_f64(),
                s.indexed_sessions,
                s.skipped_sessions,
                s.indexed_messages,
                s.archive_search_rows_indexed,
            );
        }
    }

    #[test]
    fn eval_skip_keys_on_byte_offset_not_mtime() {
        // The index-skip decision must depend ONLY on byte offset vs file size —
        // never on the file mtime string (whose nanoseconds drift between reads
        // and silently disabled the old skip, re-parsing 100s of MB every pass).
        let meta = |msgs: i64, offset: i64| queries::SessionMeta {
            id: "s".to_string(),
            // A deliberately "stale"/garbage mtime: it must not affect the result.
            file_mtime: Some("1999-01-01T00:00:00.000000001+00:00".to_string()),
            message_count: msgs,
            archived_message_count: 0, // some sessions legitimately archive nothing
            total_input_tokens: 0,
            total_output_tokens: 0,
            last_indexed_byte_offset: offset,
            last_usage_key: None,
        };

        // Cursor at EOF → SKIP, regardless of the mismatched mtime or zero archive.
        assert!(session_fully_indexed(&meta(5, 1000), 1000));
        // File grew (size > offset) → must re-index the appended tail.
        assert!(!session_fully_indexed(&meta(5, 1000), 2000));
        // Never cursored (offset 0) → must index once.
        assert!(!session_fully_indexed(&meta(5, 0), 0));
        // File shrank/rotated (size < offset) → must re-parse.
        assert!(!session_fully_indexed(&meta(5, 1000), 500));
        // No messages yet (quick-startup stub) → must do a full parse.
        assert!(!session_fully_indexed(&meta(0, 1000), 1000));
    }

    #[test]
    fn grok_turn_days_keep_first_valid_timestamp_attribution() {
        let turn_millis = 1_700_000_000_123;
        let mut turn_days = std::collections::BTreeMap::new();

        record_turn_local_day(&mut turn_days, turn_millis);
        let recorded = turn_days
            .get(&turn_millis)
            .expect("valid millisecond timestamp should be attributed")
            .clone();
        assert_eq!(recorded.len(), 10);

        turn_days.insert(turn_millis, "existing-day".to_string());
        record_turn_local_day(&mut turn_days, turn_millis);
        assert_eq!(turn_days.get(&turn_millis).unwrap(), "existing-day");

        record_turn_local_day(&mut turn_days, -1);
        assert!(!turn_days.contains_key(&-1));
    }

    #[test]
    fn eval_estimate_cost_uses_current_prices() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        // Opus 4.6+ is $5/$25 per 1M — NOT the old $15/$75. This guards against
        // a stale price table inflating the headline $ (the "$49K Claude" bug).
        assert!(near(
            estimate_cost("claude-opus-4-8", 1_000_000, 0, 0, 0),
            5.0
        ));
        assert!(near(
            estimate_cost("claude-opus-4-8", 0, 1_000_000, 0, 0),
            25.0
        ));
        // Cache reads bill at 0.1× input ($0.50/1M for Opus).
        assert!(near(
            estimate_cost("claude-opus-4-8", 1_000_000, 0, 1_000_000, 0),
            0.50
        ));
        // Haiku 4.5 is $1/$5 (not the old $0.25/$1.25).
        assert!(near(
            estimate_cost("claude-haiku-4-5-20251001", 1_000_000, 0, 0, 0),
            1.0
        ));
        // OpenAI o3 (codex) is $2/$8.
        assert!(near(estimate_cost("o3", 1_000_000, 0, 0, 0), 2.0));
        // GPT-5.5 (codex mid-2026) is $5/$30, cached input $0.50; the generic
        // gpt-5 family arm must NOT swallow it.
        assert!(near(estimate_cost("gpt-5.5", 1_000_000, 0, 0, 0), 5.0));
        assert!(near(estimate_cost("gpt-5.5", 0, 1_000_000, 0, 0), 30.0));
        assert!(near(
            estimate_cost("gpt-5.5", 1_000_000, 0, 1_000_000, 0),
            0.50
        ));
        assert!(near(estimate_cost("gpt-5", 1_000_000, 0, 0, 0), 1.25));
        // GPT-5.6 tiers (Jul 2026): Sol $5/$30 cached $0.50, Terra $2.50/$15,
        // Luna $1/$6 — none may fall through to the generic GPT-5 family arm
        // (5.6-sol previously booked at ~1/4 its real price that way).
        assert!(near(estimate_cost("gpt-5.6-sol", 1_000_000, 0, 0, 0), 5.0));
        assert!(near(estimate_cost("gpt-5.6-sol", 0, 1_000_000, 0, 0), 30.0));
        assert!(near(
            estimate_cost("gpt-5.6-sol", 1_000_000, 0, 1_000_000, 0),
            0.50
        ));
        assert!(near(
            estimate_cost("gpt-5.6-terra", 1_000_000, 0, 0, 0),
            2.5
        ));
        assert!(near(estimate_cost("gpt-5.6-luna", 0, 1_000_000, 0, 0), 6.0));
        // GPT-5.4 must beat the generic GPT-5 family arm.
        assert!(near(estimate_cost("gpt-5.4", 1_000_000, 0, 0, 0), 2.50));
        assert!(near(estimate_cost("gpt-5.4", 0, 1_000_000, 0, 0), 15.0));
        assert!(near(
            estimate_cost("gpt-5.4", 1_000_000, 0, 1_000_000, 0),
            0.25
        ));
        // GPT-5.4 mini beats both the full 5.4 and generic mini arms.
        assert!(near(
            estimate_cost("gpt-5.4-mini", 1_000_000, 0, 0, 0),
            0.75
        ));
        assert!(near(estimate_cost("gpt-5.4-mini", 0, 1_000_000, 0, 0), 4.5));
        assert!(near(
            estimate_cost("gpt-5.4-mini", 1_000_000, 0, 1_000_000, 0),
            0.08
        ));
        assert!(near(
            estimate_cost("gpt-5.3-codex", 1_000_000, 0, 0, 0),
            1.75
        ));
        assert!(near(
            estimate_cost("gpt-5.3-codex", 0, 1_000_000, 0, 0),
            14.0
        ));
        assert!(near(estimate_cost("gpt-5.5-mini", 0, 1_000_000, 0, 0), 2.0));
        // GLM-5.2 (Devin) is $1.40/$4.40, cached $0.26.
        assert!(near(estimate_cost("glm-5-2", 1_000_000, 0, 0, 0), 1.4));
        assert!(near(estimate_cost("glm-5-2", 0, 1_000_000, 0, 0), 4.4));
        assert!(near(estimate_cost("glm-5-2", 0, 0, 1_000_000, 0), 0.26));
    }

    #[test]
    fn devin_steady_state_skips_unchanged_message_payloads() {
        let source_dir = tempfile::tempdir().expect("source dir");
        let source_path = source_dir.path().join("sessions.db");
        let source = Connection::open(&source_path).expect("source db");
        source
            .execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    working_directory TEXT NOT NULL,
                    backend_type TEXT NOT NULL,
                    model TEXT NOT NULL,
                    agent_mode TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    last_activity_at INTEGER NOT NULL,
                    title TEXT
                 );
                 CREATE TABLE message_nodes (
                    row_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    node_id INTEGER NOT NULL,
                    parent_node_id INTEGER,
                    chat_message TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    metadata TEXT,
                    UNIQUE(session_id, node_id)
                 );
                 CREATE INDEX idx_message_nodes_session ON message_nodes(session_id);",
            )
            .expect("source schema");
        source
            .execute(
                "INSERT INTO sessions (
                    id, working_directory, backend_type, model, agent_mode,
                    created_at, last_activity_at, title
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "steady-session",
                    "/repo/codevetter",
                    "local",
                    "glm-5-2",
                    "default",
                    1_750_000_000_i64,
                    1_750_000_100_i64,
                    "Steady session"
                ],
            )
            .expect("session");
        let message = json!({
            "role": "assistant",
            "message_id": "message-1",
            "metadata": {
                "generation_model": "glm-5-2",
                "metrics": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cache_read_tokens": 30,
                    "cache_creation_tokens": 5
                }
            }
        })
        .to_string();
        for node_id in [1_i64, 2_i64] {
            source
                .execute(
                    "INSERT INTO message_nodes (
                        session_id, node_id, chat_message, created_at
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params!["steady-session", node_id, message, 1_750_000_050_i64],
                )
                .expect("duplicate message row");
        }

        let conn = memory_conn_with_project();
        let first = index_devin_sessions_from_path(&conn, &source_path).expect("first index");
        assert_eq!(first, (1, 1, 0));
        let usage: (i64, i64, i64) = conn
            .query_row(
                "SELECT total_input_tokens, total_output_tokens, message_count
                   FROM cc_sessions WHERE jsonl_path = 'devin:steady-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("indexed usage");
        assert_eq!(usage, (135, 20, 1));

        // If steady-state indexing still scans chat_message, SQLite's JSON
        // functions fail on this payload. The unchanged watermark must skip it.
        source
            .execute("UPDATE message_nodes SET chat_message = 'not-json'", [])
            .expect("replace payload");
        let second = index_devin_sessions_from_path(&conn, &source_path).expect("steady index");
        assert_eq!(second, (0, 0, 1));
    }

    // Diagnostic (not a CI eval): runs index_devin_sessions against the live
    // Devin sessions.db on this machine and reports the ingested rows. Run with:
    // cargo test --bin codevetter-desktop diag_devin_index -- --ignored --nocapture
    #[test]
    #[ignore]
    fn diag_devin_index() {
        if !resolve_devin_sessions_db().exists() {
            eprintln!("SKIP: no devin sessions.db on this machine");
            return;
        }
        let conn = memory_conn_with_project();
        let (indexed, messages, skipped) = index_devin_sessions(&conn).expect("devin index");
        eprintln!("pass 1: indexed={indexed} messages={messages} skipped={skipped}");

        let rows: Vec<(String, i64, i64, i64, f64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, message_count, total_input_tokens, total_output_tokens,
                            estimated_cost_usd
                     FROM cc_sessions WHERE agent_type='devin'
                     ORDER BY total_input_tokens DESC",
                )
                .unwrap();
            stmt.query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect()
        };
        for (id, msgs, input, output, cost) in &rows {
            eprintln!(
                "  devin session {id}: msgs={msgs} input={input} output={output} cost=${cost:.2}"
            );
        }
        assert!(indexed > 0, "expected at least one devin session indexed");
        assert!(
            rows.iter().any(|(_, _, input, _, _)| *input > 0),
            "expected non-zero input tokens"
        );

        // Pass 2: steady state — every session should be skipped (unchanged
        // last_activity_at), confirming the incremental skip works.
        let (_, _, skipped2) = index_devin_sessions(&conn).expect("devin index pass 2");
        eprintln!("pass 2: skipped={skipped2} (should equal pass-1 indexed)");
        assert_eq!(
            skipped2, indexed,
            "pass 2 should skip all unchanged sessions"
        );
    }
}
