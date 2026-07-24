use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use super::agent_terminal::{resolve_live_agent_session_identity, AgentTerminalEvent};

pub const NATIVE_ISLAND_PROTOCOL_VERSION: u16 = 1;
pub const NATIVE_ISLAND_MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_CHARS: usize = 256;
const MAX_SESSIONS: usize = 64;
const MAX_RECEIPTS: usize = 200;
const MAX_RECEIPT_STORAGE_BYTES: usize = 256 * 1024;
const FOCUS_EVENT: &str = "native-agent-island-focus";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NativeAgentStatus {
    Working,
    NeedsHelp,
    Failed,
    Completed,
    Paused,
    Disconnected,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAgentCapabilities {
    pub can_focus: bool,
    pub can_reply: bool,
    pub can_approve: bool,
    pub can_deny: bool,
    pub can_snooze: bool,
    pub can_dismiss: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAgentSession {
    pub session_id: String,
    pub event_id: String,
    pub provider: String,
    pub project: String,
    pub status: NativeAgentStatus,
    pub reason: String,
    pub confirmed: bool,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub capabilities: NativeAgentCapabilities,
    #[serde(skip)]
    request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeSpeechSettings {
    pub muted: bool,
    pub completion_enabled: bool,
    pub attention_enabled: bool,
    pub failure_enabled: bool,
    pub codex_voice: Option<String>,
    pub claude_voice: Option<String>,
    pub rate: f32,
    pub volume: f32,
    pub quiet_hours_start: Option<u8>,
    pub quiet_hours_end: Option<u8>,
    pub cooldown_seconds: u64,
}

impl Default for NativeSpeechSettings {
    fn default() -> Self {
        Self {
            muted: false,
            completion_enabled: true,
            attention_enabled: true,
            failure_enabled: true,
            codex_voice: None,
            claude_voice: None,
            rate: 0.48,
            volume: 0.8,
            quiet_hours_start: None,
            quiet_hours_end: None,
            cooldown_seconds: 30,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NativeIslandSettings {
    pub enabled: bool,
    pub speech: NativeSpeechSettings,
}

#[derive(Debug, Clone, Serialize)]
struct NativeIslandSnapshot {
    sessions: Vec<NativeAgentSession>,
    settings: NativeIslandSettings,
    preview: bool,
}

#[derive(Debug, Clone, Serialize)]
struct NativeOutboundEnvelope {
    v: u16,
    seq: u64,
    sent_at_ms: u64,
    kind: String,
    payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeIntentEnvelope {
    v: u16,
    seq: u64,
    sent_at_ms: u64,
    kind: String,
    payload: NativeIntent,
}

#[derive(Debug, Clone, Deserialize)]
struct NativeIntent {
    action: String,
    session_id: Option<String>,
    event_id: Option<String>,
    value: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct NativeFocusEvent {
    session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeActionReceipt {
    pub at_ms: u64,
    #[serde(default)]
    pub provider: Option<String>,
    pub session_id: Option<String>,
    pub event_id: Option<String>,
    pub action: String,
    pub disposition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeIslandStatus {
    pub enabled: bool,
    pub connected: bool,
    pub session_count: usize,
    pub helper_path: Option<String>,
    pub last_error: Option<String>,
    pub receipts: Vec<NativeActionReceipt>,
}

#[derive(Debug, Clone)]
struct PendingNativeAction {
    session_id: String,
    event_id: String,
    provider: String,
    request_id: Option<String>,
    capabilities: NativeAgentCapabilities,
}

#[derive(Default)]
struct NativeIslandRuntime {
    settings: NativeIslandSettings,
    sessions: HashMap<String, NativeAgentSession>,
    pending: HashMap<String, PendingNativeAction>,
    consumed: HashSet<String>,
    receipts: VecDeque<NativeActionReceipt>,
    child: Option<Arc<Mutex<Child>>>,
    stdin: Option<ChildStdin>,
    helper_path: Option<PathBuf>,
    next_seq: u64,
    last_error: Option<String>,
}

fn runtime() -> &'static Mutex<NativeIslandRuntime> {
    static RUNTIME: OnceLock<Mutex<NativeIslandRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(NativeIslandRuntime::default()))
}

#[tauri::command]
pub fn set_native_agent_island_enabled(
    app: AppHandle,
    enabled: bool,
) -> Result<NativeIslandStatus, String> {
    configure_enabled(&app, enabled)?;
    get_native_agent_island_status()
}

#[tauri::command]
pub fn get_native_agent_island_status() -> Result<NativeIslandStatus, String> {
    let mut state = runtime()
        .lock()
        .map_err(|error| format!("native island state lock poisoned: {error}"))?;
    refresh_child_state(&mut state);
    Ok(status_from_runtime(&state))
}

#[tauri::command]
pub fn preview_native_agent_island(app: AppHandle) -> Result<NativeIslandStatus, String> {
    {
        let mut state = runtime()
            .lock()
            .map_err(|error| format!("native island state lock poisoned: {error}"))?;
        ensure_helper_started(&app, &mut state)?;
        send_snapshot_locked(&mut state, true)?;
    }
    get_native_agent_island_status()
}

pub fn configure_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut state = runtime()
        .lock()
        .map_err(|error| format!("native island state lock poisoned: {error}"))?;
    state.settings.enabled = enabled;
    if !enabled {
        stop_helper(&mut state);
        return Ok(());
    }
    if !state.sessions.is_empty() {
        ensure_helper_started(app, &mut state)?;
        send_snapshot_locked(&mut state, false)?;
    }
    Ok(())
}

pub fn apply_preference(app: &AppHandle, key: &str, value: &str) {
    let result = match key {
        "native_agent_island_enabled" => configure_enabled(app, value == "true"),
        _ => {
            let Ok(mut state) = runtime().lock() else {
                return;
            };
            if apply_speech_preference(&mut state.settings.speech, key, value) {
                if state.settings.enabled && state.stdin.is_some() {
                    send_snapshot_locked(&mut state, false)
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        }
    };
    if let Err(error) = result {
        if let Ok(mut state) = runtime().lock() {
            state.last_error = Some(error);
        }
    }
}

pub fn hydrate_preferences(app: &AppHandle, preferences: &HashMap<String, String>) {
    if let Ok(mut state) = runtime().lock() {
        state.receipts = load_receipts(app).into_iter().take(MAX_RECEIPTS).collect();
        for (key, value) in preferences {
            if key != "native_agent_island_enabled" {
                apply_speech_preference(&mut state.settings.speech, key, value);
            }
        }
    }
    let enabled = preferences
        .get("native_agent_island_enabled")
        .is_some_and(|value| value == "true");
    let _ = configure_enabled(app, enabled);
}

fn apply_speech_preference(settings: &mut NativeSpeechSettings, key: &str, value: &str) -> bool {
    match key {
        "native_agent_island_speech_muted" => settings.muted = value == "true",
        "native_agent_island_speak_completion" => settings.completion_enabled = value == "true",
        "native_agent_island_speak_attention" => settings.attention_enabled = value == "true",
        "native_agent_island_speak_failure" => settings.failure_enabled = value == "true",
        "native_agent_island_codex_voice" => {
            settings.codex_voice = non_empty_bounded(value, MAX_IDENTIFIER_CHARS)
        }
        "native_agent_island_claude_voice" => {
            settings.claude_voice = non_empty_bounded(value, MAX_IDENTIFIER_CHARS)
        }
        "native_agent_island_speech_rate" => {
            settings.rate = parse_unit_float(value).unwrap_or(settings.rate)
        }
        "native_agent_island_speech_volume" => {
            settings.volume = parse_unit_float(value).unwrap_or(settings.volume)
        }
        "native_agent_island_speech_cooldown" => {
            settings.cooldown_seconds = value
                .parse::<u64>()
                .ok()
                .filter(|seconds| (5..=600).contains(seconds))
                .unwrap_or(settings.cooldown_seconds)
        }
        "native_agent_island_quiet_start" => {
            settings.quiet_hours_start = parse_hour(value);
        }
        "native_agent_island_quiet_end" => {
            settings.quiet_hours_end = parse_hour(value);
        }
        _ => return false,
    }
    true
}

fn parse_unit_float(value: &str) -> Option<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|value| (0.0..=1.0).contains(value))
}

fn parse_hour(value: &str) -> Option<u8> {
    if value.trim().is_empty() {
        return None;
    }
    value.parse::<u8>().ok().filter(|hour| *hour < 24)
}

pub fn ingest_agent_terminal_event(app: &AppHandle, event: &AgentTerminalEvent) {
    let Ok(mut state) = runtime().lock() else {
        return;
    };

    let now = current_unix_millis();
    match event.kind.as_str() {
        "started" => {
            if state.sessions.len() >= MAX_SESSIONS
                && !state.sessions.contains_key(&event.session_id)
            {
                return;
            }
            if let Ok(Some(identity)) = resolve_live_agent_session_identity(&event.session_id) {
                let event_id = bounded_event_id(&event.session_id, "started", event.seq);
                state.sessions.insert(
                    event.session_id.clone(),
                    NativeAgentSession {
                        session_id: event.session_id.clone(),
                        event_id: event_id.clone(),
                        provider: identity.provider,
                        project: project_label(&identity.project_path),
                        status: NativeAgentStatus::Working,
                        reason: "Running".to_string(),
                        confirmed: true,
                        started_at_ms: now,
                        updated_at_ms: now,
                        capabilities: default_capabilities(),
                        request_id: None,
                    },
                );
                refresh_pending_for_session(&mut state, &event.session_id);
            }
        }
        "agent_event" => update_structured_event(&mut state, event, now),
        "error" => update_terminal_state(
            &mut state,
            event,
            now,
            NativeAgentStatus::Failed,
            "Agent failed",
            true,
        ),
        "exit" => update_terminal_state(
            &mut state,
            event,
            now,
            if event.success == Some(true) {
                NativeAgentStatus::Completed
            } else {
                NativeAgentStatus::Failed
            },
            if event.intentional_stop == Some(true) {
                "Stopped"
            } else if event.success == Some(true) {
                "Completed"
            } else {
                "Agent exited"
            },
            true,
        ),
        _ => {}
    }

    if !state.settings.enabled || state.sessions.is_empty() {
        return;
    }
    if ensure_helper_started(app, &mut state).is_ok() {
        if let Err(error) = send_snapshot_locked(&mut state, false) {
            state.last_error = Some(error);
        }
    }
}

fn update_structured_event(state: &mut NativeIslandRuntime, event: &AgentTerminalEvent, now: u64) {
    let Some(data) = event.data.as_deref() else {
        return;
    };
    let Ok(payload) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let Some(kind) = payload.get("event").and_then(Value::as_str) else {
        return;
    };
    let request_id = payload
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(MAX_IDENTIFIER_CHARS).collect::<String>());
    let reason = payload
        .get("summary")
        .or_else(|| payload.get("response"))
        .and_then(Value::as_str)
        .map(bounded_reason)
        .unwrap_or_else(|| structured_reason(kind).to_string());
    let (status, confirmed) = match kind {
        "permission_request" | "question_asked" => (NativeAgentStatus::NeedsHelp, true),
        "attention_resolved" => (NativeAgentStatus::Working, true),
        "stop_failure" | "tool_error" | "failure" => (NativeAgentStatus::Failed, true),
        "stop" | "idle_prompt" | "turn_complete" => (NativeAgentStatus::Completed, true),
        "session_end" => (NativeAgentStatus::Paused, true),
        "session_start" | "prompt_submit" | "tool_start" | "tool_complete" => {
            (NativeAgentStatus::Working, true)
        }
        _ => return,
    };
    update_terminal_state(state, event, now, status, &reason, confirmed);
    let mut capabilities = default_capabilities();
    if kind == "question_asked"
        && request_id.is_some()
        && payload.get("agent").and_then(Value::as_str) == Some("claude")
    {
        capabilities.can_reply = true;
    }
    if kind == "permission_request"
        && payload.get("agent").and_then(Value::as_str) == Some("claude")
        && request_id.as_deref().is_some_and(|request_id| {
            super::agent_terminal::claude_permission_response_available(
                &event.session_id,
                request_id,
            )
        })
    {
        capabilities.can_approve = true;
        capabilities.can_deny = true;
    }
    if payload.get("source").and_then(Value::as_str) == Some("codex-app-server") {
        if let Some(request_id) = request_id.as_deref() {
            let (can_reply, can_approve, can_deny) =
                super::codex_app_server::pending_capabilities(&event.session_id, request_id);
            capabilities.can_reply = can_reply;
            capabilities.can_approve = can_approve;
            capabilities.can_deny = can_deny;
        }
    }
    if let Some(session) = state.sessions.get_mut(&event.session_id) {
        if let Some(request_id) = request_id {
            session.event_id = bounded_request_event_id(&event.session_id, kind, &request_id);
            session.request_id = Some(request_id);
        } else {
            session.request_id = None;
        }
        session.capabilities = capabilities;
    }
    refresh_pending_for_session(state, &event.session_id);
}

fn update_terminal_state(
    state: &mut NativeIslandRuntime,
    event: &AgentTerminalEvent,
    now: u64,
    status: NativeAgentStatus,
    reason: &str,
    confirmed: bool,
) {
    let Some(session) = state.sessions.get_mut(&event.session_id) else {
        return;
    };
    session.status = status;
    session.reason = bounded_reason(reason);
    session.confirmed = confirmed;
    session.updated_at_ms = now;
    session.event_id = bounded_event_id(&event.session_id, &event.kind, event.seq);
    session.capabilities = default_capabilities();
    session.request_id = None;
    refresh_pending_for_session(state, &event.session_id);
}

fn structured_reason(kind: &str) -> &'static str {
    match kind {
        "permission_request" => "Needs approval",
        "question_asked" => "Waiting for your answer",
        "attention_resolved" => "Response accepted; working",
        "stop_failure" => "Turn failed",
        "tool_error" => "Tool failed",
        "stop" | "turn_complete" => "Turn completed",
        "idle_prompt" => "Ready for your next message",
        "session_end" => "Session ended",
        "session_start" => "Session started",
        "prompt_submit" => "Working",
        "tool_start" => "Using a tool",
        "tool_complete" => "Tool completed",
        _ => "Updated",
    }
}

fn default_capabilities() -> NativeAgentCapabilities {
    NativeAgentCapabilities {
        can_focus: true,
        can_reply: false,
        can_approve: false,
        can_deny: false,
        can_snooze: true,
        can_dismiss: true,
    }
}

fn refresh_pending_for_session(state: &mut NativeIslandRuntime, session_id: &str) {
    state
        .pending
        .retain(|_, pending| pending.session_id != session_id);
    let Some(session) = state.sessions.get(session_id) else {
        return;
    };
    state.pending.insert(
        session.event_id.clone(),
        PendingNativeAction {
            session_id: session.session_id.clone(),
            event_id: session.event_id.clone(),
            provider: session.provider.clone(),
            request_id: session.request_id.clone(),
            capabilities: session.capabilities.clone(),
        },
    );
}

fn ensure_helper_started(app: &AppHandle, state: &mut NativeIslandRuntime) -> Result<(), String> {
    refresh_child_state(state);
    if state.stdin.is_some() {
        return Ok(());
    }

    let helper_path = resolve_helper_path()?;
    let mut child = Command::new(&helper_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("launch native agent island: {error}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "native agent island stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "native agent island stdout unavailable".to_string())?;
    let child = Arc::new(Mutex::new(child));

    state.helper_path = Some(helper_path);
    state.stdin = Some(stdin);
    state.child = Some(Arc::clone(&child));
    state.last_error = None;

    let reader_app = app.clone();
    let _ = thread::Builder::new()
        .name("native-agent-island-events".to_string())
        .spawn(move || read_helper_events(reader_app, stdout));
    let _ = thread::Builder::new()
        .name("native-agent-island-monitor".to_string())
        .spawn(move || monitor_helper(child));
    Ok(())
}

fn resolve_helper_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEVETTER_AGENT_ISLAND_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }

    let executable_name = if cfg!(windows) {
        "codevetter-agent-island.exe"
    } else {
        "codevetter-agent-island"
    };
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            let bundled = parent.join(executable_name);
            if bundled.is_file() {
                return Ok(bundled);
            }
        }
    }

    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../native/AgentIsland/.build/debug")
        .join(executable_name);
    if development.is_file() {
        return Ok(development);
    }
    Err(
        "Native Agent Island helper is not built. Run pnpm prepare:agent-island from apps/desktop."
            .to_string(),
    )
}

fn monitor_helper(child: Arc<Mutex<Child>>) {
    loop {
        let finished = child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok().flatten())
            .is_some();
        if finished {
            if let Ok(mut state) = runtime().lock() {
                if state
                    .child
                    .as_ref()
                    .is_some_and(|active| Arc::ptr_eq(active, &child))
                {
                    state.child = None;
                    state.stdin = None;
                    state.last_error = Some("Native Agent Island helper exited".to_string());
                }
            }
            return;
        }
        thread::sleep(std::time::Duration::from_secs(1));
    }
}

fn read_helper_events(app: AppHandle, stdout: std::process::ChildStdout) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) if line.len() > NATIVE_ISLAND_MAX_MESSAGE_BYTES => {
                record_rejected_intent(Some(&app), None, None, "oversized", "rejected");
            }
            Ok(_) => handle_helper_line(&app, line.trim_end()),
            Err(_) => return,
        }
    }
}

