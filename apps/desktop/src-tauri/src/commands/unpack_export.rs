//! Repo Unpacked export renderers.

use crate::commands::unpack_types::{RepoInventory, ReportSection, UnpackReport};

// ─── Export helpers ─────────────────────────────────────────────────────────

pub(crate) fn render_markdown(
    repo_name: &str,
    created_at: &str,
    agent: Option<&str>,
    model: Option<&str>,
    report: &UnpackReport,
    inventory: Option<&RepoInventory>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Repo Unpacked — {}\n\n", repo_name));
    out.push_str(&format!("_Generated: {}", created_at));
    if let Some(a) = agent {
        out.push_str(&format!(" · agent: {}", a));
    }
    if let Some(m) = model {
        out.push_str(&format!(" · model: {}", m));
    }
    out.push_str("_\n\n");

    if let Some(o) = &report.overview {
        out.push_str(&format!("> {}\n\n", o));
    }

    if let Some(inv) = inventory {
        out.push_str(&format!(
            "**Stack:** {}\n\n",
            if inv.stack_tags.is_empty() {
                "—".to_string()
            } else {
                inv.stack_tags.join(", ")
            }
        ));
        out.push_str(&format!(
            "**Files scanned:** {} ({} skipped, {} bytes)\n\n",
            inv.files_scanned, inv.files_skipped, inv.bytes_scanned
        ));
        out.push_str(&format!(
            "**Synthetic QA readiness:** {} / 100 ({}) — {}\n\n",
            inv.qa_readiness.score, inv.qa_readiness.status, inv.qa_readiness.summary
        ));
        if !inv.qa_readiness.signals.is_empty() {
            out.push_str("### Synthetic QA Signals\n\n");
            for signal in &inv.qa_readiness.signals {
                out.push_str(&format!(
                    "- **{}:** {} — {}\n",
                    signal.label, signal.status, signal.detail
                ));
                if !signal.sources.is_empty() {
                    let srcs: Vec<String> =
                        signal.sources.iter().map(|s| format!("`{s}`")).collect();
                    out.push_str(&format!("  - sources: {}\n", srcs.join(", ")));
                }
            }
            out.push('\n');
        }
        if !inv.qa_readiness.suggested_flows.is_empty() {
            out.push_str("### Suggested Synthetic QA Flows\n\n");
            for flow in &inv.qa_readiness.suggested_flows {
                let srcs: Vec<String> = flow.sources.iter().map(|s| format!("`{s}`")).collect();
                out.push_str(&format!(
                    "- `{}` — {}{}{}\n",
                    flow.route,
                    flow.goal,
                    if srcs.is_empty() { "" } else { " (sources: " },
                    if srcs.is_empty() {
                        String::new()
                    } else {
                        format!("{})", srcs.join(", "))
                    }
                ));
            }
            out.push('\n');
        }
        if !inv.repo_graph.nodes.is_empty() {
            out.push_str(&format!(
                "### Repo Memory Graph\n\nSchema v{} · {} nodes · {} edges{}\n\n",
                inv.repo_graph.schema_version,
                inv.repo_graph.nodes.len(),
                inv.repo_graph.edges.len(),
                if inv.repo_graph.truncated {
                    " · truncated"
                } else {
                    ""
                }
            ));
            for node in inv.repo_graph.nodes.iter().take(20) {
                out.push_str(&format!("- **{}** `{}`", node.kind, node.label));
                if let Some(path) = &node.path {
                    out.push_str(&format!(" — `{path}`"));
                }
                if let Some(detail) = &node.detail {
                    out.push_str(&format!(" — {detail}"));
                }
                out.push('\n');
            }
            for edge in inv.repo_graph.edges.iter().take(20) {
                out.push_str(&format!(
                    "- `{}` -> `{}` ({}) — {}\n",
                    edge.from, edge.to, edge.kind, edge.evidence
                ));
            }
            out.push('\n');
        }
        if !inv.history_brief.recent_commits.is_empty()
            || !inv.history_brief.decisions.is_empty()
            || !inv.history_brief.test_hints.is_empty()
            || !inv.history_brief.temporal_couplings.is_empty()
        {
            out.push_str(&format!(
                "### Codebase History Brief\n\nSchema v{}{} · {}\n\n",
                inv.history_brief.schema_version,
                if inv.history_brief.truncated {
                    " · truncated"
                } else {
                    ""
                },
                inv.history_brief.summary
            ));
            if !inv.history_brief.recent_commits.is_empty() {
                out.push_str("**Recent commits**\n\n");
                for commit in inv.history_brief.recent_commits.iter().take(8) {
                    out.push_str(&format!(
                        "- `{}`{} — {}\n",
                        commit.sha,
                        commit
                            .date
                            .as_deref()
                            .map(|date| format!(" {date}"))
                            .unwrap_or_default(),
                        commit.subject
                    ));
                }
                out.push('\n');
            }
            if !inv.history_brief.decisions.is_empty() {
                out.push_str("**Decision markers**\n\n");
                for decision in inv.history_brief.decisions.iter().take(10) {
                    out.push_str(&format!(
                        "- **{}** `{}` — {}\n",
                        decision.marker, decision.source, decision.text
                    ));
                }
                out.push('\n');
            }
            if !inv.history_brief.test_hints.is_empty() {
                out.push_str("**Verification hints**\n\n");
                for hint in inv.history_brief.test_hints.iter().take(10) {
                    out.push_str(&format!("- `{}` — {}\n", hint.path, hint.reason));
                }
                out.push('\n');
            }
            if !inv.history_brief.temporal_couplings.is_empty() {
                out.push_str("**Co-change clusters**\n\n");
                for coupling in inv.history_brief.temporal_couplings.iter().take(8) {
                    out.push_str(&format!(
                        "- `{}` — {} commit{}{}; {}\n",
                        coupling.files.join("` + `"),
                        coupling.commit_count,
                        if coupling.commit_count == 1 { "" } else { "s" },
                        coupling
                            .last_commit
                            .as_deref()
                            .map(|commit| format!("; latest `{commit}`"))
                            .unwrap_or_default(),
                        coupling.reason
                    ));
                }
                out.push('\n');
            }
        }

        if inv.repo_health.files_analyzed > 0 {
            out.push_str("## Deterministic Repo Health\n\n");
            out.push_str(&format!(
                "{}\n\nAverage score: {:.1}/10 · hotspots: {} · files analyzed: {}{}\n\n",
                inv.repo_health.summary,
                inv.repo_health.average_score,
                inv.repo_health.hotspot_count,
                inv.repo_health.files_analyzed,
                if inv.repo_health.truncated {
                    " · truncated"
                } else {
                    ""
                }
            ));
            for file in inv.repo_health.top_files.iter().take(10) {
                out.push_str(&format!(
                    "- `{}` — {:.1}/10 `{}` · {} lines · churn {}\n",
                    file.path, file.score, file.bucket, file.lines, file.churn
                ));
                for finding in file.findings.iter().take(4) {
                    out.push_str(&format!(
                        "  - {} [{}:{}] {}\n",
                        finding.label, finding.dimension, finding.severity, finding.detail
                    ));
                }
                for target in file.refactoring_targets.iter().take(2) {
                    out.push_str(&format!("  - refactor lead: {target}\n"));
                }
            }
            out.push('\n');
        }
    }

    let render_section = |out: &mut String, sec: &Option<ReportSection>| {
        let Some(sec) = sec else { return };
        out.push_str(&format!("## {}\n\n", sec.title));
        if !sec.summary.is_empty() {
            out.push_str(&format!("{}\n\n", sec.summary));
        }
        for c in &sec.claims {
            let kind_marker = match c.kind.as_deref() {
                Some("inference") => " _(inference)_",
                _ => "",
            };
            out.push_str(&format!("- {}{}\n", c.claim, kind_marker));
            if !c.sources.is_empty() {
                let srcs: Vec<String> = c.sources.iter().map(|s| format!("`{}`", s)).collect();
                out.push_str(&format!("  - sources: {}\n", srcs.join(", ")));
            }
        }
        out.push('\n');
    };

    render_section(&mut out, &report.system_map);
    render_section(&mut out, &report.feature_catalog);
    render_section(&mut out, &report.data_flow);
    render_section(&mut out, &report.behavior_traces);
    render_section(&mut out, &report.testing_signals);
    render_section(&mut out, &report.risk_map);
    render_section(&mut out, &report.extension_points);
    render_section(&mut out, &report.agent_handoff);

    if let Some(prompt) = &report.agent_prompt {
        out.push_str("## Agent Handoff Prompt\n\n");
        out.push_str("```text\n");
        out.push_str(prompt);
        out.push_str("\n```\n");
    }

    out
}

