use serde_json::{Map, Value};

const MAX_STREAM_MESSAGE_CHARS: usize = 128 * 1024;
const MAX_TEXT_CHARS: usize = 500;
const MAX_ID_CHARS: usize = 256;

pub fn normalize_claude_hook_event(raw: &str) -> Option<String> {
    let input = parse_bounded_object(raw)?;
    let hook = string_at(&input, "hook_event_name")?;
    let tool_name = string_at(&input, "tool_name");
    let notification_type = string_at(&input, "notification_type");
    let event = match (hook, tool_name, notification_type) {
        ("SessionStart", _, _) => "session_start",
        ("SessionEnd", _, _) => "session_end",
        ("UserPromptSubmit", _, _) => "prompt_submit",
        ("PermissionRequest", _, _) => "permission_request",
        ("PreToolUse", Some("AskUserQuestion"), _) => "question_asked",
        ("PreToolUse", _, _) => "tool_start",
        ("PostToolUse", _, _) => "tool_complete",
        ("PostToolUseFailure", _, _) => "tool_error",
        ("Stop", _, _) => "stop",
        ("StopFailure", _, _) => "stop_failure",
        ("Notification", _, Some("permission_prompt")) => "permission_request",
        ("Notification", _, Some("elicitation_dialog")) => "question_asked",
        ("Notification", _, Some("idle_prompt")) => "idle_prompt",
        ("Notification", _, Some("agent_needs_input")) => "question_asked",
        ("Notification", _, Some("agent_completed")) => "stop",
        _ => return None,
    };

    let mut output = base_event("claude", event, "claude-hook");
    copy_bounded_string(&input, &mut output, "session_id");
    copy_bounded_string(&input, &mut output, "transcript_path");
    copy_bounded_string(&input, &mut output, "cwd");
    if let Some(tool_name) = tool_name {
        output.insert(
            "tool_name".to_string(),
            Value::from(bounded_text(tool_name)),
        );
    }
    if let Some(request_id) = first_string(
        &input,
        &[
            "/tool_use_id",
            "/permission_request_id",
            "/tool_input/request_id",
        ],
    ) {
        output.insert(
            "request_id".to_string(),
            Value::from(bounded_id(request_id)),
        );
    }

    match event {
        "prompt_submit" => {
            copy_detail(&input, &mut output, "/prompt", "query");
        }
        "question_asked" => {
            let questions = input
                .pointer("/tool_input/questions")
                .and_then(Value::as_array)
                .map(|questions| sanitize_questions(questions))
                .unwrap_or_default();
            if let Some(question) = questions
                .first()
                .and_then(|question| question.get("question"))
                .and_then(Value::as_str)
            {
                output.insert("summary".to_string(), Value::from(question));
            } else if let Some(message) = first_string(&input, &["/message", "/notification"]) {
                output.insert("summary".to_string(), Value::from(bounded_text(message)));
            }
            if !questions.is_empty() {
                output.insert("questions".to_string(), Value::Array(questions));
            }
        }
        "stop" => {
            copy_detail(&input, &mut output, "/last_assistant_message", "response");
        }
        "tool_error" | "stop_failure" => {
            if let Some(error) = first_string(&input, &["/error", "/error_message"]) {
                output.insert("summary".to_string(), Value::from(bounded_text(error)));
            }
        }
        "permission_request" => {
            let summary = match tool_name {
                Some(tool) => format!("Claude requested permission for {}", bounded_text(tool)),
                None => "Claude requested permission".to_string(),
            };
            output.insert("summary".to_string(), Value::from(summary));
        }
        _ => {}
    }

    serde_json::to_string(&Value::Object(output)).ok()
}

