//! Rust-native `Brain` impl that spawns claude / codex CLIs directly,
//! mirroring local-ai's per-provider args + JSON-stream parsing. Used in
//! the shipped DMG (and dev mode) — no Node runtime, no HTTP gateway.
//!
//! Performance tricks:
//!   - claude: `--bare` (when ANTHROPIC_API_KEY is set) skips auto-discovery
//!     of hooks, skills, plugins, MCP, CLAUDE.md → ~500ms-2s saved per spawn.
//!     We can't use it under OAuth/keychain auth (subscription users), so
//!     it's opt-in via env-var presence.
//!   - claude: session reuse via `--resume <session_id>`. After the first
//!     step we capture the session id from stream-json events and resume on
//!     every subsequent step, so the conversation prefix (system prompt +
//!     accumulated history) lives in Anthropic's prompt cache. Per their
//!     numbers, cached input is 90% cheaper and noticeably faster.
//!   - On a resumed turn we send only the NEW browser state — no history
//!     list, no goal restatement — since claude already has all of that.
//!
//! Stays in sync with `../local-ai/index.mjs`:
//!   - claude: `-p --output-format stream-json --verbose --system-prompt SYS`
//!     prompt via stdin; collect text from `assistant.message.content`
//!     and `content_block_delta.delta.text`.
//!   - codex:  `exec --json [-i FILE…]` with system prompt embedded in prompt
//!     body; collect text from `item.completed → agent_message`.

use std::process::Stdio;
use std::sync::Mutex;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::brain::{Brain, BrainContext};
use super::prompts::system_prompt_for_goal;
use super::types::AgentAction;

pub struct CliBrain {
    pub provider: String,
    pub model: Option<String>,
    /// Captured from the first `claude -p` invocation's stream-json events.
    /// Subsequent calls pass `--resume <id>` to keep the same conversation
    /// (and hit Anthropic's prompt cache on the prefix).
    session_id: Mutex<Option<String>>,
    /// `claude --bare` skips auto-discovery overhead but requires API-key
    /// auth (it doesn't read OAuth/keychain). Enabled when ANTHROPIC_API_KEY
    /// is set so subscription users keep working.
    bare_mode: bool,
}

impl CliBrain {
    pub fn new(provider: String, model: Option<String>) -> Self {
        let bare_mode = std::env::var_os("ANTHROPIC_API_KEY").is_some();
        Self {
            provider,
            model,
            session_id: Mutex::new(None),
            bare_mode,
        }
    }

    #[cfg(test)]
    pub fn current_session_id(&self) -> Option<String> {
        self.session_id.lock().unwrap().clone()
    }
}

impl Brain for CliBrain {
    async fn next_action(&self, ctx: BrainContext<'_>) -> Result<AgentAction, String> {
        let text = match self.provider.as_str() {
            "claude" => self.spawn_claude(&ctx).await?,
            "codex" => spawn_codex(&ctx, self.model.as_deref()).await?,
            other => {
                return Err(format!(
                    "CliBrain: provider `{other}` not supported in the bundled brain. \
                     Run local-ai if you need gemini."
                ))
            }
        };
        super::brain::extract_action(&text)
    }
}

fn format_user_message_initial(ctx: &BrainContext<'_>) -> String {
    let mut buf = String::new();
    if !ctx.history.is_empty() {
        buf.push_str("Previous steps:\n");
        for (i, line) in ctx.history.iter().enumerate() {
            buf.push_str(&format!("  {}. {}\n", i + 1, line));
        }
        buf.push('\n');
    }
    buf.push_str(&format!("Current URL: {}\n", ctx.url));
    buf.push_str(&format!("Page title: {}\n\n", ctx.page_title));
    buf.push_str("Visible interactable elements:\n");
    buf.push_str(ctx.accessibility_tree);
    buf.push_str("\n\nReturn the next action as a JSON object on its own line.");
    buf
}

/// Followup prompt for a resumed claude session: claude already has the
/// goal, persona, and prior steps in conversation memory, so we send only
/// the new browser state. Much cheaper, much faster — and still all the
/// model needs to pick the next action.
fn format_user_message_followup(ctx: &BrainContext<'_>) -> String {
    let mut buf = String::new();
    buf.push_str(&format!("Current URL: {}\n", ctx.url));
    buf.push_str(&format!("Page title: {}\n\n", ctx.page_title));
    buf.push_str("Visible interactable elements:\n");
    buf.push_str(ctx.accessibility_tree);
    buf.push_str("\n\nNext action JSON?");
    buf
}

/// codex doesn't (yet) expose a session-resume flag on `exec --json`, so
/// every call gets the full system+history prompt. Mirrors local-ai exactly.
fn build_codex_prompt(ctx: &BrainContext<'_>) -> String {
    let sys = system_prompt_for_goal(ctx.goal, ctx.persona);
    let body = format_user_message_initial(ctx);
    format!("System instructions: {sys}\n\nUser: {body}")
}

