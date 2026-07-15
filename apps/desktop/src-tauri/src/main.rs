// Prevent a console window from popping up on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use codevetter_desktop::{commands, db, DbState};
use std::sync::{Arc, Mutex};
use tauri::Manager;

const STARTUP_FULL_INDEX_DELAY_SECS: u64 = 6 * 60 * 60;
const PERIODIC_INDEX_INITIAL_DELAY_SECS: u64 = 6 * 60 * 60;
const PERIODIC_INDEX_INTERVAL_SECS: u64 = commands::history::FULL_INDEX_RECOVERY_INTERVAL_SECS;

/// Repair `PATH` for GUI launches.
///
/// When the app is started from Finder/Dock (not a terminal), macOS gives it a
/// bare `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`. Tools installed by Homebrew
/// (`/opt/homebrew/bin` on Apple Silicon, `/usr/local/bin` on Intel) and user
/// installs (`~/.local/bin`) are then invisible, so `StdCommand::new("gh")`
/// fails with "not found" and GitHub auth detection reports "not connected"
/// even though the user is fully signed in via the `gh` CLI in their terminal.
///
/// We unconditionally *prepend* the common install dirs that aren't already on
/// `PATH`. This is instant and can never hang (unlike sourcing a login shell),
/// and it fixes every `gh`/`git`/`curl` spawn at once.
fn repair_path_for_gui() {
    let existing = std::env::var("PATH").unwrap_or_default();
    let present: std::collections::HashSet<&str> =
        existing.split(':').filter(|s| !s.is_empty()).collect();

    let mut prefix: Vec<String> = Vec::new();
    let mut candidates = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
    ];
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/.local/bin"));
    }
    for dir in candidates {
        if !present.contains(dir.as_str()) && std::path::Path::new(&dir).is_dir() {
            prefix.push(dir);
        }
    }
    if prefix.is_empty() {
        return;
    }
    let new_path = if existing.is_empty() {
        prefix.join(":")
    } else {
        format!("{}:{}", prefix.join(":"), existing)
    };
    std::env::set_var("PATH", &new_path);
    log::info!("repair_path_for_gui: prepended {}", prefix.join(":"));
}