pub fn normalize_codex_app_server_message(raw: &str) -> Option<String> {
    let input = parse_bounded_object(raw)?;
    let method = string_at(&input, "method")?;
    let params = input.get("params").unwrap_or(&Value::Null);
    let (event, summary) = match method {
        "thread/started" => ("session_start", "Codex session started".to_string()),
        "thread/closed" | "thread/archived" => ("session_end", "Codex session ended".to_string()),
        "turn/started" => ("prompt_submit", "Codex is working".to_string()),
        "turn/completed" => {
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            if status == "failed" {
                (
                    "failure",
                    first_string(
                        params,
                        &[
                            "/turn/error/message",
                            "/turn/error/additionalDetails",
                            "/error/message",
                        ],
                    )
                    .map(bounded_text)
                    .unwrap_or_else(|| "Codex turn failed".to_string()),
                )
            } else if status == "interrupted" {
                ("session_end", "Codex turn was interrupted".to_string())
            } else {
                ("turn_complete", "Codex completed its turn".to_string())
            }
        }
        "item/commandExecution/requestApproval" => (
            "permission_request",
            permission_summary(params, "Codex requested command approval"),
        ),
        "item/fileChange/requestApproval" => (
            "permission_request",
            permission_summary(params, "Codex requested file-change approval"),
        ),
        "item/permissions/requestApproval" => (
            "permission_request",
            permission_summary(params, "Codex requested additional permissions"),
        ),
        "item/tool/requestUserInput" => (
            "question_asked",
            first_question(params)
                .unwrap_or_else(|| "Codex is waiting for your answer".to_string()),
        ),
        "mcpServer/elicitation/request" => (
            "question_asked",
            first_string(params, &["/message"])
                .map(bounded_text)
                .unwrap_or_else(|| "A connected tool needs your input".to_string()),
        ),
        "serverRequest/resolved" => (
            "attention_resolved",
            "Codex resumed after your response".to_string(),
        ),
        "item/started" => (
            "tool_start",
            item_summary(params, "Codex started an action"),
        ),
        "item/completed" => {
            let failed = params
                .pointer("/item/status")
                .and_then(Value::as_str)
                .is_some_and(|status| matches!(status, "failed" | "declined"));
            if failed {
                ("tool_error", item_summary(params, "Codex action failed"))
            } else {
                (
                    "tool_complete",
                    item_summary(params, "Codex completed an action"),
                )
            }
        }
        "turn/plan/updated" => ("plan_updated", "Codex updated its plan".to_string()),
        "error" => (
            "failure",
            first_string(params, &["/error/message", "/message"])
                .map(bounded_text)
                .unwrap_or_else(|| "Codex reported an error".to_string()),
        ),
        _ => return None,
    };

    let mut output = base_event("codex", event, "codex-app-server");
    output.insert("summary".to_string(), Value::from(summary));
    for (field, pointers) in [
        ("session_id", ["/threadId", "/thread/id", "/turn/threadId"]),
        ("turn_id", ["/turnId", "/turn/id", "/item/turnId"]),
        ("item_id", ["/itemId", "/item/id", "/request/itemId"]),
    ] {
        if let Some(value) = first_string(params, &pointers) {
            output.insert(field.to_string(), Value::from(bounded_id(value)));
        }
    }
    if let Some(request_id) = request_identifier(&input) {
        output.insert("request_id".to_string(), Value::from(request_id));
    }
    if event == "question_asked" {
        let questions = params
            .get("questions")
            .and_then(Value::as_array)
            .map(|questions| sanitize_questions(questions))
            .unwrap_or_default();
        if !questions.is_empty() {
            output.insert("questions".to_string(), Value::Array(questions));
        }
    }
    if event == "permission_request" {
        if let Some(decisions) = params.get("availableDecisions").and_then(Value::as_array) {
            let decisions = decisions
                .iter()
                .filter_map(Value::as_str)
                .map(bounded_id)
                .map(Value::from)
                .take(8)
                .collect::<Vec<_>>();
            if !decisions.is_empty() {
                output.insert("available_decisions".to_string(), Value::Array(decisions));
            }
        }
    }
    serde_json::to_string(&Value::Object(output)).ok()
}