fn handle_helper_line(app: &AppHandle, line: &str) {
    if is_render_ack(line) {
        return;
    }
    let envelope = match parse_helper_intent(line) {
        Ok(envelope) => envelope,
        Err(reason) => {
            record_rejected_intent(Some(app), None, None, reason, "rejected");
            return;
        }
    };
    if envelope.payload.action.trim().is_empty() {
        record_rejected_intent(
            Some(app),
            envelope.payload.session_id,
            envelope.payload.event_id,
            &envelope.payload.action,
            "unsupported",
        );
        return;
    }
    let provider = envelope
        .payload
        .session_id
        .as_deref()
        .and_then(|session_id| {
            runtime().lock().ok().and_then(|state| {
                state
                    .sessions
                    .get(session_id)
                    .map(|session| session.provider.clone())
            })
        });
    let result = dispatch_intent(app, &envelope.payload);
    let disposition = if result.is_ok() {
        "accepted"
    } else {
        "rejected"
    };
    push_receipt(
        Some(app),
        NativeActionReceipt {
            at_ms: current_unix_millis(),
            provider,
            session_id: envelope.payload.session_id.clone(),
            event_id: envelope.payload.event_id.clone(),
            action: bounded_reason(&envelope.payload.action),
            disposition: disposition.to_string(),
        },
    );
    send_action_result(
        envelope.seq,
        envelope.payload.session_id.as_deref(),
        envelope.payload.event_id.as_deref(),
        disposition,
        result.err().as_deref(),
    );
}

