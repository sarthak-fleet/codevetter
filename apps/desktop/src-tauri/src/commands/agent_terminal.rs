use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use super::review::resolve_agent_cli_path;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const OUTPUT_LIMIT_CHARS: usize = 64_000;
const AGENT_TERMINAL_EVENT: &str = "agent-terminal-event";
const PTY_READ_BUFFER_BYTES: usize = 64 * 1024;
const PTY_OUTPUT_EMIT_INTERVAL_MS: u64 = 16;
const PTY_OUTPUT_EMIT_CHARS: usize = 128 * 1024;
// Reattach only needs recent visual context; full Codex history remains in the
// rollout JSONL and frontend copy buffer. Keep this small enough for 10-12 panes.
const AGENT_OUTPUT_TAIL_CHARS: usize = 120_000;
const AGENT_GRACEFUL_EXIT_COMMAND: &[u8] = b"/exit\r";
const CODEX_FORCE_STOP_AFTER_MS: u64 = 3_000;
const WARP_CLI_AGENT_PROTOCOL_VERSION: &str = "1";
const CODEVETTER_WARP_COMPAT_VERSION: &str = "codevetter-agent-panel-0.1";
const CODEVETTER_TERM_PROGRAM: &str = "CodeVetter";
const AGENT_EVENT_LOG_LIMIT: usize = 80;
const CODEX_WARP_MARKETPLACE: &str = "codex-warp";
const CODEX_WARP_MARKETPLACE_SOURCE: &str = "warpdotdev/codex-warp";
const CODEX_WARP_PLUGIN: &str = "warp@codex-warp";
const CODEX_WARP_ORCHESTRATION_PLUGIN: &str = "orchestration@codex-warp";
const CLAUDE_HOOK_POLL_INTERVAL_MS: u64 = 25;
const CLAUDE_HOOK_EVENT_LIMIT_CHARS: usize = 8_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentProvider {
    Codex,
    Claude,
}

impl AgentProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}

struct RunningCodexAgent {
    tx: Sender<AgentPtyCommand>,
    provider: AgentProvider,
    pid: Option<u32>,
    cwd: String,
    started_at_ms: u64,
    output_tail: Arc<Mutex<String>>,
    last_output_at: Arc<Mutex<Instant>>,
    last_agent_event: Arc<Mutex<Option<String>>>,
    agent_events: Arc<Mutex<Vec<AgentStructuredEvent>>>,
    codex_session_id: Arc<Mutex<Option<String>>>,
    transcript_path: Arc<Mutex<Option<String>>>,
    claude_response_dir: Option<PathBuf>,
    stop_requested: Arc<AtomicBool>,
}

struct ClaudeHookBridge {
    directory: PathBuf,
    settings_path: PathBuf,
    events_path: PathBuf,
    response_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveAgentSessionIdentity {
    pub provider: String,
    pub provider_session_id: Option<String>,
    pub project_path: String,
}

enum AgentPtyCommand {
    Input(Vec<u8>),
    Resize(PtySize),
    Stop,
}

fn codex_agents() -> &'static Mutex<HashMap<String, RunningCodexAgent>> {
    static STORE: OnceLock<Mutex<HashMap<String, RunningCodexAgent>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn resolve_live_agent_session_identity(
    terminal_id: &str,
) -> Result<Option<LiveAgentSessionIdentity>, String> {
    if let Some(identity) = super::codex_app_server::identity(terminal_id)? {
        return Ok(Some(identity));
    }
    let sessions = codex_agents()
        .lock()
        .map_err(|error| format!("agent registry lock poisoned: {error}"))?;
    resolve_live_agent_session_identity_from_registry(&sessions, terminal_id)
}

fn resolve_live_agent_session_identity_from_registry(
    sessions: &HashMap<String, RunningCodexAgent>,
    terminal_id: &str,
) -> Result<Option<LiveAgentSessionIdentity>, String> {
    let Some(session) = sessions.get(terminal_id.trim()) else {
        return Ok(None);
    };
    let provider_session_id = session
        .codex_session_id
        .lock()
        .map_err(|error| format!("agent session identity lock poisoned: {error}"))?
        .clone();
    Ok(Some(LiveAgentSessionIdentity {
        provider: session.provider.as_str().to_string(),
        provider_session_id,
        project_path: session.cwd.clone(),
    }))
}

fn ensure_agent_heartbeat(app: AppHandle) {
    static HEARTBEAT_STARTED: OnceLock<()> = OnceLock::new();
    HEARTBEAT_STARTED.get_or_init(|| {
        let heartbeat_app = app.clone();
        let _ = thread::Builder::new()
            .name("Codex PTY heartbeat".to_string())
            .spawn(move || loop {
                thread::sleep(Duration::from_millis(2_000));
                let heartbeats = codex_agents()
                    .lock()
                    .map(|sessions| collect_agent_heartbeats(&sessions))
                    .unwrap_or_default();

                for (session_id, pid, idle_ms) in heartbeats {
                    emit_agent_event(
                        &heartbeat_app,
                        &session_id,
                        "heartbeat",
                        None,
                        pid,
                        Some(idle_ms),
                        None,
                        None,
                        None,
                    );
                }
            });
    });
}

fn collect_agent_heartbeats(
    sessions: &HashMap<String, RunningCodexAgent>,
) -> Vec<(String, Option<u32>, u64)> {
    sessions
        .iter()
        .map(|(session_id, session)| {
            let idle_ms = session
                .last_output_at
                .lock()
                .map(|last_output_at| last_output_at.elapsed().as_millis() as u64)
                .unwrap_or_default();
            (session_id.clone(), session.pid, idle_ms)
        })
        .collect()
}

#[derive(Serialize)]
pub struct AgentTerminalCommandResult {
    pub command: String,
    pub cwd: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub timeout_ms: u64,
    pub timed_out: bool,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Serialize, Clone)]
pub struct CodexAgentTerminalSnapshot {
    pub session_id: String,
    pub provider: AgentProvider,
    pub cwd: String,
    pub pid: Option<u32>,
    pub started_at_ms: u64,
    pub running: bool,
    pub output_tail: String,
    pub last_agent_event: Option<String>,
    pub agent_events: Vec<AgentStructuredEvent>,
    pub codex_session_id: Option<String>,
    pub transcript_path: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct AgentStructuredEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub data: String,
}

#[derive(Serialize, Clone)]
pub struct AgentTerminalEvent {
    pub session_id: String,
    pub kind: String,
    pub data: Option<String>,
    pub pid: Option<u32>,
    pub idle_ms: Option<u64>,
    pub seq: Option<u64>,
    pub exit_code: Option<u32>,
    pub success: Option<bool>,
    pub intentional_stop: Option<bool>,
}

#[derive(Serialize, Clone)]
pub struct CodexWarpPluginStatus {
    pub codex_available: bool,
    pub marketplace_installed: bool,
    pub warp_plugin_installed: bool,
    pub warp_plugin_enabled: bool,
    pub orchestration_plugin_installed: bool,
    pub orchestration_plugin_enabled: bool,
    pub structured_env_enabled: bool,
    pub needs_install: bool,
    pub codex_path: String,
    pub marketplace_output: String,
    pub plugin_output: String,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct AgentLifecycleNotification {
    pub v: Option<u64>,
    pub agent: Option<String>,
    pub event: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub query: Option<String>,
    pub response: Option<String>,
    pub summary: Option<String>,
    pub tool_name: Option<String>,
    pub transcript_path: Option<String>,
    pub plugin_version: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

#[tauri::command]
pub async fn get_codex_warp_plugin_status() -> Result<CodexWarpPluginStatus, String> {
    tokio::task::spawn_blocking(load_codex_warp_plugin_status)
        .await
        .map_err(|e| format!("Codex-Warp status task join error: {e}"))?
}

#[tauri::command]
pub async fn install_codex_warp_plugin() -> Result<CodexWarpPluginStatus, String> {
    tokio::task::spawn_blocking(|| {
        let mut status = load_codex_warp_plugin_status()?;
        if !status.codex_available {
            return Ok(status);
        }

        let codex_path = status.codex_path.clone();
        if !status.marketplace_installed {
            let install = run_codex_command(
                &codex_path,
                &[
                    "plugin",
                    "marketplace",
                    "add",
                    CODEX_WARP_MARKETPLACE_SOURCE,
                ],
            )?;
            if !install.success {
                status.error = Some(format_command_error(
                    "install Codex-Warp marketplace",
                    &install,
                ));
                return Ok(status);
            }
        }

        status = load_codex_warp_plugin_status()?;
        if !status.warp_plugin_installed || !status.warp_plugin_enabled {
            let install = run_codex_command(&codex_path, &["plugin", "add", CODEX_WARP_PLUGIN])?;
            if !install.success {
                status.error = Some(format_command_error("install Codex-Warp plugin", &install));
                return Ok(status);
            }
        }

        load_codex_warp_plugin_status()
    })
    .await
    .map_err(|e| format!("Codex-Warp install task join error: {e}"))?
}

#[tauri::command]
pub fn list_codex_agent_terminals() -> Result<Vec<CodexAgentTerminalSnapshot>, String> {
    let sessions = codex_agents()
        .lock()
        .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
    let mut snapshots = collect_agent_snapshots(&sessions);
    drop(sessions);
    snapshots.extend(super::codex_app_server::snapshots()?);
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.started_at_ms));
    Ok(snapshots)
}

#[tauri::command]
pub fn list_agent_terminals() -> Result<Vec<CodexAgentTerminalSnapshot>, String> {
    list_codex_agent_terminals()
}

fn collect_agent_snapshots(
    sessions: &HashMap<String, RunningCodexAgent>,
) -> Vec<CodexAgentTerminalSnapshot> {
    sessions
        .iter()
        .map(|(session_id, session)| CodexAgentTerminalSnapshot {
            session_id: session_id.clone(),
            provider: session.provider,
            cwd: session.cwd.clone(),
            pid: session.pid,
            started_at_ms: session.started_at_ms,
            running: true,
            output_tail: session
                .output_tail
                .lock()
                .map(|tail| tail.clone())
                .unwrap_or_default(),
            last_agent_event: session
                .last_agent_event
                .lock()
                .map(|event| event.clone())
                .unwrap_or_default(),
            agent_events: session
                .agent_events
                .lock()
                .map(|events| events.clone())
                .unwrap_or_default(),
            codex_session_id: session
                .codex_session_id
                .lock()
                .map(|session_id| session_id.clone())
                .unwrap_or_default(),
            transcript_path: session
                .transcript_path
                .lock()
                .map(|path| path.clone())
                .unwrap_or_default(),
        })
        .collect()
}

