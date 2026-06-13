use serde::{Deserialize, Serialize};

use crate::db::queries::{AgentTalkInput, AgentTalkRow};

/// Maximum length for the rendered talk context injected into prompts.
const MAX_RENDER_BYTES: usize = 2048;

/// Talks older than this are not injected as handover context.
pub const STALENESS_SECS: i64 = 3600; // 1 hour

/// Maximum length for raw output stored in the database.
const MAX_RAW_OUTPUT_BYTES: usize = 200 * 1024;

/// The `talk` sub-object we ask the review agent to include in its JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TalkPayload {
    #[serde(default)]
    pub files_read: Vec<String>,
    #[serde(default)]
    pub files_modified: Vec<String>,
    #[serde(default)]
    pub actions_summary: Option<String>,
    #[serde(default)]
    pub unfinished_work: Option<String>,
    #[serde(default)]
    pub key_decisions: Option<String>,
    #[serde(default)]
    pub recommended_next_steps: Option<String>,
}

/// Build an `AgentTalkInput` from a review run where the agent returned JSON
/// that may include a `talk` key.
pub fn build_talk_from_review(
    agent_type: &str,
    project_path: &str,
    input_prompt: &str,
    raw_output: &str,
    parsed_json: &serde_json::Value,
    review_id: Option<&str>,
    duration_ms: Option<i64>,
    input_context: Option<&str>,
) -> AgentTalkInput {
    let talk: TalkPayload = parsed_json
        .get("talk")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let truncated_raw = truncate_string(raw_output, MAX_RAW_OUTPUT_BYTES);

    // Build output_structured from the findings + score (strip the talk key)
    let output_structured = {
        let mut obj = parsed_json.clone();
        if let Some(map) = obj.as_object_mut() {
            map.remove("talk");
        }
        Some(serde_json::to_string(&obj).unwrap_or_default())
    };

    AgentTalkInput {
        agent_process_id: None,
        review_id: review_id.map(|s| s.to_string()),
        agent_type: agent_type.to_string(),
        project_path: project_path.to_string(),
        role: Some("reviewer".to_string()),
        input_prompt: input_prompt.to_string(),
        input_context: input_context.map(|s| s.to_string()),
        files_read: json_string_array(&talk.files_read),
        files_modified: json_string_array(&talk.files_modified),
        actions_summary: talk.actions_summary,
        output_raw: Some(truncated_raw),
        output_structured,
        exit_code: Some(0),
        unfinished_work: talk.unfinished_work,
        blockers: None,
        key_decisions: talk.key_decisions,
        codebase_state: None,
        recommended_next_steps: talk.recommended_next_steps,
        duration_ms,
        session_id: None,
    }
}

/// Build an `AgentTalkInput` from a fix run where we only have stdout + git diff.
pub fn build_talk_from_fix(
    agent_type: &str,
    project_path: &str,
    input_prompt: &str,
    raw_output: &str,
    files_modified: &[String],
    review_id: Option<&str>,
    duration_ms: Option<i64>,
    exit_code: Option<i32>,
    input_context: Option<&str>,
) -> AgentTalkInput {
    let truncated_raw = truncate_string(raw_output, MAX_RAW_OUTPUT_BYTES);

    let summary = if files_modified.is_empty() {
        "Agent ran but no files were modified.".to_string()
    } else {
        format!(
            "Fixed issues in {} file(s): {}",
            files_modified.len(),
            files_modified.join(", ")
        )
    };

    AgentTalkInput {
        agent_process_id: None,
        review_id: review_id.map(|s| s.to_string()),
        agent_type: agent_type.to_string(),
        project_path: project_path.to_string(),
        role: Some("fixer".to_string()),
        input_prompt: input_prompt.to_string(),
        input_context: input_context.map(|s| s.to_string()),
        files_read: None,
        files_modified: json_string_array(files_modified),
        actions_summary: Some(summary),
        output_raw: Some(truncated_raw),
        output_structured: None,
        exit_code,
        unfinished_work: None,
        blockers: None,
        key_decisions: None,
        codebase_state: None,
        recommended_next_steps: None,
        duration_ms,
        session_id: None,
    }
}

/// Render a talk row into a markdown preamble suitable for injecting into
/// the next agent's prompt.
pub fn render_talk_for_prompt(talk: &AgentTalkRow) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "## Previous Agent Context ({}, {})\n",
        talk.agent_type, talk.created_at
    ));

    // Task given (truncate to 500 chars)
    let task_preview = truncate_string(&talk.input_prompt, 500);
    parts.push(format!("**Task given:** {task_preview}\n"));

    if let Some(ref summary) = talk.actions_summary {
        parts.push(format!("**What was done:** {summary}\n"));
    }

    if let Some(ref files) = talk.files_modified {
        if files != "[]" {
            parts.push(format!("**Files modified:** {files}\n"));
        }
    }

    if let Some(ref decisions) = talk.key_decisions {
        parts.push(format!("**Key decisions:** {decisions}\n"));
    }

    if let Some(ref unfinished) = talk.unfinished_work {
        parts.push(format!("**Unfinished work:** {unfinished}\n"));
    }

    if let Some(ref next) = talk.recommended_next_steps {
        parts.push(format!("**Recommended next steps:** {next}\n"));
    }

    parts.push("---\n".to_string());

    let rendered = parts.join("\n");
    truncate_string(&rendered, MAX_RENDER_BYTES)
}

