use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use super::agent_stream::normalize_codex_app_server_message;
use super::agent_terminal::{
    emit_agent_event, emit_agent_exit_event, AgentProvider, AgentStructuredEvent,
    CodexAgentTerminalSnapshot, LiveAgentSessionIdentity,
};
use super::review::resolve_agent_cli_path;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(12);
const OUTPUT_TAIL_CHARS: usize = 120_000;
const EVENT_LIMIT: usize = 80;

// Generated as the bounded subset CodeVetter sends from the schemas emitted by
// `codex app-server generate-json-schema --experimental` (codex-cli 0.145.0).
// Unknown inbound fields remain forward-compatible through serde_json::Value.
mod schema {
    use super::Serialize;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct ClientInfo<'a> {
        pub name: &'a str,
        pub title: &'a str,
        pub version: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct InitializeCapabilities {
        pub experimental_api: bool,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct InitializeParams<'a> {
        pub client_info: ClientInfo<'a>,
        pub capabilities: InitializeCapabilities,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct ThreadStartParams<'a> {
        pub cwd: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub model: Option<&'a str>,
        pub approval_policy: &'a str,
        pub sandbox: &'a str,
        pub ephemeral: bool,
        pub experimental_raw_events: bool,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct TextUserInput<'a> {
        #[serde(rename = "type")]
        pub input_type: &'static str,
        pub text: &'a str,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct TurnStartParams<'a> {
        pub thread_id: &'a str,
        pub input: [TextUserInput<'a>; 1],
    }
}

struct PendingServerRequest {
    id: Value,
    method: String,
    params: Value,
    dispatched: bool,
}

struct SharedSession {
    app: AppHandle,
    local_session_id: String,
    cwd: String,
    pid: Option<u32>,
    started_at_ms: u64,
    stdin: Mutex<ChildStdin>,
    next_request_id: AtomicU64,
    pending_client_responses: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
    pending_server_requests: Mutex<HashMap<String, PendingServerRequest>>,
    output_tail: Mutex<String>,
    last_output_at: Mutex<Instant>,
    last_agent_event: Mutex<Option<String>>,
    agent_events: Mutex<Vec<AgentStructuredEvent>>,
    event_sequence: AtomicU64,
    output_sequence: AtomicU64,
    thread_id: Mutex<Option<String>>,
    active_turn_id: Mutex<Option<String>>,
    ready: AtomicBool,
    buffered_messages: Mutex<Vec<Value>>,
    stop_requested: AtomicBool,
}

struct CodexAppServerSession {
    shared: Arc<SharedSession>,
    child: Arc<Mutex<Child>>,
}

fn sessions() -> &'static Mutex<HashMap<String, CodexAppServerSession>> {
    static STORE: OnceLock<Mutex<HashMap<String, CodexAppServerSession>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn is_running(local_session_id: &str) -> bool {
    sessions()
        .lock()
        .is_ok_and(|sessions| sessions.contains_key(local_session_id.trim()))
}

pub(crate) fn identity(local_session_id: &str) -> Result<Option<LiveAgentSessionIdentity>, String> {
    let sessions = sessions()
        .lock()
        .map_err(|error| format!("Codex app-server registry lock poisoned: {error}"))?;
    let Some(session) = sessions.get(local_session_id.trim()) else {
        return Ok(None);
    };
    let provider_session_id = session
        .shared
        .thread_id
        .lock()
        .map_err(|error| format!("Codex thread identity lock poisoned: {error}"))?
        .clone();
    let project_path = session.shared.cwd.clone();
    Ok(Some(LiveAgentSessionIdentity {
        provider: "codex".to_string(),
        provider_session_id,
        project_path,
    }))
}

pub(crate) fn snapshots() -> Result<Vec<CodexAgentTerminalSnapshot>, String> {
    let sessions = sessions()
        .lock()
        .map_err(|error| format!("Codex app-server registry lock poisoned: {error}"))?;
    Ok(sessions
        .iter()
        .map(|(session_id, session)| {
            let shared = &session.shared;
            CodexAgentTerminalSnapshot {
                session_id: session_id.clone(),
                provider: AgentProvider::Codex,
                cwd: shared.cwd.clone(),
                pid: shared.pid,
                started_at_ms: shared.started_at_ms,
                running: true,
                output_tail: shared
                    .output_tail
                    .lock()
                    .map(|tail| tail.clone())
                    .unwrap_or_default(),
                last_agent_event: shared
                    .last_agent_event
                    .lock()
                    .map(|event| event.clone())
                    .unwrap_or_default(),
                agent_events: shared
                    .agent_events
                    .lock()
                    .map(|events| events.clone())
                    .unwrap_or_default(),
                codex_session_id: shared
                    .thread_id
                    .lock()
                    .map(|thread_id| thread_id.clone())
                    .unwrap_or_default(),
                transcript_path: None,
            }
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn start(
    app: AppHandle,
    local_session_id: &str,
    cwd: &Path,
    prompt: Option<&str>,
    model: Option<&str>,
    sandbox: Option<&str>,
    approval_policy: Option<&str>,
) -> Result<Value, String> {
    let codex_path = resolve_agent_cli_path("codex");
    let mut child = Command::new(&codex_path)
        .arg("app-server")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn Codex app-server ({codex_path}): {error}"))?;
    let pid = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Codex app-server stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Codex app-server stdout unavailable".to_string())?;
    let stderr = child.stderr.take();
    let shared = Arc::new(SharedSession {
        app: app.clone(),
        local_session_id: local_session_id.to_string(),
        cwd: cwd.to_string_lossy().to_string(),
        pid: Some(pid),
        started_at_ms: current_unix_millis(),
        stdin: Mutex::new(stdin),
        next_request_id: AtomicU64::new(0),
        pending_client_responses: Mutex::new(HashMap::new()),
        pending_server_requests: Mutex::new(HashMap::new()),
        output_tail: Mutex::new(String::new()),
        last_output_at: Mutex::new(Instant::now()),
        last_agent_event: Mutex::new(None),
        agent_events: Mutex::new(Vec::new()),
        event_sequence: AtomicU64::new(0),
        output_sequence: AtomicU64::new(0),
        thread_id: Mutex::new(None),
        active_turn_id: Mutex::new(None),
        ready: AtomicBool::new(false),
        buffered_messages: Mutex::new(Vec::new()),
        stop_requested: AtomicBool::new(false),
    });

    start_stdout_reader(Arc::clone(&shared), stdout)?;
    if let Some(stderr) = stderr {
        start_stderr_reader(Arc::clone(&shared), stderr);
    }

    let initialize = request(
        &shared,
        "initialize",
        serde_json::to_value(schema::InitializeParams {
            client_info: schema::ClientInfo {
                name: "codevetter",
                title: "CodeVetter",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: schema::InitializeCapabilities {
                experimental_api: true,
            },
        })
        .map_err(|error| format!("encode Codex initialize params: {error}"))?,
        STARTUP_TIMEOUT,
    );
    if let Err(error) = initialize {
        let _ = child.kill();
        return Err(format!("initialize Codex app-server: {error}"));
    }
    write_message(&shared, &json!({"method": "initialized", "params": {}}))?;

    let thread_response = request(
        &shared,
        "thread/start",
        serde_json::to_value(schema::ThreadStartParams {
            cwd: &shared.cwd,
            model: non_empty(model),
            approval_policy: non_empty(approval_policy).unwrap_or("on-request"),
            sandbox: non_empty(sandbox).unwrap_or("workspace-write"),
            ephemeral: false,
            experimental_raw_events: false,
        })
        .map_err(|error| format!("encode Codex thread params: {error}"))?,
        STARTUP_TIMEOUT,
    )
    .inspect_err(|_| {
        let _ = child.kill();
    })?;
    let thread_id = thread_response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Codex app-server thread/start omitted thread.id".to_string())?;
    if let Ok(mut active_thread_id) = shared.thread_id.lock() {
        *active_thread_id = Some(thread_id.clone());
    }

    let child = Arc::new(Mutex::new(child));
    {
        let mut active = sessions()
            .lock()
            .map_err(|error| format!("Codex app-server registry lock poisoned: {error}"))?;
        if active.contains_key(local_session_id) {
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
            return Err(format!("Codex agent already running: {local_session_id}"));
        }
        active.insert(
            local_session_id.to_string(),
            CodexAppServerSession {
                shared: Arc::clone(&shared),
                child: Arc::clone(&child),
            },
        );
    }

    emit_agent_event(
        &app,
        local_session_id,
        "started",
        None,
        Some(pid),
        Some(0),
        None,
        None,
        None,
    );
    shared.ready.store(true, Ordering::Release);
    drain_buffered_messages(&shared);
    start_child_monitor(Arc::clone(&shared), child);

    if let Some(prompt) = non_empty(prompt) {
        if let Err(error) = start_turn(&shared, prompt) {
            let _ = stop(local_session_id);
            return Err(error);
        }
    }

    Ok(json!({
        "session_id": local_session_id,
        "provider": AgentProvider::Codex,
        "cwd": shared.cwd,
        "pid": pid,
        "transport": "app-server",
        "provider_session_id": thread_id,
    }))
}

pub(crate) fn send_input(local_session_id: &str, input: &str) -> Result<(), String> {
    let shared = shared_session(local_session_id)?;
    let input = input.trim();
    if input.is_empty() {
        return Ok(());
    }
    start_turn(&shared, input)
}

pub(crate) fn stop(local_session_id: &str) -> Result<(), String> {
    let (shared, child) = {
        let active = sessions()
            .lock()
            .map_err(|error| format!("Codex app-server registry lock poisoned: {error}"))?;
        let session = active
            .get(local_session_id.trim())
            .ok_or_else(|| format!("Agent is not running: {}", local_session_id.trim()))?;
        (Arc::clone(&session.shared), Arc::clone(&session.child))
    };
    shared.stop_requested.store(true, Ordering::Release);
    if let (Some(thread_id), Some(turn_id)) = (
        shared.thread_id.lock().ok().and_then(|value| value.clone()),
        shared
            .active_turn_id
            .lock()
            .ok()
            .and_then(|value| value.clone()),
    ) {
        let _ = send_notification(
            &shared,
            "turn/interrupt",
            interrupt_params(&thread_id, &turn_id),
        );
    }
    let result = child
        .lock()
        .map_err(|error| format!("Codex app-server process lock poisoned: {error}"))?
        .kill()
        .map_err(|error| format!("stop Codex app-server: {error}"));
    result
}

pub(crate) fn pending_capabilities(local_session_id: &str, request_id: &str) -> (bool, bool, bool) {
    let Ok(shared) = shared_session(local_session_id) else {
        return (false, false, false);
    };
    let Ok(requests) = shared.pending_server_requests.lock() else {
        return (false, false, false);
    };
    let Some(request) = requests.get(request_id) else {
        return (false, false, false);
    };
    if request.dispatched {
        return (false, false, false);
    }
    match request.method.as_str() {
        "item/tool/requestUserInput"
            if request
                .params
                .get("questions")
                .and_then(Value::as_array)
                .is_some_and(|questions| questions.len() == 1) =>
        {
            (true, false, false)
        }
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decisions = allowed_decisions(&request.params);
            (
                false,
                decisions.iter().any(|decision| decision == "accept"),
                decisions
                    .iter()
                    .any(|decision| matches!(decision.as_str(), "decline" | "cancel")),
            )
        }
        _ => (false, false, false),
    }
}

pub(crate) fn resolve_pending_request(
    local_session_id: &str,
    request_id: &str,
    action: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let shared = shared_session(local_session_id)?;
    let (id, result) = {
        let mut requests = shared
            .pending_server_requests
            .lock()
            .map_err(|error| format!("Codex request registry lock poisoned: {error}"))?;
        let request = requests
            .get_mut(request_id)
            .ok_or_else(|| "Codex request is stale or already resolved".to_string())?;
        if request.dispatched {
            return Err("Codex request response was already sent".to_string());
        }
        let result = prepare_response(request, action, value)?;
        (request.id.clone(), result)
    };

    let response = json!({"id": id, "result": result});
    if let Err(error) = write_message(&shared, &response) {
        if let Ok(mut requests) = shared.pending_server_requests.lock() {
            if let Some(request) = requests.get_mut(request_id) {
                request.dispatched = false;
            }
        }
        return Err(error);
    }
    Ok(())
}

fn shared_session(local_session_id: &str) -> Result<Arc<SharedSession>, String> {
    let active = sessions()
        .lock()
        .map_err(|error| format!("Codex app-server registry lock poisoned: {error}"))?;
    active
        .get(local_session_id.trim())
        .map(|session| Arc::clone(&session.shared))
        .ok_or_else(|| format!("Agent is not running: {}", local_session_id.trim()))
}

fn start_turn(shared: &Arc<SharedSession>, input: &str) -> Result<(), String> {
    let thread_id = shared
        .thread_id
        .lock()
        .map_err(|error| format!("Codex thread identity lock poisoned: {error}"))?
        .clone()
        .ok_or_else(|| "Codex thread is not ready".to_string())?;
    send_notification(
        shared,
        "turn/start",
        serde_json::to_value(schema::TurnStartParams {
            thread_id: &thread_id,
            input: [schema::TextUserInput {
                input_type: "text",
                text: input,
            }],
        })
        .map_err(|error| format!("encode Codex turn params: {error}"))?,
    )
}

fn request(
    shared: &Arc<SharedSession>,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let id = shared.next_request_id.fetch_add(1, Ordering::AcqRel) + 1;
    let (tx, rx) = mpsc::channel();
    shared
        .pending_client_responses
        .lock()
        .map_err(|error| format!("Codex response registry lock poisoned: {error}"))?
        .insert(id, tx);
    if let Err(error) = write_message(
        shared,
        &json!({"id": id, "method": method, "params": params}),
    ) {
        if let Ok(mut responses) = shared.pending_client_responses.lock() {
            responses.remove(&id);
        }
        return Err(error);
    }
    rx.recv_timeout(timeout)
        .map_err(|_| format!("Codex app-server {method} timed out"))?
}

fn send_notification(
    shared: &Arc<SharedSession>,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let id = shared.next_request_id.fetch_add(1, Ordering::AcqRel) + 1;
    write_message(
        shared,
        &json!({"id": id, "method": method, "params": params}),
    )
}

fn write_message(shared: &SharedSession, value: &Value) -> Result<(), String> {
    let mut encoded =
        serde_json::to_vec(value).map_err(|error| format!("encode Codex request: {error}"))?;
    encoded.push(b'\n');
    let mut stdin = shared
        .stdin
        .lock()
        .map_err(|error| format!("Codex app-server stdin lock poisoned: {error}"))?;
    stdin
        .write_all(&encoded)
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("write Codex app-server request: {error}"))
}

fn start_stdout_reader(
    shared: Arc<SharedSession>,
    stdout: std::process::ChildStdout,
) -> Result<(), String> {
    thread::Builder::new()
        .name(format!(
            "Codex app-server reader {}",
            shared.local_session_id
        ))
        .spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if route_client_response(&shared, &message) {
                    continue;
                }
                if shared.ready.load(Ordering::Acquire) {
                    process_server_message(&shared, message);
                } else if let Ok(mut buffered) = shared.buffered_messages.lock() {
                    buffered.push(message);
                }
            }
        })
        .map(|_| ())
        .map_err(|error| format!("start Codex app-server reader: {error}"))
}

fn start_stderr_reader(shared: Arc<SharedSession>, stderr: std::process::ChildStderr) {
    let _ = thread::Builder::new()
        .name(format!(
            "Codex app-server stderr {}",
            shared.local_session_id
        ))
        .spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    emit_output(&shared, format!("{line}\n"));
                }
            }
        });
}

