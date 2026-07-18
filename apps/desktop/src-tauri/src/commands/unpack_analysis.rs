//! Full/deferred Repo Unpacked analysis: graph, health, and history.

use crate::commands::history_summary_graph::build_history_graph;
use crate::commands::unpack_qa::{push_unique_limited, suggested_qa_flows};
use crate::commands::unpack_scan::is_binary_path;
use crate::commands::unpack_types::{
    EntrypointHint, ManifestSummary, RepoGraph, RepoGraphEdge, RepoGraphNode, RepoHealth,
    RepoHealthFile, RepoHealthFinding, RepoHistoryBrief, RepoHistoryCommit, RepoHistoryDecision,
    RepoHistoryTestHint, RepoTemporalCoupling, WorkspaceUnitSummary,
};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command as StdCommand;

const MAX_HEALTH_FILES_ANALYZED: usize = 96;
const HEALTH_FILE_READ_BYTES: usize = 48 * 1024;
const SOURCE_MARKER_READ_BYTES: usize = 80 * 1024;

const MAX_REPO_GRAPH_NODES: usize = 1024;
const MAX_REPO_GRAPH_EDGES: usize = 2048;
pub(crate) type SourcePreviewCache = HashMap<String, String>;

pub(crate) fn build_source_preview_cache(
    root: &Path,
    files: &[(String, u64)],
) -> SourcePreviewCache {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for (path, _) in files
        .iter()
        .filter(|(path, _)| should_scan_for_graph_markers(path))
        .take(300)
    {
        if seen.insert(path.clone()) {
            paths.push(path.clone());
        }
    }
    for (path, _) in files
        .iter()
        .filter(|(path, _)| should_scan_for_history_markers(path))
        .take(320)
    {
        if seen.insert(path.clone()) {
            paths.push(path.clone());
        }
    }

    paths
        .into_par_iter()
        .filter_map(|path| {
            let content = read_first_bytes(&root.join(&path), SOURCE_MARKER_READ_BYTES);
            if content.is_empty() {
                None
            } else {
                Some((path, content))
            }
        })
        .collect()
}

fn source_preview<'a>(
    root: &Path,
    path: &str,
    previews: Option<&'a SourcePreviewCache>,
    limit: usize,
) -> std::borrow::Cow<'a, str> {
    if let Some(content) = previews.and_then(|cache| cache.get(path)) {
        return std::borrow::Cow::Borrowed(content.as_str());
    }
    std::borrow::Cow::Owned(read_first_bytes(&root.join(path), limit))
}

fn graph_id(kind: &str, value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{kind}:{slug}")
}

fn push_repo_graph_node(nodes: &mut Vec<RepoGraphNode>, node: RepoGraphNode) -> bool {
    if nodes.iter().any(|existing| existing.id == node.id) {
        return true;
    }
    if nodes.len() >= MAX_REPO_GRAPH_NODES {
        return false;
    }
    nodes.push(node);
    true
}

fn push_repo_graph_edge(edges: &mut Vec<RepoGraphEdge>, edge: RepoGraphEdge) -> bool {
    if edges.iter().any(|existing| {
        existing.from == edge.from && existing.to == edge.to && existing.kind == edge.kind
    }) {
        return true;
    }
    if edges.len() >= MAX_REPO_GRAPH_EDGES {
        return false;
    }
    edges.push(edge);
    true
}

fn file_graph_node(path: &str, kind: &str, detail: &str) -> RepoGraphNode {
    RepoGraphNode {
        id: graph_id("file", path),
        kind: kind.to_string(),
        label: path.to_string(),
        path: Some(path.to_string()),
        detail: Some(detail.to_string()),
        sources: vec![path.to_string()],
        source_location: None,
        community: None,
    }
}

