use crate::{db::queries, DbState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tauri::{Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticQaTrace {
    pub final_url: String,
    pub page_title: String,
    pub console_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticQaRunResult {
    pub loop_id: String,
    pub route: String,
    pub goal: String,
    pub pass: bool,
    pub notes: String,
    pub screenshot_path: Option<String>,
    pub artifacts: Vec<String>,
    pub duration_ms: u64,
    pub trace: SyntheticQaTrace,
    pub error: Option<String>,
    pub runner_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaywrightSpecCandidate {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSyntheticQaRunInput {
    pub review_id: Option<String>,
    pub repo_path: Option<String>,
    pub base_url: Option<String>,
    pub run: SyntheticQaRunResult,
}

#[derive(Debug, Default)]
struct RepoPlaywrightSummary {
    expected: usize,
    unexpected: usize,
    flaky: usize,
    skipped: usize,
    failures: Vec<String>,
    artifacts: Vec<String>,
}

fn resolve_runner_script() -> Result<PathBuf, String> {
    // Dev / local repo: CARGO_MANIFEST_DIR = apps/desktop/src-tauri
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_script = manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|desktop| desktop.join("scripts").join("run-synthetic-qa.mjs"));
    if let Some(path) = dev_script {
        if path.exists() {
            return Ok(path);
        }
    }
    Err(
        "Synthetic QA runner script not found. Run from the CodeVetter repo with apps/desktop/scripts/run-synthetic-qa.mjs present.".into(),
    )
}

fn resolve_repo_playwright_binary(repo_path: &str) -> String {
    let mut local = PathBuf::from(repo_path);
    local.push("node_modules");
    local.push(".bin");
    local.push(if cfg!(windows) {
        "playwright.cmd"
    } else {
        "playwright"
    });
    if local.exists() {
        return local.to_string_lossy().into_owned();
    }
    "npx".to_string()
}

fn split_shell_like_command(command: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut token_started = false;

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            token_started = true;
            continue;
        }

        match ch {
            '\\' if quote != Some('\'') => {
                escaped = true;
            }
            '\'' | '"' => {
                if let Some(q) = quote {
                    if q == ch {
                        quote = None;
                    } else {
                        current.push(ch);
                    }
                } else {
                    quote = Some(ch);
                    token_started = true;
                }
            }
            c if c.is_whitespace() && quote.is_none() => {
                if token_started {
                    args.push(std::mem::take(&mut current));
                }
                token_started = false;
                while matches!(chars.peek(), Some(next) if next.is_whitespace()) {
                    chars.next();
                }
            }
            c => {
                current.push(c);
                token_started = true;
            }
        }
    }

    if escaped {
        return Err("external_command ends with an incomplete escape".into());
    }
    if quote.is_some() {
        return Err("external_command has an unterminated quote".into());
    }
    if token_started {
        args.push(current);
    }
    if args.is_empty() {
        return Err("external_command is empty".into());
    }
    Ok(args)
}

fn should_skip_scan_dir(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    matches!(
        name,
        ".git" | "node_modules" | "target" | "dist" | "out" | "build" | ".next" | "coverage"
    )
}

fn looks_like_spec(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    (lower.contains(".spec.") || lower.contains(".test."))
        && matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs")
        )
}

fn relative_path(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}

fn short_line(value: &str, limit: usize) -> String {
    let line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(value)
        .trim();
    let mut out: String = line.chars().take(limit).collect();
    if line.chars().count() > limit {
        out.push_str("...");
    }
    out
}

fn playwright_error_text(result: &Value) -> Option<String> {
    result
        .get("error")
        .and_then(|error| {
            error
                .get("message")
                .or_else(|| error.get("stack"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            result
                .get("errors")
                .and_then(Value::as_array)
                .and_then(|errors| errors.first())
                .and_then(|error| {
                    error
                        .get("message")
                        .or_else(|| error.get("stack"))
                        .and_then(Value::as_str)
                })
        })
        .map(|message| short_line(message, 220))
}

fn collect_playwright_attachments(
    result: &Value,
    repo: Option<&Path>,
    artifacts: &mut Vec<String>,
) {
    if artifacts.len() >= 12 {
        return;
    }
    let Some(attachments) = result.get("attachments").and_then(Value::as_array) else {
        return;
    };
    for attachment in attachments {
        if artifacts.len() >= 12 {
            break;
        }
        let Some(path) = attachment.get("path").and_then(Value::as_str) else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        let normalized = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else if let Some(repo) = repo {
            repo.join(path)
        } else {
            PathBuf::from(path)
        }
        .to_string_lossy()
        .into_owned();
        if !artifacts.contains(&normalized) {
            artifacts.push(normalized);
        }
    }
}

fn collect_playwright_details(
    suite: &Value,
    parents: &mut Vec<String>,
    failures: &mut Vec<String>,
    artifacts: &mut Vec<String>,
    repo: Option<&Path>,
) {
    if let Some(title) = suite.get("title").and_then(Value::as_str) {
        if !title.trim().is_empty() {
            parents.push(title.trim().to_string());
        }
    }

    if let Some(specs) = suite.get("specs").and_then(Value::as_array) {
        for spec in specs {
            if failures.len() >= 8 {
                break;
            }
            let spec_title = spec.get("title").and_then(Value::as_str).unwrap_or("spec");
            let mut title_parts = parents.clone();
            title_parts.push(spec_title.to_string());
            let title = title_parts.join(" > ");
            let Some(tests) = spec.get("tests").and_then(Value::as_array) else {
                continue;
            };
            for test in tests {
                let status = test.get("status").and_then(Value::as_str).unwrap_or("");
                if let Some(results) = test.get("results").and_then(Value::as_array) {
                    for result in results {
                        collect_playwright_attachments(result, repo, artifacts);
                    }
                }
                if status == "unexpected" || status == "flaky" {
                    let project = test
                        .get("projectName")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                        .map(|name| format!(" [{name}]"))
                        .unwrap_or_default();
                    let detail = test
                        .get("results")
                        .and_then(Value::as_array)
                        .and_then(|results| results.iter().find_map(playwright_error_text))
                        .unwrap_or_else(|| format!("status={status}"));
                    if failures.len() < 8 {
                        failures.push(format!("{title}{project}: {detail}"));
                    }
                }
                if failures.len() >= 8 && artifacts.len() >= 12 {
                    break;
                }
            }
        }
    }

    if let Some(children) = suite.get("suites").and_then(Value::as_array) {
        for child in children {
            collect_playwright_details(child, parents, failures, artifacts, repo);
            if failures.len() >= 8 && artifacts.len() >= 12 {
                break;
            }
        }
    }

    if suite
        .get("title")
        .and_then(Value::as_str)
        .is_some_and(|title| !title.trim().is_empty())
    {
        parents.pop();
    }
}

fn parse_repo_playwright_summary(raw: &str, repo: Option<&Path>) -> Option<RepoPlaywrightSummary> {
    let parsed: Value = serde_json::from_str(raw).ok()?;
    let stats = parsed.get("stats");
    let mut summary = RepoPlaywrightSummary {
        expected: stats
            .and_then(|s| s.get("expected"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        unexpected: stats
            .and_then(|s| s.get("unexpected"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        flaky: stats
            .and_then(|s| s.get("flaky"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        skipped: stats
            .and_then(|s| s.get("skipped"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize,
        failures: Vec::new(),
        artifacts: Vec::new(),
    };

    if let Some(suites) = parsed.get("suites").and_then(Value::as_array) {
        for suite in suites {
            collect_playwright_details(
                suite,
                &mut Vec::new(),
                &mut summary.failures,
                &mut summary.artifacts,
                repo,
            );
            if summary.failures.len() >= 8 && summary.artifacts.len() >= 12 {
                break;
            }
        }
    }

    Some(summary)
}

fn normalize_repo_trace_mode(value: Option<&str>) -> Result<String, String> {
    let mode = value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("retain-on-failure");
    match mode {
        "off" | "on" | "retain-on-failure" => Ok(mode.to_string()),
        other => Err(format!(
            "unsupported repo Playwright trace mode: {other}. Supported: off, on, retain-on-failure"
        )),
    }
}

fn is_loopback_base_url(value: &str) -> bool {
    let lower = value.trim().to_lowercase();
    let without_scheme = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"));
    let Some(rest) = without_scheme else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .unwrap_or(host)
            .trim_start_matches('[')
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
        || host.ends_with(".localhost")
        || host.starts_with("127.")
}

fn classify_playwright_spec(path: &Path) -> Option<String> {
    let lower_path = path.to_string_lossy().to_lowercase();
    if lower_path.contains("/e2e/") || lower_path.contains("playwright") {
        return Some("path".to_string());
    }
    let content = std::fs::read_to_string(path).ok()?;
    let sample: String = content.chars().take(8192).collect();
    if sample.contains("@playwright/test") || sample.contains("from \"playwright\"") {
        return Some("import".to_string());
    }
    None
}

fn scan_playwright_specs(root: &Path) -> Vec<PlaywrightSpecCandidate> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0usize;

    while let Some(dir) = stack.pop() {
        if visited > 5000 || out.len() >= 60 {
            break;
        }
        if should_skip_scan_dir(&dir) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            let path = entry.path();
            if path.is_dir() {
                if !should_skip_scan_dir(&path) {
                    stack.push(path);
                }
                continue;
            }
            if !looks_like_spec(&path) {
                continue;
            }
            if let Some(reason) = classify_playwright_spec(&path) {
                if let Some(rel) = relative_path(root, &path) {
                    out.push(PlaywrightSpecCandidate { path: rel, reason });
                }
            }
        }
    }

    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[tauri::command]
pub async fn discover_playwright_specs(repo_path: String) -> Result<Value, String> {
    let root = PathBuf::from(repo_path.trim());
    if !root.is_dir() {
        return Err("repo_path must be an existing directory".into());
    }
    let specs = scan_playwright_specs(&root);
    Ok(json!({ "specs": specs }))
}

#[tauri::command]
pub async fn record_synthetic_qa_run(
    db: State<'_, DbState>,
    input: RecordSyntheticQaRunInput,
) -> Result<Value, String> {
    let trace_json = serde_json::to_string(&input.run.trace).ok();
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let row = queries::insert_synthetic_qa_run(
        &conn,
        &queries::SyntheticQaRunInput {
            review_id: input
                .review_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            repo_path: input
                .repo_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            loop_id: input.run.loop_id.clone(),
            runner_type: input
                .run
                .runner_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            base_url: input
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            route: Some(input.run.route.clone()),
            goal: Some(input.run.goal.clone()),
            pass: input.run.pass,
            duration_ms: input.run.duration_ms as i64,
            notes: Some(input.run.notes.clone()),
            screenshot_path: input.run.screenshot_path.clone(),
            artifacts: input.run.artifacts.clone(),
            console_errors: input.run.trace.console_errors.len() as i64,
            error: input.run.error.clone(),
            trace_json,
        },
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({ "run": row }))
}

#[tauri::command]
pub async fn list_synthetic_qa_runs(
    db: State<'_, DbState>,
    review_id: String,
    limit: Option<i64>,
) -> Result<Value, String> {
    let review_id = review_id.trim().to_string();
    if review_id.is_empty() {
        return Ok(json!({ "runs": [] }));
    }
    let limit = limit.unwrap_or(8).clamp(1, 50);
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let runs = queries::list_synthetic_qa_runs_for_review(&conn, &review_id, limit)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "runs": runs }))
}

/// Run the first synthetic-user QA loop against a local HTTP app (Playwright).
#[tauri::command]
pub async fn run_synthetic_qa(
    app: tauri::AppHandle,
    base_url: String,
    loop_id: Option<String>,
    runner_type: Option<String>,
    goal: Option<String>,
    external_command: Option<String>,
    auth_mode: Option<String>,
    storage_state_path: Option<String>,
    target_route: Option<String>,
    repo_path: Option<String>,
    spec_path: Option<String>,
    allow_remote_target: Option<bool>,
    repo_trace_mode: Option<String>,
) -> Result<SyntheticQaRunResult, String> {
    let loop_id = loop_id.unwrap_or_else(|| "codevetter-review-shell".to_string());
    let runner_type = runner_type.unwrap_or_else(|| "playwright_builtin".to_string());
    let auth_mode = auth_mode.unwrap_or_else(|| "none".to_string());
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() {
        return Err("base_url is required (e.g. http://localhost:1420)".into());
    }
    if !allow_remote_target.unwrap_or(false) && !is_loopback_base_url(&base_url) {
        return Err(
            "Synthetic QA remote targets are disabled. Use a localhost URL or enable remote target QA explicitly.".into(),
        );
    }
    if auth_mode != "none" && auth_mode != "storage_state" {
        return Err(format!(
            "unsupported synthetic QA auth_mode: {auth_mode}. Supported: none, storage_state"
        ));
    }
    let storage_state_path = storage_state_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let target_route = target_route
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let repo_path = repo_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let spec_path = spec_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    if auth_mode == "storage_state" && storage_state_path.is_none() {
        return Err("storage_state_path is required when auth_mode=storage_state".into());
    }
    let repo_trace_mode = normalize_repo_trace_mode(repo_trace_mode.as_deref())?;

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    let run_id = format!(
        "{}-{}",
        loop_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let artifact_dir = app_data.join("synthetic-qa").join(&run_id);
    std::fs::create_dir_all(&artifact_dir).map_err(|e| format!("create artifact dir: {e}"))?;

    let output = match runner_type.as_str() {
        "playwright_builtin" => {
            let script = resolve_runner_script()?;
            StdCommand::new("node")
                .arg(&script)
                .arg("--base-url")
                .arg(&base_url)
                .arg("--loop-id")
                .arg(&loop_id)
                .arg("--artifact-dir")
                .arg(&artifact_dir)
                .arg("--goal")
                .arg(goal.as_deref().unwrap_or(""))
                .arg("--auth-mode")
                .arg(&auth_mode)
                .args(
                    target_route
                        .as_deref()
                        .map(|route| vec!["--route", route])
                        .unwrap_or_default(),
                )
                .args(
                    storage_state_path
                        .as_deref()
                        .map(|p| vec!["--storage-state", p])
                        .unwrap_or_default(),
                )
                .output()
                .map_err(|e| format!("failed to spawn node runner: {e}"))?
        }
        "external_skill" => {
            let command = external_command
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    "external_command is required for external_skill runner".to_string()
                })?;
            let mut parts = split_shell_like_command(command)?;
            let program = parts.remove(0);
            let goal = goal.unwrap_or_else(|| {
                "Exercise the changed user workflow and return CodeVetter SyntheticQaRunResult JSON.".to_string()
            });

            StdCommand::new(program)
                .args(parts)
                .arg("--base-url")
                .arg(&base_url)
                .arg("--loop-id")
                .arg(&loop_id)
                .arg("--goal")
                .arg(&goal)
                .arg("--artifact-dir")
                .arg(&artifact_dir)
                .arg("--auth-mode")
                .arg(&auth_mode)
                .args(
                    target_route
                        .as_deref()
                        .map(|route| vec!["--route", route])
                        .unwrap_or_default(),
                )
                .args(
                    storage_state_path
                        .as_deref()
                        .map(|p| vec!["--storage-state", p])
                        .unwrap_or_default(),
                )
                .output()
                .map_err(|e| format!("failed to spawn external synthetic QA runner: {e}"))?
        }
        "repo_playwright" => {
            let repo = repo_path
                .as_deref()
                .ok_or_else(|| "repo_path is required for repo_playwright runner".to_string())?;
            let spec = spec_path
                .as_deref()
                .ok_or_else(|| "spec_path is required for repo_playwright runner".to_string())?;
            let spec_as_path = std::path::Path::new(spec);
            if spec_as_path.is_absolute() || spec.split('/').any(|part| part == "..") {
                return Err("repo_playwright spec_path must be repository-relative".into());
            }

            let started = std::time::Instant::now();
            let playwright = resolve_repo_playwright_binary(repo);
            let mut command = StdCommand::new(&playwright);
            if playwright == "npx" {
                command.arg("playwright");
            }
            command
                .env("CODEVETTER_SYNTHETIC_QA_BASE_URL", &base_url)
                .env(
                    "CODEVETTER_SYNTHETIC_QA_ROUTE",
                    target_route.as_deref().unwrap_or("/"),
                )
                .env("CODEVETTER_SYNTHETIC_QA_LOOP_ID", &loop_id)
                .env(
                    "CODEVETTER_SYNTHETIC_QA_GOAL",
                    goal.as_deref().unwrap_or(""),
                )
                .env("CODEVETTER_SYNTHETIC_QA_AUTH_MODE", &auth_mode)
                .env(
                    "CODEVETTER_SYNTHETIC_QA_ARTIFACT_DIR",
                    artifact_dir.to_string_lossy().as_ref(),
                );
            if let Some(path) = storage_state_path.as_deref() {
                command.env("CODEVETTER_SYNTHETIC_QA_STORAGE_STATE", path);
            }
            let repo_output_dir = artifact_dir.join("repo-playwright-output");
            command
                .env("CODEVETTER_SYNTHETIC_QA_TRACE_MODE", &repo_trace_mode)
                .env(
                    "CODEVETTER_SYNTHETIC_QA_PLAYWRIGHT_OUTPUT_DIR",
                    repo_output_dir.to_string_lossy().as_ref(),
                );
            let output = command
                .args(["test", spec, "--reporter=json"])
                .arg("--trace")
                .arg(&repo_trace_mode)
                .arg("--output")
                .arg(&repo_output_dir)
                .current_dir(repo)
                .output()
                .map_err(|e| format!("failed to spawn repo Playwright runner: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let log_path = artifact_dir.join("repo-playwright.log");
            let report_path = artifact_dir.join("repo-playwright-report.json");
            std::fs::write(
                &log_path,
                format!(
                    "$ {} test {spec} --reporter=json --trace {} --output {}\n\n--- stdout ---\n{}\n\n--- stderr ---\n{}",
                    playwright,
                    repo_trace_mode,
                    repo_output_dir.to_string_lossy(),
                    stdout.trim(),
                    stderr.trim()
                ),
            )
            .map_err(|e| format!("write repo Playwright log: {e}"))?;
            let repo_root = PathBuf::from(repo);
            let summary = parse_repo_playwright_summary(&stdout, Some(&repo_root));
            if summary.is_some() {
                std::fs::write(&report_path, stdout.as_bytes())
                    .map_err(|e| format!("write repo Playwright report: {e}"))?;
            }

            let pass = output.status.success();
            let notes = match (pass, summary.as_ref()) {
                (true, Some(summary)) => format!(
                    "Repo Playwright spec passed: {spec} ({} passed, {} skipped, {} artifact(s), trace={}). Report: {}",
                    summary.expected,
                    summary.skipped,
                    summary.artifacts.len(),
                    repo_trace_mode,
                    report_path.to_string_lossy()
                ),
                (true, None) => format!("Repo Playwright spec passed: {spec}"),
                (false, Some(summary)) => {
                    let failures = if summary.failures.is_empty() {
                        "No structured failure message found.".to_string()
                    } else {
                        summary.failures.join(" | ")
                    };
                    let artifacts = if summary.artifacts.is_empty() {
                        String::new()
                    } else {
                        format!(" Artifacts: {}.", summary.artifacts.join(", "))
                    };
                    format!(
                        "Repo Playwright spec failed: {spec} ({} failed, {} flaky, {} passed, {} skipped). {failures}.{artifacts} Log: {}",
                        summary.unexpected,
                        summary.flaky,
                        summary.expected,
                        summary.skipped,
                        log_path.to_string_lossy()
                    )
                }
                (false, None) => format!(
                    "Repo Playwright spec failed: {spec}. Log: {}",
                    log_path.to_string_lossy()
                ),
            };
            let console_errors = if pass {
                Vec::new()
            } else if let Some(summary) = summary.as_ref() {
                summary.failures.clone()
            } else {
                vec![stderr.trim().chars().take(500).collect::<String>()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            let mut artifacts = summary
                .as_ref()
                .map(|summary| summary.artifacts.clone())
                .unwrap_or_default();
            let report_artifact = report_path.to_string_lossy().into_owned();
            if summary.is_some() && !artifacts.contains(&report_artifact) {
                artifacts.push(report_artifact);
            }
            let log_artifact = log_path.to_string_lossy().into_owned();
            if !artifacts.contains(&log_artifact) {
                artifacts.push(log_artifact);
            }
            return Ok(SyntheticQaRunResult {
                loop_id,
                route: target_route.unwrap_or_else(|| "/".to_string()),
                goal: goal.unwrap_or_else(|| format!("Run repo Playwright spec {spec}")),
                pass,
                notes,
                screenshot_path: if pass {
                    None
                } else {
                    artifacts
                        .first()
                        .cloned()
                        .or_else(|| Some(log_path.to_string_lossy().into_owned()))
                },
                artifacts,
                duration_ms: started.elapsed().as_millis() as u64,
                trace: SyntheticQaTrace {
                    final_url: base_url,
                    page_title: "repo_playwright".to_string(),
                    console_errors,
                },
                error: if pass {
                    None
                } else {
                    Some(format!(
                        "Playwright test exited with {:?}",
                        output.status.code()
                    ))
                },
                runner_type: Some(runner_type),
            });
        }
        other => {
            return Err(format!(
                "unsupported synthetic QA runner_type: {other}. Supported: playwright_builtin, external_skill, repo_playwright"
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            format!(
                "runner produced no JSON (exit {}). stdout: {} stderr: {}",
                output.status.code().unwrap_or(-1),
                stdout.trim(),
                stderr.trim()
            )
        })?;

    let mut result: SyntheticQaRunResult =
        serde_json::from_str(line).map_err(|e| format!("parse runner JSON: {e} ({line})"))?;

    // Normalize screenshot path to absolute string for the UI
    if let Some(ref p) = result.screenshot_path {
        if !p.is_empty() {
            result.screenshot_path = Some(PathBuf::from(p).to_string_lossy().into_owned());
        }
    }
    result.artifacts = result
        .artifacts
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| PathBuf::from(p).to_string_lossy().into_owned())
        .collect();
    if result.artifacts.is_empty() {
        if let Some(path) = result.screenshot_path.as_deref() {
            if !path.trim().is_empty() {
                result.artifacts.push(path.to_string());
            }
        }
    }
    result.runner_type = Some(runner_type.clone());

    if !output.status.success() && result.error.is_none() && !result.pass {
        // Playwright exit 2 = failed assertions; still return structured result
        log::info!(
            "Synthetic QA loop {} finished with exit {:?}",
            loop_id,
            output.status.code()
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_playwright_specs_and_skips_plain_unit_tests() {
        let root = std::env::temp_dir().join(format!(
            "codevetter-spec-scan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("tests/e2e")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("tests/e2e/settings.spec.ts"),
            "import { test } from '@playwright/test'; test('ok', async () => {});",
        )
        .unwrap();
        std::fs::write(
            root.join("src/math.test.ts"),
            "import { test } from 'node:test';",
        )
        .unwrap();

        let specs = scan_playwright_specs(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].path, "tests/e2e/settings.spec.ts");
    }

    #[test]
    fn loopback_base_url_guard_rejects_remote_targets() {
        assert!(is_loopback_base_url("http://localhost:1420"));
        assert!(is_loopback_base_url("https://127.0.0.1:3000/app"));
        assert!(is_loopback_base_url("http://app.localhost:5173"));
        assert!(is_loopback_base_url("http://[::1]:1420"));
        assert!(!is_loopback_base_url("https://example.com"));
        assert!(!is_loopback_base_url("http://192.168.1.4:3000"));
    }

    #[test]
    fn normalizes_repo_trace_mode() {
        assert_eq!(
            normalize_repo_trace_mode(None).unwrap(),
            "retain-on-failure"
        );
        assert_eq!(normalize_repo_trace_mode(Some("on")).unwrap(), "on");
        assert_eq!(normalize_repo_trace_mode(Some("off")).unwrap(), "off");
        assert!(normalize_repo_trace_mode(Some("always")).is_err());
    }

    #[test]
    fn parses_repo_playwright_json_failures() {
        let raw = r#"{
          "stats": { "expected": 2, "unexpected": 1, "flaky": 0, "skipped": 1 },
          "suites": [{
            "title": "tests/e2e/settings.spec.ts",
            "specs": [{
              "title": "can save settings",
              "tests": [{
                "projectName": "chromium",
                "status": "unexpected",
                "results": [{
                  "status": "failed",
                  "error": { "message": "Error: expected Save button to be enabled\n    at settings.spec.ts:10" },
                  "attachments": [
                    { "name": "screenshot", "contentType": "image/png", "path": "test-results/settings/failure.png" },
                    { "name": "trace", "contentType": "application/zip", "path": "/tmp/codevetter-trace.zip" }
                  ]
                }]
              }]
            }]
          }]
        }"#;

        let repo = PathBuf::from("/repo/project");
        let summary = parse_repo_playwright_summary(raw, Some(&repo)).expect("valid report");

        assert_eq!(summary.expected, 2);
        assert_eq!(summary.unexpected, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.failures[0].contains("tests/e2e/settings.spec.ts > can save settings"));
        assert!(summary.failures[0].contains("[chromium]"));
        assert!(summary.failures[0].contains("expected Save button"));
        assert_eq!(summary.artifacts.len(), 2);
        assert_eq!(
            summary.artifacts[0],
            "/repo/project/test-results/settings/failure.png"
        );
        assert_eq!(summary.artifacts[1], "/tmp/codevetter-trace.zip");
    }

    #[test]
    fn parse_repo_playwright_json_returns_none_for_raw_logs() {
        assert!(parse_repo_playwright_summary("Running 1 test\n1 passed", None).is_none());
    }

    #[test]
    fn split_shell_like_command_preserves_quoted_args() {
        let args = split_shell_like_command(r#"python -c "print('hello world')" --flag 'a b'"#)
            .expect("split should work");
        assert_eq!(
            args,
            vec![
                "python".to_string(),
                "-c".to_string(),
                "print('hello world')".to_string(),
                "--flag".to_string(),
                "a b".to_string(),
            ]
        );
    }

    #[test]
    fn split_shell_like_command_preserves_empty_quoted_args() {
        let args = split_shell_like_command(r#"tool --flag "" tail"#).expect("split should work");
        assert_eq!(
            args,
            vec![
                "tool".to_string(),
                "--flag".to_string(),
                "".to_string(),
                "tail".to_string(),
            ]
        );
    }
}
