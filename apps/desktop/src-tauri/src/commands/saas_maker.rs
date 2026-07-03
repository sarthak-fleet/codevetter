//! SaaS Maker integration — auth, projects, tasks. CodeVetter is a client of
//! the saas-maker spine (Cloudflare D1 + saasmaker-api Worker + cockpit UI).
//!
//! Auth resolution: env `SAASMAKER_SESSION_TOKEN` wins over a stored token in
//! the `preferences` row (`saas_maker_token`). Calls return graceful
//! "skipped"/"not configured" rather than panicking when unset.
//!
//! Two write-paths the UI uses today:
//!   - `push_finding_to_saas_maker` → POST /v1/tasks from a CodeVetter finding.
//!   - `update_saas_maker_task` → PATCH /v1/tasks/{id} for status transitions.
//!
//! v1.1.76 added the sign-in flow:
//!   - `start_saas_maker_signin` opens the cockpit's existing /cli/auth?code=
//!     page (reuses the CLI auth flow — no new infra on the cockpit side).
//!   - `poll_saas_maker_signin` polls /v1/cli/poll until the user approves,
//!     then stores the token + a cached user record.
//!   - `get_current_user`, `sign_out_of_saas_maker` round out the session.
//!   - `detect_project_for_repo` shells `git remote get-url origin`,
//!     normalizes the URL, and matches against the fleet project list so the
//!     correct project_slug auto-selects when picking a repo.

use std::time::Duration;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::db::queries;
use crate::DbState;

const DEFAULT_BASE_URL: &str = "https://api.sassmaker.com";
const DEFAULT_COCKPIT_URL: &str = "https://app.sassmaker.com";
const TOKEN_ENV: &str = "SAASMAKER_SESSION_TOKEN";
const URL_ENV: &str = "SAASMAKER_API_URL";
const COCKPIT_ENV: &str = "SAASMAKER_COCKPIT_URL";
const PREF_TOKEN: &str = "saas_maker_token";
const PREF_BASE_URL: &str = "saas_maker_base_url";
const PREF_COCKPIT_URL: &str = "saas_maker_cockpit_url";
const PREF_PROJECT_SLUG: &str = "saas_maker_project_slug";
const PREF_CACHED_USER: &str = "saas_maker_cached_user";

// Token cache freshness — re-fetch /v1/auth/session after this.
const USER_CACHE_FRESH_SECS: i64 = 24 * 60 * 60;
// Polling cadence + timeout for the CLI-style sign-in flow.
const POLL_INTERVAL_MS: u64 = 1500;
const POLL_TIMEOUT_SECS: u64 = 300;