pub(crate) fn build_repo_graph_with_previews(
    root: &Path,
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    entrypoints: &[EntrypointHint],
    workspace_units: &[WorkspaceUnitSummary],
    previews: Option<&SourcePreviewCache>,
) -> RepoGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut truncated = false;
    let file_paths: Vec<&str> = files.iter().map(|(path, _)| path.as_str()).collect();

    add_workspace_units_to_repo_graph(workspace_units, &mut nodes, &mut edges, &mut truncated);

    for entry in entrypoints.iter().take(80) {
        if !push_repo_graph_node(
            &mut nodes,
            file_graph_node(&entry.path, "file", &entry.reason),
        ) {
            truncated = true;
        }
    }

    for manifest in manifests.iter().take(40) {
        let package_label = manifest
            .name
            .clone()
            .unwrap_or_else(|| manifest.path.clone());
        let package_id = graph_id("package", &manifest.path);
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: package_id.clone(),
                kind: "package".to_string(),
                label: package_label,
                path: Some(manifest.path.clone()),
                detail: Some(format!("{} manifest", manifest.kind)),
                sources: vec![manifest.path.clone()],
                source_location: None,
                community: None,
            },
        ) {
            truncated = true;
        }

        for script in manifest.scripts.iter().take(18) {
            let script_id = graph_id("script", &format!("{}:{script}", manifest.path));
            if !push_repo_graph_node(
                &mut nodes,
                RepoGraphNode {
                    id: script_id.clone(),
                    kind: "script".to_string(),
                    label: script.clone(),
                    path: Some(manifest.path.clone()),
                    detail: Some("package script".to_string()),
                    sources: vec![manifest.path.clone()],
                    source_location: None,
                    community: None,
                },
            ) {
                truncated = true;
            }
            if !push_repo_graph_edge(
                &mut edges,
                RepoGraphEdge {
                    from: package_id.clone(),
                    to: script_id,
                    kind: "defines".to_string(),
                    evidence: format!("{} defines npm script `{script}`", manifest.path),
                    sources: vec![manifest.path.clone()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                truncated = true;
            }
        }
    }

    for flow in suggested_qa_flows(&file_paths) {
        let Some(source) = flow.sources.first() else {
            continue;
        };
        let route_id = graph_id("route", &flow.route);
        let file_id = graph_id("file", source);
        let _ = push_repo_graph_node(&mut nodes, file_graph_node(source, "file", "route file"));
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: route_id.clone(),
                kind: "route".to_string(),
                label: flow.route.clone(),
                path: Some(source.clone()),
                detail: Some(flow.goal.clone()),
                sources: flow.sources.clone(),
                source_location: None,
                community: None,
            },
        ) {
            truncated = true;
        }
        if !push_repo_graph_edge(
            &mut edges,
            RepoGraphEdge {
                from: file_id,
                to: route_id,
                kind: "routes_to".to_string(),
                evidence: "route inferred from page file path".to_string(),
                sources: flow.sources,
                trust: "inferred".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            },
        ) {
            truncated = true;
        }
    }

    for path in file_paths.iter().filter(|path| is_test_path(path)).take(80) {
        let test_id = graph_id("test", path);
        if !push_repo_graph_node(
            &mut nodes,
            RepoGraphNode {
                id: test_id.clone(),
                kind: "test".to_string(),
                label: (*path).to_string(),
                path: Some((*path).to_string()),
                detail: Some("test/spec file".to_string()),
                sources: vec![(*path).to_string()],
                source_location: None,
                community: None,
            },
        ) {
            truncated = true;
        }
    }

    for path in file_paths
        .iter()
        .filter(|path| should_scan_for_graph_markers(path))
        .take(300)
    {
        let content = source_preview(root, path, previews, SOURCE_MARKER_READ_BYTES);
        if content.is_empty() {
            continue;
        }
        let file_id = graph_id("file", path);
        let _ = push_repo_graph_node(
            &mut nodes,
            file_graph_node(path, "file", "source marker candidate"),
        );
        scan_tauri_commands(
            path,
            content.as_ref(),
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
        scan_db_tables(
            path,
            content.as_ref(),
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
        scan_system_repo_markers(
            path,
            content.as_ref(),
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
        scan_decision_markers(
            path,
            content.as_ref(),
            &mut nodes,
            &mut edges,
            &mut truncated,
            &file_id,
        );
    }

    nodes.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.label.cmp(&b.label)));
    edges.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });

    RepoGraph {
        schema_version: 2,
        nodes,
        edges,
        truncated,
    }
}

fn add_workspace_units_to_repo_graph(
    workspace_units: &[WorkspaceUnitSummary],
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
) {
    for unit in workspace_units.iter().take(48) {
        let unit_id = graph_id("workspace", &unit.path);
        let language_summary = unit
            .languages
            .iter()
            .take(3)
            .map(|language| format!("{} {}", language.files, language.language))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sources: Vec<String> = Vec::new();
        if let Some(manifest_path) = &unit.manifest_path {
            sources.push(manifest_path.clone());
        }
        sources.extend(unit.entrypoints.iter().take(2).cloned());
        sources.extend(unit.test_files.iter().take(2).cloned());
        if sources.is_empty() && unit.path != "." {
            sources.push(unit.path.clone());
        }
        sources.sort();
        sources.dedup();

        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: unit_id.clone(),
                kind: if unit.kind == "subsystem" {
                    "subsystem".to_string()
                } else {
                    "workspace_unit".to_string()
                },
                label: unit.name.clone(),
                path: if unit.path == "." {
                    None
                } else {
                    Some(unit.path.clone())
                },
                detail: Some(format!(
                    "{} | {} files{}",
                    unit.kind.replace('_', " "),
                    unit.file_count,
                    if language_summary.is_empty() {
                        String::new()
                    } else {
                        format!(" | {language_summary}")
                    }
                )),
                sources,
                source_location: None,
                community: None,
            },
        ) {
            *truncated = true;
        }

        if let Some(manifest_path) = &unit.manifest_path {
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: unit_id.clone(),
                    to: graph_id("package", manifest_path),
                    kind: "defines".to_string(),
                    evidence: "workspace unit owns this manifest".to_string(),
                    sources: vec![manifest_path.clone()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                *truncated = true;
            }
        }

        for entrypoint in unit.entrypoints.iter().take(6) {
            let file_id = graph_id("file", entrypoint);
            let _ = push_repo_graph_node(
                nodes,
                file_graph_node(entrypoint, "file", "workspace entrypoint"),
            );
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: unit_id.clone(),
                    to: file_id,
                    kind: "entrypoint".to_string(),
                    evidence: "entrypoint belongs to this workspace unit".to_string(),
                    sources: vec![entrypoint.clone()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                *truncated = true;
            }
        }

        for test_file in unit.test_files.iter().take(6) {
            let test_id = graph_id("test", test_file);
            let _ = push_repo_graph_node(
                nodes,
                RepoGraphNode {
                    id: test_id.clone(),
                    kind: "test".to_string(),
                    label: test_file.clone(),
                    path: Some(test_file.clone()),
                    detail: Some("workspace test/spec file".to_string()),
                    sources: vec![test_file.clone()],
                    source_location: None,
                    community: None,
                },
            );
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: unit_id.clone(),
                    to: test_id,
                    kind: "tests".to_string(),
                    evidence: "test file belongs to this workspace unit".to_string(),
                    sources: vec![test_file.clone()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                *truncated = true;
            }
        }
    }
}

