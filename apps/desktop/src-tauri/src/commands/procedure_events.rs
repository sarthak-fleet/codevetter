use crate::{db::queries, DbState};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command as StdCommand;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{Manager, State};

static RUNNING_COMMANDS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
static CANCELED_COMMANDS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn running_commands() -> &'static Mutex<HashMap<String, u32>> {
    RUNNING_COMMANDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn canceled_commands() -> &'static Mutex<HashSet<String>> {
    CANCELED_COMMANDS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn make_run_id(review_id: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("review-command-{review_id}-{millis}")
}

fn mark_command_running(run_id: &str, pid: u32) -> Result<(), String> {
    let mut running = running_commands().lock().map_err(|e| e.to_string())?;
    running.insert(run_id.to_string(), pid);
    Ok(())
}

fn remove_running_command(run_id: &str) {
    if let Ok(mut running) = running_commands().lock() {
        running.remove(run_id);
    }
}

fn mark_command_canceled(run_id: &str) -> Result<(), String> {
    let mut canceled = canceled_commands().lock().map_err(|e| e.to_string())?;
    canceled.insert(run_id.to_string());
    Ok(())
}

fn take_command_canceled(run_id: &str) -> bool {
    canceled_commands()
        .lock()
        .map(|mut canceled| canceled.remove(run_id))
        .unwrap_or(false)
}

fn validate_event_status(status: &str) -> Result<(), String> {
    match status {
        "satisfied" | "blocked" | "observed" => Ok(()),
        _ => Err(format!(
            "unsupported procedure event status: {status}. Supported: satisfied, blocked, observed"
        )),
    }
}

fn parse_command(command: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                } else {
                    current.push(ch);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ch if ch.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }

    if quote.is_some() {
        return Err("unterminated quote in command".into());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        return Err("command is required".into());
    }
    Ok(parts)
}

fn reject_destructive_command(command: &str) -> Result<(), String> {
    let lower = command.to_ascii_lowercase();
    let blocked = [
        " rm -rf ",
        " rm -fr ",
        "git reset --hard",
        "git clean -fd",
        "git clean -xdf",
        "drop database",
        "truncate table",
        "kubectl delete",
        "terraform destroy",
    ];
    let padded = format!(" {lower} ");
    if blocked.iter().any(|needle| padded.contains(needle)) {
        return Err("Refusing to run a destructive-looking verification command.".into());
    }
    if [";", "&&", "||", "|", ">", "<", "`", "$("]
        .iter()
        .any(|needle| command.contains(needle))
    {
        return Err(
            "Shell operators are not supported for verification commands. Enter one command with args."
                .into(),
        );
    }
    Ok(())
}

fn log_path_for(app: &tauri::AppHandle, review_id: &str) -> Result<PathBuf, String> {
    let safe_review_id = review_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("review-command-events")
        .join(safe_review_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create command log dir: {e}"))?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Ok(dir.join(format!("{millis}.log")))
}

#[derive(Clone, Debug)]
struct ScoredCommandSuggestion {
    command: String,
    reason: String,
    source: String,
    score: i64,
}

fn add_scored_command(
    commands: &mut Vec<ScoredCommandSuggestion>,
    command: &str,
    reason: &str,
    source: &str,
    score: i64,
) {
    if let Some(existing) = commands.iter_mut().find(|item| item.command == command) {
        existing.score += score;
        if !existing.reason.contains(reason) {
            existing.reason = format!("{}; {}", existing.reason, reason);
        }
        if !existing.source.contains(source) {
            existing.source = format!("{}+{}", existing.source, source);
        }
        return;
    }
    commands.push(ScoredCommandSuggestion {
        command: command.to_string(),
        reason: reason.to_string(),
        source: source.to_string(),
        score,
    });
}

fn package_manager_for(repo: &std::path::Path) -> &'static str {
    if repo.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if repo.join("yarn.lock").is_file() {
        "yarn"
    } else if repo.join("bun.lockb").is_file() || repo.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
}