#[tauri::command]
pub fn start_codex_agent_terminal(
    app: AppHandle,
    session_id: String,
    cwd: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    sandbox: Option<String>,
    approval_policy: Option<String>,
    resume_session_id: Option<String>,
    fork_session_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<Value, String> {
    start_agent_terminal_impl(
        app,
        AgentProvider::Codex,
        session_id,
        cwd,
        prompt,
        model,
        sandbox,
        approval_policy,
        resume_session_id,
        fork_session_id,
        cols,
        rows,
    )
}

#[tauri::command]
pub fn start_agent_terminal(
    app: AppHandle,
    provider: AgentProvider,
    session_id: String,
    cwd: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    sandbox: Option<String>,
    approval_policy: Option<String>,
    resume_session_id: Option<String>,
    fork_session_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<Value, String> {
    start_agent_terminal_impl(
        app,
        provider,
        session_id,
        cwd,
        prompt,
        model,
        sandbox,
        approval_policy,
        resume_session_id,
        fork_session_id,
        cols,
        rows,
    )
}

#[allow(clippy::too_many_arguments)]
fn start_agent_terminal_impl(
    app: AppHandle,
    provider: AgentProvider,
    session_id: String,
    cwd: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    sandbox: Option<String>,
    approval_policy: Option<String>,
    resume_session_id: Option<String>,
    fork_session_id: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<Value, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("session_id is required".into());
    }
    {
        let sessions = codex_agents()
            .lock()
            .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
        if sessions.contains_key(&session_id) {
            return Err(format!(
                "{} agent already running: {session_id}",
                provider.display_name()
            ));
        }
    }
    if super::codex_app_server::is_running(&session_id) {
        return Err(format!(
            "{} agent already running: {session_id}",
            provider.display_name()
        ));
    }

    let cwd = resolve_cwd(cwd.as_deref())?;
    let agent_path = resolve_agent_cli_path(provider.as_str());
    let resume_session_id = resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let fork_session_id = fork_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if resume_session_id.is_some() && fork_session_id.is_some() {
        return Err("resume_session_id and fork_session_id are mutually exclusive".into());
    }
    let prefer_codex_app_server = provider == AgentProvider::Codex
        && resume_session_id.is_none()
        && fork_session_id.is_none()
        && std::env::var("CODEVETTER_CODEX_TRANSPORT")
            .map(|value| !value.eq_ignore_ascii_case("pty"))
            .unwrap_or(true);
    if prefer_codex_app_server {
        match super::codex_app_server::start(
            app.clone(),
            &session_id,
            &cwd,
            prompt.as_deref(),
            model.as_deref(),
            sandbox.as_deref(),
            approval_policy.as_deref(),
        ) {
            Ok(result) => return Ok(result),
            Err(error) => {
                eprintln!(
                    "Codex app-server unavailable for {session_id}; falling back to PTY: {error}"
                );
            }
        }
    }
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.unwrap_or(24).max(8),
            cols: cols.unwrap_or(100).max(40),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("open {} PTY: {e}", provider.display_name()))?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("clone {} PTY reader: {e}", provider.display_name()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("open {} PTY writer: {e}", provider.display_name()))?;
    let claude_hook_bridge = if provider == AgentProvider::Claude {
        Some(create_claude_hook_bridge(&session_id)?)
    } else {
        None
    };
    let args = match provider {
        AgentProvider::Codex => codex_agent_command_args(
            &cwd,
            sandbox.as_deref(),
            approval_policy.as_deref(),
            model.as_deref(),
            prompt.as_deref(),
            resume_session_id,
            fork_session_id,
        ),
        AgentProvider::Claude => claude_agent_command_args(
            approval_policy.as_deref(),
            model.as_deref(),
            prompt.as_deref(),
            resume_session_id,
            fork_session_id,
            claude_hook_bridge
                .as_ref()
                .map(|bridge| bridge.settings_path.as_path()),
        ),
    };
    let mut command = CommandBuilder::new(&agent_path);
    for arg in args {
        command.arg(arg);
    }
    for (key, value) in agent_terminal_env(provider) {
        command.env(key, value);
    }
    if let Some(bridge) = claude_hook_bridge.as_ref() {
        command.env(
            "CODEVETTER_AGENT_EVENT_FILE",
            bridge.events_path.as_os_str(),
        );
        command.env(
            "CODEVETTER_AGENT_RESPONSE_DIR",
            bridge.response_dir.as_os_str(),
        );
        if let Ok(executable) = std::env::current_exe() {
            command.env("CODEVETTER_AGENT_HOOK_BIN", executable.as_os_str());
        }
    }
    command.cwd(&cwd);

    let mut child = match pair.slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            if let Some(bridge) = claude_hook_bridge.as_ref() {
                cleanup_claude_hook_bridge(bridge);
            }
            return Err(format!(
                "spawn {} PTY ({agent_path}): {error}",
                provider.display_name()
            ));
        }
    };
    let pid = child.process_id();
    let killer = child.clone_killer();
    drop(pair.slave);
    let master = pair.master;
    let (tx, rx) = mpsc::channel::<AgentPtyCommand>();
    let output_tail = Arc::new(Mutex::new(String::new()));
    let last_output_at = Arc::new(Mutex::new(Instant::now()));
    let last_agent_event = Arc::new(Mutex::new(None));
    let agent_events = Arc::new(Mutex::new(Vec::new()));
    let codex_session_id = Arc::new(Mutex::new(None));
    let transcript_path = Arc::new(Mutex::new(None));
    let stop_requested = Arc::new(AtomicBool::new(false));

    {
        let mut sessions = codex_agents()
            .lock()
            .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
        sessions.insert(
            session_id.clone(),
            RunningCodexAgent {
                tx: tx.clone(),
                provider,
                pid,
                cwd: cwd.to_string_lossy().to_string(),
                started_at_ms: current_unix_millis(),
                output_tail: Arc::clone(&output_tail),
                last_output_at: Arc::clone(&last_output_at),
                last_agent_event: Arc::clone(&last_agent_event),
                agent_events: Arc::clone(&agent_events),
                codex_session_id: Arc::clone(&codex_session_id),
                transcript_path: Arc::clone(&transcript_path),
                claude_response_dir: claude_hook_bridge
                    .as_ref()
                    .map(|bridge| bridge.response_dir.clone()),
                stop_requested: Arc::clone(&stop_requested),
            },
        );
    }
    ensure_agent_heartbeat(app.clone());

    let claude_hook_stop = Arc::new(AtomicBool::new(false));
    let claude_hook_thread = if let Some(bridge) = claude_hook_bridge.as_ref() {
        match start_claude_hook_event_reader(
            app.clone(),
            session_id.clone(),
            pid,
            bridge.events_path.clone(),
            Arc::clone(&claude_hook_stop),
            Arc::clone(&last_agent_event),
            Arc::clone(&agent_events),
            Arc::clone(&codex_session_id),
            Arc::clone(&transcript_path),
        ) {
            Ok(thread) => Some(thread),
            Err(error) => {
                if let Ok(mut sessions) = codex_agents().lock() {
                    sessions.remove(&session_id);
                }
                let mut abort = child.clone_killer();
                let _ = abort.kill();
                let _ = child.wait();
                cleanup_claude_hook_bridge(bridge);
                return Err(error);
            }
        }
    } else {
        None
    };

    emit_agent_event(
        &app,
        &session_id,
        "started",
        None,
        pid,
        Some(0),
        None,
        None,
        None,
    );

    let control_app = app.clone();
    let control_session = session_id.clone();
    thread::Builder::new()
        .name(format!(
            "{} PTY control {session_id}",
            provider.display_name()
        ))
        .spawn(move || {
            run_agent_pty_control_loop(
                control_app,
                control_session,
                provider,
                writer,
                master,
                killer,
                rx,
                pid,
            )
        })
        .map_err(|e| format!("spawn {} PTY control loop: {e}", provider.display_name()))?;

    let reader_app = app.clone();
    let reader_session = session_id.clone();
    let reader_last_output_at = Arc::clone(&last_output_at);
    let reader_output_tail = Arc::clone(&output_tail);
    let reader_last_agent_event = Arc::clone(&last_agent_event);
    let reader_agent_events = Arc::clone(&agent_events);
    let reader_codex_session_id = Arc::clone(&codex_session_id);
    let reader_transcript_path = Arc::clone(&transcript_path);
    thread::Builder::new()
        .name(format!(
            "{} PTY reader {session_id}",
            provider.display_name()
        ))
        .spawn(move || {
            let mut buf = vec![0_u8; PTY_READ_BUFFER_BYTES];
            let mut output_seq = 0_u64;
            let mut agent_event_seq = 0_u64;
            let mut pending_output = String::new();
            let mut last_output_emit =
                Instant::now() - Duration::from_millis(PTY_OUTPUT_EMIT_INTERVAL_MS);
            let mut notification_buffer = String::new();
            let mut rich_notifications_active = false;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        flush_pending_pty_output(
                            &reader_app,
                            &reader_session,
                            &mut pending_output,
                            &mut output_seq,
                            pid,
                            &mut last_output_emit,
                        );
                        break;
                    }
                    Ok(n) => {
                        if let Ok(mut last_output_at) = reader_last_output_at.lock() {
                            *last_output_at = Instant::now();
                        }
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        append_output_tail(&reader_output_tail, &chunk);
                        let notifications = if provider == AgentProvider::Codex {
                            extract_codex_agent_notifications(
                                &mut notification_buffer,
                                &mut rich_notifications_active,
                                &chunk,
                            )
                        } else {
                            Vec::new()
                        };
                        pending_output.push_str(&chunk);
                        for notification in notifications {
                            agent_event_seq = agent_event_seq.saturating_add(1);
                            if let Some((codex_id, transcript)) =
                                agent_event_identity(&notification)
                            {
                                if let Some(codex_id) = codex_id {
                                    if let Ok(mut session_id) = reader_codex_session_id.lock() {
                                        *session_id = Some(codex_id);
                                    }
                                }
                                if let Some(transcript) = transcript {
                                    if let Ok(mut path) = reader_transcript_path.lock() {
                                        *path = Some(transcript);
                                    }
                                }
                            }
                            if let Ok(mut last_agent_event) = reader_last_agent_event.lock() {
                                *last_agent_event = Some(notification.clone());
                            }
                            append_agent_structured_event(
                                &reader_agent_events,
                                AgentStructuredEvent {
                                    seq: agent_event_seq,
                                    at_ms: current_unix_millis(),
                                    data: notification.clone(),
                                },
                            );
                            emit_agent_event(
                                &reader_app,
                                &reader_session,
                                "agent_event",
                                Some(notification),
                                pid,
                                Some(0),
                                Some(agent_event_seq),
                                None,
                                None,
                            );
                        }
                        flush_pending_pty_output_if_due(
                            &reader_app,
                            &reader_session,
                            &mut pending_output,
                            &mut output_seq,
                            pid,
                            &mut last_output_emit,
                        );
                    }
                    Err(error) => {
                        flush_pending_pty_output(
                            &reader_app,
                            &reader_session,
                            &mut pending_output,
                            &mut output_seq,
                            pid,
                            &mut last_output_emit,
                        );
                        emit_agent_event(
                            &reader_app,
                            &reader_session,
                            "error",
                            Some(format!(
                                "read {} PTY output: {error}",
                                provider.display_name()
                            )),
                            pid,
                            None,
                            None,
                            None,
                            Some(false),
                        );
                        break;
                    }
                }
            }
        })
        .map_err(|e| format!("spawn {} PTY reader: {e}", provider.display_name()))?;

    let wait_app = app.clone();
    let wait_session = session_id.clone();
    let wait_stop_requested = Arc::clone(&stop_requested);
    thread::spawn(move || {
        let status = child.wait();
        claude_hook_stop.store(true, Ordering::Release);
        if let Some(hook_thread) = claude_hook_thread {
            let _ = hook_thread.join();
        }
        if let Some(bridge) = claude_hook_bridge.as_ref() {
            cleanup_claude_hook_bridge(bridge);
        }
        let (exit_code, success, data) = match status {
            Ok(status) => (
                Some(status.exit_code()),
                Some(status.success()),
                status
                    .signal()
                    .map(|signal| format!("terminated by {signal}")),
            ),
            Err(error) => (
                None,
                Some(false),
                Some(format!(
                    "wait for {} agent: {error}",
                    provider.display_name()
                )),
            ),
        };
        if let Ok(mut sessions) = codex_agents().lock() {
            sessions.remove(&wait_session);
        }
        emit_agent_exit_event(
            &wait_app,
            &wait_session,
            pid,
            exit_code,
            success,
            data,
            wait_stop_requested.load(Ordering::Acquire),
        );
    });

    Ok(json!({
        "session_id": session_id,
        "provider": provider,
        "cwd": cwd.to_string_lossy(),
        "pid": pid,
    }))
}

