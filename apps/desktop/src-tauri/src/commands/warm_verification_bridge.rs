//! Safe Tauri orchestration for a repository-owned warm-verification CLI.

use crate::{
    commands::{differential_verification, warm_verification},
    DbState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::State;
use tokio::{io::AsyncReadExt, process::Command, time::timeout};

const MAX_PACKAGE_JSON_BYTES: u64 = 262_144;
const MAX_PROCESS_OUTPUT_BYTES: u64 = 1_048_576;
const MAX_WORKSPACE_PATTERNS: usize = 128;
const MAX_WORKSPACE_CANDIDATES: usize = 2_048;
const STATUS_TIMEOUT: Duration = Duration::from_secs(8);
const START_TIMEOUT: Duration = Duration::from_secs(45);
const STOP_TIMEOUT: Duration = Duration::from_secs(20);
// `verify changed` may spend up to 30 seconds warming the owned daemon before
// its separately bounded 30-second batch and 5-second IPC response window.
const RUN_TIMEOUT: Duration = Duration::from_secs(70);
const DIFFERENTIAL_RUN_TIMEOUT: Duration = Duration::from_secs(320);
const DIFFERENTIAL_PREPARE_TIMEOUT: Duration = Duration::from_secs(320);
const SETUP_REMEDIATION: &str = "Add one workspace package with a compatible `verify` script, install its lockfile dependencies, and ensure that lockfile's package manager is on PATH.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PackageManager {
    Pnpm,
    Npm,
    Yarn,
    Bun,
}

impl PackageManager {
    fn executable(self) -> &'static str {
        match self {
            Self::Pnpm => "pnpm",
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }

