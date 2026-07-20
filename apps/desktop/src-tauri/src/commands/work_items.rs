//! Local evidence-aware work items for the Work board.
//!
//! The row owns workflow intent and pointers only. Reviews, verification runs,
//! and terminal processes remain authoritative in their existing stores.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::State;
use uuid::Uuid;

use crate::DbState;

const WORK_ITEM_SCHEMA_VERSION: i64 = 1;
const DEFAULT_LIST_LIMIT: i64 = 250;
const MAX_LIST_LIMIT: i64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkItem {
    pub schema_version: i64,
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub project_path: Option<String>,
    pub workspace_id: Option<String>,
    pub status: String,
    pub preferred_provider: String,
    pub assigned_agent: Option<String>,
    pub agent_terminal_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub change_identity: Option<String>,
    pub review_id: Option<String>,
    pub review_score: Option<f64>,
    pub review_attempts: i64,
    pub verification_run_id: Option<String>,
    pub verification_status: String,
    pub completion_disposition: Option<String>,
    pub attention: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkItemInput {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub preferred_provider: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateWorkItemInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub preferred_provider: Option<String>,
    #[serde(default)]
    pub assigned_agent: Option<String>,
    #[serde(default)]
    pub agent_terminal_id: Option<String>,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub change_identity: Option<String>,
    #[serde(default)]
    pub review_id: Option<String>,
    #[serde(default)]
    pub review_score: Option<f64>,
    #[serde(default)]
    pub verification_run_id: Option<String>,
    #[serde(default)]
    pub verification_status: Option<String>,
    #[serde(default)]
    pub attention: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AttachWorkItemSessionInput {
    pub provider: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
}

#[tauri::command]
pub fn list_work_items(
    db: State<'_, DbState>,
    project_path: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<WorkItem>, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    list_work_items_from_connection(
        &conn,
        project_path.as_deref(),
        limit.unwrap_or(DEFAULT_LIST_LIMIT),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_work_item(
    db: State<'_, DbState>,
    input: CreateWorkItemInput,
) -> Result<WorkItem, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    create_work_item_in_connection(&conn, input).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn update_work_item(
    db: State<'_, DbState>,
    id: String,
    input: UpdateWorkItemInput,
) -> Result<WorkItem, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    update_work_item_in_connection(&conn, id.trim(), input).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn attach_work_item_session(
    db: State<'_, DbState>,
    id: String,
    input: AttachWorkItemSessionInput,
) -> Result<WorkItem, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    attach_work_item_session_in_connection(&conn, id.trim(), input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn transition_work_item(
    db: State<'_, DbState>,
    id: String,
    status: String,
    completion_disposition: Option<String>,
) -> Result<WorkItem, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    transition_work_item_in_connection(&conn, id.trim(), &status, completion_disposition.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_work_item(db: State<'_, DbState>, id: String) -> Result<(), String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    let changed = conn
        .execute("DELETE FROM agent_tasks WHERE id = ?1", params![id.trim()])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err(format!("Work item not found: {}", id.trim()));
    }
    Ok(())
}

fn list_work_items_from_connection(
    conn: &Connection,
    project_path: Option<&str>,
    limit: i64,
) -> rusqlite::Result<Vec<WorkItem>> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT);
    let mut statement = conn.prepare(
        "SELECT schema_version, id, title, description, acceptance_criteria,
                project_path, workspace_id, status, preferred_provider,
                assigned_agent, agent_terminal_id, agent_session_id,
                change_identity, review_id, review_score, review_attempts,
                verification_run_id, verification_status,
                completion_disposition, attention, created_at, updated_at
         FROM agent_tasks
         WHERE (?1 IS NULL OR project_path = ?1)
         ORDER BY updated_at DESC, id ASC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![project_path, limit], map_work_item)?;
    rows.collect()
}

fn create_work_item_in_connection(
    conn: &Connection,
    input: CreateWorkItemInput,
) -> rusqlite::Result<WorkItem> {
    let title = required_text(&input.title, "title")?;
    let provider = normalize_provider(input.preferred_provider.as_deref())?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO agent_tasks (
            id, title, description, acceptance_criteria, project_path,
            workspace_id, status, preferred_provider, schema_version,
            verification_status, attention, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'plan', ?7, ?8, 'missing', 0, ?9, ?9)",
        params![
            id,
            title,
            clean_optional(input.description),
            clean_optional(input.acceptance_criteria),
            clean_optional(input.project_path),
            clean_optional(input.workspace_id),
            provider,
            WORK_ITEM_SCHEMA_VERSION,
            now,
        ],
    )?;
    get_work_item(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn update_work_item_in_connection(
    conn: &Connection,
    id: &str,
    input: UpdateWorkItemInput,
) -> rusqlite::Result<WorkItem> {
    if id.is_empty() {
        return Err(invalid_input("id is required"));
    }
    let title = input
        .title
        .as_deref()
        .map(|value| required_text(value, "title"))
        .transpose()?;
    let provider = input
        .preferred_provider
        .as_deref()
        .map(|value| normalize_provider(Some(value)))
        .transpose()?;
    let verification_status = input
        .verification_status
        .as_deref()
        .map(normalize_verification_status)
        .transpose()?;
    let description_provided = input.description.is_some();
    let acceptance_criteria_provided = input.acceptance_criteria.is_some();
    let project_path_provided = input.project_path.is_some();
    let description = clean_optional(input.description);
    let acceptance_criteria = clean_optional(input.acceptance_criteria);
    let project_path = clean_optional(input.project_path);
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn.execute(
        "UPDATE agent_tasks SET
            title = COALESCE(?2, title),
            description = CASE WHEN ?3 THEN ?4 ELSE description END,
            acceptance_criteria = CASE WHEN ?5 THEN ?6 ELSE acceptance_criteria END,
            project_path = CASE WHEN ?7 THEN ?8 ELSE project_path END,
            preferred_provider = COALESCE(?9, preferred_provider),
            assigned_agent = COALESCE(?10, assigned_agent),
            agent_terminal_id = COALESCE(?11, agent_terminal_id),
            agent_session_id = COALESCE(?12, agent_session_id),
            change_identity = COALESCE(?13, change_identity),
            review_id = COALESCE(?14, review_id),
            review_score = COALESCE(?15, review_score),
            verification_run_id = COALESCE(?16, verification_run_id),
            verification_status = COALESCE(?17, verification_status),
            attention = COALESCE(?18, attention),
            updated_at = ?19
         WHERE id = ?1",
        params![
            id,
            title,
            description_provided,
            description,
            acceptance_criteria_provided,
            acceptance_criteria,
            project_path_provided,
            project_path,
            provider,
            clean_optional(input.assigned_agent),
            clean_optional(input.agent_terminal_id),
            clean_optional(input.agent_session_id),
            clean_optional(input.change_identity),
            clean_optional(input.review_id),
            input.review_score,
            clean_optional(input.verification_run_id),
            verification_status,
            input.attention.map(i64::from),
            now,
        ],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    get_work_item(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn transition_work_item_in_connection(
    conn: &Connection,
    id: &str,
    requested_status: &str,
    requested_disposition: Option<&str>,
) -> rusqlite::Result<WorkItem> {
    let status = normalize_work_status(requested_status)?;
    let disposition = normalize_completion_disposition(requested_disposition)?;
    let current = get_work_item(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let completion_disposition = if status == "done" {
        let disposition = disposition.ok_or_else(|| {
            invalid_input("Done requires completion_disposition: verified or waived")
        })?;
        if disposition == "verified"
            && (current.review_id.is_none()
                || current.verification_run_id.is_none()
                || current.verification_status != "passed"
                || current.change_identity.is_none())
        {
            return Err(invalid_input(
                "Verified completion requires review, passed verification, and change identity",
            ));
        }
        Some(disposition)
    } else {
        None
    };
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE agent_tasks
         SET status = ?2, completion_disposition = ?3, updated_at = ?4
         WHERE id = ?1",
        params![id, status, completion_disposition, now],
    )?;
    get_work_item(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn attach_work_item_session_in_connection(
    conn: &Connection,
    id: &str,
    input: AttachWorkItemSessionInput,
) -> rusqlite::Result<WorkItem> {
    if id.is_empty() {
        return Err(invalid_input("id is required"));
    }
    let provider = normalize_provider(Some(&input.provider))?;
    let terminal_id = clean_optional(input.terminal_id);
    let session_id = clean_optional(input.session_id);
    if terminal_id.is_none() && session_id.is_none() {
        return Err(invalid_input("terminal_id or session_id is required"));
    }
    let project_path = clean_optional(input.project_path);
    let current = get_work_item(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    if let (Some(item_path), Some(session_path)) =
        (current.project_path.as_deref(), project_path.as_deref())
    {
        if !same_project_path(item_path, session_path) {
            return Err(invalid_input(
                "The agent run belongs to a different repository than this work item",
            ));
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn.execute(
        "UPDATE agent_tasks SET
            preferred_provider = ?2,
            agent_terminal_id = ?3,
            agent_session_id = ?4,
            project_path = COALESCE(project_path, ?5),
            attention = 0,
            updated_at = ?6
         WHERE id = ?1",
        params![id, provider, terminal_id, session_id, project_path, now],
    )?;
    if changed == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    get_work_item(conn, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn get_work_item(conn: &Connection, id: &str) -> rusqlite::Result<Option<WorkItem>> {
    conn.query_row(
        "SELECT schema_version, id, title, description, acceptance_criteria,
                project_path, workspace_id, status, preferred_provider,
                assigned_agent, agent_terminal_id, agent_session_id,
                change_identity, review_id, review_score, review_attempts,
                verification_run_id, verification_status,
                completion_disposition, attention, created_at, updated_at
         FROM agent_tasks WHERE id = ?1",
        params![id],
        map_work_item,
    )
    .optional()
}

fn map_work_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let raw_status: String = row.get(7)?;
    Ok(WorkItem {
        schema_version: row.get(0)?,
        id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        acceptance_criteria: row.get(4)?,
        project_path: row.get(5)?,
        workspace_id: row.get(6)?,
        status: normalize_work_status(&raw_status).unwrap_or_else(|_| "plan".to_string()),
        preferred_provider: row.get(8)?,
        assigned_agent: row.get(9)?,
        agent_terminal_id: row.get(10)?,
        agent_session_id: row.get(11)?,
        change_identity: row.get(12)?,
        review_id: row.get(13)?,
        review_score: row.get(14)?,
        review_attempts: row.get(15)?,
        verification_run_id: row.get(16)?,
        verification_status: row.get(17)?,
        completion_disposition: row.get(18)?,
        attention: row.get::<_, i64>(19)? != 0,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn normalize_work_status(value: &str) -> rusqlite::Result<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "plan" | "backlog" | "todo" | "pending" => Ok("plan".to_string()),
        "build" | "in_progress" | "in-progress" => Ok("build".to_string()),
        "review" | "in_review" | "in-review" => Ok("review".to_string()),
        "verify" | "test" | "in_test" | "in-test" => Ok("verify".to_string()),
        "done" | "completed" => Ok("done".to_string()),
        _ => Err(invalid_input("Unknown work-item status")),
    }
}

fn normalize_provider(value: Option<&str>) -> rusqlite::Result<String> {
    match value
        .unwrap_or("codex")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "codex" => Ok("codex".to_string()),
        "claude" | "claude-code" => Ok("claude".to_string()),
        _ => Err(invalid_input("preferred_provider must be codex or claude")),
    }
}

fn normalize_verification_status(value: &str) -> rusqlite::Result<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "missing" | "running" | "passed" | "failed" | "stale" => {
            Ok(value.trim().to_ascii_lowercase())
        }
        _ => Err(invalid_input("Unknown verification status")),
    }
}

fn normalize_completion_disposition(value: Option<&str>) -> rusqlite::Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some("verified") => Ok(Some("verified".to_string())),
        Some("waived") => Ok(Some("waived".to_string())),
        Some(_) => Err(invalid_input(
            "completion_disposition must be verified or waived",
        )),
    }
}

fn required_text(value: &str, field: &str) -> rusqlite::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid_input(&format!("{field} is required")));
    }
    if value.chars().count() > 240 {
        return Err(invalid_input(&format!("{field} is too long")));
    }
    Ok(value.to_string())
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn same_project_path(left: &str, right: &str) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => normalize_project_path(left) == normalize_project_path(right),
    }
}

fn normalize_project_path(value: &str) -> String {
    let path = Path::new(value.trim());
    path.components()
        .collect::<std::path::PathBuf>()
        .to_string_lossy()
        .trim_end_matches(std::path::MAIN_SEPARATOR)
        .to_string()
}

fn invalid_input(message: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();
        conn
    }

    fn create(conn: &Connection) -> WorkItem {
        create_work_item_in_connection(
            conn,
            CreateWorkItemInput {
                title: "Ship the Work surface".to_string(),
                description: Some("Connect the product loop".to_string()),
                acceptance_criteria: Some("Build, review, and verify".to_string()),
                project_path: Some("/tmp/repo".to_string()),
                workspace_id: None,
                preferred_provider: Some("claude".to_string()),
            },
        )
        .unwrap()
    }

    #[test]
    fn legacy_statuses_normalize_without_rewriting() {
        assert_eq!(normalize_work_status("backlog").unwrap(), "plan");
        assert_eq!(normalize_work_status("in_progress").unwrap(), "build");
        assert_eq!(normalize_work_status("in_review").unwrap(), "review");
        assert_eq!(normalize_work_status("in_test").unwrap(), "verify");
        assert_eq!(normalize_work_status("completed").unwrap(), "done");
    }

    #[test]
    fn creates_and_updates_a_local_work_item() {
        let conn = connection();
        let item = create(&conn);
        assert_eq!(item.status, "plan");
        assert_eq!(item.preferred_provider, "claude");

        let updated = update_work_item_in_connection(
            &conn,
            &item.id,
            UpdateWorkItemInput {
                agent_terminal_id: Some("terminal-1".to_string()),
                agent_session_id: Some("provider-session-1".to_string()),
                attention: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.agent_terminal_id.as_deref(), Some("terminal-1"));
        assert!(updated.attention);
    }

    #[test]
    fn attaches_live_and_historical_sessions_without_creating_process_state() {
        let conn = connection();
        let item = create(&conn);

        let live = attach_work_item_session_in_connection(
            &conn,
            &item.id,
            AttachWorkItemSessionInput {
                provider: "codex".to_string(),
                terminal_id: Some("terminal-1".to_string()),
                session_id: Some("session-1".to_string()),
                project_path: Some("/tmp/repo/".to_string()),
            },
        )
        .unwrap();
        assert_eq!(live.preferred_provider, "codex");
        assert_eq!(live.assigned_agent, None);
        assert_eq!(live.agent_terminal_id.as_deref(), Some("terminal-1"));
        assert_eq!(live.agent_session_id.as_deref(), Some("session-1"));
        assert_eq!(live.project_path.as_deref(), Some("/tmp/repo"));

        let historical = attach_work_item_session_in_connection(
            &conn,
            &item.id,
            AttachWorkItemSessionInput {
                provider: "claude-code".to_string(),
                terminal_id: None,
                session_id: Some("historical-1".to_string()),
                project_path: None,
            },
        )
        .unwrap();
        assert_eq!(historical.preferred_provider, "claude");
        assert_eq!(historical.agent_terminal_id, None);
        assert_eq!(historical.agent_session_id.as_deref(), Some("historical-1"));
    }

    #[test]
    fn attachment_rejects_a_session_from_a_different_repository() {
        let conn = connection();
        let item = create(&conn);
        let error = attach_work_item_session_in_connection(
            &conn,
            &item.id,
            AttachWorkItemSessionInput {
                provider: "codex".to_string(),
                terminal_id: Some("terminal-1".to_string()),
                session_id: None,
                project_path: Some("/tmp/another-repo".to_string()),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("different repository"));
    }

    #[test]
    fn attachment_requires_a_terminal_or_provider_session_identity() {
        let conn = connection();
        let item = create(&conn);
        let error = attach_work_item_session_in_connection(
            &conn,
            &item.id,
            AttachWorkItemSessionInput {
                provider: "codex".to_string(),
                terminal_id: None,
                session_id: None,
                project_path: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("terminal_id or session_id"));
    }

    #[test]
    fn editable_optional_fields_can_be_cleared() {
        let conn = connection();
        let item = create(&conn);

        let updated = update_work_item_in_connection(
            &conn,
            &item.id,
            UpdateWorkItemInput {
                description: Some(String::new()),
                acceptance_criteria: Some("  ".to_string()),
                project_path: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(updated.description, None);
        assert_eq!(updated.acceptance_criteria, None);
        assert_eq!(updated.project_path, None);
    }

    #[test]
    fn done_requires_an_explicit_honest_disposition() {
        let conn = connection();
        let item = create(&conn);
        let error = transition_work_item_in_connection(&conn, &item.id, "done", None).unwrap_err();
        assert!(error.to_string().contains("completion_disposition"));

        let waived =
            transition_work_item_in_connection(&conn, &item.id, "done", Some("waived")).unwrap();
        assert_eq!(waived.completion_disposition.as_deref(), Some("waived"));
    }

    #[test]
    fn verified_completion_requires_review_verification_and_identity() {
        let conn = connection();
        let item = create(&conn);
        let error = transition_work_item_in_connection(&conn, &item.id, "done", Some("verified"))
            .unwrap_err();
        assert!(error.to_string().contains("Verified completion"));
    }

    #[test]
    fn list_is_bounded_and_project_scoped() {
        let conn = connection();
        let first = create(&conn);
        create_work_item_in_connection(
            &conn,
            CreateWorkItemInput {
                title: "Other repo".to_string(),
                description: None,
                acceptance_criteria: None,
                project_path: Some("/tmp/other".to_string()),
                workspace_id: None,
                preferred_provider: None,
            },
        )
        .unwrap();
        let items = list_work_items_from_connection(&conn, Some("/tmp/repo"), 250).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, first.id);
    }
}
