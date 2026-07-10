//! T-Rex v2 — background watcher that polls open PRs on a watched repo and
//! runs the T-Rex sandbox (commands::sandbox::run_branch_sandbox_inner)
//! whenever a PR's head SHA changes. Each run also posts a GitHub commit
//! status check under context `codevetter/t-rex`, so the PR page shows the
//! verdict alongside CI.
//!
//! State lives in two SQLite tables:
//!   - `trex_watchers`  — per-repo config + last_polled_at + last_error
//!   - `trex_pr_runs`   — append-only history of runs (used to detect SHA churn)
//!
//! The Tokio task per watcher holds an in-memory in-flight set so two ticks
//! can't kick off the same PR sandbox concurrently. State persists across
//! app restarts; enabled watchers auto-resume in `resume_enabled_watchers`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::async_runtime::{spawn as runtime_spawn, JoinHandle};
use tauri::{AppHandle, Manager, State};
use tokio::process::Command;
use tokio::sync::oneshot;

use crate::commands::sandbox::{run_branch_sandbox_inner, SandboxOptions, SandboxRunInput};
use crate::DbState;

const PREF_GITHUB_TOKEN: &str = "github_token";
const STATUS_CONTEXT: &str = "codevetter/t-rex";
const MIN_INTERVAL_SECS: u64 = 60;
const DEFAULT_INTERVAL_SECS: u64 = 300;
const MAX_PRS_PER_TICK: usize = 10;

// ─── State container ────────────────────────────────────────────────────────

pub struct WatcherHandles(Mutex<HashMap<String, WatcherSlot>>);

struct WatcherSlot {
    handle: JoinHandle<()>,
    cancel: oneshot::Sender<()>,
}