impl CliBrain {
    async fn spawn_claude(&self, ctx: &BrainContext<'_>) -> Result<String, String> {
        let session = self.session_id.lock().unwrap().clone();
        let is_followup = session.is_some();

        let prompt = if is_followup {
            format_user_message_followup(ctx)
        } else {
            format!("User: {}", format_user_message_initial(ctx))
        };

        let mut cmd = Command::new("claude");
        cmd.args(["-p", "--output-format", "stream-json", "--verbose"]);
        if self.bare_mode {
            cmd.arg("--bare");
        }
        if let Some(m) = &self.model {
            cmd.args(["--model", m.as_str()]);
        }
        if let Some(id) = &session {
            cmd.args(["--resume", id.as_str()]);
        } else {
            let system_prompt = system_prompt_for_goal(ctx.goal, ctx.persona);
            cmd.args(["--system-prompt", &system_prompt]);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("spawn claude CLI: {e}. Is `claude` on PATH?"))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| format!("claude stdin write: {e}"))?;
            let _ = stdin.shutdown().await;
        }

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "claude: no stdout pipe".to_string())?;
        let mut lines = BufReader::new(stdout).lines();
        let mut assembled = String::new();
        let mut seen_session_id: Option<String> = None;
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("read claude stdout: {e}"))?
        {
            parse_claude_line(&line, &mut assembled);
            if seen_session_id.is_none() {
                seen_session_id = extract_session_id(&line);
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| format!("wait claude: {e}"))?;
        if !status.success() {
            return Err(format!("claude exited with status {status}"));
        }

        // Only persist the session id when this was the FIRST call. On
        // followups the id is the resumed one and is already stored. (Some
        // claude versions emit a fresh id per resumed call; sticking with
        // the original keeps the conversation linear.)
        if !is_followup {
            if let Some(id) = seen_session_id {
                *self.session_id.lock().unwrap() = Some(id);
            }
        }

        Ok(assembled)
    }
}

async fn spawn_codex(ctx: &BrainContext<'_>, model: Option<&str>) -> Result<String, String> {
    let prompt = build_codex_prompt(ctx);

    let mut cmd = Command::new("codex");
    cmd.args(["exec", "--json"]);
    if let Some(m) = model {
        cmd.args(["--model", m]);
    }
    if let Some(path) = ctx.screenshot_path {
        cmd.arg("-i").arg(path);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn codex CLI: {e}. Is `codex` on PATH?"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("codex stdin write: {e}"))?;
        let _ = stdin.shutdown().await;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "codex: no stdout pipe".to_string())?;
    let mut lines = BufReader::new(stdout).lines();
    let mut assembled = String::new();
    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("read codex stdout: {e}"))?
    {
        parse_codex_line(&line, &mut assembled);
    }

    let status = child.wait().await.map_err(|e| format!("wait codex: {e}"))?;
    if !status.success() {
        return Err(format!("codex exited with status {status}"));
    }
    Ok(assembled)
}

/// Append any text fragments from one claude stream-json line into the
/// assembled buffer. Stays tolerant of unrelated event types.
pub(crate) fn parse_claude_line(line: &str, out: &mut String) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => {
            if let Some(content) = v["message"]["content"].as_array() {
                for block in content {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            out.push_str(text);
                        }
                    }
                }
            }
        }
        Some("content_block_delta") => {
            if let Some(text) = v["delta"]["text"].as_str() {
                out.push_str(text);
            }
        }
        _ => {}
    }
}

/// Pull `session_id` off any claude stream-json event. Per Anthropic's
/// headless docs, system/init is first and includes session_id; the field
/// also appears on later events so we'll catch it even if init parsing
/// drifts. Returns None for lines without the field.
pub(crate) fn extract_session_id(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    v.get("session_id")
        .and_then(|s| s.as_str())
        .map(String::from)
}