fn package_script_command(
    package_json: &serde_json::Value,
    package_manager: &str,
    script: &str,
) -> Option<String> {
    package_json
        .get("scripts")
        .and_then(|scripts| scripts.get(script))
        .and_then(|value| value.as_str())
        .map(|_| match package_manager {
            "yarn" => format!("yarn {script}"),
            "bun" => format!("bun run {script}"),
            "pnpm" => format!("pnpm {script}"),
            _ => format!("npm run {script}"),
        })
}

fn path_has_extension(path: &str, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn command_file_affinity_score(
    command: &str,
    paths: &[String],
    finding_file_path: Option<&str>,
) -> i64 {
    let lower = command.to_ascii_lowercase();
    let mut score = 0;
    let has_js = paths.iter().any(|path| {
        path_has_extension(path, &[".ts", ".tsx", ".js", ".jsx"]) || path.contains("package")
    });
    let has_rust = paths
        .iter()
        .any(|path| path_has_extension(path, &[".rs"]) || path == "Cargo.toml");
    let has_python = paths.iter().any(|path| path_has_extension(path, &[".py"]));

    if has_js
        && ["npm ", "pnpm ", "yarn ", "bun "]
            .iter()
            .any(|needle| lower.contains(needle))
    {
        score += 30;
    }
    if has_rust && lower.contains("cargo ") {
        score += 30;
    }
    if has_python && (lower.contains("pytest") || lower.contains("python ")) {
        score += 30;
    }
    if lower.contains("test") {
        score += 10;
    }
    if lower == "git diff --check" {
        score += 6;
    }

    if let Some(finding_file_path) = finding_file_path {
        let finding_paths = [finding_file_path.to_string()];
        score += command_file_affinity_score(command, &finding_paths, None) / 2;
    }

    score
}

fn history_recency_score(date: Option<&str>, fallback_index: usize) -> i64 {
    let fallback = (14_i64 - (fallback_index as i64 * 2)).max(0);
    let Some(date) = date else {
        return fallback;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(date).map(|value| value.with_timezone(&Utc))
    else {
        return fallback;
    };
    let age_days = Utc::now().signed_duration_since(parsed).num_days().max(0);
    match age_days {
        0..=1 => 25,
        2..=7 => 20,
        8..=30 => 12,
        31..=90 => 6,
        _ => 0,
    }
}

fn history_status_score(status: &str) -> i64 {
    match status {
        "passed" => 45,
        "failed" => 36,
        "unknown" => 12,
        _ => 0,
    }
}

#[tauri::command]
pub async fn record_review_procedure_event(
    db: State<'_, DbState>,
    review_id: String,
    step_id: String,
    status: String,
    source: String,
    summary: String,
    artifact: Option<String>,
    metadata: Option<Value>,
) -> Result<Value, String> {
    let review_id = review_id.trim().to_string();
    let step_id = step_id.trim().to_string();
    let status = status.trim().to_string();
    let source = source.trim().to_string();
    let summary = summary.trim().to_string();

    if review_id.is_empty() {
        return Err("review_id is required".into());
    }
    if step_id.is_empty() {
        return Err("step_id is required".into());
    }
    if source.is_empty() {
        return Err("source is required".into());
    }
    if summary.is_empty() {
        return Err("summary is required".into());
    }
    validate_event_status(&status)?;

    let input = queries::ReviewProcedureEventInput {
        review_id,
        step_id,
        status,
        source,
        summary,
        artifact: artifact
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        metadata: metadata.map(|value| value.to_string()),
    };

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let event = queries::insert_review_procedure_event(&conn, &input).map_err(|e| e.to_string())?;
    Ok(json!(event))
}

#[tauri::command]
pub async fn list_review_procedure_events(
    db: State<'_, DbState>,
    review_id: String,
) -> Result<Value, String> {
    let review_id = review_id.trim().to_string();
    if review_id.is_empty() {
        return Err("review_id is required".into());
    }

    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let events =
        queries::list_review_procedure_events(&conn, &review_id).map_err(|e| e.to_string())?;
    Ok(json!({ "events": events }))
}

#[tauri::command]
pub async fn suggest_review_verification_commands(
    repo_path: String,
    changed_files: Option<Vec<String>>,
    finding_file_path: Option<String>,
    history_commands: Option<Vec<Value>>,
) -> Result<Value, String> {
    let repo_path = repo_path.trim().to_string();
    if repo_path.is_empty() {
        return Err("repo_path is required".into());
    }
    let repo = PathBuf::from(&repo_path);
    if !repo.is_dir() {
        return Err(format!("repo_path must be a directory: {repo_path}"));
    }

    let mut paths = changed_files.unwrap_or_default();
    if let Some(path) = finding_file_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        paths.push(path.to_string());
    }
    let mut commands: Vec<ScoredCommandSuggestion> = Vec::new();
    let package_manager = package_manager_for(&repo);

    for (index, signal) in history_commands.unwrap_or_default().into_iter().enumerate() {
        let Some(command) = signal
            .get("command")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if reject_destructive_command(command).is_err() || parse_command(command).is_err() {
            continue;
        }
        let status = signal
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        if status == "stale" {
            continue;
        }
        let source = signal
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or("history");
        let artifact_score = signal
            .get("artifacts")
            .and_then(|value| value.as_array())
            .filter(|artifacts| !artifacts.is_empty())
            .map(|_| 5)
            .unwrap_or(0);
        let score = history_status_score(status)
            + history_recency_score(signal.get("date").and_then(|value| value.as_str()), index)
            + command_file_affinity_score(command, &paths, finding_file_path.as_deref())
            + artifact_score
            + if source == "output_structured" { 8 } else { 4 };
        let reason = match status {
            "passed" => "previously passed in recent repo history",
            "failed" => "previously failed in repo history; useful regression check",
            _ => "seen in repo history for related work",
        };
        add_scored_command(&mut commands, command, reason, source, score);
    }

    let package_json_path = repo.join("package.json");
    if package_json_path.is_file() {
        if let Ok(text) = std::fs::read_to_string(&package_json_path) {
            if let Ok(package_json) = serde_json::from_str::<serde_json::Value>(&text) {
                if paths.iter().any(|path| {
                    path_has_extension(path, &[".ts", ".tsx", ".js", ".jsx"])
                        || path.contains("package")
                }) {
                    if let Some(command) =
                        package_script_command(&package_json, package_manager, "test")
                    {
                        add_scored_command(
                            &mut commands,
                            &command,
                            &format!(
                                "package.json has a test script for JS/TS changes via {package_manager}"
                            ),
                            "package.json",
                            42 + command_file_affinity_score(
                                &command,
                                &paths,
                                finding_file_path.as_deref(),
                            ),
                        );
                    }
                    if let Some(command) =
                        package_script_command(&package_json, package_manager, "lint")
                    {
                        add_scored_command(
                            &mut commands,
                            &command,
                            &format!(
                                "package.json has a lint script for JS/TS changes via {package_manager}"
                            ),
                            "package.json",
                            32 + command_file_affinity_score(
                                &command,
                                &paths,
                                finding_file_path.as_deref(),
                            ),
                        );
                    }
                    if let Some(command) =
                        package_script_command(&package_json, package_manager, "build")
                    {
                        add_scored_command(
                            &mut commands,
                            &command,
                            &format!(
                                "package.json has a build script for compile coverage via {package_manager}"
                            ),
                            "package.json",
                            24 + command_file_affinity_score(
                                &command,
                                &paths,
                                finding_file_path.as_deref(),
                            ),
                        );
                    }
                }
            }
        }
    }

    if repo.join("Cargo.toml").is_file()
        || paths
            .iter()
            .any(|path| path_has_extension(path, &[".rs"]) || path == "Cargo.toml")
    {
        add_scored_command(
            &mut commands,
            "cargo test",
            "Rust project or Rust file changed",
            "repo-files",
            42 + command_file_affinity_score("cargo test", &paths, finding_file_path.as_deref()),
        );
        add_scored_command(
            &mut commands,
            "cargo check",
            "Rust compile check for changed code",
            "repo-files",
            30 + command_file_affinity_score("cargo check", &paths, finding_file_path.as_deref()),
        );
    }

    if repo.join("pyproject.toml").is_file()
        || repo.join("pytest.ini").is_file()
        || paths.iter().any(|path| path_has_extension(path, &[".py"]))
    {
        add_scored_command(
            &mut commands,
            "python -m pytest",
            "Python project or Python file changed",
            "repo-files",
            42 + command_file_affinity_score(
                "python -m pytest",
                &paths,
                finding_file_path.as_deref(),
            ),
        );
    }

    if commands.is_empty() {
        add_scored_command(
            &mut commands,
            "git diff --check",
            "generic whitespace and conflict-marker check",
            "fallback",
            6,
        );
    }

    commands.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.command.cmp(&b.command))
    });

    Ok(json!({
        "commands": commands
            .into_iter()
            .take(6)
            .map(|item| {
                json!({
                    "command": item.command,
                    "reason": item.reason,
                    "source": item.source,
                    "score": item.score,
                })
            })
            .collect::<Vec<_>>()
    }))
}