fn route_client_response(shared: &SharedSession, message: &Value) -> bool {
    if message.get("method").is_some() {
        return false;
    }
    let Some(id) = message.get("id").and_then(Value::as_u64) else {
        return false;
    };
    let sender = shared
        .pending_client_responses
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(&id));
    let Some(sender) = sender else {
        return true;
    };
    let result = if let Some(error) = message.get("error") {
        Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Codex app-server request failed")
            .to_string())
    } else {
        Ok(message.clone())
    };
    let _ = sender.send(result);
    true
}

fn drain_buffered_messages(shared: &Arc<SharedSession>) {
    let messages = shared
        .buffered_messages
        .lock()
        .map(|mut messages| std::mem::take(&mut *messages))
        .unwrap_or_default();
    for message in messages {
        process_server_message(shared, message);
    }
}

fn process_server_message(shared: &Arc<SharedSession>, message: Value) {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    if message.get("id").is_some() {
        let request_id = request_id(&message["id"]);
        if let Ok(mut requests) = shared.pending_server_requests.lock() {
            requests.insert(
                request_id,
                PendingServerRequest {
                    id: message["id"].clone(),
                    method: method.to_string(),
                    params: params.clone(),
                    dispatched: false,
                },
            );
        }
    }
    if method == "serverRequest/resolved" {
        if let Some(id) = params.get("requestId") {
            if let Ok(mut requests) = shared.pending_server_requests.lock() {
                requests.remove(&request_id(id));
            }
        }
    }
    if method == "turn/started" {
        if let Some(turn_id) = params.pointer("/turn/id").and_then(Value::as_str) {
            if let Ok(mut active_turn_id) = shared.active_turn_id.lock() {
                *active_turn_id = Some(turn_id.to_string());
            }
        }
    } else if method == "turn/completed" {
        if let Ok(mut active_turn_id) = shared.active_turn_id.lock() {
            *active_turn_id = None;
        }
    } else if method == "item/agentMessage/delta" {
        if let Some(delta) = params.get("delta").and_then(Value::as_str) {
            emit_output(shared, delta.to_string());
        }
    }

    let Ok(raw) = serde_json::to_string(&message) else {
        return;
    };
    let Some(normalized) = normalize_codex_app_server_message(&raw) else {
        return;
    };
    if let Ok(mut last) = shared.last_agent_event.lock() {
        *last = Some(normalized.clone());
    }
    let sequence = shared.event_sequence.fetch_add(1, Ordering::AcqRel) + 1;
    if let Ok(mut events) = shared.agent_events.lock() {
        events.push(AgentStructuredEvent {
            seq: sequence,
            at_ms: current_unix_millis(),
            data: normalized.clone(),
        });
        if events.len() > EVENT_LIMIT {
            let remove = events.len() - EVENT_LIMIT;
            events.drain(..remove);
        }
    }
    emit_agent_event(
        &shared.app,
        &shared.local_session_id,
        "agent_event",
        Some(normalized),
        shared.pid,
        Some(0),
        Some(sequence),
        None,
        None,
    );
}

