use crate::db::queries::{self, ActivityInput, LocalReviewInput};
use crate::talk;
use crate::DbState;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::process::{Command as StdCommand, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::Duration;
use tauri::{Emitter, Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;

static ACTIVE_REVIEW_CANCELLATIONS: OnceLock<
    Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>,
> = OnceLock::new();

struct ReviewCancellationGuard {
    repository_root: String,
}

impl Drop for ReviewCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE_REVIEW_CANCELLATIONS
            .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock()
        {
            active.remove(&self.repository_root);
        }
    }
}

fn register_review_cancellation(repository_root: &str) -> Result<ReviewCancellationGuard, String> {
    let mut active = ACTIVE_REVIEW_CANCELLATIONS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .map_err(|_| "Review cancellation state is unavailable".to_string())?;
    if active.contains_key(repository_root) {
        return Err("A review is already active for this repository".to_string());
    }
    active.insert(
        repository_root.to_string(),
        Arc::new(AtomicBool::new(false)),
    );
    Ok(ReviewCancellationGuard {
        repository_root: repository_root.to_string(),
    })
}

fn review_cancellation(repo_path: &str) -> Arc<AtomicBool> {
    let key = std::fs::canonicalize(repo_path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| repo_path.to_string());
    ACTIVE_REVIEW_CANCELLATIONS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .ok()
        .and_then(|active| active.get(&key).cloned())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
}

#[tauri::command]
pub async fn cancel_cli_review(repo_path: String) -> Result<Value, String> {
    let key = std::fs::canonicalize(&repo_path)
        .map_err(|_| "Review repository is unavailable".to_string())?
        .to_string_lossy()
        .into_owned();
    let active = ACTIVE_REVIEW_CANCELLATIONS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .map_err(|_| "Review cancellation state is unavailable".to_string())?;
    let Some(cancellation) = active.get(&key) else {
        return Ok(json!({"cancelled": false, "reason": "no_active_review"}));
    };
    cancellation.store(true, Ordering::SeqCst);
    Ok(json!({"cancelled": true}))
}

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