fn is_render_ack(line: &str) -> bool {
    let Ok(envelope) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    envelope.get("v").and_then(Value::as_u64) == Some(NATIVE_ISLAND_PROTOCOL_VERSION as u64)
        && envelope
            .get("seq")
            .and_then(Value::as_u64)
            .is_some_and(|seq| seq > 0)
        && envelope
            .get("sent_at_ms")
            .and_then(Value::as_u64)
            .is_some_and(|sent_at| sent_at > 0)
        && envelope.get("kind").and_then(Value::as_str) == Some("render_ack")
}

fn parse_helper_intent(line: &str) -> Result<NativeIntentEnvelope, &'static str> {
    if line.is_empty() || line.len() > NATIVE_ISLAND_MAX_MESSAGE_BYTES {
        return Err("oversized");
    }
    let envelope = serde_json::from_str::<NativeIntentEnvelope>(line).map_err(|_| "malformed")?;
    if envelope.v != NATIVE_ISLAND_PROTOCOL_VERSION
        || envelope.kind != "intent"
        || envelope.seq == 0
        || envelope.sent_at_ms == 0
    {
        return Err("unsupported");
    }
    Ok(envelope)
}

fn dispatch_intent(app: &AppHandle, intent: &NativeIntent) -> Result<(), String> {
    let action = intent.action.trim();
    let session_id = bounded_required(intent.session_id.as_deref(), "session_id")?;
    let event_id = bounded_required(intent.event_id.as_deref(), "event_id")?;
    let pending = validate_pending_action(action, &session_id, &event_id)?;

    match action {
        "focus_session" => {
            if !pending.capabilities.can_focus {
                return Err("focus is unavailable for this event".to_string());
            }
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "main CodeVetter window is unavailable".to_string())?;
            window
                .show()
                .map_err(|error| format!("show CodeVetter window: {error}"))?;
            window
                .set_focus()
                .map_err(|error| format!("focus CodeVetter window: {error}"))?;
            app.emit(
                FOCUS_EVENT,
                NativeFocusEvent {
                    session_id: session_id.clone(),
                },
            )
            .map_err(|error| format!("route to Work conversation: {error}"))?;
        }
        "dismiss" => {
            if !pending.capabilities.can_dismiss {
                return Err("dismiss is unavailable for this event".to_string());
            }
            let mut state = runtime()
                .lock()
                .map_err(|error| format!("native island state lock poisoned: {error}"))?;
            state.sessions.remove(&session_id);
            state.pending.remove(&event_id);
            state.consumed.insert(event_id);
            send_snapshot_locked(&mut state, false)?;
        }
        "snooze" => {
            if !pending.capabilities.can_snooze {
                return Err("snooze is unavailable for this event".to_string());
            }
            let mut state = runtime()
                .lock()
                .map_err(|error| format!("native island state lock poisoned: {error}"))?;
            if let Some(session) = state.sessions.get_mut(&session_id) {
                session.status = NativeAgentStatus::Paused;
                session.reason = "Snoozed".to_string();
                session.updated_at_ms = current_unix_millis();
            }
            state.pending.remove(&event_id);
            state.consumed.insert(event_id);
            send_snapshot_locked(&mut state, false)?;
        }
        "submit_reply" => {
            if !pending.capabilities.can_reply {
                return Err("This provider session does not expose a safe inline reply".to_string());
            }
            let value = intent
                .value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "reply is required".to_string())?;
            if value.chars().count() > 4_000 {
                return Err("reply exceeded the native response limit".to_string());
            }
            if pending.provider == "codex" {
                let request_id = pending
                    .request_id
                    .as_deref()
                    .ok_or_else(|| "Codex question identity is unavailable".to_string())?;
                super::codex_app_server::resolve_pending_request(
                    &session_id,
                    request_id,
                    action,
                    Some(value),
                )?;
                consume_native_action(&session_id, &event_id, "Reply sent; waiting for Codex")?;
            } else if pending.provider == "claude" {
                super::agent_terminal::send_agent_terminal_input_from_native(
                    &session_id,
                    &format!("{value}\r"),
                )?;
                consume_native_action(&session_id, &event_id, "Reply sent; waiting for Claude")?;
            } else {
                return Err("This provider session does not expose a safe inline reply".to_string());
            }
        }
        "approve" | "deny" => {
            let request_id = pending
                .request_id
                .as_deref()
                .ok_or_else(|| "permission request identity is unavailable".to_string())?;
            if pending.provider == "codex" {
                super::codex_app_server::resolve_pending_request(
                    &session_id,
                    request_id,
                    action,
                    None,
                )?;
            } else if pending.provider == "claude" {
                super::agent_terminal::resolve_claude_permission_request(
                    &session_id,
                    request_id,
                    action == "approve",
                )?;
            } else {
                return Err(
                    "This provider session does not expose a safe inline decision".to_string(),
                );
            }
            consume_native_action(
                &session_id,
                &event_id,
                if action == "approve" && pending.provider == "codex" {
                    "Approval sent; waiting for Codex"
                } else if action == "approve" {
                    "Approval sent; waiting for Claude"
                } else if pending.provider == "codex" {
                    "Denial sent; waiting for Codex"
                } else {
                    "Denial sent; waiting for Claude"
                },
            )?;
        }
        _ => return Err("unsupported native island action".to_string()),
    }
    Ok(())
}