fn is_test_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with("_test.rs")
        || lower.contains("/tests/")
        || lower.starts_with("tests/")
}

fn should_scan_for_graph_markers(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = Path::new(&lower)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    lower.ends_with(".rs")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".sql")
        || lower.ends_with(".md")
        || lower.ends_with(".mdx")
        || lower.ends_with(".c")
        || lower.ends_with(".h")
        || lower.ends_with(".mk")
        || name == "makefile"
        || name == "kconfig"
}

pub(crate) fn build_repo_health(root: &Path, files: &[(String, u64)]) -> RepoHealth {
    build_repo_health_with_previews(root, files, None)
}

pub(crate) fn build_repo_health_with_previews(
    root: &Path,
    files: &[(String, u64)],
    previews: Option<&SourcePreviewCache>,
) -> RepoHealth {
    let churn = read_git_file_churn(root, 120);
    let test_paths: Vec<String> = files
        .iter()
        .filter(|(path, _)| is_test_path(path))
        .map(|(path, _)| path.clone())
        .collect();

    let mut candidates: Vec<(String, u64)> = files
        .iter()
        .filter(|(path, _)| should_scan_for_health(path))
        .cloned()
        .collect();
    candidates.sort_by(|a, b| {
        let churn_a = churn.get(&a.0).copied().unwrap_or(0);
        let churn_b = churn.get(&b.0).copied().unwrap_or(0);
        churn_b
            .cmp(&churn_a)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.0.cmp(&b.0))
    });

    let truncated = candidates.len() > MAX_HEALTH_FILES_ANALYZED;
    let root_buf = root.to_path_buf();
    let churn_map = churn;
    let test_paths_owned = test_paths;

    let mut analyzed: Vec<RepoHealthFile> = candidates
        .par_iter()
        .take(MAX_HEALTH_FILES_ANALYZED)
        .filter_map(|(path, bytes)| {
            let content = source_preview(&root_buf, path, previews, HEALTH_FILE_READ_BYTES);
            if content.trim().is_empty() {
                return None;
            }
            let file_churn = churn_map.get(path).copied().unwrap_or(0);
            let has_test_signal = has_test_signal_for_path(path, &test_paths_owned);
            Some(analyze_health_file(
                path,
                *bytes,
                content.as_ref(),
                file_churn,
                has_test_signal,
            ))
        })
        .collect();

    let files_with_test_signal = analyzed.iter().filter(|f| f.has_test_signal).count();
    let total_score: f64 = analyzed.iter().map(|f| f.score).sum();

    analyzed.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.churn.cmp(&a.churn))
            .then_with(|| b.lines.cmp(&a.lines))
            .then_with(|| a.path.cmp(&b.path))
    });

    let files_analyzed = analyzed.len();
    let average_score = if files_analyzed == 0 {
        10.0
    } else {
        round_one(total_score / files_analyzed as f64)
    };
    let hotspot_count = analyzed
        .iter()
        .filter(|file| file.bucket == "hotspot")
        .count();
    let top_files: Vec<RepoHealthFile> = analyzed.into_iter().take(12).collect();

    let summary = if files_analyzed == 0 {
        "No source files were eligible for deterministic health analysis.".to_string()
    } else {
        format!(
            "Deterministic scan scored {} source file{} across simple size, churn, structural, test-adjacency, and performance-risk signals. {} hotspot{} surfaced; average score is {:.1}/10. Treat these as review leads, not proof of a bug.",
            files_analyzed,
            if files_analyzed == 1 { "" } else { "s" },
            hotspot_count,
            if hotspot_count == 1 { "" } else { "s" },
            average_score
        )
    };

    RepoHealth {
        schema_version: 1,
        summary,
        average_score,
        hotspot_count,
        files_analyzed,
        files_with_test_signal,
        top_files,
        truncated,
    }
}

fn should_scan_for_health(path: &str) -> bool {
    if looks_sensitive_path(path) || is_test_path(path) {
        return false;
    }
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".rs")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".jsx")
        || lower.ends_with(".py")
        || lower.ends_with(".go")
        || lower.ends_with(".java")
        || lower.ends_with(".kt")
        || lower.ends_with(".swift")
        || lower.ends_with(".rb")
        || lower.ends_with(".php")
        || lower.ends_with(".cs")
        || lower.ends_with(".vue")
        || lower.ends_with(".svelte")
}

