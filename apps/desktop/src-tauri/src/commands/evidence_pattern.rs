use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};

const MAX_CANDIDATES: usize = 6;
const MAX_STRUCTURAL_FILES: usize = 20;
const MAX_STRUCTURAL_MATCHES: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceRef {
    pub kind: String,
    pub label: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceCandidate {
    pub id: String,
    pub kind: String,
    pub severity_hint: String,
    pub confidence: f64,
    pub affected_files: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
    pub scale: String,
    pub why_it_matters: String,
    pub caveats: Vec<String>,
    pub open_questions: Vec<String>,
    pub suggested_checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceProcedureStep {
    pub id: String,
    pub procedure: String,
    pub status: String,
    pub candidate_ids: Vec<String>,
    pub input: String,
    pub action: String,
    pub output: String,
    pub artifact: String,
    pub gate: String,
    pub blocked_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuralEvidenceMatch {
    pub rule_id: String,
    pub label: String,
    pub file: String,
    pub line: Option<usize>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EvidenceCandidateInput<'a> {
    pub changed_files: &'a [String],
    pub changed_lines: usize,
    pub sensitive_paths: &'a [String],
    pub history_section: &'a str,
    pub blast_section: &'a str,
    pub structural_evidence: &'a [StructuralEvidenceMatch],
}

struct AstGrepRule {
    id: &'static str,
    label: &'static str,
    lang: &'static str,
    pattern: &'static str,
    extensions: &'static [&'static str],
}

const AST_GREP_RULES: &[AstGrepRule] = &[
    AstGrepRule {
        id: "ts-tauri-invoke",
        label: "Tauri IPC invoke call",
        lang: "ts",
        pattern: "invoke($CMD, $$$ARGS)",
        extensions: &[".ts", ".tsx", ".js", ".jsx"],
    },
    AstGrepRule {
        id: "rust-process-command",
        label: "Rust process command spawn",
        lang: "rust",
        pattern: "Command::new($CMD)",
        extensions: &[".rs"],
    },
];

pub fn collect_structural_evidence(
    repo_path: &str,
    changed_files: &[String],
) -> Vec<StructuralEvidenceMatch> {
    let Some(sg_path) = resolve_sg_path() else {
        return Vec::new();
    };
    let repo = PathBuf::from(repo_path);
    if !repo.is_dir() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for relative_path in changed_files
        .iter()
        .filter(|path| is_structural_scan_path(path))
        .take(MAX_STRUCTURAL_FILES)
    {
        let Some(rule) = AST_GREP_RULES
            .iter()
            .find(|rule| path_has_extension(relative_path, rule.extensions))
        else {
            continue;
        };
        let file_path = repo.join(relative_path);
        if !file_path.is_file() {
            continue;
        }
        matches.extend(run_ast_grep_rule(
            &sg_path,
            &repo,
            relative_path,
            &file_path,
            rule,
        ));
        if matches.len() >= MAX_STRUCTURAL_MATCHES {
            matches.truncate(MAX_STRUCTURAL_MATCHES);
            break;
        }
    }

    matches
}

fn resolve_sg_path() -> Option<String> {
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("sg");
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    std::env::var("HOME").ok().and_then(|home| {
        [
            format!("{home}/.local/bin/sg"),
            format!("{home}/.cargo/bin/sg"),
            format!("/opt/homebrew/bin/sg"),
            format!("/usr/local/bin/sg"),
        ]
        .into_iter()
        .find(|candidate| Path::new(candidate).is_file())
    })
}

fn run_ast_grep_rule(
    sg_path: &str,
    repo: &Path,
    relative_path: &str,
    file_path: &Path,
    rule: &AstGrepRule,
) -> Vec<StructuralEvidenceMatch> {
    let output = StdCommand::new(sg_path)
        .args([
            "run",
            "--pattern",
            rule.pattern,
            "--lang",
            rule.lang,
            "--json",
        ])
        .arg(file_path)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if output.stdout.is_empty() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ast_grep_json_output(&stdout, rule, relative_path)
}

fn parse_ast_grep_json_output(
    stdout: &str,
    rule: &AstGrepRule,
    fallback_file: &str,
) -> Vec<StructuralEvidenceMatch> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return ast_grep_values(value)
            .into_iter()
            .filter_map(|value| structural_match_from_value(&value, rule, fallback_file))
            .take(MAX_STRUCTURAL_MATCHES)
            .collect();
    }

    trimmed
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| structural_match_from_value(&value, rule, fallback_file))
        .take(MAX_STRUCTURAL_MATCHES)
        .collect()
}