const ASSUMPTION_INTEGRITY_SPECIALIST: ReviewSpecialist = ReviewSpecialist {
    id: "assumption-integrity",
    name: "Assumption Integrity",
    focus: "Agent-made assumptions, contradictions between stated intent and code, implicit invariants, and unverified claims that can snowball into wrong follow-up work.",
    checks: &[
        "Extract assumptions from the change description, code comments, renamed symbols, deleted guards, history context, and prior agent claims before judging the implementation.",
        "Confirm each material assumption against repo conventions, actual callers, tests, persisted data, IPC/API contracts, and runtime evidence when available.",
        "Flag contradicted assumptions, assumptions relied on but not enforced, and comments/docs that would steer the next agent incorrectly.",
        "Drop any finding whose own premise is only an unconfirmed assumption.",
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

#[derive(Clone)]
struct ReviewPromptJob {
    specialist: ReviewSpecialist,
    prompt: String,
    unit_indexes: Vec<usize>,
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
            mode: "assumption-first",
            changed_lines,
            sensitive_paths,
            specialists: vec![ASSUMPTION_INTEGRITY_SPECIALIST, GENERAL_REVIEW_SPECIALIST],
            uses_coordinator: false,
        };
    }

    if changed_lines <= 100 && sensitive_paths.is_empty() {
        return ReviewPlan {
            tier: "lite",
            mode: "specialist-lite",
            changed_lines,
            sensitive_paths,
            specialists: vec![
                ASSUMPTION_INTEGRITY_SPECIALIST,
                PRODUCT_SAFETY_SPECIALIST,
                AGENT_HANDOFF_SPECIALIST,
            ],
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
            ASSUMPTION_INTEGRITY_SPECIALIST,
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
    trusted_paths: Vec<crate::commands::graph_trust::GraphPathResult>,
    truncated: bool,
}

fn load_latest_native_repo_graph(
    conn: &rusqlite::Connection,
    repo_path: &str,
) -> Option<crate::commands::unpack_types::RepoGraph> {
    let inventory_json = conn
        .query_row(
            "SELECT inventory_json FROM repo_unpacked_reports
             WHERE repo_path = ?1 AND inventory_json IS NOT NULL
             ORDER BY datetime(created_at) DESC LIMIT 1",
            rusqlite::params![repo_path],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    serde_json::from_str::<crate::commands::unpack_types::RepoInventory>(&inventory_json)
        .ok()
        .map(|inventory| inventory.repo_graph)
}

fn derive_native_review_paths(
    graph: Option<&crate::commands::unpack_types::RepoGraph>,
    changed_files: &[String],
) -> Vec<crate::commands::graph_trust::GraphPathResult> {
    let Some(graph) = graph.filter(|graph| graph.schema_version >= 2) else {
        return Vec::new();
    };
    const BOUNDARIES: [&str; 5] = ["route", "tauri_command", "db_table", "test", "script"];
    let targets = graph
        .nodes
        .iter()
        .filter(|node| BOUNDARIES.contains(&node.kind.as_str()))
        .take(24)
        .collect::<Vec<_>>();
    let mut paths = Vec::new();
    for changed_file in changed_files.iter().take(8) {
        let Some(source) = graph.nodes.iter().find(|node| {
            node.path.as_deref() == Some(changed_file.as_str()) || node.label == *changed_file
        }) else {
            continue;
        };
        for target in &targets {
            if target.id == source.id {
                continue;
            }
            let result = crate::commands::graph_trust::trace_graph_path(
                graph,
                &source.id,
                &target.id,
                Some(&source.id),
                Some(&target.id),
                6,
                2_000,
            );
            if result.found && !result.hops.is_empty() {
                paths.push(result);
            }
        }
    }
    paths.sort_by(|a, b| {
        a.requires_verification
            .cmp(&b.requires_verification)
            .then_with(|| a.hops.len().cmp(&b.hops.len()))
            .then_with(|| {
                a.target
                    .selected
                    .as_ref()
                    .map(|value| value.id.as_str())
                    .cmp(&b.target.selected.as_ref().map(|value| value.id.as_str()))
            })
    });
    paths.dedup_by(|a, b| {
        a.source.selected.as_ref().map(|value| &value.id)
            == b.source.selected.as_ref().map(|value| &value.id)
            && a.target.selected.as_ref().map(|value| &value.id)
                == b.target.selected.as_ref().map(|value| &value.id)
    });
    paths.truncate(4);
    paths
}

#[derive(Debug, Clone, Serialize)]
struct TrustedReviewGraphContext {
    schema_version: i64,
    snapshot_id: String,
    engine_id: String,
    engine_version: String,
    indexed_head: Option<String>,
    current_head: Option<String>,
    stale: bool,
    coverage: crate::commands::structural_graph::types::StructuralGraphCoverage,
    nodes: Vec<crate::commands::structural_graph::types::StructuralGraphNode>,
    edges: Vec<crate::commands::structural_graph::types::StructuralGraphEdge>,
    truncated: bool,
    qualification: String,
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
    trusted_paths: Vec<crate::commands::graph_trust::GraphPathResult>,
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
        trusted_paths,
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
    for path in graph.trusted_paths.iter().take(4) {
        let route = path
            .hops
            .iter()
            .map(|hop| {
                format!(
                    "{} {}[{}; {}; {}] {}",
                    hop.from.label,
                    if hop.follows_stored_direction {
                        "->"
                    } else {
                        "<-"
                    },
                    hop.kind,
                    hop.trust,
                    hop.origin,
                    hop.to.label
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let qualification = if path.requires_verification {
            "navigation lead; verify uncertain/imported/legacy hops against source"
        } else {
            "source-backed connectivity context"
        };
        lines.push(format!(
            "  trusted path: {route} ({qualification}; cannot independently create a finding or verified claim)"
        ));
    }
    if graph.truncated {
        lines.push("- graph truncated; inspect source files for complete context".to_string());
    }
    lines.push(String::new());
    lines.join("\n")
}

fn build_trusted_review_graph_context(
    connection: &rusqlite::Connection,
    repo_path: &str,
    changed_files: &[String],
) -> Option<TrustedReviewGraphContext> {
    use crate::commands::structural_graph::{
        query::{GraphDirection, GraphQueryFilter},
        service::StructuralGraphReadService,
    };
    use std::collections::{BTreeMap, HashSet};

    const MAX_SEEDS_PER_FILE: usize = 2;
    const MAX_NODES: usize = 24;
    const MAX_EDGES: usize = 48;

    let service = StructuralGraphReadService::new(connection, repo_path);
    let status = service.status().ok()?;
    if !status.indexed {
        return None;
    }
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut context = None;
    let mut truncated = status.truncated;
    let filter = GraphQueryFilter::default();

    for file in changed_files.iter().take(12) {
        let search = service.search(file, &filter, 12).ok()?;
        truncated |= search.truncated;
        context.get_or_insert_with(|| search.context.clone());
        let seeds = search
            .hits
            .iter()
            .filter(|hit| hit.node.path.as_deref() == Some(file.as_str()))
            .take(MAX_SEEDS_PER_FILE)
            .map(|hit| hit.node.clone())
            .collect::<Vec<_>>();
        for seed in seeds {
            nodes.entry(seed.id.clone()).or_insert(seed.clone());
            let neighborhood = service
                .neighbors(&seed.id, GraphDirection::Both, &filter, 8, None)
                .ok()?;
            truncated |= neighborhood.truncated;
            for node in neighborhood.nodes {
                nodes.entry(node.id.clone()).or_insert(node);
            }
            for edge in neighborhood.edges {
                edges.entry(edge.id.clone()).or_insert(edge);
            }
        }
    }

    let context = context?;
    if nodes.is_empty() {
        return None;
    }
    truncated |= nodes.len() > MAX_NODES || edges.len() > MAX_EDGES;
    let nodes = nodes.into_values().take(MAX_NODES).collect::<Vec<_>>();
    let node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let edges = edges
        .into_values()
        .filter(|edge| node_ids.contains(edge.from.as_str()) && node_ids.contains(edge.to.as_str()))
        .take(MAX_EDGES)
        .collect::<Vec<_>>();

    Some(TrustedReviewGraphContext {
        schema_version: context.schema_version,
        snapshot_id: context.snapshot_id,
        engine_id: context.engine_id,
        engine_version: context.engine_version,
        indexed_head: status.indexed_head,
        current_head: status.current_head,
        stale: status.stale,
        coverage: context.coverage,
        nodes,
        edges,
        truncated,
        qualification: "Navigation-only structural context. Trust and source anchors are preserved; topology never creates findings, changes severity, or upgrades a claim to verified evidence."
            .to_string(),
    })
}

fn render_trusted_review_graph_for_prompt(context: &TrustedReviewGraphContext) -> String {
    let mut lines = vec![
        "\nCanonical structural graph leads (navigation only; not findings or runtime proof):"
            .to_string(),
        format!(
            "- snapshot {} · engine {}@{} · schema v{} · {} · {}/{} files indexed",
            context.snapshot_id,
            context.engine_id,
            context.engine_version,
            context.schema_version,
            if context.stale { "stale" } else { "current" },
            context.coverage.indexed_files,
            context.coverage.discovered_files,
        ),
        format!("- qualification: {}", context.qualification),
    ];
    for node in context.nodes.iter().take(12) {
        let source = node.sources.first().map(|source| {
            format!(
                " · source {}{}",
                source.path,
                source
                    .start_line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default()
            )
        });
        lines.push(format!(
            "- node [{} / {}] {}{}",
            node.trust.as_str(),
            node.origin.as_str(),
            node.label,
            source.unwrap_or_default()
        ));
    }
    for edge in context.edges.iter().take(16) {
        let source = edge.sources.first().map(|source| {
            format!(
                " · source {}{}",
                source.path,
                source
                    .start_line
                    .map(|line| format!(":{line}"))
                    .unwrap_or_default()
            )
        });
        lines.push(format!(
            "  edge: {} -> {} [{} / {}]{}",
            edge.from,
            edge.to,
            edge.kind,
            edge.trust.as_str(),
            source.unwrap_or_default()
        ));
    }
    if context.truncated {
        lines.push(
            "- structural graph context truncated; query the canonical graph for more".to_string(),
        );
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
1. Start by extracting the material assumptions the agent appears to be making: stated intent, comments, deleted guards, renamed concepts, changed defaults, history/talk claims, and any "this is safe because..." premise.
2. Check those assumptions for contradictions against repo conventions, the actual code, caller contracts, persisted data, IPC/API boundaries, tests, and runtime evidence. Treat a contradicted assumption, or an assumption the code relies on but does not enforce, as a real review target.
3. Read the diff carefully. You have file-read tools — use them when a finding's validity depends on context the diff doesn't show (callers, tests, related files, imports, prior implementation).
4. Verify each potential issue against the actual code before reporting. If you cannot cite specific lines that prove the problem, drop the finding — or, if the signal is real but unverified, lower confidence honestly instead of hiding the uncertainty.
5. Use the blast-radius data above to weight severity: a behavior change to a symbol with 6+ callers should be at least medium severity unless the change is provably backward-compatible.
6. Skip nitpicks (formatting, naming preference, missing comments) unless they will cause real bugs, enforce a false assumption, or break a workflow.
7. Repo conventions above are authoritative. Drop findings that contradict them.
8. History signals (if present) explain prior commits and agent work on the touched files — use them to understand *intent* and avoid re-flagging deliberate past decisions. Only call out if the new diff re-opens an old problem or contradicts the stated intent.
9. Changed-file and canonical structural graph neighborhoods (if present) are navigation leads only. Preserve their extracted/inferred/ambiguous/legacy trust, verify every hop against source or runtime evidence, and never let topology alone create a finding, change severity, or upgrade a claim to verified.
10. Synthetic QA evidence (if present) is runtime evidence from prior user-flow runs. Use failures to focus review, but do not confuse runner/setup failures with app bugs.
11. Ranked evidence candidates (if present) are deterministic search leads, not conclusions. Validate them against code/evidence, reject them if wrong, and preserve any remaining open questions in the summary or finding suggestion.
12. Procedure steps (if present) are explicit evidence gates. Treat blocked steps as remaining work unless the current code/evidence resolves the gate.
13. In the final summary or talk.key_decisions, name the important assumptions you confirmed, contradicted, or left open so the next agent cannot keep building on a false premise.

Output format:

Think through the review first (you may use tools and write reasoning notes). Then output **exactly one** ```json fenced block as the very LAST thing in your response, matching this shape. Do not emit any other ```json fenced blocks anywhere — examples in your reasoning should be unfenced or use a different language tag.

JSON shape (literal text, not a fenced example):
{{"findings":[{{"severity":"critical|high|medium|low","title":"...","summary":"... — include the specific lines that prove the problem","suggestion":"...","filePath":"...","line":42,"sourceAnchor":"exact trimmed source line at line 42","confidence":0.9}}],"score":75,"summary":"Overall assessment","talk":{{"files_read":["src/file.ts"],"files_modified":[],"actions_summary":"What you reviewed and found","unfinished_work":null,"key_decisions":"Important observations about the code","recommended_next_steps":"What should happen next"}}}}

Rules:
- severity must be one of: critical, high, medium, low
- confidence is 0.0-1.0 — be honest; downgrade rather than overclaim
- line is optional (null if unknown); filePath relative to repo root
- sourceAnchor is required for findings and must be the exact trimmed source line at line
- score is 0-100 (100 = perfect)
- Each finding's `summary` must reference the specific line(s) or symbol(s) that prove the problem
- The "talk" object captures context for the next review/fix run — populate `files_read` with anything you actually opened

Diff:
{diff_text}"#
    )
}

const REVIEW_EXECUTOR_TIMEOUT: Duration =
    Duration::from_secs(crate::commands::deterministic_review::REVIEW_WALL_TIME_SECONDS);
const REVIEW_EXECUTOR_OUTPUT_BYTES: usize =
    crate::commands::deterministic_review::REVIEW_OUTPUT_BYTES;

async fn read_review_output<R: AsyncRead + Unpin>(
    reader: R,
    max_output_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take((max_output_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("Could not read review executor output: {error}"))?;
    if bytes.len() > max_output_bytes {
        return Err(format!(
            "Review executor output exceeded {} bytes",
            max_output_bytes
        ));
    }
    Ok(bytes)
}

/// Spawn one explicitly selected review executor with bounded output, wall
/// time, and owned-process cleanup. Dropping or timing out the future kills the
/// process group so child tools cannot outlive the review.
async fn run_agent_json(
    cli_path: String,
    cli_cmd: &str,
    repo_path: String,
    prompt: String,
) -> Result<(Value, String), String> {
    run_agent_json_with_limits(
        cli_path,
        cli_cmd,
        repo_path,
        prompt,
        REVIEW_EXECUTOR_TIMEOUT,
        REVIEW_EXECUTOR_OUTPUT_BYTES,
    )
    .await
}

async fn run_agent_json_with_limits(
    cli_path: String,
    cli_cmd: &str,
    repo_path: String,
    prompt: String,
    deadline: Duration,
    max_output_bytes: usize,
) -> Result<(Value, String), String> {
    if !matches!(cli_cmd, "claude" | "gemini") {
        return Err(format!("Unsupported review executor: {cli_cmd}"));
    }
    if cli_path == cli_cmd && resolve_cli_path(cli_cmd) == cli_cmd {
        return Err(format!(
            "Review executor `{cli_cmd}` is unavailable. Install it or select another configured executor."
        ));
    }

    let mut command = TokioCommand::new(&cli_path);
    command
        .args(["-p", &prompt])
        .current_dir(&repo_path)
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
    let mut child = command.spawn().map_err(|error| {
        format!("Review executor `{cli_cmd}` is unavailable at `{cli_path}`: {error}")
    })?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{cli_cmd} stdout was unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{cli_cmd} stderr was unavailable"))?;
    let stdout_task = tokio::spawn(read_review_output(stdout, max_output_bytes));
    let stderr_task = tokio::spawn(read_review_output(stderr, max_output_bytes));
    enum ProcessWait {
        Completed(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let cancellation = review_cancellation(&repo_path);
    let wait = tokio::select! {
        status = child.wait() => ProcessWait::Completed(status),
        _ = tokio::time::sleep(deadline) => ProcessWait::TimedOut,
        _ = async {
            while !cancellation.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        } => ProcessWait::Cancelled,
    };
    let was_cancelled = matches!(&wait, ProcessWait::Cancelled);
    let status = match wait {
        ProcessWait::Completed(status) => {
            status.map_err(|error| format!("{cli_cmd} wait failed: {error}"))?
        }
        ProcessWait::TimedOut | ProcessWait::Cancelled => {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return if was_cancelled {
                Err(format!(
                    "{cli_cmd} review was cancelled and its process tree was stopped"
                ))
            } else {
                Err(format!(
                    "{cli_cmd} timed out after {} seconds and its process tree was stopped",
                    deadline.as_secs_f64()
                ))
            };
        }
    };
    let stdout = stdout_task
        .await
        .map_err(|error| format!("{cli_cmd} stdout task failed: {error}"))??;
    let stderr = stderr_task
        .await
        .map_err(|error| format!("{cli_cmd} stderr task failed: {error}"))??;
    if !status.success() {
        return Err(format!(
            "{cli_cmd} failed (exit {:?}): {}",
            status.code(),
            String::from_utf8_lossy(&stderr)
        ));
    }
    let raw_output =
        String::from_utf8(stdout).map_err(|_| format!("{cli_cmd} returned non-UTF-8 output"))?;

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

fn checkpoint_projection(parsed: &Value, file_path: &str) -> Value {
    let findings = findings_from(parsed)
        .into_iter()
        .filter(|finding| finding.get("filePath").and_then(Value::as_str) == Some(file_path))
        .collect::<Vec<_>>();
    json!({
        "findings": findings,
        "summary": parsed.get("summary").and_then(Value::as_str).unwrap_or("Checkpointed review unit"),
        "score": parsed.get("score").and_then(Value::as_f64),
        "specialist": parsed.get("specialist").cloned().unwrap_or(Value::Null),
    })
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

/// Normalized title tokens for near-duplicate detection: lowercase, split on
/// non-alphanumerics, drop short/stop words, strip a plural 's'. Kept small
/// and deterministic — this feeds a similarity check, not NLP.
fn finding_title_tokens(finding: &Value) -> std::collections::BTreeSet<String> {
    const STOPWORDS: &[&str] = &[
        "the", "a", "an", "in", "on", "of", "to", "and", "or", "for", "with", "via", "is", "are",
        "into", "from", "by", "at", "that", "this", "when", "can",
    ];
    finding
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2 && !STOPWORDS.contains(t))
        .map(|t| {
            if t.len() > 3 && t.ends_with('s') {
                t[..t.len() - 1].to_string()
            } else {
                t.to_string()
            }
        })
        .collect()
}

fn token_jaccard(
    a: &std::collections::BTreeSet<String>,
    b: &std::collections::BTreeSet<String>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    intersection / union
}

/// Same defect stated twice? Specialists phrase one issue many ways and drift
/// a line or two, so exact `file:line:title` keys leak near-duplicates (the
/// public benchmark measured 41 redundant restatements across 95 findings).
/// Two findings collapse when they are in the same file and EITHER
/// close-by with moderate title overlap, or further apart with strong overlap.
fn is_duplicate_finding(a: &Value, b: &Value) -> bool {
    let file = |f: &Value| {
        f.get("filePath")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    if file(a) != file(b) {
        return false;
    }
    let line = |f: &Value| f.get("line").and_then(|v| v.as_i64());
    let line_delta = match (line(a), line(b)) {
        (Some(la), Some(lb)) => (la - lb).abs(),
        // No line info on either side → require the strong-similarity arm.
        _ => i64::MAX,
    };
    let similarity = token_jaccard(&finding_title_tokens(a), &finding_title_tokens(b));
    (line_delta <= 2 && similarity >= 0.30) || (line_delta <= 10 && similarity >= 0.65)
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
    // Pass 1: exact-key dedupe (identical file:line:title from a re-run).
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

    // Pass 2: near-duplicate clustering — greedy against kept representatives,
    // keeping the higher-severity (then higher-confidence) statement of each
    // defect. Finding counts are small (≤ a few dozen), so O(n²) is fine.
    let confidence = |f: &Value| f.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let rank = |f: &Value| {
        f.get("severity")
            .and_then(|v| v.as_str())
            .map(severity_rank)
            .unwrap_or(0)
    };
    let mut kept: Vec<Value> = Vec::new();
    for finding in by_key.into_values() {
        match kept.iter_mut().find(|k| is_duplicate_finding(k, &finding)) {
            Some(existing) => {
                let better = rank(&finding) > rank(existing)
                    || (rank(&finding) == rank(existing)
                        && confidence(&finding) > confidence(existing));
                if better {
                    *existing = finding;
                }
            }
            None => kept.push(finding),
        }
    }

    let mut deduped: Vec<Value> = kept;
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

Assumption integrity is mandatory: before keeping any finding, confirm the premise it depends on. Preserve findings where the implementation contradicts its stated intent, a comment/docs claim is false, an implicit invariant is relied on but unenforced, or prior agent claims would steer follow-up work incorrectly. Drop findings that are themselves based on an unconfirmed assumption.

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
{{"findings":[{{"severity":"critical|high|medium|low","title":"...","summary":"... — include the specific lines that prove the problem","suggestion":"...","filePath":"...","line":42,"sourceAnchor":"exact trimmed source line at line 42","confidence":0.9}}],"score":75,"summary":"Coordinator summary including what was deduplicated and why the remaining findings matter","talk":{{"files_read":[],"files_modified":[],"actions_summary":"Coordinated specialist findings for {mode} review","unfinished_work":null,"key_decisions":"Why final findings were kept or dropped","recommended_next_steps":"What should happen next"}}}}

Rules:
- severity must be one of: critical, high, medium, low
- confidence is 0.0-1.0
- sourceAnchor must be the exact trimmed current source line named by line
- score is 0-100
- The summary must mention the review tier, whether deduplication changed the finding set, and any confirmed/contradicted/open assumptions that matter."#,
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
    let range = diff_range.as_deref().unwrap_or("WORKTREE");
    let target = crate::commands::deterministic_review::resolve_target(&repo_path, range)?;
    let diff_text = crate::commands::deterministic_review::read_target_diff(&target)?;
    let files = crate::commands::deterministic_review::plan_units(&target, "preview")?
        .into_iter()
        .map(|unit| {
            let status = match unit.file_status.as_str() {
                "A" => "added",
                "D" => "removed",
                "R" => "renamed",
                _ => "modified",
            };
            json!({"path": unit.file_path, "status": status})
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "diff": diff_text,
        "files": files,
        "empty": diff_text.trim().is_empty(),
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

#[tauri::command]
pub async fn get_review_manifest(
    db: State<'_, DbState>,
    review_id: String,
) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    if let Some(manifest) =
        crate::commands::deterministic_review::load_manifest_for_review(&conn, &review_id)?
    {
        return serde_json::to_value(manifest).map_err(|error| error.to_string());
    }
    Ok(json!({
        "schema_version": 1,
        "review_id": review_id,
        "coverage_kind": "legacy_aggregate",
        "complete_coverage": false,
        "limitation": "This review predates deterministic per-file coverage. Existing findings remain readable, but coverage completeness is unknown."
    }))
}

#[tauri::command]
pub async fn delete_review(db: State<'_, DbState>, id: String) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let deleted = crate::db::with_busy_retry(
        || {
            conn.execute(
                "DELETE FROM local_reviews WHERE id = ?1",
                rusqlite::params![id],
            )
        },
        5,
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({ "deleted": deleted > 0 }))
}

/// Record (or clear) the owner's usefulness verdict on a finding.
/// `disposition` must be `"accepted"`, `"dismissed"`, or `None` to clear
/// back to unreviewed. Returns `{ "updated": <rows> }`.
#[tauri::command]
pub async fn set_finding_disposition(
    db: State<'_, DbState>,
    finding_id: String,
    disposition: Option<String>,
) -> Result<Value, String> {
    let normalized = match disposition.as_deref() {
        None => None,
        Some("accepted") => Some("accepted"),
        Some("dismissed") => Some("dismissed"),
        Some(other) => {
            return Err(format!(
                "invalid disposition '{other}' (expected 'accepted', 'dismissed', or null)"
            ))
        }
    };
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let updated = queries::set_finding_disposition(&conn, &finding_id, normalized)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "updated": updated }))
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
    standards_pack: Option<String>,
) -> Result<Value, String> {
    run_cli_review_core(
        db.inner().clone(),
        repo_path,
        diff_range,
        project_description,
        change_description,
        agent,
        qa_runs,
        standards_pack,
    )
    .await
}

/// State-free core of `run_cli_review` so headless harnesses (the public
/// benchmark generator) can run the EXACT production pipeline — risk tiers,
/// specialists, coordinator, dedup — without a Tauri runtime.
#[allow(clippy::too_many_arguments)]
pub async fn run_cli_review_core(
    db: DbState,
    repo_path: String,
    diff_range: String,
    project_description: String,
    change_description: String,
    agent: Option<String>,
    qa_runs: Option<Vec<Value>>,
    standards_pack: Option<String>,
) -> Result<Value, String> {
    let agent = agent.unwrap_or_else(|| "claude".to_string());
    if !matches!(agent.as_str(), "claude" | "gemini") {
        return Err(format!(
            "Unsupported review executor `{agent}`. Choose `claude` or `gemini`."
        ));
    }
    let qa_runs = qa_runs.unwrap_or_default();
    let start_time = std::time::Instant::now();

    // 1. Resolve a safe immutable target and create the zero-model coverage manifest
    // before any provider process starts. Unknown option-like ranges fail closed.
    let target = crate::commands::deterministic_review::resolve_target(&repo_path, &diff_range)?;
    let planning_context = json!({
        "project_description": project_description,
        "change_description": change_description,
        "qa_runs": qa_runs,
        "standards_pack": standards_pack,
    })
    .to_string();
    let units = crate::commands::deterministic_review::plan_units_with_context(
        &target,
        &agent,
        &planning_context,
    )?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut review_manifest =
        crate::commands::deterministic_review::new_manifest(run_id, target, agent.clone(), units);
    let raw_diff =
        crate::commands::deterministic_review::read_target_diff(&review_manifest.target)?;

    if raw_diff.trim().is_empty() {
        return Err("Empty diff — nothing to review".to_string());
    }
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        crate::commands::deterministic_review::claim_manifest(&conn, &review_manifest)?;
    }
    let _cancellation_guard =
        register_review_cancellation(&review_manifest.target.repository_root)?;

    const MAX_DIFF_BYTES: usize = 100 * 1024;
    // One-release rollback: setting CODEVETTER_REVIEW_PIPELINE=legacy keeps
    // the previous aggregate executor path while preserving readable manifest
    // records. The default remains the qualified per-unit path for broad diffs.
    let manifest_pipeline_enabled = std::env::var("CODEVETTER_REVIEW_PIPELINE")
        .map(|value| !value.eq_ignore_ascii_case("legacy"))
        .unwrap_or(true);
    let requires_unit_execution = manifest_pipeline_enabled && raw_diff.len() > MAX_DIFF_BYTES;
    let diff_text = raw_diff;

    let changed_files = review_manifest
        .units
        .iter()
        .map(|unit| unit.file_path.clone())
        .collect::<Vec<_>>();

    let files_section = if changed_files.is_empty() {
        String::new()
    } else {
        let listed = changed_files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nFiles changed in this range ({} total):\n{}\n",
            changed_files.len(),
            listed
        )
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
    let (history_section, trusted_graph_context) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        let h = crate::commands::git::build_compact_history_section_for_prompt(
            &repo_path,
            &changed_files,
            &conn,
        );
        let graph = build_trusted_review_graph_context(&conn, &repo_path, &changed_files);
        drop(conn);
        (h, graph)
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
    let native_repo_graph = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        load_latest_native_repo_graph(&conn, &repo_path)
    };
    let trusted_paths = derive_native_review_paths(native_repo_graph.as_ref(), &changed_files);
    let review_memory_graph = build_review_memory_graph(
        &changed_files,
        &evidence_candidates,
        &evidence_procedure_steps,
        &history_section,
        &blast_section,
        trusted_paths,
    );
    let review_memory_graph_section = render_review_memory_graph_for_prompt(&review_memory_graph);
    let trusted_graph_section = trusted_graph_context
        .as_ref()
        .map(render_trusted_review_graph_for_prompt)
        .unwrap_or_default();
    let graph_section = format!("{review_memory_graph_section}{trusted_graph_section}");
    let review_memory_graph_json =
        serde_json::to_value(&review_memory_graph).unwrap_or_else(|_| json!({}));
    let trusted_graph_context_json =
        serde_json::to_value(&trusted_graph_context).unwrap_or(Value::Null);
    let qa_evidence_section = render_qa_evidence_for_prompt(&qa_runs);
    let qa_evidence_json = json!(qa_runs.iter().take(5).cloned().collect::<Vec<_>>());

    // 4. Spawn the CLI agent for the selected tier/specialists.
    let cli_cmd = agent.as_str();
    let cli_path = resolve_cli_path(cli_cmd);

    let mut raw_outputs: Vec<String> = Vec::new();
    let mut prompts_used: Vec<String> = Vec::new();

    let mut specialist_outputs: Vec<Value> = Vec::new();
    let mut unit_outputs = vec![Vec::<Value>::new(); review_manifest.units.len()];
    let mut pending_units = Vec::new();
    let checkpoint_context = review_manifest.clone();
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        for (index, unit) in review_manifest.units.iter_mut().enumerate() {
            if matches!(
                unit.coverage_state,
                crate::commands::deterministic_review::ReviewCoverageState::Skipped
            ) {
                continue;
            }
            if let Some(outputs) = crate::commands::deterministic_review::load_checkpoint_outputs(
                &conn,
                &checkpoint_context,
                unit,
            )? {
                unit.coverage_state =
                    crate::commands::deterministic_review::ReviewCoverageState::Reused;
                unit.coverage_reason = Some("fingerprint_checkpoint_match".to_string());
                specialist_outputs.extend(outputs.clone());
                unit_outputs[index] = outputs;
            } else {
                pending_units.push(index);
            }
        }
    }

    // The fast path remains one aggregate payload. Once the aggregate exceeds
    // 100 KiB (or a partial resume exists), each unfinished file becomes its
    // own bounded execution unit. Nothing is silently truncated.
    let use_unit_execution = requires_unit_execution
        || pending_units.len()
            < review_manifest
                .units
                .iter()
                .filter(|unit| {
                    !matches!(
                        unit.coverage_state,
                        crate::commands::deterministic_review::ReviewCoverageState::Skipped
                    )
                })
                .count();
    let mut execution_batches: Vec<(Vec<usize>, String, String)> = Vec::new();
    if use_unit_execution {
        for index in pending_units {
            let unit = &mut review_manifest.units[index];
            let unit_diff = crate::commands::deterministic_review::read_unit_diff(
                &review_manifest.target,
                &unit.file_path,
            )?;
            if unit_diff.len() > unit.prompt_budget_bytes {
                unit.coverage_state =
                    crate::commands::deterministic_review::ReviewCoverageState::Failed;
                unit.coverage_reason = Some("file_diff_exceeds_prompt_budget".to_string());
                continue;
            }
            execution_batches.push((
                vec![index],
                unit_diff,
                format!("\nReview unit file:\n- {}\n", unit.file_path),
            ));
        }
    } else if !pending_units.is_empty() {
        execution_batches.push((pending_units, diff_text.clone(), files_section.clone()));
    }

    let mut specialist_prompts = Vec::<ReviewPromptJob>::new();
    for (unit_indexes, batch_diff, batch_files) in execution_batches {
        for specialist in &plan.specialists {
            let specialist_block = build_specialist_block(specialist, &plan);
            let base_prompt = build_review_prompt(
                &project_description,
                &change_description,
                &conventions_section,
                &batch_files,
                &blast_section,
                &history_section,
                &graph_section,
                &qa_evidence_section,
                &evidence_section,
                &procedure_section,
                &specialist_block,
                &batch_diff,
            );
            let prompt = {
                let conn = db.0.lock().map_err(|e| e.to_string())?;
                maybe_prepend_talk_context(&conn, &repo_path, &base_prompt)
            };
            specialist_prompts.push(ReviewPromptJob {
                specialist: *specialist,
                prompt,
                unit_indexes: unit_indexes.clone(),
            });
        }
    }
    {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        crate::commands::deterministic_review::persist_manifest(
            &conn,
            &review_manifest,
            "running",
        )?;
    }

    // Bounded concurrency: at most MAX_CONCURRENT specialist CLI calls in
    // flight at once. Results are collected by index so the order matches the
    // plan regardless of completion order.
    const MAX_CONCURRENT_SPECIALISTS: usize =
        crate::commands::deterministic_review::REVIEW_MAX_CONCURRENCY;
    let total = specialist_prompts.len();
    type SpecialistOutcome = Result<(Value, String), String>;
    type RecordedSpecialistOutcome = (String, SpecialistOutcome);
    type SpecialistTaskResult = (usize, String, SpecialistOutcome);
    let mut results: Vec<Option<RecordedSpecialistOutcome>> = (0..total).map(|_| None).collect();
    let mut join_set: tokio::task::JoinSet<SpecialistTaskResult> = tokio::task::JoinSet::new();
    let mut next = 0usize;

    while next < total || !join_set.is_empty() {
        while join_set.len() < MAX_CONCURRENT_SPECIALISTS && next < total {
            let idx = next;
            let job = specialist_prompts[idx].clone();
            let cli_path = cli_path.clone();
            let cli_cmd = cli_cmd.to_string();
            let repo_path = repo_path.clone();
            join_set.spawn(async move {
                let started_at = chrono::Utc::now().to_rfc3339();
                let outcome = run_agent_json(cli_path, &cli_cmd, repo_path, job.prompt)
                    .await
                    .map(|(mut parsed, raw_output)| {
                        if let Some(obj) = parsed.as_object_mut() {
                            obj.insert(
                                "specialist".to_string(),
                                json!({
                                    "id": job.specialist.id,
                                    "name": job.specialist.name,
                                    "focus": job.specialist.focus,
                                }),
                            );
                        }
                        (parsed, raw_output)
                    });
                (idx, started_at, outcome)
            });
            next += 1;
        }

        if let Some(joined) = join_set.join_next().await {
            let (idx, started_at, outcome) = joined.map_err(|e| format!("Task join error: {e}"))?;
            results[idx] = Some((started_at, outcome));
        }
    }

    for (idx, slot) in results.into_iter().enumerate() {
        let job = &specialist_prompts[idx];
        let (started_at, outcome) = slot.expect("every specialist slot is filled");
        match outcome {
            Ok((parsed, raw_output)) => {
                for unit_index in &job.unit_indexes {
                    let unit = &review_manifest.units[*unit_index];
                    unit_outputs[*unit_index].push(checkpoint_projection(&parsed, &unit.file_path));
                    let conn = db.0.lock().map_err(|e| e.to_string())?;
                    crate::commands::deterministic_review::record_attempt(
                        &conn,
                        &review_manifest,
                        &unit.id,
                        idx + 1,
                        "completed",
                        None,
                        raw_output.len(),
                        &started_at,
                        None,
                    )?;
                }
                specialist_outputs.push(parsed);
                raw_outputs.push(raw_output);
                prompts_used.push(job.prompt.clone());
            }
            Err(error) => {
                let cancelled = error.contains("review was cancelled");
                if cancelled {
                    review_manifest.cancelled = true;
                }
                for unit_index in &job.unit_indexes {
                    let unit_id = {
                        let unit = &mut review_manifest.units[*unit_index];
                        unit.coverage_state = if cancelled {
                            crate::commands::deterministic_review::ReviewCoverageState::Cancelled
                        } else {
                            crate::commands::deterministic_review::ReviewCoverageState::Failed
                        };
                        unit.coverage_reason = Some(if cancelled {
                            "user_cancelled".to_string()
                        } else {
                            "executor_failed".to_string()
                        });
                        unit.id.clone()
                    };
                    let conn = db.0.lock().map_err(|e| e.to_string())?;
                    crate::commands::deterministic_review::record_attempt(
                        &conn,
                        &review_manifest,
                        &unit_id,
                        idx + 1,
                        if cancelled { "cancelled" } else { "failed" },
                        Some(&error.chars().take(2_048).collect::<String>()),
                        0,
                        &started_at,
                        Some((
                            if cancelled { "cancelled" } else { "failed" },
                            if cancelled {
                                "user_cancelled"
                            } else {
                                "executor_failed"
                            },
                        )),
                    )?;
                }
            }
        }
    }

    for (index, outputs) in unit_outputs.iter().enumerate() {
        if outputs.len() == plan.specialists.len()
            && !matches!(
                review_manifest.units[index].coverage_state,
                crate::commands::deterministic_review::ReviewCoverageState::Reused
            )
        {
            review_manifest.units[index].coverage_state =
                crate::commands::deterministic_review::ReviewCoverageState::Reviewed;
            review_manifest.units[index].coverage_reason = None;
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            crate::commands::deterministic_review::persist_unit_checkpoint(
                &conn,
                &review_manifest,
                &review_manifest.units[index],
                outputs,
            )?;
        }
    }
    if specialist_outputs.is_empty() {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        crate::commands::deterministic_review::persist_manifest(
            &conn,
            &review_manifest,
            if review_manifest.cancelled {
                "cancelled"
            } else {
                "failed"
            },
        )?;
        return Err("No review unit completed successfully".to_string());
    }

    // Qualify every specialist candidate before the optional coordinator sees
    // it. The coordinator can rank and deduplicate only source-backed evidence;
    // it never gets a chance to upgrade an unsafe path or stale line.
    let mut prequalification_counts =
        crate::commands::deterministic_review::QualificationCounts::default();
    let mut prequalification_diagnostics = Vec::new();
    for output in &mut specialist_outputs {
        let qualified = crate::commands::deterministic_review::qualify_candidates(
            &repo_path,
            &changed_files,
            findings_from(output),
        );
        prequalification_counts.qualified += qualified.counts.qualified;
        prequalification_counts.stale += qualified.counts.stale;
        prequalification_counts.unresolved += qualified.counts.unresolved;
        prequalification_counts.rejected += qualified.counts.rejected;
        let offset = prequalification_diagnostics.len();
        prequalification_diagnostics.extend(qualified.diagnostics.into_iter().map(
            |mut diagnostic| {
                diagnostic.candidate_index += offset;
                diagnostic
            },
        ));
        if let Some(object) = output.as_object_mut() {
            object.insert("findings".to_string(), Value::Array(qualified.findings));
        }
    }

    let mut coordinator_failed: Option<String> = None;
    let parsed = if plan.uses_coordinator
        && !specialist_prompts.is_empty()
        && !review_manifest.cancelled
    {
        let coordinator_prompt = build_coordinator_prompt(
            &project_description,
            &change_description,
            &plan,
            &evidence_section,
            &specialist_outputs,
        );
        prompts_used.push(coordinator_prompt.clone());

        match run_agent_json(
            cli_path.clone(),
            cli_cmd,
            repo_path.clone(),
            coordinator_prompt.clone(),
        )
        .await
        {
            Ok((mut parsed, raw_output)) => {
                if let Some(obj) = parsed.as_object_mut() {
                    obj.insert("coordinator".to_string(), json!({"status": "completed"}));
                }
                raw_outputs.push(raw_output);
                parsed
            }
            Err(err) => {
                if err.contains("review was cancelled") {
                    review_manifest.cancelled = true;
                }
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

    // 5. Qualify candidates against exact current repository source before
    // dedupe, scoring, persistence, proof, or actionable UI.
    let final_candidates = findings_from(&parsed);
    review_manifest.stale =
        !crate::commands::deterministic_review::target_is_current(&review_manifest.target);
    if review_manifest.stale {
        for diagnostic in &mut prequalification_diagnostics {
            diagnostic.state =
                crate::commands::deterministic_review::CandidateQualificationState::Stale;
            diagnostic.reason = "target_mutated_during_review".to_string();
            diagnostic.resolved_line = None;
        }
        prequalification_counts = crate::commands::deterministic_review::QualificationCounts {
            stale: prequalification_diagnostics.len(),
            ..Default::default()
        };
    }
    let qualified = if review_manifest.stale {
        crate::commands::deterministic_review::invalidate_candidates(
            final_candidates,
            crate::commands::deterministic_review::CandidateQualificationState::Stale,
            "target_mutated_during_review",
        )
    } else if review_manifest.cancelled {
        crate::commands::deterministic_review::invalidate_candidates(
            final_candidates,
            crate::commands::deterministic_review::CandidateQualificationState::Rejected,
            "review_cancelled_before_final_qualification",
        )
    } else {
        crate::commands::deterministic_review::qualify_candidates(
            &repo_path,
            &changed_files,
            final_candidates,
        )
    };
    let findings_val = dedupe_findings(qualified.findings);
    review_manifest.qualification_counts =
        crate::commands::deterministic_review::QualificationCounts {
            qualified: prequalification_counts.qualified + qualified.counts.qualified,
            stale: prequalification_counts.stale + qualified.counts.stale,
            unresolved: prequalification_counts.unresolved + qualified.counts.unresolved,
            rejected: prequalification_counts.rejected + qualified.counts.rejected,
        };
    let diagnostic_offset = prequalification_diagnostics.len();
    prequalification_diagnostics.extend(qualified.diagnostics.into_iter().map(|mut diagnostic| {
        diagnostic.candidate_index += diagnostic_offset;
        diagnostic
    }));
    review_manifest.qualification_diagnostics = prequalification_diagnostics;
    if review_manifest.stale {
        for unit in &mut review_manifest.units {
            if !matches!(
                unit.coverage_state,
                crate::commands::deterministic_review::ReviewCoverageState::Skipped
            ) {
                unit.coverage_state =
                    crate::commands::deterministic_review::ReviewCoverageState::Failed;
                unit.coverage_reason = Some("target_mutated_during_review".to_string());
            }
        }
        review_manifest.complete_coverage = false;
    } else {
        review_manifest.complete_coverage = review_manifest.units.iter().all(|unit| {
            matches!(
                unit.coverage_state,
                crate::commands::deterministic_review::ReviewCoverageState::Reviewed
                    | crate::commands::deterministic_review::ReviewCoverageState::Reused
            )
        });
    }

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
        standards_pack,
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
                discovery_method: None,
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

    review_manifest.review_id = Some(review_id.clone());
    review_manifest.completed_at = Some(chrono::Utc::now().to_rfc3339());
    crate::commands::deterministic_review::persist_manifest(
        &conn,
        &review_manifest,
        if review_manifest.cancelled {
            "cancelled"
        } else if review_manifest.stale {
            "stale"
        } else if review_manifest.complete_coverage {
            "completed"
        } else {
            "completed_with_limitations"
        },
    )?;

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
                    "trusted_graph_context": trusted_graph_context_json.clone(),
                    "qa_evidence": qa_evidence_json.clone(),
                    "evidence_candidates": evidence_candidates_json.clone(),
                    "evidence_procedure_steps": evidence_procedure_steps_json.clone(),
                    "coordinator_failed": coordinator_failed,
                    "review_manifest": review_manifest.clone(),
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
        "trusted_graph_context": trusted_graph_context_json,
        "qa_evidence": qa_evidence_json,
        "evidence_candidates": evidence_candidates_json,
        "evidence_procedure_steps": evidence_procedure_steps_json,
        "review_manifest": review_manifest,
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

pub(crate) fn resolve_agent_cli_path(agent: &str) -> String {
    match agent {
        "cursor" => {
            let cursor_agent = resolve_cli_path("cursor-agent");
            if std::path::Path::new(&cursor_agent).is_file() {
                return cursor_agent;
            }
            resolve_cli_path("agent")
        }
        "command-code" => {
            for name in ["cmd", "command-code", "commandcode", "cmdc"] {
                let candidate = resolve_cli_path(name);
                if std::path::Path::new(&candidate).is_file() {
                    return candidate;
                }
            }
            resolve_cli_path("cmd")
        }
        "codex" => resolve_cli_path("codex"),
        "grok" => resolve_cli_path("grok"),
        "gemini" => resolve_cli_path("gemini"),
        _ => resolve_cli_path("claude"),
    }
}

pub(crate) fn agent_cli_label(agent: &str) -> &'static str {
    match agent {
        "gemini" => "gemini",
        "codex" => "codex",
        "grok" => "grok",
        "cursor" => "cursor",
        "command-code" => "cmd",
        _ => "claude",
    }
}

pub(crate) fn unwrap_agent_envelope(agent: &str, raw: &str) -> String {
    if agent != "grok" && agent != "cursor" {
        return raw.to_string();
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        if let Some(text) = parsed.get("text").and_then(|v| v.as_str()) {
            return text.to_string();
        }
        if let Some(result) = parsed.get("result").and_then(|v| v.as_str()) {
            return result.to_string();
        }
    }
    raw.to_string()
}

#[cfg(test)]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandCodeModelRow {
    pub id: String,
    pub description: String,
    pub group: String,
}

#[cfg(test)]
fn parse_command_code_models_output(raw: &str) -> Vec<CommandCodeModelRow> {
    let known_groups = ["Open Source", "Anthropic", "OpenAI", "Google", "Sakana"];
    let mut current_group = "Other".to_string();
    let mut models = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("Available models")
            || trimmed.starts_with("Pass the full id")
            || trimmed.starts_with("Docs:")
        {
            continue;
        }
        if known_groups.contains(&trimmed) {
            current_group = trimmed.to_string();
            continue;
        }
        let Some((id, description)) = trimmed.split_once("  ") else {
            continue;
        };
        let id = id.trim();
        let description = description.trim();
        if id.is_empty() {
            continue;
        }
        models.push(CommandCodeModelRow {
            id: id.to_string(),
            description: description.to_string(),
            group: current_group.clone(),
        });
    }

    models
}

/// Public re-export for `unpack.rs` (and any future module) — same logic, no
/// duplication.
pub fn extract_json_from_output_pub(output: &str) -> Option<String> {
    extract_json_from_output(output)
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

    /// Generate CodeVetter's public-benchmark comparator outputs by running
    /// every `benchmark/cases/<id>/` through the REAL production review
    /// pipeline (risk tiers, specialists, coordinator, dedup) headlessly.
    /// Raw pipeline output lands in `benchmark/reviews-raw/<id>.codevetter.raw.json`;
    /// ground-truth mapping is a separate, human-checked step. Requires the
    /// `claude` CLI on PATH and burns real quota — hence ignored.
    #[test]
    #[ignore]
    fn diag_benchmark_generate_codevetter_reviews() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../benchmark");
        let cases_dir = root.join("cases");
        let out_dir = root.join("reviews-raw");
        std::fs::create_dir_all(&out_dir).expect("create reviews-raw");
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

        let mut case_ids: Vec<String> = std::fs::read_dir(&cases_dir)
            .expect("read cases dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        case_ids.sort();
        eprintln!("benchmark cases: {}", case_ids.len());

        for case_id in case_ids {
            let out_path = out_dir.join(format!("{case_id}.codevetter.raw.json"));
            if out_path.exists() {
                eprintln!("SKIP {case_id} (output exists)");
                continue;
            }
            let label: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(cases_dir.join(&case_id).join("label.json"))
                    .expect("label"),
            )
            .expect("label json");
            let source_file = label
                .get("source_file")
                .and_then(|v| v.as_str())
                .expect("source_file");
            let source = std::fs::read_to_string(cases_dir.join(&case_id).join(source_file))
                .expect("source");

            // Scratch repo: baseline commit, then the case file as the change
            // under review — exactly how a real diff reaches the pipeline.
            let tmp = std::env::temp_dir().join(format!("cv-bench-{case_id}"));
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).expect("tmp dir");
            let git = |args: &[&str]| {
                let out = StdCommand::new("git")
                    .args(args)
                    .current_dir(&tmp)
                    .env("GIT_AUTHOR_NAME", "bench")
                    .env("GIT_AUTHOR_EMAIL", "bench@local")
                    .env("GIT_COMMITTER_NAME", "bench")
                    .env("GIT_COMMITTER_EMAIL", "bench@local")
                    .output()
                    .expect("git");
                assert!(
                    out.status.success(),
                    "git {args:?} failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            };
            git(&["init", "-q"]);
            git(&["commit", "-q", "--allow-empty", "-m", "baseline"]);
            std::fs::write(tmp.join(source_file), &source).expect("write source");
            git(&["add", "."]);
            git(&["commit", "-q", "-m", "agent change under review"]);

            let conn = rusqlite::Connection::open_in_memory().expect("db");
            crate::db::schema::run_migrations(&conn).expect("migrations");
            let db = crate::DbState(std::sync::Arc::new(std::sync::Mutex::new(conn)));

            eprintln!("RUN  {case_id} ...");
            let t0 = std::time::Instant::now();
            let result = rt.block_on(run_cli_review_core(
                db,
                tmp.to_string_lossy().to_string(),
                "HEAD~1..HEAD".to_string(),
                String::new(),
                String::new(),
                Some("claude".to_string()),
                None,
                None,
            ));
            match result {
                Ok(value) => {
                    std::fs::write(&out_path, serde_json::to_string_pretty(&value).unwrap())
                        .expect("write output");
                    eprintln!(
                        "DONE {case_id} in {:.0}s — findings: {}",
                        t0.elapsed().as_secs_f64(),
                        value
                            .get("findings")
                            .and_then(|f| f.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0)
                    );
                }
                Err(error) => eprintln!("FAIL {case_id}: {error}"),
            }
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }

    #[test]
    fn changed_line_count_ignores_diff_headers() {
        let diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1,1 +1,2 @@\n-old\n+new\n+another\n";
        assert_eq!(changed_line_count(diff), 3);
    }

    #[test]
    fn trusted_review_graph_preserves_sources_and_is_explicitly_navigation_only() {
        use crate::commands::structural_graph::{
            storage::persist_snapshot,
            types::{
                GraphOrigin, GraphSourceAnchor, GraphTrust, StructuralGraphCoverage,
                StructuralGraphEdge, StructuralGraphEngineInfo, StructuralGraphNode,
                StructuralGraphSnapshot, STRUCTURAL_GRAPH_SCHEMA_VERSION,
            },
        };

        let root =
            std::env::temp_dir().join(format!("codevetter-review-graph-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).expect("create fixture repo");
        std::fs::write(root.join("src/review.rs"), "pub fn review() {}\n")
            .expect("write fixture source");
        let git = |args: &[&str]| {
            let output = StdCommand::new("git")
                .args(args)
                .current_dir(&root)
                .env("GIT_AUTHOR_NAME", "CodeVetter test")
                .env("GIT_AUTHOR_EMAIL", "test@codevetter.local")
                .env("GIT_COMMITTER_NAME", "CodeVetter test")
                .env("GIT_COMMITTER_EMAIL", "test@codevetter.local")
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["add", "src/review.rs"]);
        git(&["commit", "-q", "-m", "fixture"]);
        let head = git(&["rev-parse", "HEAD"]);

        let connection = rusqlite::Connection::open_in_memory().expect("database");
        crate::db::schema::run_migrations(&connection).expect("migrations");
        let source = GraphSourceAnchor {
            path: "src/review.rs".to_string(),
            start_line: Some(1),
            start_column: Some(1),
            end_line: Some(1),
            end_column: Some(19),
            excerpt: Some("pub fn review() {}".to_string()),
        };
        persist_snapshot(
            &connection,
            &StructuralGraphSnapshot {
                schema_version: STRUCTURAL_GRAPH_SCHEMA_VERSION,
                id: "snapshot:review-trust".to_string(),
                repo_path: root.to_string_lossy().to_string(),
                repo_head: Some(head.clone()),
                created_at: "2026-07-14T00:00:00Z".to_string(),
                engine: StructuralGraphEngineInfo {
                    id: "tree-sitter".to_string(),
                    version: "1".to_string(),
                    bundled: true,
                    syntax_aware: true,
                    supported_languages: vec!["rust".to_string()],
                },
                cursor: None,
                ignore_fingerprint: None,
                coverage: StructuralGraphCoverage {
                    discovered_files: 1,
                    indexed_files: 1,
                    ..StructuralGraphCoverage::default()
                },
                diagnostics: Vec::new(),
                communities: Vec::new(),
                files: Vec::new(),
                nodes: vec![
                    StructuralGraphNode {
                        id: "file:review".to_string(),
                        kind: "file".to_string(),
                        label: "src/review.rs".to_string(),
                        qualified_name: None,
                        path: Some("src/review.rs".to_string()),
                        detail: None,
                        language: Some("rust".to_string()),
                        community_id: None,
                        trust: GraphTrust::Extracted,
                        origin: GraphOrigin::Syntax,
                        sources: vec![source.clone()],
                    },
                    StructuralGraphNode {
                        id: "function:review".to_string(),
                        kind: "function".to_string(),
                        label: "review".to_string(),
                        qualified_name: Some("src/review.rs::review".to_string()),
                        path: Some("src/review.rs".to_string()),
                        detail: None,
                        language: Some("rust".to_string()),
                        community_id: None,
                        trust: GraphTrust::Extracted,
                        origin: GraphOrigin::Syntax,
                        sources: vec![source.clone()],
                    },
                ],
                edges: vec![StructuralGraphEdge {
                    id: "edge:file-review-function-review".to_string(),
                    from: "file:review".to_string(),
                    to: "function:review".to_string(),
                    kind: "defines".to_string(),
                    evidence: "function declaration".to_string(),
                    trust: GraphTrust::Extracted,
                    origin: GraphOrigin::Syntax,
                    sources: vec![source],
                    candidates: Vec::new(),
                }],
                metrics: Vec::new(),
                clone_groups: Vec::new(),
                truncated: false,
            },
        )
        .expect("persist graph fixture");

        let context = build_trusted_review_graph_context(
            &connection,
            &root.to_string_lossy(),
            &["src/review.rs".to_string()],
        )
        .expect("trusted graph context");
        assert_eq!(context.snapshot_id, "snapshot:review-trust");
        assert_eq!(context.current_head.as_deref(), Some(head.as_str()));
        assert!(!context.stale);
        assert!(context
            .nodes
            .iter()
            .all(|node| node.trust == GraphTrust::Extracted && !node.sources.is_empty()));
        assert_eq!(context.edges.len(), 1);
        assert_eq!(context.edges[0].sources[0].path, "src/review.rs");
        assert!(context.qualification.contains("never creates findings"));

        let prompt = render_trusted_review_graph_for_prompt(&context);
        assert!(prompt.contains("navigation only"));
        assert!(prompt.contains("extracted / syntax"));
        assert!(prompt.contains("source src/review.rs:1"));
        assert!(prompt.contains("never creates findings"));
    }

    fn bench_finding(title: &str, line: i64, severity: &str, confidence: f64) -> Value {
        json!({
            "title": title,
            "filePath": "source.ts",
            "line": line,
            "severity": severity,
            "confidence": confidence,
            "summary": "s",
        })
    }

    // Pairs below are REAL specialist outputs from the public benchmark run —
    // the calibration set for the near-duplicate rule. Keep them verbatim.
    #[test]
    fn dedupe_collapses_same_defect_different_phrasing() {
        let out = dedupe_findings(vec![
            bench_finding(
                "SQL injection via string interpolation in findUserByEmail",
                13,
                "critical",
                0.9,
            ),
            bench_finding(
                "SQL injection via string concatenation in findUserByEmail",
                13,
                "critical",
                0.8,
            ),
            bench_finding(
                "os.WriteFile error silently discarded; SaveConfig cannot signal write failure",
                12,
                "high",
                0.9,
            ),
            bench_finding(
                "SaveConfig silently swallows write failures — signature cannot report errors",
                12,
                "high",
                0.8,
            ),
        ]);
        assert_eq!(out.len(), 2, "two defects, two findings: {out:?}");
    }

    #[test]
    fn dedupe_keeps_different_defects_on_adjacent_lines() {
        let out = dedupe_findings(vec![
            bench_finding(
                "connect() uses fetch() on a postgres:// URL — cannot establish a DB connection",
                13,
                "high",
                0.9,
            ),
            bench_finding(
                "Password special characters are not URL-encoded in the connection string",
                12,
                "medium",
                0.8,
            ),
            bench_finding(
                "Passwords hashed with unsalted MD5 (account-compromise / credential-loss risk)",
                8,
                "critical",
                0.9,
            ),
            bench_finding(
                "Non-constant-time hash comparison enables timing side channel",
                12,
                "medium",
                0.7,
            ),
        ]);
        assert_eq!(out.len(), 4, "distinct defects must all survive: {out:?}");
    }

    #[test]
    fn dedupe_collapses_strong_match_across_distant_lines() {
        // Same defect anchored at the use site vs the import line.
        let out = dedupe_findings(vec![
            bench_finding(
                "Reset tokens generated from predictable java.util.Random (CWE-338)",
                11,
                "critical",
                0.9,
            ),
            bench_finding(
                "Reset tokens generated with predictable java.util.Random (CWE-338)",
                5,
                "high",
                0.8,
            ),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].get("severity").and_then(|v| v.as_str()),
            Some("critical")
        );
    }

    #[test]
    fn review_plan_keeps_trivial_diff_assumption_first() {
        let diff = "diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-old\n+new\n";
        let plan = build_review_plan(diff, &["src/a.ts".to_string()]);
        assert_eq!(plan.tier, "trivial");
        assert_eq!(plan.mode, "assumption-first");
        assert!(!plan.uses_coordinator);
        assert_eq!(
            plan.specialists.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec!["assumption-integrity", "general"]
        );
    }

    #[test]
    fn review_plan_forces_full_on_sensitive_path() {
        let diff = "diff --git a/src/auth.ts b/src/auth.ts\n--- a/src/auth.ts\n+++ b/src/auth.ts\n@@ -1 +1 @@\n-old\n+new\n";
        let plan = build_review_plan(diff, &["src/auth.ts".to_string()]);
        assert_eq!(plan.tier, "full-sensitive");
        assert_eq!(plan.mode, "specialist-full");
        assert!(plan.uses_coordinator);
        assert_eq!(
            plan.specialists.iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![
                "assumption-integrity",
                "product-safety",
                "security-boundary",
                "agent-handoff"
            ]
        );
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
        assert!(prompt.contains("Start by extracting the material assumptions"));
        assert!(prompt.contains("contradicted assumption"));
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
            Vec::new(),
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

    #[test]
    fn native_review_paths_are_bounded_qualified_context_not_claims() {
        use crate::commands::unpack_types::{RepoGraph, RepoGraphEdge, RepoGraphNode};
        let node = |id: &str, kind: &str, label: &str, path: Option<&str>| RepoGraphNode {
            id: id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            path: path.map(ToOwned::to_owned),
            detail: None,
            sources: path.into_iter().map(ToOwned::to_owned).collect(),
            source_location: None,
            community: None,
        };
        let graph = RepoGraph {
            schema_version: 2,
            nodes: vec![
                node("file", "file", "src/page.tsx", Some("src/page.tsx")),
                node("route", "route", "/billing", Some("src/page.tsx")),
            ],
            edges: vec![RepoGraphEdge {
                from: "file".to_string(),
                to: "route".to_string(),
                kind: "routes_to".to_string(),
                evidence: "route inferred from file convention".to_string(),
                sources: vec!["src/page.tsx".to_string()],
                trust: "inferred".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            }],
            truncated: false,
        };
        let paths = derive_native_review_paths(Some(&graph), &["src/page.tsx".to_string()]);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].requires_verification);
        let review_graph =
            build_review_memory_graph(&["src/page.tsx".to_string()], &[], &[], "", "", paths);
        let rendered = render_review_memory_graph_for_prompt(&review_graph);
        assert!(rendered.contains("navigation lead"));
        assert!(rendered.contains("cannot independently create a finding or verified claim"));
    }

    #[test]
    fn parse_command_code_models_output_groups_models() {
        let raw = "Available models  ·  35 models\n\nOpen Source\n\ndeepseek/deepseek-v4-flash           fast hybrid-attention reasoning (default)\n\nAnthropic\n\nclaude-sonnet-5                      best combo of speed & intelligence (recommended)\n";
        let models = parse_command_code_models_output(raw);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "deepseek/deepseek-v4-flash");
        assert_eq!(models[0].group, "Open Source");
        assert_eq!(models[1].id, "claude-sonnet-5");
        assert_eq!(models[1].group, "Anthropic");
    }

    #[cfg(unix)]
    fn executable_script(temp: &tempfile::TempDir, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = temp.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("permissions");
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_executor_is_bounded_and_rejects_malformed_output() {
        let temp = tempfile::tempdir().expect("temp");
        let valid = executable_script(
            &temp,
            "valid",
            "printf '```json\\n{\"findings\":[],\"summary\":\"ok\"}\\n```'",
        );
        let (parsed, _) = run_agent_json_with_limits(
            valid,
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_secs(1),
            1024,
        )
        .await
        .expect("valid");
        assert_eq!(parsed["summary"], "ok");

        let malformed = executable_script(&temp, "malformed", "printf 'not-json'");
        assert!(run_agent_json_with_limits(
            malformed,
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_secs(1),
            1024,
        )
        .await
        .expect_err("malformed")
        .contains("Could not find JSON"));

        let oversized = executable_script(&temp, "oversized", "head -c 256 /dev/zero");
        assert!(run_agent_json_with_limits(
            oversized,
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_secs(1),
            64,
        )
        .await
        .expect_err("oversized")
        .contains("exceeded 64 bytes"));

        assert!(run_agent_json_with_limits(
            temp.path()
                .join("missing-executor")
                .to_string_lossy()
                .into_owned(),
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_secs(1),
            1024,
        )
        .await
        .expect_err("unavailable")
        .contains("is unavailable"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_executor_timeout_and_cancellation_leave_no_child() {
        let temp = tempfile::tempdir().expect("temp");
        let timeout_script = executable_script(&temp, "timeout", "sleep 2");
        assert!(run_agent_json_with_limits(
            timeout_script,
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_millis(30),
            1024,
        )
        .await
        .expect_err("timeout")
        .contains("timed out"));

        let marker = temp.path().join("orphan-marker");
        let cancel_script = executable_script(
            &temp,
            "cancel",
            &format!("sleep 1; touch '{}'", marker.to_string_lossy()),
        );
        let task = tokio::spawn(run_agent_json_with_limits(
            cancel_script,
            "claude",
            temp.path().to_string_lossy().into_owned(),
            "review".into(),
            Duration::from_secs(5),
            1024,
        ));
        tokio::time::sleep(Duration::from_millis(40)).await;
        task.abort();
        let _ = task.await;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert!(!marker.exists(), "cancelled executor left a live child");

        let signal_marker = temp.path().join("signal-marker");
        let signal_script = executable_script(
            &temp,
            "signal-cancel",
            &format!("sleep 1; touch '{}'", signal_marker.to_string_lossy()),
        );
        let canonical = std::fs::canonicalize(temp.path())
            .expect("canonical")
            .to_string_lossy()
            .into_owned();
        let guard = register_review_cancellation(&canonical).expect("register");
        let task = tokio::spawn(run_agent_json_with_limits(
            signal_script,
            "claude",
            canonical.clone(),
            "review".into(),
            Duration::from_secs(5),
            1024,
        ));
        tokio::time::sleep(Duration::from_millis(40)).await;
        review_cancellation(&canonical).store(true, Ordering::SeqCst);
        assert!(task
            .await
            .expect("join")
            .expect_err("cancelled")
            .contains("review was cancelled"));
        drop(guard);
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert!(
            !signal_marker.exists(),
            "signaled cancellation left a live child"
        );
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

/// Per-standards-pack review usage (review count + total findings). Powers the
/// Rubrics page usage display.
#[tauri::command]
pub async fn get_standards_pack_usage(db: State<'_, DbState>) -> Result<Value, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let usage = queries::get_standards_pack_usage(&conn).map_err(|e| e.to_string())?;
    Ok(json!({ "usage": usage }))
}
