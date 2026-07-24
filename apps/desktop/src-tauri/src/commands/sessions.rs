use crate::commands::secret_policy::redact_secret_text;
use crate::db::queries;
use crate::DbState;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

const TRANSCRIPT_MESSAGE_LIMIT: i64 = 300;
const TRANSCRIPT_CONTENT_CHAR_LIMIT: usize = 8_000;

#[derive(Debug, Serialize)]
struct SessionTranscriptMessage {
    id: String,
    message_index: i64,
    role: Option<String>,
    kind: String,
    timestamp: Option<String>,
    content_text: Option<String>,
    content_truncated: bool,
    tool_name: Option<String>,
}

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

/// Read a bounded, redacted transcript from the normalized local archive.
/// This command never touches the provider process or original transcript file.
#[tauri::command]
pub async fn get_session_transcript(
    db: State<'_, DbState>,
    session_id: String,
) -> Result<Value, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() || session_id.len() > 512 {
        return Err("A valid session id is required".to_string());
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let total =
        queries::count_session_message_archive(&conn, session_id).map_err(|e| e.to_string())?;
    let messages =
        queries::list_session_message_archive(&conn, session_id, TRANSCRIPT_MESSAGE_LIMIT)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(session_transcript_message)
            .collect::<Vec<_>>();

    Ok(json!({
        "session_id": session_id,
        "messages": messages,
        "total_messages": total,
        "truncated": total > messages.len() as i64,
    }))
}

fn session_transcript_message(row: queries::SessionMessageArchiveRow) -> SessionTranscriptMessage {
    let (content_text, content_truncated) = row
        .content_text
        .as_deref()
        .map(redact_secret_text)
        .map(|(content, _redacted)| {
            let truncated = content.chars().count() > TRANSCRIPT_CONTENT_CHAR_LIMIT;
            let bounded = if truncated {
                content
                    .chars()
                    .take(TRANSCRIPT_CONTENT_CHAR_LIMIT)
                    .collect()
            } else {
                content
            };
            (Some(bounded), truncated)
        })
        .unwrap_or((None, false));

    SessionTranscriptMessage {
        id: row.id,
        message_index: row.message_index,
        role: row.role,
        kind: row.kind,
        timestamp: row.timestamp,
        content_text,
        content_truncated,
        tool_name: row.tool_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_row(content_text: &str) -> queries::SessionMessageArchiveRow {
        queries::SessionMessageArchiveRow {
            id: "message-1".to_string(),
            session_id: "session-1".to_string(),
            adapter_id: "codex".to_string(),
            agent_type: "codex".to_string(),
            source_ref: "/tmp/transcript.jsonl".to_string(),
            source_line: Some(1),
            message_index: 0,
            role: Some("user".to_string()),
            kind: "message".to_string(),
            timestamp: None,
            content_text: Some(content_text.to_string()),
            tool_name: None,
            tool_call_id: None,
            raw_type: None,
            created_at: "2026-07-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn transcript_preview_redacts_secret_like_content() {
        let message = session_transcript_message(archive_row("api_key=do-not-show"));
        assert_eq!(message.content_text.as_deref(), Some("[redacted]"));
        assert!(!message.content_truncated);
    }

    #[test]
    fn transcript_preview_bounds_individual_messages() {
        let message =
            session_transcript_message(archive_row(&"a".repeat(TRANSCRIPT_CONTENT_CHAR_LIMIT + 1)));
        assert_eq!(
            message.content_text.as_deref().map(str::len),
            Some(TRANSCRIPT_CONTENT_CHAR_LIMIT)
        );
        assert!(message.content_truncated);
    }
}
