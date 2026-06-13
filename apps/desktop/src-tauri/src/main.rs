// Prevent a console window from popping up on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod talk;

use std::sync::{Arc, Mutex};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;

/// Shared database state accessible from every Tauri command via
/// `tauri::State<DbState>`.
#[derive(Clone)]
pub struct DbState(pub Arc<Mutex<rusqlite::Connection>>);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");

            let conn = db::init_db(app_data_dir.clone()).expect("failed to initialize database");
            app.manage(DbState(Arc::new(Mutex::new(conn))));

            // ── Lightweight tray usage refresh ──────────────────
            // Keep the menu-bar usage indicator alive without walking Claude,
            // Codex, or Cursor transcript stores in the background. Full
            // session indexing is intentionally explicit via `trigger_index`;
            // idle "usage only" mode should be a cheap SQLite aggregate read.
            let periodic_data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            let periodic_handle = app.handle().clone();

            std::thread::Builder::new()
                .name("usage-tray-refresh".into())
                .spawn(move || {
                    loop {
                        match db::init_db(periodic_data_dir.clone()) {
                            Ok(conn) => {
                                if let Ok(stats) = crate::db::queries::get_token_usage_stats(&conn)
                                {
                                    let text = format_tokens_compact(stats.today);
                                    if let Some(tray) = periodic_handle.tray_by_id("main") {
                                        let _ = tray.set_title(Some(&text));
                                        let _ = tray.set_tooltip(Some(&format!(
                                            "CodeVetter\nToday: {text}"
                                        )));
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("Usage tray refresh DB init failed: {e}");
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_secs(120));
                    }
                })
                .expect("failed to spawn usage-tray-refresh thread");

            // ── Menu-bar tray icon ───────────────────────────────
            // Surfaces token-usage stats next to volume/battery on macOS.
            // Frontend pushes a compact string via `set_tray_text` whenever
            // the dashboard polls (every 60s).
            let show = MenuItemBuilder::with_id("show", "Open CodeVetter").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().expect("default icon").clone())
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .tooltip("CodeVetter")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Intercept window close (X button): hide instead of quit so the
            // tray icon stays alive and the user can reopen via "Open CodeVetter".
            if let Some(window) = app.get_webview_window("main") {
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        let _ = w.hide();
                        api.prevent_close();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Review
            commands::review::get_local_diff,
            commands::review::save_review,
            commands::review::get_review,
            commands::review::list_reviews,
            commands::review::run_cli_review,
            commands::review::fix_findings,
            commands::review::merge_fix,
            commands::review::discard_fix,
            commands::review::revert_files,
            commands::review::revert_diff_hunk,
            commands::procedure_events::record_review_procedure_event,
            commands::procedure_events::list_review_procedure_events,
            commands::procedure_events::suggest_review_verification_commands,
            commands::procedure_events::run_review_verification_command,
            commands::procedure_events::cancel_review_verification_command,
            // Blast radius (graph-aware PR analysis)
            commands::blast_radius::analyze_blast_radius,
            // Sessions (used by Home for index stats)
            commands::sessions::list_sessions,
            commands::sessions::list_session_message_archive,
            commands::sessions::search_session_message_archive,
            commands::sessions::merge_projects,
            commands::session_intelligence::get_ai_session_scorecard,
            commands::session_intelligence::list_ai_session_adapter_runs,
            // History / indexer
            commands::history::trigger_index,
            commands::history::get_index_stats,
            commands::history::get_token_usage_stats,
            commands::history::detect_cursor,
            // Git
            commands::git::list_git_branches,
            commands::git::get_git_remote_info,
            commands::git::list_pull_requests,
            commands::git::check_github_auth,
            commands::git::sync_github_token,
            commands::git::get_git_changed_files,
            commands::git::get_repo_history_context,
            commands::git::read_raw_session_context,
            // GitHub PR & CI
            commands::github_ops::create_pull_request,
            commands::github_ops::list_pull_requests_for_repo,
            commands::github_ops::get_pull_request,
            commands::github_ops::merge_pull_request,
            commands::github_ops::list_ci_checks,
            commands::github_ops::rerun_failed_checks,
            // Provider Accounts (Usage tab)
            commands::accounts::list_provider_accounts,
            commands::accounts::create_provider_account,
            commands::accounts::update_provider_account,
            commands::accounts::delete_provider_account,
            commands::accounts::check_account_usage,
            commands::accounts::check_live_usage,
            commands::accounts::detect_provider_accounts,
            // Preferences
            commands::preferences::get_preference,
            commands::preferences::set_preference,
            // File operations (used by Review)
            commands::files::list_directory_tree,
            commands::files::read_file_preview,
            commands::files::read_file_around_line,
            commands::files::open_in_app,
            // Setup
            commands::setup::check_prerequisites,
            // Agent Talks
            commands::talks::get_talk,
            commands::talks::list_project_talks,
            commands::talks::get_latest_talk,
            // Tray
            commands::tray::set_tray_text,
            commands::tray::set_tray_menu,
            commands::tray::send_tray_notification,
            // Repo Unpacked
            commands::unpack::scan_repo_inventory,
            commands::unpack::generate_unpack_report,
            commands::unpack::list_repo_unpack_reports,
            commands::unpack::get_repo_unpack_report,
            commands::unpack::delete_repo_unpack_report,
            commands::unpack::export_repo_unpack_report,
            commands::unpack::import_repo_graph_json,
            // Synthetic user QA
            commands::synthetic_qa::run_synthetic_qa,
            commands::synthetic_qa::discover_playwright_specs,
            commands::synthetic_qa::record_synthetic_qa_run,
            commands::synthetic_qa::list_synthetic_qa_runs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn format_tokens_compact(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}