#[tauri::command]
pub async fn run_review_verification_command(
    app: tauri::AppHandle,
    db: State<'_, DbState>,
    repo_path: String,
    review_id: String,
    command: String,
    step_id: Option<String>,
    timeout_ms: Option<u64>,
    run_id: Option<String>,
) -> Result<Value, String> {
    let repo_path = repo_path.trim().to_string();
    let review_id = review_id.trim().to_string();
    let command = command.trim().to_string();
    let step_id = step_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("rerun_relevant_verification")
        .to_string();

    if review_id.is_empty() {
        return Err("review_id is required".into());
    }
    let run_id = run_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| make_run_id(&review_id));
    if repo_path.is_empty() {
        return Err("repo_path is required".into());
    }
    let repo = PathBuf::from(&repo_path);
    if !repo.is_dir() {
        return Err(format!("repo_path must be a directory: {repo_path}"));
    }
    reject_destructive_command(&command)?;
    let parts = parse_command(&command)?;
    let executable = parts[0].clone();
    let args = parts[1..].to_vec();
    let timeout_ms = timeout_ms.unwrap_or(120_000).clamp(1_000, 600_000);
    let started = std::time::Instant::now();
    let output = tokio::task::spawn_blocking({
        let repo = repo.clone();
        let executable = executable.clone();
        let args = args.clone();
        let timeout = Duration::from_millis(timeout_ms);
        let run_id = run_id.clone();
        move || {
            let mut child = StdCommand::new(executable)
                .args(args)
                .current_dir(repo)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
            mark_command_running(&run_id, child.id())?;

            loop {
                match child.try_wait().map_err(|e| e.to_string())? {
                    Some(_) => {
                        let output = child.wait_with_output().map_err(|e| e.to_string());
                        remove_running_command(&run_id);
                        return output;
                    }
                    None if started.elapsed() >= timeout => {
                        let _ = child.kill();
                        let output = child.wait_with_output().map_err(|e| e.to_string());
                        remove_running_command(&run_id);
                        return output;
                    }
                    None => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
    })
    .await
    .map_err(|e| format!("command task join error: {e}"))??;
    let duration_ms = started.elapsed().as_millis() as u64;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let canceled = take_command_canceled(&run_id);
    let timed_out = !canceled && duration_ms >= timeout_ms && !output.status.success();
    let passed = output.status.success() && !timed_out && !canceled;
    let status = if passed { "satisfied" } else { "blocked" };
    let artifact_path = log_path_for(&app, &review_id)?;
    let mut file =
        std::fs::File::create(&artifact_path).map_err(|e| format!("create command log: {e}"))?;
    writeln!(file, "$ {command}").map_err(|e| e.to_string())?;
    writeln!(file, "cwd: {repo_path}").map_err(|e| e.to_string())?;
    writeln!(file, "exit_code: {exit_code}").map_err(|e| e.to_string())?;
    writeln!(file, "duration_ms: {duration_ms}").map_err(|e| e.to_string())?;
    writeln!(file, "timeout_ms: {timeout_ms}").map_err(|e| e.to_string())?;
    writeln!(file, "timed_out: {timed_out}").map_err(|e| e.to_string())?;
    writeln!(file, "canceled: {canceled}").map_err(|e| e.to_string())?;
    writeln!(file, "\n--- stdout ---\n{stdout}").map_err(|e| e.to_string())?;
    writeln!(file, "\n--- stderr ---\n{stderr}").map_err(|e| e.to_string())?;

    let artifact = artifact_path.to_string_lossy().to_string();
    let summary = format!(
        "{} `{}` ({}ms)",
        if passed {
            "PASS"
        } else if canceled {
            "CANCELED"
        } else if timed_out {
            "TIMEOUT"
        } else {
            "FAIL"
        },
        command,
        duration_ms
    );
    let input = queries::ReviewProcedureEventInput {
        review_id: review_id.clone(),
        step_id: step_id.clone(),
        status: status.to_string(),
        source: "command".to_string(),
        summary: summary.clone(),
        artifact: Some(artifact.clone()),
        metadata: Some(
            json!({
                "command": command.clone(),
                "repo_path": repo_path.clone(),
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "timeout_ms": timeout_ms,
                "timed_out": timed_out,
                "canceled": canceled,
                "run_id": run_id.clone(),
            })
            .to_string(),
        ),
    };

    let event = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        queries::insert_review_procedure_event(&conn, &input).map_err(|e| e.to_string())?
    };

    Ok(json!({
        "event": event,
        "run_id": run_id,
        "command": command,
        "exit_code": exit_code,
        "duration_ms": duration_ms,
        "timeout_ms": timeout_ms,
        "timed_out": timed_out,
        "canceled": canceled,
        "passed": passed,
        "artifact": artifact,
        "stdout_tail": stdout.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>(),
        "stderr_tail": stderr.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>(),
    }))
}

#[tauri::command]
pub async fn cancel_review_verification_command(run_id: String) -> Result<Value, String> {
    let run_id = run_id.trim().to_string();
    if run_id.is_empty() {
        return Err("run_id is required".into());
    }

    let pid = {
        let running = running_commands().lock().map_err(|e| e.to_string())?;
        running.get(&run_id).copied()
    };
    let Some(pid) = pid else {
        return Ok(json!({ "run_id": run_id, "canceled": false, "reason": "not_running" }));
    };

    #[cfg(target_family = "unix")]
    let status = StdCommand::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .map_err(|e| format!("cancel command: {e}"))?;

    #[cfg(target_family = "windows")]
    let status = StdCommand::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .map_err(|e| format!("cancel command: {e}"))?;

    if !status.success() {
        return Err(format!("cancel command exited with {status}"));
    }
    mark_command_canceled(&run_id)?;
    Ok(json!({ "run_id": run_id, "canceled": true, "pid": pid }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_keeps_quoted_args() {
        let parsed = parse_command("npm run test -- --grep \"review proof\"").expect("parsed");
        assert_eq!(
            parsed,
            vec!["npm", "run", "test", "--", "--grep", "review proof"]
        );
    }

    #[test]
    fn rejects_shell_operators_and_destructive_commands() {
        assert!(reject_destructive_command("npm test && rm -rf .").is_err());
        assert!(reject_destructive_command("git reset --hard HEAD").is_err());
        assert!(reject_destructive_command("npm run test:review-proof").is_ok());
    }

    #[test]
    fn package_script_command_uses_detected_package_manager() {
        let package_json = json!({
            "scripts": {
                "test": "vitest"
            }
        });

        assert_eq!(
            package_script_command(&package_json, "pnpm", "test"),
            Some("pnpm test".to_string())
        );
        assert_eq!(
            package_script_command(&package_json, "npm", "test"),
            Some("npm run test".to_string())
        );
    }

    #[test]
    fn command_file_affinity_prefers_matching_stack() {
        let rust_paths = vec!["src/lib.rs".to_string()];
        let js_paths = vec!["src/App.tsx".to_string()];

        assert!(command_file_affinity_score("cargo test", &rust_paths, Some("src/lib.rs")) > 30);
        assert!(command_file_affinity_score("npm run test", &js_paths, Some("src/App.tsx")) > 30);
        assert_eq!(
            command_file_affinity_score("cargo test", &js_paths, None),
            10
        );
    }
}
