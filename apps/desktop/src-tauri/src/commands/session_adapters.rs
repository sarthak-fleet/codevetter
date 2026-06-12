#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RawSessionAdapterSummary {
    pub adapter_id: String,
    pub agent_type: String,
    pub stable_id: Option<String>,
    pub source_ref: String,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cli_version: Option<String>,
    pub model_used: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub message_count: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub compaction_count: i64,
    pub day_counts: BTreeMap<String, i64>,
    pub parse_warnings: Vec<String>,
}

pub trait SessionSourceAdapter {
    fn adapter_id(&self) -> &'static str;
    fn agent_type(&self) -> &'static str;
    fn parse_raw(&self, source_ref: &str, raw: &str) -> RawSessionAdapterSummary;
}

pub struct ClaudeCodeAdapter;
pub struct CodexAdapter;
pub struct CursorAdapter;

fn empty_summary(adapter_id: &str, agent_type: &str, source_ref: &str) -> RawSessionAdapterSummary {
    RawSessionAdapterSummary {
        adapter_id: adapter_id.to_string(),
        agent_type: agent_type.to_string(),
        stable_id: None,
        source_ref: source_ref.to_string(),
        cwd: None,
        git_branch: None,
        cli_version: None,
        model_used: None,
        first_timestamp: None,
        last_timestamp: None,
        message_count: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        compaction_count: 0,
        day_counts: BTreeMap::new(),
        parse_warnings: Vec::new(),
    }
}

fn update_timestamp(summary: &mut RawSessionAdapterSummary, timestamp: Option<String>) {
    if summary.first_timestamp.is_none() {
        summary.first_timestamp = timestamp.clone();
    }
    if timestamp.is_some() {
        summary.last_timestamp = timestamp;
    }
}

fn record_day(summary: &mut RawSessionAdapterSummary, timestamp: Option<&str>) {
    if let Some(timestamp) = timestamp {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) {
            let day = dt
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string();
            *summary.day_counts.entry(day).or_insert(0) += 1;
        }
    }
}

fn value_string(value: Option<&Value>, key: &str) -> Option<String> {
    value?
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
}

fn millis_to_rfc3339(value: Option<&Value>) -> Option<String> {
    value?
        .as_i64()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339())
}