    fn arguments(self, cli_arguments: &[String]) -> Vec<String> {
        let mut arguments = match self {
            Self::Pnpm | Self::Yarn => vec!["--silent", "run", "verify"],
            Self::Npm => vec!["--silent", "run", "verify", "--"],
            Self::Bun => vec!["run", "--silent", "verify"],
        }
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        arguments.extend_from_slice(cli_arguments);
        arguments
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifyPackage {
    repo_root: PathBuf,
    package_root: PathBuf,
    manager: PackageManager,
}

#[derive(Debug)]
struct ProcessOutput {
    success: bool,
    status_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct CliError {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct CliErrorResponse {
    #[serde(rename = "type")]
    response_type: String,
    error: CliError,
}

#[derive(Debug, Serialize)]
pub struct WarmCancelResponse {
    accepted: bool,
}

#[derive(Debug, Serialize)]
pub struct WarmStopResponse {
    active_run_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DifferentialPreparedSummary {
    schema_version: u8,
    run_id: String,
    status: String,
    reference_sha: Option<String>,
    candidate_kind: String,
    candidate_identity: Option<String>,
    selection_identity: Option<String>,
    scenario_count: u32,
    source_cache_hits: u8,
    dependency_cache_hit: bool,
    prepared_bytes: u64,
    reason_codes: Vec<String>,
    model_call_count: u8,
    cleanup_complete: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DifferentialCleanupSummary {
    schema_version: u8,
    dry_run: bool,
    complete: bool,
    removed_source_cache_keys: Vec<String>,
    removed_dependency_cache_keys: Vec<String>,
    removed_targets: u32,
    removed_staging: u32,
    skipped_entries: u32,
    retained_entries: u32,
    retained_logical_bytes: u64,
    retained_allocated_bytes: u64,
    warm_artifact_reclaimed_bytes: u64,
    warm_artifact_removed_files: u32,
    shared_playwright_cache_bytes: u64,
    error_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DifferentialStatusSummary {
    schema_version: u8,
    run_id: String,
    state: String,
    updated_at: String,
    classification: Option<String>,
    reason_codes: Vec<String>,
}

fn valid_reason_codes(values: &[String]) -> bool {
    values.len() <= 100
        && values
            .iter()
            .all(|value| valid_bounded_text(value) && value.len() <= 256)
}

fn validate_differential_prepared(
    summary: DifferentialPreparedSummary,
) -> Result<DifferentialPreparedSummary, String> {
    let hashes_valid = summary
        .reference_sha
        .as_deref()
        .is_none_or(|value| valid_hash(value, 40, 64))
        && summary
            .candidate_identity
            .as_deref()
            .is_none_or(|value| valid_hash(value, 64, 64))
        && summary
            .selection_identity
            .as_deref()
            .is_none_or(|value| valid_hash(value, 64, 64));
    if summary.schema_version != 1
        || !valid_id(&summary.run_id)
        || !matches!(summary.status.as_str(), "ready" | "incomparable")
        || !matches!(
            summary.candidate_kind.as_str(),
            "worktree" | "staged" | "commit" | "range"
        )
        || summary.scenario_count > 500
        || summary.source_cache_hits > 2
        || summary.model_call_count != 0
        || !hashes_valid
        || !valid_reason_codes(&summary.reason_codes)
    {
        return Err("Repository verifier returned invalid differential preparation data".into());
    }
    Ok(summary)
}

fn validate_differential_cleanup(
    summary: DifferentialCleanupSummary,
) -> Result<DifferentialCleanupSummary, String> {
    let valid_cache_keys = |values: &[String]| {
        values.len() <= 1_000 && values.iter().all(|value| valid_hash(value, 64, 64))
    };
    if summary.schema_version != 1
        || !valid_cache_keys(&summary.removed_source_cache_keys)
        || !valid_cache_keys(&summary.removed_dependency_cache_keys)
        || !valid_reason_codes(&summary.error_codes)
    {
        return Err("Repository verifier returned invalid differential cleanup data".into());
    }
    Ok(summary)
}

fn validate_differential_status(
    summary: DifferentialStatusSummary,
) -> Result<DifferentialStatusSummary, String> {
    if summary.schema_version != 1
        || !valid_id(&summary.run_id)
        || !matches!(
            summary.state.as_str(),
            "not_found"
                | "preparing"
                | "running"
                | "cancelling"
                | "completed"
                | "incomparable"
                | "cancelled"
                | "locked"
        )
        || summary
            .classification
            .as_deref()
            .is_some_and(|classification| {
                !matches!(
                    classification,
                    "regressed" | "improved" | "unchanged" | "incomparable"
                )
            })
        || !valid_reason_codes(&summary.reason_codes)
        || chrono::DateTime::parse_from_rfc3339(&summary.updated_at).is_err()
    {
        return Err("Repository verifier returned invalid differential status data".into());
    }
    Ok(summary)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarmRuntimeExit {
    code: Option<i32>,
    signal: Option<String>,
    at: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarmOwnedRuntimeHealth {
    kind: String,
    state: String,
    owned: bool,
    pid: Option<u32>,
    start_identity: Option<String>,
    restart_attempts: u8,
    last_exit: Option<WarmRuntimeExit>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarmDaemonResourceUsage {
    rss_bytes: u64,
    heap_used_bytes: u64,
    active_contexts: u32,
    retained_artifact_bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarmDaemonHealth {
    schema_version: u8,
    daemon_pid: u32,
    daemon_start_identity: String,
    target_root: String,
    target_sha: String,
    config_hash: String,
    chromium_revision: String,
    cold_startup_ms: Option<f64>,
    warm: bool,
    server: WarmOwnedRuntimeHealth,
    browser: WarmOwnedRuntimeHealth,
    active_run_ids: Vec<String>,
    resources: WarmDaemonResourceUsage,
    checked_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WarmVerificationCleanupReport {
    schema_version: u8,
    dry_run: bool,
    removed_runs: usize,
    removed_files: usize,
    reclaimed_bytes: u64,
    retained_bytes: u64,
    shared_playwright_cache_bytes: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentWarmVerificationIdentity {
    schema_version: u8,
    target_sha: String,
    change_set_kind: String,
    change_set_identity: String,
    config_hash: String,
    manifest_hash: String,
    source_hash: String,
    observation_policy_profile_id: String,
}

fn canonical_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(repo_path.trim());
    if repo_path.len() > 4_096 || !candidate.is_absolute() {
        return Err("Repository path must be a bounded absolute path".into());
    }
    candidate
        .canonicalize()
        .map_err(|_| "Repository path is not accessible".to_string())
        .and_then(|path| {
            path.is_dir()
                .then_some(path)
                .ok_or_else(|| "Repository path is not a directory".to_string())
        })
}

fn read_package_json(path: &Path) -> Result<Value, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| format!("Package manifest is not readable: {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!(
            "Package manifest must be a real file: {}",
            path.display()
        ));
    }
    if metadata.len() > MAX_PACKAGE_JSON_BYTES {
        return Err(format!(
            "Package manifest exceeds {MAX_PACKAGE_JSON_BYTES} bytes"
        ));
    }
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|_| format!("Package manifest is not valid JSON: {}", path.display()))
}

fn workspace_patterns(root_manifest: &Value) -> Result<Vec<String>, String> {
    let value = root_manifest.get("workspaces");
    let entries = match value {
        Some(Value::Array(entries)) => entries,
        Some(Value::Object(object)) => object
            .get("packages")
            .and_then(Value::as_array)
            .ok_or_else(|| "package.json workspaces.packages must be an array".to_string())?,
        None => return Ok(Vec::new()),
        _ => return Err("package.json workspaces must be an array or packages object".into()),
    };
    if entries.len() > MAX_WORKSPACE_PATTERNS {
        return Err(format!(
            "package.json exceeds {MAX_WORKSPACE_PATTERNS} workspace patterns"
        ));
    }
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= 512)
                .map(str::to_owned)
                .ok_or_else(|| "Workspace patterns must be bounded non-empty strings".to_string())
        })
        .collect()
}

fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("Unsafe workspace path: {value}"));
    }
    Ok(path.to_path_buf())
}

fn expand_workspace_pattern(repo_root: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    if let Some(base) = pattern.strip_suffix("/*") {
        let base = repo_root.join(safe_relative_path(base)?);
        if !base.is_dir() {
            return Ok(Vec::new());
        }
        let mut directories = Vec::new();
        for entry in fs::read_dir(base).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let metadata = entry.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir()
                && !entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_symlink()
            {
                directories.push(entry.path());
                if directories.len() > MAX_WORKSPACE_CANDIDATES {
                    return Err(format!(
                        "Workspace pattern exceeds {MAX_WORKSPACE_CANDIDATES} directories"
                    ));
                }
            }
        }
        directories.sort();
        return Ok(directories);
    }
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        return Err(format!(
            "Unsupported workspace pattern `{pattern}`; use a literal path or one trailing /*"
        ));
    }
    Ok(vec![repo_root.join(safe_relative_path(pattern)?)])
}