pub(crate) fn analyze_health_file(
    path: &str,
    bytes: u64,
    content: &str,
    churn: usize,
    has_test_signal: bool,
) -> RepoHealthFile {
    let lines = content.lines().count();
    let mut findings = Vec::new();

    if lines >= 900 {
        push_health_finding(
            &mut findings,
            path,
            "large_file",
            "Large file",
            "maintainability",
            "high",
            format!("{lines} lines in one source file; inspect whether the module hides multiple responsibilities."),
        );
    } else if lines >= 450 {
        push_health_finding(
            &mut findings,
            path,
            "large_file",
            "Large file",
            "maintainability",
            "medium",
            format!("{lines} lines in one source file; changes here may be harder to review."),
        );
    }

    let max_indent = max_indent_depth(content);
    if max_indent >= 8 {
        push_health_finding(
            &mut findings,
            path,
            "deep_nesting",
            "Deep nesting",
            "defect",
            "high",
            format!("Maximum indentation depth is about {max_indent}; deeply nested branches are bug-prone review targets."),
        );
    } else if max_indent >= 6 {
        push_health_finding(
            &mut findings,
            path,
            "deep_nesting",
            "Deep nesting",
            "defect",
            "medium",
            format!("Maximum indentation depth is about {max_indent}; inspect branch-heavy paths before risky edits."),
        );
    }

    let long_block = longest_brace_block(content);
    if long_block >= 180 {
        push_health_finding(
            &mut findings,
            path,
            "long_block",
            "Long function/block",
            "maintainability",
            "medium",
            format!("A brace-delimited block spans roughly {long_block} lines; consider extracting a helper when touching it."),
        );
    }

    if churn >= 180 {
        push_health_finding(
            &mut findings,
            path,
            "churn_hotspot",
            "High churn",
            "defect",
            "high",
            format!("Recent git history shows {churn} changed lines in this file; combine review with history and tests."),
        );
    } else if churn >= 60 {
        push_health_finding(
            &mut findings,
            path,
            "churn_hotspot",
            "Moderate churn",
            "defect",
            "medium",
            format!("Recent git history shows {churn} changed lines in this file; it is a change hotspot."),
        );
    }

    if !has_test_signal && (churn >= 50 || lines >= 400) {
        push_health_finding(
            &mut findings,
            path,
            "untested_hotspot",
            "No adjacent test signal",
            "defect",
            "medium",
            "No obvious sibling test/spec file was found for a large or churny source file."
                .to_string(),
        );
    }

    if detects_io_in_loop(content) {
        push_health_finding(
            &mut findings,
            path,
            "io_in_loop",
            "I/O inside loop",
            "performance",
            "medium",
            "Loop-shaped code appears near filesystem, subprocess, database, or network I/O; inspect for N+1 or repeated setup work.".to_string(),
        );
    }

    if detects_boundary_shell_or_fs(content) {
        push_health_finding(
            &mut findings,
            path,
            "io_boundary",
            "I/O or process boundary",
            "defect",
            "low",
            "File touches filesystem, network, database, or subprocess boundaries; changes need concrete runtime proof.".to_string(),
        );
    }

    let mut score: f64 = 10.0;
    let mut category_deductions: HashMap<String, f64> = HashMap::new();
    for finding in &findings {
        let deduction: f64 = match finding.severity.as_str() {
            "high" => 1.2,
            "medium" => 0.7,
            _ => 0.3,
        };
        let cap: f64 = match finding.dimension.as_str() {
            "defect" => 3.5,
            "maintainability" => 2.5,
            "performance" => 1.0,
            _ => 1.0,
        };
        let used = category_deductions
            .entry(finding.dimension.clone())
            .or_insert(0.0);
        let allowed = (cap - *used).max(0.0);
        let applied = deduction.min(allowed);
        *used += applied;
        score -= applied;
    }
    score = round_one(score.clamp(1.0, 10.0));
    let bucket = if score <= 6.5 {
        "hotspot"
    } else if score <= 8.0 {
        "watch"
    } else {
        "healthy"
    }
    .to_string();

    let mut refactoring_targets = Vec::new();
    if lines >= 900 || long_block >= 180 {
        refactoring_targets
            .push("Split file or extract helper around the largest cohesive flow.".to_string());
    }
    if max_indent >= 6 {
        refactoring_targets
            .push("Flatten guard-heavy branches before adding behavior.".to_string());
    }
    if detects_io_in_loop(content) {
        refactoring_targets
            .push("Hoist repeated I/O or batch loop work where behavior allows.".to_string());
    }
    refactoring_targets.truncate(3);

    RepoHealthFile {
        path: path.to_string(),
        score,
        bucket,
        lines,
        bytes,
        churn,
        has_test_signal,
        findings,
        refactoring_targets,
    }
}

fn push_health_finding(
    findings: &mut Vec<RepoHealthFinding>,
    path: &str,
    id: &str,
    label: &str,
    dimension: &str,
    severity: &str,
    detail: String,
) {
    if findings.iter().any(|finding| finding.id == id) {
        return;
    }
    findings.push(RepoHealthFinding {
        id: id.to_string(),
        label: label.to_string(),
        dimension: dimension.to_string(),
        severity: severity.to_string(),
        detail,
        sources: vec![path.to_string()],
    });
}

fn max_indent_depth(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('#')
        })
        .map(|line| {
            line.chars()
                .take_while(|ch| *ch == ' ' || *ch == '\t')
                .count()
                / 2
        })
        .max()
        .unwrap_or(0)
}

fn longest_brace_block(content: &str) -> usize {
    let mut stack: Vec<usize> = Vec::new();
    let mut longest = 0usize;
    for (idx, line) in content.lines().enumerate() {
        for ch in line.chars() {
            if ch == '{' {
                stack.push(idx);
            } else if ch == '}' {
                if let Some(start) = stack.pop() {
                    longest = longest.max(idx.saturating_sub(start) + 1);
                }
            }
        }
    }
    longest
}

fn detects_io_in_loop(content: &str) -> bool {
    let mut loop_window = 0usize;
    for line in content.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.starts_with("for ")
            || lower.starts_with("while ")
            || lower.contains(".map(")
            || lower.contains(".foreach(")
            || lower.contains("for_each(")
        {
            loop_window = 18;
        } else {
            loop_window = loop_window.saturating_sub(1);
        }
        if loop_window > 0 && looks_like_io_call(&lower) {
            return true;
        }
    }
    false
}

fn detects_boundary_shell_or_fs(content: &str) -> bool {
    content
        .lines()
        .take(900)
        .map(|line| line.trim().to_ascii_lowercase())
        .any(|line| looks_like_io_call(&line))
}

