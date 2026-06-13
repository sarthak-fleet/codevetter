use serde_json::{json, Value};
use tauri::State;

use crate::db::queries;
use crate::DbState;

#[tauri::command]
pub async fn get_talk(db: State<'_, DbState>, id: String) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let talk = queries::get_agent_talk(&conn, &id).map_err(|e| e.to_string())?;
    match talk {
        Some(t) => Ok(serde_json::to_value(t).map_err(|e| e.to_string())?),
        None => Ok(Value::Null),
    }
}

#[tauri::command]
pub async fn list_project_talks(
    db: State<'_, DbState>,
    project_path: String,
    limit: Option<i64>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let talks = queries::list_talks_for_project(&conn, &project_path, limit.unwrap_or(20))
        .map_err(|e| e.to_string())?;
    Ok(json!(talks))
}

#[tauri::command]
pub async fn get_latest_talk(
    db: State<'_, DbState>,
    project_path: String,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let talk =
        queries::get_latest_talk_for_project(&conn, &project_path).map_err(|e| e.to_string())?;
    match talk {
        Some(t) => Ok(serde_json::to_value(t).map_err(|e| e.to_string())?),
        None => Ok(Value::Null),
    }
}
