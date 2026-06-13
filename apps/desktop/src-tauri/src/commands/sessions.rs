use crate::db::queries;
use crate::DbState;
use serde_json::{json, Value};
use tauri::State;

/// List or search sessions with optional filtering by project and text query.
#[tauri::command]
pub async fn list_sessions(
    db: State<'_, DbState>,
    query: Option<String>,
    project: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let sessions = queries::list_sessions(
        &conn,
        query.as_deref(),
        project.as_deref(),
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "sessions": sessions }))
}

/// Read the compact normalized adapter message/tool-call archive for one session.
#[tauri::command]
pub async fn list_session_message_archive(
    db: State<'_, DbState>,
    session_id: String,
    limit: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let messages = queries::list_session_message_archive(&conn, &session_id, limit.unwrap_or(200))
        .map_err(|e| e.to_string())?;
    Ok(json!({ "messages": messages }))
}

/// Full-text search over the compact normalized adapter message/tool archive.
#[tauri::command]
pub async fn search_session_message_archive(
    db: State<'_, DbState>,
    query: String,
    adapter_id: Option<String>,
    kind: Option<String>,
    limit: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    queries::sync_session_message_archive_fts(&conn).map_err(|e| e.to_string())?;
    let results = queries::search_session_message_archive(
        &conn,
        &query,
        adapter_id.as_deref(),
        kind.as_deref(),
        limit.unwrap_or(50),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "results": results }))
}

/// Move all sessions from one or more source projects into a target project.
/// Updates session counts on both source and target projects.
#[tauri::command]
pub async fn merge_projects(
    db: State<'_, DbState>,
    source_project_ids: Vec<String>,
    target_project_id: String,
) -> Result<Value, String> {
    if source_project_ids.is_empty() {
        return Err("At least one source project is required".to_string());
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Verify target project exists
    let target_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM cc_projects WHERE id = ?1",
            rusqlite::params![target_project_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| e.to_string())?;

    if !target_exists {
        return Err(format!("Target project not found: {}", target_project_id));
    }

    let mut total_moved: i64 = 0;

    for source_id in &source_project_ids {
        if source_id == &target_project_id {
            continue; // Skip self-merge
        }

        // Count sessions being moved
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cc_sessions WHERE project_id = ?1",
                rusqlite::params![source_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        // Move sessions to the target project
        conn.execute(
            "UPDATE cc_sessions SET project_id = ?2 WHERE project_id = ?1",
            rusqlite::params![source_id, target_project_id],
        )
        .map_err(|e| e.to_string())?;

        // Update source project session count to 0
        conn.execute(
            "UPDATE cc_projects SET session_count = 0 WHERE id = ?1",
            rusqlite::params![source_id],
        )
        .map_err(|e| e.to_string())?;

        total_moved += count;
    }

    // Update target project session count
    let new_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM cc_sessions WHERE project_id = ?1",
            rusqlite::params![target_project_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE cc_projects SET session_count = ?2 WHERE id = ?1",
        rusqlite::params![target_project_id, new_count],
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({ "moved_sessions": total_moved }))
}
