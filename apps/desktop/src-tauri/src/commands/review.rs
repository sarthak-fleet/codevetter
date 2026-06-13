use crate::db::queries::{self, ActivityInput, LocalReviewInput};
use crate::talk;
use crate::DbState;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};
use tauri::{Emitter, Manager, State};

/// Resolve a CLI binary (e.g. "claude", "gemini") to an absolute path.
///
/// Tauri apps on macOS don't always inherit the user's shell PATH — especially
/// when launched outside a terminal — so `StdCommand::new("claude")` can fail
/// with ENOENT even though the user has it installed. This helper walks the
/// usual user-install locations (asdf shims, bun, pnpm, npm global, homebrew,
/// `~/.local/bin`) and returns the first match. Falls back to the bare name
/// so the existing PATH lookup still runs if none match.
fn resolve_cli_path(name: &str) -> String {
    // First, honor PATH if it works
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }

    // Common user-install locations not always in PATH of GUI-launched apps
    if let Ok(home) = std::env::var("HOME") {
        let fallbacks = [
            format!("{home}/.local/bin/{name}"),
            format!("{home}/.bun/bin/{name}"),
            format!("{home}/.asdf/shims/{name}"),
            format!("{home}/Library/pnpm/{name}"),
            format!("{home}/.nvm/versions/node/current/bin/{name}"),
            format!("/opt/homebrew/bin/{name}"),
            format!("/usr/local/bin/{name}"),
        ];
        for c in fallbacks {
            if std::path::Path::new(&c).is_file() {
                return c;
            }
        }
    }

    // Give up — let Command::new fail with its usual error
    name.to_string()
}

fn read_repo_conventions(repo_path: &str) -> String {
    const BUDGET: usize = 16 * 1024;
    let candidates = ["CLAUDE.md", "agents.md", "AGENTS.md"];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut parts: Vec<String> = Vec::new();
    let mut budget = BUDGET;

    for name in candidates {
        let path = std::path::Path::new(repo_path).join(name);
        if !path.is_file() {
            continue;
        }
        let key = name.to_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        let take = trimmed.len().min(budget);
        if take == 0 {
            break;
        }
        let mut end = take;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        let slice = &trimmed[..end];
        parts.push(format!("### {name}\n{slice}"));
        budget = budget.saturating_sub(end);
        if budget == 0 {
            break;
        }
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(
            "\nRepo conventions (authoritative — findings that contradict these should be dropped):\n{}\n",
            parts.join("\n\n")
        )
    }
}

/// Look up the latest talk for this project and prepend it as context if fresh enough.
fn maybe_prepend_talk_context(
    conn: &rusqlite::Connection,
    project_path: &str,
    prompt: &str,
) -> String {
    if let Ok(Some(t)) = queries::get_latest_talk_for_project(conn, project_path) {
        // Check staleness
        if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&t.created_at) {
            let age = chrono::Utc::now().signed_duration_since(created);
            if age.num_seconds() <= talk::STALENESS_SECS {
                let context = talk::render_talk_for_prompt(&t);
                return format!("{context}\n\n{prompt}");
            }
        }
    }
    prompt.to_string()
}

#[derive(Debug, Clone, Copy)]
struct ReviewSpecialist {
    id: &'static str,
    name: &'static str,
    focus: &'static str,
    checks: &'static [&'static str],
}

const PRODUCT_SAFETY_SPECIALIST: ReviewSpecialist = ReviewSpecialist {
    id: "product-safety",
    name: "Product Safety",
    focus: "User-facing regressions, broken flows, data loss, confusing states, and behavior changes that violate the described goal.",
    checks: &[
        "Flag behavior changes that can break an existing user workflow.",
        "Check loading, empty, error, and permission states for touched user-facing screens.",
        "Prefer concrete reproduction paths over style commentary.",
    ],
};

const SECURITY_BOUNDARY_SPECIALIST: ReviewSpecialist = ReviewSpecialist {
    id: "security-boundary",
    name: "Security Boundary",
    focus: "Auth, authorization, secret handling, trust boundaries, injection, shell/network execution, and unsafe IPC or persistence boundaries.",
    checks: &[
        "Verify server-side or backend enforcement, not just hidden client controls.",
        "Flag secrets, tokens, PII, prompts, or credentials that can leak into logs, storage, analytics, or model calls.",
        "Check untrusted input before database, shell, filesystem, network, IPC, or model calls.",
    ],
};

const AGENT_HANDOFF_SPECIALIST: ReviewSpecialist = ReviewSpecialist {
    id: "agent-handoff",
    name: "Agent Handoff",
    focus: "Agent-written-code failure modes: over-editing, silent scope drift, missing verification, fake-green summaries, brittle fixes, and incomplete handoff.",
    checks: &[
        "Call out missing tests or verification commands the next agent must run.",
        "Prefer findings with file paths, line numbers, and a bounded fix.",
        "Separate real blockers from optional cleanup so agents do not waste context.",
    ],
};

const GENERAL_REVIEW_SPECIALIST: ReviewSpecialist = ReviewSpecialist {
    id: "general",
    name: "General Code Review",
    focus: "Correctness, security, regression risk, broken contracts, and changed behavior that can realistically break the described goal.",
    checks: &[
        "Find real correctness, security, and regression bugs.",
        "Use repo conventions, blast radius, and history context before reporting.",
        "Skip style-only or speculative findings.",
    ],
};

#[derive(Debug, Clone)]
struct ReviewPlan {
    tier: &'static str,
    mode: &'static str,
    changed_lines: usize,
    sensitive_paths: Vec<String>,
    specialists: Vec<ReviewSpecialist>,
    uses_coordinator: bool,
}

fn changed_line_count(diff: &str) -> usize {
    diff.lines()
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++ "))
                || (line.starts_with('-') && !line.starts_with("--- "))
        })
        .count()
}

fn is_sensitive_review_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let sensitive_terms = [
        "auth",
        "login",
        "session",
        "token",
        "secret",
        "credential",
        "password",
        "permission",
        "rbac",
        "acl",
        "migration",
        "schema",
        "sql",
        "ipc",
        "invoke",
        "command",
        "shell",
        "exec",
        "network",
        "webhook",
        "billing",
        "payment",
    ];
    sensitive_terms.iter().any(|term| lower.contains(term))
}