pub(crate) fn render_agent_context_sidecar(
    repo_name: &str,
    created_at: &str,
    inventory: &RepoInventory,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Agent Context Sidecar — {repo_name}\n\n"));
    out.push_str(&format!(
        "_Generated: {created_at} · schema: repo_graph.v{} / history_brief.v{}_\n\n",
        inventory.repo_graph.schema_version, inventory.history_brief.schema_version
    ));
    out.push_str("## Use This For\n\n");
    out.push_str("- Paste into a compatible graph viewer or agent session as local context.\n");
    out.push_str("- Treat graph edges as navigation leads, not proof by themselves.\n");
    out.push_str("- Prefer cited files and decision markers when resolving conflicts.\n\n");

    out.push_str("## Repo\n\n");
    out.push_str(&format!("- path: `{}`\n", inventory.repo_path));
    if let Some(branch) = &inventory.branch {
        out.push_str(&format!("- branch: `{branch}`\n"));
    }
    if let Some(sha) = &inventory.commit_sha {
        out.push_str(&format!("- commit: `{}`\n", sha));
    }
    if !inventory.stack_tags.is_empty() {
        out.push_str(&format!("- stack: {}\n", inventory.stack_tags.join(", ")));
    }
    out.push('\n');

    if !inventory.history_brief.summary.is_empty() {
        out.push_str("## History Brief\n\n");
        out.push_str(&format!("{}\n\n", inventory.history_brief.summary));
        for decision in inventory.history_brief.decisions.iter().take(12) {
            out.push_str(&format!(
                "- **{}** `{}` — {}\n",
                decision.marker, decision.source, decision.text
            ));
        }
        for hint in inventory.history_brief.test_hints.iter().take(12) {
            out.push_str(&format!("- `{}` — {}\n", hint.path, hint.reason));
        }
        for coupling in inventory.history_brief.temporal_couplings.iter().take(8) {
            out.push_str(&format!(
                "- co-change `{}` — {} commit{}{}; {}\n",
                coupling.files.join("` + `"),
                coupling.commit_count,
                if coupling.commit_count == 1 { "" } else { "s" },
                coupling
                    .last_commit
                    .as_deref()
                    .map(|commit| format!("; latest `{commit}`"))
                    .unwrap_or_default(),
                coupling.reason
            ));
        }
        if !inventory.history_brief.recent_commits.is_empty() {
            out.push_str("\nRecent commits:\n");
            for commit in inventory.history_brief.recent_commits.iter().take(8) {
                out.push_str(&format!(
                    "- `{}`{} — {}\n",
                    commit.sha,
                    commit
                        .date
                        .as_deref()
                        .map(|date| format!(" {date}"))
                        .unwrap_or_default(),
                    commit.subject
                ));
            }
        }
        out.push('\n');
    }

    if inventory.repo_health.files_analyzed > 0 {
        out.push_str("## Deterministic Repo Health\n\n");
        out.push_str(&format!(
            "{}\n\nAverage score: {:.1}/10; hotspots: {}; files analyzed: {}{}.\n\n",
            inventory.repo_health.summary,
            inventory.repo_health.average_score,
            inventory.repo_health.hotspot_count,
            inventory.repo_health.files_analyzed,
            if inventory.repo_health.truncated {
                "; truncated"
            } else {
                ""
            }
        ));
        for file in inventory.repo_health.top_files.iter().take(12) {
            out.push_str(&format!(
                "- `{}` — {:.1}/10 `{}`; {} lines; churn {}; test signal: {}\n",
                file.path, file.score, file.bucket, file.lines, file.churn, file.has_test_signal
            ));
            for finding in file.findings.iter().take(4) {
                out.push_str(&format!(
                    "  - {} [{}:{}] {}\n",
                    finding.label, finding.dimension, finding.severity, finding.detail
                ));
            }
            for target in file.refactoring_targets.iter().take(2) {
                out.push_str(&format!("  - refactor lead: {target}\n"));
            }
        }
        out.push('\n');
    }

    if !inventory.repo_graph.nodes.is_empty() {
        out.push_str("## Repo Graph Nodes\n\n");
        for node in inventory.repo_graph.nodes.iter().take(80) {
            out.push_str(&format!("- **{}** `{}`", node.kind, node.label));
            if let Some(path) = &node.path {
                out.push_str(&format!(" — `{path}`"));
            }
            if let Some(detail) = &node.detail {
                out.push_str(&format!(" — {detail}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if !inventory.repo_graph.edges.is_empty() {
        out.push_str("## Repo Graph Edges\n\n");
        for edge in inventory.repo_graph.edges.iter().take(120) {
            out.push_str(&format!(
                "- `{}` -> `{}` ({}) — {}",
                edge.from, edge.to, edge.kind, edge.evidence
            ));
            if !edge.sources.is_empty() {
                let sources: Vec<String> = edge.sources.iter().map(|s| format!("`{s}`")).collect();
                out.push_str(&format!("; sources: {}", sources.join(", ")));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if inventory.repo_graph.truncated || inventory.history_brief.truncated {
        out.push_str(
            "> This sidecar was truncated by CodeVetter's bounded local inventory scan.\n",
        );
    }

    out
}

pub(crate) fn render_repo_memory_markdown(
    repo_name: &str,
    created_at: &str,
    inventory: &RepoInventory,
    report: Option<&UnpackReport>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Repo Memory — {repo_name}\n\n"));
    out.push_str(&format!(
        "_Generated: {created_at} · deterministic local inventory"
    ));
    if report.and_then(|r| r.overview.as_ref()).is_some() {
        out.push_str(" · AI analysis attached");
    }
    out.push_str("_\n\n");

    out.push_str("## Start Here\n\n");
    out.push_str(&format!("- repo path: `{}`\n", inventory.repo_path));
    if let Some(branch) = &inventory.branch {
        out.push_str(&format!("- branch: `{branch}`\n"));
    }
    if let Some(sha) = &inventory.commit_sha {
        out.push_str(&format!("- commit: `{sha}`\n"));
    }
    out.push_str(&format!(
        "- scan shape: {} files scanned, {} skipped, {} bytes{}\n",
        inventory.files_scanned,
        inventory.files_skipped,
        inventory.bytes_scanned,
        if inventory.max_files_hit {
            "; safety cap hit"
        } else {
            ""
        }
    ));
    if !inventory.stack_tags.is_empty() {
        out.push_str(&format!(
            "- stack tags: {}\n",
            inventory.stack_tags.join(", ")
        ));
    }
    if let Some(overview) = report.and_then(|r| r.overview.as_ref()) {
        out.push_str(&format!("\n> {overview}\n"));
    }
    out.push('\n');

    out.push_str("## Source Map\n\n");
    if !inventory.docs.is_empty() {
        out.push_str("### Docs\n\n");
        for doc in inventory.docs.iter().take(10) {
            out.push_str(&format!("- `{}` ({} bytes)\n", doc.path, doc.bytes));
        }
        out.push('\n');
    }
    if !inventory.manifests.is_empty() {
        out.push_str("### Manifests\n\n");
        for manifest in inventory.manifests.iter().take(12) {
            out.push_str(&format!(
                "- `{}` — {}{}{}\n",
                manifest.path,
                manifest.kind,
                manifest
                    .name
                    .as_deref()
                    .map(|name| format!(" · {name}"))
                    .unwrap_or_default(),
                if manifest.scripts.is_empty() {
                    String::new()
                } else {
                    format!(
                        " · scripts: {}",
                        manifest
                            .scripts
                            .iter()
                            .take(6)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ));
        }
        out.push('\n');
    }
    if !inventory.entrypoints.is_empty() {
        out.push_str("### Entrypoints\n\n");
        for entrypoint in inventory.entrypoints.iter().take(12) {
            out.push_str(&format!(
                "- `{}` — {} ({})\n",
                entrypoint.path, entrypoint.kind, entrypoint.reason
            ));
        }
        out.push('\n');
    }
    if !inventory.config_files.is_empty() {
        out.push_str("### Config\n\n");
        for file in inventory.config_files.iter().take(12) {
            out.push_str(&format!("- `{file}`\n"));
        }
        out.push('\n');
    }

    out.push_str("## Architecture Leads\n\n");
    if !inventory.workspace_units.is_empty() {
        out.push_str("### Workspace Units\n\n");
        for unit in inventory.workspace_units.iter().take(12) {
            out.push_str(&format!(
                "- **{}** `{}` — {}; {} files",
                unit.name, unit.path, unit.kind, unit.file_count
            ));
            if let Some(build_system) = &unit.build_system {
                out.push_str(&format!("; build: {build_system}"));
            }
            if let Some(manifest) = &unit.manifest_path {
                out.push_str(&format!("; manifest: `{manifest}`"));
            }
            out.push('\n');
            if !unit.entrypoints.is_empty() {
                out.push_str(&format!(
                    "  - entrypoints: {}\n",
                    unit.entrypoints
                        .iter()
                        .take(6)
                        .map(|path| format!("`{path}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !unit.test_files.is_empty() {
                out.push_str(&format!(
                    "  - tests: {}\n",
                    unit.test_files
                        .iter()
                        .take(6)
                        .map(|path| format!("`{path}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        out.push('\n');
    }
    if !inventory.repo_graph.nodes.is_empty() {
        out.push_str("### Graph Nodes\n\n");
        for node in inventory.repo_graph.nodes.iter().take(20) {
            out.push_str(&format!("- **{}** `{}`", node.kind, node.label));
            if let Some(path) = &node.path {
                out.push_str(&format!(" — `{path}`"));
            }
            if let Some(detail) = &node.detail {
                out.push_str(&format!(" — {detail}"));
            }
            out.push('\n');
        }
        out.push('\n');
    }
    if !inventory.repo_graph.edges.is_empty() {
        out.push_str("### Graph Edges\n\n");
        for edge in inventory.repo_graph.edges.iter().take(24) {
            out.push_str(&format!(
                "- `{}` -> `{}` ({}) — {}\n",
                edge.from, edge.to, edge.kind, edge.evidence
            ));
        }
        out.push('\n');
    }

    out.push_str("## Verification\n\n");
    out.push_str(&format!(
        "- QA posture: {}/100 ({}) — {}\n",
        inventory.qa_readiness.score, inventory.qa_readiness.status, inventory.qa_readiness.summary
    ));
    for signal in inventory.qa_readiness.signals.iter().take(10) {
        out.push_str(&format!(
            "- {}: {} — {}\n",
            signal.label, signal.status, signal.detail
        ));
        if !signal.sources.is_empty() {
            out.push_str(&format!(
                "  - sources: {}\n",
                signal
                    .sources
                    .iter()
                    .take(6)
                    .map(|source| format!("`{source}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if !inventory.qa_readiness.suggested_flows.is_empty() {
        out.push_str("\nSuggested flows:\n");
        for flow in inventory.qa_readiness.suggested_flows.iter().take(10) {
            out.push_str(&format!("- `{}` — {}\n", flow.route, flow.goal));
        }
    }
    if inventory.repo_health.files_analyzed > 0 {
        out.push_str(&format!(
            "\nRepo health: {:.1}/10 average; {} hotspots across {} analyzed files{}.\n",
            inventory.repo_health.average_score,
            inventory.repo_health.hotspot_count,
            inventory.repo_health.files_analyzed,
            if inventory.repo_health.truncated {
                "; truncated"
            } else {
                ""
            }
        ));
        for file in inventory.repo_health.top_files.iter().take(8) {
            out.push_str(&format!(
                "- `{}` — {:.1}/10 {}; {} lines; churn {}\n",
                file.path, file.score, file.bucket, file.lines, file.churn
            ));
            for finding in file.findings.iter().take(3) {
                out.push_str(&format!(
                    "  - {} [{}:{}] {}\n",
                    finding.label, finding.dimension, finding.severity, finding.detail
                ));
            }
        }
    }
    out.push('\n');

    out.push_str("## Change Memory\n\n");
    if !inventory.history_brief.summary.is_empty() {
        out.push_str(&format!("{}\n\n", inventory.history_brief.summary));
    }
    if !inventory.history_brief.decisions.is_empty() {
        out.push_str("### Decision Markers\n\n");
        for decision in inventory.history_brief.decisions.iter().take(12) {
            out.push_str(&format!(
                "- **{}** `{}` — {}\n",
                decision.marker, decision.source, decision.text
            ));
        }
        out.push('\n');
    }
    if !inventory.history_brief.recent_commits.is_empty() {
        out.push_str("### Recent Commits\n\n");
        for commit in inventory.history_brief.recent_commits.iter().take(10) {
            out.push_str(&format!(
                "- `{}`{} — {}\n",
                commit.sha,
                commit
                    .date
                    .as_deref()
                    .map(|date| format!(" {date}"))
                    .unwrap_or_default(),
                commit.subject
            ));
        }
        out.push('\n');
    }
    if !inventory.history_brief.temporal_couplings.is_empty() {
        out.push_str("### Co-change Clusters\n\n");
        for coupling in inventory.history_brief.temporal_couplings.iter().take(8) {
            out.push_str(&format!(
                "- `{}` — {} commit{}{}; {}\n",
                coupling.files.join("` + `"),
                coupling.commit_count,
                if coupling.commit_count == 1 { "" } else { "s" },
                coupling
                    .last_commit
                    .as_deref()
                    .map(|commit| format!("; latest `{commit}`"))
                    .unwrap_or_default(),
                coupling.reason
            ));
        }
        out.push('\n');
    }

    out.push_str("## Operating Notes\n\n");
    out.push_str("- Graph edges are navigation leads, not proof by themselves.\n");
    out.push_str("- Prefer cited source files and decision markers when resolving conflicts.\n");
    out.push_str("- Rerun Unpack after branch changes, dependency changes, or large refactors.\n");
    out.push_str("- This repo memory is generated without AI. Any attached AI analysis is explicitly marked above.\n");
    out.push_str("- Do not read or expose secrets while expanding this memory into docs.\n");

    if inventory.repo_graph.truncated
        || inventory.history_brief.truncated
        || inventory.repo_health.truncated
    {
        out.push_str(
            "\n> Some sections were truncated by CodeVetter's bounded local inventory scan.\n",
        );
    }

    out
}

pub(crate) fn render_html(repo_name: &str, markdown_body: &str) -> String {
    // Minimal static HTML — no external assets so the export is self-contained.
    let escaped = markdown_body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Repo Unpacked — {repo_name}</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 920px; margin: 2.5rem auto; padding: 0 1.5rem; color: #1f2128; background: #fafafa; }}
  pre {{ background: #f1f3f5; padding: 1rem; overflow-x: auto; font-size: 0.85rem; border-radius: 4px; white-space: pre-wrap; }}
  code {{ background: #eef0f3; padding: 0.05rem 0.35rem; border-radius: 3px; font-size: 0.85rem; }}
  h1, h2, h3 {{ font-weight: 600; }}
</style>
</head>
<body>
<pre>{escaped}</pre>
</body>
</html>"#
    )
}