fn lockfile_manager(repo_root: &Path) -> Result<PackageManager, String> {
    let candidates = [
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("package-lock.json", PackageManager::Npm),
        ("yarn.lock", PackageManager::Yarn),
        ("bun.lock", PackageManager::Bun),
        ("bun.lockb", PackageManager::Bun),
    ];
    let managers = candidates
        .into_iter()
        .filter(|(name, _)| repo_root.join(name).is_file())
        .map(|(_, manager)| manager)
        .collect::<BTreeSet<_>>();
    let managers = managers.into_iter().collect::<Vec<_>>();
    match managers.as_slice() {
        [manager] => Ok(*manager),
        [] => Err(format!(
            "No supported lockfile was found. {SETUP_REMEDIATION}"
        )),
        _ => Err("Multiple package-manager lockfiles make verifier execution ambiguous".into()),
    }
}

fn find_verify_package(repo_path: &str) -> Result<VerifyPackage, String> {
    let repo_root = canonical_repo_path(repo_path)?;
    let root_manifest = read_package_json(&repo_root.join("package.json"))?;
    let patterns = workspace_patterns(&root_manifest)?;
    let package_roots = if patterns.is_empty() {
        BTreeSet::new()
    } else {
        patterns
            .iter()
            .map(|pattern| expand_workspace_pattern(&repo_root, pattern))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<BTreeSet<_>>()
    };
    if package_roots.len() > MAX_WORKSPACE_CANDIDATES {
        return Err(format!(
            "Workspace discovery exceeds {MAX_WORKSPACE_CANDIDATES} candidate packages"
        ));
    }
    let mut matches = BTreeSet::new();
    for package_root in package_roots {
        let metadata = match fs::symlink_metadata(&package_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_dir() && !metadata.file_type().is_symlink() {
            continue;
        }
        // Resolve and contain the package directory before reading any file
        // through it. A literal workspace entry may itself be a symlink.
        let canonical = package_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !canonical.starts_with(&repo_root) {
            return Err("Verifier workspace resolves outside the repository".into());
        }
        let manifest_path = canonical.join("package.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = read_package_json(&manifest_path)?;
        let verify_script = manifest
            .get("scripts")
            .and_then(Value::as_object)
            .and_then(|scripts| scripts.get("verify"))
            .and_then(Value::as_str)
            .filter(|script| !script.trim().is_empty() && script.len() <= 2_048);
        if verify_script.is_some() {
            matches.insert(canonical);
        }
    }
    // Prefer one concrete workspace verifier. If none owns the command, permit
    // a root verifier so pnpm repositories that declare workspaces only in
    // pnpm-workspace.yaml still have a bounded setup path.
    if matches.is_empty() {
        let root_verify_script = root_manifest
            .get("scripts")
            .and_then(Value::as_object)
            .and_then(|scripts| scripts.get("verify"))
            .and_then(Value::as_str)
            .filter(|script| !script.trim().is_empty() && script.len() <= 2_048);
        if root_verify_script.is_some() {
            matches.insert(repo_root.clone());
        }
    }
    if matches.len() != 1 {
        return Err(format!(
            "Expected exactly one workspace package with a `verify` script, found {}. {SETUP_REMEDIATION}",
            matches.len()
        ));
    }
    Ok(VerifyPackage {
        manager: lockfile_manager(&repo_root)?,
        repo_root,
        package_root: matches
            .into_iter()
            .next()
            .ok_or_else(|| "Verifier package disappeared during discovery".to_string())?,
    })
}

fn allowed_environment() -> Vec<(String, String)> {
    const NAMES: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "TMPDIR",
        "TMP",
        "TEMP",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "PNPM_HOME",
        "NVM_BIN",
        "VOLTA_HOME",
        "COREPACK_HOME",
        "PLAYWRIGHT_BROWSERS_PATH",
        "NO_COLOR",
    ];
    NAMES
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_string(), value))
        })
        .filter(|(_, value)| value.len() <= 16_384)
        .collect()
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(reader: R) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_PROCESS_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PROCESS_OUTPUT_BYTES {
        return Err(format!(
            "Verifier output exceeds {MAX_PROCESS_OUTPUT_BYTES} bytes"
        ));
    }
    Ok(bytes)
}

async fn execute_verify(
    package: &VerifyPackage,
    cli_arguments: &[String],
    deadline: Duration,
) -> Result<ProcessOutput, String> {
    execute_verify_program(
        package,
        cli_arguments,
        deadline,
        Path::new(package.manager.executable()),
    )
    .await
}

