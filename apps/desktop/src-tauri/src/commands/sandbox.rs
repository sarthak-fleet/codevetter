//! T-Rex sandbox: runs a candidate branch in isolation, drives the dev
//! server, optionally runs project tests, then asks an LLM to synthesize a
//! verdict (APPROVE / NEEDS_REVIEW / BLOCK) + any new findings discovered
//! via execution. Mirrors the worktree pattern from `commands/review.rs`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[cfg(feature = "browser-agent")]
use crate::agent::cli_brain::CliBrain;
use crate::agent::local_server::LocalServer;
#[cfg(feature = "browser-agent")]
use crate::agent::runner::run_with_brain;
#[cfg(feature = "browser-agent")]
use crate::agent::types::AgentRunInput;
use crate::agent::types::AgentStep;
use crate::db::queries;
use crate::DbState;

const STEP_EVENT: &str = "sandbox:step";

// Verdicts. Keep in sync with the TS union.
const VERDICT_APPROVE: &str = "APPROVE";
const VERDICT_NEEDS_REVIEW: &str = "NEEDS_REVIEW";
const VERDICT_BLOCK: &str = "BLOCK";

// ─── Public IO ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxOptions {
    #[serde(default = "default_true")]
    pub run_dev_server: bool,
    #[serde(default = "default_true")]
    pub drive_browser: bool,
    #[serde(default = "default_true")]
    pub run_tests: bool,
    #[serde(default)]
    pub browser_goal: Option<String>,
    #[serde(default)]
    pub start_path: Option<String>, // e.g. "/login"
    #[serde(default)]
    pub max_steps: Option<u32>,
    #[serde(default = "default_provider")]
    pub provider: String, // "claude" | "codex"
    #[serde(default)]
    pub test_cmd: Option<String>, // override auto-discovery
}
fn default_true() -> bool {
    true
}
fn default_provider() -> String {
    "claude".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRunInput {
    pub repo_path: String,
    pub branch: String,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub review_id: Option<String>,
    #[serde(default)]
    pub options: SandboxOptions,
}

impl Default for SandboxOptions {
    fn default() -> Self {
        Self {
            run_dev_server: true,
            drive_browser: true,
            run_tests: true,
            browser_goal: None,
            start_path: None,
            max_steps: None,
            provider: default_provider(),
            test_cmd: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunResult {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionFinding {
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub suggestion: Option<String>,
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub evidence: Option<String>, // step index / log line that triggered it
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRunResult {
    pub run_id: String,
    pub repo_path: String,
    pub branch: String,
    pub worktree_path: Option<String>,
    pub server_url: Option<String>,
    pub agent_steps: Vec<AgentStep>,
    pub test_result: Option<TestRunResult>,
    pub verdict: String, // APPROVE / NEEDS_REVIEW / BLOCK
    pub confidence: f64, // 0.0 – 1.0
    pub summary: String, // 1-2 sentences for the verdict panel
    pub findings: Vec<ExecutionFinding>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

// Step event for the UI: lightweight phase markers + per-agent steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxStep {
    Phase {
        phase: String,
        detail: Option<String>,
    },
    Agent {
        step: AgentStep,
    },
    TestLog {
        line: String,
    },
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_branch_sandbox(
    app: AppHandle,
    db: State<'_, DbState>,
    input: SandboxRunInput,
) -> Result<SandboxRunResult, String> {
    run_branch_sandbox_inner(app, &*db, input).await
}

/// Sandbox runner without the Tauri-State wrapper, so background tasks
/// (T-Rex watcher) can invoke it with a DbState they own.
pub async fn run_branch_sandbox_inner(
    app: AppHandle,
    db: &DbState,
    input: SandboxRunInput,
) -> Result<SandboxRunResult, String> {
    let started = Instant::now();
    let run_id = uuid::Uuid::new_v4().to_string();

    let emit = |s: SandboxStep| {
        let _ = app.emit(STEP_EVENT, s);
    };

    emit(SandboxStep::Phase {
        phase: "setup".into(),
        detail: Some(format!("branch={}", input.branch)),
    });

    // 1. Worktree
    let (worktree_path, _worktree_branch) =
        match create_sandbox_worktree(&input.repo_path, &input.branch) {
            Ok(pair) => pair,
            Err(e) => {
                return Ok(failed_result(
                    run_id,
                    &input,
                    started.elapsed(),
                    None,
                    None,
                    vec![],
                    None,
                    &format!("Worktree setup failed: {e}"),
                ));
            }
        };

    // 2. Optionally install deps if absent.
    if has_package_json(&worktree_path).await && !has_node_modules(&worktree_path).await {
        emit(SandboxStep::Phase {
            phase: "install".into(),
            detail: Some("npm install".into()),
        });
        if let Err(e) = run_npm_install(&worktree_path).await {
            let _ = remove_worktree(&input.repo_path, &worktree_path);
            return Ok(failed_result(
                run_id,
                &input,
                started.elapsed(),
                Some(worktree_path.display().to_string()),
                None,
                vec![],
                None,
                &format!("npm install failed: {e}"),
            ));
        }
    }

    // 3. Dev server + 4. browser drive — coupled because the agent needs the URL.
    // `agent_steps` is only mutated by the browser-agent phase below.
    #[cfg_attr(not(feature = "browser-agent"), allow(unused_mut))]
    let mut agent_steps: Vec<AgentStep> = Vec::new();
    let mut server_url: Option<String> = None;
    let mut server_handle: Option<LocalServer> = None;

    if input.options.run_dev_server {
        let target_url = format!(
            "http://localhost:1420{}",
            input.options.start_path.as_deref().unwrap_or("")
        );
        emit(SandboxStep::Phase {
            phase: "dev_server".into(),
            detail: Some(format!("waiting for {target_url}")),
        });
        match LocalServer::start(&worktree_path, &target_url, Duration::from_secs(90)).await {
            Ok(h) => {
                server_handle = Some(h);
                server_url = Some(target_url.clone());
            }
            Err(e) => {
                emit(SandboxStep::Phase {
                    phase: "dev_server".into(),
                    detail: Some(format!("skipped: {e}")),
                });
            }
        }
    }

    #[cfg(feature = "browser-agent")]
    if let (true, Some(url)) = (input.options.drive_browser, server_url.clone()) {
        emit(SandboxStep::Phase {
            phase: "browser".into(),
            detail: Some(format!("driving {url}")),
        });
        let goal = input
            .options
            .browser_goal
            .clone()
            .unwrap_or_else(default_browser_goal);
        let agent_input = AgentRunInput {
            url: url.clone(),
            goal,
            persona: None,
            provider: input.options.provider.clone(),
            model: None,
            max_steps: input.options.max_steps.or(Some(12)),
            project_dir: None, // server is already up; don't re-launch
        };
        let brain = CliBrain::new(input.options.provider.clone(), None);
        let app_clone = app.clone();
        let result = run_with_brain(agent_input, brain, move |step| {
            let _ = app_clone.emit(STEP_EVENT, SandboxStep::Agent { step: step.clone() });
        })
        .await;
        match result {
            Ok(r) => agent_steps = r.steps,
            Err(e) => {
                emit(SandboxStep::Phase {
                    phase: "browser".into(),
                    detail: Some(format!("agent error: {e}")),
                });
            }
        }
    }
    // When the browser-agent feature is disabled the browser-driving phase is
    // skipped entirely; the sandbox still runs the dev server + tests + verdict.
    #[cfg(not(feature = "browser-agent"))]
    let _ = &server_url;

    // 5. Tests
    let mut test_result: Option<TestRunResult> = None;
    if input.options.run_tests {
        emit(SandboxStep::Phase {
            phase: "tests".into(),
            detail: input.options.test_cmd.clone(),
        });
        test_result =
            Some(run_project_tests(&worktree_path, input.options.test_cmd.as_deref()).await);
    }

    // 6. Synthesize verdict
    emit(SandboxStep::Phase {
        phase: "synthesize".into(),
        detail: Some("asking model for verdict".into()),
    });
    let synth = synthesize_verdict(
        &input.options.provider,
        &input.branch,
        &agent_steps,
        test_result.as_ref(),
    )
    .await
    .unwrap_or_else(|e| Synthesis {
        verdict: VERDICT_NEEDS_REVIEW.into(),
        confidence: 0.0,
        summary: format!("Sandbox completed but synthesis failed: {e}. Review findings manually."),
        findings: vec![],
    });

    // 7. Persist (if review_id supplied)
    if let Some(rid) = &input.review_id {
        if let Ok(conn) = db.0.lock() {
            let _ = queries::update_sandbox_verdict(
                &conn,
                rid,
                &synth.verdict,
                synth.confidence,
                &synth.summary,
            );
            for f in &synth.findings {
                let _ = queries::insert_review_finding(
                    &conn,
                    &queries::LocalReviewFindingInput {
                        review_id: rid.clone(),
                        severity: f.severity.clone(),
                        title: f.title.clone(),
                        summary: f.summary.clone(),
                        suggestion: f.suggestion.clone(),
                        file_path: f.file_path.clone(),
                        line: f.line,
                        confidence: Some(synth.confidence),
                        fingerprint: None,
                        discovery_method: Some("execution".into()),
                    },
                );
            }
        }
    }

    // 8. Cleanup (drop server, remove worktree).
    drop(server_handle);
    let _ = remove_worktree(&input.repo_path, &worktree_path);

    emit(SandboxStep::Phase {
        phase: "done".into(),
        detail: Some(synth.verdict.clone()),
    });

    Ok(SandboxRunResult {
        run_id,
        repo_path: input.repo_path,
        branch: input.branch,
        worktree_path: Some(worktree_path.display().to_string()),
        server_url,
        agent_steps,
        test_result,
        verdict: synth.verdict,
        confidence: synth.confidence,
        summary: synth.summary,
        findings: synth.findings,
        duration_ms: started.elapsed().as_millis() as u64,
        error: None,
    })
}

#[tauri::command]
pub async fn detect_test_command(repo_path: String) -> Result<Option<String>, String> {
    let path = PathBuf::from(repo_path);
    Ok(discover_test_command(&path).await)
}

// ─── Helpers: worktree ──────────────────────────────────────────────────────

fn create_sandbox_worktree(repo_path: &str, branch: &str) -> Result<(PathBuf, String), String> {
    let worktree_name = format!("sandbox-{}", uuid::Uuid::new_v4().simple());
    let worktree_dir = PathBuf::from(format!("{repo_path}/.codevetter-worktrees/{worktree_name}"));

    // Ensure parent + git exclude.
    if let Some(parent) = worktree_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create worktree parent: {e}"))?;
    }
    add_codevetter_worktrees_to_exclude(repo_path);

    // `--detach` checks out the branch's tip without claiming the branch
    // (so the user can keep working on it elsewhere). Same convention as
    // review.rs but without creating a sibling branch.
    let out = std::process::Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&worktree_dir)
        .arg(branch)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git worktree add failed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok((worktree_dir, worktree_name))
}

fn remove_worktree(repo_path: &str, worktree_dir: &Path) -> Result<(), String> {
    let out = std::process::Command::new("git")
        .args(["worktree", "remove", "--force"])
        .arg(worktree_dir)
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git worktree remove failed: {e}"))?;
    if !out.status.success() {
        // Best-effort: prune + force-delete the directory.
        let _ = std::process::Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_path)
            .output();
        let _ = std::fs::remove_dir_all(worktree_dir);
    }
    Ok(())
}

fn add_codevetter_worktrees_to_exclude(repo_path: &str) {
    let exclude_path = format!("{repo_path}/.git/info/exclude");
    let entry = ".codevetter-worktrees";
    if let Ok(contents) = std::fs::read_to_string(&exclude_path) {
        if !contents.lines().any(|l| l.trim() == entry) {
            let mut new_contents = contents;
            if !new_contents.ends_with('\n') {
                new_contents.push('\n');
            }
            new_contents.push_str(entry);
            new_contents.push('\n');
            let _ = std::fs::write(&exclude_path, new_contents);
        }
    } else {
        let _ = std::fs::create_dir_all(format!("{repo_path}/.git/info"));
        let _ = std::fs::write(&exclude_path, format!("{entry}\n"));
    }
}

// ─── Helpers: install ───────────────────────────────────────────────────────

async fn has_package_json(dir: &Path) -> bool {
    tokio::fs::metadata(dir.join("package.json")).await.is_ok()
}

async fn has_node_modules(dir: &Path) -> bool {
    tokio::fs::metadata(dir.join("node_modules")).await.is_ok()
}

async fn run_npm_install(dir: &Path) -> Result<(), String> {
    let out = Command::new("npm")
        .args(["install", "--no-audit", "--no-fund", "--prefer-offline"])
        .current_dir(dir)
        .output()
        .await
        .map_err(|e| format!("spawn npm install: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(())
}

// ─── Helpers: test runner ───────────────────────────────────────────────────

const TEST_TIMEOUT_SECS: u64 = 600;
const LOG_TAIL_BYTES: usize = 8 * 1024;

pub(crate) async fn discover_test_command(dir: &Path) -> Option<String> {
    // 1) package.json scripts.test wins.
    if let Ok(contents) = tokio::fs::read_to_string(dir.join("package.json")).await {
        if let Ok(v) = serde_json::from_str::<Value>(&contents) {
            if let Some(script) = v
                .get("scripts")
                .and_then(|s| s.get("test"))
                .and_then(|t| t.as_str())
            {
                if !script.trim().is_empty()
                    && !script.contains("Error: no test specified")
                    && !script.contains("echo \"Error: no test")
                {
                    return Some("npm test --silent".to_string());
                }
            }
        }
    }
    // 2) Rust.
    if tokio::fs::metadata(dir.join("Cargo.toml")).await.is_ok() {
        return Some("cargo test --quiet".to_string());
    }
    // 3) Python.
    for f in ["pytest.ini", "pyproject.toml", "setup.cfg"] {
        if tokio::fs::metadata(dir.join(f)).await.is_ok() {
            return Some("pytest -q".to_string());
        }
    }
    None
}

async fn run_project_tests(dir: &Path, override_cmd: Option<&str>) -> TestRunResult {
    let started = Instant::now();
    let cmd_str = match override_cmd {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        _ => match discover_test_command(dir).await {
            Some(c) => c,
            None => {
                return TestRunResult {
                    command: String::new(),
                    exit_code: None,
                    stdout_tail: String::new(),
                    stderr_tail: String::new(),
                    duration_ms: 0,
                    timed_out: false,
                    skipped_reason: Some(
                        "no test command discovered (no package.json `test`, Cargo.toml, or pytest config)"
                            .into(),
                    ),
                };
            }
        },
    };

    let mut child = match Command::new("sh")
        .arg("-c")
        .arg(format!("exec {cmd_str}"))
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return TestRunResult {
                command: cmd_str,
                exit_code: None,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                duration_ms: started.elapsed().as_millis() as u64,
                timed_out: false,
                skipped_reason: Some(format!("spawn failed: {e}")),
            };
        }
    };

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();
    let wait_fut = child.wait();

    let outcome = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), wait_fut).await;

    let stdout_tail = match stdout_handle {
        Some(mut h) => read_tail(&mut h, LOG_TAIL_BYTES).await,
        None => String::new(),
    };
    let stderr_tail = match stderr_handle {
        Some(mut h) => read_tail(&mut h, LOG_TAIL_BYTES).await,
        None => String::new(),
    };

    match outcome {
        Ok(Ok(status)) => TestRunResult {
            command: cmd_str,
            exit_code: status.code(),
            stdout_tail,
            stderr_tail,
            duration_ms: started.elapsed().as_millis() as u64,
            timed_out: false,
            skipped_reason: None,
        },
        Ok(Err(e)) => TestRunResult {
            command: cmd_str,
            exit_code: None,
            stdout_tail,
            stderr_tail,
            duration_ms: started.elapsed().as_millis() as u64,
            timed_out: false,
            skipped_reason: Some(format!("wait error: {e}")),
        },
        Err(_) => TestRunResult {
            command: cmd_str,
            exit_code: None,
            stdout_tail,
            stderr_tail,
            duration_ms: started.elapsed().as_millis() as u64,
            timed_out: true,
            skipped_reason: Some(format!("timed out after {TEST_TIMEOUT_SECS}s")),
        },
    }
}

async fn read_tail<R: AsyncReadExt + Unpin>(reader: &mut R, max_bytes: usize) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let _ = reader.read_to_end(&mut buf).await;
    let start = buf.len().saturating_sub(max_bytes);
    String::from_utf8_lossy(&buf[start..]).to_string()
}

// ─── Helpers: synthesis ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Synthesis {
    verdict: String,
    confidence: f64,
    summary: String,
    findings: Vec<ExecutionFinding>,
}

