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
    agent_type: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let sessions = queries::list_sessions(
        &conn,
        query.as_deref(),
        project.as_deref(),
        agent_type.as_deref(),
        limit.unwrap_or(50),
        offset.unwrap_or(0),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "sessions": sessions }))
}