async fn execute_verify_program(
    package: &VerifyPackage,
    cli_arguments: &[String],
    deadline: Duration,
    program: &Path,
) -> Result<ProcessOutput, String> {
    let mut command = Command::new(program);
    command
        .args(package.manager.arguments(cli_arguments))
        .current_dir(&package.package_root)
        .env_clear()
        .envs(allowed_environment())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|_| {
        format!(
            "Could not start `{}`. {SETUP_REMEDIATION}",
            package.manager.executable()
        )
    })?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or("Verifier stdout was unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Verifier stderr was unavailable")?;
    let stdout_task = tokio::spawn(read_bounded(stdout));
    let stderr_task = tokio::spawn(read_bounded(stderr));
    let status = match timeout(deadline, child.wait()).await {
        Ok(status) => status.map_err(|error| error.to_string())?,
        Err(_) => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(
                "Repository verifier timed out and its owned client process was stopped".into(),
            );
        }
    };
    let stdout = stdout_task.await.map_err(|error| error.to_string())??;
    let stderr = stderr_task.await.map_err(|error| error.to_string())??;
    Ok(ProcessOutput {
        success: status.success(),
        status_code: status.code(),
        stdout,
        stderr,
    })
}

fn parse_json_output(output: &ProcessOutput) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        format!(
            "Repository verifier returned invalid versioned JSON (exit {:?})",
            output.status_code
        )
    })?;
    if let Ok(error) = serde_json::from_value::<CliErrorResponse>(value.clone()) {
        if error.response_type == "error" {
            return Err(format!("{}: {}", error.error.code, error.error.message));
        }
    }
    let result_exit = matches!(
        value.get("type").and_then(Value::as_str),
        Some(
            "verify_result"
                | "differential_result"
                | "differential_prepared"
                | "differential_status"
                | "differential_cleanup"
        )
    ) && matches!(output.status_code, Some(0 | 2 | 3));
    let scenario_exit = value.get("schema_version").and_then(Value::as_u64) == Some(1)
        && value.get("action").and_then(Value::as_str).is_some()
        && matches!(
            value.get("status").and_then(Value::as_str),
            Some("rejected" | "failed")
        )
        && matches!(output.status_code, Some(2 | 3));
    if !output.success && !result_exit && !scenario_exit {
        let detail = bounded_diagnostic(&output.stderr);
        return Err(format!(
            "Repository verifier exited {:?}{}",
            output.status_code,
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }
    Ok(value)
}

fn differential_candidate_arguments<'a>(
    candidate_kind: &'a str,
    candidate_revision: Option<&'a str>,
) -> Result<Vec<&'a str>, String> {
    match candidate_kind {
        "worktree" => Ok(Vec::new()),
        "staged" => Ok(vec!["--staged"]),
        "commit" | "range" => {
            let revision = candidate_revision
                .filter(|revision| valid_bounded_text(revision))
                .ok_or("Differential candidate revision is invalid")?;
            Ok(vec![
                if candidate_kind == "commit" {
                    "--commit"
                } else {
                    "--range"
                },
                revision,
            ])
        }
        _ => Err("Differential candidate kind is invalid".into()),
    }
}