impl WatcherHandles {
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

// ─── Public types (mirrored in TS) ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrexWatcher {
    pub repo_path: String,
    pub interval_secs: u64,
    pub enabled: bool,
    pub base_branch: Option<String>,
    pub last_polled_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrexPrRun {
    pub id: String,
    pub repo_path: String,
    pub pr_number: i64,
    pub head_sha: String,
    pub verdict: String,
    pub confidence: f64,
    pub summary: String,
    pub status_state: Option<String>,
    pub status_error: Option<String>,
    pub duration_ms: i64,
    pub ran_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartTrexWatcherInput {
    pub repo_path: String,
    pub interval_secs: Option<u64>,
    pub base_branch: Option<String>,
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_trex_watcher(
    app: AppHandle,
    db: State<'_, DbState>,
    handles: State<'_, WatcherHandles>,
    input: StartTrexWatcherInput,
) -> Result<TrexWatcher, String> {
    let interval = input
        .interval_secs
        .unwrap_or(DEFAULT_INTERVAL_SECS)
        .max(MIN_INTERVAL_SECS);

    upsert_watcher_row(
        &db,
        &input.repo_path,
        interval,
        true,
        input.base_branch.as_deref(),
    )?;
    spawn_watcher_task(
        &app,
        &handles,
        &input.repo_path,
        interval,
        input.base_branch.clone(),
    );
    read_watcher_row(&db, &input.repo_path)?
        .ok_or_else(|| "watcher row missing after upsert".to_string())
}

#[tauri::command]
pub async fn stop_trex_watcher(
    db: State<'_, DbState>,
    handles: State<'_, WatcherHandles>,
    repo_path: String,
) -> Result<(), String> {
    set_watcher_enabled(&db, &repo_path, false)?;
    if let Ok(mut map) = handles.0.lock() {
        if let Some(slot) = map.remove(&repo_path) {
            let _ = slot.cancel.send(());
            slot.handle.abort();
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_trex_watchers(db: State<'_, DbState>) -> Result<Vec<TrexWatcher>, String> {
    list_watchers(&db)
}

#[tauri::command]
pub async fn list_trex_pr_runs(
    db: State<'_, DbState>,
    repo_path: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<TrexPrRun>, String> {
    list_pr_runs(&db, repo_path.as_deref(), limit.unwrap_or(50))
}

#[tauri::command]
pub async fn force_poll_trex_watcher(
    app: AppHandle,
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<u32, String> {
    let row = read_watcher_row(&db, &repo_path)?
        .ok_or_else(|| format!("no watcher registered for {repo_path}"))?;
    let in_flight = Arc::new(Mutex::new(HashSet::<i64>::new()));
    let kicked = tick_once(&app, &db_state_from_app(&app), &row, &in_flight).await?;
    Ok(kicked)
}

// ─── Startup resume ─────────────────────────────────────────────────────────

/// Called from `main.rs::setup` after the DB is initialized. Re-spawns a
/// watcher task for every row in `trex_watchers` where `enabled = 1`.
pub fn resume_enabled_watchers(app: &AppHandle) {
    let db = app.state::<DbState>();
    let handles = app.state::<WatcherHandles>();
    let rows = match list_watchers(&db) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[trex-watcher] resume: list_watchers failed: {e}");
            return;
        }
    };
    for w in rows.into_iter().filter(|w| w.enabled) {
        spawn_watcher_task(
            app,
            &handles,
            &w.repo_path,
            w.interval_secs,
            w.base_branch.clone(),
        );
    }
}

// ─── Watcher task ───────────────────────────────────────────────────────────

fn spawn_watcher_task(
    app: &AppHandle,
    handles: &State<'_, WatcherHandles>,
    repo_path: &str,
    interval_secs: u64,
    base_branch: Option<String>,
) {
    if let Ok(map) = handles.0.lock() {
        if map.contains_key(repo_path) {
            log::info!("[trex-watcher] {repo_path} already running");
            return;
        }
    }
    let (tx, rx) = oneshot::channel::<()>();
    let app_clone = app.clone();
    let repo_owned = repo_path.to_string();
    let base_owned = base_branch.clone();
    let interval = interval_secs.max(MIN_INTERVAL_SECS);

    let task = runtime_spawn(async move {
        let in_flight = Arc::new(Mutex::new(HashSet::<i64>::new()));
        let mut rx = rx;
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        // Skip the immediate first tick from interval; we want a brief delay
        // so a freshly-started watcher doesn't slam the GitHub API at boot.
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = &mut rx => {
                    log::info!("[trex-watcher] {repo_owned}: shutdown signal");
                    return;
                }
                _ = ticker.tick() => {
                    let db = db_state_from_app(&app_clone);
                    let row = match read_watcher_row(&db, &repo_owned) {
                        Ok(Some(r)) if r.enabled => r,
                        Ok(_) => {
                            log::info!("[trex-watcher] {repo_owned}: disabled, exiting");
                            return;
                        }
                        Err(e) => {
                            log::warn!("[trex-watcher] {repo_owned}: read_watcher: {e}");
                            continue;
                        }
                    };
                    let row = TrexWatcher { base_branch: row.base_branch.or(base_owned.clone()), ..row };
                    match tick_once(&app_clone, &db, &row, &in_flight).await {
                        Ok(n) => log::debug!("[trex-watcher] {repo_owned}: tick kicked {n} runs"),
                        Err(e) => log::warn!("[trex-watcher] {repo_owned}: tick error: {e}"),
                    }
                }
            }
        }
    });

    if let Ok(mut map) = handles.0.lock() {
        map.insert(
            repo_path.to_string(),
            WatcherSlot {
                handle: task,
                cancel: tx,
            },
        );
    }
}

async fn tick_once(
    app: &AppHandle,
    db: &DbState,
    watcher: &TrexWatcher,
    in_flight: &Arc<Mutex<HashSet<i64>>>,
) -> Result<u32, String> {
    set_last_polled(db, &watcher.repo_path)?;
    let prs = list_open_prs(&watcher.repo_path).await?;
    let mut kicked = 0;

    for pr in prs.into_iter().take(MAX_PRS_PER_TICK) {
        let pr_number = pr.number;
        let head_sha = pr.head_sha;
        let head_ref = pr.head_ref.clone();

        // Skip if a previous tick already kicked this PR and it's still running.
        if let Ok(mut s) = in_flight.lock() {
            if s.contains(&pr_number) {
                continue;
            }
            s.insert(pr_number);
        }

        let last = latest_pr_run_sha(db, &watcher.repo_path, pr_number)?;
        if last.as_deref() == Some(head_sha.as_str()) {
            if let Ok(mut s) = in_flight.lock() {
                s.remove(&pr_number);
            }
            continue;
        }

        kicked += 1;

        let app_c = app.clone();
        let db_c = clone_db_state(db);
        let repo_path_c = watcher.repo_path.clone();
        let base_c = watcher.base_branch.clone();
        let in_flight_c = in_flight.clone();

        runtime_spawn(async move {
            let token = read_github_token(&db_c);
            let remote = remote_owner_repo(&repo_path_c).await.ok();
            if let (Some(tok), Some((owner, repo))) = (token.as_deref(), remote.as_ref()) {
                let _ = post_status(
                    tok,
                    owner,
                    repo,
                    &head_sha,
                    "pending",
                    "T-Rex sandbox running…",
                    None,
                )
                .await;
            }

            let started = std::time::Instant::now();
            let input = SandboxRunInput {
                repo_path: repo_path_c.clone(),
                branch: head_ref,
                base_branch: base_c,
                review_id: None,
                options: SandboxOptions::default(),
            };
            let run = run_branch_sandbox_inner(app_c.clone(), &db_c, input).await;
            let duration_ms = started.elapsed().as_millis() as i64;

            let (verdict, confidence, summary, error) = match &run {
                Ok(r) => (r.verdict.clone(), r.confidence, r.summary.clone(), None),
                Err(e) => (
                    "BLOCK".to_string(),
                    0.0,
                    "T-Rex sandbox failed to run".to_string(),
                    Some(e.clone()),
                ),
            };

            let (state, status_err) = match (token.as_deref(), remote.as_ref()) {
                (Some(tok), Some((owner, repo))) => {
                    let gh_state = verdict_to_gh_state(&verdict);
                    let desc = truncate_for_status(&summary);
                    let res = post_status(tok, owner, repo, &head_sha, gh_state, &desc, None).await;
                    match res {
                        Ok(_) => (Some(gh_state.to_string()), None),
                        Err(e) => (None, Some(e)),
                    }
                }
                _ => (
                    None,
                    Some("missing github_token or remote — status not posted".into()),
                ),
            };

            let _ = insert_pr_run(
                &db_c,
                &TrexPrRun {
                    id: uuid::Uuid::new_v4().to_string(),
                    repo_path: repo_path_c,
                    pr_number,
                    head_sha,
                    verdict,
                    confidence,
                    summary: error.clone().unwrap_or(summary),
                    status_state: state,
                    status_error: status_err,
                    duration_ms,
                    ran_at: chrono::Utc::now().to_rfc3339(),
                },
            );

            if let Ok(mut s) = in_flight_c.lock() {
                s.remove(&pr_number);
            }
        });
    }
    Ok(kicked)
}

// ─── PR enumeration (gh CLI) ────────────────────────────────────────────────

struct OpenPr {
    number: i64,
    head_ref: String,
    head_sha: String,
}

async fn list_open_prs(repo_path: &str) -> Result<Vec<OpenPr>, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            "number,headRefName,headRefOid",
            "--limit",
            "30",
        ])
        .current_dir(repo_path)
        .output()
        .await
        .map_err(|e| format!("gh pr list: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "gh pr list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let v: Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("parse gh pr list: {e}"))?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let number = item.get("number").and_then(|x| x.as_i64()).unwrap_or(0);
        let head_ref = item
            .get("headRefName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let head_sha = item
            .get("headRefOid")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if number > 0 && !head_ref.is_empty() && !head_sha.is_empty() {
            out.push(OpenPr {
                number,
                head_ref,
                head_sha,
            });
        }
    }
    Ok(out)
}

async fn remote_owner_repo(repo_path: &str) -> Result<(String, String), String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .await
        .map_err(|e| format!("git remote: {e}"))?;
    if !output.status.success() {
        return Err("no origin remote".into());
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_owner_repo(&url).ok_or_else(|| format!("could not parse owner/repo from {url}"))
}

fn parse_owner_repo(url: &str) -> Option<(String, String)> {
    // Accept both git@github.com:owner/repo(.git) and https://github.com/owner/repo(.git)
    let stripped = url.trim_end_matches('/').trim_end_matches(".git");
    let tail = if let Some(rest) = stripped.strip_prefix("git@github.com:") {
        rest.to_string()
    } else if let Some(idx) = stripped.find("github.com/") {
        stripped[idx + "github.com/".len()..].to_string()
    } else {
        return None;
    };
    let mut parts = tail.splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        None
    } else {
        Some((owner, repo))
    }
}

// ─── GitHub status check ────────────────────────────────────────────────────

async fn post_status(
    token: &str,
    owner: &str,
    repo: &str,
    sha: &str,
    state: &str,
    description: &str,
    target_url: Option<&str>,
) -> Result<(), String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/statuses/{sha}");
    let mut body = serde_json::Map::new();
    body.insert("state".into(), Value::String(state.into()));
    body.insert("context".into(), Value::String(STATUS_CONTEXT.into()));
    body.insert("description".into(), Value::String(description.into()));
    if let Some(t) = target_url {
        body.insert("target_url".into(), Value::String(t.into()));
    }
    let client = reqwest::Client::builder()
        .user_agent("CodeVetter/trex-watcher")
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?;
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&Value::Object(body))
        .send()
        .await
        .map_err(|e| format!("status post: {e}"))?;
    if !resp.status().is_success() {
        let code = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(format!("status {code}: {txt}"));
    }
    Ok(())
}