fn consume_native_action(session_id: &str, event_id: &str, reason: &str) -> Result<(), String> {
    let mut state = runtime()
        .lock()
        .map_err(|error| format!("native island state lock poisoned: {error}"))?;
    state.pending.remove(event_id);
    state.consumed.insert(event_id.to_string());
    if let Some(session) = state.sessions.get_mut(session_id) {
        session.reason = reason.to_string();
        session.capabilities = default_capabilities();
        session.updated_at_ms = current_unix_millis();
    }
    send_snapshot_locked(&mut state, false)
}

fn validate_pending_action(
    action: &str,
    session_id: &str,
    event_id: &str,
) -> Result<PendingNativeAction, String> {
    let state = runtime()
        .lock()
        .map_err(|error| format!("native island state lock poisoned: {error}"))?;
    if state.consumed.contains(event_id) {
        return Err("native island action was already consumed".to_string());
    }
    let pending = state
        .pending
        .get(event_id)
        .ok_or_else(|| "native island event is stale".to_string())?;
    if pending.session_id != session_id || pending.event_id != event_id {
        return Err("native island action identity mismatch".to_string());
    }
    let allowed = match action {
        "focus_session" => pending.capabilities.can_focus,
        "submit_reply" => pending.capabilities.can_reply,
        "approve" => pending.capabilities.can_approve,
        "deny" => pending.capabilities.can_deny,
        "snooze" => pending.capabilities.can_snooze,
        "dismiss" => pending.capabilities.can_dismiss,
        _ => false,
    };
    if !allowed {
        return Err("native island action is not allowed".to_string());
    }
    Ok(pending.clone())
}