fn require_pnpm_differential(package: &VerifyPackage) -> Result<(), String> {
    if package.manager != PackageManager::Pnpm {
        return Err(
            "Differential verification currently supports pnpm repositories only; use warm verification for this repository"
                .into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn prepare_differential_verification(
    repo_path: String,
    run_id: String,
    reference_revision: String,
    candidate_kind: String,
    candidate_revision: Option<String>,
) -> Result<DifferentialPreparedSummary, String> {
    if !valid_id(&run_id) || !valid_bounded_text(&reference_revision) {
        return Err("Differential run identity or reference is invalid".into());
    }
    let mut command = vec![
        "differential",
        "prepare",
        "--run-id",
        run_id.as_str(),
        "--reference",
        reference_revision.as_str(),
    ];
    command.extend(differential_candidate_arguments(
        candidate_kind.as_str(),
        candidate_revision.as_deref(),
    )?);
    let package = find_verify_package(&repo_path)?;
    require_pnpm_differential(&package)?;
    let output = execute_verify(
        &package,
        &cli_args(&package.repo_root, &command),
        DIFFERENTIAL_PREPARE_TIMEOUT,
    )
    .await?;
    let value = parse_json_output(&output)?;
    let summary = validate_differential_prepared(
        serde_json::from_value(response_payload(value, "differential_prepared", "summary")?)
            .map_err(|_| "Repository verifier returned invalid differential preparation data")?,
    )?;
    if summary.run_id != run_id || summary.candidate_kind != candidate_kind {
        return Err(
            "Repository verifier returned different differential preparation inputs".into(),
        );
    }
    Ok(summary)
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace(['\r', '\n'], " ")
        .chars()
        .filter(|character| !character.is_control())
        .take(500)
        .collect::<String>()
        .trim()
        .to_string()
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn valid_hash(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_bounded_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16_384
}

fn validate_runtime_health(runtime: &WarmOwnedRuntimeHealth) -> Result<(), String> {
    if !matches!(runtime.kind.as_str(), "process" | "browser")
        || !matches!(
            runtime.state.as_str(),
            "stopped" | "starting" | "ready" | "unhealthy" | "recovering" | "locked"
        )
        || runtime.restart_attempts > 1
        || runtime
            .start_identity
            .as_deref()
            .is_some_and(|identity| !valid_bounded_text(identity))
    {
        return Err("Repository verifier returned invalid runtime health".into());
    }
    if !runtime.owned && (runtime.pid.is_some() || runtime.start_identity.is_some()) {
        return Err("Unowned runtime health exposed an owned identity".into());
    }
    if runtime.kind == "browser" && runtime.pid.is_some() {
        return Err("Browser runtime health must not invent a PID".into());
    }
    if runtime.state == "ready"
        && (!runtime.owned
            || runtime.start_identity.is_none()
            || (runtime.kind == "process" && runtime.pid.is_none()))
    {
        return Err("Ready runtime health is missing ownership identity".into());
    }
    if let Some(exit) = &runtime.last_exit {
        if exit
            .signal
            .as_deref()
            .is_some_and(|signal| !valid_bounded_text(signal))
            || chrono::DateTime::parse_from_rfc3339(&exit.at).is_err()
        {
            return Err("Repository verifier returned invalid runtime exit health".into());
        }
    }
    Ok(())
}

fn parse_health(value: Value, expected_repo_root: &Path) -> Result<WarmDaemonHealth, String> {
    let health: WarmDaemonHealth = serde_json::from_value(value)
        .map_err(|_| "Repository verifier returned invalid daemon health".to_string())?;
    let target_root = canonical_repo_path(&health.target_root)?;
    if health.schema_version != 1
        || health.daemon_pid == 0
        || !valid_bounded_text(&health.daemon_start_identity)
        || target_root != expected_repo_root
        || !valid_hash(&health.target_sha, 40, 64)
        || !valid_hash(&health.config_hash, 64, 64)
        || !valid_bounded_text(&health.chromium_revision)
        || health
            .cold_startup_ms
            .is_some_and(|duration| !(0.0..=300_000.0).contains(&duration))
        || health.active_run_ids.len() > 32
        || health.active_run_ids.iter().any(|run_id| !valid_id(run_id))
        || chrono::DateTime::parse_from_rfc3339(&health.checked_at).is_err()
    {
        return Err("Repository verifier daemon health contract is invalid".into());
    }
    validate_runtime_health(&health.server)?;
    validate_runtime_health(&health.browser)?;
    Ok(health)
}

fn cli_args(repo_root: &Path, command: &[&str]) -> Vec<String> {
    command
        .iter()
        .map(|value| (*value).to_string())
        .chain([
            "--repo".to_string(),
            repo_root.to_string_lossy().to_string(),
            "--json".to_string(),
        ])
        .collect()
}

fn response_payload(value: Value, expected_type: &str, field: &str) -> Result<Value, String> {
    if value.get("type").and_then(Value::as_str) != Some(expected_type) {
        return Err(format!(
            "Repository verifier did not return {expected_type}"
        ));
    }
    value
        .get(field)
        .cloned()
        .ok_or_else(|| format!("Repository verifier response is missing {field}"))
}

pub(crate) async fn run_cli(
    repo_path: &str,
    command: &[&str],
    deadline: Duration,
) -> Result<Value, String> {
    let package = find_verify_package(repo_path)?;
    let output = execute_verify(&package, &cli_args(&package.repo_root, command), deadline).await?;
    parse_json_output(&output)
}

async fn run_differential_cli(
    repo_path: &str,
    command: &[&str],
    deadline: Duration,
) -> Result<Value, String> {
    let package = find_verify_package(repo_path)?;
    require_pnpm_differential(&package)?;
    let output = execute_verify(&package, &cli_args(&package.repo_root, command), deadline).await?;
    parse_json_output(&output)
}

#[tauri::command]
pub async fn get_warm_verification_daemon_health(
    repo_path: String,
) -> Result<Option<WarmDaemonHealth>, String> {
    let package = find_verify_package(&repo_path)?;
    let output = execute_verify(
        &package,
        &cli_args(&package.repo_root, &["daemon", "status"]),
        STATUS_TIMEOUT,
    )
    .await?;
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Repository verifier returned invalid versioned JSON".to_string())?;
    if let Ok(error) = serde_json::from_value::<CliErrorResponse>(value.clone()) {
        if error.response_type == "error"
            && ["connection", "timeout"].contains(&error.error.code.as_str())
        {
            return Ok(None);
        }
    }
    let value = parse_json_output(&output)?;
    let health = response_payload(value, "health", "health")?;
    Ok(Some(parse_health(health, &package.repo_root)?))
}

#[tauri::command]
pub async fn start_warm_verification_daemon(repo_path: String) -> Result<WarmDaemonHealth, String> {
    let package = find_verify_package(&repo_path)?;
    let output = execute_verify(
        &package,
        &cli_args(&package.repo_root, &["daemon", "start"]),
        START_TIMEOUT,
    )
    .await?;
    let value = parse_json_output(&output)?;
    let health = response_payload(value, "health", "health")?;
    parse_health(health, &package.repo_root)
}

#[tauri::command]
pub async fn stop_warm_verification_daemon(repo_path: String) -> Result<WarmStopResponse, String> {
    let value = run_cli(&repo_path, &["daemon", "stop"], STOP_TIMEOUT).await?;
    let active_run_ids = response_payload(value, "shutdown_ack", "active_run_ids")?;
    let active_run_ids: Vec<String> = serde_json::from_value(active_run_ids)
        .map_err(|_| "Repository verifier returned invalid active run IDs".to_string())?;
    if active_run_ids.len() > 32 || active_run_ids.iter().any(|run_id| !valid_id(run_id)) {
        return Err("Repository verifier returned invalid active run IDs".into());
    }
    Ok(WarmStopResponse { active_run_ids })
}

#[tauri::command]
pub async fn run_warm_changed_verification(
    db: State<'_, DbState>,
    repo_path: String,
    detailed_capture: bool,
    run_id: String,
) -> Result<warm_verification::StoredWarmVerificationRun, String> {
    if !valid_id(&run_id) {
        return Err("Run identity is invalid".into());
    }
    let detailed = detailed_capture.then_some("--detailed");
    let mut command = vec![
        "changed",
        "--run-id",
        run_id.as_str(),
        "--timeout-ms",
        "30000",
    ];
    if let Some(detailed) = detailed {
        command.push(detailed);
    }
    let package = find_verify_package(&repo_path)?;
    let output = execute_verify(
        &package,
        &cli_args(&package.repo_root, &command),
        RUN_TIMEOUT,
    )
    .await?;
    let value = parse_json_output(&output)?;
    let result = response_payload(value, "verify_result", "result")?;
    if result.get("run_id").and_then(Value::as_str) != Some(run_id.as_str()) {
        return Err("Repository verifier returned a different run identity".into());
    }
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    warm_verification::persist_validated_run(&conn, &package.repo_root.to_string_lossy(), &result)
}

#[tauri::command]
pub async fn run_differential_verification(
    db: State<'_, DbState>,
    repo_path: String,
    run_id: String,
    reference_revision: String,
    candidate_kind: String,
    candidate_revision: Option<String>,
) -> Result<differential_verification::StoredDifferentialVerificationRun, String> {
    if !valid_id(&run_id) || !valid_bounded_text(&reference_revision) {
        return Err("Differential run identity or reference is invalid".into());
    }
    let mut command = vec![
        "differential",
        "run",
        "--run-id",
        run_id.as_str(),
        "--reference",
        reference_revision.as_str(),
    ];
    command.extend(differential_candidate_arguments(
        candidate_kind.as_str(),
        candidate_revision.as_deref(),
    )?);
    let package = find_verify_package(&repo_path)?;
    require_pnpm_differential(&package)?;
    let output = execute_verify(
        &package,
        &cli_args(&package.repo_root, &command),
        DIFFERENTIAL_RUN_TIMEOUT,
    )
    .await?;
    let value = parse_json_output(&output)?;
    let summary = response_payload(value, "differential_result", "summary")?;
    if summary.get("run_id").and_then(Value::as_str) != Some(run_id.as_str()) {
        return Err("Repository verifier returned a different differential run identity".into());
    }
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    differential_verification::persist_validated_run(
        &conn,
        &package.repo_root.to_string_lossy(),
        &summary,
    )
}

#[tauri::command]
pub async fn cleanup_differential_verification_artifacts(
    repo_path: String,
    dry_run: bool,
) -> Result<DifferentialCleanupSummary, String> {
    let command = if dry_run {
        vec!["differential", "cleanup", "--dry-run"]
    } else {
        vec!["differential", "cleanup"]
    };
    let value = run_differential_cli(&repo_path, &command, RUN_TIMEOUT).await?;
    let summary = validate_differential_cleanup(
        serde_json::from_value(response_payload(value, "differential_cleanup", "summary")?)
            .map_err(|_| "Repository verifier returned invalid differential cleanup data")?,
    )?;
    if summary.dry_run != dry_run {
        return Err("Repository verifier returned a different differential cleanup mode".into());
    }
    Ok(summary)
}

#[tauri::command]
pub async fn cancel_warm_verification_run(
    repo_path: String,
    run_id: String,
) -> Result<WarmCancelResponse, String> {
    if !valid_id(&run_id) {
        return Err("Run identity is invalid".into());
    }
    let value = run_cli(&repo_path, &["cancel", "--run-id", &run_id], STATUS_TIMEOUT).await?;
    let accepted = response_payload(value, "cancel_ack", "accepted")?
        .as_bool()
        .ok_or("Repository verifier returned an invalid cancellation acknowledgement")?;
    Ok(WarmCancelResponse { accepted })
}

#[tauri::command]
pub async fn cancel_differential_verification_run(
    repo_path: String,
    run_id: String,
) -> Result<WarmCancelResponse, String> {
    if !valid_id(&run_id) {
        return Err("Differential run identity is invalid".into());
    }
    let value = run_differential_cli(
        &repo_path,
        &["differential", "cancel", "--run-id", &run_id],
        STATUS_TIMEOUT,
    )
    .await?;
    let summary = validate_differential_status(
        serde_json::from_value(response_payload(value, "differential_status", "summary")?)
            .map_err(|_| "Repository verifier returned invalid differential cancellation")?,
    )?;
    if summary.run_id != run_id {
        return Err(
            "Repository verifier returned a different differential cancellation identity".into(),
        );
    }
    Ok(WarmCancelResponse {
        accepted: summary.state != "not_found",
    })
}

#[tauri::command]
pub async fn cleanup_warm_verification_artifacts(
    repo_path: String,
    dry_run: bool,
) -> Result<WarmVerificationCleanupReport, String> {
    let command = if dry_run {
        vec!["cleanup", "--dry-run"]
    } else {
        vec!["cleanup"]
    };
    let value = run_cli(&repo_path, &command, STOP_TIMEOUT).await?;
    let report: WarmVerificationCleanupReport = serde_json::from_value(value)
        .map_err(|_| "Repository verifier returned an invalid cleanup report".to_string())?;
    if report.schema_version != 1 {
        return Err("Repository verifier cleanup schema is unsupported".into());
    }
    Ok(report)
}

#[tauri::command]
pub async fn get_current_warm_verification_identity(
    repo_path: String,
) -> Result<CurrentWarmVerificationIdentity, String> {
    let value = run_cli(&repo_path, &["current"], STOP_TIMEOUT).await?;
    let identity: CurrentWarmVerificationIdentity = serde_json::from_value(value)
        .map_err(|_| "Repository verifier returned an invalid current identity".to_string())?;
    if identity.schema_version != 1
        || !matches!(
            identity.change_set_kind.as_str(),
            "worktree" | "staged" | "commit" | "range"
        )
        || !valid_hash(&identity.target_sha, 40, 64)
        || !valid_hash(&identity.change_set_identity, 64, 64)
        || !valid_hash(&identity.config_hash, 64, 64)
        || !valid_hash(&identity.manifest_hash, 64, 64)
        || !valid_hash(&identity.source_hash, 64, 64)
        || !valid_id(&identity.observation_policy_profile_id)
    {
        return Err("Repository verifier current identity contract is invalid".into());
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, contents).expect("write");
    }

    fn fixture() -> TempDir {
        let temp = tempfile::tempdir().expect("temp");
        write(
            &temp.path().join("package.json"),
            r#"{"private":true,"workspaces":["apps/*"]}"#,
        );
        write(
            &temp.path().join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n",
        );
        write(
            &temp.path().join("apps/web/package.json"),
            r#"{"name":"web","scripts":{"verify":"node verify.js"}}"#,
        );
        temp
    }

    #[test]
    fn locates_one_workspace_verify_script_and_lockfile_manager() {
        let temp = fixture();
        let found = find_verify_package(temp.path().to_str().expect("path")).expect("package");
        assert_eq!(
            found.package_root,
            temp.path()
                .join("apps/web")
                .canonicalize()
                .expect("canonical")
        );
        assert_eq!(found.manager, PackageManager::Pnpm);
        assert_eq!(
            found
                .manager
                .arguments(&["current".into(), "--json".into()]),
            ["--silent", "run", "verify", "current", "--json"]
        );
    }

    #[test]
    fn locator_fails_closed_for_missing_or_ambiguous_scripts() {
        let temp = fixture();
        write(
            &temp.path().join("apps/admin/package.json"),
            r#"{"name":"admin","scripts":{"verify":"node verify.js"}}"#,
        );
        let error =
            find_verify_package(temp.path().to_str().expect("path")).expect_err("ambiguous");
        assert!(error.contains("found 2"));
        fs::remove_file(temp.path().join("apps/admin/package.json")).expect("remove");
        fs::remove_file(temp.path().join("apps/web/package.json")).expect("remove");
        let error = find_verify_package(temp.path().to_str().expect("path")).expect_err("missing");
        assert!(error.contains("found 0"));
        assert!(error.contains("remediation") || error.contains("Add one workspace"));
    }

    #[test]
    fn differential_candidate_arguments_are_exact_and_bounded() {
        assert!(differential_candidate_arguments("worktree", None)
            .expect("worktree")
            .is_empty());
        assert_eq!(
            differential_candidate_arguments("staged", None).expect("staged"),
            ["--staged"]
        );
        assert_eq!(
            differential_candidate_arguments("commit", Some("main~1")).expect("commit"),
            ["--commit", "main~1"]
        );
        assert_eq!(
            differential_candidate_arguments("range", Some("main..HEAD")).expect("range"),
            ["--range", "main..HEAD"]
        );
        assert!(differential_candidate_arguments("commit", None).is_err());
        assert!(differential_candidate_arguments("remote", None).is_err());
    }

    #[test]
    fn differential_commands_reject_non_pnpm_packages_without_narrowing_warm_verification() {
        let temp = fixture();
        let package = VerifyPackage {
            repo_root: temp.path().to_path_buf(),
            package_root: temp.path().join("apps/web"),
            manager: PackageManager::Npm,
        };
        let error = require_pnpm_differential(&package).expect_err("npm must be rejected");
        assert!(error.contains("supports pnpm repositories only"));
        assert_eq!(package.manager.arguments(&["current".into()])[3], "--");
    }

    #[test]
    fn locator_deduplicates_overlapping_workspace_patterns() {
        let temp = fixture();
        write(
            &temp.path().join("package.json"),
            r#"{"private":true,"workspaces":["apps/*","apps/web"]}"#,
        );
        let found = find_verify_package(temp.path().to_str().expect("path")).expect("package");
        assert_eq!(
            found.package_root,
            temp.path()
                .join("apps/web")
                .canonicalize()
                .expect("canonical")
        );
    }

    #[test]
    fn locator_falls_back_to_a_root_verify_script() {
        let temp = fixture();
        fs::remove_file(temp.path().join("apps/web/package.json")).expect("remove workspace");
        write(
            &temp.path().join("package.json"),
            r#"{"private":true,"workspaces":["apps/*"],"scripts":{"verify":"node verify.js"}}"#,
        );
        let found = find_verify_package(temp.path().to_str().expect("path")).expect("package");
        assert_eq!(
            found.package_root,
            temp.path().canonicalize().expect("root")
        );
    }

    #[cfg(unix)]
    #[test]
    fn locator_rejects_workspace_symlinks_before_reading_outside_manifests() {
        use std::os::unix::fs::symlink;

        let temp = fixture();
        let outside = tempfile::tempdir().expect("outside");
        write(&outside.path().join("package.json"), "not-json");
        write(
            &temp.path().join("package.json"),
            r#"{"private":true,"workspaces":["linked"]}"#,
        );
        symlink(outside.path(), temp.path().join("linked")).expect("symlink");
        let error = find_verify_package(temp.path().to_str().expect("path")).expect_err("escape");
        assert_eq!(error, "Verifier workspace resolves outside the repository");
    }

    #[test]
    fn health_parser_rejects_incomplete_or_wrong_repository_payloads() {
        let temp = fixture();
        assert!(parse_health(serde_json::json!({ "schema_version": 1 }), temp.path()).is_err());
        let health = serde_json::json!({
            "schema_version": 1,
            "daemon_pid": 42,
            "daemon_start_identity": "42:start",
            "target_root": temp.path(),
            "target_sha": "a".repeat(40),
            "config_hash": "b".repeat(64),
            "chromium_revision": "123",
            "cold_startup_ms": 1000.0,
            "warm": true,
            "server": {
                "kind": "process", "state": "ready", "owned": true, "pid": 43,
                "start_identity": "43:start", "restart_attempts": 0, "last_exit": null
            },
            "browser": {
                "kind": "browser", "state": "ready", "owned": true, "pid": null,
                "start_identity": "playwright:1", "restart_attempts": 0, "last_exit": null
            },
            "active_run_ids": [],
            "resources": {
                "rss_bytes": 1, "heap_used_bytes": 1, "active_contexts": 0,
                "retained_artifact_bytes": 0
            },
            "checked_at": "2026-07-15T00:00:00Z"
        });
        let canonical_root = temp.path().canonicalize().expect("canonical root");
        assert!(parse_health(health.clone(), &canonical_root).is_ok());
        let other = tempfile::tempdir().expect("other");
        assert!(parse_health(health, other.path()).is_err());
    }

    #[test]
    fn parser_rejects_trailing_or_unversioned_process_output() {
        let trailing = ProcessOutput {
            success: true,
            status_code: Some(0),
            stdout: br#"{"type":"health","health":{"schema_version":1}} trailing"#.to_vec(),
            stderr: Vec::new(),
        };
        assert!(parse_json_output(&trailing).is_err());
        let error = ProcessOutput {
            success: false,
            status_code: Some(3),
            stdout: br#"{"type":"error","error":{"code":"config_invalid","message":"bad config","retryable":false}}"#.to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(
            parse_json_output(&error).expect_err("error"),
            "config_invalid: bad config"
        );

        let regression = ProcessOutput {
            success: false,
            status_code: Some(2),
            stdout: br#"{"type":"verify_result","result":{"run_id":"run-1"}}"#.to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(
            parse_json_output(&regression).expect("regression result")["type"],
            "verify_result"
        );
        let incomparable_prepare = ProcessOutput {
            success: false,
            status_code: Some(3),
            stdout: br#"{"type":"differential_prepared","summary":{"run_id":"run-1","status":"incomparable"}}"#.to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(
            parse_json_output(&incomparable_prepare).expect("incomparable preparation")["type"],
            "differential_prepared"
        );
    }

    #[tokio::test]
    async fn process_runner_uses_argv_and_bounded_output_without_a_shell() {
        let temp = fixture();
        let bin = temp.path().join("bin");
        fs::create_dir_all(&bin).expect("bin");
        let executable = bin.join("pnpm");
        write(
            &executable,
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s|' \"$arg\"; done\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("chmod");
        }
        let package = find_verify_package(temp.path().to_str().expect("path")).expect("package");
        let output = execute_verify_program(
            &package,
            &[
                "current".into(),
                "--repo".into(),
                "path with spaces".into(),
                "--json".into(),
            ],
            Duration::from_secs(2),
            &executable,
        )
        .await
        .expect("execute");
        assert!(output.success);
        assert_eq!(
            String::from_utf8(output.stdout).expect("utf8"),
            "--silent|run|verify|current|--repo|path with spaces|--json|"
        );
    }
}