fn ast_grep_values(value: Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values,
        value => vec![value],
    }
}

fn structural_match_from_value(
    value: &Value,
    rule: &AstGrepRule,
    fallback_file: &str,
) -> Option<StructuralEvidenceMatch> {
    let file = value
        .get("file")
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .unwrap_or(fallback_file)
        .to_string();
    let line = value
        .get("range")
        .and_then(|range| range.get("start"))
        .and_then(|start| start.get("line"))
        .and_then(Value::as_u64)
        .or_else(|| value.get("line").and_then(Value::as_u64))
        .map(|line| line as usize);
    let snippet = value
        .get("text")
        .or_else(|| value.get("lines"))
        .and_then(Value::as_str)
        .map(|text| text.trim().chars().take(180).collect::<String>())
        .filter(|text| !text.is_empty());

    Some(StructuralEvidenceMatch {
        rule_id: rule.id.to_string(),
        label: rule.label.to_string(),
        file,
        line,
        snippet,
    })
}

fn is_structural_scan_path(path: &str) -> bool {
    path_has_extension(path, &[".ts", ".tsx", ".js", ".jsx", ".rs"])
        && !is_lock_or_generated_path(path)
}

fn path_has_extension(path: &str, extensions: &[&str]) -> bool {
    let lower = path.to_ascii_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

pub fn generate_evidence_candidates(input: EvidenceCandidateInput<'_>) -> Vec<EvidenceCandidate> {
    let history_lower = input.history_section.to_ascii_lowercase();
    let blast_lower = input.blast_section.to_ascii_lowercase();
    let mut candidates = Vec::new();

    if !input.sensitive_paths.is_empty() {
        candidates.push(EvidenceCandidate {
            id: "sensitive-path-needs-boundary-proof".to_string(),
            kind: "sensitive_path_without_boundary_evidence".to_string(),
            severity_hint: "high".to_string(),
            confidence: 0.86,
            affected_files: input.sensitive_paths.iter().take(8).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "changed_file".to_string(),
                label: "Sensitive changed path".to_string(),
                detail: Some(input.sensitive_paths.join(", ")),
            }],
            scale: format!("{} sensitive file(s)", input.sensitive_paths.len()),
            why_it_matters: "Sensitive paths need explicit auth, secret, persistence, shell, IPC, or data-boundary proof before the review can call the change safe.".to_string(),
            caveats: vec![
                "Path matching is conservative and may over-label files with words like command, auth, token, or schema.".to_string(),
            ],
            open_questions: vec![
                "Which boundary changed, and what test or manual proof verifies it?".to_string(),
            ],
            suggested_checks: vec![
                "Inspect callers and trust boundaries for each sensitive file.".to_string(),
                "Require at least one concrete test, command, or runtime artifact for the boundary.".to_string(),
            ],
        });
    }

    if !input.structural_evidence.is_empty() {
        let affected_files = input
            .structural_evidence
            .iter()
            .map(|evidence| evidence.file.clone())
            .take(8)
            .collect::<Vec<_>>();
        let evidence_refs = input
            .structural_evidence
            .iter()
            .take(6)
            .map(|evidence| EvidenceRef {
                kind: "ast_grep".to_string(),
                label: format!("{} ({})", evidence.label, evidence.rule_id),
                detail: Some(match (evidence.line, evidence.snippet.as_deref()) {
                    (Some(line), Some(snippet)) => {
                        format!("{}:{line} — {snippet}", evidence.file)
                    }
                    (Some(line), None) => format!("{}:{line}", evidence.file),
                    (None, Some(snippet)) => format!("{} — {snippet}", evidence.file),
                    (None, None) => evidence.file.clone(),
                }),
            })
            .collect::<Vec<_>>();

        candidates.push(EvidenceCandidate {
            id: "structural-boundary-evidence".to_string(),
            kind: "structural_boundary_evidence".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.76,
            affected_files,
            evidence_refs,
            scale: format!("{} structural match(es)", input.structural_evidence.len()),
            why_it_matters: "Syntax-aware changed-file search found boundary-shaped API usage that deserves targeted review instead of relying on text search alone.".to_string(),
            caveats: vec![
                "`ast-grep` rules are optional and intentionally narrow; absence of matches is not proof of safety.".to_string(),
            ],
            open_questions: vec![
                "Does each structural match have an explicit trust-boundary, caller, or regression check?".to_string(),
            ],
            suggested_checks: vec![
                "Inspect each structural match and attach the nearest command or caller proof.".to_string(),
            ],
        });
    }

    if history_lower.contains("status=failed")
        || history_lower.contains("status failed")
        || history_lower.contains(" failed")
    {
        candidates.push(EvidenceCandidate {
            id: "failed-command-evidence".to_string(),
            kind: "failed_command_evidence".to_string(),
            severity_hint: "high".to_string(),
            confidence: 0.82,
            affected_files: input.changed_files.iter().take(8).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "history".to_string(),
                label: "Prior command/test evidence includes a failed status".to_string(),
                detail: first_matching_line(input.history_section, "failed"),
            }],
            scale: "prior command/test signal".to_string(),
            why_it_matters: "A failed command near this change can turn a clean-looking review into false confidence if the agent does not reconcile it.".to_string(),
            caveats: vec![
                "History snippets are compact; inspect the raw command context before treating this as a defect.".to_string(),
            ],
            open_questions: vec![
                "Was the failing command rerun successfully after the current diff?".to_string(),
            ],
            suggested_checks: vec![
                "Find the exact command, exit status, and artifact before accepting any related fix.".to_string(),
            ],
        });
    } else if history_lower.contains("status=stale") || history_lower.contains(" stale") {
        candidates.push(EvidenceCandidate {
            id: "stale-command-evidence".to_string(),
            kind: "stale_command_evidence".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.74,
            affected_files: input.changed_files.iter().take(8).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "history".to_string(),
                label: "Prior command/test evidence is stale".to_string(),
                detail: first_matching_line(input.history_section, "stale"),
            }],
            scale: "prior command/test signal".to_string(),
            why_it_matters:
                "Stale verification should not be used as proof that the current diff is safe."
                    .to_string(),
            caveats: vec![
                "The command may still be relevant, but it predates the current review evidence."
                    .to_string(),
            ],
            open_questions: vec![
                "Which smallest command should be rerun for the touched files?".to_string(),
            ],
            suggested_checks: vec![
                "Rerun the nearest test/build/lint command and attach the fresh artifact."
                    .to_string(),
            ],
        });
    }

    if touches_ui_surface(input.changed_files)
        && !history_lower.contains("screenshot")
        && !history_lower.contains("trace")
        && !history_lower.contains("browser")
        && !history_lower.contains("playwright")
    {
        candidates.push(EvidenceCandidate {
            id: "ui-change-needs-browser-proof".to_string(),
            kind: "ui_without_browser_proof".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.72,
            affected_files: input
                .changed_files
                .iter()
                .filter(|path| is_ui_path(path))
                .take(8)
                .cloned()
                .collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "changed_file".to_string(),
                label: "UI-facing file changed".to_string(),
                detail: None,
            }],
            scale: "UI surface changed".to_string(),
            why_it_matters: "Agent-written UI changes often pass static review while breaking layout, loading, empty, or interaction states.".to_string(),
            caveats: vec![
                "Static path matching cannot prove the UI is user-visible.".to_string(),
            ],
            open_questions: vec![
                "What route or user task proves the changed UI still works?".to_string(),
            ],
            suggested_checks: vec![
                "Run or attach a browser/Playwright artifact for the affected route.".to_string(),
            ],
        });
    }

    if input.changed_lines > 100 {
        candidates.push(EvidenceCandidate {
            id: "large-diff-needs-scope-control".to_string(),
            kind: "large_diff_scope_risk".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.7,
            affected_files: input.changed_files.iter().take(10).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "diff".to_string(),
                label: "Changed line count".to_string(),
                detail: Some(input.changed_lines.to_string()),
            }],
            scale: format!(
                "{} changed line(s) across {} file(s)",
                input.changed_lines,
                input.changed_files.len()
            ),
            why_it_matters: "Large agent diffs are more likely to contain scope drift, accidental refactors, and hidden behavior changes.".to_string(),
            caveats: vec![
                "Generated, lockfile, and mechanical changes can inflate this count.".to_string(),
            ],
            open_questions: vec![
                "Which files are essential to the stated goal, and which are incidental?".to_string(),
            ],
            suggested_checks: vec![
                "Review changed files by goal-critical path first and reject unrelated edits.".to_string(),
            ],
        });
    }

    let lock_or_generated = input
        .changed_files
        .iter()
        .filter(|path| is_lock_or_generated_path(path))
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    if !lock_or_generated.is_empty() {
        candidates.push(EvidenceCandidate {
            id: "generated-or-lockfile-context-noise".to_string(),
            kind: "generated_or_lockfile_noise".to_string(),
            severity_hint: "low".to_string(),
            confidence: 0.68,
            affected_files: lock_or_generated.clone(),
            evidence_refs: vec![EvidenceRef {
                kind: "changed_file".to_string(),
                label: "Generated or lockfile path changed".to_string(),
                detail: Some(lock_or_generated.join(", ")),
            }],
            scale: format!("{} generated/lockfile path(s)", lock_or_generated.len()),
            why_it_matters: "Generated and lockfile changes can dominate context and hide the handwritten code that actually needs review.".to_string(),
            caveats: vec![
                "Lockfile changes can be legitimate and should not be dropped automatically.".to_string(),
            ],
            open_questions: vec![
                "Which handwritten dependency or generated-source change caused these files to move?".to_string(),
            ],
            suggested_checks: vec![
                "Review the source change that produced the generated or lockfile diff.".to_string(),
            ],
        });
    }

    if !has_fresh_verification_signal(&history_lower)
        && input.changed_lines > 0
        && !input.changed_files.is_empty()
    {
        candidates.push(EvidenceCandidate {
            id: "no-fresh-verification-evidence".to_string(),
            kind: "not_verified".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.66,
            affected_files: input.changed_files.iter().take(8).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "history".to_string(),
                label: "No fresh command/test/browser proof found in compact history".to_string(),
                detail: None,
            }],
            scale: format!("{} changed file(s)", input.changed_files.len()),
            why_it_matters: "A review without fresh verification evidence can only prove static plausibility, not that the change works.".to_string(),
            caveats: vec![
                "The user may have run verification outside indexed agent sessions.".to_string(),
            ],
            open_questions: vec![
                "What is the smallest relevant command or browser task for this diff?".to_string(),
            ],
            suggested_checks: vec![
                "Attach a fresh command, test, log, screenshot, or trace before marking the finding fixed.".to_string(),
            ],
        });
    }

    if blast_lower.contains("caller")
        && (blast_lower.contains("6 caller")
            || blast_lower.contains("7 caller")
            || blast_lower.contains("8 caller")
            || blast_lower.contains("9 caller"))
    {
        candidates.push(EvidenceCandidate {
            id: "blast-radius-callers-need-compatibility-proof".to_string(),
            kind: "blast_radius_compatibility_risk".to_string(),
            severity_hint: "medium".to_string(),
            confidence: 0.62,
            affected_files: input.changed_files.iter().take(8).cloned().collect(),
            evidence_refs: vec![EvidenceRef {
                kind: "blast_radius".to_string(),
                label: "Blast-radius summary mentions multiple callers".to_string(),
                detail: first_matching_line(input.blast_section, "caller"),
            }],
            scale: "multi-caller change".to_string(),
            why_it_matters: "Behavior changes to symbols with several callers can silently regress paths outside the diff.".to_string(),
            caveats: vec![
                "This is derived from a compact blast-radius summary, not a complete call graph.".to_string(),
            ],
            open_questions: vec![
                "Which callers are covered by tests or manual proof?".to_string(),
            ],
            suggested_checks: vec![
                "Inspect high-risk callers and require compatibility proof for changed behavior.".to_string(),
            ],
        });
    }

    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| severity_rank(&b.severity_hint).cmp(&severity_rank(&a.severity_hint)))
            .then_with(|| a.id.cmp(&b.id))
    });
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