fn send_snapshot_locked(state: &mut NativeIslandRuntime, preview: bool) -> Result<(), String> {
    let mut sessions = state.sessions.values().cloned().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        status_priority(&left.status)
            .cmp(&status_priority(&right.status))
            .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
    });
    if preview && sessions.is_empty() {
        let now = current_unix_millis();
        sessions.push(NativeAgentSession {
            session_id: "preview".to_string(),
            event_id: "preview:ready".to_string(),
            provider: "codex".to_string(),
            project: "CodeVetter".to_string(),
            status: NativeAgentStatus::Completed,
            reason: "Native Agent Island is ready".to_string(),
            confirmed: true,
            started_at_ms: now,
            updated_at_ms: now,
            capabilities: NativeAgentCapabilities::default(),
            request_id: None,
        });
    }
    let snapshot = NativeIslandSnapshot {
        sessions,
        settings: state.settings.clone(),
        preview,
    };
    send_envelope_locked(
        state,
        "snapshot",
        serde_json::to_value(snapshot).unwrap_or_default(),
    )
}

fn send_action_result(
    request_seq: u64,
    session_id: Option<&str>,
    event_id: Option<&str>,
    disposition: &str,
    error: Option<&str>,
) {
    let Ok(mut state) = runtime().lock() else {
        return;
    };
    let payload = json!({
        "request_seq": request_seq,
        "session_id": session_id,
        "event_id": event_id,
        "disposition": disposition,
        "error": error.map(bounded_reason),
    });
    let _ = send_envelope_locked(&mut state, "action_result", payload);
}