fn looks_like_io_call(line: &str) -> bool {
    line.contains("command::new")
        || line.contains("std::process")
        || line.contains("child_process")
        || line.contains("subprocess.")
        || line.contains(".spawn(")
        || line.contains("fs::")
        || line.contains("std::fs")
        || line.contains("read_to_string")
        || line.contains("file::open")
        || line.contains("fetch(")
        || line.contains("axios.")
        || line.contains("request(")
        || line.contains(".execute(")
        || line.contains(".query(")
        || line.contains("sqlx::")
        || line.contains("rusqlite")
}

fn has_test_signal_for_path(path: &str, test_paths: &[String]) -> bool {
    if is_test_path(path) {
        return true;
    }
    let stem = Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem.is_empty() {
        return false;
    }
    test_paths.iter().any(|test_path| {
        let lower = test_path.to_ascii_lowercase();
        lower.contains(&format!("{stem}.test"))
            || lower.contains(&format!("{stem}.spec"))
            || lower.contains(&format!("{stem}_test"))
            || lower.contains(&format!("/{stem}/"))
    })
}

fn read_git_file_churn(root: &Path, limit: usize) -> HashMap<String, usize> {
    if !git_head_has_parent(root) {
        return HashMap::new();
    }

    let output = StdCommand::new("git")
        .args([
            "log",
            &format!("-n{limit}"),
            "--numstat",
            "--format=commit %H",
            "--",
            ".",
        ])
        .current_dir(root)
        .output();

    let Ok(output) = output else {
        return HashMap::new();
    };
    if !output.status.success() {
        return HashMap::new();
    }

    let mut churn = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.starts_with("commit ") || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let added = parts
            .next()
            .and_then(|part| part.parse::<usize>().ok())
            .unwrap_or(0);
        let deleted = parts
            .next()
            .and_then(|part| part.parse::<usize>().ok())
            .unwrap_or(0);
        let Some(path) = parts.next() else {
            continue;
        };
        if path.is_empty() || is_binary_path(path) {
            continue;
        }
        *churn.entry(path.to_string()).or_insert(0) += added + deleted;
    }
    churn
}

fn git_head_has_parent(root: &Path) -> bool {
    let output = StdCommand::new("git")
        .args(["rev-list", "--parents", "-n", "1", "HEAD"])
        .current_dir(root)
        .output();

    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .count()
        > 1
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn scan_tauri_commands(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    let mut pending_command_attr = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[tauri::command") {
            pending_command_attr = true;
            continue;
        }
        if !pending_command_attr {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("pub async fn ")
            .or_else(|| trimmed.strip_prefix("pub fn "))
            .or_else(|| trimmed.strip_prefix("async fn "))
            .or_else(|| trimmed.strip_prefix("fn "))
        {
            let name = rest
                .split(|ch: char| ch == '(' || ch.is_whitespace())
                .next()
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                pending_command_attr = false;
                continue;
            }
            let command_id = graph_id("tauri_command", name);
            if !push_repo_graph_node(
                nodes,
                RepoGraphNode {
                    id: command_id.clone(),
                    kind: "tauri_command".to_string(),
                    label: name.to_string(),
                    path: Some(path.to_string()),
                    detail: Some("Tauri command boundary".to_string()),
                    sources: vec![path.to_string()],
                    source_location: None,
                    community: None,
                },
            ) {
                *truncated = true;
            }
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: file_id.to_string(),
                    to: command_id,
                    kind: "defines".to_string(),
                    evidence: "function has #[tauri::command] attribute".to_string(),
                    sources: vec![path.to_string()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                *truncated = true;
            }
            pending_command_attr = false;
        }
    }
}

fn scan_db_tables(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    for line in content.lines() {
        let upper = line.to_ascii_uppercase();
        let Some(idx) = upper.find("CREATE TABLE") else {
            continue;
        };
        let after = line[idx + "CREATE TABLE".len()..]
            .trim()
            .trim_start_matches("IF NOT EXISTS")
            .trim();
        let table = after
            .split(|ch: char| ch == '(' || ch.is_whitespace())
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .trim_matches('`')
            .trim();
        if table.is_empty() {
            continue;
        }
        let table_id = graph_id("db_table", table);
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: table_id.clone(),
                kind: "db_table".to_string(),
                label: table.to_string(),
                path: Some(path.to_string()),
                detail: Some("database table".to_string()),
                sources: vec![path.to_string()],
                source_location: None,
                community: None,
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: table_id,
                kind: "persists_to".to_string(),
                evidence: "CREATE TABLE statement".to_string(),
                sources: vec![path.to_string()],
                trust: "extracted".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            },
        ) {
            *truncated = true;
        }
    }
}

fn scan_system_repo_markers(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    let lower = path.to_ascii_lowercase();
    let name = Path::new(&lower)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    if name == "kconfig" || lower.ends_with(".kconfig") {
        scan_kconfig_symbols(path, content, nodes, edges, truncated, file_id);
    }
    if name == "makefile" || lower.ends_with(".mk") {
        scan_makefile_targets(path, content, nodes, edges, truncated, file_id);
    }
    if lower.ends_with(".c") || lower.ends_with(".h") {
        scan_c_symbols(path, content, nodes, edges, truncated, file_id);
    }
}