// ─── Public IO ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaasMakerTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub project_slug: Option<String>,
    #[serde(default)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub pr_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaasMakerStatus {
    pub configured: bool,
    pub base_url: String,
    pub project_slug: Option<String>,
    /// Source of the token: "env", "preferences", or "none".
    pub token_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaasMakerSetConfig {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub project_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushFindingInput {
    pub review_id: String,
    pub finding_id: String,
    #[serde(default)]
    pub project_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushFindingResult {
    pub task: Option<SaasMakerTask>,
    pub skipped: bool,
    pub skipped_reason: Option<String>,
    /// True when the finding was already linked to a task before this call.
    pub already_synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaasMakerProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// Optional git URL of the canonical repo for this project. Used by
    /// `detect_project_for_repo` to auto-match a local repo to its fleet
    /// project. Field is added on the saas-maker side as a Drizzle migration;
    /// if absent (old worker), this stays None and detection falls back to
    /// the local `repo_project_mapping` table.
    #[serde(default)]
    pub git_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaasMakerUser {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInStart {
    /// One-time auth code; pass to `poll_saas_maker_signin`.
    pub code: String,
    /// Fully-built cockpit URL we just opened in the user's browser.
    pub approval_url: String,
    /// Seconds until the code expires.
    pub expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SignInResult {
    /// User approved in their browser. Token is already persisted; user is
    /// the freshly-cached identity.
    Approved { user: SaasMakerUser },
    /// Auth code expired before approval (10-minute window) or polling timed
    /// out after our 5-minute cap. Either way, ask the user to try again.
    Expired,
    /// Polling was cancelled from the frontend.
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoDetectResult {
    pub project: Option<SaasMakerProject>,
    /// "git_url" (matched via fleet `git_url` field), "manual_mapping"
    /// (matched via local `repo_project_mapping` row), or "none".
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTaskPatch {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_saas_maker_status(db: State<'_, DbState>) -> Result<SaasMakerStatus, String> {
    let (token, source) = resolve_token(&db);
    let base_url = resolve_base_url(&db);
    let project_slug = read_pref(&db, PREF_PROJECT_SLUG);
    Ok(SaasMakerStatus {
        configured: token.is_some(),
        base_url,
        project_slug,
        token_source: source.to_string(),
    })
}

#[tauri::command]
pub async fn set_saas_maker_config(
    db: State<'_, DbState>,
    config: SaasMakerSetConfig,
) -> Result<SaasMakerStatus, String> {
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        if let Some(token) = config.token.as_deref() {
            // Empty string = clear.
            if token.is_empty() {
                let _ = conn.execute(
                    "DELETE FROM preferences WHERE key = ?1",
                    params![PREF_TOKEN],
                );
            } else {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                    params![PREF_TOKEN, token],
                );
            }
        }
        if let Some(base) = config.base_url.as_deref() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                params![PREF_BASE_URL, base.trim_end_matches('/')],
            );
        }
        if let Some(slug) = config.project_slug.as_deref() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                params![PREF_PROJECT_SLUG, slug],
            );
        }
    }
    get_saas_maker_status(db).await
}

#[tauri::command]
pub async fn list_saas_maker_tasks(
    db: State<'_, DbState>,
    project_slug: Option<String>,
) -> Result<Vec<SaasMakerTask>, String> {
    let (token, _) = resolve_token(&db);
    let token = match token {
        Some(t) => t,
        None => {
            return Err(format!(
                "SaaS Maker not configured. Set {TOKEN_ENV} or configure via Settings."
            ))
        }
    };
    let base = resolve_base_url(&db);
    let slug = project_slug
        .or_else(|| read_pref(&db, PREF_PROJECT_SLUG))
        .filter(|s| !s.trim().is_empty());

    let mut url = format!("{base}/v1/tasks");
    if let Some(s) = &slug {
        url.push_str(&format!("?project_slug={}", urlencode(s)));
    }

    let resp = client()?
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("SaaS Maker GET {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "SaaS Maker GET {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    parse_task_list(&body)
}

#[tauri::command]
pub async fn list_saas_maker_projects(
    db: State<'_, DbState>,
) -> Result<Vec<SaasMakerProject>, String> {
    let (token, _) = resolve_token(&db);
    let token = token.ok_or_else(|| {
        format!("SaaS Maker not configured. Set {TOKEN_ENV} or use Settings.")
    })?;
    let base = resolve_base_url(&db);
    let url = format!("{base}/v1/projects");
    let resp = client()?
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("SaaS Maker GET {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "SaaS Maker GET {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    parse_project_list(&body)
}

#[tauri::command]
pub async fn update_saas_maker_task(
    db: State<'_, DbState>,
    task_id: String,
    patch: UpdateTaskPatch,
) -> Result<SaasMakerTask, String> {
    let (token, _) = resolve_token(&db);
    let token = token.ok_or_else(|| {
        format!("SaaS Maker not configured. Set {TOKEN_ENV} or use Settings.")
    })?;
    let base = resolve_base_url(&db);
    let url = format!("{base}/v1/tasks/{}", urlencode(&task_id));
    let payload = serde_json::to_value(&patch)
        .map_err(|e| format!("serialize patch: {e}"))?;
    let resp = client()?
        .patch(&url)
        .bearer_auth(&token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("SaaS Maker PATCH {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "SaaS Maker PATCH {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    let task = parse_single_task(&body)?;
    // Keep the cached payload current so re-pushes see the new status.
    let _ = refresh_sync_payload(&db, &task);
    Ok(task)
}

#[tauri::command]
pub async fn push_finding_to_saas_maker(
    db: State<'_, DbState>,
    input: PushFindingInput,
) -> Result<PushFindingResult, String> {
    // 1. Hydrate the finding from the local DB.
    let (review, finding) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let (review, findings) = queries::get_local_review_with_findings(&conn, &input.review_id)
            .map_err(|e| e.to_string())?;
        let finding = findings
            .into_iter()
            .find(|f| f.id == input.finding_id)
            .ok_or_else(|| format!("finding {} not found on review", input.finding_id))?;
        (review, finding)
    };

    // 2. Already synced?
    if let Some(prior) = lookup_existing_sync(&db, &input.finding_id)? {
        return Ok(PushFindingResult {
            task: Some(prior),
            skipped: true,
            skipped_reason: Some("already pushed".into()),
            already_synced: true,
        });
    }

    let (token, _) = resolve_token(&db);
    let token = match token {
        Some(t) => t,
        None => {
            return Ok(PushFindingResult {
                task: None,
                skipped: true,
                skipped_reason: Some(format!(
                    "SaaS Maker not configured. Set {TOKEN_ENV} or use Settings."
                )),
                already_synced: false,
            })
        }
    };
    let base = resolve_base_url(&db);
    let slug = input
        .project_slug
        .or_else(|| read_pref(&db, PREF_PROJECT_SLUG))
        .or(review.repo_full_name.clone())
        .or(review.repo_path.clone());

    let payload = build_task_payload(&review, &finding, slug.as_deref());
    let url = format!("{base}/v1/tasks");
    let resp = client()?
        .post(&url)
        .bearer_auth(&token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("SaaS Maker POST {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "SaaS Maker POST {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    let task = parse_single_task(&body)?;

    // 3. Persist the link so re-push is a no-op.
    record_sync(&db, &input.finding_id, &task)?;

    Ok(PushFindingResult {
        task: Some(task),
        skipped: false,
        skipped_reason: None,
        already_synced: false,
    })
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("build reqwest client: {e}"))
}

fn resolve_token(db: &State<'_, DbState>) -> (Option<String>, &'static str) {
    // Env wins over preferences so a shell-set token always overrides a
    // stale stored one.
    if let Ok(v) = std::env::var(TOKEN_ENV) {
        if !v.trim().is_empty() {
            return (Some(v), "env");
        }
    }
    if let Some(v) = read_pref(db, PREF_TOKEN) {
        return (Some(v), "preferences");
    }
    (None, "none")
}

fn resolve_base_url(db: &State<'_, DbState>) -> String {
    if let Ok(v) = std::env::var(URL_ENV) {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    if let Some(v) = read_pref(db, PREF_BASE_URL) {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    DEFAULT_BASE_URL.to_string()
}

fn read_pref(db: &State<'_, DbState>, key: &str) -> Option<String> {
    let conn = db.0.lock().ok()?;
    conn.query_row(
        "SELECT value FROM preferences WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn lookup_existing_sync(
    db: &State<'_, DbState>,
    finding_id: &str,
) -> Result<Option<SaasMakerTask>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT saas_maker_task_id, last_payload FROM saas_maker_sync
             WHERE local_source_kind = 'finding' AND local_source_id = ?1",
            params![finding_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .ok();
    match row {
        Some((_id, payload)) => {
            let task = serde_json::from_str::<SaasMakerTask>(&payload)
                .map_err(|e| format!("parse stored sync payload: {e}"))?;
            Ok(Some(task))
        }
        None => Ok(None),
    }
}

fn record_sync(
    db: &State<'_, DbState>,
    finding_id: &str,
    task: &SaasMakerTask,
) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let payload = serde_json::to_string(task).map_err(|e| format!("serialize task: {e}"))?;
    conn.execute(
        "INSERT OR REPLACE INTO saas_maker_sync
            (saas_maker_task_id, local_source_kind, local_source_id, last_payload, synced_at)
         VALUES (?1, 'finding', ?2, ?3, ?4)",
        params![task.id, finding_id, payload, now],
    )
    .map_err(|e| format!("insert sync row: {e}"))?;
    Ok(())
}

fn build_task_payload(
    review: &queries::LocalReviewRow,
    finding: &queries::LocalReviewFindingRow,
    project_slug: Option<&str>,
) -> Value {
    let discovery = finding
        .discovery_method
        .clone()
        .unwrap_or_else(|| "inspection".into());
    let severity = finding.severity.clone().unwrap_or_else(|| "medium".into());
    let priority = match severity.to_ascii_lowercase().as_str() {
        "critical" | "high" => "high",
        "low" | "info" => "low",
        _ => "medium",
    };
    let description = build_description(review, finding, &discovery);
    let title = finding.title.clone().unwrap_or_else(|| "CodeVetter finding".into());

    json!({
        "title": title,
        "description": description,
        "status": "todo",
        "priority": priority,
        "project_slug": project_slug.unwrap_or(""),
        "task_type": "bug",
    })
}

fn build_description(
    review: &queries::LocalReviewRow,
    finding: &queries::LocalReviewFindingRow,
    discovery: &str,
) -> String {
    let summary = finding.summary.clone().unwrap_or_default();
    let suggestion = finding.suggestion.clone().unwrap_or_default();
    let mut buf = String::new();
    if !summary.is_empty() {
        buf.push_str(&summary);
        buf.push_str("\n\n");
    }
    if !suggestion.is_empty() {
        buf.push_str("**Suggestion:** ");
        buf.push_str(&suggestion);
        buf.push_str("\n\n");
    }
    if let Some(p) = finding.file_path.as_deref() {
        let suffix = finding.line.map(|l| format!(":{l}")).unwrap_or_default();
        buf.push_str(&format!("**Location:** `{p}{suffix}`\n"));
    }
    buf.push_str(&format!("**Discovered via:** {discovery}\n"));
    if let Some(repo) = review.repo_full_name.as_deref().or(review.repo_path.as_deref()) {
        buf.push_str(&format!("**Repo:** {repo}\n"));
    }
    buf.push_str(&format!("\n_Pushed from CodeVetter review {}_\n", review.id));
    buf
}

fn parse_task_list(body: &str) -> Result<Vec<SaasMakerTask>, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| format!("SaaS Maker tasks response not JSON: {e}"))?;
    // Match reel-pipeline: payload.data is the array; fall back to the root if
    // an older shape ever appears.
    let arr = v
        .get("data")
        .and_then(|x| x.as_array())
        .cloned()
        .or_else(|| v.as_array().cloned())
        .ok_or_else(|| "expected `data` array in SaaS Maker tasks response".to_string())?;
    let mut out: Vec<SaasMakerTask> = Vec::with_capacity(arr.len());
    for item in arr {
        if let Ok(t) = serde_json::from_value::<SaasMakerTask>(item) {
            out.push(t);
        }
    }
    Ok(out)
}

fn parse_single_task(body: &str) -> Result<SaasMakerTask, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| format!("SaaS Maker create-task response not JSON: {e}"))?;
    let inner = v.get("data").cloned().unwrap_or(v);
    serde_json::from_value::<SaasMakerTask>(inner)
        .map_err(|e| format!("SaaS Maker create-task shape: {e}"))
}

fn parse_project_list(body: &str) -> Result<Vec<SaasMakerProject>, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| format!("SaaS Maker projects response not JSON: {e}"))?;
    let arr = v
        .get("data")
        .and_then(|x| x.as_array())
        .cloned()
        .or_else(|| v.as_array().cloned())
        .ok_or_else(|| "expected `data` array in SaaS Maker projects response".to_string())?;
    let mut out: Vec<SaasMakerProject> = Vec::with_capacity(arr.len());
    for item in arr {
        if let Ok(p) = serde_json::from_value::<SaasMakerProject>(item) {
            out.push(p);
        }
    }
    Ok(out)
}

/// Refresh the cached payload for a task whose status we just PATCHed, so the
/// next dedup lookup sees the new state and the UI can decide whether to
/// re-push or mark complete locally.
fn refresh_sync_payload(db: &State<'_, DbState>, task: &SaasMakerTask) -> Result<(), String> {
    let Ok(conn) = db.0.lock() else { return Ok(()); };
    let now = chrono::Utc::now().to_rfc3339();
    let Ok(payload) = serde_json::to_string(task) else { return Ok(()); };
    let _ = conn.execute(
        "UPDATE saas_maker_sync
            SET last_payload = ?1, synced_at = ?2
            WHERE saas_maker_task_id = ?3",
        params![payload, now, task.id],
    );
    Ok(())
}

fn urlencode(s: &str) -> String {
    // Minimal encoder for the few characters that matter in a slug. Avoids
    // pulling in a full crate just for this.
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push_str("%20"),
            other => {
                for byte in other.to_string().bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

// ─── v1.1.76: sign-in + identity + repo detect ──────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PollOutcome {
    Pending,
    Approved(String),
    Expired,
}

#[tauri::command]
pub async fn start_saas_maker_signin(db: State<'_, DbState>) -> Result<SignInStart, String> {
    let base = resolve_base_url(&db);
    let cockpit = resolve_cockpit_url(&db);

    let resp = client()?
        .post(format!("{base}/v1/cli/code"))
        .send()
        .await
        .map_err(|e| format!("POST /v1/cli/code failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "POST /v1/cli/code returned {status}: {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| format!("/v1/cli/code response not JSON: {e}"))?;
    let code = v
        .get("code")
        .and_then(|s| s.as_str())
        .ok_or_else(|| "missing `code` in /v1/cli/code response".to_string())?
        .to_string();
    let expires_in = v.get("expires_in").and_then(|n| n.as_u64()).unwrap_or(600);

    let approval_url = build_approval_url(&cockpit, &code);

    if let Err(e) = open_url_in_browser(&approval_url) {
        // Don't fail the whole call — the user can still copy the URL from the
        // returned struct. The frontend can fall back to "open this link" UI.
        log::warn!("failed to open browser to {approval_url}: {e}");
    }

    Ok(SignInStart {
        code,
        approval_url,
        expires_in,
    })
}

#[tauri::command]
pub async fn poll_saas_maker_signin(
    db: State<'_, DbState>,
    code: String,
) -> Result<SignInResult, String> {
    let base = resolve_base_url(&db);
    let deadline = std::time::Instant::now() + Duration::from_secs(POLL_TIMEOUT_SECS);
    let url = format!("{base}/v1/cli/poll?code={}", urlencode(&code));

    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(SignInResult::Expired);
        }

        let resp = client()?
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("GET {url} failed: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND {
            // Server lost the code (deleted after retrieval, or never existed).
            return Ok(SignInResult::Expired);
        }
        if !status.is_success() {
            return Err(format!(
                "/v1/cli/poll returned {status}: {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| format!("/v1/cli/poll response not JSON: {e}"))?;

        match parse_poll_response(&v) {
            PollOutcome::Pending => {
                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                continue;
            }
            PollOutcome::Expired => return Ok(SignInResult::Expired),
            PollOutcome::Approved(token) => {
                // Persist token through the same path that the manual paste
                // flow uses, so every downstream call (list/push/patch) Just
                // Works once we return.
                {
                    let conn = db.0.lock().map_err(|e| e.to_string())?;
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
                        params![PREF_TOKEN, token],
                    );
                }
                // Fetch the user record so the badge can render immediately
                // and survive app restarts without an extra round-trip.
                let user = fetch_session_user(&base, &token).await?;
                cache_user(&db, &user);
                return Ok(SignInResult::Approved { user });
            }
        }
    }
}

#[tauri::command]
pub async fn sign_out_of_saas_maker(db: State<'_, DbState>) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "DELETE FROM preferences WHERE key IN (?1, ?2)",
        params![PREF_TOKEN, PREF_CACHED_USER],
    );
    Ok(())
}

#[tauri::command]
pub async fn get_current_user(db: State<'_, DbState>) -> Result<Option<SaasMakerUser>, String> {
    let (token, _) = resolve_token(&db);
    let Some(token) = token else { return Ok(None); };

    // 1. Try cached user if fresh.
    if let Some((user, ts)) = read_cached_user(&db) {
        if is_cached_user_fresh(&ts) {
            return Ok(Some(user));
        }
    }

    // 2. Refresh from /v1/auth/session.
    let base = resolve_base_url(&db);
    match fetch_session_user(&base, &token).await {
        Ok(user) => {
            cache_user(&db, &user);
            Ok(Some(user))
        }
        Err(e) => {
            // Fall back to a stale cache if we have one — better than a
            // sudden sign-out on a transient network blip.
            if let Some((stale, _)) = read_cached_user(&db) {
                log::warn!("failed to refresh session, returning stale cache: {e}");
                return Ok(Some(stale));
            }
            Err(e)
        }
    }
}

#[tauri::command]
pub async fn detect_project_for_repo(
    db: State<'_, DbState>,
    repo_path: String,
) -> Result<RepoDetectResult, String> {
    let trimmed = repo_path.trim().to_string();
    if trimmed.is_empty() {
        return Err("repo_path is empty".to_string());
    }

    // 1. Try the local manual-mapping table first — once the user has linked
    //    a repo, we never want to "guess" again.
    if let Some(slug) = lookup_local_repo_mapping(&db, &trimmed) {
        // Hydrate the project from the live list (best-effort — if offline,
        // we still report the slug).
        let projects = list_saas_maker_projects(db.clone()).await.unwrap_or_default();
        let proj = projects
            .into_iter()
            .find(|p| p.slug.as_deref() == Some(slug.as_str()))
            .or(Some(SaasMakerProject {
                id: format!("local:{slug}"),
                name: slug.clone(),
                slug: Some(slug),
                source: None,
                git_url: None,
            }));
        return Ok(RepoDetectResult {
            project: proj,
            source: "manual_mapping".to_string(),
        });
    }

    // 2. Read `git remote get-url origin` and try to match against fleet
    //    project git_urls.
    let origin = match read_origin_url(&trimmed) {
        Ok(u) => u,
        Err(_) => {
            return Ok(RepoDetectResult {
                project: None,
                source: "none".to_string(),
            });
        }
    };
    let projects = list_saas_maker_projects(db).await.unwrap_or_default();
    match match_project_by_url(&origin, &projects) {
        Some(p) => Ok(RepoDetectResult {
            project: Some(p.clone()),
            source: "git_url".to_string(),
        }),
        None => Ok(RepoDetectResult {
            project: None,
            source: "none".to_string(),
        }),
    }
}

#[tauri::command]
pub async fn set_repo_project_mapping(
    db: State<'_, DbState>,
    repo_path: String,
    project_slug: String,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "INSERT OR REPLACE INTO repo_project_mapping (repo_path, project_slug, set_at)
         VALUES (?1, ?2, ?3)",
        params![repo_path.trim(), project_slug.trim(), now],
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedRepoEntry {
    pub project_name: String,
    pub project_slug: String,
    pub repo_path: String,
    pub origin_url: Option<String>,
    /// True if we PATCHed this project's `git_url` onto the spine.
    pub backfilled: bool,
    /// Set if backfill was attempted but the PATCH failed.
    pub backfill_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkAllResult {
    /// Whether the SaaS Maker API exposes a `git_url` field at all. When false,
    /// we only write local mappings and skip every backfill PATCH.
    pub git_url_supported: bool,
    pub scanned_repo_count: u32,
    pub linked: Vec<LinkedRepoEntry>,
    pub unmatched_repo_count: u32,
    pub backfilled_count: u32,
}

/// Normalize an arbitrary name to an alphanumeric, lowercase key for fuzzy
/// matching a local repo directory to a fleet project name.
/// "CodeVetter" / "code-vetter" / "code_vetter" all → "codevetter".
fn name_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Best-effort repo name out of an origin URL: last path segment, `.git` dropped.
fn repo_name_from_origin(origin: &str) -> Option<String> {
    let norm = normalize_git_url(origin); // host/owner/repo, lowercased, no .git
    norm.rsplit('/').next().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// Claude Code stores session logs under a directory whose name is the working
/// directory with every `/` replaced by `-`
/// (e.g. `-Users-me-Desktop-fleet-CodeVetter`). `cc_projects.dir_path` points
/// at that session-storage folder, not the real repo. Decoding is ambiguous
/// because real directory names can themselves contain `-` (`email-manager`),
/// so we resolve greedily against the filesystem: at each level take the
/// longest run of tokens that forms an existing directory.
fn decode_claude_session_path(encoded: &str) -> std::path::PathBuf {
    let toks: Vec<&str> = encoded.trim_start_matches('-').split('-').collect();
    let mut path = std::path::PathBuf::from("/");
    let mut i = 0;
    while i < toks.len() {
        let mut matched = false;
        let mut j = toks.len();
        while j > i {
            let cand = toks[i..j].join("-");
            let trial = path.join(&cand);
            if trial.is_dir() {
                path = trial;
                i = j;
                matched = true;
                break;
            }
            j -= 1;
        }
        if !matched {
            path = path.join(toks[i]);
            i += 1;
        }
    }
    path
}

/// Map a `cc_projects.dir_path` to the real working directory it represents.
/// Session-storage rows (basename begins with `-`) are decoded; anything else
/// is treated as a literal path.
fn resolve_real_dir(dir_path: &str) -> String {
    let base = std::path::Path::new(dir_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if base.starts_with('-') {
        decode_claude_session_path(&base)
            .to_string_lossy()
            .to_string()
    } else {
        dir_path.to_string()
    }
}

/// Resolve the git toplevel for a directory (the dir may be a subdir of the
/// repo). Returns None if the dir isn't inside a git repo.
fn git_toplevel(dir: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if top.is_empty() {
        None
    } else {
        Some(top)
    }
}

/// Bulk-link every indexed local repo to its matching fleet project by name,
/// persisting a local `repo_project_mapping` row for each match. When the spine
/// exposes `git_url`, also backfills each matched repo's origin URL onto the
/// project so future detection works by `git_url` fleet-wide.
#[tauri::command]
pub async fn link_all_repos_to_fleet(db: State<'_, DbState>) -> Result<LinkAllResult, String> {
    let (token, _) = resolve_token(&db);
    let token =
        token.ok_or_else(|| format!("SaaS Maker not configured. Set {TOKEN_ENV} or use Settings."))?;
    let base = resolve_base_url(&db);

    // 1. Fetch projects (raw) so we can both parse the typed list and detect
    //    whether the API surfaces a `git_url` field at all.
    let url = format!("{base}/v1/projects");
    let resp = client()?
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("SaaS Maker GET {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "SaaS Maker GET {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    let git_url_supported = body.contains("\"git_url\"");
    let projects = parse_project_list(&body)?;

    // 2. Collect the distinct git roots of every indexed directory.
    let dirs: Vec<String> = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT dir_path FROM cc_projects")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let mut roots: std::collections::HashSet<String> = std::collections::HashSet::new();
    for d in &dirs {
        let real = resolve_real_dir(d);
        if let Some(top) = git_toplevel(&real) {
            roots.insert(top);
        }
    }

    // 3. Match each root to a fleet project by name, write the mapping, and
    //    (when supported) backfill git_url.
    let mut linked: Vec<LinkedRepoEntry> = Vec::new();
    let mut unmatched = 0u32;
    let scanned = roots.len() as u32;

    for root in &roots {
        let origin = read_origin_url(root).ok();
        let base_name = std::path::Path::new(root)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut candidate_keys = vec![name_key(&base_name)];
        if let Some(ref o) = origin {
            if let Some(rn) = repo_name_from_origin(o) {
                candidate_keys.push(name_key(&rn));
            }
        }
        candidate_keys.retain(|k| !k.is_empty());

        let matched = projects.iter().find(|p| {
            let pk = name_key(&p.name);
            !pk.is_empty() && candidate_keys.iter().any(|k| k == &pk)
        });

        let Some(project) = matched else {
            unmatched += 1;
            continue;
        };
        let Some(slug) = project.slug.clone() else {
            // Can't persist a mapping without a slug.
            unmatched += 1;
            continue;
        };

        // 3a. Local mapping (idempotent).
        {
            let now = chrono::Utc::now().to_rfc3339();
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO repo_project_mapping (repo_path, project_slug, set_at)
                 VALUES (?1, ?2, ?3)",
                params![root.as_str(), slug.as_str(), now],
            );
        }

        // 3b. Backfill git_url onto the spine, if supported and we have an origin.
        let mut backfilled = false;
        let mut backfill_error: Option<String> = None;
        if git_url_supported {
            if let Some(ref o) = origin {
                let purl = format!("{base}/v1/projects/{}", urlencode(&project.id));
                match client()?
                    .patch(&purl)
                    .bearer_auth(&token)
                    .json(&json!({ "git_url": o }))
                    .send()
                    .await
                {
                    Ok(r) if r.status().is_success() => backfilled = true,
                    Ok(r) => {
                        let st = r.status();
                        let b = r.text().await.unwrap_or_default();
                        backfill_error = Some(format!(
                            "PATCH returned {st}: {}",
                            b.chars().take(200).collect::<String>()
                        ));
                    }
                    Err(e) => backfill_error = Some(format!("PATCH failed: {e}")),
                }
            }
        }

        linked.push(LinkedRepoEntry {
            project_name: project.name.clone(),
            project_slug: slug,
            repo_path: root.clone(),
            origin_url: origin,
            backfilled,
            backfill_error,
        });
    }

    let backfilled_count = linked.iter().filter(|l| l.backfilled).count() as u32;
    linked.sort_by(|a, b| a.project_name.to_lowercase().cmp(&b.project_name.to_lowercase()));

    Ok(LinkAllResult {
        git_url_supported,
        scanned_repo_count: scanned,
        linked,
        unmatched_repo_count: unmatched,
        backfilled_count,
    })
}

// ─── Internal helpers (v1.1.76) ─────────────────────────────────────────────

fn resolve_cockpit_url(db: &State<'_, DbState>) -> String {
    if let Ok(v) = std::env::var(COCKPIT_ENV) {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    if let Some(v) = read_pref(db, PREF_COCKPIT_URL) {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    DEFAULT_COCKPIT_URL.to_string()
}

fn build_approval_url(cockpit_base: &str, code: &str) -> String {
    format!(
        "{}/cli/auth?code={}&source=codevetter",
        cockpit_base.trim_end_matches('/'),
        urlencode(code)
    )
}

fn open_url_in_browser(url: &str) -> Result<(), String> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn()
        .map_err(|e| format!("open URL via OS opener: {e}"))?;
    Ok(())
}

pub(crate) fn parse_poll_response(v: &Value) -> PollOutcome {
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
    match status {
        "approved" => v
            .get("token")
            .and_then(|t| t.as_str())
            .map(|t| PollOutcome::Approved(t.to_string()))
            .unwrap_or(PollOutcome::Pending),
        "expired" => PollOutcome::Expired,
        // Anything else (pending / unknown / missing) → keep polling. Server
        // is the source of truth on whether the code is still alive.
        _ => PollOutcome::Pending,
    }
}

async fn fetch_session_user(base: &str, token: &str) -> Result<SaasMakerUser, String> {
    let url = format!("{base}/v1/auth/session");
    let resp = client()?
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("GET {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "GET {url} returned {status}: {}",
            body.chars().take(300).collect::<String>()
        ));
    }
    parse_session_user(&body)
}

pub(crate) fn parse_session_user(body: &str) -> Result<SaasMakerUser, String> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| format!("/v1/auth/session response not JSON: {e}"))?;
    if !v
        .get("authenticated")
        .and_then(|b| b.as_bool())
        .unwrap_or(false)
    {
        return Err("session not authenticated".to_string());
    }
    let user_v = v
        .get("user")
        .ok_or_else(|| "missing `user` in session response".to_string())?;
    serde_json::from_value::<SaasMakerUser>(user_v.clone())
        .map_err(|e| format!("user record shape: {e}"))
}

fn cache_user(db: &State<'_, DbState>, user: &SaasMakerUser) {
    let Ok(conn) = db.0.lock() else { return; };
    let payload = match serde_json::to_string(&json!({
        "user": user,
        "ts": chrono::Utc::now().to_rfc3339(),
    })) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = conn.execute(
        "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
        params![PREF_CACHED_USER, payload],
    );
}

fn read_cached_user(db: &State<'_, DbState>) -> Option<(SaasMakerUser, String)> {
    let raw = read_pref(db, PREF_CACHED_USER)?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let user = serde_json::from_value::<SaasMakerUser>(v.get("user")?.clone()).ok()?;
    let ts = v.get("ts")?.as_str()?.to_string();
    Some((user, ts))
}

pub(crate) fn is_cached_user_fresh(ts: &str) -> bool {
    use chrono::DateTime;
    match DateTime::parse_from_rfc3339(ts) {
        Ok(parsed) => {
            let age = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
            age.num_seconds() < USER_CACHE_FRESH_SECS && age.num_seconds() >= 0
        }
        Err(_) => false,
    }
}

pub(crate) fn normalize_git_url(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }
    // Strip protocol prefixes.
    let stripped = s
        .strip_prefix("git+https://")
        .or_else(|| s.strip_prefix("https://"))
        .or_else(|| s.strip_prefix("http://"))
        .or_else(|| s.strip_prefix("ssh://"))
        .unwrap_or(s);

    // Normalize SCP-style `git@host:path` → `host/path` only if no protocol
    // was stripped (so `ssh://git@host/path` doesn't get double-treated).
    let after_user = stripped
        .strip_prefix("git@")
        .map(|rest| rest.replacen(':', "/", 1))
        .unwrap_or_else(|| stripped.to_string());

    // Drop user@ if any survived after a protocol prefix strip.
    let no_user = match after_user.find('@') {
        Some(idx) if idx < after_user.find('/').unwrap_or(usize::MAX) => {
            after_user[idx + 1..].to_string()
        }
        _ => after_user,
    };

    // Strip `.git` suffix + trailing slashes + lowercase for case-insensitive
    // GitHub URLs (the server is case-insensitive on owner/repo).
    let trimmed_tail = no_user
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(no_user.trim_end_matches('/'))
        .to_string();

    trimmed_tail.to_lowercase()
}

pub(crate) fn match_project_by_url<'a>(
    local: &str,
    projects: &'a [SaasMakerProject],
) -> Option<&'a SaasMakerProject> {
    let norm_local = normalize_git_url(local);
    if norm_local.is_empty() {
        return None;
    }
    projects.iter().find(|p| {
        p.git_url
            .as_deref()
            .map(normalize_git_url)
            .map(|gp| !gp.is_empty() && gp == norm_local)
            .unwrap_or(false)
    })
}

fn read_origin_url(repo_path: &str) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("spawn git remote: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git remote get-url origin failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Shared changelog helper called from both saas_maker.rs and fleet.rs.
/// Hits `POST /v1/changelog/dashboard/{project_id}` with the user's Bearer
/// token. Returns the created changelog entry as JSON for the UI to render.
pub(crate) async fn push_changelog_helper(
    db: &State<'_, DbState>,
    input: crate::commands::fleet::PushChangelogInput,
) -> Result<serde_json::Value, String> {
    let (token, _) = resolve_token(db);
    let token = token.ok_or_else(|| {
        format!("SaaS Maker not configured. Set {TOKEN_ENV} or use Settings.")
    })?;
    let base = resolve_base_url(db);
    let url = format!(
        "{base}/v1/changelog/dashboard/{}",
        urlencode(&input.project_id)
    );
    let payload = serde_json::json!({
        "title": input.title,
        "content": input.content,
        "version": input.version,
        "type": input.r#type.clone().unwrap_or_else(|| "improvement".into()),
        "published": input.published.unwrap_or(false),
        "source": "codevetter",
        "agent": "codevetter-cli",
    });
    let resp = client()?
        .post(&url)
        .bearer_auth(&token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("POST {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "POST {url} returned {status}: {}",
            body.chars().take(400).collect::<String>()
        ));
    }
    serde_json::from_str(&body)
        .map_err(|e| format!("changelog response not JSON: {e}"))
}

fn lookup_local_repo_mapping(db: &State<'_, DbState>, repo_path: &str) -> Option<String> {
    let conn = db.0.lock().ok()?;
    conn.query_row(
        "SELECT project_slug FROM repo_project_mapping WHERE repo_path = ?1",
        params![repo_path],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_review() -> queries::LocalReviewRow {
        queries::LocalReviewRow {
            id: "rev-1".into(),
            review_type: Some("pr".into()),
            source_label: None,
            repo_path: Some("/repo/path".into()),
            repo_full_name: Some("acme/widget".into()),
            pr_number: Some(42),
            agent_used: "claude-code".into(),
            score_composite: Some(0.7),
            findings_count: Some(1),
            review_action: None,
            summary_markdown: None,
            status: "completed".into(),
            error_message: None,
            started_at: None,
            completed_at: None,
            created_at: "2026-06-16T00:00:00Z".into(),
        }
    }

    fn mk_finding(method: Option<&str>) -> queries::LocalReviewFindingRow {
        queries::LocalReviewFindingRow {
            id: "f-1".into(),
            review_id: "rev-1".into(),
            severity: Some("high".into()),
            title: Some("Null pointer in checkout".into()),
            summary: Some("the cart total crashes when count == 0".into()),
            suggestion: Some("guard with `if cart.items.is_empty()`".into()),
            file_path: Some("src/checkout.rs".into()),
            line: Some(89),
            confidence: Some(0.9),
            fingerprint: None,
            discovery_method: method.map(String::from),
            disposition: None,
        }
    }

    #[test]
    fn name_key_normalizes_casing_and_separators() {
        assert_eq!(name_key("CodeVetter"), "codevetter");
        assert_eq!(name_key("code-vetter"), "codevetter");
        assert_eq!(name_key("code_vetter"), "codevetter");
        assert_eq!(name_key("reel-pipeline"), "reelpipeline");
        assert_eq!(name_key("   "), "");
    }

    #[test]
    fn decode_claude_session_path_preserves_hyphenated_dirs() {
        // Greedy filesystem-guided decode must keep a real hyphenated dir
        // (`foo-bar`) intact rather than splitting it into `foo/bar`.
        let base = std::env::temp_dir().join(format!("cvtest_{}", std::process::id()));
        let nested = base.join("foo-bar").join("baz");
        std::fs::create_dir_all(&nested).unwrap();
        let encoded = nested.to_string_lossy().replace('/', "-");
        let decoded = decode_claude_session_path(&encoded);
        assert_eq!(decoded, nested);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn repo_name_from_origin_extracts_last_segment() {
        assert_eq!(
            repo_name_from_origin("https://github.com/sarthak/CodeVetter.git").as_deref(),
            Some("codevetter")
        );
        assert_eq!(
            repo_name_from_origin("git@github.com:sarthak/reel-pipeline.git").as_deref(),
            Some("reel-pipeline")
        );
        assert_eq!(repo_name_from_origin("").as_deref(), None);
    }

    #[test]
    fn link_match_pairs_repo_basename_to_project_name() {
        let projects = vec![
            SaasMakerProject {
                id: "p1".into(),
                name: "CodeVetter".into(),
                slug: Some("codevetter-modh33a1".into()),
                source: None,
                git_url: None,
            },
            SaasMakerProject {
                id: "p2".into(),
                name: "reel-pipeline".into(),
                slug: Some("reel-pipeline-x".into()),
                source: None,
                git_url: None,
            },
        ];
        // A repo whose dir basename is "CodeVetter" matches by name_key even
        // though the project slug carries a random suffix.
        let candidate_keys = vec![name_key("CodeVetter")];
        let matched = projects
            .iter()
            .find(|p| candidate_keys.iter().any(|k| k == &name_key(&p.name)));
        assert_eq!(matched.map(|p| p.id.as_str()), Some("p1"));
    }

    #[test]
    fn payload_has_priority_from_severity_and_renders_description() {
        let review = mk_review();
        let finding = mk_finding(Some("execution"));
        let v = build_task_payload(&review, &finding, Some("widget"));
        assert_eq!(v["priority"], "high");
        assert_eq!(v["status"], "todo");
        assert_eq!(v["task_type"], "bug");
        assert_eq!(v["project_slug"], "widget");
        let desc = v["description"].as_str().unwrap();
        assert!(desc.contains("the cart total crashes"));
        assert!(desc.contains("`if cart.items.is_empty()`"));
        assert!(desc.contains("src/checkout.rs:89"));
        assert!(desc.contains("execution"));
        assert!(desc.contains("acme/widget"));
        assert!(desc.contains("rev-1"));
    }

    #[test]
    fn severity_low_maps_to_low_priority() {
        let review = mk_review();
        let mut finding = mk_finding(None);
        finding.severity = Some("low".into());
        let v = build_task_payload(&review, &finding, None);
        assert_eq!(v["priority"], "low");
    }

    #[test]
    fn parses_task_list_with_data_envelope() {
        let body = r#"{"data":[{"id":"t1","title":"Bug X","status":"todo"},{"id":"t2","title":"Bug Y"}]}"#;
        let tasks = parse_task_list(body).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "t1");
        assert_eq!(tasks[1].title, "Bug Y");
    }

    #[test]
    fn parses_task_list_root_array_fallback() {
        let body = r#"[{"id":"t1","title":"Bug X"}]"#;
        let tasks = parse_task_list(body).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn parses_single_task_with_data_envelope() {
        let body = r#"{"data":{"id":"t99","title":"Created"}}"#;
        let t = parse_single_task(body).unwrap();
        assert_eq!(t.id, "t99");
    }

    #[test]
    fn parses_single_task_with_bare_object() {
        let body = r#"{"id":"t99","title":"Created"}"#;
        let t = parse_single_task(body).unwrap();
        assert_eq!(t.id, "t99");
    }

    #[test]
    fn url_encoder_handles_safe_chars() {
        assert_eq!(urlencode("hello-world"), "hello-world");
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }

    // ─── v1.1.76: URL normalization + repo detect ──────────────────────────

    #[test]
    fn normalizes_https_with_dot_git() {
        assert_eq!(
            normalize_git_url("https://github.com/sarthak-fleet/CodeVetter.git"),
            "github.com/sarthak-fleet/codevetter"
        );
    }

    #[test]
    fn normalizes_https_without_dot_git() {
        assert_eq!(
            normalize_git_url("https://github.com/sarthak-fleet/CodeVetter"),
            "github.com/sarthak-fleet/codevetter"
        );
    }

    #[test]
    fn normalizes_ssh_form() {
        assert_eq!(
            normalize_git_url("git@github.com:sarthak-fleet/CodeVetter.git"),
            "github.com/sarthak-fleet/codevetter"
        );
    }

    #[test]
    fn normalizes_ssh_form_with_user_and_port() {
        assert_eq!(
            normalize_git_url("ssh://git@github.com/sarthak-fleet/CodeVetter.git"),
            "github.com/sarthak-fleet/codevetter"
        );
    }

    #[test]
    fn normalizes_trailing_slash_and_casing() {
        assert_eq!(
            normalize_git_url("https://GitHub.com/Sarthak-FLEET/CodeVetter/"),
            "github.com/sarthak-fleet/codevetter"
        );
    }

    #[test]
    fn normalizes_empty_safely() {
        assert_eq!(normalize_git_url(""), "");
        assert_eq!(normalize_git_url("   "), "");
    }

    #[test]
    fn detects_project_by_git_url() {
        let projects = vec![
            SaasMakerProject {
                id: "1".into(),
                name: "Other".into(),
                slug: Some("other".into()),
                source: None,
                git_url: Some("https://github.com/x/other.git".into()),
            },
            SaasMakerProject {
                id: "2".into(),
                name: "CodeVetter".into(),
                slug: Some("codevetter".into()),
                source: None,
                git_url: Some("git@github.com:sarthak-fleet/CodeVetter.git".into()),
            },
        ];
        let local = "https://github.com/sarthak-fleet/CodeVetter";
        let m = match_project_by_url(local, &projects);
        assert!(m.is_some());
        assert_eq!(m.unwrap().slug.as_deref(), Some("codevetter"));
    }

    #[test]
    fn no_match_returns_none() {
        let projects = vec![SaasMakerProject {
            id: "1".into(),
            name: "Other".into(),
            slug: Some("other".into()),
            source: None,
            git_url: Some("https://github.com/x/other.git".into()),
        }];
        assert!(match_project_by_url("https://github.com/sarthak/CodeVetter", &projects).is_none());
    }

    #[test]
    fn projects_without_git_url_are_skipped_gracefully() {
        let projects = vec![SaasMakerProject {
            id: "1".into(),
            name: "Untagged".into(),
            slug: Some("untagged".into()),
            source: None,
            git_url: None,
        }];
        assert!(match_project_by_url("https://github.com/x/y", &projects).is_none());
    }

    // ─── v1.1.76: cached user freshness ────────────────────────────────────

    #[test]
    fn cached_user_within_window_is_fresh() {
        let now = chrono::Utc::now();
        let recent = now - chrono::Duration::hours(1);
        assert!(is_cached_user_fresh(&recent.to_rfc3339()));
    }

    #[test]
    fn cached_user_past_window_is_stale() {
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::hours(25);
        assert!(!is_cached_user_fresh(&old.to_rfc3339()));
    }

    #[test]
    fn cached_user_invalid_timestamp_is_stale() {
        assert!(!is_cached_user_fresh("not-a-date"));
        assert!(!is_cached_user_fresh(""));
    }

    // ─── v1.1.76: sign-in URL build ────────────────────────────────────────

    #[test]
    fn build_approval_url_includes_code_and_source() {
        let url = build_approval_url("https://app.sassmaker.com", "abc123");
        assert!(url.contains("abc123"));
        assert!(url.contains("source=codevetter"));
        assert!(url.starts_with("https://app.sassmaker.com/cli/auth?"));
    }

    #[test]
    fn build_approval_url_strips_trailing_slash() {
        let url = build_approval_url("https://app.sassmaker.com/", "abc");
        assert!(url.starts_with("https://app.sassmaker.com/cli/auth"));
        assert!(!url.starts_with("https://app.sassmaker.com//"));
    }

    // ─── v1.1.76: poll-response parsing ────────────────────────────────────

    #[test]
    fn poll_response_approved_extracts_token() {
        let v: Value =
            serde_json::from_str(r#"{"status":"approved","token":"sm_abc123"}"#).unwrap();
        assert_eq!(parse_poll_response(&v), PollOutcome::Approved("sm_abc123".into()));
    }

    #[test]
    fn poll_response_pending() {
        let v: Value = serde_json::from_str(r#"{"status":"pending"}"#).unwrap();
        assert_eq!(parse_poll_response(&v), PollOutcome::Pending);
    }

    #[test]
    fn poll_response_expired() {
        let v: Value = serde_json::from_str(r#"{"status":"expired"}"#).unwrap();
        assert_eq!(parse_poll_response(&v), PollOutcome::Expired);
    }

    #[test]
    fn poll_response_unknown_status_treated_as_pending() {
        let v: Value = serde_json::from_str(r#"{"status":"unrecognized"}"#).unwrap();
        assert_eq!(parse_poll_response(&v), PollOutcome::Pending);
    }

    // ─── v1.1.76: session response parsing ─────────────────────────────────

    #[test]
    fn parses_session_user() {
        let body = r#"{"authenticated":true,"user":{"id":"u1","email":"a@b.co","name":"Alice","avatar_url":"https://x/a.png"}}"#;
        let u = parse_session_user(body).unwrap();
        assert_eq!(u.id, "u1");
        assert_eq!(u.email.as_deref(), Some("a@b.co"));
        assert_eq!(u.name.as_deref(), Some("Alice"));
    }

    #[test]
    fn parses_session_user_minimal_fields() {
        let body = r#"{"authenticated":true,"user":{"id":"u2"}}"#;
        let u = parse_session_user(body).unwrap();
        assert_eq!(u.id, "u2");
        assert!(u.email.is_none());
    }

    #[test]
    fn session_unauthenticated_returns_error() {
        let body = r#"{"authenticated":false}"#;
        assert!(parse_session_user(body).is_err());
    }
}