fn codex_agent_command_args(
    cwd: &Path,
    sandbox: Option<&str>,
    approval_policy: Option<&str>,
    model: Option<&str>,
    prompt: Option<&str>,
    resume_session_id: Option<&str>,
    fork_session_id: Option<&str>,
) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(fork_session_id) = fork_session_id {
        if !fork_session_id.trim().is_empty() {
            args.push(OsString::from("fork"));
        }
    } else if let Some(resume_session_id) = resume_session_id {
        if !resume_session_id.trim().is_empty() {
            args.push(OsString::from("resume"));
        }
    }

    args.push(OsString::from("--no-alt-screen"));
    args.push(OsString::from("-C"));
    args.push(cwd.as_os_str().to_os_string());
    args.push(OsString::from("-s"));
    args.push(OsString::from(
        sandbox
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("workspace-write"),
    ));
    args.push(OsString::from("-a"));
    args.push(OsString::from(
        approval_policy
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("on-request"),
    ));

    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(OsString::from("-m"));
        args.push(OsString::from(model));
    }
    if let Some(fork_session_id) = fork_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from(fork_session_id));
    } else if let Some(resume_session_id) = resume_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from(resume_session_id));
    }
    if let Some(prompt) = prompt.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(OsString::from(prompt));
    }
    args
}

fn create_claude_hook_bridge(session_id: &str) -> Result<ClaudeHookBridge, String> {
    static BRIDGE_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = BRIDGE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let parent = std::env::temp_dir().join("codevetter-agent-hooks");
    fs::create_dir_all(&parent)
        .map_err(|error| format!("create Claude hook bridge root for {session_id}: {error}"))?;
    let directory = parent.join(format!(
        "{}-{}-{unique}",
        std::process::id(),
        current_unix_millis()
    ));
    let mut directory_options = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory_options.mode(0o700);
    }
    directory_options
        .create(&directory)
        .map_err(|error| format!("create Claude hook bridge for {session_id}: {error}"))?;

    let settings_path = directory.join("settings.json");
    let events_path = directory.join("events.jsonl");
    let response_dir = directory.join("responses");
    let mut response_directory_options = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        response_directory_options.mode(0o700);
    }
    if let Err(error) = response_directory_options.create(&response_dir) {
        let _ = fs::remove_dir_all(&directory);
        return Err(format!(
            "create Claude hook response directory for {session_id}: {error}"
        ));
    }
    let mut events_options = OpenOptions::new();
    events_options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        events_options.mode(0o600);
    }
    if let Err(error) = events_options.open(&events_path) {
        let _ = fs::remove_dir_all(&directory);
        return Err(format!(
            "create Claude hook event stream for {session_id}: {error}"
        ));
    }

    let hook = json!([{
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": "\"$CODEVETTER_AGENT_HOOK_BIN\" --claude-hook-bridge",
            "timeout": 120
        }]
    }]);
    let mut hooks = serde_json::Map::new();
    for event in [
        "SessionStart",
        "SessionEnd",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionRequest",
        "Stop",
        "StopFailure",
        "Notification",
    ] {
        hooks.insert(event.to_string(), hook.clone());
    }
    let settings = Value::Object(
        [("hooks".to_string(), Value::Object(hooks))]
            .into_iter()
            .collect(),
    );
    let settings_bytes = serde_json::to_vec(&settings)
        .map_err(|error| format!("serialize Claude hook settings: {error}"))?;
    let mut settings_options = OpenOptions::new();
    settings_options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        settings_options.mode(0o600);
    }
    let settings_result = settings_options
        .open(&settings_path)
        .and_then(|mut file| file.write_all(&settings_bytes));
    if let Err(error) = settings_result {
        let _ = fs::remove_dir_all(&directory);
        return Err(format!(
            "write Claude hook settings for {session_id}: {error}"
        ));
    }

    Ok(ClaudeHookBridge {
        directory,
        settings_path,
        events_path,
        response_dir,
    })
}

fn cleanup_claude_hook_bridge(bridge: &ClaudeHookBridge) {
    let _ = fs::remove_dir_all(&bridge.directory);
}

pub fn maybe_run_claude_hook_bridge() -> bool {
    if !std::env::args().any(|argument| argument == "--claude-hook-bridge") {
        return false;
    }
    if let Err(error) = run_claude_hook_bridge() {
        eprintln!("CodeVetter Claude hook bridge: {error}");
    }
    true
}

fn run_claude_hook_bridge() -> Result<(), String> {
    let event_path = std::env::var_os("CODEVETTER_AGENT_EVENT_FILE")
        .map(PathBuf::from)
        .ok_or_else(|| "event file is unavailable".to_string())?;
    let response_dir = std::env::var_os("CODEVETTER_AGENT_RESPONSE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "response directory is unavailable".to_string())?;
    let mut raw = String::new();
    std::io::stdin()
        .take((CLAUDE_HOOK_EVENT_LIMIT_CHARS + 1) as u64)
        .read_to_string(&mut raw)
        .map_err(|error| format!("read hook input: {error}"))?;
    if raw.chars().count() > CLAUDE_HOOK_EVENT_LIMIT_CHARS {
        return Err("hook input exceeded the event limit".to_string());
    }
    let pending_request = append_claude_hook_payload(&raw, &event_path, &response_dir)?;
    let Some((pending_path, response_path)) = pending_request else {
        return Ok(());
    };

    if let Some(response) = wait_for_claude_permission_response(
        &pending_path,
        &response_path,
        Duration::from_secs(110),
    )? {
        println!("{response}");
    }
    Ok(())
}

fn wait_for_claude_permission_response(
    pending_path: &Path,
    response_path: &Path,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if response_path.is_file() {
            let response = fs::read_to_string(response_path)
                .map_err(|error| format!("read hook response: {error}"))?;
            let _ = fs::remove_file(response_path);
            let _ = fs::remove_file(pending_path);
            if response.chars().count() <= CLAUDE_HOOK_EVENT_LIMIT_CHARS {
                return Ok(Some(response.trim().to_string()));
            }
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = fs::remove_file(pending_path);
    Ok(None)
}

fn append_claude_hook_payload(
    raw: &str,
    event_path: &Path,
    response_dir: &Path,
) -> Result<Option<(PathBuf, PathBuf)>, String> {
    let input =
        serde_json::from_str::<Value>(raw).map_err(|error| format!("parse hook input: {error}"))?;
    if !input.is_object() {
        return Err("hook input must be an object".to_string());
    }
    let pending_request =
        if input.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest") {
            claude_hook_request_id(&input).map(|request_id| {
                let pending_path = claude_hook_pending_path(response_dir, &request_id);
                let response_path = claude_hook_response_path(response_dir, &request_id);
                (pending_path, response_path)
            })
        } else {
            None
        };
    if let Some((pending_path, _)) = pending_request.as_ref() {
        create_private_marker(pending_path)?;
    }

    let append_result =
        OpenOptions::new()
            .append(true)
            .open(event_path)
            .and_then(|mut event_file| {
                event_file
                    .write_all(raw.trim().as_bytes())
                    .and_then(|_| event_file.write_all(b"\n"))
                    .and_then(|_| event_file.flush())
            });
    if let Err(error) = append_result {
        if let Some((pending_path, _)) = pending_request.as_ref() {
            let _ = fs::remove_file(pending_path);
        }
        return Err(format!("append hook event: {error}"));
    }
    Ok(pending_request)
}

pub(crate) fn claude_permission_response_available(terminal_id: &str, request_id: &str) -> bool {
    codex_agents()
        .lock()
        .ok()
        .and_then(|sessions| {
            sessions
                .get(terminal_id.trim())
                .and_then(|session| session.claude_response_dir.as_ref())
                .map(|directory| claude_hook_pending_path(directory, request_id).is_file())
        })
        .unwrap_or(false)
}

pub(crate) fn resolve_claude_permission_request(
    terminal_id: &str,
    request_id: &str,
    allow: bool,
) -> Result<(), String> {
    let response_dir = {
        let sessions = codex_agents()
            .lock()
            .map_err(|error| format!("agent registry lock poisoned: {error}"))?;
        let session = sessions
            .get(terminal_id.trim())
            .ok_or_else(|| format!("Agent is not running: {}", terminal_id.trim()))?;
        if session.provider != AgentProvider::Claude {
            return Err("Only Claude hook requests use this response channel".to_string());
        }
        session
            .claude_response_dir
            .clone()
            .ok_or_else(|| "Claude response bridge is unavailable".to_string())?
    };
    let pending_path = claude_hook_pending_path(&response_dir, request_id);
    if !pending_path.is_file() {
        return Err("Claude permission request is stale".to_string());
    }
    let response_path = claude_hook_response_path(&response_dir, request_id);
    let temporary_path = response_path.with_extension(format!(
        "json.{}.{}.tmp",
        std::process::id(),
        current_unix_millis()
    ));
    let response = claude_permission_decision(allow);
    write_private_file(
        &temporary_path,
        serde_json::to_string(&response)
            .map_err(|error| format!("serialize Claude permission decision: {error}"))?
            .as_bytes(),
    )?;
    fs::rename(&temporary_path, &response_path)
        .map_err(|error| format!("publish Claude permission decision: {error}"))?;
    Ok(())
}

fn claude_permission_decision(allow: bool) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {
                "behavior": if allow { "allow" } else { "deny" },
                "message": if allow {
                    Value::Null
                } else {
                    Value::String("Denied from CodeVetter".to_string())
                }
            }
        }
    })
}