pub fn normalize_claude_stream_message(raw: &str) -> Option<String> {
    let input = parse_bounded_object(raw)?;
    let message_type = string_at(&input, "type")?;
    let (event, summary) = match message_type {
        "system" if string_at(&input, "subtype") == Some("init") => {
            ("session_start", "Claude session started".to_string())
        }
        "assistant" => {
            let tool = input
                .pointer("/message/content")
                .and_then(Value::as_array)
                .and_then(|content| {
                    content
                        .iter()
                        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                });
            if let Some(tool) = tool {
                (
                    "tool_start",
                    tool.get("name")
                        .and_then(Value::as_str)
                        .map(|name| format!("Claude is using {}", bounded_text(name)))
                        .unwrap_or_else(|| "Claude started a tool".to_string()),
                )
            } else {
                ("assistant_message", "Claude responded".to_string())
            }
        }
        "user" => {
            let failed = input
                .pointer("/message/content")
                .and_then(Value::as_array)
                .is_some_and(|content| {
                    content
                        .iter()
                        .any(|block| block.get("is_error").and_then(Value::as_bool) == Some(true))
                });
            if failed {
                ("tool_error", "Claude tool failed".to_string())
            } else {
                ("tool_complete", "Claude tool completed".to_string())
            }
        }
        "result" => {
            if input.get("is_error").and_then(Value::as_bool) == Some(true) {
                (
                    "failure",
                    first_string(&input, &["/result", "/error"])
                        .map(bounded_text)
                        .unwrap_or_else(|| "Claude turn failed".to_string()),
                )
            } else {
                ("turn_complete", "Claude completed its turn".to_string())
            }
        }
        "control_request" => (
            "permission_request",
            "Claude requested permission".to_string(),
        ),
        _ => return None,
    };

    let mut output = base_event("claude", event, "claude-stream-json");
    output.insert("summary".to_string(), Value::from(summary));
    for (field, pointers) in [
        ("session_id", ["/session_id", "/message/session_id"]),
        ("request_id", ["/request_id", "/request/id"]),
        ("item_id", ["/tool_use_id", "/request/tool_use_id"]),
    ] {
        if let Some(value) = first_string(&input, &pointers) {
            output.insert(field.to_string(), Value::from(bounded_id(value)));
        }
    }
    serde_json::to_string(&Value::Object(output)).ok()
}

fn parse_bounded_object(raw: &str) -> Option<Value> {
    if raw.chars().count() > MAX_STREAM_MESSAGE_CHARS {
        return None;
    }
    let value = serde_json::from_str::<Value>(raw).ok()?;
    value.is_object().then_some(value)
}

fn base_event(agent: &str, event: &str, source: &str) -> Map<String, Value> {
    [
        ("v".to_string(), Value::from(1)),
        ("agent".to_string(), Value::from(agent)),
        ("event".to_string(), Value::from(event)),
        ("source".to_string(), Value::from(source)),
    ]
    .into_iter()
    .collect()
}

fn string_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn first_string<'a>(value: &'a Value, pointers: &[&str]) -> Option<&'a str> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn copy_bounded_string(input: &Value, output: &mut Map<String, Value>, key: &str) {
    if let Some(value) = string_at(input, key) {
        output.insert(key.to_string(), Value::from(bounded_text(value)));
    }
}

fn copy_detail(input: &Value, output: &mut Map<String, Value>, pointer: &str, field: &str) {
    if let Some(value) = input.pointer(pointer).and_then(Value::as_str) {
        output.insert(field.to_string(), Value::from(bounded_text(value)));
    }
}