fn send_envelope_locked(
    state: &mut NativeIslandRuntime,
    kind: &str,
    payload: Value,
) -> Result<(), String> {
    state.next_seq = state.next_seq.saturating_add(1).max(1);
    let envelope = NativeOutboundEnvelope {
        v: NATIVE_ISLAND_PROTOCOL_VERSION,
        seq: state.next_seq,
        sent_at_ms: current_unix_millis(),
        kind: kind.to_string(),
        payload,
    };
    let mut encoded = serde_json::to_vec(&envelope)
        .map_err(|error| format!("serialize native island message: {error}"))?;
    if encoded.len() > NATIVE_ISLAND_MAX_MESSAGE_BYTES {
        return Err("native island message exceeded the protocol limit".to_string());
    }
    encoded.push(b'\n');
    let Some(stdin) = state.stdin.as_mut() else {
        return Err("native island helper is disconnected".to_string());
    };
    if let Err(error) = stdin.write_all(&encoded).and_then(|_| stdin.flush()) {
        state.stdin = None;
        state.child = None;
        return Err(format!("write native island message: {error}"));
    }
    Ok(())
}

fn stop_helper(state: &mut NativeIslandRuntime) {
    state.stdin = None;
    if let Some(child) = state.child.take() {
        if let Ok(mut child) = child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn refresh_child_state(state: &mut NativeIslandRuntime) {
    let exited = state
        .child
        .as_ref()
        .and_then(|child| child.lock().ok())
        .and_then(|mut child| child.try_wait().ok().flatten())
        .is_some();
    if exited {
        state.child = None;
        state.stdin = None;
    }
}

fn status_from_runtime(state: &NativeIslandRuntime) -> NativeIslandStatus {
    NativeIslandStatus {
        enabled: state.settings.enabled,
        connected: state.stdin.is_some(),
        session_count: state.sessions.len(),
        helper_path: state
            .helper_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        last_error: state.last_error.clone(),
        receipts: state.receipts.iter().cloned().collect(),
    }
}

fn push_receipt(app: Option<&AppHandle>, mut receipt: NativeActionReceipt) {
    receipt.provider = receipt
        .provider
        .as_deref()
        .and_then(|value| non_empty_bounded(value, 32));
    receipt.session_id = receipt
        .session_id
        .as_deref()
        .and_then(|value| non_empty_bounded(value, MAX_IDENTIFIER_CHARS));
    receipt.event_id = receipt
        .event_id
        .as_deref()
        .and_then(|value| non_empty_bounded(value, MAX_IDENTIFIER_CHARS));
    receipt.action = bounded_reason(&receipt.action);
    receipt.disposition =
        non_empty_bounded(&receipt.disposition, 32).unwrap_or_else(|| "unknown".to_string());
    let receipts = if let Ok(mut state) = runtime().lock() {
        if state.receipts.len() >= MAX_RECEIPTS {
            state.receipts.pop_front();
        }
        state.receipts.push_back(receipt);
        state.receipts.iter().cloned().collect::<Vec<_>>()
    } else {
        return;
    };
    if let Some(app) = app {
        if let Err(error) = persist_receipts(app, &receipts) {
            if let Ok(mut state) = runtime().lock() {
                state.last_error = Some(error);
            }
        }
    }
}

fn record_rejected_intent(
    app: Option<&AppHandle>,
    session_id: Option<String>,
    event_id: Option<String>,
    action: &str,
    disposition: &str,
) {
    push_receipt(
        app,
        NativeActionReceipt {
            at_ms: current_unix_millis(),
            provider: None,
            session_id,
            event_id,
            action: bounded_reason(action),
            disposition: disposition.to_string(),
        },
    );
}

fn receipt_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|directory| directory.join("native-agent-island-receipts.json"))
}