// ── Helpers ──────────────────────────────────────────────────────

fn json_string_array(items: &[String]) -> Option<String> {
    if items.is_empty() {
        None
    } else {
        Some(serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string()))
    }
}

fn truncate_string(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        // Don't split mid-character
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...[truncated]", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── truncate_string ──────────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_string("hello", 100), "hello");
    }

    #[test]
    fn truncate_at_exact_limit() {
        assert_eq!(truncate_string("hello", 5), "hello");
    }

    #[test]
    fn truncate_over_limit() {
        let result = truncate_string("hello world", 5);
        assert!(result.starts_with("hello"));
        assert!(result.ends_with("...[truncated]"));
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        // "é" is 2 bytes in UTF-8
        let s = "café";
        let result = truncate_string(s, 4); // "caf" is 3 bytes, "é" starts at 3
        assert!(result.ends_with("...[truncated]"));
        // Should not panic or produce invalid UTF-8
        assert!(result.is_char_boundary(0));
    }

    // ── json_string_array ────────────────────────────────────────

    #[test]
    fn json_string_array_empty_returns_none() {
        assert_eq!(json_string_array(&[]), None);
    }

    #[test]
    fn json_string_array_with_items() {
        let items = vec!["a.ts".to_string(), "b.rs".to_string()];
        let result = json_string_array(&items).unwrap();
        assert_eq!(result, r#"["a.ts","b.rs"]"#);
    }

    // ── build_talk_from_review ───────────────────────────────────

    #[test]
    fn review_talk_extracts_talk_key() {
        let parsed = json!({
            "findings": [],
            "score": 95,
            "summary": "Looks good",
            "talk": {
                "files_read": ["src/main.rs"],
                "actions_summary": "Reviewed main module",
                "key_decisions": "No issues found"
            }
        });

        let talk = build_talk_from_review(
            "claude",
            "/tmp/repo",
            "Review this",
            "raw output",
            &parsed,
            Some("rev-1"),
            Some(5000),
            None,
        );

        assert_eq!(talk.agent_type, "claude");
        assert_eq!(talk.role, Some("reviewer".to_string()));
        assert_eq!(talk.review_id, Some("rev-1".to_string()));
        assert_eq!(talk.files_read, Some(r#"["src/main.rs"]"#.to_string()));
        assert_eq!(
            talk.actions_summary,
            Some("Reviewed main module".to_string())
        );
        assert_eq!(talk.key_decisions, Some("No issues found".to_string()));
        assert_eq!(talk.duration_ms, Some(5000));
    }

    #[test]
    fn review_talk_handles_missing_talk_key() {
        let parsed = json!({
            "findings": [{"severity": "low", "title": "Nit"}],
            "score": 90,
            "summary": "Minor nit"
        });

        let talk = build_talk_from_review(
            "gemini",
            "/tmp/repo",
            "Review",
            "output",
            &parsed,
            None,
            None,
            None,
        );

        // Should still succeed with defaults
        assert_eq!(talk.agent_type, "gemini");
        assert_eq!(talk.files_read, None); // empty vec → None
        assert_eq!(talk.actions_summary, None);
        assert_eq!(talk.key_decisions, None);
    }

    #[test]
    fn review_talk_strips_talk_from_output_structured() {
        let parsed = json!({
            "findings": [],
            "score": 100,
            "talk": { "actions_summary": "Reviewed" }
        });

        let talk = build_talk_from_review(
            "claude",
            "/tmp/repo",
            "prompt",
            "raw",
            &parsed,
            None,
            None,
            None,
        );

        let structured: serde_json::Value =
            serde_json::from_str(talk.output_structured.as_ref().unwrap()).unwrap();
        assert!(structured.get("talk").is_none());
        assert!(structured.get("score").is_some());
    }

    // ── build_talk_from_fix ──────────────────────────────────────

    #[test]
    fn fix_talk_with_modified_files() {
        let files = vec!["src/lib.rs".to_string(), "src/main.rs".to_string()];
        let talk = build_talk_from_fix(
            "claude",
            "/tmp/repo",
            "Fix issues",
            "agent output",
            &files,
            Some("rev-1"),
            Some(3000),
            Some(0),
            None,
        );

        assert_eq!(talk.role, Some("fixer".to_string()));
        assert_eq!(talk.exit_code, Some(0));
        assert!(talk.actions_summary.as_ref().unwrap().contains("2 file(s)"));
        assert!(talk.files_modified.is_some());
    }

    #[test]
    fn fix_talk_no_files_modified() {
        let talk = build_talk_from_fix(
            "gemini",
            "/tmp/repo",
            "Fix it",
            "described changes only",
            &[],
            None,
            Some(1000),
            Some(0),
            None,
        );

        assert_eq!(
            talk.actions_summary,
            Some("Agent ran but no files were modified.".to_string())
        );
        assert_eq!(talk.files_modified, None);
    }

    #[test]
    fn fix_talk_preserves_input_context() {
        let talk = build_talk_from_fix(
            "claude",
            "/tmp/repo",
            "Fix",
            "out",
            &[],
            None,
            None,
            None,
            Some("previous talk context"),
        );

        assert_eq!(
            talk.input_context,
            Some("previous talk context".to_string())
        );
    }

    // ── render_talk_for_prompt ────────────────────────────────────

    fn make_talk_row(overrides: impl FnOnce(&mut AgentTalkRow)) -> AgentTalkRow {
        let mut row = AgentTalkRow {
            id: "t-1".to_string(),
            agent_process_id: None,
            review_id: None,
            agent_type: "claude".to_string(),
            project_path: "/tmp/repo".to_string(),
            role: Some("reviewer".to_string()),
            input_prompt: "Review this diff".to_string(),
            input_context: None,
            files_read: None,
            files_modified: Some(r#"["src/main.rs"]"#.to_string()),
            actions_summary: Some("Reviewed 5 files".to_string()),
            output_raw: None,
            output_structured: None,
            exit_code: Some(0),
            unfinished_work: Some("Security audit pending".to_string()),
            blockers: None,
            key_decisions: Some("Auth module needs refactor".to_string()),
            codebase_state: None,
            recommended_next_steps: Some("Fix SQL injection in auth.rs".to_string()),
            duration_ms: Some(5000),
            session_id: None,
            created_at: "2026-04-05T10:00:00Z".to_string(),
        };
        overrides(&mut row);
        row
    }

    #[test]
    fn render_includes_all_populated_fields() {
        let row = make_talk_row(|_| {});
        let rendered = render_talk_for_prompt(&row);

        assert!(rendered.contains("Previous Agent Context"));
        assert!(rendered.contains("claude"));
        assert!(rendered.contains("Review this diff"));
        assert!(rendered.contains("Reviewed 5 files"));
        assert!(rendered.contains("src/main.rs"));
        assert!(rendered.contains("Auth module needs refactor"));
        assert!(rendered.contains("Security audit pending"));
        assert!(rendered.contains("Fix SQL injection in auth.rs"));
        assert!(rendered.contains("---"));
    }

    #[test]
    fn render_skips_none_fields() {
        let row = make_talk_row(|r| {
            r.actions_summary = None;
            r.files_modified = None;
            r.key_decisions = None;
            r.unfinished_work = None;
            r.recommended_next_steps = None;
        });
        let rendered = render_talk_for_prompt(&row);

        assert!(rendered.contains("Previous Agent Context"));
        assert!(rendered.contains("Task given"));
        assert!(!rendered.contains("What was done"));
        assert!(!rendered.contains("Files modified"));
        assert!(!rendered.contains("Key decisions"));
        assert!(!rendered.contains("Unfinished work"));
        assert!(!rendered.contains("Recommended next steps"));
    }

    #[test]
    fn render_skips_empty_files_array() {
        let row = make_talk_row(|r| {
            r.files_modified = Some("[]".to_string());
        });
        let rendered = render_talk_for_prompt(&row);
        assert!(!rendered.contains("Files modified"));
    }

    #[test]
    fn render_truncates_long_prompt() {
        let long_prompt = "x".repeat(1000);
        let row = make_talk_row(|r| {
            r.input_prompt = long_prompt;
        });
        let rendered = render_talk_for_prompt(&row);
        assert!(rendered.contains("...[truncated]"));
    }

    #[test]
    fn render_respects_max_bytes() {
        // Make a talk row with very long content that exceeds 2KB
        let row = make_talk_row(|r| {
            r.actions_summary = Some("a".repeat(3000));
        });
        let rendered = render_talk_for_prompt(&row);
        assert!(rendered.len() <= MAX_RENDER_BYTES + 20); // +20 for "...[truncated]" suffix
    }

    // ── TalkPayload serde ────────────────────────────────────────

    #[test]
    fn talk_payload_deserializes_partial() {
        let json = json!({
            "files_read": ["a.ts"],
            "actions_summary": "Did stuff"
        });
        let payload: TalkPayload = serde_json::from_value(json).unwrap();
        assert_eq!(payload.files_read, vec!["a.ts"]);
        assert_eq!(payload.actions_summary, Some("Did stuff".to_string()));
        assert!(payload.files_modified.is_empty());
        assert!(payload.unfinished_work.is_none());
    }

    #[test]
    fn talk_payload_deserializes_empty_object() {
        let payload: TalkPayload = serde_json::from_value(json!({})).unwrap();
        assert!(payload.files_read.is_empty());
        assert!(payload.actions_summary.is_none());
    }
}