#[cfg(feature = "browser-agent")]
fn default_browser_goal() -> String {
    "You are reviewing a candidate branch. Open the home route. From there, \
     visit every link on the page in order, fill any visible forms with \
     plausible values, and submit them. After each interaction, briefly check \
     the page for visible errors or stuck states. Return `done` when you've \
     exercised at least 3 distinct routes or hit a hard failure. Be quick — \
     this is verification, not exploration."
        .to_string()
}

async fn synthesize_verdict(
    provider: &str,
    branch: &str,
    agent_steps: &[AgentStep],
    test_result: Option<&TestRunResult>,
) -> Result<Synthesis, String> {
    let prompt = build_synthesis_prompt(branch, agent_steps, test_result);
    let raw = match provider {
        "codex" => spawn_oneshot("codex", &["exec", "--json"], &prompt).await?,
        _ => spawn_oneshot("claude", &["-p", "--output-format", "text"], &prompt).await?,
    };
    parse_synthesis(&raw)
}

fn build_synthesis_prompt(
    branch: &str,
    agent_steps: &[AgentStep],
    test_result: Option<&TestRunResult>,
) -> String {
    let mut steps_blob = String::new();
    for s in agent_steps.iter().take(20) {
        let act = match &s.action {
            crate::agent::types::AgentAction::Click { selector, .. } => format!("click {selector}"),
            crate::agent::types::AgentAction::Type { selector, text, .. } => {
                format!("type into {selector}: {text:?}")
            }
            crate::agent::types::AgentAction::Key { key, .. } => format!("press {key}"),
            crate::agent::types::AgentAction::Scroll { delta, .. } => format!("scroll {delta}"),
            crate::agent::types::AgentAction::Goto { url, .. } => format!("goto {url}"),
            crate::agent::types::AgentAction::Done { .. } => "done".into(),
            crate::agent::types::AgentAction::GiveUp { reasoning } => {
                format!("give_up: {reasoning}")
            }
        };
        steps_blob.push_str(&format!(
            "  {}. {act} @ {} \"{}\"{}\n",
            s.index,
            s.url,
            s.page_title,
            s.error
                .as_deref()
                .map(|e| format!(" [error: {e}]"))
                .unwrap_or_default()
        ));
    }
    if steps_blob.is_empty() {
        steps_blob.push_str("  (no browser steps recorded)\n");
    }

    let test_blob = match test_result {
        None => "  (tests not run)".to_string(),
        Some(t) if t.skipped_reason.is_some() => {
            format!("  skipped: {}", t.skipped_reason.as_deref().unwrap_or(""))
        }
        Some(t) => {
            let pass = t.exit_code == Some(0);
            format!(
                "  command: {}\n  pass: {}\n  exit_code: {:?}\n  stdout_tail (last 8KB):\n{}\n  stderr_tail:\n{}",
                t.command, pass, t.exit_code, t.stdout_tail, t.stderr_tail
            )
        }
    };

    format!(
        r#"You are T-Rex, an automated PR reviewer. You just sandbox-ran a candidate branch.

Branch: {branch}

Browser exercise (first 20 steps):
{steps_blob}

Test run:
{test_blob}

Decide one of three verdicts based ONLY on what the execution evidence showed:
  - APPROVE: tests pass AND no breakage observed in browser exercise. High confidence the branch works.
  - NEEDS_REVIEW: ambiguous or partial evidence (tests skipped, agent gave up early, mild console errors). Human should look.
  - BLOCK: tests failed OR the agent observed a clearly broken page (500, blank screen, stuck spinner, fatal error).

Output EXACTLY one JSON object on one line, no prose, no markdown fences. Schema:

{{
  "verdict": "APPROVE" | "NEEDS_REVIEW" | "BLOCK",
  "confidence": 0.0–1.0,
  "summary": "<= 200 chars. One sentence. Cite the concrete signal (e.g. \"tests passed and 3 routes rendered cleanly\" or \"test suite exited 1 with type error in src/foo.ts\").",
  "findings": [
    {{
      "severity": "high|medium|low",
      "title": "short",
      "summary": "what happened, citing a step or log line",
      "suggestion": "optional",
      "file_path": "optional",
      "line": optional integer,
      "evidence": "step index or log line snippet"
    }}
  ]
}}

Findings should ONLY be things execution revealed that pure inspection could not have. Empty array is correct if execution went clean.
"#
    )
}