fn load_receipts(app: &AppHandle) -> Vec<NativeActionReceipt> {
    let Some(path) = receipt_path(app) else {
        return Vec::new();
    };
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    if bytes.len() > MAX_RECEIPT_STORAGE_BYTES {
        return Vec::new();
    }
    serde_json::from_slice::<Vec<NativeActionReceipt>>(&bytes).unwrap_or_default()
}

fn persist_receipts(app: &AppHandle, receipts: &[NativeActionReceipt]) -> Result<(), String> {
    let path =
        receipt_path(app).ok_or_else(|| "Agent Island receipt path is unavailable".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Agent Island receipt directory is unavailable".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("create receipt directory: {error}"))?;
    let bytes =
        serde_json::to_vec(receipts).map_err(|error| format!("serialize receipts: {error}"))?;
    if bytes.len() > MAX_RECEIPT_STORAGE_BYTES {
        return Err("Agent Island receipts exceeded the storage limit".to_string());
    }
    let temporary = path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        current_unix_millis()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&temporary)
        .and_then(|mut file| file.write_all(&bytes).and_then(|_| file.flush()))
        .map_err(|error| format!("write Agent Island receipts: {error}"))?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("publish Agent Island receipts: {error}")
    })
}

fn bounded_required(value: Option<&str>, label: &str) -> Result<String, String> {
    let value = value.unwrap_or_default().trim();
    if value.is_empty() || value.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(format!("{label} is missing or invalid"));
    }
    Ok(value.to_string())
}

fn non_empty_bounded(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(max_chars).collect())
    }
}

fn bounded_reason(value: &str) -> String {
    const LIMIT: usize = 160;
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= LIMIT {
        cleaned
    } else {
        cleaned.chars().take(LIMIT).collect::<String>() + "…"
    }
}

fn bounded_event_id(session_id: &str, kind: &str, seq: Option<u64>) -> String {
    format!(
        "{}:{}:{}",
        session_id.chars().take(120).collect::<String>(),
        kind.chars().take(80).collect::<String>(),
        seq.unwrap_or_default()
    )
}

fn bounded_request_event_id(session_id: &str, kind: &str, request_id: &str) -> String {
    let request_key = format!("{:x}", Sha256::digest(request_id.as_bytes()));
    format!(
        "{}:{}:{}",
        session_id.chars().take(100).collect::<String>(),
        kind.chars().take(50).collect::<String>(),
        request_key
    )
}

fn project_label(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Local project".to_string())
}

