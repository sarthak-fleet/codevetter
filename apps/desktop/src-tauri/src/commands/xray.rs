//! Agent PR X-Ray: deterministic, offline, public-safe review evidence exports.

use crate::commands::audience_validation::{self, VerificationStage};
use crate::commands::deterministic_review;
use crate::db::queries;
use crate::DbState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use tauri::State;

const XRAY_SCHEMA_VERSION: u32 = 1;
const MAX_PUBLIC_TEXT_BYTES: usize = 8 * 1024;
const MAX_EXPORT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct XrayRequest {
    pub review_id: String,
    pub public_source_confirmed: bool,
    pub public_source: Option<String>,
    #[serde(default)]
    pub approved_excerpt_finding_ids: Vec<String>,
    pub corpus_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum XrayOutcome {
    Verified,
    NeedsReview,
    Blocked,
    Incomplete,
}

impl XrayOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::NeedsReview => "needs_review",
            Self::Blocked => "blocked",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XrayLocator {
    pub file_path: String,
    pub line: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct XrayFinding {
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub confidence: Option<f64>,
    pub disposition: String,
    pub review_source: String,
    pub locator: XrayLocator,
    pub excerpt_approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_suggestion_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XrayStage {
    pub id: String,
    pub label: String,
    pub status: String,
    pub provenance: String,
    pub recorded_at: Option<String>,
    pub evidence: Vec<String>,
    pub caveats: Vec<String>,
    pub omission_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct XrayCoverage {
    pub kind: String,
    pub complete: bool,
    pub reviewed: usize,
    pub reused: usize,
    pub skipped: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub rejected_candidates: usize,
    pub unresolved_candidates: usize,
    pub stale_candidates: usize,
    pub limitation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPrXray {
    pub schema_version: u32,
    pub xray_id: String,
    pub source: String,
    pub generated_at: String,
    pub corpus_state: String,
    pub outcome: XrayOutcome,
    pub confidence: String,
    pub score: Option<f64>,
    pub review_status: String,
    pub findings: Vec<XrayFinding>,
    pub stages: Vec<XrayStage>,
    pub coverage: XrayCoverage,
    pub changed_behavior: Vec<String>,
    pub trusted_impact_paths: Vec<String>,
    pub checks_run: Vec<String>,
    pub verified_claims: Vec<String>,
    pub missing_proof: Vec<String>,
    pub unresolved_risks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrayBuildResult {
    pub eligible: bool,
    pub missing_requirements: Vec<String>,
    pub sanitizer_issues: Vec<String>,
    pub payload: AgentPrXray,
    pub json: String,
    pub markdown: String,
    pub html: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XrayFormat {
    Json,
    Markdown,
    Html,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveXrayRequest {
    pub xray: XrayRequest,
    pub format: XrayFormat,
    pub path: String,
}

fn clean_public_source(value: Option<String>) -> Result<String, String> {
    let value = value.unwrap_or_default().trim().to_string();
    if value.is_empty() {
        return Ok(value);
    }
    if value.len() > 300
        || value.contains('\0')
        || value.starts_with('/')
        || value.contains("../")
        || value.contains("\\")
        || value.contains("file://")
    {
        return Err("Public source must be a bounded repository or pull-request reference".into());
    }
    Ok(value)
}

fn stage(
    id: &str,
    value: &VerificationStage,
    provenance: &str,
    recorded_at: Option<String>,
) -> XrayStage {
    XrayStage {
        id: id.to_string(),
        label: value.label.clone(),
        status: value.status.clone(),
        provenance: provenance.to_string(),
        recorded_at,
        evidence: value.evidence.clone(),
        caveats: value.caveats.clone(),
        omission_reason: if value.status == "waived" {
            Some("Stage was explicitly waived; no verification occurred.".to_string())
        } else if value.evidence.is_empty() {
            Some(
                match value.status.as_str() {
                    "not_run" | "not_verified" => "No qualifying public evidence was recorded.",
                    "failed" => "The stage failed; passing evidence is unavailable.",
                    _ => "No public evidence was recorded for this stage.",
                }
                .to_string(),
            )
        } else {
            None
        },
    }
}

fn coverage(conn: &rusqlite::Connection, review_id: &str) -> Result<XrayCoverage, String> {
    let Some(manifest) = deterministic_review::load_manifest_for_review(conn, review_id)? else {
        return Ok(XrayCoverage {
            kind: "legacy_aggregate".into(),
            complete: false,
            reviewed: 0,
            reused: 0,
            skipped: 0,
            failed: 0,
            cancelled: 0,
            rejected_candidates: 0,
            unresolved_candidates: 0,
            stale_candidates: 0,
            limitation: Some("Per-file coverage is unknown for this legacy review.".into()),
        });
    };
    let count = |name| {
        manifest
            .units
            .iter()
            .filter(|unit| format!("{:?}", unit.coverage_state).eq_ignore_ascii_case(name))
            .count()
    };
    Ok(XrayCoverage {
        kind: "deterministic_units".into(),
        complete: manifest.complete_coverage && !manifest.stale,
        reviewed: count("reviewed"),
        reused: count("reused"),
        skipped: count("skipped"),
        failed: count("failed"),
        cancelled: count("cancelled"),
        rejected_candidates: manifest.qualification_counts.rejected,
        unresolved_candidates: manifest.qualification_counts.unresolved,
        stale_candidates: manifest.qualification_counts.stale,
        limitation: if manifest.stale {
            Some("The repository target changed during review.".into())
        } else if !manifest.complete_coverage {
            Some("One or more changed files were not reviewed successfully.".into())
        } else {
            None
        },
    })
}

fn public_text(value: Option<String>, fallback: &str) -> String {
    value
        .unwrap_or_else(|| fallback.to_string())
        .chars()
        .take(MAX_PUBLIC_TEXT_BYTES)
        .collect()
}

fn stable_id(source: &str, review_id: &str) -> String {
    let digest = format!(
        "{:x}",
        Sha256::digest(format!("{source}\0{review_id}").as_bytes())
    );
    format!("xray-{}", &digest[..20])
}

fn build(conn: &rusqlite::Connection, request: XrayRequest) -> Result<XrayBuildResult, String> {
    let review_id = request.review_id.trim();
    if review_id.is_empty() {
        return Err("review_id is required".into());
    }
    let source = clean_public_source(request.public_source)?;
    let (review, rows) = queries::get_local_review_with_findings(conn, review_id)
        .map_err(|_| "Review not found".to_string())?;
    let bundle = audience_validation::load_bundle(conn, review_id)?;
    let verification = &bundle.verification;
    let coverage = coverage(conn, review_id)?;
    let approved = request
        .approved_excerpt_finding_ids
        .into_iter()
        .collect::<HashSet<_>>();
    let mut findings = rows
        .into_iter()
        .map(|finding| {
            let excerpt_approved = approved.contains(&finding.id) && finding.suggestion.is_some();
            XrayFinding {
                severity: finding.severity.unwrap_or_else(|| "unknown".into()),
                title: public_text(finding.title, "Untitled finding"),
                summary: public_text(finding.summary, "No public summary recorded."),
                confidence: finding.confidence,
                disposition: finding.disposition.unwrap_or_else(|| "unreviewed".into()),
                review_source: source.clone(),
                locator: XrayLocator {
                    file_path: finding.file_path.unwrap_or_default(),
                    line: finding.line,
                },
                excerpt_approved,
                approved_suggestion_excerpt: if excerpt_approved {
                    finding
                        .suggestion
                        .map(|value| value.chars().take(2_000).collect())
                } else {
                    None
                },
            }
        })
        .collect::<Vec<_>>();
    findings.sort_by(|left, right| {
        left.locator
            .file_path
            .cmp(&right.locator.file_path)
            .then(left.locator.line.cmp(&right.locator.line))
            .then(left.title.cmp(&right.title))
    });
    let outcome = match verification.aggregate_status.as_str() {
        "verified" => XrayOutcome::Verified,
        "needs_review" => XrayOutcome::NeedsReview,
        "blocked" => XrayOutcome::Blocked,
        _ => XrayOutcome::Incomplete,
    };
    let corpus_state = request.corpus_state.unwrap_or_else(|| "dogfood".into());
    if !matches!(
        corpus_state.as_str(),
        "dogfood" | "reviewed_public" | "benchmark_ground_truth"
    ) {
        return Err(
            "corpus_state must be dogfood, reviewed_public, or benchmark_ground_truth".into(),
        );
    }
    let review_recorded_at = review
        .completed_at
        .clone()
        .or(Some(review.created_at.clone()));
    let audience_recorded_at = bundle.run.as_ref().map(|run| run.updated_at.clone());
    let stages = vec![
        stage(
            "review",
            &verification.review,
            "persisted_local_review",
            review_recorded_at,
        ),
        stage(
            "executable_test",
            &verification.executable_test,
            "qualified_local_verification",
            None,
        ),
        stage(
            "audience",
            &verification.audience,
            "persisted_audience_validation",
            audience_recorded_at,
        ),
    ];
    // Finding summaries describe risks, not product behavior. Until Review
    // persists an independently qualified change summary, the public export
    // must leave changed behavior empty rather than rebrand a model claim.
    let changed_behavior = Vec::new();
    let checks_run = stages
        .iter()
        .flat_map(|stage| {
            stage
                .evidence
                .iter()
                .map(move |evidence| format!("{}: {evidence}", stage.id))
        })
        .collect::<Vec<_>>();
    let missing_proof = stages
        .iter()
        .filter_map(|stage| {
            stage
                .omission_reason
                .clone()
                .map(|reason| format!("{}: {reason}", stage.id))
        })
        .chain(stages.iter().flat_map(|stage| {
            stage
                .caveats
                .iter()
                .map(move |caveat| format!("{}: {caveat}", stage.id))
        }))
        .collect::<Vec<_>>();
    let unresolved_risks = findings
        .iter()
        .filter(|finding| finding.disposition != "dismissed")
        .map(|finding| format!("{}: {}", finding.severity, finding.title))
        .collect::<Vec<_>>();
    let verified_claims = Vec::new();
    let payload = AgentPrXray {
        schema_version: XRAY_SCHEMA_VERSION,
        xray_id: stable_id(&source, review_id),
        source,
        generated_at: review
            .completed_at
            .clone()
            .unwrap_or_else(|| review.created_at.clone()),
        corpus_state,
        outcome,
        confidence: verification.confidence.clone(),
        score: review.score_composite,
        review_status: review.status.clone(),
        findings,
        stages,
        coverage,
        changed_behavior,
        trusted_impact_paths: Vec::new(),
        checks_run,
        verified_claims,
        missing_proof,
        unresolved_risks,
    };
    let mut missing_requirements = Vec::new();
    if review.status != "completed" {
        missing_requirements.push("The review has not completed.".into());
    }
    if !request.public_source_confirmed {
        missing_requirements
            .push("Confirm that the source repository and change are public.".into());
    }
    if payload.source.is_empty() {
        missing_requirements.push("Add a public repository or pull-request reference.".into());
    }
    let sanitizer_issues = scan_payload(&payload);
    let json = serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?;
    let markdown = render_markdown(&payload);
    let html = render_html(&payload);
    if json.len() > MAX_EXPORT_BYTES
        || markdown.len() > MAX_EXPORT_BYTES
        || html.len() > MAX_EXPORT_BYTES
    {
        return Err("X-Ray exceeds the bounded export size".into());
    }
    Ok(XrayBuildResult {
        eligible: missing_requirements.is_empty() && sanitizer_issues.is_empty(),
        missing_requirements,
        sanitizer_issues,
        payload,
        json,
        markdown,
        html,
    })
}

fn scan_payload(payload: &AgentPrXray) -> Vec<String> {
    let serialized = serde_json::to_string(payload).unwrap_or_default();
    let lower = serialized.to_ascii_lowercase();
    let patterns = [
        ("/users/", "Local macOS path detected."),
        ("/home/", "Local home path detected."),
        ("file://", "Local file URL detected."),
        ("-----begin ", "Private key material detected."),
        ("sk-ant-", "Anthropic credential detected."),
        ("sk-proj-", "OpenAI credential detected."),
        ("ghp_", "GitHub credential detected."),
        ("<script", "Executable HTML detected."),
        ("javascript:", "Executable URL detected."),
        ("input_prompt", "Private prompt field detected."),
        ("output_raw", "Raw provider output field detected."),
    ];
    let mut issues = patterns
        .into_iter()
        .filter(|(needle, _)| lower.contains(needle))
        .map(|(_, issue)| issue.to_string())
        .collect::<Vec<_>>();
    for finding in &payload.findings {
        let path = Path::new(&finding.locator.file_path);
        if finding.locator.file_path.is_empty()
            || finding.locator.file_path.contains('\\')
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            issues.push(format!(
                "Unsafe or missing evidence locator for '{}'.",
                finding.title
            ));
        }
    }
    issues.sort();
    issues.dedup();
    issues
}

fn md(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render_markdown(payload: &AgentPrXray) -> String {
    let mut output = format!(
        "# Agent PR X-Ray\n\n**Schema:** v{}  \n**X-Ray:** {}  \n**Source:** {}  \n**Outcome:** {}  \n**Confidence:** {}  \n**Corpus:** {}  \n**Generated:** {}\n\n## Verification\n\n",
        payload.schema_version,
        md(&payload.xray_id),
        md(&payload.source),
        payload.outcome.as_str(),
        md(&payload.confidence),
        md(&payload.corpus_state),
        md(&payload.generated_at),
    );
    for stage in &payload.stages {
        output.push_str(&format!(
            "- **{}:** {} · provenance: {}{}",
            md(&stage.label),
            md(&stage.status),
            md(&stage.provenance),
            stage
                .recorded_at
                .as_ref()
                .map(|value| format!(" · recorded: {}", md(value)))
                .unwrap_or_default(),
        ));
        if let Some(reason) = &stage.omission_reason {
            output.push_str(&format!(" — {}", md(reason)));
        }
        for evidence in &stage.evidence {
            output.push_str(&format!(" · evidence: {}", md(evidence)));
        }
        for caveat in &stage.caveats {
            output.push_str(&format!(" · caveat: {}", md(caveat)));
        }
        output.push('\n');
    }
    output.push_str(&format!(
        "\n## Coverage\n\n{}; complete: {}; reviewed: {}; reused: {}; skipped: {}; failed: {}; cancelled: {}; rejected: {}; unresolved: {}; stale: {}.{}\n\n## Findings\n\n",
        md(&payload.coverage.kind), payload.coverage.complete, payload.coverage.reviewed,
        payload.coverage.reused, payload.coverage.skipped, payload.coverage.failed,
        payload.coverage.cancelled, payload.coverage.rejected_candidates,
        payload.coverage.unresolved_candidates, payload.coverage.stale_candidates,
        payload.coverage.limitation.as_ref().map(|value| format!(" Limitation: {}", md(value))).unwrap_or_default(),
    ));
    if payload.findings.is_empty() {
        output.push_str("No qualified findings were recorded.\n");
    }
    for finding in &payload.findings {
        output.push_str(&format!(
            "### {} · {}\n\n{}\n\nEvidence: `{}`{} · disposition: {}\n\n",
            md(&finding.severity),
            md(&finding.title),
            md(&finding.summary),
            md(&finding.locator.file_path),
            finding
                .locator
                .line
                .map(|line| format!(":{line}"))
                .unwrap_or_default(),
            md(&finding.disposition)
        ));
        if let Some(excerpt) = &finding.approved_suggestion_excerpt {
            output.push_str(&format!(
                "Approved suggestion excerpt:\n\n> {}\n\n",
                md(excerpt)
            ));
        }
    }
    push_markdown_list(
        &mut output,
        "Changed behavior",
        &payload.changed_behavior,
        "No changed behavior was exportable.",
    );
    push_markdown_list(
        &mut output,
        "Trusted impact paths",
        &payload.trusted_impact_paths,
        "No trusted impact path was recorded.",
    );
    push_markdown_list(
        &mut output,
        "Checks run",
        &payload.checks_run,
        "No qualifying check evidence was recorded.",
    );
    push_markdown_list(
        &mut output,
        "Verified claims",
        &payload.verified_claims,
        "No claim is verified by every required stage.",
    );
    push_markdown_list(
        &mut output,
        "Missing proof",
        &payload.missing_proof,
        "No missing proof was recorded.",
    );
    push_markdown_list(
        &mut output,
        "Unresolved risks",
        &payload.unresolved_risks,
        "No unresolved risk was recorded.",
    );
    output
}

fn push_markdown_list(output: &mut String, title: &str, values: &[String], empty: &str) {
    output.push_str(&format!("\n## {}\n\n", md(title)));
    if values.is_empty() {
        output.push_str(empty);
        output.push('\n');
    } else {
        for value in values {
            output.push_str(&format!("- {}\n", md(value)));
        }
    }
}

fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_html(payload: &AgentPrXray) -> String {
    let stages = payload
        .stages
        .iter()
        .map(|stage| {
            format!(
                "<li><strong>{}</strong><span>{}</span><p>{}</p><small>{}{}</small></li>",
                html(&stage.label),
                html(&stage.status),
                html(stage.omission_reason.as_deref().unwrap_or_else(|| {
                    stage
                        .evidence
                        .first()
                        .map(String::as_str)
                        .unwrap_or("Evidence recorded")
                })),
                html(&stage.provenance),
                stage
                    .recorded_at
                    .as_ref()
                    .map(|value| format!(" · {}", html(value)))
                    .unwrap_or_default(),
            )
        })
        .collect::<String>();
    let findings = if payload.findings.is_empty() {
        "<p class=empty>No qualified findings were recorded.</p>".into()
    } else {
        payload.findings.iter().map(|finding| format!(
            "<article><div><b>{}</b><em>{}</em></div><h3>{}</h3><p>{}</p><code>{}{}</code><small>{}</small></article>",
            html(&finding.severity), html(&finding.disposition), html(&finding.title), html(&finding.summary),
            html(&finding.locator.file_path), finding.locator.line.map(|line| format!(":{line}")).unwrap_or_default(),
            finding.approved_suggestion_excerpt.as_deref().map(|value| format!("Approved suggestion excerpt: {}", html(value))).unwrap_or_else(|| "No suggestion excerpt approved.".into())
        )).collect::<String>()
    };
    let evidence_sections = [
        (
            "Changed behavior",
            &payload.changed_behavior,
            "No changed behavior was exportable.",
        ),
        (
            "Trusted impact paths",
            &payload.trusted_impact_paths,
            "No trusted impact path was recorded.",
        ),
        (
            "Checks run",
            &payload.checks_run,
            "No qualifying check evidence was recorded.",
        ),
        (
            "Verified claims",
            &payload.verified_claims,
            "No claim is verified by every required stage.",
        ),
        (
            "Missing proof",
            &payload.missing_proof,
            "No missing proof was recorded.",
        ),
        (
            "Unresolved risks",
            &payload.unresolved_risks,
            "No unresolved risk was recorded.",
        ),
    ]
    .into_iter()
    .map(|(title, values, empty)| {
        let rows = if values.is_empty() {
            format!("<p class=empty>{}</p>", html(empty))
        } else {
            format!(
                "<ul class=evidence>{}</ul>",
                values
                    .iter()
                    .map(|value| format!("<li>{}</li>", html(value)))
                    .collect::<String>()
            )
        };
        format!("<section><h2>{}</h2>{}</section>", html(title), rows)
    })
    .collect::<String>();
    format!(
        r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Agent PR X-Ray</title><style>:root{{color-scheme:dark}}*{{box-sizing:border-box}}body{{margin:0;background:#08090b;color:#e7e8eb;font:15px/1.55 Inter,ui-sans-serif,system-ui,sans-serif}}main{{width:min(920px,calc(100% - 40px));margin:64px auto}}header{{padding-bottom:28px;border-bottom:1px solid #24262b}}h1{{font-size:34px;letter-spacing:-.04em;margin:8px 0}}.eyebrow,small,code,.empty{{color:#9298a3}}.outcome{{color:#efc56e}}section{{margin-top:36px}}ul{{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;padding:0}}li,article{{list-style:none;border:1px solid #25282e;border-radius:14px;background:#101216;padding:18px}}li strong,li span{{display:block}}li span{{color:#efc56e;margin-top:8px}}li p{{color:#9298a3}}article{{margin:12px 0}}article div{{display:flex;justify-content:space-between;color:#efc56e}}article h3{{margin:12px 0 6px}}article p{{color:#b6bac2}}article code,article small{{display:block;margin-top:12px}}ul.evidence{{display:block}}ul.evidence li{{margin:8px 0;padding:12px}}@media(max-width:700px){{ul{{grid-template-columns:1fr}}main{{margin-top:32px}}}}</style><main><header><div class="eyebrow">CodeVetter · local evidence export · schema v{}</div><h1>Agent PR X-Ray</h1><div>{}</div><div class="outcome">{} · {} confidence</div><small>{} · {} · {}</small></header><section><h2>Verification</h2><ul>{}</ul></section><section><h2>Coverage</h2><p>{}; complete: {}; {} reviewed, {} reused, {} skipped, {} failed, {} cancelled, {} rejected, {} unresolved, {} stale. {}</p></section><section><h2>Findings</h2>{}</section>{}</main></html>"#,
        payload.schema_version,
        html(&payload.source),
        payload.outcome.as_str(),
        html(&payload.confidence),
        html(&payload.xray_id),
        html(&payload.corpus_state),
        html(&payload.generated_at),
        stages,
        html(&payload.coverage.kind),
        payload.coverage.complete,
        payload.coverage.reviewed,
        payload.coverage.reused,
        payload.coverage.skipped,
        payload.coverage.failed,
        payload.coverage.cancelled,
        payload.coverage.rejected_candidates,
        payload.coverage.unresolved_candidates,
        payload.coverage.stale_candidates,
        html(payload.coverage.limitation.as_deref().unwrap_or("")),
        findings,
        evidence_sections,
    )
}

#[tauri::command]
pub async fn build_agent_pr_xray(
    db: State<'_, DbState>,
    request: XrayRequest,
) -> Result<XrayBuildResult, String> {
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    build(&conn, request)
}

#[tauri::command]
pub async fn save_agent_pr_xray(
    db: State<'_, DbState>,
    request: SaveXrayRequest,
) -> Result<String, String> {
    let path = Path::new(request.path.trim());
    let expected = match request.format {
        XrayFormat::Json => "json",
        XrayFormat::Markdown => "md",
        XrayFormat::Html => "html",
    };
    if path.extension().and_then(|value| value.to_str()) != Some(expected) {
        return Err(format!("X-Ray path must end in .{expected}"));
    }
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err("X-Ray destination cannot be a symlink".into());
    }
    let parent = path
        .parent()
        .ok_or("X-Ray destination needs a parent directory")?;
    fs::canonicalize(parent).map_err(|_| "X-Ray destination directory is unavailable")?;
    let result = {
        let conn = db.0.lock().map_err(|error| error.to_string())?;
        build(&conn, request.xray)?
    };
    if !result.eligible {
        return Err(format!(
            "X-Ray export is blocked: {}",
            result
                .missing_requirements
                .iter()
                .chain(result.sanitizer_issues.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    let content = match request.format {
        XrayFormat::Json => result.json,
        XrayFormat::Markdown => result.markdown,
        XrayFormat::Html => result.html,
    };
    let temporary = path.with_extension(format!("{expected}.tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, content).map_err(|error| format!("Could not write X-Ray: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Could not finalize X-Ray: {error}")
    })?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use rusqlite::params;

    fn fixture() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("schema");
        conn.execute(
            "INSERT INTO local_reviews (id, repo_full_name, pr_number, agent_used, score_composite, findings_count, status, completed_at, created_at) VALUES ('review-1','owner/repo',7,'claude',88,1,'completed','2026-07-22T00:00:00Z','2026-07-22T00:00:00Z')",
            [],
        ).expect("review");
        conn.execute(
            "INSERT INTO local_review_findings (id, review_id, severity, title, summary, suggestion, file_path, line, confidence, disposition) VALUES ('finding-1','review-1','high','Broken guard','The guard accepts an invalid state.','Return an error.','src/lib.rs',12,.9,'accepted')",
            [],
        ).expect("finding");
        conn
    }

    #[test]
    fn renderers_share_one_truthful_payload() {
        let result = build(
            &fixture(),
            XrayRequest {
                review_id: "review-1".into(),
                public_source_confirmed: true,
                public_source: Some("owner/repo#7".into()),
                approved_excerpt_finding_ids: vec!["finding-1".into()],
                corpus_state: Some("benchmark_ground_truth".into()),
            },
        )
        .expect("xray");
        assert!(result.eligible);
        assert!(result.json.contains("owner/repo#7"));
        assert!(result.markdown.contains("not\\_verified"));
        assert!(result.html.contains("not_verified"));
        assert!(!result.html.contains("<script"));
        for section in [
            "Changed behavior",
            "Trusted impact paths",
            "Checks run",
            "Verified claims",
            "Missing proof",
            "Unresolved risks",
        ] {
            assert!(result.markdown.contains(section));
            assert!(result.html.contains(section));
        }
        assert!(result.markdown.contains("**Schema:** v1"));
        assert!(result.html.contains("schema v1"));
        assert!(result.markdown.contains("persisted\\_local\\_review"));
        assert!(result.html.contains("persisted_local_review"));
        assert_eq!(result.payload.outcome, XrayOutcome::Incomplete);
    }

    #[test]
    fn suggestion_excerpts_require_explicit_per_finding_approval() {
        let conn = fixture();
        conn.execute(
            "UPDATE local_review_findings SET suggestion='PRIVATE_SNIPPET' WHERE id='finding-1'",
            [],
        )
        .expect("suggestion");
        let request = |approved_excerpt_finding_ids| XrayRequest {
            review_id: "review-1".into(),
            public_source_confirmed: true,
            public_source: Some("owner/repo#7".into()),
            approved_excerpt_finding_ids,
            corpus_state: None,
        };
        let blocked = build(&conn, request(vec![])).expect("unapproved");
        assert!(!blocked.json.contains("PRIVATE_SNIPPET"));
        assert!(!blocked.markdown.contains("PRIVATE_SNIPPET"));
        assert!(!blocked.html.contains("PRIVATE_SNIPPET"));
        assert!(!blocked.payload.findings[0].excerpt_approved);

        let approved = build(&conn, request(vec!["finding-1".into()])).expect("approved");
        assert!(approved.json.contains("PRIVATE_SNIPPET"));
        assert!(approved.markdown.contains("PRIVATE\\_SNIPPET"));
        assert!(approved.html.contains("PRIVATE_SNIPPET"));
        assert!(approved.payload.findings[0].excerpt_approved);
    }

    #[test]
    fn verified_and_mixed_payloads_keep_their_exact_outcomes() {
        let mut payload = build(
            &fixture(),
            XrayRequest {
                review_id: "review-1".into(),
                public_source_confirmed: true,
                public_source: Some("owner/repo#7".into()),
                approved_excerpt_finding_ids: Vec::new(),
                corpus_state: Some("reviewed_public".into()),
            },
        )
        .expect("base payload")
        .payload;
        payload.outcome = XrayOutcome::Verified;
        payload.verified_claims = vec!["The public behavior is verified.".into()];
        payload.missing_proof.clear();
        for stage in &mut payload.stages {
            stage.status = if stage.id == "audience" {
                "waived".into()
            } else {
                "passed".into()
            };
            stage.omission_reason = (stage.id == "audience")
                .then(|| "Audience was explicitly waived; no verification occurred.".into());
        }
        assert!(render_markdown(&payload).contains("**Outcome:** verified"));
        assert!(render_html(&payload).contains("verified ·"));

        payload.outcome = XrayOutcome::NeedsReview;
        payload.verified_claims.clear();
        payload.unresolved_risks = vec!["medium: Guard invalid state".into()];
        assert!(render_markdown(&payload).contains("**Outcome:** needs_review"));
        assert!(render_html(&payload).contains("needs_review ·"));
        assert!(render_html(&payload).contains("Guard invalid state"));
    }

    #[test]
    fn sanitizer_blocks_paths_secrets_and_unsafe_html() {
        let conn = fixture();
        conn.execute(
            "UPDATE local_review_findings SET summary=?1 WHERE id='finding-1'",
            params!["See /Users/alice/.ssh/id_ed25519 and <script>alert(1)</script>"],
        )
        .expect("update");
        let result = build(
            &conn,
            XrayRequest {
                review_id: "review-1".into(),
                public_source_confirmed: true,
                public_source: Some("owner/repo#7".into()),
                approved_excerpt_finding_ids: vec![],
                corpus_state: None,
            },
        )
        .expect("xray");
        assert!(!result.eligible);
        assert!(result
            .sanitizer_issues
            .iter()
            .any(|issue| issue.contains("path")));
        assert!(result
            .sanitizer_issues
            .iter()
            .any(|issue| issue.contains("HTML")));
    }

    #[test]
    fn waiver_is_not_upgraded_to_passed() {
        let conn = fixture();
        conn.execute(
            "INSERT INTO audience_validation_runs (id, review_id, audience, task, candidate_a, criteria_json, min_responses, required, waived_reason, status, created_at, updated_at) VALUES ('audience-1','review-1','developers','inspect','current','[\"trust\"]',1,1,'not applicable','waived','2026-07-22','2026-07-22')",
            [],
        ).expect("audience");
        let result = build(
            &conn,
            XrayRequest {
                review_id: "review-1".into(),
                public_source_confirmed: true,
                public_source: Some("owner/repo#7".into()),
                approved_excerpt_finding_ids: vec![],
                corpus_state: None,
            },
        )
        .expect("xray");
        let audience = result
            .payload
            .stages
            .iter()
            .find(|stage| stage.id == "audience")
            .expect("stage");
        assert_eq!(audience.status, "waived");
        assert!(audience
            .omission_reason
            .as_deref()
            .unwrap_or_default()
            .contains("waived"));
    }

    #[test]
    fn missing_and_failed_stages_remain_explicit_omissions() {
        for (status, expected) in [
            ("not_run", "No qualifying public evidence"),
            ("not_verified", "No qualifying public evidence"),
            ("failed", "stage failed"),
        ] {
            let exported = stage(
                "fixture",
                &VerificationStage {
                    status: status.into(),
                    label: "Fixture".into(),
                    evidence: Vec::new(),
                    caveats: Vec::new(),
                },
                "test_fixture",
                None,
            );
            assert_eq!(exported.status, status);
            assert_ne!(exported.status, "passed");
            assert!(exported
                .omission_reason
                .as_deref()
                .unwrap_or_default()
                .contains(expected));
        }
    }
}