pub fn generate_procedure_steps(candidates: &[EvidenceCandidate]) -> Vec<EvidenceProcedureStep> {
    let mut steps = Vec::new();
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["sensitive_path_without_boundary_evidence"],
        EvidenceProcedureStep {
            id: "review_changed_sensitive_path".to_string(),
            procedure: "review_changed_sensitive_path".to_string(),
            status: "ready".to_string(),
            candidate_ids: Vec::new(),
            input: "Sensitive changed files and their callers/trust boundaries.".to_string(),
            action: "Inspect the changed boundary, caller assumptions, persistence/IPC/shell/data handling, and nearest tests.".to_string(),
            output: "Boundary review note with accepted risk, rejected candidate, or required fix.".to_string(),
            artifact: "code reference, test output, or manual boundary proof".to_string(),
            gate: "Every sensitive candidate is confirmed, rejected, or left needs_proof with a named missing artifact.".to_string(),
            blocked_on: Vec::new(),
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["structural_boundary_evidence"],
        EvidenceProcedureStep {
            id: "inspect_structural_matches".to_string(),
            procedure: "inspect_structural_matches".to_string(),
            status: "ready".to_string(),
            candidate_ids: Vec::new(),
            input: "Optional ast-grep matches from changed TypeScript/Rust files.".to_string(),
            action: "Inspect each syntax match, decide whether it is a real boundary or regression risk, and attach nearest caller/test proof.".to_string(),
            output: "Structural evidence note with accepted, rejected, or needs-proof matches.".to_string(),
            artifact: "ast-grep match reference, caller code reference, or focused verification output".to_string(),
            gate: "Each structural match is confirmed, rejected, or left needs_proof with a named missing artifact.".to_string(),
            blocked_on: Vec::new(),
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &[
            "failed_command_evidence",
            "stale_command_evidence",
            "not_verified",
        ],
        EvidenceProcedureStep {
            id: "rerun_relevant_verification".to_string(),
            procedure: "rerun_relevant_verification".to_string(),
            status: "blocked".to_string(),
            candidate_ids: Vec::new(),
            input: "Failed, stale, or missing command/test/browser evidence.".to_string(),
            action: "Choose the smallest relevant repo command or browser task, rerun it, and attach the fresh output.".to_string(),
            output: "Fresh pass/fail evidence linked to the review.".to_string(),
            artifact: "command log, test report, screenshot, trace, or QA run artifact".to_string(),
            gate: "No candidate is marked confirmed or fixed using stale or missing verification.".to_string(),
            blocked_on: vec!["fresh verification artifact".to_string()],
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["ui_without_browser_proof"],
        EvidenceProcedureStep {
            id: "verify_ui_route_change".to_string(),
            procedure: "verify_ui_route_change".to_string(),
            status: "blocked".to_string(),
            candidate_ids: Vec::new(),
            input: "UI-facing changed files and the route or task they affect.".to_string(),
            action: "Open the affected route or run the nearest Playwright flow, then capture interaction, console, and network evidence.".to_string(),
            output: "Browser proof linked to the candidate and affected route.".to_string(),
            artifact: "screenshot, trace, console/network log, or Playwright report".to_string(),
            gate: "Changed UI has at least one fresh visual or interaction artifact, or remains needs_proof.".to_string(),
            blocked_on: vec!["browser or Playwright artifact".to_string()],
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["large_diff_scope_risk"],
        EvidenceProcedureStep {
            id: "scope_control_review".to_string(),
            procedure: "scope_control_review".to_string(),
            status: "ready".to_string(),
            candidate_ids: Vec::new(),
            input: "Large changed-file set and stated task goal.".to_string(),
            action: "Classify changed files as goal-critical, support, generated, or unrelated before accepting findings/fixes.".to_string(),
            output: "Scope note listing kept, questioned, and rejected edits.".to_string(),
            artifact: "changed-file classification note".to_string(),
            gate: "Unrelated edits are called out before the review is marked shippable.".to_string(),
            blocked_on: Vec::new(),
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["generated_or_lockfile_noise"],
        EvidenceProcedureStep {
            id: "inspect_generated_or_lockfile_source".to_string(),
            procedure: "inspect_generated_or_lockfile_source".to_string(),
            status: "ready".to_string(),
            candidate_ids: Vec::new(),
            input: "Generated or lockfile paths in the diff.".to_string(),
            action: "Find the handwritten dependency, schema, or source change that caused the generated/lockfile movement.".to_string(),
            output: "Source-change note or rejection if generated noise has no matching source reason.".to_string(),
            artifact: "source file reference or dependency command output".to_string(),
            gate: "Generated/lockfile movement is explained by a source change before it absorbs review context.".to_string(),
            blocked_on: Vec::new(),
        },
    );
    push_step_for_kinds(
        &mut steps,
        candidates,
        &["blast_radius_compatibility_risk"],
        EvidenceProcedureStep {
            id: "inspect_blast_radius_callers".to_string(),
            procedure: "inspect_blast_radius_callers".to_string(),
            status: "ready".to_string(),
            candidate_ids: Vec::new(),
            input: "Blast-radius summary and changed symbols with multiple callers.".to_string(),
            action: "Inspect high-risk callers and verify the changed contract remains compatible.".to_string(),
            output: "Caller compatibility note with covered and uncovered paths.".to_string(),
            artifact: "caller code references, focused test output, or manual proof".to_string(),
            gate: "At least the highest-risk caller path is checked, or the candidate remains needs_proof.".to_string(),
            blocked_on: Vec::new(),
        },
    );

    steps.truncate(MAX_CANDIDATES);
    steps
}

pub fn render_procedure_steps_for_prompt(steps: &[EvidenceProcedureStep]) -> String {
    if steps.is_empty() {
        return String::new();
    }

    let mut out = String::from("\nProcedure steps (deterministic evidence gates):\n");
    out.push_str("Use these to decide what proof is missing. Treat blocked steps as explicit remaining work unless the current evidence resolves them.\n");

    for step in steps {
        out.push_str(&format!(
            "- [{}] {} status={} candidates={}\n",
            step.id,
            step.procedure,
            step.status,
            step.candidate_ids.join(", ")
        ));
        out.push_str(&format!("  action: {}\n", step.action));
        out.push_str(&format!("  artifact: {}\n", step.artifact));
        out.push_str(&format!("  gate: {}\n", step.gate));
        if !step.blocked_on.is_empty() {
            out.push_str(&format!("  blocked_on: {}\n", step.blocked_on.join(", ")));
        }
    }

    out
}

fn push_step_for_kinds(
    steps: &mut Vec<EvidenceProcedureStep>,
    candidates: &[EvidenceCandidate],
    kinds: &[&str],
    mut step: EvidenceProcedureStep,
) {
    let candidate_ids = candidates
        .iter()
        .filter(|candidate| kinds.iter().any(|kind| *kind == candidate.kind))
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();

    if candidate_ids.is_empty() {
        return;
    }

    step.candidate_ids = candidate_ids;
    steps.push(step);
}

pub fn render_candidates_for_prompt(candidates: &[EvidenceCandidate]) -> String {
    if candidates.is_empty() {
        return String::new();
    }

    let mut out = String::from("\nRanked evidence candidates (deterministic pre-review search):\n");
    out.push_str("Use these as leads, not conclusions. Validate, reject, or preserve open questions explicitly in the review.\n");

    for candidate in candidates {
        out.push_str(&format!(
            "- [{}] {} severity_hint={} confidence={:.2} scale={}\n",
            candidate.id,
            candidate.kind,
            candidate.severity_hint,
            candidate.confidence,
            candidate.scale
        ));
        if !candidate.affected_files.is_empty() {
            out.push_str(&format!(
                "  affected_files: {}\n",
                candidate.affected_files.join(", ")
            ));
        }
        if !candidate.evidence_refs.is_empty() {
            let refs = candidate
                .evidence_refs
                .iter()
                .take(4)
                .map(|evidence| match evidence.detail.as_deref() {
                    Some(detail) => format!("{}:{}={}", evidence.kind, evidence.label, detail),
                    None => format!("{}:{}", evidence.kind, evidence.label),
                })
                .collect::<Vec<_>>();
            out.push_str(&format!("  evidence_refs: {}\n", refs.join(" | ")));
        }
        out.push_str(&format!("  why: {}\n", candidate.why_it_matters));
        if !candidate.open_questions.is_empty() {
            out.push_str(&format!(
                "  open_questions: {}\n",
                candidate.open_questions.join(" | ")
            ));
        }
        if !candidate.suggested_checks.is_empty() {
            out.push_str(&format!(
                "  suggested_checks: {}\n",
                candidate.suggested_checks.join(" | ")
            ));
        }
    }

    out
}

fn severity_rank(severity: &str) -> usize {
    match severity {
        "critical" => 4,
        "high" => 3,
        "medium" => 2,
        "low" => 1,
        _ => 0,
    }
}

fn first_matching_line(text: &str, needle: &str) -> Option<String> {
    let needle = needle.to_ascii_lowercase();
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains(&needle))
        .map(|line| line.trim().chars().take(220).collect())
}

fn has_fresh_verification_signal(history_lower: &str) -> bool {
    (history_lower.contains("status=passed") || history_lower.contains("status passed"))
        && !history_lower.contains("status=stale")
}

fn touches_ui_surface(paths: &[String]) -> bool {
    paths.iter().any(|path| is_ui_path(path))
}

fn is_ui_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tsx")
        || lower.ends_with(".jsx")
        || lower.contains("/pages/")
        || lower.contains("/components/")
        || lower.contains("/routes/")
        || lower.contains("/app/")
        || lower.contains("playwright")
}

fn is_lock_or_generated_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with("package-lock.json")
        || lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("yarn.lock")
        || lower.ends_with("cargo.lock")
        || lower.ends_with(".generated.ts")
        || lower.ends_with(".generated.rs")
        || lower.contains("/generated/")
        || lower.contains("/dist/")
        || lower.contains("/build/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        changed_files: &'a [String],
        changed_lines: usize,
        sensitive_paths: &'a [String],
        history_section: &'a str,
        blast_section: &'a str,
        structural_evidence: &'a [StructuralEvidenceMatch],
    ) -> EvidenceCandidateInput<'a> {
        EvidenceCandidateInput {
            changed_files,
            changed_lines,
            sensitive_paths,
            history_section,
            blast_section,
            structural_evidence,
        }
    }

    #[test]
    fn flags_sensitive_path_and_failed_command_evidence() {
        let files = vec![
            "src/auth/session.ts".to_string(),
            "src/components/Login.tsx".to_string(),
        ];
        let sensitive = vec!["src/auth/session.ts".to_string()];
        let candidates = generate_evidence_candidates(input(
            &files,
            42,
            &sensitive,
            "\nPrior command/test evidence:\n- npm test status=failed artifact=/tmp/test.log\n",
            "",
            &[],
        ));

        assert!(candidates
            .iter()
            .any(|candidate| candidate.kind == "sensitive_path_without_boundary_evidence"));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.kind == "failed_command_evidence"));
    }

    #[test]
    fn flags_ui_change_without_browser_proof() {
        let files = vec!["apps/web/src/pages/Billing.tsx".to_string()];
        let candidates = generate_evidence_candidates(input(&files, 12, &[], "", "", &[]));

        let ui = candidates
            .iter()
            .find(|candidate| candidate.kind == "ui_without_browser_proof")
            .expect("ui candidate");
        assert_eq!(
            ui.open_questions[0],
            "What route or user task proves the changed UI still works?"
        );
    }

    #[test]
    fn stale_history_does_not_count_as_fresh_verification() {
        let files = vec!["src/lib/review.ts".to_string()];
        let candidates = generate_evidence_candidates(input(
            &files,
            8,
            &[],
            "Prior command/test evidence:\n- npm run test status=stale\n",
            "",
            &[],
        ));

        assert!(candidates
            .iter()
            .any(|candidate| candidate.kind == "stale_command_evidence"));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.kind == "not_verified"));
    }

    #[test]
    fn renders_prompt_section_with_open_questions() {
        let files = vec!["src/auth/session.ts".to_string()];
        let sensitive = vec!["src/auth/session.ts".to_string()];
        let candidates = generate_evidence_candidates(input(&files, 20, &sensitive, "", "", &[]));
        let rendered = render_candidates_for_prompt(&candidates);

        assert!(rendered.contains("Ranked evidence candidates"));
        assert!(rendered.contains("sensitive-path-needs-boundary-proof"));
        assert!(rendered.contains("open_questions"));
    }

    #[test]
    fn generates_procedure_steps_for_candidate_gates() {
        let files = vec![
            "src/auth/session.ts".to_string(),
            "src/pages/Billing.tsx".to_string(),
        ];
        let sensitive = vec!["src/auth/session.ts".to_string()];
        let candidates = generate_evidence_candidates(input(&files, 34, &sensitive, "", "", &[]));
        let steps = generate_procedure_steps(&candidates);

        assert!(steps
            .iter()
            .any(|step| step.id == "review_changed_sensitive_path"));
        let browser_step = steps
            .iter()
            .find(|step| step.id == "verify_ui_route_change")
            .expect("ui procedure step");
        assert_eq!(browser_step.status, "blocked");
        assert!(browser_step
            .blocked_on
            .contains(&"browser or Playwright artifact".to_string()));
        assert!(browser_step
            .candidate_ids
            .contains(&"ui-change-needs-browser-proof".to_string()));
    }

    #[test]
    fn renders_procedure_steps_for_prompt() {
        let files = vec!["src/pages/Billing.tsx".to_string()];
        let candidates = generate_evidence_candidates(input(&files, 12, &[], "", "", &[]));
        let steps = generate_procedure_steps(&candidates);
        let rendered = render_procedure_steps_for_prompt(&steps);

        assert!(rendered.contains("Procedure steps"));
        assert!(rendered.contains("verify_ui_route_change"));
        assert!(rendered.contains("blocked_on: browser or Playwright artifact"));
    }

    #[test]
    fn parses_ast_grep_json_lines_into_structural_matches() {
        let rule = AstGrepRule {
            id: "ts-tauri-invoke",
            label: "Tauri IPC invoke call",
            lang: "ts",
            pattern: "invoke($CMD, $$$ARGS)",
            extensions: &[".ts"],
        };
        let stdout = r#"{"file":"src/lib/ipc.ts","range":{"start":{"line":12,"column":4}},"text":"invoke(\"run\", args)"}
{"file":"src/lib/ipc.ts","line":30,"lines":"invoke(\"save\", payload)"}"#;

        let matches = parse_ast_grep_json_output(stdout, &rule, "src/lib/ipc.ts");

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].rule_id, "ts-tauri-invoke");
        assert_eq!(matches[0].line, Some(12));
        assert_eq!(
            matches[1].snippet.as_deref(),
            Some("invoke(\"save\", payload)")
        );
    }

    #[test]
    fn structural_evidence_generates_candidate_and_gate() {
        let files = vec!["src/lib/ipc.ts".to_string()];
        let structural = vec![StructuralEvidenceMatch {
            rule_id: "ts-tauri-invoke".to_string(),
            label: "Tauri IPC invoke call".to_string(),
            file: "src/lib/ipc.ts".to_string(),
            line: Some(12),
            snippet: Some("invoke(\"run\", args)".to_string()),
        }];
        let candidates = generate_evidence_candidates(input(
            &files,
            10,
            &[],
            "Prior command/test evidence:\n- npm run test status=passed\n",
            "",
            &structural,
        ));
        let candidate = candidates
            .iter()
            .find(|candidate| candidate.kind == "structural_boundary_evidence")
            .expect("structural candidate");
        assert_eq!(candidate.evidence_refs[0].kind, "ast_grep");
        let steps = generate_procedure_steps(&candidates);
        assert!(steps
            .iter()
            .any(|step| step.id == "inspect_structural_matches"));
    }
}