fn claude_hook_request_id(input: &Value) -> Option<String> {
    [
        "/tool_use_id",
        "/permission_request_id",
        "/tool_input/request_id",
    ]
    .into_iter()
    .find_map(|pointer| input.pointer(pointer).and_then(Value::as_str))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(|value| value.chars().take(256).collect())
}

fn claude_hook_pending_path(directory: &Path, request_id: &str) -> PathBuf {
    directory.join(format!("{}.pending", claude_hook_request_key(request_id)))
}

fn claude_hook_response_path(directory: &Path, request_id: &str) -> PathBuf {
    directory.join(format!("{}.json", claude_hook_request_key(request_id)))
}

fn claude_hook_request_key(request_id: &str) -> String {
    format!("{:x}", Sha256::digest(request_id.as_bytes()))
}

fn create_private_marker(path: &Path) -> Result<(), String> {
    write_private_file(path, b"pending")
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(bytes).and_then(|_| file.flush()))
        .map_err(|error| format!("write private hook bridge file: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn start_claude_hook_event_reader(
    app: AppHandle,
    terminal_id: String,
    pid: Option<u32>,
    events_path: PathBuf,
    stop: Arc<AtomicBool>,
    last_agent_event: Arc<Mutex<Option<String>>>,
    agent_events: Arc<Mutex<Vec<AgentStructuredEvent>>>,
    provider_session_id: Arc<Mutex<Option<String>>>,
    transcript_path: Arc<Mutex<Option<String>>>,
) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name(format!("Claude hook reader {terminal_id}"))
        .spawn(move || {
            let Ok(file) = OpenOptions::new().read(true).open(events_path) else {
                return;
            };
            let mut reader = BufReader::new(file);
            let mut pending = String::new();
            let mut seq = 0_u64;
            loop {
                match reader.read_line(&mut pending) {
                    Ok(0) if stop.load(Ordering::Acquire) => break,
                    Ok(0) => thread::sleep(Duration::from_millis(CLAUDE_HOOK_POLL_INTERVAL_MS)),
                    Ok(_) if !pending.ends_with('\n') => continue,
                    Ok(_) => {
                        let raw = pending.trim();
                        if raw.chars().count() <= CLAUDE_HOOK_EVENT_LIMIT_CHARS {
                            if let Some(event) = normalize_claude_hook_event(raw) {
                                seq = seq.saturating_add(1);
                                if let Some((session, transcript)) = agent_event_identity(&event) {
                                    if let Some(session) = session {
                                        if let Ok(mut value) = provider_session_id.lock() {
                                            *value = Some(session);
                                        }
                                    }
                                    if let Some(transcript) = transcript {
                                        if let Ok(mut value) = transcript_path.lock() {
                                            *value = Some(transcript);
                                        }
                                    }
                                }
                                if let Ok(mut last) = last_agent_event.lock() {
                                    *last = Some(event.clone());
                                }
                                append_agent_structured_event(
                                    &agent_events,
                                    AgentStructuredEvent {
                                        seq,
                                        at_ms: current_unix_millis(),
                                        data: event.clone(),
                                    },
                                );
                                emit_agent_event(
                                    &app,
                                    &terminal_id,
                                    "agent_event",
                                    Some(event),
                                    pid,
                                    Some(0),
                                    Some(seq),
                                    None,
                                    None,
                                );
                            }
                        }
                        pending.clear();
                    }
                    Err(_) => break,
                }
            }
        })
        .map_err(|error| format!("start Claude hook event reader: {error}"))
}

fn normalize_claude_hook_event(raw: &str) -> Option<String> {
    super::agent_stream::normalize_claude_hook_event(raw)
}

fn claude_agent_command_args(
    approval_policy: Option<&str>,
    model: Option<&str>,
    prompt: Option<&str>,
    resume_session_id: Option<&str>,
    fork_session_id: Option<&str>,
    settings_path: Option<&Path>,
) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(settings_path) = settings_path {
        args.push(OsString::from("--settings"));
        args.push(settings_path.as_os_str().to_os_string());
    }
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(OsString::from("--model"));
        args.push(OsString::from(model));
    }
    args.push(OsString::from("--permission-mode"));
    args.push(OsString::from(claude_permission_mode(approval_policy)));

    if let Some(fork_session_id) = fork_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--resume"));
        args.push(OsString::from(fork_session_id));
        args.push(OsString::from("--fork-session"));
    } else if let Some(resume_session_id) = resume_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.push(OsString::from("--resume"));
        args.push(OsString::from(resume_session_id));
    }
    if let Some(prompt) = prompt.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(OsString::from(prompt));
    }
    args
}

fn claude_permission_mode(approval_policy: Option<&str>) -> &'static str {
    match approval_policy.map(str::trim) {
        Some("never") | Some("dontAsk") => "dontAsk",
        Some("plan") | Some("read-only") => "plan",
        Some("acceptEdits") => "acceptEdits",
        Some("auto") => "auto",
        _ => "default",
    }
}

fn agent_terminal_env(provider: AgentProvider) -> Vec<(&'static str, &'static str)> {
    if provider == AgentProvider::Codex {
        return codex_agent_terminal_env();
    }
    vec![
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", CODEVETTER_TERM_PROGRAM),
        ("CODEVETTER_AGENT_PANEL", "1"),
    ]
}

fn codex_agent_terminal_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", CODEVETTER_TERM_PROGRAM),
        ("TERM_PROGRAM_VERSION", CODEVETTER_WARP_COMPAT_VERSION),
        ("CODEVETTER_AGENT_PANEL", "1"),
        (
            "WARP_CLI_AGENT_PROTOCOL_VERSION",
            WARP_CLI_AGENT_PROTOCOL_VERSION,
        ),
        ("WARP_CLIENT_VERSION", CODEVETTER_WARP_COMPAT_VERSION),
    ]
}

fn run_agent_pty_control_loop(
    app: AppHandle,
    session_id: String,
    provider: AgentProvider,
    mut writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
    rx: Receiver<AgentPtyCommand>,
    pid: Option<u32>,
) {
    for message in rx {
        match message {
            AgentPtyCommand::Input(data) => {
                if let Err(error) = writer.write_all(&data).and_then(|_| writer.flush()) {
                    emit_agent_event(
                        &app,
                        &session_id,
                        "error",
                        Some(format!("write {} input: {error}", provider.display_name())),
                        pid,
                        None,
                        None,
                        None,
                        Some(false),
                    );
                    break;
                }
            }
            AgentPtyCommand::Resize(size) => {
                if let Err(error) = master.resize(size) {
                    emit_agent_event(
                        &app,
                        &session_id,
                        "error",
                        Some(format!("resize {} PTY: {error}", provider.display_name())),
                        pid,
                        None,
                        None,
                        None,
                        Some(false),
                    );
                }
            }
            AgentPtyCommand::Stop => {
                if let Err(error) = writer
                    .write_all(AGENT_GRACEFUL_EXIT_COMMAND)
                    .and_then(|_| writer.flush())
                {
                    if let Err(kill_error) = killer.kill() {
                        emit_agent_event(
                            &app,
                            &session_id,
                            "error",
                            Some(format!(
                                "stop {} agent after /exit write failed ({error}): {kill_error}",
                                provider.display_name(),
                            )),
                            pid,
                            None,
                            None,
                            None,
                            Some(false),
                        );
                    }
                } else {
                    schedule_force_stop_after_grace(
                        app.clone(),
                        session_id.clone(),
                        provider,
                        killer,
                        pid,
                    );
                }
                break;
            }
        }
    }
}

fn schedule_force_stop_after_grace(
    app: AppHandle,
    session_id: String,
    provider: AgentProvider,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
    pid: Option<u32>,
) {
    let _ = thread::Builder::new()
        .name(format!(
            "{} PTY force stop {session_id}",
            provider.display_name()
        ))
        .spawn(move || {
            thread::sleep(Duration::from_millis(CODEX_FORCE_STOP_AFTER_MS));
            let still_running = codex_agents()
                .lock()
                .map(|sessions| sessions.contains_key(&session_id))
                .unwrap_or(false);
            if !still_running {
                return;
            }
            if let Err(error) = killer.kill() {
                emit_agent_event(
                    &app,
                    &session_id,
                    "error",
                    Some(format!(
                        "force stop {} agent after /exit: {error}",
                        provider.display_name()
                    )),
                    pid,
                    None,
                    None,
                    None,
                    Some(false),
                );
            }
        });
}