fn verdict_to_gh_state(verdict: &str) -> &'static str {
    match verdict {
        "APPROVE" => "success",
        "NEEDS_REVIEW" => "pending",
        _ => "failure",
    }
}

fn truncate_for_status(s: &str) -> String {
    // GitHub limits description to 140 chars.
    if s.chars().count() <= 140 {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(137).collect();
        out.push_str("…");
        out
    }
}

// ─── DB helpers ─────────────────────────────────────────────────────────────

fn db_state_from_app(app: &AppHandle) -> DbState {
    let st = app.state::<DbState>();
    DbState(st.0.clone())
}

fn clone_db_state(db: &DbState) -> DbState {
    DbState(db.0.clone())
}

fn upsert_watcher_row(
    db: &DbState,
    repo_path: &str,
    interval_secs: u64,
    enabled: bool,
    base_branch: Option<&str>,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO trex_watchers (repo_path, interval_secs, enabled, base_branch)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(repo_path) DO UPDATE SET
            interval_secs = excluded.interval_secs,
            enabled       = excluded.enabled,
            base_branch   = COALESCE(excluded.base_branch, trex_watchers.base_branch),
            last_error    = NULL",
        params![
            repo_path,
            interval_secs as i64,
            if enabled { 1 } else { 0 },
            base_branch
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn set_watcher_enabled(db: &DbState, repo_path: &str, enabled: bool) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE trex_watchers SET enabled = ?1 WHERE repo_path = ?2",
        params![if enabled { 1 } else { 0 }, repo_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn set_last_polled(db: &DbState, repo_path: &str) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE trex_watchers SET last_polled_at = datetime('now'), last_error = NULL WHERE repo_path = ?1",
        params![repo_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_watcher_row(db: &DbState, repo_path: &str) -> Result<Option<TrexWatcher>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let result = conn.query_row(
        "SELECT repo_path, interval_secs, enabled, base_branch, last_polled_at, last_error, created_at
         FROM trex_watchers WHERE repo_path = ?1",
        params![repo_path],
        row_to_watcher,
    );
    match result {
        Ok(w) => Ok(Some(w)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn list_watchers(db: &DbState) -> Result<Vec<TrexWatcher>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT repo_path, interval_secs, enabled, base_branch, last_polled_at, last_error, created_at
             FROM trex_watchers ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_watcher)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn row_to_watcher(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrexWatcher> {
    Ok(TrexWatcher {
        repo_path: row.get(0)?,
        interval_secs: row.get::<_, i64>(1)? as u64,
        enabled: row.get::<_, i64>(2)? != 0,
        base_branch: row.get(3)?,
        last_polled_at: row.get(4)?,
        last_error: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn latest_pr_run_sha(
    db: &DbState,
    repo_path: &str,
    pr_number: i64,
) -> Result<Option<String>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let result = conn.query_row(
        "SELECT head_sha FROM trex_pr_runs
         WHERE repo_path = ?1 AND pr_number = ?2
         ORDER BY ran_at DESC LIMIT 1",
        params![repo_path, pr_number],
        |r| r.get::<_, String>(0),
    );
    match result {
        Ok(s) => Ok(Some(s)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn insert_pr_run(db: &DbState, run: &TrexPrRun) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO trex_pr_runs (
            id, repo_path, pr_number, head_sha, verdict, confidence,
            summary, status_state, status_error, duration_ms, ran_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            run.id,
            run.repo_path,
            run.pr_number,
            run.head_sha,
            run.verdict,
            run.confidence,
            run.summary,
            run.status_state,
            run.status_error,
            run.duration_ms,
            run.ran_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_pr_runs(
    db: &DbState,
    repo_path: Option<&str>,
    limit: u32,
) -> Result<Vec<TrexPrRun>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let sql = if repo_path.is_some() {
        "SELECT id, repo_path, pr_number, head_sha, verdict, confidence, summary,
                status_state, status_error, duration_ms, ran_at
         FROM trex_pr_runs WHERE repo_path = ?1
         ORDER BY ran_at DESC LIMIT ?2"
    } else {
        "SELECT id, repo_path, pr_number, head_sha, verdict, confidence, summary,
                status_state, status_error, duration_ms, ran_at
         FROM trex_pr_runs ORDER BY ran_at DESC LIMIT ?1"
    };
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<TrexPrRun> {
        Ok(TrexPrRun {
            id: row.get(0)?,
            repo_path: row.get(1)?,
            pr_number: row.get(2)?,
            head_sha: row.get(3)?,
            verdict: row.get(4)?,
            confidence: row.get(5)?,
            summary: row.get(6)?,
            status_state: row.get(7)?,
            status_error: row.get(8)?,
            duration_ms: row.get(9)?,
            ran_at: row.get(10)?,
        })
    };
    let rows: Vec<TrexPrRun> = if let Some(rp) = repo_path {
        stmt.query_map(params![rp, limit as i64], mapper)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    } else {
        stmt.query_map(params![limit as i64], mapper)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect()
    };
    Ok(rows)
}

fn read_github_token(db: &DbState) -> Option<String> {
    let conn = db.0.lock().ok()?;
    read_pref(&conn, PREF_GITHUB_TOKEN)
}

fn read_pref(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM preferences WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_repo_from_https() {
        assert_eq!(
            parse_owner_repo("https://github.com/sarthak-fleet/CodeVetter.git"),
            Some(("sarthak-fleet".into(), "CodeVetter".into()))
        );
        assert_eq!(
            parse_owner_repo("https://github.com/sarthak-fleet/CodeVetter"),
            Some(("sarthak-fleet".into(), "CodeVetter".into()))
        );
    }

    #[test]
    fn owner_repo_from_ssh() {
        assert_eq!(
            parse_owner_repo("git@github.com:sarthak-fleet/CodeVetter.git"),
            Some(("sarthak-fleet".into(), "CodeVetter".into()))
        );
    }

    #[test]
    fn owner_repo_rejects_other_hosts() {
        assert_eq!(parse_owner_repo("https://gitlab.com/x/y.git"), None);
        assert_eq!(parse_owner_repo("bogus"), None);
    }

    #[test]
    fn verdict_state_mapping() {
        assert_eq!(verdict_to_gh_state("APPROVE"), "success");
        assert_eq!(verdict_to_gh_state("NEEDS_REVIEW"), "pending");
        assert_eq!(verdict_to_gh_state("BLOCK"), "failure");
        assert_eq!(verdict_to_gh_state("OTHER"), "failure");
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate_for_status("hi"), "hi");
    }

    #[test]
    fn truncate_long() {
        let s = "x".repeat(200);
        let out = truncate_for_status(&s);
        assert_eq!(out.chars().count(), 138); // 137 + '…'
        assert!(out.ends_with('…'));
    }
}