impl SessionSourceAdapter for ClaudeCodeAdapter {
    fn adapter_id(&self) -> &'static str {
        "claude-code"
    }

    fn agent_type(&self) -> &'static str {
        "claude-code"
    }

    fn parse_raw(&self, source_ref: &str, raw: &str) -> RawSessionAdapterSummary {
        let mut summary = empty_summary(self.adapter_id(), self.agent_type(), source_ref);

        for (idx, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parsed: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => {
                    summary
                        .parse_warnings
                        .push(format!("line {} is not valid JSON", idx + 1));
                    continue;
                }
            };

            let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(
                msg_type,
                "progress"
                    | "file-history-snapshot"
                    | "queue-operation"
                    | "last-prompt"
                    | "permission-mode"
                    | "pr-link"
                    | "agent-name"
                    | "custom-title"
                    | "attachment"
            ) {
                continue;
            }

            if msg_type == "summary"
                || parsed
                    .get("autoCompact")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                || parsed
                    .get("isCompacted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            {
                summary.compaction_count += 1;
            }

            if summary.stable_id.is_none() {
                summary.stable_id = value_string(Some(&parsed), "sessionId");
            }
            if summary.cli_version.is_none() {
                summary.cli_version = value_string(Some(&parsed), "version");
            }
            if summary.git_branch.is_none() {
                summary.git_branch = value_string(Some(&parsed), "gitBranch");
            }
            if summary.cwd.is_none() {
                summary.cwd = value_string(Some(&parsed), "cwd");
            }

            let timestamp = value_string(Some(&parsed), "timestamp");
            record_day(&mut summary, timestamp.as_deref());
            update_timestamp(&mut summary, timestamp);

            let usage = parsed
                .get("message")
                .and_then(|message| message.get("usage"));
            let input = usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache_creation = usage
                .and_then(|u| u.get("cache_creation_input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache_read = usage
                .and_then(|u| u.get("cache_read_input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            summary.total_input_tokens += input + cache_creation + cache_read;
            summary.total_output_tokens += output;
            summary.cache_read_tokens += cache_read;
            summary.cache_creation_tokens += cache_creation;

            if let Some(model) = parsed
                .get("message")
                .and_then(|message| message.get("model"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                summary.model_used = Some(model.to_string());
            }

            summary.message_count += 1;
        }

        if summary.stable_id.is_none() {
            summary
                .parse_warnings
                .push("missing stable session id".to_string());
        }
        summary
    }
}

impl SessionSourceAdapter for CodexAdapter {
    fn adapter_id(&self) -> &'static str {
        "codex"
    }

    fn agent_type(&self) -> &'static str {
        "codex"
    }

    fn parse_raw(&self, source_ref: &str, raw: &str) -> RawSessionAdapterSummary {
        let mut summary = empty_summary(self.adapter_id(), self.agent_type(), source_ref);
        let mut has_cumulative_token_count = false;

        for (idx, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parsed: Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => {
                    summary
                        .parse_warnings
                        .push(format!("line {} is not valid JSON", idx + 1));
                    continue;
                }
            };
            let msg_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let payload = parsed.get("payload");

            if msg_type == "session_meta" {
                if let Some(payload) = payload {
                    summary.stable_id = value_string(Some(payload), "id");
                    summary.cwd = value_string(Some(payload), "cwd");
                    summary.cli_version = value_string(Some(payload), "cli_version");
                    summary.git_branch = payload
                        .get("git")
                        .and_then(|git| git.get("branch"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    summary.model_used = value_string(Some(payload), "model").or_else(|| {
                        value_string(Some(payload), "model_provider").map(|provider| {
                            if provider == "openai" {
                                "o3".to_string()
                            } else {
                                provider
                            }
                        })
                    });
                }
                continue;
            }

            if msg_type == "event_msg" {
                let sub_type = payload
                    .and_then(|p| p.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if sub_type == "token_count" {
                    if let Some(total_usage) = payload
                        .and_then(|p| p.get("info"))
                        .and_then(|info| info.get("total_token_usage"))
                    {
                        summary.total_input_tokens = total_usage
                            .get("input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        summary.total_output_tokens = total_usage
                            .get("output_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        summary.cache_read_tokens = total_usage
                            .get("cached_input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        has_cumulative_token_count = true;
                    }
                }
                continue;
            }

            if msg_type == "response_item" {
                let timestamp = value_string(Some(&parsed), "timestamp");
                record_day(&mut summary, timestamp.as_deref());
                update_timestamp(&mut summary, timestamp);
                if let Some(usage) = payload.and_then(|p| p.get("usage")) {
                    if !has_cumulative_token_count {
                        summary.total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        summary.total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                    }
                }
                summary.message_count += 1;
            }
        }

        if summary.stable_id.is_none() {
            summary
                .parse_warnings
                .push("missing session_meta id".to_string());
        }
        if summary.cwd.is_none() {
            summary
                .parse_warnings
                .push("missing session_meta cwd".to_string());
        }
        summary
    }
}

impl SessionSourceAdapter for CursorAdapter {
    fn adapter_id(&self) -> &'static str {
        "cursor"
    }

    fn agent_type(&self) -> &'static str {
        "cursor"
    }

    fn parse_raw(&self, source_ref: &str, raw: &str) -> RawSessionAdapterSummary {
        let mut summary = empty_summary(self.adapter_id(), self.agent_type(), source_ref);
        let parsed: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => {
                summary
                    .parse_warnings
                    .push("cursor fixture is not valid JSON".to_string());
                return summary;
            }
        };

        let composer_id = parsed
            .get("composer_id")
            .and_then(|v| v.as_str())
            .or_else(|| parsed.get("composerId").and_then(|v| v.as_str()))
            .unwrap_or("unknown");
        summary.stable_id = Some(format!("cursor-{composer_id}"));

        let composer = parsed.get("composer").unwrap_or(&parsed);
        summary.cwd = composer
            .pointer("/workspaceIdentifier/uri/fsPath")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                composer
                    .get("trackedGitRepos")
                    .and_then(|v| v.as_array())
                    .and_then(|repos| repos.first())
                    .and_then(|repo| {
                        repo.get("path")
                            .or_else(|| repo.get("repoPath"))
                            .or_else(|| repo.get("rootPath"))
                    })
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
        summary.model_used = composer
            .pointer("/modelConfig/modelName")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty() && *s != "default")
            .map(String::from);
        summary.first_timestamp = millis_to_rfc3339(composer.get("createdAt"));
        summary.last_timestamp = millis_to_rfc3339(composer.get("lastUpdatedAt"));

        let bubbles = parsed
            .get("bubbles")
            .and_then(|v| v.as_array())
            .or_else(|| parsed.get("messages").and_then(|v| v.as_array()));
        if let Some(bubbles) = bubbles {
            for bubble in bubbles {
                let timestamp = bubble.get("createdAt").and_then(|v| {
                    v.as_str().map(String::from).or_else(|| {
                        v.as_i64()
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|dt| dt.to_rfc3339())
                    })
                });
                record_day(&mut summary, timestamp.as_deref());
                update_timestamp(&mut summary, timestamp);
                summary.message_count += 1;
            }
        } else if let Some(headers) = composer
            .get("fullConversationHeadersOnly")
            .and_then(|v| v.as_array())
        {
            summary.message_count = headers.len() as i64;
        }

        if summary.message_count == 0 {
            summary
                .parse_warnings
                .push("cursor conversation has no indexed bubbles".to_string());
        }
        if summary.cwd.is_none() {
            summary
                .parse_warnings
                .push("cursor conversation missing workspace path".to_string());
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_fixture_into_normalized_summary() {
        let raw = include_str!("../../tests/fixtures/session_adapters/claude-code.jsonl");
        let summary = ClaudeCodeAdapter.parse_raw("/fixtures/claude-code.jsonl", raw);

        assert_eq!(summary.adapter_id, "claude-code");
        assert_eq!(summary.stable_id.as_deref(), Some("claude-session-1"));
        assert_eq!(summary.cwd.as_deref(), Some("/repo/codevetter"));
        assert_eq!(summary.git_branch.as_deref(), Some("main"));
        assert_eq!(summary.message_count, 3);
        assert_eq!(summary.total_input_tokens, 135);
        assert_eq!(summary.total_output_tokens, 40);
        assert_eq!(summary.cache_read_tokens, 25);
        assert_eq!(summary.cache_creation_tokens, 10);
        assert_eq!(summary.compaction_count, 1);
        assert_eq!(summary.day_counts.get("2026-06-12"), Some(&3));
        assert!(summary.parse_warnings.is_empty());
    }

    #[test]
    fn parses_codex_fixture_into_normalized_summary() {
        let raw = include_str!("../../tests/fixtures/session_adapters/codex.jsonl");
        let summary = CodexAdapter.parse_raw("/fixtures/codex.jsonl", raw);

        assert_eq!(summary.adapter_id, "codex");
        assert_eq!(summary.stable_id.as_deref(), Some("codex-session-1"));
        assert_eq!(summary.cwd.as_deref(), Some("/repo/codevetter"));
        assert_eq!(summary.git_branch.as_deref(), Some("feature/adapter"));
        assert_eq!(summary.model_used.as_deref(), Some("o3"));
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.total_input_tokens, 500);
        assert_eq!(summary.total_output_tokens, 150);
        assert_eq!(summary.cache_read_tokens, 100);
        assert_eq!(summary.day_counts.get("2026-06-12"), Some(&2));
        assert!(summary.parse_warnings.is_empty());
    }

    #[test]
    fn parses_cursor_fixture_into_normalized_summary() {
        let raw = include_str!("../../tests/fixtures/session_adapters/cursor.json");
        let summary = CursorAdapter.parse_raw("/fixtures/cursor.json", raw);

        assert_eq!(summary.adapter_id, "cursor");
        assert_eq!(summary.stable_id.as_deref(), Some("cursor-composer-1"));
        assert_eq!(summary.cwd.as_deref(), Some("/repo/codevetter"));
        assert_eq!(summary.model_used.as_deref(), Some("cursor-small"));
        assert_eq!(summary.message_count, 2);
        assert_eq!(
            summary.first_timestamp.as_deref(),
            Some("2026-06-12T16:00:00+00:00")
        );
        assert_eq!(
            summary.last_timestamp.as_deref(),
            Some("2026-06-12T16:02:00+00:00")
        );
        assert_eq!(summary.day_counts.get("2026-06-12"), Some(&2));
        assert!(summary.parse_warnings.is_empty());
    }

    #[test]
    fn malformed_adapter_input_degrades_to_parse_warning() {
        let summary = CodexAdapter.parse_raw("/fixtures/bad.jsonl", "{not-json");

        assert_eq!(summary.message_count, 0);
        assert!(summary
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("not valid JSON")));
        assert!(summary
            .parse_warnings
            .iter()
            .any(|warning| warning.contains("missing session_meta id")));
    }
}