fn scan_kconfig_symbols(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    for line in content.lines().take(900) {
        let trimmed = line.trim();
        let symbol = trimmed
            .strip_prefix("config ")
            .or_else(|| trimmed.strip_prefix("menuconfig "))
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap_or("");
        if symbol.is_empty() {
            continue;
        }
        let id = graph_id("kconfig", symbol);
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: id.clone(),
                kind: "kconfig_symbol".to_string(),
                label: symbol.to_string(),
                path: Some(path.to_string()),
                detail: Some("Kconfig feature/configuration symbol".to_string()),
                sources: vec![path.to_string()],
                source_location: None,
                community: None,
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: id,
                kind: "defines".to_string(),
                evidence: "Kconfig config/menuconfig declaration".to_string(),
                sources: vec![path.to_string()],
                trust: "extracted".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            },
        ) {
            *truncated = true;
        }
    }
}

fn scan_makefile_targets(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    let mut emitted = 0usize;
    for line in content.lines().take(900) {
        if emitted >= 24 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('#') || !trimmed.contains("+=") {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once("+=") else {
            continue;
        };
        let lhs = lhs.trim();
        if !(lhs.starts_with("obj-")
            || lhs.starts_with("lib-")
            || lhs.starts_with("hostprogs")
            || lhs.starts_with("targets"))
        {
            continue;
        }
        for target in rhs.split_whitespace().take(4) {
            let target = target.trim_matches('\\').trim();
            if target.is_empty() {
                continue;
            }
            let id = graph_id("build_target", &format!("{path}:{target}"));
            if !push_repo_graph_node(
                nodes,
                RepoGraphNode {
                    id: id.clone(),
                    kind: "build_target".to_string(),
                    label: target.to_string(),
                    path: Some(path.to_string()),
                    detail: Some(format!("Makefile target from `{lhs}`")),
                    sources: vec![path.to_string()],
                    source_location: None,
                    community: None,
                },
            ) {
                *truncated = true;
            }
            if !push_repo_graph_edge(
                edges,
                RepoGraphEdge {
                    from: file_id.to_string(),
                    to: id,
                    kind: "builds".to_string(),
                    evidence: format!("Makefile appends target via `{lhs} +=`"),
                    sources: vec![path.to_string()],
                    trust: "extracted".to_string(),
                    origin: "codevetter".to_string(),
                    confidence_label: None,
                },
            ) {
                *truncated = true;
            }
            emitted += 1;
            if emitted >= 24 {
                break;
            }
        }
    }
}

fn scan_c_symbols(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    let mut emitted = 0usize;
    for line in content.lines().take(1200) {
        if emitted >= 8 {
            break;
        }
        let trimmed = line.trim();
        let symbol = exported_c_symbol(trimmed).or_else(|| c_function_symbol(trimmed));
        let Some(symbol) = symbol else {
            continue;
        };
        let id = graph_id("c_symbol", &format!("{path}:{symbol}"));
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: id.clone(),
                kind: "c_symbol".to_string(),
                label: symbol.clone(),
                path: Some(path.to_string()),
                detail: Some("C function/export symbol".to_string()),
                sources: vec![path.to_string()],
                source_location: None,
                community: None,
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: id,
                kind: "defines".to_string(),
                evidence: "C symbol declaration/definition".to_string(),
                sources: vec![path.to_string()],
                trust: "extracted".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            },
        ) {
            *truncated = true;
        }
        emitted += 1;
    }
}

fn exported_c_symbol(line: &str) -> Option<String> {
    let start = line.find("EXPORT_SYMBOL")?;
    let after = &line[start..];
    let open = after.find('(')?;
    let close = after[open + 1..].find(')')? + open + 1;
    let symbol = after[open + 1..close].trim();
    if is_identifier_like(symbol) {
        Some(symbol.to_string())
    } else {
        None
    }
}

fn c_function_symbol(line: &str) -> Option<String> {
    if line.ends_with(';')
        || line.starts_with('#')
        || line.starts_with("typedef ")
        || line.starts_with("return ")
        || line.starts_with("if ")
        || line.starts_with("for ")
        || line.starts_with("while ")
        || line.starts_with("switch ")
        || !line.contains('(')
        || !line.contains(')')
        || !line.contains('{')
    {
        return None;
    }
    let before_open = line.split('(').next()?.trim();
    let symbol = before_open
        .split_whitespace()
        .last()
        .unwrap_or("")
        .trim_start_matches('*');
    if is_identifier_like(symbol) {
        Some(symbol.to_string())
    } else {
        None
    }
}