fn permission_summary(params: &Value, fallback: &str) -> String {
    first_string(params, &["/reason"])
        .map(bounded_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn item_summary(params: &Value, fallback: &str) -> String {
    let item_type = params
        .pointer("/item/type")
        .and_then(Value::as_str)
        .map(humanize_identifier);
    item_type
        .map(|item| format!("{}: {item}", fallback.trim_end_matches(" an action")))
        .unwrap_or_else(|| fallback.to_string())
}

fn first_question(params: &Value) -> Option<String> {
    params
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|questions| questions.first())
        .and_then(|question| question.get("question"))
        .and_then(Value::as_str)
        .map(bounded_text)
}

fn sanitize_questions(questions: &[Value]) -> Vec<Value> {
    questions
        .iter()
        .filter_map(|question| {
            let text = question.get("question").and_then(Value::as_str)?;
            let mut sanitized = Map::new();
            sanitized.insert("question".to_string(), Value::from(bounded_text(text)));
            if let Some(header) = question.get("header").and_then(Value::as_str) {
                sanitized.insert("header".to_string(), Value::from(bounded_text(header)));
            }
            if let Some(multi_select) = question.get("multiSelect").and_then(Value::as_bool) {
                sanitized.insert("multi_select".to_string(), Value::from(multi_select));
            }
            if let Some(options) = question.get("options").and_then(Value::as_array) {
                let options = options
                    .iter()
                    .filter_map(|option| {
                        option
                            .get("label")
                            .and_then(Value::as_str)
                            .or_else(|| option.as_str())
                    })
                    .map(bounded_text)
                    .map(Value::from)
                    .take(12)
                    .collect::<Vec<_>>();
                if !options.is_empty() {
                    sanitized.insert("options".to_string(), Value::Array(options));
                }
            }
            Some(Value::Object(sanitized))
        })
        .take(8)
        .collect()
}

fn request_identifier(input: &Value) -> Option<String> {
    let id = input.get("id")?;
    if let Some(id) = id.as_str() {
        return Some(bounded_id(id));
    }
    if let Some(id) = id.as_i64() {
        return Some(id.to_string());
    }
    id.as_u64().map(|id| id.to_string())
}

fn bounded_id(value: &str) -> String {
    value.trim().chars().take(MAX_ID_CHARS).collect()
}

fn bounded_text(value: &str) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() <= MAX_TEXT_CHARS {
        value
    } else {
        value.chars().take(MAX_TEXT_CHARS).collect::<String>() + "…"
    }
}