fn status_priority(status: &NativeAgentStatus) -> u8 {
    match status {
        NativeAgentStatus::NeedsHelp => 0,
        NativeAgentStatus::Failed => 1,
        NativeAgentStatus::Completed => 2,
        NativeAgentStatus::Working => 3,
        NativeAgentStatus::Paused => 4,
        NativeAgentStatus::Disconnected => 5,
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test lock")
    }

    fn pending(capabilities: NativeAgentCapabilities) -> NativeIslandRuntime {
        let mut state = NativeIslandRuntime::default();
        state.pending.insert(
            "event-1".to_string(),
            PendingNativeAction {
                session_id: "session-1".to_string(),
                event_id: "event-1".to_string(),
                provider: "claude".to_string(),
                request_id: Some("request-1".to_string()),
                capabilities,
            },
        );
        state
    }

    #[test]
    fn rejects_unsupported_and_mismatched_actions() {
        let _guard = test_guard();
        {
            let mut state = runtime().lock().expect("state");
            *state = pending(NativeAgentCapabilities {
                can_focus: true,
                ..NativeAgentCapabilities::default()
            });
        }
        assert!(validate_pending_action("approve", "session-1", "event-1").is_err());
        assert!(validate_pending_action("focus_session", "other", "event-1").is_err());
        assert!(validate_pending_action("focus_session", "session-1", "event-1").is_ok());
    }

    #[test]
    fn rejects_stale_and_consumed_events() {
        let _guard = test_guard();
        {
            let mut state = runtime().lock().expect("state");
            *state = pending(NativeAgentCapabilities {
                can_focus: true,
                ..NativeAgentCapabilities::default()
            });
            state.consumed.insert("event-1".to_string());
        }
        assert!(validate_pending_action("focus_session", "session-1", "event-1").is_err());
        {
            let mut state = runtime().lock().expect("state");
            state.consumed.clear();
            state.pending.clear();
        }
        assert!(validate_pending_action("focus_session", "session-1", "event-1").is_err());
    }

    #[test]
    fn bounds_protocol_content_and_privacy_text() {
        assert!(bounded_required(Some(&"x".repeat(MAX_IDENTIFIER_CHARS + 1)), "id").is_err());
        assert!(bounded_reason(&"word ".repeat(100)).chars().count() <= 161);
        let settings = NativeSpeechSettings::default();
        let serialized = serde_json::to_string(&settings).expect("settings");
        assert!(!serialized.contains("prompt"));
        assert!(!serialized.contains("output"));
        assert!(!serialized.contains("command"));
        assert!(!serialized.contains("diff"));
        let event_id = bounded_request_event_id("session-1", "permission_request", "req:secret");
        assert!(!event_id.contains("secret"));
        assert_eq!(
            event_id,
            bounded_request_event_id("session-1", "permission_request", "req:secret")
        );
    }

    #[test]
    fn event_priority_is_stable() {
        assert!(
            status_priority(&NativeAgentStatus::NeedsHelp)
                < status_priority(&NativeAgentStatus::Failed)
        );
        assert!(
            status_priority(&NativeAgentStatus::Failed)
                < status_priority(&NativeAgentStatus::Completed)
        );
        assert!(
            status_priority(&NativeAgentStatus::Completed)
                < status_priority(&NativeAgentStatus::Working)
        );
    }

    #[test]
    fn pending_action_keeps_the_exact_provider_request_id() {
        let mut state = NativeIslandRuntime::default();
        state.sessions.insert(
            "session-1".to_string(),
            NativeAgentSession {
                session_id: "session-1".to_string(),
                event_id: "session-1:permission_request:req:part:3".to_string(),
                provider: "claude".to_string(),
                project: "CodeVetter".to_string(),
                status: NativeAgentStatus::NeedsHelp,
                reason: "Needs approval".to_string(),
                confirmed: true,
                started_at_ms: 1,
                updated_at_ms: 2,
                capabilities: NativeAgentCapabilities {
                    can_approve: true,
                    ..NativeAgentCapabilities::default()
                },
                request_id: Some("req:part:3".to_string()),
            },
        );
        refresh_pending_for_session(&mut state, "session-1");
        assert_eq!(
            state
                .pending
                .get("session-1:permission_request:req:part:3")
                .and_then(|pending| pending.request_id.as_deref()),
            Some("req:part:3")
        );
    }

    #[test]
    fn helper_failure_is_isolated_from_owned_agent_sessions() {
        let now = current_unix_millis();
        let session = NativeAgentSession {
            session_id: "session-1".to_string(),
            event_id: "event-1".to_string(),
            provider: "codex".to_string(),
            project: "CodeVetter".to_string(),
            status: NativeAgentStatus::Working,
            reason: "Running".to_string(),
            confirmed: true,
            started_at_ms: now,
            updated_at_ms: now,
            capabilities: default_capabilities(),
            request_id: None,
        };
        let mut state = NativeIslandRuntime::default();
        state.sessions.insert(session.session_id.clone(), session);

        assert!(matches!(
            parse_helper_intent("{malformed"),
            Err("malformed")
        ));
        assert!(matches!(
            parse_helper_intent(
                r#"{"v":2,"seq":1,"sent_at_ms":1,"kind":"intent","payload":{"action":"focus_session"}}"#
            ),
            Err("unsupported")
        ));

        let child = Command::new("/usr/bin/false")
            .spawn()
            .expect("spawn crash fixture");
        state.child = Some(Arc::new(Mutex::new(child)));
        for _ in 0..50 {
            refresh_child_state(&mut state);
            if state.child.is_none() {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(state.child.is_none());
        assert!(state.sessions.contains_key("session-1"));

        stop_helper(&mut state);
        assert!(state.sessions.contains_key("session-1"));
    }
}