fn is_identifier_like(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn scan_decision_markers(
    path: &str,
    content: &str,
    nodes: &mut Vec<RepoGraphNode>,
    edges: &mut Vec<RepoGraphEdge>,
    truncated: &mut bool,
    file_id: &str,
) {
    for (idx, line) in content.lines().enumerate().take(600) {
        let marker = ["WHY:", "DECISION:", "TRADEOFF:"]
            .iter()
            .find(|marker| line.contains(**marker));
        let Some(marker) = marker else {
            continue;
        };
        let detail = line
            .split_once(marker)
            .map(|(_, rest)| rest.trim())
            .unwrap_or(line.trim())
            .chars()
            .take(160)
            .collect::<String>();
        let source = format!("{path}#L{}", idx + 1);
        let decision_id = graph_id("decision", &source);
        if !push_repo_graph_node(
            nodes,
            RepoGraphNode {
                id: decision_id.clone(),
                kind: "decision".to_string(),
                label: marker.trim_end_matches(':').to_ascii_lowercase(),
                path: Some(path.to_string()),
                detail: Some(detail),
                sources: vec![source.clone()],
                source_location: None,
                community: None,
            },
        ) {
            *truncated = true;
        }
        if !push_repo_graph_edge(
            edges,
            RepoGraphEdge {
                from: file_id.to_string(),
                to: decision_id,
                kind: "decided_by".to_string(),
                evidence: format!("{marker} marker"),
                sources: vec![source],
                trust: "extracted".to_string(),
                origin: "codevetter".to_string(),
                confidence_label: None,
            },
        ) {
            *truncated = true;
        }
    }
}

pub(crate) fn build_history_brief(
    root: &Path,
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
) -> RepoHistoryBrief {
    build_history_brief_with_previews(root, files, manifests, None)
}

pub(crate) fn build_history_brief_with_previews(
    root: &Path,
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    previews: Option<&SourcePreviewCache>,
) -> RepoHistoryBrief {
    let commits = read_recent_git_commits(root, 12);
    let mut decisions = collect_history_decisions(root, files, 16, previews);
    let mut test_hints = collect_history_test_hints(files, manifests, 16);
    let mut temporal_couplings = if files.len() >= 3_000 {
        Vec::new()
    } else {
        read_temporal_couplings(root, 80, 8)
    };
    let mut sources = Vec::new();
    let mut truncated = false;

    if decisions.len() > 12 {
        decisions.truncate(12);
        truncated = true;
    }
    if test_hints.len() > 12 {
        test_hints.truncate(12);
        truncated = true;
    }
    if temporal_couplings.len() > 8 {
        temporal_couplings.truncate(8);
        truncated = true;
    }

    for decision in &decisions {
        push_unique_limited(&mut sources, decision.source.clone(), 24);
    }
    for hint in &test_hints {
        push_unique_limited(&mut sources, hint.path.clone(), 24);
    }
    for coupling in &temporal_couplings {
        for file in coupling.files.iter().take(2) {
            push_unique_limited(&mut sources, file.clone(), 24);
        }
    }
    for manifest in manifests.iter().take(4) {
        push_unique_limited(&mut sources, manifest.path.clone(), 24);
    }

    let summary = if commits.is_empty()
        && decisions.is_empty()
        && test_hints.is_empty()
        && temporal_couplings.is_empty()
    {
        "No recent git commits, decision markers, or test hints were available from the bounded local scan.".to_string()
    } else {
        format!(
            "Local history brief captured {} recent commit{}, {} decision marker{}, {} test hint{}, and {} co-change cluster{} for Repo Unpacked. Treat commit subjects as leads and rely on cited files for durable constraints.",
            commits.len(),
            if commits.len() == 1 { "" } else { "s" },
            decisions.len(),
            if decisions.len() == 1 { "" } else { "s" },
            test_hints.len(),
            if test_hints.len() == 1 { "" } else { "s" },
            temporal_couplings.len(),
            if temporal_couplings.len() == 1 { "" } else { "s" },
        )
    };

    let mut brief = RepoHistoryBrief {
        schema_version: 2,
        summary,
        recent_commits: commits,
        decisions,
        test_hints,
        temporal_couplings,
        graph: Default::default(),
        sources,
        truncated,
    };
    brief.graph = build_history_graph(&brief);
    brief.truncated |= brief.graph.truncated;
    brief
}

fn read_temporal_couplings(
    root: &Path,
    commit_limit: usize,
    result_limit: usize,
) -> Vec<RepoTemporalCoupling> {
    let output = StdCommand::new("git")
        .args([
            "log",
            &format!("-n{commit_limit}"),
            "--name-only",
            "--pretty=format:%x1e%H",
            "--",
        ])
        .current_dir(root)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_temporal_coupling_log(&String::from_utf8_lossy(&output.stdout), result_limit)
}

pub(crate) fn parse_temporal_coupling_log(
    raw: &str,
    result_limit: usize,
) -> Vec<RepoTemporalCoupling> {
    let mut pair_counts: HashMap<(String, String), (usize, Option<String>)> = HashMap::new();

    for record in raw.split('\x1e') {
        let mut lines = record
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());
        let Some(commit) = lines.next() else {
            continue;
        };
        let mut changed = Vec::new();
        let mut too_broad = false;
        for path in lines.filter(|path| is_temporal_coupling_path(path)) {
            if changed.len() >= 25 {
                too_broad = true;
                continue;
            }
            changed.push(path.to_string());
        }
        changed.sort();
        changed.dedup();
        if too_broad || changed.len() < 2 || changed.len() > 24 {
            continue;
        }

        for i in 0..changed.len() {
            for j in (i + 1)..changed.len() {
                let key = (changed[i].clone(), changed[j].clone());
                let entry = pair_counts.entry(key).or_insert((0, None));
                entry.0 += 1;
                if entry.1.is_none() {
                    entry.1 = Some(commit.chars().take(12).collect());
                }
            }
        }
    }

    let mut pairs = pair_counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= 2)
        .map(|((left, right), (commit_count, last_commit))| RepoTemporalCoupling {
            files: vec![left, right],
            commit_count,
            last_commit,
            reason: format!(
                "These files changed together in {commit_count} recent commits; inspect both when either side moves."
            ),
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| {
        b.commit_count
            .cmp(&a.commit_count)
            .then_with(|| a.files.cmp(&b.files))
    });
    pairs.truncate(result_limit);
    pairs
}

fn is_temporal_coupling_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.is_empty()
        || lower.contains("/node_modules/")
        || lower.contains("/target/")
        || lower.contains("/vendor/")
        || lower.contains("/dist/")
        || lower.contains("/build/")
        || lower.ends_with(".lock")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("package-lock.json")
        || lower.ends_with("yarn.lock")
        || is_binary_path(&lower)
        || looks_sensitive_path(&lower)
    {
        return false;
    }

    should_scan_for_graph_markers(&lower)
        || lower.ends_with(".json")
        || lower.ends_with(".toml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
}