fn humanize_identifier(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if character.is_ascii_uppercase() {
                vec![' ', character.to_ascii_lowercase()]
            } else if character == '_' || character == '-' {
                vec![' ']
            } else {
                vec![character]
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codex_approval_with_exact_request_identity() {
        let normalized = normalize_codex_app_server_message(
            r#"{"id":41,"method":"item/commandExecution/requestApproval","params":{"threadId":"thr-1","turnId":"turn-2","itemId":"item-3","reason":"Needs network access","command":"curl secret.example","availableDecisions":["accept","decline"]}}"#,
        )
        .expect("normalized");
        let value = serde_json::from_str::<Value>(&normalized).expect("json");
        assert_eq!(value["event"], "permission_request");
        assert_eq!(value["session_id"], "thr-1");
        assert_eq!(value["turn_id"], "turn-2");
        assert_eq!(value["item_id"], "item-3");
        assert_eq!(value["request_id"], "41");
        assert_eq!(value["summary"], "Needs network access");
        assert!(!normalized.contains("curl"));
        assert!(!normalized.contains("secret.example"));
    }

    #[test]
    fn parses_codex_question_without_flattening_options() {
        let normalized = normalize_codex_app_server_message(
            r#"{"id":"req-9","method":"item/tool/requestUserInput","params":{"threadId":"thr-1","turnId":"turn-2","itemId":"item-3","questions":[{"header":"Scope","question":"Which change?","options":[{"label":"Small"},{"label":"Broad"}],"multiSelect":false}]}}"#,
        )
        .expect("normalized");
        let value = serde_json::from_str::<Value>(&normalized).expect("json");
        assert_eq!(value["event"], "question_asked");
        assert_eq!(value["summary"], "Which change?");
        assert_eq!(value["questions"][0]["options"][1], "Broad");
    }

    #[test]
    fn parses_codex_turn_completion_and_failure() {
        let completed = normalize_codex_app_server_message(
            r#"{"method":"turn/completed","params":{"threadId":"thr-1","turn":{"id":"turn-1","status":"completed"}}}"#,
        )
        .expect("completed");
        assert_eq!(
            serde_json::from_str::<Value>(&completed).unwrap()["event"],
            "turn_complete"
        );
        let failed = normalize_codex_app_server_message(
            r#"{"method":"turn/completed","params":{"threadId":"thr-1","turn":{"id":"turn-1","status":"failed","error":{"message":"Model unavailable"}}}}"#,
        )
        .expect("failed");
        assert_eq!(
            serde_json::from_str::<Value>(&failed).unwrap()["event"],
            "failure"
        );
    }

    #[test]
    fn parses_claude_hook_question_and_request_identity() {
        let normalized = normalize_claude_hook_event(
            r#"{"hook_event_name":"PreToolUse","session_id":"claude-1","tool_name":"AskUserQuestion","tool_use_id":"tool-2","tool_input":{"questions":[{"header":"Choice","question":"Choose one","options":[{"label":"A"},{"label":"B"}]}]}}"#,
        )
        .expect("normalized");
        let value = serde_json::from_str::<Value>(&normalized).expect("json");
        assert_eq!(value["event"], "question_asked");
        assert_eq!(value["request_id"], "tool-2");
        assert_eq!(value["questions"][0]["options"][0], "A");
    }

    #[test]
    fn parses_claude_stream_results_without_message_scraping() {
        let completed = normalize_claude_stream_message(
            r#"{"type":"result","subtype":"success","session_id":"claude-1","is_error":false,"result":"private response"}"#,
        )
        .expect("normalized");
        let value = serde_json::from_str::<Value>(&completed).expect("json");
        assert_eq!(value["event"], "turn_complete");
        assert!(!completed.contains("private response"));
    }

    #[test]
    fn rejects_malformed_unknown_and_oversized_stream_messages() {
        assert!(normalize_codex_app_server_message("not-json").is_none());
        assert!(
            normalize_codex_app_server_message(r#"{"method":"unknown","params":{}}"#).is_none()
        );
        assert!(normalize_claude_hook_event(&"x".repeat(MAX_STREAM_MESSAGE_CHARS + 1)).is_none());
    }

    #[test]
    fn current_provider_smoke_fixtures_cover_attention_and_terminal_outcomes() {
        let codex_cases = [
            (
                include_str!("../../tests/fixtures/agent-stream/codex-completed.json"),
                "turn_complete",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/codex-failed.json"),
                "failure",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/codex-question.json"),
                "question_asked",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/codex-approval.json"),
                "permission_request",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/codex-resolved.json"),
                "attention_resolved",
            ),
        ];
        for (fixture, expected) in codex_cases {
            let normalized = normalize_codex_app_server_message(fixture).expect(expected);
            assert_eq!(
                serde_json::from_str::<Value>(&normalized).unwrap()["event"],
                expected
            );
        }

        let claude_stream_cases = [
            (
                include_str!("../../tests/fixtures/agent-stream/claude-completed.json"),
                "turn_complete",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/claude-failed.json"),
                "failure",
            ),
        ];
        for (fixture, expected) in claude_stream_cases {
            let normalized = normalize_claude_stream_message(fixture).expect(expected);
            assert_eq!(
                serde_json::from_str::<Value>(&normalized).unwrap()["event"],
                expected
            );
        }

        let claude_hook_cases = [
            (
                include_str!("../../tests/fixtures/agent-stream/claude-question.json"),
                "question_asked",
            ),
            (
                include_str!("../../tests/fixtures/agent-stream/claude-approval.json"),
                "permission_request",
            ),
        ];
        for (fixture, expected) in claude_hook_cases {
            let normalized = normalize_claude_hook_event(fixture).expect(expected);
            assert_eq!(
                serde_json::from_str::<Value>(&normalized).unwrap()["event"],
                expected
            );
        }
    }
}