fn extract_codex_agent_notifications(
    buffer: &mut String,
    rich_notifications_active: &mut bool,
    chunk: &str,
) -> Vec<String> {
    const RICH_PREFIX: &str = "\x1b]777;notify;";
    const OSC9_PREFIX: &str = "\x1b]9;";
    const TITLE: &str = "warp://cli-agent";
    const MAX_BUFFER_CHARS: usize = 128 * 1024;

    buffer.push_str(chunk);
    let mut notifications = Vec::new();

    loop {
        let Some((start, prefix)) = earliest_osc_notification(buffer, RICH_PREFIX, OSC9_PREFIX)
        else {
            if buffer.len() > MAX_BUFFER_CHARS {
                let keep_from = buffer
                    .char_indices()
                    .rev()
                    .nth(MAX_BUFFER_CHARS / 4)
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
                buffer.drain(..keep_from);
            }
            break;
        };
        if start > 0 {
            buffer.drain(..start);
        }

        let payload_start = prefix.len();
        let Some((terminator_start, terminator_len)) =
            find_osc_terminator(&buffer[payload_start..])
        else {
            if buffer.len() > MAX_BUFFER_CHARS {
                buffer.truncate(MAX_BUFFER_CHARS);
            }
            break;
        };
        let payload_end = payload_start + terminator_start;
        let payload = &buffer[payload_start..payload_end];
        if prefix == RICH_PREFIX {
            if let Some((title, body)) = payload.split_once(';') {
                if title == TITLE && is_codex_cli_agent_payload(body) {
                    *rich_notifications_active = true;
                    notifications.push(body.to_string());
                }
            }
        } else if !*rich_notifications_active {
            if let Some(body) = codex_osc9_fallback_payload(payload) {
                notifications.push(body.to_string());
            }
        }
        buffer.drain(..payload_end + terminator_len);
    }

    notifications
}

fn earliest_osc_notification<'a>(
    value: &str,
    rich_prefix: &'a str,
    osc9_prefix: &'a str,
) -> Option<(usize, &'a str)> {
    match (value.find(rich_prefix), value.find(osc9_prefix)) {
        (Some(rich), Some(osc9)) if rich <= osc9 => Some((rich, rich_prefix)),
        (Some(_), Some(osc9)) => Some((osc9, osc9_prefix)),
        (Some(rich), None) => Some((rich, rich_prefix)),
        (None, Some(osc9)) => Some((osc9, osc9_prefix)),
        (None, None) => None,
    }
}

fn find_osc_terminator(value: &str) -> Option<(usize, usize)> {
    let bel = value.find('\x07').map(|idx| (idx, 1));
    let st = value.find("\x1b\\").map(|idx| (idx, 2));
    match (bel, st) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn codex_osc9_fallback_payload(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    Some(
        json!({
            "v": 1,
            "agent": "codex",
            "event": "stop",
            "query": body,
            "fallback": "osc9",
        })
        .to_string(),
    )
}

#[cfg(test)]
fn extract_codex_warp_notifications(buffer: &mut String, chunk: &str) -> Vec<String> {
    let mut rich_notifications_active = false;
    extract_codex_agent_notifications(buffer, &mut rich_notifications_active, chunk)
}

fn is_codex_cli_agent_payload(body: &str) -> bool {
    serde_json::from_str::<AgentLifecycleNotification>(body)
        .ok()
        .and_then(|payload| payload.agent)
        .is_some_and(|agent| agent == "codex")
}

fn agent_event_identity(notification: &str) -> Option<(Option<String>, Option<String>)> {
    let payload = serde_json::from_str::<AgentLifecycleNotification>(notification).ok()?;
    if !matches!(payload.agent.as_deref(), Some("codex" | "claude")) {
        return None;
    }
    let codex_session_id = payload
        .session_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let transcript_path = payload
        .transcript_path
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Some((codex_session_id, transcript_path))
}

fn append_output_tail(output_tail: &Arc<Mutex<String>>, chunk: &str) {
    if chunk.is_empty() {
        return;
    }
    let Ok(mut tail) = output_tail.lock() else {
        return;
    };
    tail.push_str(chunk);
    trim_string_tail(&mut tail, AGENT_OUTPUT_TAIL_CHARS);
}

fn append_agent_structured_event(
    events: &Arc<Mutex<Vec<AgentStructuredEvent>>>,
    event: AgentStructuredEvent,
) {
    let Ok(mut events) = events.lock() else {
        return;
    };
    events.push(event);
    if events.len() > AGENT_EVENT_LOG_LIMIT {
        let excess = events.len() - AGENT_EVENT_LOG_LIMIT;
        events.drain(..excess);
    }
}

fn trim_string_tail(value: &mut String, max_chars: usize) {
    if value.chars().count() <= max_chars {
        return;
    }
    let keep_from = value
        .char_indices()
        .rev()
        .nth(max_chars.saturating_sub(1))
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    value.drain(..keep_from);
}

#[derive(Clone)]
struct CodexCommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn load_codex_warp_plugin_status() -> Result<CodexWarpPluginStatus, String> {
    let codex_path = resolve_agent_cli_path("codex");
    let marketplace = run_codex_command(&codex_path, &["plugin", "marketplace", "list"]);
    let plugins = run_codex_command(&codex_path, &["plugin", "list"]);

    let codex_available = marketplace.is_ok() || plugins.is_ok();
    let marketplace_output = marketplace
        .as_ref()
        .map(command_combined_output)
        .unwrap_or_else(|error| error.clone());
    let plugin_output = plugins
        .as_ref()
        .map(command_combined_output)
        .unwrap_or_else(|error| error.clone());
    let marketplace_installed = marketplace_output
        .lines()
        .any(|line| line.contains(CODEX_WARP_MARKETPLACE));
    let warp_plugin_status = plugin_status_line(&plugin_output, CODEX_WARP_PLUGIN);
    let orchestration_plugin_status =
        plugin_status_line(&plugin_output, CODEX_WARP_ORCHESTRATION_PLUGIN);
    let warp_plugin_installed = is_plugin_installed(warp_plugin_status);
    let warp_plugin_enabled = is_plugin_enabled(warp_plugin_status);
    let orchestration_plugin_installed = is_plugin_installed(orchestration_plugin_status);
    let orchestration_plugin_enabled = is_plugin_enabled(orchestration_plugin_status);
    let error = match (marketplace.as_ref(), plugins.as_ref()) {
        (Err(error), _) | (_, Err(error)) => Some(error.clone()),
        (Ok(marketplace), _) if !marketplace.success => Some(format_command_error(
            "list Codex plugin marketplaces",
            marketplace,
        )),
        (_, Ok(plugins)) if !plugins.success => {
            Some(format_command_error("list Codex plugins", plugins))
        }
        _ => None,
    };

    Ok(CodexWarpPluginStatus {
        codex_available,
        marketplace_installed,
        warp_plugin_installed,
        warp_plugin_enabled,
        orchestration_plugin_installed,
        orchestration_plugin_enabled,
        structured_env_enabled: true,
        needs_install: !marketplace_installed || !warp_plugin_installed || !warp_plugin_enabled,
        codex_path,
        marketplace_output: truncate_command_text(&marketplace_output),
        plugin_output: truncate_command_text(&plugin_output),
        error,
    })
}

fn run_codex_command(codex_path: &str, args: &[&str]) -> Result<CodexCommandOutput, String> {
    let output = StdCommand::new(codex_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("run `{}` {}: {e}", codex_path, args.join(" ")))?;
    Ok(CodexCommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn plugin_status_line<'a>(output: &'a str, selector: &str) -> Option<&'a str> {
    output.lines().find(|line| line.contains(selector))
}

fn is_plugin_installed(status_line: Option<&str>) -> bool {
    status_line.is_some_and(|line| line.contains("installed") && !line.contains("not installed"))
}

fn is_plugin_enabled(status_line: Option<&str>) -> bool {
    status_line.is_some_and(|line| is_plugin_installed(Some(line)) && line.contains("enabled"))
}

fn command_combined_output(output: &CodexCommandOutput) -> String {
    match (
        output.stdout.trim().is_empty(),
        output.stderr.trim().is_empty(),
    ) {
        (false, false) => format!("{}\n{}", output.stdout, output.stderr),
        (false, true) => output.stdout.clone(),
        (true, false) => output.stderr.clone(),
        (true, true) => String::new(),
    }
}

fn format_command_error(action: &str, output: &CodexCommandOutput) -> String {
    let details = command_combined_output(output);
    let details = details.trim();
    if details.is_empty() {
        format!("{action} failed")
    } else {
        format!("{action} failed: {details}")
    }
}

fn truncate_command_text(value: &str) -> String {
    const LIMIT: usize = 12_000;
    if value.len() <= LIMIT {
        return value.to_string();
    }
    let keep_from = value
        .char_indices()
        .rev()
        .nth(LIMIT)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    format!("... truncated ...\n{}", &value[keep_from..])
}

#[tauri::command]
pub fn send_codex_agent_terminal_input(session_id: String, data: String) -> Result<(), String> {
    send_agent_terminal_input_impl(session_id, data)
}

fn send_agent_terminal_input_impl(session_id: String, data: String) -> Result<(), String> {
    if super::codex_app_server::is_running(session_id.trim()) {
        return super::codex_app_server::send_input(session_id.trim(), &data);
    }
    let (tx, provider) = {
        let sessions = codex_agents()
            .lock()
            .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
        let session = sessions
            .get(session_id.trim())
            .ok_or_else(|| format!("Agent is not running: {}", session_id.trim()))?;
        (session.tx.clone(), session.provider)
    };
    tx.send(AgentPtyCommand::Input(data.into_bytes()))
        .map_err(|e| format!("send {} input: {e}", provider.display_name()))
}

pub(crate) fn send_agent_terminal_input_from_native(
    session_id: &str,
    data: &str,
) -> Result<(), String> {
    send_agent_terminal_input_impl(session_id.to_string(), data.to_string())
}

#[tauri::command]
pub fn send_agent_terminal_input(session_id: String, data: String) -> Result<(), String> {
    send_agent_terminal_input_impl(session_id, data)
}

#[tauri::command]
pub fn stop_codex_agent_terminal(session_id: String) -> Result<(), String> {
    stop_agent_terminal_impl(session_id)
}

fn stop_agent_terminal_impl(session_id: String) -> Result<(), String> {
    if super::codex_app_server::is_running(session_id.trim()) {
        return super::codex_app_server::stop(session_id.trim());
    }
    let (tx, provider, stop_requested) = {
        let sessions = codex_agents()
            .lock()
            .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
        let session = sessions
            .get(session_id.trim())
            .ok_or_else(|| format!("Agent is not running: {}", session_id.trim()))?;
        (
            session.tx.clone(),
            session.provider,
            Arc::clone(&session.stop_requested),
        )
    };
    let already_requested = stop_requested.swap(true, Ordering::AcqRel);
    if let Err(error) = tx.send(AgentPtyCommand::Stop) {
        if !already_requested {
            stop_requested.store(false, Ordering::Release);
        }
        return Err(format!("send {} stop: {error}", provider.display_name()));
    }
    Ok(())
}

#[tauri::command]
pub fn stop_agent_terminal(session_id: String) -> Result<(), String> {
    stop_agent_terminal_impl(session_id)
}

#[tauri::command]
pub fn resize_codex_agent_terminal(session_id: String, cols: u16, rows: u16) -> Result<(), String> {
    resize_agent_terminal_impl(session_id, cols, rows)
}

fn resize_agent_terminal_impl(session_id: String, cols: u16, rows: u16) -> Result<(), String> {
    if super::codex_app_server::is_running(session_id.trim()) {
        return Ok(());
    }
    let (tx, provider) = {
        let sessions = codex_agents()
            .lock()
            .map_err(|e| format!("agent registry lock poisoned: {e}"))?;
        let session = sessions
            .get(session_id.trim())
            .ok_or_else(|| format!("Agent is not running: {}", session_id.trim()))?;
        (session.tx.clone(), session.provider)
    };
    tx.send(AgentPtyCommand::Resize(PtySize {
        rows: rows.max(8),
        cols: cols.max(40),
        pixel_width: 0,
        pixel_height: 0,
    }))
    .map_err(|e| format!("send {} resize: {e}", provider.display_name()))
}

#[tauri::command]
pub fn resize_agent_terminal(session_id: String, cols: u16, rows: u16) -> Result<(), String> {
    resize_agent_terminal_impl(session_id, cols, rows)
}

fn flush_pending_pty_output_if_due(
    app: &AppHandle,
    session_id: &str,
    pending_output: &mut String,
    output_seq: &mut u64,
    pid: Option<u32>,
    last_output_emit: &mut Instant,
) {
    if pending_output.is_empty() {
        return;
    }
    let elapsed = last_output_emit.elapsed();
    if pending_output.len() < PTY_OUTPUT_EMIT_CHARS
        && elapsed < Duration::from_millis(PTY_OUTPUT_EMIT_INTERVAL_MS)
    {
        thread::sleep(Duration::from_millis(PTY_OUTPUT_EMIT_INTERVAL_MS) - elapsed);
    }
    flush_pending_pty_output(
        app,
        session_id,
        pending_output,
        output_seq,
        pid,
        last_output_emit,
    );
}

fn flush_pending_pty_output(
    app: &AppHandle,
    session_id: &str,
    pending_output: &mut String,
    output_seq: &mut u64,
    pid: Option<u32>,
    last_output_emit: &mut Instant,
) {
    if pending_output.is_empty() {
        return;
    }
    *output_seq = output_seq.saturating_add(1);
    let chunk = std::mem::take(pending_output);
    emit_agent_event(
        app,
        session_id,
        "output",
        Some(chunk),
        pid,
        Some(0),
        Some(*output_seq),
        None,
        None,
    );
    *last_output_emit = Instant::now();
}

pub(crate) fn emit_agent_event(
    app: &AppHandle,
    session_id: &str,
    kind: &str,
    data: Option<String>,
    pid: Option<u32>,
    idle_ms: Option<u64>,
    seq: Option<u64>,
    exit_code: Option<u32>,
    success: Option<bool>,
) {
    let event = AgentTerminalEvent {
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        data,
        pid,
        idle_ms,
        seq,
        exit_code,
        success,
        intentional_stop: None,
    };
    let _ = app.emit(AGENT_TERMINAL_EVENT, event.clone());
    super::native_agent_island::ingest_agent_terminal_event(app, &event);
}

pub(crate) fn emit_agent_exit_event(
    app: &AppHandle,
    session_id: &str,
    pid: Option<u32>,
    exit_code: Option<u32>,
    success: Option<bool>,
    data: Option<String>,
    intentional_stop: bool,
) {
    let event = agent_exit_event(session_id, pid, exit_code, success, data, intentional_stop);
    let _ = app.emit(AGENT_TERMINAL_EVENT, event.clone());
    super::native_agent_island::ingest_agent_terminal_event(app, &event);
}

fn agent_exit_event(
    session_id: &str,
    pid: Option<u32>,
    exit_code: Option<u32>,
    success: Option<bool>,
    data: Option<String>,
    intentional_stop: bool,
) -> AgentTerminalEvent {
    AgentTerminalEvent {
        session_id: session_id.to_string(),
        kind: "exit".to_string(),
        data: if intentional_stop {
            Some("Stopped by user".to_string())
        } else {
            data
        },
        pid,
        idle_ms: None,
        seq: None,
        exit_code,
        success: if intentional_stop {
            Some(true)
        } else {
            success
        },
        intentional_stop: Some(intentional_stop),
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[tauri::command]
pub async fn run_agent_terminal_command(
    command: String,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
) -> Result<AgentTerminalCommandResult, String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("command is required".into());
    }

    let cwd = resolve_cwd(cwd.as_deref())?;
    let timeout_ms = timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, MAX_TIMEOUT_MS);

    tokio::task::spawn_blocking(move || run_shell_command(command, cwd, timeout_ms))
        .await
        .map_err(|e| format!("agent terminal task join error: {e}"))?
}

