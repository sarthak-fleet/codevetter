use serde_json::{json, Value};
use std::process::Command as StdCommand;

/// Helper: check if `gh` CLI is available and authenticated.
fn ensure_gh_available() -> Result<(), String> {
    let output = StdCommand::new("which")
        .arg("gh")
        .output()
        .map_err(|e| format!("Failed to check for gh CLI: {e}"))?;

    if !output.status.success() {
        return Err(
            "GitHub CLI (gh) is not installed. Install it from https://cli.github.com".to_string(),
        );
    }

    // Quick auth check
    let auth = StdCommand::new("gh")
        .args(["auth", "status"])
        .output()
        .map_err(|e| format!("Failed to check gh auth: {e}"))?;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&auth.stdout),
        String::from_utf8_lossy(&auth.stderr)
    );

    if !auth.status.success() && !combined.contains("Logged in to") {
        return Err("GitHub CLI is not authenticated. Run `gh auth login` first.".to_string());
    }

    Ok(())
}

/// Create a pull request using `gh pr create`.
#[tauri::command]
pub async fn create_pull_request(
    repo_path: String,
    title: String,
    body: String,
    base_branch: String,
    head_branch: String,
) -> Result<Value, String> {
    ensure_gh_available()?;

    let output = StdCommand::new("gh")
        .args([
            "pr",
            "create",
            "--title",
            &title,
            "--body",
            &body,
            "--base",
            &base_branch,
            "--head",
            &head_branch,
            "--json",
            "number,url",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh pr create: {e}"))?;

    if !output.status.success() {
        // gh pr create may not support --json flag. Try without --json as fallback.
        let fallback = StdCommand::new("gh")
            .args([
                "pr",
                "create",
                "--title",
                &title,
                "--body",
                &body,
                "--base",
                &base_branch,
                "--head",
                &head_branch,
            ])
            .current_dir(&repo_path)
            .output()
            .map_err(|e| format!("Failed to run gh pr create (fallback): {e}"))?;

        if !fallback.status.success() {
            let fallback_stderr = String::from_utf8_lossy(&fallback.stderr);
            return Err(format!("gh pr create failed: {fallback_stderr}"));
        }

        let url = String::from_utf8_lossy(&fallback.stdout).trim().to_string();
        // Extract PR number from URL: https://github.com/owner/repo/pull/123
        let number = url
            .rsplit('/')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);

        return Ok(json!({
            "url": url,
            "number": number,
            "html_url": url,
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse PR response: {e}"))?;

    let url = parsed
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let number = parsed.get("number").and_then(|v| v.as_i64()).unwrap_or(0);

    Ok(json!({
        "url": url,
        "number": number,
        "html_url": url,
    }))
}

/// List pull requests for the repo. Supports filtering by state (open/closed/merged/all).
#[tauri::command]
pub async fn list_pull_requests_for_repo(
    repo_path: String,
    state: Option<String>,
) -> Result<Value, String> {
    ensure_gh_available()?;

    let state_val = state.as_deref().unwrap_or("open");

    let output = StdCommand::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            state_val,
            "--json",
            "number,title,state,url,headRefName,baseRefName,createdAt,author",
            "--limit",
            "20",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh pr list: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr list failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let prs: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse PR list: {e}"))?;

    Ok(json!({ "prs": prs }))
}

/// Get detailed information about a specific pull request.
#[tauri::command]
pub async fn get_pull_request(repo_path: String, pr_number: i64) -> Result<Value, String> {
    ensure_gh_available()?;

    let output = StdCommand::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "number,title,body,state,url,headRefName,baseRefName,mergeable,reviewDecision,statusCheckRollup,createdAt,author",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh pr view: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr view failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let pr: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse PR details: {e}"))?;

    Ok(pr)
}

/// Merge a pull request with the specified method (squash, merge, or rebase).
#[tauri::command]
pub async fn merge_pull_request(
    repo_path: String,
    pr_number: i64,
    method: String,
) -> Result<Value, String> {
    ensure_gh_available()?;

    let method_flag = match method.as_str() {
        "squash" => "--squash",
        "rebase" => "--rebase",
        "merge" => "--merge",
        _ => {
            return Err(format!(
                "Invalid merge method: {method}. Use squash, merge, or rebase."
            ))
        }
    };

    let output = StdCommand::new("gh")
        .args(["pr", "merge", &pr_number.to_string(), method_flag])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh pr merge: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh pr merge failed: {stderr}"));
    }

    Ok(json!({ "success": true }))
}

/// List CI check runs for a pull request.
#[tauri::command]
pub async fn list_ci_checks(repo_path: String, pr_number: i64) -> Result<Value, String> {
    ensure_gh_available()?;

    let output = StdCommand::new("gh")
        .args([
            "pr",
            "checks",
            &pr_number.to_string(),
            "--json",
            "name,state,conclusion,startedAt,completedAt,detailsUrl",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh pr checks: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // gh pr checks may fail if there are no checks — return empty list
        if stderr.contains("no checks") || stderr.contains("no status checks") {
            return Ok(json!({ "checks": [] }));
        }
        return Err(format!("gh pr checks failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let checks: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("Failed to parse checks: {e}"))?;

    Ok(json!({ "checks": checks }))
}

/// Re-run failed CI checks for a pull request's head commit.
/// Uses `gh run rerun --failed` for the latest workflow run on the PR's head.
#[tauri::command]
pub async fn rerun_failed_checks(repo_path: String, pr_number: i64) -> Result<Value, String> {
    ensure_gh_available()?;

    // First, get the PR's head commit SHA
    let pr_output = StdCommand::new("gh")
        .args(["pr", "view", &pr_number.to_string(), "--json", "headRefOid"])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to get PR head: {e}"))?;

    if !pr_output.status.success() {
        let stderr = String::from_utf8_lossy(&pr_output.stderr);
        return Err(format!("Failed to get PR head commit: {stderr}"));
    }

    let pr_json: Value = serde_json::from_str(&String::from_utf8_lossy(&pr_output.stdout))
        .map_err(|e| format!("Failed to parse PR head: {e}"))?;

    let head_sha = pr_json
        .get("headRefOid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Could not determine PR head commit".to_string())?;

    // List runs for that commit and re-run failed ones
    let runs_output = StdCommand::new("gh")
        .args([
            "run",
            "list",
            "--commit",
            head_sha,
            "--json",
            "databaseId,conclusion",
            "--limit",
            "10",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to list runs: {e}"))?;

    if !runs_output.status.success() {
        let stderr = String::from_utf8_lossy(&runs_output.stderr);
        return Err(format!("Failed to list workflow runs: {stderr}"));
    }

    let runs: Value = serde_json::from_str(&String::from_utf8_lossy(&runs_output.stdout))
        .map_err(|e| format!("Failed to parse runs: {e}"))?;

    let mut rerun_count = 0;

    if let Some(arr) = runs.as_array() {
        for run in arr {
            let conclusion = run.get("conclusion").and_then(|v| v.as_str()).unwrap_or("");
            let run_id = run.get("databaseId").and_then(|v| v.as_i64()).unwrap_or(0);

            if conclusion == "failure" && run_id > 0 {
                let rerun = StdCommand::new("gh")
                    .args(["run", "rerun", &run_id.to_string(), "--failed"])
                    .current_dir(&repo_path)
                    .output();

                if let Ok(out) = rerun {
                    if out.status.success() {
                        rerun_count += 1;
                    }
                }
            }
        }
    }

    Ok(json!({
        "success": true,
        "rerun_count": rerun_count,
    }))
}