fn emit_output(shared: &SharedSession, output: String) {
    if let Ok(mut tail) = shared.output_tail.lock() {
        tail.push_str(&output);
        if tail.chars().count() > OUTPUT_TAIL_CHARS {
            let keep_from = tail
                .char_indices()
                .rev()
                .nth(OUTPUT_TAIL_CHARS)
                .map(|(index, _)| index)
                .unwrap_or(0);
            tail.drain(..keep_from);
        }
    }
    if let Ok(mut last_output_at) = shared.last_output_at.lock() {
        *last_output_at = Instant::now();
    }
    let sequence = shared.output_sequence.fetch_add(1, Ordering::AcqRel) + 1;
    emit_agent_event(
        &shared.app,
        &shared.local_session_id,
        "output",
        Some(output),
        shared.pid,
        Some(0),
        Some(sequence),
        None,
        None,
    );
}

fn start_child_monitor(shared: Arc<SharedSession>, child: Arc<Mutex<Child>>) {
    thread::spawn(move || loop {
        let status = child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok().flatten());
        if let Some(status) = status {
            if let Ok(mut active) = sessions().lock() {
                active.remove(&shared.local_session_id);
            }
            emit_agent_exit_event(
                &shared.app,
                &shared.local_session_id,
                shared.pid,
                status.code().map(|code| code as u32),
                Some(status.success()),
                None,
                shared.stop_requested.load(Ordering::Acquire),
            );
            return;
        }
        thread::sleep(Duration::from_millis(250));
    });
}