fn run_shell_command(
    command: String,
    cwd: PathBuf,
    timeout_ms: u64,
) -> Result<AgentTerminalCommandResult, String> {
    if let Some(target) = parse_cd_command(&command) {
        let started = Instant::now();
        let next_cwd = resolve_cd_cwd(&cwd, target)?;
        return Ok(AgentTerminalCommandResult {
            command,
            cwd: next_cwd.to_string_lossy().to_string(),
            exit_code: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            timeout_ms,
            timed_out: false,
            success: true,
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        });
    }

    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let mut child = shell_command(&command)
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn shell command: {e}"))?;

    let output = loop {
        match child
            .try_wait()
            .map_err(|e| format!("poll shell command: {e}"))?
        {
            Some(_) => {
                break child
                    .wait_with_output()
                    .map_err(|e| format!("read command output: {e}"))?;
            }
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .map_err(|e| format!("read timed-out command output: {e}"))?;
                let duration_ms = started.elapsed().as_millis() as u64;
                let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();
                let (stdout, stdout_truncated) = trim_output(stdout_raw);
                let (stderr, stderr_truncated) = trim_output(stderr_raw);
                return Ok(AgentTerminalCommandResult {
                    command,
                    cwd: cwd.to_string_lossy().to_string(),
                    exit_code: output.status.code().unwrap_or(-1),
                    duration_ms,
                    timeout_ms,
                    timed_out: true,
                    success: false,
                    stdout,
                    stderr,
                    stdout_truncated,
                    stderr_truncated,
                });
            }
            None => std::thread::sleep(Duration::from_millis(80)),
        }
    };

    let duration_ms = started.elapsed().as_millis() as u64;
    let stdout_raw = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();
    let (stdout, stdout_truncated) = trim_output(stdout_raw);
    let (stderr, stderr_truncated) = trim_output(stderr_raw);
    let exit_code = output.status.code().unwrap_or(-1);

    Ok(AgentTerminalCommandResult {
        command,
        cwd: cwd.to_string_lossy().to_string(),
        exit_code,
        duration_ms,
        timeout_ms,
        timed_out: false,
        success: output.status.success(),
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

fn shell_command(command: &str) -> StdCommand {
    #[cfg(target_family = "windows")]
    {
        let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut cmd = StdCommand::new(shell);
        cmd.args(["/C", command]);
        cmd
    }

    #[cfg(not(target_family = "windows"))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut cmd = StdCommand::new(shell);
        cmd.args(["-lc", command]);
        cmd
    }
}

fn resolve_cwd(cwd: Option<&str>) -> Result<PathBuf, String> {
    let raw = cwd.map(str::trim).filter(|value| !value.is_empty());
    let path = raw.map(PathBuf::from).unwrap_or_else(default_cwd);
    let expanded = expand_home(path);
    let canonical = expanded
        .canonicalize()
        .map_err(|e| format!("resolve cwd {}: {e}", expanded.display()))?;
    if !canonical.is_dir() {
        return Err(format!("cwd is not a directory: {}", canonical.display()));
    }
    Ok(canonical)
}

fn parse_cd_command(command: &str) -> Option<&str> {
    let trimmed = command.trim();
    if trimmed == "cd" {
        return Some("~");
    }
    let target = trimmed.strip_prefix("cd ")?;
    if target.contains("&&") || target.contains(';') || target.contains('|') {
        return None;
    }
    Some(strip_wrapping_quotes(target.trim()))
}

fn strip_wrapping_quotes(value: &str) -> &str {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return &value[1..value.len() - 1];
        }
    }
    value
}