fn build_review_plan(diff: &str, changed_files: &[String]) -> ReviewPlan {
    let changed_lines = changed_line_count(diff);
    let sensitive_paths: Vec<String> = changed_files
        .iter()
        .filter(|path| is_sensitive_review_path(path))
        .cloned()
        .collect();

    if changed_lines <= 10 && sensitive_paths.is_empty() {
        return ReviewPlan {
            tier: "trivial",
            mode: "single-pass",
            changed_lines,
            sensitive_paths,
            specialists: vec![GENERAL_REVIEW_SPECIALIST],
            uses_coordinator: false,
        };
    }

    if changed_lines <= 100 && sensitive_paths.is_empty() {
        return ReviewPlan {
            tier: "lite",
            mode: "specialist-lite",
            changed_lines,
            sensitive_paths,
            specialists: vec![PRODUCT_SAFETY_SPECIALIST, AGENT_HANDOFF_SPECIALIST],
            uses_coordinator: false,
        };
    }

    ReviewPlan {
        tier: if sensitive_paths.is_empty() {
            "full"
        } else {
            "full-sensitive"
        },
        mode: "specialist-full",
        changed_lines,
        sensitive_paths,
        specialists: vec![
            PRODUCT_SAFETY_SPECIALIST,
            SECURITY_BOUNDARY_SPECIALIST,
            AGENT_HANDOFF_SPECIALIST,
        ],
        uses_coordinator: true,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ReviewMemoryGraphNode {
    id: String,
    kind: String,
    label: String,
    file_path: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ReviewMemoryGraphEdge {
    from: String,
    to: String,
    kind: String,
    confidence: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct ReviewMemoryGraph {
    schema_version: i64,
    scope: String,
    nodes: Vec<ReviewMemoryGraphNode>,
    edges: Vec<ReviewMemoryGraphEdge>,
    truncated: bool,
}

fn graph_node_id(kind: &str, value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug = slug.trim_matches('-').chars().take(90).collect();
    if slug.is_empty() {
        format!("{kind}-unknown")
    } else {
        format!("{kind}-{slug}")
    }
}

fn push_graph_node(nodes: &mut Vec<ReviewMemoryGraphNode>, node: ReviewMemoryGraphNode) {
    if !nodes.iter().any(|existing| existing.id == node.id) {
        nodes.push(node);
    }
}

fn push_graph_edge(edges: &mut Vec<ReviewMemoryGraphEdge>, edge: ReviewMemoryGraphEdge) {
    if !edges.iter().any(|existing| {
        existing.from == edge.from && existing.to == edge.to && existing.kind == edge.kind
    }) {
        edges.push(edge);
    }
}

fn build_review_memory_graph(
    changed_files: &[String],
    evidence_candidates: &[crate::commands::evidence_pattern::EvidenceCandidate],
    procedure_steps: &[crate::commands::evidence_pattern::EvidenceProcedureStep],
    history_section: &str,
    blast_section: &str,
) -> ReviewMemoryGraph {
    const MAX_FILE_NODES: usize = 12;
    const MAX_TOTAL_NODES: usize = 28;
    const MAX_EDGES: usize = 64;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut truncated = changed_files.len() > MAX_FILE_NODES;

    for file in changed_files.iter().take(MAX_FILE_NODES) {
        push_graph_node(
            &mut nodes,
            ReviewMemoryGraphNode {
                id: graph_node_id("file", file),
                kind: "file".to_string(),
                label: file.clone(),
                file_path: Some(file.clone()),
                detail: Some("changed file".to_string()),
            },
        );
    }

    if !history_section.trim().is_empty() {
        push_graph_node(
            &mut nodes,
            ReviewMemoryGraphNode {
                id: "history-context".to_string(),
                kind: "history_context".to_string(),
                label: "Prior commits, decisions, agents, and command evidence".to_string(),
                file_path: None,
                detail: Some(format!("{} chars in prompt section", history_section.len())),
            },
        );
        for file in changed_files.iter().take(MAX_FILE_NODES) {
            push_graph_edge(
                &mut edges,
                ReviewMemoryGraphEdge {
                    from: graph_node_id("file", file),
                    to: "history-context".to_string(),
                    kind: "has_history_context".to_string(),
                    confidence: 0.74,
                },
            );
        }
    }

    if !blast_section.trim().is_empty() {
        push_graph_node(
            &mut nodes,
            ReviewMemoryGraphNode {
                id: "blast-radius".to_string(),
                kind: "blast_radius".to_string(),
                label: "Blast-radius summary".to_string(),
                file_path: None,
                detail: Some("computed from repo relationships".to_string()),
            },
        );
        for file in changed_files.iter().take(MAX_FILE_NODES) {
            push_graph_edge(
                &mut edges,
                ReviewMemoryGraphEdge {
                    from: graph_node_id("file", file),
                    to: "blast-radius".to_string(),
                    kind: "has_blast_radius".to_string(),
                    confidence: 0.68,
                },
            );
        }
    }

    for candidate in evidence_candidates.iter().take(8) {
        let candidate_id = graph_node_id("candidate", &candidate.id);
        push_graph_node(
            &mut nodes,
            ReviewMemoryGraphNode {
                id: candidate_id.clone(),
                kind: "evidence_candidate".to_string(),
                label: candidate.id.clone(),
                file_path: candidate.affected_files.first().cloned(),
                detail: Some(format!(
                    "{} · confidence {:.2}",
                    candidate.kind, candidate.confidence
                )),
            },
        );
        for file in candidate.affected_files.iter().take(4) {
            let file_id = graph_node_id("file", file);
            push_graph_node(
                &mut nodes,
                ReviewMemoryGraphNode {
                    id: file_id.clone(),
                    kind: "file".to_string(),
                    label: file.clone(),
                    file_path: Some(file.clone()),
                    detail: Some("candidate-affected file".to_string()),
                },
            );
            push_graph_edge(
                &mut edges,
                ReviewMemoryGraphEdge {
                    from: file_id,
                    to: candidate_id.clone(),
                    kind: "raises_candidate".to_string(),
                    confidence: candidate.confidence,
                },
            );
        }
    }

    for step in procedure_steps.iter().take(8) {
        let step_id = graph_node_id("gate", &step.id);
        push_graph_node(
            &mut nodes,
            ReviewMemoryGraphNode {
                id: step_id.clone(),
                kind: "procedure_gate".to_string(),
                label: step.id.clone(),
                file_path: None,
                detail: Some(format!("{} · {}", step.status, step.gate)),
            },
        );
        for candidate_id in step.candidate_ids.iter().take(4) {
            push_graph_edge(
                &mut edges,
                ReviewMemoryGraphEdge {
                    from: graph_node_id("candidate", candidate_id),
                    to: step_id.clone(),
                    kind: "requires_gate".to_string(),
                    confidence: 0.86,
                },
            );
        }
    }

    if nodes.len() > MAX_TOTAL_NODES {
        nodes.truncate(MAX_TOTAL_NODES);
        truncated = true;
    }
    if edges.len() > MAX_EDGES {
        edges.truncate(MAX_EDGES);
        truncated = true;
    }

    ReviewMemoryGraph {
        schema_version: 1,
        scope: "review_changed_files".to_string(),
        nodes,
        edges,
        truncated,
    }
}

fn render_review_memory_graph_for_prompt(graph: &ReviewMemoryGraph) -> String {
    if graph.nodes.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(
        "\nChanged-file graph neighborhood (local review memory, not ground truth):".to_string(),
    );
    for node in graph.nodes.iter().take(14) {
        let detail = node
            .detail
            .as_ref()
            .map(|value| format!(" — {value}"))
            .unwrap_or_default();
        let path = node
            .file_path
            .as_ref()
            .filter(|path| *path != &node.label)
            .map(|path| format!(" ({path})"))
            .unwrap_or_default();
        lines.push(format!(
            "- [{}] {}{}{}",
            node.kind, node.label, path, detail
        ));
    }
    for edge in graph.edges.iter().take(16) {
        lines.push(format!(
            "  edge: {} -> {} [{} {:.2}]",
            edge.from, edge.to, edge.kind, edge.confidence
        ));
    }
    if graph.truncated {
        lines.push("- graph truncated; inspect source files for complete context".to_string());
    }
    lines.push(String::new());
    lines.join("\n")
}

fn value_string<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn value_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn value_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
}

fn value_array_len(value: &Value, keys: &[&str]) -> usize {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(Vec::len)
        .unwrap_or(0)
}

fn compact_prompt_text(value: &str, limit: usize) -> String {
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = normalized.chars().take(limit).collect::<String>();
    if normalized.chars().count() > limit {
        out.push_str("...");
    }
    out
}

fn render_qa_evidence_for_prompt(qa_runs: &[Value]) -> String {
    if qa_runs.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push("\nRecent synthetic user QA evidence (runtime proof from prior runs):".to_string());
    for run in qa_runs.iter().take(5) {
        let pass = value_bool(run, &["pass"]).unwrap_or(false);
        let status = if pass { "PASS" } else { "FAIL" };
        let runner = value_string(run, &["runner_type", "runnerType"]).unwrap_or("unknown");
        let route = value_string(run, &["route"]).unwrap_or("(route unknown)");
        let goal = value_string(run, &["goal"]).unwrap_or("(goal missing)");
        let duration = value_u64(run, &["duration_ms", "durationMs"]).unwrap_or(0);
        let console_errors = value_u64(run, &["console_errors", "consoleErrors"])
            .unwrap_or_else(|| value_array_len(run, &["console_errors", "consoleErrors"]) as u64);
        let artifact_count = value_array_len(run, &["artifacts"]);
        let primary_artifact = value_string(run, &["screenshot_path", "screenshotPath"])
            .or_else(|| {
                run.get("artifacts")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
            })
            .unwrap_or("");
        let notes = value_string(run, &["notes"])
            .map(|value| compact_prompt_text(value, 180))
            .unwrap_or_default();

        let mut parts = vec![
            format!("runner={runner}"),
            format!("route={route}"),
            format!("duration_ms={duration}"),
            format!("console_errors={console_errors}"),
            format!("artifacts={artifact_count}"),
        ];
        if !primary_artifact.is_empty() {
            parts.push(format!(
                "artifact={}",
                compact_prompt_text(primary_artifact, 160)
            ));
        }
        lines.push(format!(
            "- {status}: {} [{}]",
            compact_prompt_text(goal, 140),
            parts.join("; ")
        ));
        if !notes.is_empty() {
            lines.push(format!("  note: {notes}"));
        }
    }
    lines.push(
        "Use QA failures as runtime evidence, but distinguish app failures from runner/setup failures."
            .to_string(),
    );
    lines.push(String::new());
    lines.join("\n")
}

fn build_specialist_block(specialist: &ReviewSpecialist, plan: &ReviewPlan) -> String {
    let checks = specialist
        .checks
        .iter()
        .map(|check| format!("- {check}"))
        .collect::<Vec<_>>()
        .join("\n");
    let sensitive = if plan.sensitive_paths.is_empty() {
        String::new()
    } else {
        format!(
            "\nSensitive paths forcing full review:\n{}\n",
            plan.sensitive_paths
                .iter()
                .map(|path| format!("- {path}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    format!(
        r#"Review tier: {tier}
Review mode: {mode}
Changed lines: {changed_lines}
Specialist pass: {name} ({id})
Specialist focus: {focus}
Specialist checks:
{checks}{sensitive}
Report only issues in this specialist's scope unless a critical cross-scope defect is obvious."#,
        tier = plan.tier,
        mode = plan.mode,
        changed_lines = plan.changed_lines,
        name = specialist.name,
        id = specialist.id,
        focus = specialist.focus,
        checks = checks,
        sensitive = sensitive,
    )
}

fn build_review_prompt(
    project_description: &str,
    change_description: &str,
    conventions_section: &str,
    files_section: &str,
    blast_section: &str,
    history_section: &str,
    graph_section: &str,
    qa_section: &str,
    evidence_section: &str,
    procedure_section: &str,
    specialist_block: &str,
    diff_text: &str,
) -> String {
    format!(
        r#"You are a senior code reviewer for an experienced engineer. Find *real* issues — security holes, correctness bugs, regression risk, broken contracts. Skip style nitpicks and speculative concerns.

Project: {project_description}
Change: {change_description}
{specialist_block}
{conventions_section}{files_section}{blast_section}{history_section}{graph_section}{qa_section}{evidence_section}{procedure_section}
How to review:
1. Read the diff carefully. You have file-read tools — use them when a finding's validity depends on context the diff doesn't show (callers, tests, related files, imports, prior implementation).
2. Verify each potential issue against the actual code before reporting. If you cannot cite specific lines that prove the problem, drop the finding — or, if the signal is real but unverified, lower confidence honestly instead of hiding the uncertainty.
3. Use the blast-radius data above to weight severity: a behavior change to a symbol with 6+ callers should be at least medium severity unless the change is provably backward-compatible.
4. Skip nitpicks (formatting, naming preference, missing comments) unless they will cause real bugs or break a workflow.
5. Repo conventions above are authoritative. Drop findings that contradict them.
6. History signals (if present) explain prior commits and agent work on the touched files — use them to understand *intent* and avoid re-flagging deliberate past decisions. Only call out if the new diff re-opens an old problem.
7. Changed-file graph neighborhoods (if present) are local memory edges for navigation. Treat inferred edges as leads; verify against source before making a finding.
8. Synthetic QA evidence (if present) is runtime evidence from prior user-flow runs. Use failures to focus review, but do not confuse runner/setup failures with app bugs.
9. Ranked evidence candidates (if present) are deterministic search leads, not conclusions. Validate them against code/evidence, reject them if wrong, and preserve any remaining open questions in the summary or finding suggestion.
10. Procedure steps (if present) are explicit evidence gates. Treat blocked steps as remaining work unless the current code/evidence resolves the gate.

Output format:

Think through the review first (you may use tools and write reasoning notes). Then output **exactly one** ```json fenced block as the very LAST thing in your response, matching this shape. Do not emit any other ```json fenced blocks anywhere — examples in your reasoning should be unfenced or use a different language tag.

JSON shape (literal text, not a fenced example):
{{"findings":[{{"severity":"critical|high|medium|low","title":"...","summary":"... — include the specific lines that prove the problem","suggestion":"...","filePath":"...","line":42,"confidence":0.9}}],"score":75,"summary":"Overall assessment","talk":{{"files_read":["src/file.ts"],"files_modified":[],"actions_summary":"What you reviewed and found","unfinished_work":null,"key_decisions":"Important observations about the code","recommended_next_steps":"What should happen next"}}}}

Rules:
- severity must be one of: critical, high, medium, low
- confidence is 0.0-1.0 — be honest; downgrade rather than overclaim
- line is optional (null if unknown); filePath relative to repo root
- score is 0-100 (100 = perfect)
- Each finding's `summary` must reference the specific line(s) or symbol(s) that prove the problem
- The "talk" object captures context for the next review/fix run — populate `files_read` with anything you actually opened

Diff:
{diff_text}"#
    )
}

fn run_agent_json(
    cli_path: &str,
    cli_cmd: &str,
    repo_path: &str,
    prompt: &str,
) -> Result<(Value, String), String> {
    let cli_output = StdCommand::new(cli_path)
        .args(["-p", prompt])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to spawn {cli_cmd} (resolved to {cli_path}): {e}"))?;

    if !cli_output.status.success() {
        let stderr = String::from_utf8_lossy(&cli_output.stderr);
        return Err(format!("{cli_cmd} failed: {stderr}"));
    }

    let raw_output = String::from_utf8_lossy(&cli_output.stdout).to_string();
    let json_str = extract_json_from_output(&raw_output)
        .ok_or_else(|| format!("Could not find JSON in {cli_cmd} output"))?;
    let parsed: Value =
        serde_json::from_str(&json_str).map_err(|e| format!("Failed to parse JSON: {e}"))?;

    Ok((parsed, raw_output))
}

fn findings_from(parsed: &Value) -> Vec<Value> {
    parsed
        .get("findings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

fn score_from_findings(parsed: &Value, findings: &[Value]) -> f64 {
    parsed
        .get("score")
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| {
            let mut s: f64 = 100.0;
            for f in findings {
                let sev = f.get("severity").and_then(|v| v.as_str()).unwrap_or("low");
                s += match sev {
                    "critical" => -20.0,
                    "high" => -10.0,
                    "medium" => -5.0,
                    "low" => -2.0,
                    _ => -1.0,
                };
            }
            s.max(0.0)
        })
}

fn finding_dedupe_key(finding: &Value) -> String {
    let file = finding
        .get("filePath")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let line = finding
        .get("line")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();
    let title = finding
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    format!("{file}:{line}:{title}")
}

fn severity_rank(severity: &str) -> i32 {
    match severity {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn dedupe_findings(findings: Vec<Value>) -> Vec<Value> {
    let mut by_key: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for finding in findings {
        let key = finding_dedupe_key(&finding);
        if key == "::" {
            by_key.insert(uuid::Uuid::new_v4().to_string(), finding);
            continue;
        }
        let replace = by_key
            .get(&key)
            .map(|existing| {
                let current_rank = finding
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .map(severity_rank)
                    .unwrap_or(0);
                let existing_rank = existing
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .map(severity_rank)
                    .unwrap_or(0);
                current_rank > existing_rank
            })
            .unwrap_or(true);
        if replace {
            by_key.insert(key, finding);
        }
    }

    let mut deduped: Vec<Value> = by_key.into_values().collect();
    deduped.sort_by(|a, b| {
        let ar = a
            .get("severity")
            .and_then(|v| v.as_str())
            .map(severity_rank)
            .unwrap_or(0);
        let br = b
            .get("severity")
            .and_then(|v| v.as_str())
            .map(severity_rank)
            .unwrap_or(0);
        br.cmp(&ar)
    });
    deduped
}

fn build_coordinator_prompt(
    project_description: &str,
    change_description: &str,
    plan: &ReviewPlan,
    evidence_section: &str,
    specialist_outputs: &[Value],
) -> String {
    let outputs = serde_json::to_string_pretty(specialist_outputs).unwrap_or_else(|_| "[]".into());
    format!(
        r#"You are the coordinator for a CodeVetter specialist review. Deduplicate and rank findings from specialist reviewers. Keep only real, verified issues. Drop duplicates, style-only comments, and findings that lack a concrete file/symbol/line basis.

Project: {project_description}
Change: {change_description}
Review tier: {tier}
Review mode: {mode}
Changed lines: {changed_lines}
{evidence_section}

Specialist outputs:
{outputs}

Output exactly one final ```json fenced block as the last thing in your response.

JSON shape:
{{"findings":[{{"severity":"critical|high|medium|low","title":"...","summary":"... — include the specific lines that prove the problem","suggestion":"...","filePath":"...","line":42,"confidence":0.9}}],"score":75,"summary":"Coordinator summary including what was deduplicated and why the remaining findings matter","talk":{{"files_read":[],"files_modified":[],"actions_summary":"Coordinated specialist findings for {mode} review","unfinished_work":null,"key_decisions":"Why final findings were kept or dropped","recommended_next_steps":"What should happen next"}}}}

Rules:
- severity must be one of: critical, high, medium, low
- confidence is 0.0-1.0
- score is 0-100
- The summary must mention the review tier and whether deduplication changed the finding set."#,
        tier = plan.tier,
        mode = plan.mode,
        changed_lines = plan.changed_lines,
        evidence_section = evidence_section,
        outputs = outputs,
    )
}

/// Finding shape received from the frontend (review-core running in webview).
#[derive(Debug, Deserialize)]
pub struct ReviewFindingInput {
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub suggestion: Option<String>,
    #[serde(rename = "filePath")]
    pub file_path: Option<String>,
    pub line: Option<i64>,
    pub confidence: Option<f64>,
    pub fingerprint: Option<String>,
}

/// Get the git diff for a local repository.
/// Returns the diff text and changed file list for the frontend to feed into review-core.
#[tauri::command]
pub async fn get_local_diff(
    repo_path: String,
    diff_range: Option<String>,
) -> Result<Value, String> {
    // Run git diff
    let mut cmd = StdCommand::new("git");
    cmd.arg("diff");
    if let Some(ref range) = diff_range {
        cmd.arg(range);
    }
    cmd.current_dir(&repo_path);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git diff: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {stderr}"));
    }

    let diff_text = String::from_utf8_lossy(&output.stdout).to_string();

    // Get changed file list
    let name_status_output = StdCommand::new("git")
        .args(["diff", "--name-status"])
        .args(diff_range.as_deref().map(|r| vec![r]).unwrap_or_default())
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("git diff --name-status failed: {e}"))?;

    let files: Vec<Value> = String::from_utf8_lossy(&name_status_output.stdout)
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() == 2 {
                let status = match parts[0] {
                    "A" => "added",
                    "M" => "modified",
                    "D" => "removed",
                    "R" => "renamed",
                    _ => "modified",
                };
                Some(json!({"path": parts[1], "status": status}))
            } else {
                None
            }
        })
        .collect();

    Ok(json!({
        "diff": diff_text,
        "files": files,
        "empty": diff_text.trim().is_empty(),
    }))
}

/// Save review results from the frontend (review-core running in webview).
/// The frontend calls review-core + ai-gateway-client, then sends findings here for persistence.
#[tauri::command]
pub async fn save_review(
    db: State<'_, DbState>,
    repo_path: Option<String>,
    source_label: String,
    review_type: String,
    repo_full_name: Option<String>,
    pr_number: Option<i64>,
    score: f64,
    findings: Vec<ReviewFindingInput>,
    review_action: Option<String>,
    summary_markdown: Option<String>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    // Create review record
    let input = LocalReviewInput {
        review_type: Some(review_type),
        source_label: Some(source_label.clone()),
        repo_path: repo_path.clone(),
        repo_full_name,
        pr_number,
        agent_used: Some("review-core".to_string()),
        status: Some("completed".to_string()),
    };

    let review_id = queries::create_local_review(&conn, &input).map_err(|e| e.to_string())?;

    // Insert findings
    for f in &findings {
        queries::insert_review_finding(
            &conn,
            &crate::db::queries::LocalReviewFindingInput {
                review_id: review_id.clone(),
                severity: f.severity.clone(),
                title: f.title.clone(),
                summary: f.summary.clone(),
                suggestion: f.suggestion.clone(),
                file_path: f.file_path.clone(),
                line: f.line,
                confidence: f.confidence,
                fingerprint: f.fingerprint.clone(),
            },
        )
        .map_err(|e| e.to_string())?;
    }

    // Update review with score and completion
    queries::update_local_review(
        &conn,
        &review_id,
        &crate::db::queries::LocalReviewUpdate {
            status: Some("completed".to_string()),
            score_composite: Some(score),
            findings_count: Some(findings.len() as i64),
            review_action,
            summary_markdown,
            error_message: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
        },
    )
    .map_err(|e| e.to_string())?;

    // Log activity
    queries::log_activity(
        &conn,
        &ActivityInput {
            agent_id: None,
            event_type: Some("review_completed".to_string()),
            summary: Some(format!(
                "Review completed for {}: score={:.0}, {} findings",
                source_label,
                score,
                findings.len()
            )),
            metadata: Some(json!({"review_id": review_id}).to_string()),
        },
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "review_id": review_id,
        "status": "completed",
        "score": score,
        "findings_count": findings.len(),
    }))
}

/// Get a single review with all its findings.
#[tauri::command]
pub async fn get_review(db: State<'_, DbState>, id: String) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let (review, findings) =
        queries::get_local_review_with_findings(&conn, &id).map_err(|e| e.to_string())?;
    Ok(json!({
        "review": review,
        "findings": findings,
    }))
}

/// Run a code review via a CLI agent (claude or gemini).
///
/// 1. Gets the git diff for the given range
/// 2. Builds a review prompt and spawns the agent CLI
/// 3. Parses the JSON response, computes score, persists findings
/// 4. Returns review_id, score, findings, and summary
#[tauri::command]
pub async fn run_cli_review(
    db: State<'_, DbState>,
    repo_path: String,
    diff_range: String,
    project_description: String,
    change_description: String,
    agent: Option<String>,
    qa_runs: Option<Vec<Value>>,
) -> Result<Value, String> {
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    let qa_runs = qa_runs.unwrap_or_default();
    let start_time = std::time::Instant::now();

    // 1. Get the diff
    let mut cmd = StdCommand::new("git");
    cmd.arg("diff").arg(&diff_range).current_dir(&repo_path);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git diff: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {stderr}"));
    }

    let raw_diff = String::from_utf8_lossy(&output.stdout).to_string();

    if raw_diff.trim().is_empty() {
        return Err("Empty diff — nothing to review".to_string());
    }

    const MAX_DIFF_BYTES: usize = 100 * 1024;
    let was_truncated = raw_diff.len() > MAX_DIFF_BYTES;
    let mut diff_text = raw_diff;
    if was_truncated {
        let mut end = MAX_DIFF_BYTES;
        while end > 0 && !diff_text.is_char_boundary(end) {
            end -= 1;
        }
        diff_text.truncate(end);
        diff_text.push_str(
            "\n\n[DIFF TRUNCATED at 100KB — see file list above for the full change surface]",
        );
    }

    let changed_files: Vec<String> = StdCommand::new("git")
        .args(["diff", "--name-only", &diff_range])
        .current_dir(&repo_path)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let files_section = if changed_files.is_empty() {
        String::new()
    } else {
        let listed = changed_files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n");
        let header = if was_truncated {
            format!(
                "\nFiles changed in this range ({} total — diff below was truncated to 100KB, use your file-read tools to inspect any not fully shown):\n{}\n",
                changed_files.len(),
                listed
            )
        } else {
            format!(
                "\nFiles changed in this range ({} total):\n{}\n",
                changed_files.len(),
                listed
            )
        };
        header
    };

    let blast_summary =
        crate::commands::blast_radius::compute_blast_radius(&repo_path, &diff_range)
            .ok()
            .as_ref()
            .and_then(crate::commands::blast_radius::summarize_for_prompt)
            .unwrap_or_default();

    let blast_section = if blast_summary.is_empty() {
        String::new()
    } else {
        format!("\n{blast_summary}\n")
    };

    let conventions_section = read_repo_conventions(&repo_path);

    // History context (first signals): recent commits on touched files + prior agent summaries + recurring failures.
    // Computed here so the *reviewer agent* sees intent ("why touched files changed before") before judging the new diff.
    // Uses same changed_files list; compact + capped inside the builder. Secrets excluded in git.rs.
    let history_section = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let h = crate::commands::git::build_compact_history_section_for_prompt(
            &repo_path,
            &changed_files,
            &conn,
        );
        drop(conn);
        h
    };

    let plan = build_review_plan(&diff_text, &changed_files);
    let structural_evidence =
        crate::commands::evidence_pattern::collect_structural_evidence(&repo_path, &changed_files);
    let evidence_candidates = crate::commands::evidence_pattern::generate_evidence_candidates(
        crate::commands::evidence_pattern::EvidenceCandidateInput {
            changed_files: &changed_files,
            changed_lines: plan.changed_lines,
            sensitive_paths: &plan.sensitive_paths,
            history_section: &history_section,
            blast_section: &blast_section,
            structural_evidence: &structural_evidence,
        },
    );
    let evidence_section =
        crate::commands::evidence_pattern::render_candidates_for_prompt(&evidence_candidates);
    let evidence_candidates_json =
        serde_json::to_value(&evidence_candidates).unwrap_or_else(|_| json!([]));
    let evidence_procedure_steps =
        crate::commands::evidence_pattern::generate_procedure_steps(&evidence_candidates);
    let procedure_section = crate::commands::evidence_pattern::render_procedure_steps_for_prompt(
        &evidence_procedure_steps,
    );
    let evidence_procedure_steps_json =
        serde_json::to_value(&evidence_procedure_steps).unwrap_or_else(|_| json!([]));
    let review_memory_graph = build_review_memory_graph(
        &changed_files,
        &evidence_candidates,
        &evidence_procedure_steps,
        &history_section,
        &blast_section,
    );
    let review_memory_graph_section = render_review_memory_graph_for_prompt(&review_memory_graph);
    let review_memory_graph_json =
        serde_json::to_value(&review_memory_graph).unwrap_or_else(|_| json!({}));
    let qa_evidence_section = render_qa_evidence_for_prompt(&qa_runs);
    let qa_evidence_json = json!(qa_runs.iter().take(5).cloned().collect::<Vec<_>>());

    // 4. Spawn the CLI agent for the selected tier/specialists.
    let cli_cmd = match agent.as_str() {
        "gemini" => "gemini",
        _ => "claude",
    };
    let cli_path = resolve_cli_path(cli_cmd);

    let mut specialist_outputs: Vec<Value> = Vec::new();
    let mut raw_outputs: Vec<String> = Vec::new();
    let mut prompts_used: Vec<String> = Vec::new();

    for specialist in &plan.specialists {
        let specialist_block = build_specialist_block(specialist, &plan);
        let base_prompt = build_review_prompt(
            &project_description,
            &change_description,
            &conventions_section,
            &files_section,
            &blast_section,
            &history_section,
            &review_memory_graph_section,
            &qa_evidence_section,
            &evidence_section,
            &procedure_section,
            &specialist_block,
            &diff_text,
        );

        let prompt = {
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            let p = maybe_prepend_talk_context(&conn, &repo_path, &base_prompt);
            drop(conn);
            p
        };

        let (mut parsed, raw_output) = run_agent_json(&cli_path, cli_cmd, &repo_path, &prompt)?;
        if let Some(obj) = parsed.as_object_mut() {
            obj.insert(
                "specialist".to_string(),
                json!({
                    "id": specialist.id,
                    "name": specialist.name,
                    "focus": specialist.focus,
                }),
            );
        }
        specialist_outputs.push(parsed);
        raw_outputs.push(raw_output);
        prompts_used.push(prompt);
    }

    let mut coordinator_failed: Option<String> = None;
    let parsed = if plan.uses_coordinator {
        let coordinator_prompt = build_coordinator_prompt(
            &project_description,
            &change_description,
            &plan,
            &evidence_section,
            &specialist_outputs,
        );
        prompts_used.push(coordinator_prompt.clone());

        match run_agent_json(&cli_path, cli_cmd, &repo_path, &coordinator_prompt) {
            Ok((mut parsed, raw_output)) => {
                if let Some(obj) = parsed.as_object_mut() {
                    obj.insert("coordinator".to_string(), json!({"status": "completed"}));
                }
                raw_outputs.push(raw_output);
                parsed
            }
            Err(err) => {
                coordinator_failed = Some(err.clone());
                let merged_findings = dedupe_findings(
                    specialist_outputs
                        .iter()
                        .flat_map(findings_from)
                        .collect::<Vec<_>>(),
                );
                json!({
                    "findings": merged_findings,
                    "score": score_from_findings(&json!({}), &merged_findings),
                    "summary": format!(
                        "Risk-tiered review ({}) completed with deterministic merge because coordinator failed: {}",
                        plan.tier,
                        err
                    ),
                    "talk": {
                        "files_read": [],
                        "files_modified": [],
                        "actions_summary": "Merged specialist review outputs after coordinator failure",
                        "unfinished_work": "Review the merged findings manually; coordinator pass failed.",
                        "key_decisions": format!("Tier {} used {} specialist pass(es).", plan.tier, plan.specialists.len()),
                        "recommended_next_steps": "Re-run review if coordinator rationale is needed."
                    },
                    "coordinator": {"status": "failed", "error": err}
                })
            }
        }
    } else {
        let merged_findings = dedupe_findings(
            specialist_outputs
                .iter()
                .flat_map(findings_from)
                .collect::<Vec<_>>(),
        );
        let summaries = specialist_outputs
            .iter()
            .filter_map(|output| output.get("summary").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n");
        json!({
            "findings": merged_findings,
            "score": score_from_findings(&json!({}), &merged_findings),
            "summary": if summaries.is_empty() {
                format!("Risk-tiered review ({}) completed.", plan.tier)
            } else {
                format!("Risk-tiered review ({}) completed.\n\n{}", plan.tier, summaries)
            },
            "talk": {
                "files_read": [],
                "files_modified": [],
                "actions_summary": format!("Ran {} review pass(es) for {} tier.", plan.specialists.len(), plan.tier),
                "unfinished_work": null,
                "key_decisions": format!("Review mode {} used deterministic dedupe.", plan.mode),
                "recommended_next_steps": "Fix selected findings, then re-run review or attach runtime evidence."
            }
        })
    };

    // 5. Extract findings and score from the final/merged review output.
    let findings_val = dedupe_findings(findings_from(&parsed));

    let summary = parsed
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("Review completed")
        .to_string();

    let score = score_from_findings(&parsed, &findings_val);

    let summary_markdown = format!(
        "{}\n\n---\nReview mode: {} ({}) · changed lines: {} · specialist passes: {}{}",
        summary,
        plan.mode,
        plan.tier,
        plan.changed_lines,
        plan.specialists
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
            .join(", "),
        coordinator_failed
            .as_ref()
            .map(|err| format!(" · coordinator fallback: {err}"))
            .unwrap_or_default()
    );

    // 8. Persist the review
    let conn = db.0.lock().map_err(|e| e.to_string())?;

    let source_label = format!("cli:{agent}:{diff_range}");

    let input = LocalReviewInput {
        review_type: Some("cli".to_string()),
        source_label: Some(source_label.clone()),
        repo_path: Some(repo_path.clone()),
        repo_full_name: None,
        pr_number: None,
        agent_used: Some(agent.clone()),
        status: Some("completed".to_string()),
    };

    let review_id = queries::create_local_review(&conn, &input).map_err(|e| e.to_string())?;

    for f in &findings_val {
        let severity = f
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("medium")
            .to_string();
        let title = f
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();
        let f_summary = f
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let suggestion = f
            .get("suggestion")
            .and_then(|v| v.as_str())
            .map(String::from);
        let file_path = f.get("filePath").and_then(|v| v.as_str()).map(String::from);
        let line = f.get("line").and_then(|v| v.as_i64());
        let confidence = f.get("confidence").and_then(|v| v.as_f64());

        queries::insert_review_finding(
            &conn,
            &crate::db::queries::LocalReviewFindingInput {
                review_id: review_id.clone(),
                severity,
                title,
                summary: f_summary,
                suggestion,
                file_path,
                line,
                confidence,
                fingerprint: None,
            },
        )
        .map_err(|e| e.to_string())?;
    }

    // Update review with score and completion
    queries::update_local_review(
        &conn,
        &review_id,
        &crate::db::queries::LocalReviewUpdate {
            status: Some("completed".to_string()),
            score_composite: Some(score),
            findings_count: Some(findings_val.len() as i64),
            review_action: None,
            summary_markdown: Some(summary_markdown.clone()),
            error_message: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
        },
    )
    .map_err(|e| e.to_string())?;

    // 9. Log activity
    queries::log_activity(
        &conn,
        &ActivityInput {
            agent_id: None,
            event_type: Some("cli_review_completed".to_string()),
            summary: Some(format!(
                "CLI review ({agent}, {}:{}) for {}: score={:.0}, {} findings",
                plan.mode,
                plan.tier,
                source_label,
                score,
                findings_val.len()
            )),
            metadata: Some(
                json!({
                    "review_id": review_id,
                    "review_mode": plan.mode,
                    "risk_tier": plan.tier,
                    "changed_lines": plan.changed_lines,
                    "specialists": plan.specialists.iter().map(|s| s.id).collect::<Vec<_>>(),
                    "sensitive_paths": plan.sensitive_paths.clone(),
                    "review_memory_graph": review_memory_graph_json.clone(),
                    "qa_evidence": qa_evidence_json.clone(),
                    "evidence_candidates": evidence_candidates_json.clone(),
                    "evidence_procedure_steps": evidence_procedure_steps_json.clone(),
                    "coordinator_failed": coordinator_failed,
                })
                .to_string(),
            ),
        },
    )
    .map_err(|e| e.to_string())?;

    let duration_ms = start_time.elapsed().as_millis() as u64;

    // 10. Capture talk for handover
    let talk_prompt = prompts_used.join("\n\n--- CODEVETTER REVIEW PASS ---\n\n");
    let raw_output = raw_outputs.join("\n\n--- CODEVETTER REVIEW OUTPUT ---\n\n");
    let talk_input = talk::build_talk_from_review(
        &agent,
        &repo_path,
        &talk_prompt,
        &raw_output,
        &parsed,
        Some(&review_id),
        Some(duration_ms as i64),
        None,
    );
    let talk_id = queries::insert_agent_talk(&conn, &talk_input)
        .map_err(|e| log::warn!("Failed to save talk: {e}"))
        .ok();

    // 11. Return result
    Ok(json!({
        "review_id": review_id,
        "score": score,
        "findings": findings_val,
        "summary": summary,
        "agent": agent,
        "duration_ms": duration_ms,
        "diff_range": diff_range,
        "findings_count": findings_val.len(),
        "talk_id": talk_id,
        "review_mode": plan.mode,
        "risk_tier": plan.tier,
        "changed_lines": plan.changed_lines,
        "specialists": plan.specialists.iter().map(|s| s.id).collect::<Vec<_>>(),
        "sensitive_paths": plan.sensitive_paths.clone(),
        "coordinator_used": plan.uses_coordinator,
        "review_memory_graph": review_memory_graph_json,
        "qa_evidence": qa_evidence_json,
        "evidence_candidates": evidence_candidates_json,
        "evidence_procedure_steps": evidence_procedure_steps_json,
    }))
}

/// Create a git worktree for running fixes in isolation.
/// Returns `(worktree_path, branch_name)` on success, or `None` to fall back to the main repo.
fn create_fix_worktree(repo_path: &str) -> Option<(String, String)> {
    let branch_name = format!("codevetter/fix-{}", uuid::Uuid::new_v4().simple());
    let worktree_dir = format!("{repo_path}/.codevetter-worktrees/{branch_name}");

    // Ensure the parent directory exists
    let parent = std::path::Path::new(&worktree_dir).parent()?;
    std::fs::create_dir_all(parent).ok()?;

    // Add .codevetter-worktrees to git's local exclude (not .gitignore)
    let exclude_path = format!("{repo_path}/.git/info/exclude");
    let exclude_entry = ".codevetter-worktrees";
    if let Ok(contents) = std::fs::read_to_string(&exclude_path) {
        if !contents.lines().any(|l| l.trim() == exclude_entry) {
            let mut new_contents = contents;
            if !new_contents.ends_with('\n') {
                new_contents.push('\n');
            }
            new_contents.push_str(exclude_entry);
            new_contents.push('\n');
            let _ = std::fs::write(&exclude_path, new_contents);
        }
    } else {
        // exclude file doesn't exist or can't be read — try to create it
        let _ = std::fs::create_dir_all(format!("{repo_path}/.git/info"));
        let _ = std::fs::write(&exclude_path, format!("{exclude_entry}\n"));
    }

    // Create branch from HEAD
    let branch_output = StdCommand::new("git")
        .args(["branch", &branch_name])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !branch_output.status.success() {
        return None;
    }

    // Create worktree
    let wt_output = StdCommand::new("git")
        .args(["worktree", "add", &worktree_dir, &branch_name])
        .current_dir(repo_path)
        .output()
        .ok()?;
    if !wt_output.status.success() {
        // Clean up the branch we created
        let _ = StdCommand::new("git")
            .args(["branch", "-D", &branch_name])
            .current_dir(repo_path)
            .output();
        return None;
    }

    Some((worktree_dir, branch_name))
}

fn render_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Fix one or more review findings by sending them to a CLI agent.
/// Creates a git worktree so fixes happen in isolation (not in the user's working directory).
#[tauri::command]
pub async fn fix_findings(
    app: tauri::AppHandle,
    repo_path: String,
    findings: Vec<Value>,
    agent: Option<String>,
) -> Result<Value, String> {
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    let start_time = std::time::Instant::now();

    // Try to create a worktree for isolated fixes; fall back to main repo on failure
    let worktree_info = create_fix_worktree(&repo_path);
    let (work_dir, _using_worktree) = match &worktree_info {
        Some((wt_path, _)) => (wt_path.clone(), true),
        None => (repo_path.clone(), false),
    };

    // Build fix prompt
    let mut issues = String::new();
    for (i, f) in findings.iter().enumerate() {
        let severity = f
            .get("severity")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let title = f.get("title").and_then(|v| v.as_str()).unwrap_or("Issue");
        let summary = f.get("summary").and_then(|v| v.as_str()).unwrap_or("");
        let suggestion = f.get("suggestion").and_then(|v| v.as_str()).unwrap_or("");
        let file_path = f
            .get("filePath")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let line = f.get("line").and_then(|v| v.as_i64());
        let task_goal = f
            .get("taskGoal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let human_comment = f
            .get("humanComment")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let acceptance = render_string_array(f.get("acceptanceCriteria"));
        let non_goals = render_string_array(f.get("nonGoals"));

        issues.push_str(&format!("\n{}. [{severity}] {title}\n", i + 1));
        issues.push_str(&format!("   File: {file_path}"));
        if let Some(l) = line {
            issues.push_str(&format!(":{l}"));
        }
        issues.push_str(&format!("\n   Problem: {summary}\n"));
        if !suggestion.is_empty() {
            issues.push_str(&format!("   Fix: {suggestion}\n"));
        }
        if !task_goal.is_empty() {
            issues.push_str(&format!("   Task goal: {task_goal}\n"));
        }
        if !acceptance.is_empty() {
            issues.push_str("   Acceptance criteria:\n");
            for item in acceptance {
                issues.push_str(&format!("   - {item}\n"));
            }
        }
        if !non_goals.is_empty() {
            issues.push_str("   Non-goals:\n");
            for item in non_goals {
                issues.push_str(&format!("   - {item}\n"));
            }
        }
        if !human_comment.is_empty() {
            issues.push_str(&format!("   Human/task source: {human_comment}\n"));
        }
        if let Some(evidence_refs) = f.get("evidenceRefs").and_then(|v| v.as_array()) {
            if !evidence_refs.is_empty() {
                issues.push_str(
                    "   Evidence references (path-backed; inspect artifacts when useful):\n",
                );
                for evidence in evidence_refs {
                    match serde_json::to_string_pretty(evidence) {
                        Ok(rendered) => {
                            for line in rendered.lines() {
                                issues.push_str(&format!("     {line}\n"));
                            }
                        }
                        Err(_) => issues.push_str("     [unrenderable evidence]\n"),
                    }
                }
            }
        }
    }

    let base_fix_prompt = format!(
        "Fix the following code review issues by editing the files directly. Use your tools to read and write the actual source files. Do NOT just describe the changes — actually make the edits. Make the minimal changes needed. Do not refactor unrelated code. Respect any acceptance criteria, non-goals, and evidence references attached to each issue.\n{issues}"
    );

    // Inject previous talk context if available
    let prompt = {
        let db_state = app.state::<DbState>();
        let result = if let Ok(conn) = db_state.0.lock() {
            maybe_prepend_talk_context(&conn, &repo_path, &base_fix_prompt)
        } else {
            base_fix_prompt.clone()
        };
        result
    };

    let cli_cmd = match agent.as_str() {
        "gemini" => "gemini",
        _ => "claude",
    };
    let cli_path = resolve_cli_path(cli_cmd);

    // Spawn in a blocking thread so we don't block the Tauri event loop
    let app_handle = app.clone();
    let work_dir_clone = work_dir.clone();
    let cli_path_clone = cli_path.clone();
    let (stdout, _success, duration_ms) = tokio::task::spawn_blocking(move || {
        let mut child = StdCommand::new(&cli_path_clone)
            .args(["-p", &prompt])
            .current_dir(&work_dir_clone)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!("Failed to spawn {cli_cmd} (resolved to {cli_path_clone}): {e}")
            })?;

        // Drain stderr on a separate thread so the child can't deadlock on a
        // full stderr pipe while we're blocked reading stdout.
        let stderr_handle = child.stderr.take().map(|mut s| {
            std::thread::spawn(move || {
                let mut buf = String::new();
                use std::io::Read;
                let _ = s.read_to_string(&mut buf);
                buf
            })
        });

        let mut stdout_text = String::new();
        if let Some(stdout_pipe) = child.stdout.take() {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(stdout_pipe);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let _ = app_handle.emit("fix-progress", &l);
                        stdout_text.push_str(&l);
                        stdout_text.push('\n');
                    }
                    Err(_) => break,
                }
            }
        }

        let status = child
            .wait()
            .map_err(|e| format!("Process wait failed: {e}"))?;
        let elapsed = start_time.elapsed().as_millis() as u64;

        if !status.success() {
            let stderr_text = stderr_handle
                .and_then(|h| h.join().ok())
                .unwrap_or_default();
            return Err(format!("{cli_cmd} fix failed: {stderr_text}"));
        }

        Ok::<_, String>((stdout_text, true, elapsed))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))??;

    // Get the git diff to show what changed (compare against HEAD in worktree)
    let diff_output = StdCommand::new("git")
        .args(["diff", "HEAD"])
        .current_dir(&work_dir)
        .output()
        .ok();

    let diff_text = diff_output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    // Get list of changed files
    let changed_output = StdCommand::new("git")
        .args(["diff", "HEAD", "--name-status"])
        .current_dir(&work_dir)
        .output()
        .ok();

    let changed_files: Vec<Value> = changed_output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() == 2 {
                Some(json!({"status": parts[0], "path": parts[1]}))
            } else {
                None
            }
        })
        .collect();

    // Truncate agent output for display (max 5KB)
    let agent_output = if stdout.len() > 5000 {
        let mut end = 5000;
        while end > 0 && !stdout.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...\n[truncated]", &stdout[..end])
    } else {
        stdout
    };

    let mut result = json!({
        "success": true,
        "agent": agent,
        "duration_ms": duration_ms,
        "output_length": agent_output.len(),
        "findings_fixed": findings.len(),
        "diff": diff_text,
        "changed_files": changed_files,
        "agent_output": agent_output,
    });

    // Add worktree info if we used one
    if let Some((wt_path, branch)) = &worktree_info {
        result["worktree_path"] = json!(wt_path);
        result["worktree_branch"] = json!(branch);
        result["using_worktree"] = json!(true);
    } else {
        result["using_worktree"] = json!(false);
    }

    // Capture talk for handover
    let modified_paths: Vec<String> = changed_files
        .iter()
        .filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(String::from))
        .collect();
    let talk_input = talk::build_talk_from_fix(
        &agent,
        &repo_path,
        &base_fix_prompt,
        &agent_output,
        &modified_paths,
        None,
        Some(duration_ms as i64),
        Some(0),
        None,
    );
    if let Ok(db_conn) = app.state::<DbState>().0.lock() {
        if let Ok(tid) = queries::insert_agent_talk(&db_conn, &talk_input) {
            result["talk_id"] = json!(tid);
        }
    }

    Ok(result)
}

/// Merge fixes from a worktree branch back into the main repo.
/// Commits changes in the worktree, merges the branch, then cleans up.
#[tauri::command]
pub async fn merge_fix(
    repo_path: String,
    worktree_branch: String,
    worktree_path: String,
) -> Result<Value, String> {
    // 1. Commit all changes in the worktree
    let add_output = StdCommand::new("git")
        .args(["add", "-A"])
        .current_dir(&worktree_path)
        .output()
        .map_err(|e| format!("Failed to stage changes: {e}"))?;
    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(format!("git add failed: {stderr}"));
    }

    let commit_output = StdCommand::new("git")
        .args(["commit", "-m", "fix: resolve code review findings"])
        .current_dir(&worktree_path)
        .output()
        .map_err(|e| format!("Failed to commit: {e}"))?;
    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        // If there's nothing to commit, that's okay — the agent may not have changed anything
        if !stderr.contains("nothing to commit") {
            return Err(format!("git commit failed: {stderr}"));
        }
    }

    // 2. Merge the branch into the main repo
    let merge_output = StdCommand::new("git")
        .args([
            "merge",
            &worktree_branch,
            "--no-ff",
            "-m",
            "fix: merge code review fixes",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to merge: {e}"))?;
    if !merge_output.status.success() {
        let stderr = String::from_utf8_lossy(&merge_output.stderr);
        return Err(format!("git merge failed: {stderr}"));
    }

    // 3. Remove the worktree
    let _ = StdCommand::new("git")
        .args(["worktree", "remove", &worktree_path, "--force"])
        .current_dir(&repo_path)
        .output();

    // 4. Delete the branch
    let _ = StdCommand::new("git")
        .args(["branch", "-D", &worktree_branch])
        .current_dir(&repo_path)
        .output();

    Ok(json!({
        "success": true,
        "merged": true,
    }))
}

/// Discard fixes by removing the worktree and deleting the branch.
#[tauri::command]
pub async fn discard_fix(
    repo_path: String,
    worktree_branch: String,
    worktree_path: String,
) -> Result<Value, String> {
    // 1. Remove the worktree
    let wt_output = StdCommand::new("git")
        .args(["worktree", "remove", &worktree_path, "--force"])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to remove worktree: {e}"))?;
    if !wt_output.status.success() {
        let stderr = String::from_utf8_lossy(&wt_output.stderr);
        return Err(format!("git worktree remove failed: {stderr}"));
    }

    // 2. Delete the branch
    let branch_output = StdCommand::new("git")
        .args(["branch", "-D", &worktree_branch])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("Failed to delete branch: {e}"))?;
    if !branch_output.status.success() {
        let stderr = String::from_utf8_lossy(&branch_output.stderr);
        return Err(format!("git branch -D failed: {stderr}"));
    }

    Ok(json!({
        "success": true,
        "discarded": true,
    }))
}

/// Revert specific files to their git HEAD state.
#[tauri::command]
pub async fn revert_files(repo_path: String, files: Vec<String>) -> Result<Value, String> {
    let mut reverted = Vec::new();
    let mut failed = Vec::new();

    for file in &files {
        let output = StdCommand::new("git")
            .args(["checkout", "HEAD", "--", file])
            .current_dir(&repo_path)
            .output()
            .map_err(|e| format!("Failed to run git checkout: {e}"))?;

        if output.status.success() {
            reverted.push(file.clone());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            failed.push(json!({"file": file, "error": stderr}));
        }
    }

    Ok(json!({
        "reverted": reverted,
        "failed": failed,
    }))
}

/// Reverse-apply one unified diff hunk from the fix worktree.
#[tauri::command]
pub async fn revert_diff_hunk(
    repo_path: String,
    file_path: String,
    hunk: String,
) -> Result<Value, String> {
    let path = Path::new(&file_path);
    if path.is_absolute() || file_path.split('/').any(|part| part == "..") {
        return Err("Refusing to revert a hunk outside the repository".to_string());
    }
    if !hunk.lines().any(|line| line.starts_with("@@")) {
        return Err("Invalid diff hunk: missing hunk header".to_string());
    }

    let patch = format!(
        "diff --git a/{file_path} b/{file_path}\n--- a/{file_path}\n+++ b/{file_path}\n{}\n",
        hunk.trim_end()
    );

    let mut child = StdCommand::new("git")
        .args(["apply", "-R", "--recount", "-"])
        .current_dir(&repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run git apply: {e}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(patch.as_bytes())
            .map_err(|e| format!("Failed to write hunk patch: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to wait for git apply: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git apply -R failed: {stderr}"));
    }

    Ok(json!({
        "reverted": true,
        "file": file_path,
    }))
}

/// Public re-export for `unpack.rs` (and any future module) — same logic, no
/// duplication.
pub fn extract_json_from_output_pub(output: &str) -> Option<String> {
    extract_json_from_output(output)
}

/// Public re-export of `resolve_cli_path` for sibling command modules.
pub fn resolve_cli_path_pub(name: &str) -> String {
    resolve_cli_path(name)
}

fn extract_json_from_output(output: &str) -> Option<String> {
    let mut last_fenced: Option<String> = None;
    let mut cursor = 0;
    while let Some(rel) = output[cursor..].find("```json") {
        let start = cursor + rel + 7;
        if let Some(end_rel) = output[start..].find("```") {
            let candidate = output[start..start + end_rel].trim();
            if serde_json::from_str::<Value>(candidate).is_ok() {
                last_fenced = Some(candidate.to_string());
            }
            cursor = start + end_rel + 3;
        } else {
            break;
        }
    }
    if let Some(found) = last_fenced {
        return Some(found);
    }

    let mut last_bare: Option<String> = None;
    let mut cursor = 0;
    while let Some(rel) = output[cursor..].find("```\n") {
        let start = cursor + rel + 4;
        if let Some(end_rel) = output[start..].find("```") {
            let candidate = output[start..start + end_rel].trim();
            if serde_json::from_str::<Value>(candidate).is_ok() {
                last_bare = Some(candidate.to_string());
            }
            cursor = start + end_rel + 3;
        } else {
            break;
        }
    }
    if let Some(found) = last_bare {
        return Some(found);
    }

    let mut depth = 0i32;
    let mut json_start: Option<usize> = None;
    let mut last_raw: Option<String> = None;
    for (i, ch) in output.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    json_start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = json_start {
                        let candidate = &output[start..=i];
                        if serde_json::from_str::<Value>(candidate).is_ok() {
                            last_raw = Some(candidate.to_string());
                        }
                    }
                    json_start = None;
                }
            }
            _ => {}
        }
    }
    last_raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_line_count_ignores_diff_headers() {
        let diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\n";
        assert_eq!(changed_line_count(diff), 3);
    }

    #[test]
    fn review_plan_keeps_trivial_diff_single_pass() {
        let diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-old\n+new\n";
        let plan = build_review_plan(diff, &["src/a.ts".to_string()]);
        assert_eq!(plan.tier, "trivial");
        assert_eq!(plan.mode, "single-pass");
        assert!(!plan.uses_coordinator);
        assert_eq!(plan.specialists.len(), 1);
    }

    #[test]
    fn review_plan_forces_full_on_sensitive_path() {
        let diff = "diff --git a/src/auth.ts b/src/auth.ts\n--- a/src/auth.ts\n+++ b/src/auth.ts\n@@ -1 +1 @@\n-old\n+new\n";
        let plan = build_review_plan(diff, &["src/auth.ts".to_string()]);
        assert_eq!(plan.tier, "full-sensitive");
        assert_eq!(plan.mode, "specialist-full");
        assert!(plan.uses_coordinator);
        assert_eq!(plan.specialists.len(), 3);
        assert_eq!(plan.sensitive_paths, vec!["src/auth.ts".to_string()]);
    }

    #[test]
    fn review_prompt_includes_ranked_evidence_candidates() {
        let prompt = build_review_prompt(
            "Local desktop reviewer",
            "Change auth boundary",
            "",
            "\nFiles changed in this range (1 total):\n- src/auth.ts\n",
            "",
            "",
            "",
            "",
            "\nRanked evidence candidates (deterministic pre-review search):\n- [sensitive-path-needs-boundary-proof] sensitive_path_without_boundary_evidence severity_hint=high confidence=0.86 scale=1 sensitive file(s)\n",
            "\nProcedure steps (deterministic evidence gates):\n- [review_changed_sensitive_path] review_changed_sensitive_path status=ready candidates=sensitive-path-needs-boundary-proof\n",
            "Review tier: full-sensitive",
            "diff --git a/src/auth.ts b/src/auth.ts\n@@ -1 +1 @@\n-old\n+new\n",
        );

        assert!(prompt.contains("Ranked evidence candidates"));
        assert!(prompt.contains("Validate them against code/evidence"));
        assert!(prompt.contains("sensitive-path-needs-boundary-proof"));
        assert!(prompt.contains("Procedure steps"));
        assert!(prompt.contains("blocked steps as remaining work"));
    }

    #[test]
    fn qa_evidence_section_is_capped_and_prompted_as_runtime_proof() {
        let qa_section = render_qa_evidence_for_prompt(&[json!({
            "runner_type": "repo_playwright",
            "route": "/checkout",
            "goal": "Complete checkout",
            "pass": false,
            "duration_ms": 814,
            "console_errors": 2,
            "artifacts": ["/tmp/codevetter/trace.zip", "/tmp/codevetter/report.json"],
            "notes": "Button click threw TypeError in checkout submit handler."
        })]);
        let prompt = build_review_prompt(
            "Local desktop reviewer",
            "Change checkout flow",
            "",
            "\nFiles changed in this range (1 total):\n- src/pages/Checkout.tsx\n",
            "",
            "",
            "",
            &qa_section,
            "",
            "",
            "Review tier: lite-product",
            "diff --git a/src/pages/Checkout.tsx b/src/pages/Checkout.tsx\n@@ -1 +1 @@\n-old\n+new\n",
        );

        assert!(qa_section.contains("Recent synthetic user QA evidence"));
        assert!(qa_section.contains("FAIL: Complete checkout"));
        assert!(qa_section.contains("runner=repo_playwright"));
        assert!(qa_section.contains("route=/checkout"));
        assert!(qa_section.contains("console_errors=2"));
        assert!(qa_section.contains("artifacts=2"));
        assert!(prompt.contains("Synthetic QA evidence"));
        assert!(prompt.contains("runtime evidence from prior user-flow runs"));
    }

    #[test]
    fn review_memory_graph_links_files_candidates_and_gates() {
        let candidates = vec![crate::commands::evidence_pattern::EvidenceCandidate {
            id: "ui-change-needs-browser-proof".to_string(),
            kind: "ui_without_browser_proof".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.72,
            affected_files: vec!["src/pages/Billing.tsx".to_string()],
            evidence_refs: vec![],
            scale: "UI surface changed".to_string(),
            why_it_matters: "UI changes need interaction evidence.".to_string(),
            caveats: vec![],
            open_questions: vec![],
            suggested_checks: vec![],
        }];
        let steps = vec![crate::commands::evidence_pattern::EvidenceProcedureStep {
            id: "verify_ui_route_change".to_string(),
            procedure: "verify_ui_route_change".to_string(),
            status: "blocked".to_string(),
            candidate_ids: vec!["ui-change-needs-browser-proof".to_string()],
            input: "changed UI".to_string(),
            action: "open route".to_string(),
            output: "browser proof".to_string(),
            artifact: "screenshot".to_string(),
            gate: "Changed UI has browser evidence.".to_string(),
            blocked_on: vec!["browser artifact".to_string()],
        }];

        let graph = build_review_memory_graph(
            &["src/pages/Billing.tsx".to_string()],
            &candidates,
            &steps,
            "Prior decisions touching this change",
            "Blast radius summary",
        );
        let rendered = render_review_memory_graph_for_prompt(&graph);

        assert_eq!(graph.schema_version, 1);
        assert!(graph.nodes.iter().any(|node| node.kind == "file"));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == "raises_candidate"));
        assert!(graph.edges.iter().any(|edge| edge.kind == "requires_gate"));
        assert!(rendered.contains("Changed-file graph neighborhood"));
        assert!(rendered.contains("ui-change-needs-browser-proof"));
        assert!(rendered.contains("verify_ui_route_change"));
    }
}

/// List reviews with pagination and optional repo filter.
#[tauri::command]
pub async fn list_reviews(
    db: State<'_, DbState>,
    limit: Option<i64>,
    offset: Option<i64>,
    repo_path: Option<String>,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let reviews = queries::list_local_reviews_filtered(
        &conn,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        repo_path.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "reviews": reviews }))
}