fn request_id(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|id| id.to_string()))
        .or_else(|| value.as_u64().map(|id| id.to_string()))
        .unwrap_or_else(|| value.to_string())
}

fn prepare_response(
    request: &mut PendingServerRequest,
    action: &str,
    value: Option<&str>,
) -> Result<Value, String> {
    if request.dispatched {
        return Err("Codex request response was already sent".to_string());
    }
    let result = match (request.method.as_str(), action) {
        ("item/tool/requestUserInput", "submit_reply") => {
            let questions = request
                .params
                .get("questions")
                .and_then(Value::as_array)
                .filter(|questions| questions.len() == 1)
                .ok_or_else(|| {
                    "Multi-question Codex prompts must be answered in Work".to_string()
                })?;
            let question_id = questions[0]
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "Codex question identity is unavailable".to_string())?;
            let answer = value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "reply is required".to_string())?;
            json!({"answers": {question_id: {"answers": [answer]}}})
        }
        (
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval",
            "approve" | "deny",
        ) => {
            let allowed = allowed_decisions(&request.params);
            let decision = if action == "approve" {
                "accept"
            } else if allowed.iter().any(|decision| decision == "decline") {
                "decline"
            } else {
                "cancel"
            };
            if !allowed.iter().any(|allowed| allowed == decision) {
                return Err(format!("Codex did not allow the {decision} decision"));
            }
            json!({"decision": decision})
        }
        _ => return Err("Codex request does not support this inline action".to_string()),
    };
    request.dispatched = true;
    Ok(result)
}