fn read_recent_git_commits(root: &Path, limit: usize) -> Vec<RepoHistoryCommit> {
    let output = StdCommand::new("git")
        .args([
            "log",
            &format!("-n{limit}"),
            "--date=short",
            "--pretty=format:%x1e%H%x1f%ad%x1f%s",
            "--name-only",
            "--",
        ])
        .current_dir(root)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_git_commit_records(&String::from_utf8_lossy(&output.stdout), limit)
}

pub(crate) fn parse_git_commit_records(raw: &str, limit: usize) -> Vec<RepoHistoryCommit> {
    raw.split('\x1e')
        .filter_map(|record| {
            let mut lines = record
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty());
            let mut commit = parse_git_commit_line(lines.next()?)?;
            commit.files = lines
                .filter(|path| is_temporal_coupling_path(path))
                .take(24)
                .map(String::from)
                .collect();
            commit.files.sort();
            commit.files.dedup();
            Some(commit)
        })
        .take(limit)
        .collect()
}

pub(crate) fn parse_git_commit_line(line: &str) -> Option<RepoHistoryCommit> {
    let mut parts = line.splitn(3, '\x1f');
    let sha = parts.next()?.trim();
    let date = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let subject = parts.next()?.trim();
    if sha.is_empty() || subject.is_empty() {
        return None;
    }

    Some(RepoHistoryCommit {
        sha: sha.chars().take(12).collect(),
        date: date.map(String::from),
        subject: subject.chars().take(180).collect(),
        files: Vec::new(),
    })
}

fn collect_history_decisions(
    root: &Path,
    files: &[(String, u64)],
    limit: usize,
    previews: Option<&SourcePreviewCache>,
) -> Vec<RepoHistoryDecision> {
    let mut decisions = Vec::new();
    for (path, _) in files
        .iter()
        .filter(|(path, _)| should_scan_for_history_markers(path))
        .take(320)
    {
        if decisions.len() >= limit {
            break;
        }
        let content = source_preview(root, path, previews, SOURCE_MARKER_READ_BYTES);
        if content.is_empty() {
            continue;
        }
        for (idx, line) in content.as_ref().lines().enumerate().take(700) {
            if decisions.len() >= limit {
                break;
            }
            let marker = ["WHY:", "DECISION:", "TRADEOFF:"]
                .iter()
                .find(|marker| line.contains(**marker));
            let Some(marker) = marker else {
                continue;
            };
            let text = line
                .split_once(marker)
                .map(|(_, rest)| rest.trim())
                .unwrap_or(line.trim())
                .chars()
                .take(180)
                .collect::<String>();
            if text.is_empty() {
                continue;
            }
            decisions.push(RepoHistoryDecision {
                marker: marker.trim_end_matches(':').to_ascii_lowercase(),
                text,
                source: format!("{path}#L{}", idx + 1),
            });
        }
    }
    decisions
}

fn should_scan_for_history_markers(path: &str) -> bool {
    should_scan_for_graph_markers(path) && !looks_sensitive_path(path)
}

fn looks_sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let basename = Path::new(&lower)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    basename == ".env"
        || basename.contains("credential")
        || basename.contains("secret")
        || basename.ends_with(".pem")
        || basename.ends_with(".key")
        || basename == "id_rsa"
        || basename == "id_ed25519"
        || lower.contains("/.ssh/")
        || lower.contains("/secrets/")
        || lower.contains("/credentials/")
        || lower.contains(".production.")
        || lower.contains("/production/")
}

fn collect_history_test_hints(
    files: &[(String, u64)],
    manifests: &[ManifestSummary],
    limit: usize,
) -> Vec<RepoHistoryTestHint> {
    let mut hints = Vec::new();
    for manifest in manifests
        .iter()
        .filter(|manifest| !manifest.scripts.is_empty())
    {
        for script in &manifest.scripts {
            let lower = script.to_ascii_lowercase();
            if lower.contains("test")
                || lower.contains("lint")
                || lower.contains("check")
                || lower.contains("e2e")
                || lower.contains("playwright")
            {
                hints.push(RepoHistoryTestHint {
                    path: manifest.path.clone(),
                    reason: format!("package script `{script}` is a likely verification command"),
                });
                if hints.len() >= limit {
                    return hints;
                }
            }
        }
    }

    for (path, _) in files.iter().filter(|(path, _)| is_test_path(path)) {
        if hints.iter().any(|hint| hint.path == *path) {
            continue;
        }
        hints.push(RepoHistoryTestHint {
            path: path.clone(),
            reason: "test/spec file anchors expected behavior".to_string(),
        });
        if hints.len() >= limit {
            break;
        }
    }

    hints
}

fn read_first_bytes(path: &Path, limit: usize) -> String {
    use std::io::Read;
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let mut buf = vec![0u8; limit];
    let n = file.read(&mut buf).unwrap_or(0);
    buf.truncate(n);
    String::from_utf8_lossy(&buf).to_string()
}

#[cfg(test)]
mod tests {
    use super::detects_io_in_loop;

    #[test]
    fn io_loop_window_saturates_and_preserves_boundary() {
        let inside_window = format!(
            "for item in items {{\n{}std::fs::read_to_string(path);",
            "let value = item;\n".repeat(16)
        );
        assert!(detects_io_in_loop(&inside_window));

        let outside_window = format!(
            "for item in items {{\n{}std::fs::read_to_string(path);",
            "let value = item;\n".repeat(17)
        );
        assert!(!detects_io_in_loop(&outside_window));

        assert!(!detects_io_in_loop(
            "let a = 1;\nlet b = 2;\nstd::fs::read_to_string(path);"
        ));
    }
}