async fn spawn_oneshot(cmd: &str, args: &[&str], prompt: &str) -> Result<String, String> {
    use tokio::io::AsyncWriteExt;
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {cmd}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| format!("stdin write: {e}"))?;
        let _ = stdin.shutdown().await;
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| format!("wait {cmd}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd} exit {:?}", out.status.code()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn parse_synthesis(raw: &str) -> Result<Synthesis, String> {
    // Strip stream-json frames if any, find the first JSON object that matches
    // our schema.
    for chunk in scan_json_objects(raw) {
        if let Ok(v) = serde_json::from_str::<Value>(&chunk) {
            if let Some(verdict) = v.get("verdict").and_then(|x| x.as_str()) {
                let verdict = canon_verdict(verdict);
                let confidence = v
                    .get("confidence")
                    .and_then(|x| x.as_f64())
                    .unwrap_or(0.5)
                    .clamp(0.0, 1.0);
                let summary = v
                    .get("summary")
                    .and_then(|x| x.as_str())
                    .unwrap_or("No summary returned.")
                    .to_string();
                let findings = v
                    .get("findings")
                    .and_then(|x| x.as_array())
                    .map(|arr| arr.iter().filter_map(parse_finding).collect())
                    .unwrap_or_default();
                return Ok(Synthesis {
                    verdict,
                    confidence,
                    summary,
                    findings,
                });
            }
        }
    }
    Err("no valid synthesis JSON in model output".into())
}

fn parse_finding(v: &Value) -> Option<ExecutionFinding> {
    let title = v.get("title").and_then(|s| s.as_str())?.to_string();
    Some(ExecutionFinding {
        severity: v
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("medium")
            .to_string(),
        title,
        summary: v
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        suggestion: v
            .get("suggestion")
            .and_then(|s| s.as_str())
            .map(String::from),
        file_path: v
            .get("file_path")
            .and_then(|s| s.as_str())
            .map(String::from),
        line: v.get("line").and_then(|s| s.as_i64()),
        evidence: v.get("evidence").and_then(|s| s.as_str()).map(String::from),
    })
}

fn canon_verdict(s: &str) -> String {
    let up = s.to_ascii_uppercase();
    if up.contains("APPROVE") {
        VERDICT_APPROVE.into()
    } else if up.contains("BLOCK") {
        VERDICT_BLOCK.into()
    } else {
        VERDICT_NEEDS_REVIEW.into()
    }
}

/// Scan a blob for top-level balanced JSON objects (handles strings + escapes).
fn scan_json_objects(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_string = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if esc {
                esc = false;
                continue;
            }
            match b {
                b'\\' => esc = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = i;
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 && i + 1 >= start {
                    out.push(String::from_utf8_lossy(&bytes[start..=i]).to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn failed_result(
    run_id: String,
    input: &SandboxRunInput,
    elapsed: Duration,
    worktree_path: Option<String>,
    server_url: Option<String>,
    agent_steps: Vec<AgentStep>,
    test_result: Option<TestRunResult>,
    err: &str,
) -> SandboxRunResult {
    SandboxRunResult {
        run_id,
        repo_path: input.repo_path.clone(),
        branch: input.branch.clone(),
        worktree_path,
        server_url,
        agent_steps,
        test_result,
        verdict: VERDICT_NEEDS_REVIEW.into(),
        confidence: 0.0,
        summary: format!("Sandbox didn't complete: {err}. Treating as NEEDS_REVIEW."),
        findings: vec![],
        duration_ms: elapsed.as_millis() as u64,
        error: Some(err.into()),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovers_npm_test_from_scripts() {
        let dir = tempdir();
        tokio::fs::write(
            dir.join("package.json"),
            r#"{ "name":"x", "scripts": { "test": "jest" } }"#,
        )
        .await
        .unwrap();
        assert_eq!(
            discover_test_command(&dir).await.as_deref(),
            Some("npm test --silent")
        );
        cleanup(&dir);
    }

    #[tokio::test]
    async fn skips_default_npm_init_test_placeholder() {
        let dir = tempdir();
        tokio::fs::write(
            dir.join("package.json"),
            r#"{ "scripts": { "test": "echo \"Error: no test specified\" && exit 1" } }"#,
        )
        .await
        .unwrap();
        assert_eq!(discover_test_command(&dir).await, None);
        cleanup(&dir);
    }

    #[tokio::test]
    async fn discovers_cargo_test() {
        let dir = tempdir();
        tokio::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n")
            .await
            .unwrap();
        assert_eq!(
            discover_test_command(&dir).await.as_deref(),
            Some("cargo test --quiet")
        );
        cleanup(&dir);
    }

    #[tokio::test]
    async fn discovers_pytest() {
        let dir = tempdir();
        tokio::fs::write(dir.join("pytest.ini"), "[pytest]\n")
            .await
            .unwrap();
        assert_eq!(
            discover_test_command(&dir).await.as_deref(),
            Some("pytest -q")
        );
        cleanup(&dir);
    }

    #[test]
    fn parses_clean_synthesis_json() {
        let raw = r#"{"verdict":"APPROVE","confidence":0.92,"summary":"tests passed and 3 routes rendered cleanly","findings":[]}"#;
        let s = parse_synthesis(raw).unwrap();
        assert_eq!(s.verdict, VERDICT_APPROVE);
        assert!((s.confidence - 0.92).abs() < 1e-6);
        assert_eq!(s.findings.len(), 0);
    }

    #[test]
    fn parses_synthesis_with_findings_and_prose_around() {
        let raw = r#"
Sure, here you go:

{"verdict":"BLOCK","confidence":0.8,"summary":"jest exit 1 with type errors","findings":[
  {"severity":"high","title":"type error in foo.ts","summary":"jest exit 1","file_path":"src/foo.ts","line":42}
]}

Hope that helps!
        "#;
        let s = parse_synthesis(raw).unwrap();
        assert_eq!(s.verdict, VERDICT_BLOCK);
        assert_eq!(s.findings.len(), 1);
        assert_eq!(s.findings[0].file_path.as_deref(), Some("src/foo.ts"));
    }

    #[test]
    fn canon_verdict_normalizes_garbage() {
        assert_eq!(canon_verdict("approve"), VERDICT_APPROVE);
        assert_eq!(canon_verdict("Block"), VERDICT_BLOCK);
        assert_eq!(canon_verdict("idk"), VERDICT_NEEDS_REVIEW);
        assert_eq!(canon_verdict("NEEDS_REVIEW"), VERDICT_NEEDS_REVIEW);
    }

    #[tokio::test]
    async fn test_runner_returns_skipped_when_no_command() {
        let dir = tempdir();
        let r = run_project_tests(&dir, None).await;
        assert!(r.skipped_reason.is_some());
        assert!(r.exit_code.is_none());
        cleanup(&dir);
    }

    #[tokio::test]
    async fn test_runner_captures_exit_code_and_stdout() {
        let dir = tempdir();
        let r = run_project_tests(&dir, Some("printf hello; exit 0")).await;
        assert_eq!(r.exit_code, Some(0));
        assert!(r.stdout_tail.contains("hello"));
        assert!(!r.timed_out);
        cleanup(&dir);
    }

    // ─── tiny tempdir helper ──────────────────────────────────────────────

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("cv-sandbox-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn cleanup(p: &Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    /// Real-git e2e (gated). Spins up a worktree, runs a passing test cmd,
    /// asserts cleanup.
    #[test]
    #[ignore]
    fn e2e_worktree_and_test_runner() {
        use std::process::Command as SC;
        let repo = std::env::temp_dir().join(format!("cv-sandbox-repo-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&repo).unwrap();
        let run = |args: &[&str]| {
            let s = SC::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap();
            assert!(s.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "a@a"]);
        run(&["config", "user.name", "A"]);
        std::fs::write(repo.join("a.txt"), "hello\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        run(&["branch", "feature"]);

        let (wt, _name) = create_sandbox_worktree(repo.to_str().unwrap(), "feature").unwrap();
        assert!(wt.exists());

        // tokio runtime for the async runner.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let r = rt.block_on(run_project_tests(&wt, Some("printf ok; exit 0")));
        assert_eq!(r.exit_code, Some(0));

        remove_worktree(repo.to_str().unwrap(), &wt).unwrap();
        assert!(!wt.exists());
        let _ = std::fs::remove_dir_all(&repo);
    }
}