fn interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    json!({"threadId": thread_id, "turnId": turn_id})
}

fn allowed_decisions(params: &Value) -> Vec<String> {
    params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "accept".to_string(),
                "decline".to_string(),
                "cancel".to_string(),
            ]
        })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_stable_across_string_and_numeric_json_rpc_ids() {
        assert_eq!(request_id(&json!(41)), "41");
        assert_eq!(request_id(&json!("request-9")), "request-9");
    }

    #[test]
    fn decisions_default_to_safe_one_shot_values() {
        assert_eq!(
            allowed_decisions(&Value::Null),
            vec!["accept", "decline", "cancel"]
        );
        assert_eq!(
            allowed_decisions(&json!({"availableDecisions": ["accept", "cancel"]})),
            vec!["accept", "cancel"]
        );
    }

    #[test]
    fn question_response_preserves_schema_identity_and_is_single_use() {
        let mut request = PendingServerRequest {
            id: json!("request-question"),
            method: "item/tool/requestUserInput".to_string(),
            params: serde_json::from_str(include_str!(
                "../../tests/fixtures/agent-stream/codex-question.json"
            ))
            .unwrap(),
            dispatched: false,
        };
        request.params = request.params["params"].clone();
        assert_eq!(
            prepare_response(&mut request, "submit_reply", Some("Small")).unwrap(),
            json!({"answers": {"scope": {"answers": ["Small"]}}})
        );
        assert!(prepare_response(&mut request, "submit_reply", Some("Broad")).is_err());
    }

    #[test]
    fn approval_ack_resolution_and_stale_races_are_deterministic() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/agent-stream/codex-approval.json"
        ))
        .unwrap();
        let mut requests = HashMap::from([(
            "41".to_string(),
            PendingServerRequest {
                id: json!(41),
                method: fixture["method"].as_str().unwrap().to_string(),
                params: fixture["params"].clone(),
                dispatched: false,
            },
        )]);
        let request = requests.get_mut("41").unwrap();
        assert_eq!(
            prepare_response(request, "approve", None).unwrap(),
            json!({"decision": "accept"})
        );
        assert!(prepare_response(request, "deny", None).is_err());
        assert!(requests.remove("41").is_some());
        assert!(requests.get_mut("41").is_none());
    }

    #[test]
    fn interruption_targets_the_active_thread_and_turn() {
        assert_eq!(
            interrupt_params("thread-1", "turn-2"),
            json!({"threadId": "thread-1", "turnId": "turn-2"})
        );
    }

    #[test]
    fn generated_schema_subset_matches_the_installed_codex_contract_when_available() {
        use std::fs;
        use std::process::Command;

        if Command::new("codex").arg("--version").output().is_err() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let status = Command::new("codex")
            .args([
                "app-server",
                "generate-json-schema",
                "--experimental",
                "--out",
            ])
            .arg(directory.path())
            .status()
            .unwrap();
        assert!(status.success());

        let initialize =
            fs::read_to_string(directory.path().join("v1/InitializeParams.json")).unwrap();
        let thread_start =
            fs::read_to_string(directory.path().join("v2/ThreadStartParams.json")).unwrap();
        let turn_start =
            fs::read_to_string(directory.path().join("v2/TurnStartParams.json")).unwrap();
        let server_requests =
            fs::read_to_string(directory.path().join("ServerRequest.json")).unwrap();
        for required in ["clientInfo", "experimentalApi"] {
            assert!(initialize.contains(required));
        }
        for required in ["approvalPolicy", "sandbox", "experimentalRawEvents"] {
            assert!(thread_start.contains(required));
        }
        for required in ["threadId", "\"input\"", "\"text\""] {
            assert!(turn_start.contains(required));
        }
        for required in [
            "item/commandExecution/requestApproval",
            "item/fileChange/requestApproval",
            "item/tool/requestUserInput",
        ] {
            assert!(server_requests.contains(required));
        }
    }
}
