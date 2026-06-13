use crate::db::queries;
use crate::DbState;
use serde_json::{json, Value};
use tauri::State;

/// Get a preference value by key.
#[tauri::command]
pub async fn get_preference(db: State<'_, DbState>, key: String) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let value = queries::get_preference(&conn, &key).map_err(|e| e.to_string())?;
    Ok(json!({ "key": key, "value": value }))
}

/// Set a preference value.
#[tauri::command]
pub async fn set_preference(
    db: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    queries::set_preference(&conn, &key, &value).map_err(|e| e.to_string())?;
    Ok(json!({ "key": key, "value": value, "saved": true }))
}