/// Append any text fragments from one codex `exec --json` line into the
/// assembled buffer. We only care about `item.completed → agent_message`.
pub(crate) fn parse_codex_line(line: &str, out: &mut String) {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if v.get("type").and_then(|t| t.as_str()) != Some("item.completed") {
        return;
    }
    if v["item"]["type"].as_str() != Some("agent_message") {
        return;
    }
    if let Some(text) = v["item"]["text"].as_str() {
        out.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_for(history: &[String]) -> BrainContext<'_> {
        BrainContext {
            goal: "find the price",
            persona: None,
            history,
            url: "https://x.test",
            page_title: "X",
            accessibility_tree: "0 button \"Pricing\" #p",
            screenshot_path: None,
        }
    }

    #[test]
    fn parse_claude_assistant_text_block() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}"#;
        let mut out = String::new();
        parse_claude_line(line, &mut out);
        assert_eq!(out, "hello");
    }

    #[test]
    fn parse_claude_content_block_delta() {
        let line = r#"{"type":"content_block_delta","delta":{"text":"world"}}"#;
        let mut out = String::new();
        parse_claude_line(line, &mut out);
        assert_eq!(out, "world");
    }

    #[test]
    fn parse_claude_assistant_skips_non_text_blocks() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"x"}]}}"#;
        let mut out = String::new();
        parse_claude_line(line, &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn parse_claude_ignores_unknown_types() {
        let line = r#"{"type":"system","message":"warming up"}"#;
        let mut out = String::new();
        parse_claude_line(line, &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn parse_claude_tolerates_malformed_lines() {
        let mut out = String::new();
        parse_claude_line("not json", &mut out);
        parse_claude_line("", &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn extract_session_id_from_system_init() {
        let line = r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"sonnet"}"#;
        assert_eq!(extract_session_id(line).as_deref(), Some("abc-123"));
    }

    #[test]
    fn extract_session_id_returns_none_when_absent() {
        let line = r#"{"type":"content_block_delta","delta":{"text":"x"}}"#;
        assert!(extract_session_id(line).is_none());
    }

    #[test]
    fn extract_session_id_tolerates_malformed_input() {
        assert!(extract_session_id("not-json").is_none());
        assert!(extract_session_id("").is_none());
    }

    #[test]
    fn parse_codex_agent_message() {
        let line = r#"{"type":"item.completed","item":{"type":"agent_message","text":"hi"}}"#;
        let mut out = String::new();
        parse_codex_line(line, &mut out);
        assert_eq!(out, "hi");
    }

    #[test]
    fn parse_codex_skips_non_agent_messages() {
        let line = r#"{"type":"item.completed","item":{"type":"reasoning","text":"thinking"}}"#;
        let mut out = String::new();
        parse_codex_line(line, &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn parse_codex_ignores_non_completion_events() {
        let line = r#"{"type":"item.started","item":{"type":"agent_message"}}"#;
        let mut out = String::new();
        parse_codex_line(line, &mut out);
        assert_eq!(out, "");
    }

    #[test]
    fn followup_prompt_strips_history_and_goal() {
        let history = vec!["clicked something".to_string()];
        let ctx = ctx_for(&history);
        let followup = format_user_message_followup(&ctx);
        assert!(
            !followup.contains("Previous steps"),
            "history should be in claude's session memory, not the new turn"
        );
        assert!(
            !followup.contains("find the price"),
            "goal lives in the resumed session prompt"
        );
        assert!(followup.contains("Current URL"));
        assert!(followup.contains("Pricing"));
    }

    #[test]
    fn initial_prompt_includes_history_when_present() {
        let history = vec!["scrolled 600px".to_string()];
        let ctx = ctx_for(&history);
        let initial = format_user_message_initial(&ctx);
        assert!(initial.contains("Previous steps:"), "{initial}");
        assert!(initial.contains("scrolled 600px"));
    }

    #[test]
    fn codex_prompt_embeds_system_block() {
        let ctx = ctx_for(&[]);
        let prompt = build_codex_prompt(&ctx);
        assert!(prompt.starts_with("System instructions: "));
        assert!(prompt.contains("Current URL: https://x.test"));
        assert!(prompt.contains("\n\nUser: "));
    }

    /// End-to-end smoke against the real `claude` CLI. Ignored by default.
    #[tokio::test]
    #[ignore]
    async fn e2e_claude_returns_text() {
        let brain = CliBrain::new("claude".into(), None);
        let ctx = BrainContext {
            goal: "return the literal text DONE",
            persona: None,
            history: &[],
            url: "https://example.com",
            page_title: "Example",
            accessibility_tree: "(nothing)",
            screenshot_path: None,
        };
        let text = brain.spawn_claude(&ctx).await.expect("spawn claude");
        assert!(!text.is_empty(), "expected non-empty response");
    }

    /// Verifies two calls in a row reuse the same claude session via
    /// `--resume`. The first call captures session_id; the second observes
    /// it before spawning.
    #[tokio::test]
    #[ignore]
    async fn e2e_claude_reuses_session_across_calls() {
        let brain = CliBrain::new("claude".into(), None);
        let ctx = BrainContext {
            goal: "answer the user, then on the next turn answer again",
            persona: None,
            history: &[],
            url: "https://example.com",
            page_title: "Example",
            accessibility_tree: "(none)",
            screenshot_path: None,
        };
        let _ = brain.spawn_claude(&ctx).await.expect("first turn");
        let id_after_first = brain.current_session_id();
        assert!(id_after_first.is_some(), "session_id should be captured");

        let _ = brain.spawn_claude(&ctx).await.expect("second turn");
        let id_after_second = brain.current_session_id();
        assert_eq!(
            id_after_first, id_after_second,
            "session should remain stable across resumed calls"
        );
    }

    /// End-to-end smoke against the real `codex` CLI. Ignored by default.
    #[tokio::test]
    #[ignore]
    async fn e2e_codex_returns_text() {
        let ctx = BrainContext {
            goal: "respond with the literal text DONE and nothing else",
            persona: None,
            history: &[],
            url: "https://example.com",
            page_title: "Example",
            accessibility_tree: "(nothing)",
            screenshot_path: None,
        };
        let text = spawn_codex(&ctx, None).await.expect("spawn codex");
        assert!(!text.is_empty(), "expected non-empty response");
    }
}