fn resolve_cd_cwd(base: &Path, target: &str) -> Result<PathBuf, String> {
    let target = target.trim();
    let path = if target.is_empty() {
        default_cwd()
    } else {
        let expanded = expand_home(PathBuf::from(target));
        if expanded.is_absolute() {
            expanded
        } else {
            base.join(expanded)
        }
    };
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("resolve cd target {}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "cd target is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn default_cwd() -> PathBuf {
    #[cfg(target_family = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    #[cfg(not(target_family = "windows"))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}

fn expand_home(path: PathBuf) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path;
    };
    if raw == "~" {
        return default_cwd();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return default_cwd().join(rest);
    }
    path
}

fn trim_output(raw: String) -> (String, bool) {
    if raw.chars().count() <= OUTPUT_LIMIT_CHARS {
        return (raw, false);
    }
    let trimmed = raw
        .chars()
        .rev()
        .take(OUTPUT_LIMIT_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    (trimmed, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intentional_stop_is_a_clean_resumable_exit_even_when_pty_reports_hangup() {
        let event = agent_exit_event(
            "agent-1",
            Some(42),
            Some(1),
            Some(false),
            Some("terminated by Hangup".to_string()),
            true,
        );

        assert_eq!(event.kind, "exit");
        assert_eq!(event.exit_code, Some(1));
        assert_eq!(event.success, Some(true));
        assert_eq!(event.intentional_stop, Some(true));
        assert_eq!(event.data.as_deref(), Some("Stopped by user"));
    }

    #[test]
    fn unexpected_hangup_remains_a_failure() {
        let event = agent_exit_event(
            "agent-1",
            Some(42),
            Some(1),
            Some(false),
            Some("terminated by Hangup".to_string()),
            false,
        );

        assert_eq!(event.exit_code, Some(1));
        assert_eq!(event.success, Some(false));
        assert_eq!(event.intentional_stop, Some(false));
        assert_eq!(event.data.as_deref(), Some("terminated by Hangup"));
    }

    #[test]
    fn resolves_empty_cwd_to_existing_directory() {
        let cwd = resolve_cwd(None).expect("cwd");
        assert!(cwd.is_dir());
    }

    #[test]
    fn runs_shell_command_and_captures_stdout() {
        let cwd = std::env::current_dir().expect("cwd");
        let result =
            run_shell_command("printf agent-terminal".to_string(), cwd, 5_000).expect("run");
        assert!(result.success);
        assert_eq!(result.stdout, "agent-terminal");
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn reports_non_zero_exit() {
        let cwd = std::env::current_dir().expect("cwd");
        let result = run_shell_command("exit 7".to_string(), cwd, 5_000).expect("run");
        assert!(!result.success);
        assert_eq!(result.exit_code, 7);
    }

    #[test]
    fn cd_command_updates_cwd_without_spawning_shell_state() {
        let cwd = std::env::current_dir().expect("cwd");
        let result = run_shell_command("cd ..".to_string(), cwd.clone(), 5_000).expect("run");
        assert!(result.success);
        assert_eq!(
            PathBuf::from(result.cwd),
            cwd.parent().expect("parent").canonicalize().unwrap()
        );
    }

    #[test]
    fn trims_large_output_from_the_tail() {
        let (out, truncated) = trim_output("x".repeat(OUTPUT_LIMIT_CHARS + 10));
        assert!(truncated);
        assert_eq!(out.len(), OUTPUT_LIMIT_CHARS);
    }

    #[test]
    fn trims_agent_output_tail_from_the_tail() {
        let mut value = "abcdef".to_string();
        trim_string_tail(&mut value, 3);
        assert_eq!(value, "def");
    }

    #[test]
    fn codex_agent_command_args_build_start_command() {
        let cwd = Path::new("/tmp/project");
        let args = command_args_as_strings(codex_agent_command_args(
            cwd,
            Some("workspace-write"),
            Some("on-request"),
            Some("gpt-5.5"),
            Some("review changes"),
            None,
            None,
        ));

        assert_eq!(
            args,
            vec![
                "--no-alt-screen",
                "-C",
                "/tmp/project",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
                "-m",
                "gpt-5.5",
                "review changes",
            ]
        );
    }

    #[test]
    fn codex_agent_command_args_build_resume_command() {
        let args = command_args_as_strings(codex_agent_command_args(
            Path::new("/tmp/project"),
            None,
            None,
            None,
            None,
            Some("codex-session-1"),
            None,
        ));

        assert_eq!(
            args,
            vec![
                "resume",
                "--no-alt-screen",
                "-C",
                "/tmp/project",
                "-s",
                "workspace-write",
                "-a",
                "on-request",
                "codex-session-1",
            ]
        );
    }

    #[test]
    fn codex_agent_command_args_build_fork_command() {
        let args = command_args_as_strings(codex_agent_command_args(
            Path::new("/tmp/project"),
            Some("read-only"),
            Some("never"),
            Some("gpt-5.5"),
            Some("continue from fork"),
            None,
            Some("codex-session-2"),
        ));

        assert_eq!(
            args,
            vec![
                "fork",
                "--no-alt-screen",
                "-C",
                "/tmp/project",
                "-s",
                "read-only",
                "-a",
                "never",
                "-m",
                "gpt-5.5",
                "codex-session-2",
                "continue from fork",
            ]
        );
    }

    #[test]
    fn claude_agent_command_args_build_safe_start_command() {
        let args = command_args_as_strings(claude_agent_command_args(
            Some("on-request"),
            Some("claude-opus-4-6"),
            Some("review changes"),
            None,
            None,
            Some(Path::new("/tmp/codevetter/settings.json")),
        ));
        assert_eq!(
            args,
            vec![
                "--settings",
                "/tmp/codevetter/settings.json",
                "--model",
                "claude-opus-4-6",
                "--permission-mode",
                "default",
                "review changes",
            ]
        );
        assert!(!args.iter().any(|arg| arg.contains("dangerously")));
    }

    #[test]
    fn claude_agent_command_args_build_resume_and_fork_commands() {
        let resume = command_args_as_strings(claude_agent_command_args(
            Some("never"),
            None,
            None,
            Some("claude-session-1"),
            None,
            None,
        ));
        assert_eq!(
            resume,
            vec![
                "--permission-mode",
                "dontAsk",
                "--resume",
                "claude-session-1"
            ]
        );

        let fork = command_args_as_strings(claude_agent_command_args(
            Some("read-only"),
            None,
            Some("continue safely"),
            None,
            Some("claude-session-2"),
            None,
        ));
        assert_eq!(
            fork,
            vec![
                "--permission-mode",
                "plan",
                "--resume",
                "claude-session-2",
                "--fork-session",
                "continue safely"
            ]
        );
    }

    #[test]
    fn codex_agent_terminal_env_declares_terminal_capabilities() {
        let env = codex_agent_terminal_env();
        assert!(env.contains(&("TERM", "xterm-256color")));
        assert!(env.contains(&("COLORTERM", "truecolor")));
        assert!(env.contains(&("TERM_PROGRAM", "CodeVetter")));
        assert!(env.contains(&("TERM_PROGRAM_VERSION", "codevetter-agent-panel-0.1",)));
        assert!(env.contains(&("CODEVETTER_AGENT_PANEL", "1")));
        assert!(env.contains(&("WARP_CLI_AGENT_PROTOCOL_VERSION", "1")));
        assert!(env.contains(&("WARP_CLIENT_VERSION", "codevetter-agent-panel-0.1",)));
    }

    #[test]
    fn extracts_codex_warp_cli_agent_notification() {
        let mut buffer = String::new();
        let body = r#"{"v":1,"agent":"codex","event":"permission_request","summary":"Wants to run shell"}"#;
        let chunk = format!("before\x1b]777;notify;warp://cli-agent;{body}\x07after");
        let notifications = extract_codex_warp_notifications(&mut buffer, &chunk);
        assert_eq!(notifications, vec![body.to_string()]);
        assert_eq!(buffer, "after");
    }

    #[test]
    fn extracts_split_codex_warp_notification() {
        let mut buffer = String::new();
        let body = r#"{"v":1,"agent":"codex","event":"stop","response":"done"}"#;
        assert!(extract_codex_warp_notifications(
            &mut buffer,
            "\x1b]777;notify;warp://cli-agent;{\"v\":1,"
        )
        .is_empty());
        let notifications = extract_codex_warp_notifications(
            &mut buffer,
            "\"agent\":\"codex\",\"event\":\"stop\",\"response\":\"done\"}\x07",
        );
        assert_eq!(notifications, vec![body.to_string()]);
    }

    #[test]
    fn ignores_non_codex_warp_notification() {
        let mut buffer = String::new();
        let notifications = extract_codex_warp_notifications(
            &mut buffer,
            "\x1b]777;notify;warp://cli-agent;{\"v\":1,\"agent\":\"claude\",\"event\":\"stop\"}\x07",
        );
        assert!(notifications.is_empty());
    }

    #[test]
    fn extracts_st_terminated_warp_notification() {
        let mut buffer = String::new();
        let body = r#"{"v":1,"agent":"codex","event":"tool_complete","tool_name":"shell"}"#;
        let chunk = format!("\x1b]777;notify;warp://cli-agent;{body}\x1b\\");
        let notifications = extract_codex_warp_notifications(&mut buffer, &chunk);
        assert_eq!(notifications, vec![body.to_string()]);
    }

    #[test]
    fn extracts_codex_osc9_fallback_notification() {
        let mut buffer = String::new();
        let mut rich_active = false;
        let notifications = extract_codex_agent_notifications(
            &mut buffer,
            &mut rich_active,
            "\x1b]9;Finished reviewing changes\x07",
        );
        assert_eq!(notifications.len(), 1);
        let payload: Value = serde_json::from_str(&notifications[0]).expect("json payload");
        assert_eq!(payload["agent"], "codex");
        assert_eq!(payload["event"], "stop");
        assert_eq!(payload["query"], "Finished reviewing changes");
        assert_eq!(payload["fallback"], "osc9");
        assert!(!rich_active);
    }

    #[test]
    fn ignores_codex_osc9_after_rich_notification_is_active() {
        let mut buffer = String::new();
        let mut rich_active = false;
        let body =
            r#"{"v":1,"agent":"codex","event":"permission_request","summary":"review hooks"}"#;
        let notifications = extract_codex_agent_notifications(
            &mut buffer,
            &mut rich_active,
            &format!("\x1b]777;notify;warp://cli-agent;{body}\x07"),
        );
        assert_eq!(notifications, vec![body.to_string()]);
        assert!(rich_active);

        let notifications = extract_codex_agent_notifications(
            &mut buffer,
            &mut rich_active,
            "\x1b]9;legacy duplicate\x07",
        );
        assert!(notifications.is_empty());
    }

    #[test]
    fn extracts_agent_identity_from_notification() {
        let body = r#"{"v":1,"agent":"codex","event":"stop","session_id":"abc-123","transcript_path":"/tmp/rollout.jsonl"}"#;
        let identity = agent_event_identity(body).expect("identity");
        assert_eq!(identity.0.as_deref(), Some("abc-123"));
        assert_eq!(identity.1.as_deref(), Some("/tmp/rollout.jsonl"));
    }

    #[test]
    fn normalizes_claude_permission_and_question_hooks() {
        let permission = normalize_claude_hook_event(
            r#"{"hook_event_name":"PermissionRequest","session_id":"claude-1","transcript_path":"/tmp/claude.jsonl","tool_name":"Bash"}"#,
        )
        .expect("permission event");
        let permission: Value = serde_json::from_str(&permission).expect("permission json");
        assert_eq!(permission["agent"], "claude");
        assert_eq!(permission["event"], "permission_request");
        assert_eq!(
            permission["summary"],
            "Claude requested permission for Bash"
        );
        assert_eq!(permission["session_id"], "claude-1");

        let question = normalize_claude_hook_event(
            r#"{"hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which release should I use?"}]}}"#,
        )
        .expect("question event");
        let question: Value = serde_json::from_str(&question).expect("question json");
        assert_eq!(question["event"], "question_asked");
        assert_eq!(question["summary"], "Which release should I use?");
    }

    #[test]
    fn normalizes_claude_resume_and_completion_hooks() {
        let tool_start =
            normalize_claude_hook_event(r#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#)
                .expect("tool start event");
        let tool_start: Value = serde_json::from_str(&tool_start).expect("tool start json");
        assert_eq!(tool_start["event"], "tool_start");

        let stop = normalize_claude_hook_event(
            r#"{"hook_event_name":"Stop","last_assistant_message":"All checks pass."}"#,
        )
        .expect("stop event");
        let stop: Value = serde_json::from_str(&stop).expect("stop json");
        assert_eq!(stop["event"], "stop");
        assert_eq!(stop["response"], "All checks pass.");
    }

    #[test]
    fn ignores_unknown_or_invalid_claude_hook_input() {
        assert!(normalize_claude_hook_event("not json").is_none());
        assert!(normalize_claude_hook_event(r#"{"hook_event_name":"Unknown"}"#).is_none());
        assert!(normalize_claude_hook_event(
            r#"{"hook_event_name":"Notification","notification_type":"auth_success"}"#
        )
        .is_none());
        let permission = normalize_claude_hook_event(
            r#"{"hook_event_name":"Notification","notification_type":"permission_prompt"}"#,
        )
        .expect("permission notification");
        assert_eq!(
            serde_json::from_str::<Value>(&permission).expect("permission json")["event"],
            "permission_request"
        );
    }

    #[test]
    fn claude_hook_bridge_is_session_scoped_and_cleanup_is_bounded() {
        let bridge = create_claude_hook_bridge("test-session").expect("bridge");
        assert!(bridge.directory.starts_with(std::env::temp_dir()));
        assert!(bridge.settings_path.exists());
        assert!(bridge.events_path.exists());

        let settings: Value = serde_json::from_slice(
            &fs::read(&bridge.settings_path).expect("read session settings"),
        )
        .expect("settings json");
        assert!(settings.pointer("/hooks/PermissionRequest/0").is_some());
        assert!(settings.pointer("/hooks/Stop/0").is_some());
        assert!(settings.pointer("/hooks/PostToolUse/0").is_some());
        assert!(settings.pointer("/hooks/PostToolUseFailure/0").is_some());
        let command = settings
            .pointer("/hooks/PermissionRequest/0/hooks/0/command")
            .and_then(Value::as_str)
            .expect("hook command");
        assert!(command.contains("CODEVETTER_AGENT_HOOK_BIN"));
        assert!(settings
            .pointer("/hooks/PermissionRequest/0/hooks/0/args")
            .is_none());
        assert!(bridge.response_dir.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&bridge.directory)
                    .expect("bridge directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&bridge.settings_path)
                    .expect("settings metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&bridge.events_path)
                    .expect("events metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );

            append_claude_hook_payload(
                r#"{"hook_event_name":"Stop"}"#,
                &bridge.events_path,
                &bridge.response_dir,
            )
            .expect("append hook payload");
            assert_eq!(
                fs::read_to_string(&bridge.events_path).expect("read hook event stream"),
                "{\"hook_event_name\":\"Stop\"}\n"
            );
        }

        let directory = bridge.directory.clone();
        cleanup_claude_hook_bridge(&bridge);
        assert!(!directory.exists());
    }

    #[test]
    fn claude_permission_bridge_preserves_request_identity_and_decision_shape() {
        let bridge = create_claude_hook_bridge("permission-session").expect("bridge");
        let pending = append_claude_hook_payload(
            r#"{"hook_event_name":"PermissionRequest","tool_use_id":"request/../../unsafe"}"#,
            &bridge.events_path,
            &bridge.response_dir,
        )
        .expect("append permission")
        .expect("pending permission");

        assert!(pending.0.starts_with(&bridge.response_dir));
        assert!(pending.0.is_file());
        assert!(!pending.0.to_string_lossy().contains("unsafe"));
        assert_eq!(
            claude_permission_decision(true)
                .pointer("/hookSpecificOutput/decision/behavior")
                .and_then(Value::as_str),
            Some("allow")
        );
        assert_eq!(
            claude_permission_decision(false)
                .pointer("/hookSpecificOutput/decision/behavior")
                .and_then(Value::as_str),
            Some("deny")
        );

        cleanup_claude_hook_bridge(&bridge);
    }

    #[test]
    fn claude_permission_bridge_times_out_without_fabricating_a_decision() {
        let bridge = create_claude_hook_bridge("timeout-session").expect("bridge");
        let (pending_path, response_path) = append_claude_hook_payload(
            r#"{"hook_event_name":"PermissionRequest","tool_use_id":"request-timeout"}"#,
            &bridge.events_path,
            &bridge.response_dir,
        )
        .expect("append permission")
        .expect("pending permission");

        assert_eq!(
            wait_for_claude_permission_response(&pending_path, &response_path, Duration::ZERO)
                .expect("timeout"),
            None
        );
        assert!(!pending_path.exists());
        assert!(!response_path.exists());

        cleanup_claude_hook_bridge(&bridge);
    }

    #[test]
    fn collect_agent_snapshots_preserves_reattach_metadata() {
        let (tx, _rx) = mpsc::channel();
        let output_tail = Arc::new(Mutex::new("recent terminal output".to_string()));
        let last_agent_event = Arc::new(Mutex::new(Some(
            r#"{"v":1,"agent":"codex","event":"stop","session_id":"codex-session","transcript_path":"/tmp/session.jsonl"}"#.to_string(),
        )));
        let agent_events = Arc::new(Mutex::new(vec![AgentStructuredEvent {
            seq: 7,
            at_ms: 456,
            data: r#"{"v":1,"agent":"codex","event":"stop"}"#.to_string(),
        }]));
        let codex_session_id = Arc::new(Mutex::new(Some("codex-session".to_string())));
        let transcript_path = Arc::new(Mutex::new(Some("/tmp/session.jsonl".to_string())));
        let mut sessions = HashMap::new();
        sessions.insert(
            "pane-1".to_string(),
            RunningCodexAgent {
                tx,
                provider: AgentProvider::Codex,
                pid: Some(42),
                cwd: "/tmp/project".to_string(),
                started_at_ms: 123,
                output_tail,
                last_output_at: Arc::new(Mutex::new(Instant::now())),
                last_agent_event,
                agent_events,
                codex_session_id,
                transcript_path,
                claude_response_dir: None,
                stop_requested: Arc::new(AtomicBool::new(false)),
            },
        );

        let snapshots = collect_agent_snapshots(&sessions);
        assert_eq!(snapshots.len(), 1);
        let snapshot = &snapshots[0];
        assert_eq!(snapshot.session_id, "pane-1");
        assert_eq!(snapshot.provider, AgentProvider::Codex);
        assert_eq!(snapshot.cwd, "/tmp/project");
        assert_eq!(snapshot.pid, Some(42));
        assert_eq!(snapshot.started_at_ms, 123);
        assert!(snapshot.running);
        assert_eq!(snapshot.output_tail, "recent terminal output");
        assert_eq!(snapshot.codex_session_id.as_deref(), Some("codex-session"));
        assert_eq!(
            snapshot.transcript_path.as_deref(),
            Some("/tmp/session.jsonl")
        );
        assert!(snapshot
            .last_agent_event
            .as_deref()
            .is_some_and(|event| event.contains("\"event\":\"stop\"")));
        assert_eq!(snapshot.agent_events.len(), 1);
        assert_eq!(snapshot.agent_events[0].seq, 7);
        assert_eq!(snapshot.agent_events[0].at_ms, 456);
        assert!(snapshot.agent_events[0].data.contains("\"event\":\"stop\""));
    }

    #[test]
    fn resolves_live_attachment_identity_from_the_runtime_registry() {
        let (tx, _rx) = mpsc::channel();
        let mut agent = test_running_agent(tx, Some(42), Instant::now());
        agent.provider = AgentProvider::Claude;
        agent.cwd = "/tmp/authoritative-repo".to_string();
        agent.codex_session_id = Arc::new(Mutex::new(Some("provider-session".to_string())));
        let mut sessions = HashMap::new();
        sessions.insert("terminal-1".to_string(), agent);

        let identity = resolve_live_agent_session_identity_from_registry(&sessions, "terminal-1")
            .expect("resolve identity")
            .expect("live terminal");

        assert_eq!(identity.provider, "claude");
        assert_eq!(identity.project_path, "/tmp/authoritative-repo");
        assert_eq!(
            identity.provider_session_id.as_deref(),
            Some("provider-session")
        );
        assert!(
            resolve_live_agent_session_identity_from_registry(&sessions, "missing")
                .expect("missing lookup")
                .is_none()
        );
    }

    #[test]
    fn append_agent_structured_event_keeps_recent_bounded_log() {
        let events = Arc::new(Mutex::new(Vec::new()));
        for seq in 0..(AGENT_EVENT_LOG_LIMIT as u64 + 5) {
            append_agent_structured_event(
                &events,
                AgentStructuredEvent {
                    seq,
                    at_ms: seq + 100,
                    data: format!(r#"{{"event":"event-{seq}"}}"#),
                },
            );
        }

        let events = events.lock().expect("events");
        assert_eq!(events.len(), AGENT_EVENT_LOG_LIMIT);
        assert_eq!(events.first().expect("first").seq, 5);
        assert_eq!(
            events.last().expect("last").seq,
            AGENT_EVENT_LOG_LIMIT as u64 + 4
        );
    }

    #[test]
    fn collect_agent_heartbeats_reports_all_sessions_without_per_session_threads() {
        let (first_tx, _first_rx) = mpsc::channel();
        let (second_tx, _second_rx) = mpsc::channel();
        let mut sessions = HashMap::new();
        sessions.insert(
            "pane-1".to_string(),
            test_running_agent(
                first_tx,
                Some(1),
                Instant::now() - Duration::from_millis(250),
            ),
        );
        sessions.insert(
            "pane-2".to_string(),
            test_running_agent(
                second_tx,
                Some(2),
                Instant::now() - Duration::from_millis(500),
            ),
        );

        let mut heartbeats = collect_agent_heartbeats(&sessions);
        heartbeats.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(heartbeats.len(), 2);
        assert_eq!(heartbeats[0].0, "pane-1");
        assert_eq!(heartbeats[0].1, Some(1));
        assert!(heartbeats[0].2 >= 200);
        assert_eq!(heartbeats[1].0, "pane-2");
        assert_eq!(heartbeats[1].1, Some(2));
        assert!(heartbeats[1].2 >= 450);
    }

    fn test_running_agent(
        tx: Sender<AgentPtyCommand>,
        pid: Option<u32>,
        last_output_at: Instant,
    ) -> RunningCodexAgent {
        RunningCodexAgent {
            tx,
            provider: AgentProvider::Codex,
            pid,
            cwd: "/tmp/project".to_string(),
            started_at_ms: 0,
            output_tail: Arc::new(Mutex::new(String::new())),
            last_output_at: Arc::new(Mutex::new(last_output_at)),
            last_agent_event: Arc::new(Mutex::new(None)),
            agent_events: Arc::new(Mutex::new(Vec::new())),
            codex_session_id: Arc::new(Mutex::new(None)),
            transcript_path: Arc::new(Mutex::new(None)),
            claude_response_dir: None,
            stop_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    fn command_args_as_strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect()
    }
}