/// Drop the calling thread to macOS *background* QoS. The OS then schedules its
/// work on efficiency cores and throttles it hard whenever the user is doing
/// anything else — so the background indexer "feels like it isn't running" even
/// while it grinds through a large catch-up. No-op off macOS.
#[cfg(target_os = "macos")]
fn set_thread_background_qos() {
    extern "C" {
        fn pthread_set_qos_class_self_np(
            qos_class: std::os::raw::c_uint,
            relative_priority: std::os::raw::c_int,
        ) -> std::os::raw::c_int;
    }
    // QOS_CLASS_BACKGROUND = 0x09
    unsafe {
        pthread_set_qos_class_self_np(0x09, 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_thread_background_qos() {}

fn run_usage_maintenance(app_data_dir: std::path::PathBuf) {
    match db::init_db(app_data_dir) {
        Ok(conn) => {
            log::info!("Usage maintenance starting...");
            db::schema::purge_message_cruft_once(&conn);
            db::schema::purge_content_text_once(&conn);
            db::schema::purge_messages_to_buckets_once(&conn);
            // Repair Codex token totals corrupted by the old cumulative-add bug
            // (one-time), then refresh stored per-session $ cost if the price
            // table changed.
            commands::history::fix_codex_token_totals(&conn);
            // One-time per-model usage backfill (v1.1.100) — must precede the
            // cost recompute so multi-model sessions reprice from their split.
            commands::history::backfill_session_model_usage(&conn);
            // Relabel o3-defaulted Codex sessions from their turn_context rows
            // before cost recompute so rev-6+ pricing books corrected models.
            commands::history::backfill_codex_session_models(&conn);
            // One-time Claude usage dedup: re-scan on-disk transcripts counting
            // each API response's usage once (duplicate content-block lines
            // inflated Claude numbers ~2.2×). Rewrites totals + cost directly,
            // so ordering vs the pricing recompute below doesn't matter.
            commands::history::fix_claude_usage_dedup(&conn);
            commands::history::recompute_all_session_costs(&conn);
            log::info!("Usage maintenance done.");
        }
        Err(e) => log::error!("Usage maintenance DB init failed: {e}"),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // v1.1.85: GUI launches inherit a bare PATH that hides Homebrew's
            // `gh`/`git`, which broke GitHub auth detection. Repair it before
            // anything shells out.
            repair_path_for_gui();

            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");

            let conn = db::init_db(app_data_dir.clone()).expect("failed to initialize database");
            app.manage(DbState(Arc::new(Mutex::new(conn))));
            app.manage(commands::trex_watcher::WatcherHandles::new());
            app.manage(commands::resources::ResourceState::new());

            // v1.1.83: resume any T-Rex watchers that were enabled before the
            // last shutdown. Each enabled row spawns its own Tokio polling task.
            commands::trex_watcher::resume_enabled_watchers(&app.handle());

            // ── Trigger initial index on startup ─────────────────
            // Storage cleanup (one-time purge of cruft message rows) runs at
            // the end of this thread so it never races with the indexer for
            // the DB write lock. VACUUM is intentionally omitted here — it
            // takes minutes and holds an exclusive lock, freezing the UI.
            let bg_data_dir = app_data_dir;
            let bg_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("initial-index".into())
                .spawn(move || {
                    set_thread_background_qos();
                    log::info!("Starting quick index on startup...");
                    match run_initial_index(bg_data_dir.clone()) {
                        Ok(msg) => log::info!("Quick index complete: {msg}"),
                        Err(e) => log::error!("Quick index failed: {e}"),
                    }
                    run_usage_maintenance(bg_data_dir.clone());

                    // Keep app launch and first-click workflows responsive. The
                    // quick index gives Home usable data immediately; the full
                    // historical pass is maintenance work and should not
                    // compete with Add Project / Unpack during an active
                    // product session. Users can still trigger it manually.
                    std::thread::sleep(std::time::Duration::from_secs(
                        STARTUP_FULL_INDEX_DELAY_SECS,
                    ));
                    log::info!("Starting full index...");
                    match run_full_index(bg_data_dir.clone()) {
                        Ok(summary) => {
                            log::info!("Full index complete: {}", summary.log_message());
                            commands::history::emit_session_archive_updated(
                                &bg_handle, &summary,
                            );
                        }
                        Err(e) => log::error!("Full index failed: {e}"),
                    }

                    run_usage_maintenance(bg_data_dir);
                })
                .expect("failed to spawn initial-index thread");

            // ── Periodic re-index ─────────────
            // The startup thread does the first full index after the initial
            // UI window. Periodic passes are best-effort and must not queue
            // behind another index while foreground commands need SQLite.
            let periodic_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            let periodic_handle = app.handle().clone();

            std::thread::Builder::new()
                .name("periodic-index".into())
                .spawn(move || {
                    set_thread_background_qos();
                    std::thread::sleep(std::time::Duration::from_secs(
                        PERIODIC_INDEX_INITIAL_DELAY_SECS,
                    ));
                    loop {
                        match db::init_db(periodic_data_dir.clone()) {
                            Ok(conn) => {
                                match commands::history::try_run_full_index_summary_with_conn(
                                    &conn,
                                ) {
                                    Ok(Some(summary)) => {
                                        log::info!(
                                            "Periodic re-index complete: {}",
                                            summary.log_message()
                                        );
                                        commands::history::emit_session_archive_updated(
                                            &periodic_handle,
                                            &summary,
                                        );
                                    }
                                    Ok(None) => {
                                        log::debug!("Periodic re-index skipped: index already running");
                                    }
                                    Err(e) => log::error!("Periodic re-index failed: {e}"),
                                }
                            }
                            Err(e) => {
                                log::error!("Periodic re-index DB init failed: {e}");
                            }
                        }
                        // Maintenance cadence only. Full indexing is useful
                        // for archive completeness, but it must not compete
                        // with foreground repo work during normal app usage.
                        // Open sessions are kept fresh by the lightweight tail
                        // watcher below.
                        std::thread::sleep(std::time::Duration::from_secs(
                            PERIODIC_INDEX_INTERVAL_SECS,
                        ));
                    }
                })
                .expect("failed to spawn periodic-index thread");

            // ── Transcript tail watcher (every 10s) ─────────────
            // Re-index recently active Claude/Codex JSONL files incrementally so
            // open sessions show up in archive search without waiting for the
            // 5-minute full index pass.
            let tail_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            let tail_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("transcript-tail".into())
                .spawn(move || {
                    set_thread_background_qos();
                    std::thread::sleep(std::time::Duration::from_secs(
                        commands::history::LIVE_TRANSCRIPT_INITIAL_DELAY_SECS,
                    ));
                    // Grok/Cursor aren't transcript-tailable, so they only
                    // refreshed on the 5-min full index and lagged Claude/Codex.
                    // Refresh them every ~60s (every 6th 10s tick) — responsive
                    // without spamming the unpruned session_adapter_runs table
                    // (the Cursor indexers each write a run row per pass).
                    let mut tick: u64 = 0;
                    loop {
                        match db::init_db(tail_data_dir.clone()) {
                            Ok(conn) => {
                                let _ = conn.busy_timeout(std::time::Duration::from_millis(250));
                                match commands::history::tail_live_transcript_sessions_with_conn(
                                    &conn,
                                ) {
                                    Ok(summary) => {
                                        if summary.messages_indexed > 0 {
                                            log::info!(
                                                "Transcript tail indexed {} messages across {} sessions",
                                                summary.messages_indexed,
                                                summary.sessions_tailed
                                            );
                                            let archive_summary =
                                                commands::history::FullIndexSummary {
                                                    indexed_sessions: summary.sessions_tailed,
                                                    indexed_messages: summary.messages_indexed,
                                                    skipped_sessions: 0,
                                                    archive_search_rows_indexed: summary
                                                        .messages_indexed
                                                        as i64,
                                                    indexed_at: summary.tailed_at,
                                                };
                                            commands::history::emit_session_archive_updated(
                                                &tail_handle,
                                                &archive_summary,
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        log::debug!("Transcript tail pass failed: {error}");
                                    }
                                }

                                if tick
                                    % (commands::history::LIVE_SECONDARY_ADAPTER_INTERVAL_SECS
                                        / commands::history::LIVE_TRANSCRIPT_INTERVAL_SECS)
                                    == 0
                                {
                                    match commands::history::refresh_secondary_agents_with_conn(
                                        &conn,
                                    ) {
                                        Ok(summary) if summary.sessions_tailed > 0 => {
                                            log::info!(
                                                "Secondary-agent refresh updated {} Grok/Cursor sessions",
                                                summary.sessions_tailed
                                            );
                                            let archive_summary =
                                                commands::history::FullIndexSummary {
                                                    indexed_sessions: summary.sessions_tailed,
                                                    indexed_messages: summary.messages_indexed,
                                                    skipped_sessions: 0,
                                                    archive_search_rows_indexed: 0,
                                                    indexed_at: summary.tailed_at,
                                                };
                                            commands::history::emit_session_archive_updated(
                                                &tail_handle,
                                                &archive_summary,
                                            );
                                        }
                                        Ok(_) => {}
                                        Err(error) => {
                                            log::debug!("Secondary-agent refresh failed: {error}");
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                log::debug!("Transcript tail DB init failed: {error}");
                            }
                        }
                        tick = tick.wrapping_add(1);
                        std::thread::sleep(std::time::Duration::from_secs(
                            commands::history::LIVE_TRANSCRIPT_INTERVAL_SECS,
                        ));
                    }
                })
                .expect("failed to spawn transcript-tail thread");

            // Menu-bar tray removed (unused). Closing the window quits the app
            // normally — no hide-to-tray interception.

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Review
            commands::review::get_local_diff,
            commands::review::get_review,
            commands::review::delete_review,
            commands::review::set_finding_disposition,
            commands::review::list_reviews,
            commands::review::get_standards_pack_usage,
            commands::review::run_cli_review,
            commands::review::fix_findings,
            commands::review::merge_fix,
            commands::review::discard_fix,
            commands::review::revert_files,
            commands::review::revert_diff_hunk,
            commands::audience_validation::create_audience_validation_run,
            commands::audience_validation::add_audience_validation_response,
            commands::audience_validation::waive_audience_validation,
            commands::audience_validation::get_audience_validation,
            commands::taste::get_project_taste_verdict,
            commands::procedure_events::record_review_procedure_event,
            commands::procedure_events::list_review_procedure_events,
            commands::procedure_events::suggest_review_verification_commands,
            commands::procedure_events::run_review_verification_command,
            commands::procedure_events::cancel_review_verification_command,
            // Blast radius (graph-aware PR analysis)
            commands::blast_radius::analyze_blast_radius,
            // Sessions (used by Home for index stats)
            commands::sessions::list_sessions,
            commands::agent_memories::list_agent_memory_sources,
            commands::agent_memories::read_agent_memory_source,
            commands::agent_terminal::start_codex_agent_terminal,
            commands::agent_terminal::send_codex_agent_terminal_input,
            commands::agent_terminal::stop_codex_agent_terminal,
            commands::agent_terminal::resize_codex_agent_terminal,
            commands::agent_terminal::run_agent_terminal_command,
            commands::agent_terminal::get_codex_warp_plugin_status,
            commands::agent_terminal::install_codex_warp_plugin,
            commands::agent_terminal::list_codex_agent_terminals,
            commands::resources::get_resource_snapshot,
            commands::agent_memories::get_memory_file_git_diff,
            // History / indexer
            commands::history::trigger_index,
            commands::history::get_live_session_evidence_policy,
            commands::history::get_token_usage_stats,
            commands::history::get_agent_usage_breakdown,
            commands::history::get_agent_usage_by_day,
            commands::history::get_usage_by_model,
            // Repo activity intelligence (shown inside Repo -> Activity)
            // T-Rex sandbox (/review → Test branch)
            commands::sandbox::run_branch_sandbox,
            // SaaS Maker fleet wireup
            commands::saas_maker::get_saas_maker_status,
            commands::saas_maker::set_saas_maker_config,
            commands::saas_maker::list_saas_maker_projects,
            // v1.1.76: sign-in + identity + repo detect
            commands::saas_maker::start_saas_maker_signin,
            commands::saas_maker::poll_saas_maker_signin,
            commands::saas_maker::sign_out_of_saas_maker,
            commands::saas_maker::get_current_user,
            commands::saas_maker::detect_project_for_repo,
            // v1.1.78: AI acceleration
            // v1.1.79: DORA metrics
            // v1.1.81: real billing + agent observability + notifications
            commands::observability::get_billing_config,
            commands::observability::set_billing_config,
            commands::observability::get_billing_snapshots,
            commands::observability::get_agent_observability,
            commands::observability::get_webhook_config,
            commands::observability::set_webhook_config,
            commands::observability::send_notification,
            // v1.1.83: T-Rex v2 watcher — background PR scanner + GitHub status check
            commands::trex_watcher::start_trex_watcher,
            commands::trex_watcher::stop_trex_watcher,
            commands::trex_watcher::list_trex_watchers,
            commands::trex_watcher::list_trex_pr_runs,
            commands::trex_watcher::force_poll_trex_watcher,
            // Git
            commands::git::list_git_branches,
            commands::git::list_pull_requests,
            commands::git::check_github_auth,
            commands::git::sync_github_token,
            commands::git::get_repo_history_context,
            commands::git::read_raw_session_context,
            // GitHub PR & CI
            // Provider Accounts (Usage tab)
            commands::accounts::list_provider_accounts,
            commands::accounts::delete_provider_account,
            commands::accounts::check_account_usage,
            commands::accounts::check_live_usage,
            commands::accounts::list_provider_usage_ledger,
            commands::accounts::detect_provider_accounts,
            // Preferences
            commands::preferences::get_preference,
            commands::preferences::set_preference,
            // File operations (used by Review)
            commands::files::read_file_preview,
            commands::files::read_file_around_line,
            commands::files::open_in_app,
            // Setup
            commands::setup::check_prerequisites,
            // Agent Talks
            // Repo Unpacked
            commands::unpack::synthesize_unpack_report,
            commands::unpack::ask_unpack_report,
            commands::unpack::cancel_unpack_generation,
            commands::repo_workspace::list_repo_projects,
            commands::repo_workspace::register_repo_project,
            commands::repo_workspace::remove_repo_project,
            commands::repo_workspace::get_repo_project_git_status,
            commands::repo_workspace::save_unpack_scan_snapshot,
            commands::repo_workspace::save_intel_snapshot,
            commands::repo_workspace::list_repo_intel_reports,
            commands::repo_workspace::get_repo_intel_report,
            commands::repo_workspace::delete_repo_intel_report,
            commands::unpack::list_repo_unpack_reports,
            commands::unpack::get_repo_unpack_report,
            commands::unpack::compare_unpack_snapshot_commits,
            commands::unpack::get_unpack_outcome_evidence,
            commands::unpack::delete_repo_unpack_report,
            commands::unpack::export_repo_unpack_report,
            commands::graph_trust::import_external_graph_preview,
            commands::graph_trust::trace_repo_graph_path,
            commands::history_summary_graph::query_repo_history_graph,
            // Canonical structural repository graph
            commands::structural_graph::api::build_structural_graph,
            commands::structural_graph::api::cancel_structural_graph_build,
            commands::structural_graph::api::diff_structural_graph_snapshots,
            commands::structural_graph::api::explain_structural_graph_node,
            commands::structural_graph::api::export_structural_graph_json,
            commands::structural_graph::api::export_structural_graph_markdown,
            commands::structural_graph::api::find_structural_graph_path,
            commands::structural_graph::api::get_structural_graph,
            commands::structural_graph::api::get_structural_graph_adapters,
            commands::structural_graph::api::get_structural_graph_analysis,
            commands::structural_graph::api::get_structural_graph_community,
            commands::structural_graph::api::get_structural_graph_impact,
            commands::structural_graph::api::get_structural_graph_metadata,
            commands::structural_graph::api::get_structural_graph_neighbors,
            commands::structural_graph::api::get_structural_graph_overview,
            commands::structural_graph::api::get_structural_graph_status,
            commands::structural_graph::api::get_structural_graph_subgraph,
            commands::structural_graph::api::list_structural_graph_snapshots,
            commands::structural_graph::api::preview_node_link_structural_graph,
            commands::structural_graph::api::search_structural_graph,
            // Temporal repository graph
            commands::history_graph::api::backfill_history_graph,
            commands::history_graph::api::cancel_history_backfill,
            commands::history_graph::state::get_history_entity_evolution,
            commands::history_graph::api::get_history_graph_status,
            commands::history_graph::api::explain_history_entity,
            commands::history_graph::api::add_history_annotation,
            commands::history_graph::api::list_history_annotations,
            commands::history_graph::state::get_history_structural_delta,
            commands::history_graph::state::get_history_structural_state,
            commands::history_graph::api::get_history_timeline,
            commands::history_evidence::service::get_history_evidence_adapters,
            commands::history_evidence::service::import_history_evidence_export,
            commands::history_query::service::get_history_causal_trace,
            // Repository-scoped local MCP access
            commands::mcp_access::get_mcp_repository_settings,
            commands::mcp_access::set_mcp_repository_enabled,
            commands::mcp_access::clear_mcp_access_audit,
            // Unpack deep graph (call-graph indexing)
            commands::unpack_deep_graph::unpack_deep_graph_status,
            commands::unpack_deep_graph::unpack_deep_graph_analyze,
            commands::unpack_deep_graph::unpack_deep_graph_cancel_analyze,
            commands::unpack_deep_graph::unpack_deep_graph_symbol_context,
            commands::unpack_deep_graph::unpack_deep_graph_symbol_impact,
            commands::unpack_deep_graph::unpack_deep_graph_query,
            commands::unpack_deep_graph::unpack_deep_graph_detect_changes,
            // Synthetic user QA
            commands::synthetic_qa::run_synthetic_qa,
            commands::synthetic_qa::discover_playwright_specs,
            commands::synthetic_qa::record_synthetic_qa_run,
            commands::synthetic_qa::list_synthetic_qa_runs,
            // Live browser agent (drives real Chrome via chromiumoxide)
            #[cfg(feature = "browser-agent")]
            commands::agent::agent_run_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Run a lightweight startup index using its own database connection.
fn run_initial_index(app_data_dir: std::path::PathBuf) -> Result<String, String> {
    use db::queries;

    let conn = db::init_db(app_data_dir).map_err(|e| e.to_string())?;

    let all_bases = resolve_all_claude_projects_dirs();

    let project_dirs: Vec<_> = all_bases
        .iter()
        .filter(|b| b.exists())
        .flat_map(|b| std::fs::read_dir(b).ok().into_iter())
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .collect();

    if project_dirs.is_empty() {
        return Ok("No Claude project directories found".to_string());
    }

    let mut indexed_sessions = 0u64;
    let mut skipped = 0u64;

    for project_entry in &project_dirs {
        let project_path = project_entry.path();
        let project_dir_name = project_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let display_name = resolve_project_display_name(&project_dir_name);
        let dir_path_str = project_path.to_string_lossy().to_string();

        let project_id = queries::get_project_id_by_dir(&conn, &dir_path_str)
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let now = chrono::Utc::now().to_rfc3339();

        queries::upsert_project(
            &conn,
            &queries::ProjectInput {
                id: project_id.clone(),
                display_name,
                dir_path: dir_path_str,
                session_count: None,
                last_activity: Some(now.clone()),
                created_at: now.clone(),
            },
        )
        .map_err(|e| e.to_string())?;

        let jsonl_files = walkdir(&project_path, "jsonl");

        for jsonl_path in &jsonl_files {
            let jsonl_path_str = jsonl_path.to_string_lossy().to_string();
            let file_meta = std::fs::metadata(jsonl_path).ok();
            let file_mtime_str = file_meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());

            if let Ok(Some(existing)) = queries::get_session_by_jsonl_path(&conn, &jsonl_path_str) {
                if existing.file_mtime.as_deref() == file_mtime_str.as_deref() {
                    skipped += 1;
                    continue;
                }
            }

            let (session_id, meta) = quick_parse_session_meta(jsonl_path);

            queries::upsert_session(
                &conn,
                &queries::SessionInput {
                    id: session_id,
                    project_id: project_id.clone(),
                    agent_type: Some("claude-code".to_string()),
                    jsonl_path: Some(jsonl_path_str),
                    git_branch: meta.git_branch,
                    cwd: meta.cwd,
                    cli_version: meta.version,
                    first_message: meta.first_timestamp,
                    last_message: None,
                    message_count: None,
                    total_input_tokens: None,
                    total_output_tokens: None,
                    model_used: meta.model,
                    slug: meta.slug,
                    file_size_bytes: None,
                    indexed_at: None,
                    file_mtime: None,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    compaction_count: None,
                    estimated_cost_usd: None,
                },
            )
            .map_err(|e| e.to_string())?;

            indexed_sessions += 1;
        }

        let session_count = jsonl_files.len() as i64;
        conn.execute(
            "UPDATE cc_projects SET session_count = ?2 WHERE id = ?1",
            rusqlite::params![project_id, session_count],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(format!(
        "projects={}, indexed={}, skipped={}",
        project_dirs.len(),
        indexed_sessions,
        skipped
    ))
}

fn run_full_index(
    app_data_dir: std::path::PathBuf,
) -> Result<commands::history::FullIndexSummary, String> {
    use commands::history;
    let conn = db::init_db(app_data_dir).map_err(|e| e.to_string())?;
    history::run_full_index_summary_with_conn(&conn)
}

struct QuickMeta {
    version: Option<String>,
    git_branch: Option<String>,
    cwd: Option<String>,
    slug: Option<String>,
    model: Option<String>,
    first_timestamp: Option<String>,
}

fn quick_parse_session_meta(path: &std::path::Path) -> (String, QuickMeta) {
    use commands::session_adapters::{ClaudeCodeAdapter, SessionSourceAdapter};
    use std::io::BufRead;

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            return (
                uuid::Uuid::new_v4().to_string(),
                QuickMeta {
                    version: None,
                    git_branch: None,
                    cwd: None,
                    slug: None,
                    model: None,
                    first_timestamp: None,
                },
            );
        }
    };

    let reader = std::io::BufReader::new(file);
    let raw = reader
        .lines()
        .take(10)
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join("\n");
    let summary = ClaudeCodeAdapter.parse_raw(&path.to_string_lossy(), &raw);

    (
        summary
            .stable_id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        QuickMeta {
            version: summary.cli_version,
            git_branch: summary.git_branch,
            cwd: summary.cwd,
            slug: summary.slug,
            model: summary.model_used,
            first_timestamp: summary.first_timestamp,
        },
    )
}

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

fn resolve_project_display_name(dir_name: &str) -> String {
    let trimmed = dir_name.trim_start_matches('-');
    if trimmed.is_empty() {
        return dir_name.to_string();
    }

    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();

    if !home.is_empty() {
        let home_encoded = home.trim_start_matches('/').replace('/', "-");
        if let Some(remainder) = trimmed.strip_prefix(&home_encoded) {
            let remainder = remainder.trim_start_matches('-');
            if remainder.is_empty() {
                return dir_name.to_string();
            }
            let parts: Vec<&str> = remainder.split('-').collect();
            let mut current_dir = std::path::PathBuf::from(&home);
            let mut consumed = 0usize;
            for start in 0..parts.len() {
                let candidate = parts[start];
                let test_path = current_dir.join(candidate);
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

    let reconstructed = trimmed.replace('-', "/");
    std::path::Path::new(&reconstructed)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| dir_name.to_string())
}

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
