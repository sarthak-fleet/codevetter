//! Local audience validation and staged verification summaries.
//!
//! The signal diagnostics are a Rust port of the reusable evaluation
//! architecture from ShipRank (`taste/src/lib/scoring.ts`): compare only
//! like-for-like judgments, treat order reversals as indecisive, surface
//! majority strength and preference cycles, and keep confidence conservative.

use crate::DbState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tauri::State;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudienceValidationRun {
    pub id: String,
    pub review_id: String,
    pub repo_path: Option<String>,
    pub audience: String,
    pub task: String,
    pub candidate_a: String,
    pub candidate_a_artifact: Option<String>,
    pub candidate_b: Option<String>,
    pub candidate_b_artifact: Option<String>,
    pub criteria: Vec<String>,
    pub min_responses: i64,
    pub required: bool,
    pub waived_reason: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudienceValidationResponse {
    pub id: String,
    pub run_id: String,
    pub participant_id: String,
    pub provenance: String,
    pub criterion: String,
    pub candidate_a: String,
    pub candidate_b: Option<String>,
    pub preferred_candidate: Option<String>,
    pub reverse_preferred_candidate: Option<String>,
    pub confidence: f64,
    pub task_passed: Option<bool>,
    pub feedback: Option<String>,
    pub evidence_ref: Option<String>,
    pub elapsed_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAudienceValidationInput {
    pub review_id: String,
    pub repo_path: Option<String>,
    pub audience: String,
    pub task: String,
    pub candidate_a: String,
    pub candidate_a_artifact: Option<String>,
    pub candidate_b: Option<String>,
    pub candidate_b_artifact: Option<String>,
    pub criteria: Vec<String>,
    pub min_responses: Option<i64>,
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddAudienceResponseInput {
    pub run_id: String,
    pub participant_id: Option<String>,
    pub provenance: String,
    pub criterion: String,
    pub candidate_a: String,
    pub candidate_b: Option<String>,
    pub preferred_candidate: Option<String>,
    pub reverse_preferred_candidate: Option<String>,
    pub confidence: Option<f64>,
    pub task_passed: Option<bool>,
    pub feedback: Option<String>,
    pub evidence_ref: Option<String>,
    pub elapsed_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CriterionSignal {
    pub criterion: String,
    pub comparable_judgments: usize,
    pub decisive_judgments: usize,
    pub majority_strength: f64,
    pub agreement: f64,
    pub low_confidence_count: usize,
    pub order_inconsistent_count: usize,
    pub cycle_detected: bool,
    pub consensus_candidate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudienceSignalDiagnostics {
    pub response_count: usize,
    pub human_response_count: usize,
    pub agent_response_count: usize,
    pub imported_response_count: usize,
    pub mean_agreement: f64,
    pub mean_majority_strength: f64,
    pub low_confidence_count: usize,
    pub order_inconsistent_count: usize,
    pub criteria_with_cycles: Vec<String>,
    pub signal_strength: String,
    pub criteria: Vec<CriterionSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerificationStage {
    pub status: String,
    pub label: String,
    pub evidence: Vec<String>,
    pub caveats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StagedVerificationSummary {
    pub review: VerificationStage,
    pub executable_test: VerificationStage,
    pub audience: VerificationStage,
    pub aggregate_status: String,
    pub confidence: String,
    pub human_validation_fulfilled: bool,
    pub proof_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudienceValidationBundle {
    pub run: Option<AudienceValidationRun>,
    pub responses: Vec<AudienceValidationResponse>,
    pub diagnostics: AudienceSignalDiagnostics,
    pub verification: StagedVerificationSummary,
}

fn clean_required(value: String, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{label} is required"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_provenance(value: &str) -> Result<&'static str, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "agent" | "agent_simulated" => Ok("agent"),
        "human" => Ok("human"),
        "imported" => Ok("imported"),
        _ => Err("provenance must be agent, human, or imported".to_string()),
    }
}

fn run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudienceValidationRun> {
    let criteria_json: String = row.get(9)?;
    let required: i64 = row.get(11)?;
    Ok(AudienceValidationRun {
        id: row.get(0)?,
        review_id: row.get(1)?,
        repo_path: row.get(2)?,
        audience: row.get(3)?,
        task: row.get(4)?,
        candidate_a: row.get(5)?,
        candidate_a_artifact: row.get(6)?,
        candidate_b: row.get(7)?,
        candidate_b_artifact: row.get(8)?,
        criteria: serde_json::from_str(&criteria_json).unwrap_or_default(),
        min_responses: row.get(10)?,
        required: required != 0,
        waived_reason: row.get(12)?,
        status: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn response_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AudienceValidationResponse> {
    let task_passed: Option<i64> = row.get(10)?;
    Ok(AudienceValidationResponse {
        id: row.get(0)?,
        run_id: row.get(1)?,
        participant_id: row.get(2)?,
        provenance: row.get(3)?,
        criterion: row.get(4)?,
        candidate_a: row.get(5)?,
        candidate_b: row.get(6)?,
        preferred_candidate: row.get(7)?,
        reverse_preferred_candidate: row.get(8)?,
        confidence: row.get(9)?,
        task_passed: task_passed.map(|value| value != 0),
        feedback: row.get(11)?,
        evidence_ref: row.get(12)?,
        elapsed_ms: row.get(13)?,
        created_at: row.get(14)?,
    })
}

fn latest_run(
    conn: &Connection,
    review_id: &str,
) -> rusqlite::Result<Option<AudienceValidationRun>> {
    conn.query_row(
        "SELECT id, review_id, repo_path, audience, task, candidate_a,
                candidate_a_artifact, candidate_b, candidate_b_artifact,
                criteria_json, min_responses, required, waived_reason, status,
                created_at, updated_at
         FROM audience_validation_runs
         WHERE review_id = ?1
         ORDER BY datetime(created_at) DESC
         LIMIT 1",
        params![review_id],
        run_from_row,
    )
    .optional()
}

fn responses_for_run(
    conn: &Connection,
    run_id: &str,
) -> rusqlite::Result<Vec<AudienceValidationResponse>> {
    let mut stmt = conn.prepare(
        "SELECT id, run_id, participant_id, provenance, criterion, candidate_a,
                candidate_b, preferred_candidate, reverse_preferred_candidate,
                confidence, task_passed, feedback, evidence_ref, elapsed_ms, created_at
         FROM audience_validation_responses
         WHERE run_id = ?1
         ORDER BY datetime(created_at) ASC",
    )?;
    let rows = stmt.query_map(params![run_id], response_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct PairKey(String, String);

impl PairKey {
    fn new(a: &str, b: &str) -> Self {
        if a <= b {
            Self(a.to_string(), b.to_string())
        } else {
            Self(b.to_string(), a.to_string())
        }
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn has_cycle(candidates: &BTreeSet<String>, preferences: &HashMap<PairKey, String>) -> bool {
    for a in candidates {
        for b in candidates {
            for c in candidates {
                if a == b || b == c || a == c {
                    continue;
                }
                if preferences.get(&PairKey::new(a, b)) == Some(a)
                    && preferences.get(&PairKey::new(b, c)) == Some(b)
                    && preferences.get(&PairKey::new(c, a)) == Some(c)
                {
                    return true;
                }
            }
        }
    }
    false
}

pub fn summarize_signals(responses: &[AudienceValidationResponse]) -> AudienceSignalDiagnostics {
    let mut by_criterion: BTreeMap<String, Vec<&AudienceValidationResponse>> = BTreeMap::new();
    for response in responses {
        by_criterion
            .entry(response.criterion.clone())
            .or_default()
            .push(response);
    }

    let mut criteria = Vec::new();
    for (criterion, criterion_responses) in by_criterion {
        let mut pair_votes: BTreeMap<PairKey, Vec<String>> = BTreeMap::new();
        let mut candidates = BTreeSet::new();
        let mut low_confidence_count = 0;
        let mut order_inconsistent_count = 0;
        let mut comparable_judgments = 0;

        for response in criterion_responses {
            if response.confidence < 0.58 {
                low_confidence_count += 1;
            }
            let Some(candidate_b) = response.candidate_b.as_deref() else {
                continue;
            };
            comparable_judgments += 1;
            candidates.insert(response.candidate_a.clone());
            candidates.insert(candidate_b.to_string());

            if let Some(reverse) = response.reverse_preferred_candidate.as_deref() {
                if response.preferred_candidate.as_deref() != Some(reverse) {
                    order_inconsistent_count += 1;
                    continue;
                }
            }

            if let Some(preferred) = response.preferred_candidate.as_deref() {
                if preferred == response.candidate_a || preferred == candidate_b {
                    pair_votes
                        .entry(PairKey::new(&response.candidate_a, candidate_b))
                        .or_default()
                        .push(preferred.to_string());
                }
            }
        }

        let decisive_judgments: usize = pair_votes.values().map(Vec::len).sum();
        let mut majority_strengths = Vec::new();
        let mut candidate_scores: BTreeMap<String, usize> = BTreeMap::new();
        let mut preferences = HashMap::new();
        for (pair, votes) in &pair_votes {
            if votes.is_empty() {
                continue;
            }
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for vote in votes {
                *counts.entry(vote).or_default() += 1;
            }
            let mut ranked = counts.into_iter().collect::<Vec<_>>();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            let top = ranked[0];
            let tied = ranked.get(1).is_some_and(|next| next.1 == top.1);
            majority_strengths.push(top.1 as f64 / votes.len() as f64);
            if !tied {
                preferences.insert(pair.clone(), top.0.to_string());
                *candidate_scores.entry(top.0.to_string()).or_default() += 1;
            }
        }

        let majority_strength = if majority_strengths.is_empty() {
            0.0
        } else {
            majority_strengths.iter().sum::<f64>() / majority_strengths.len() as f64
        };
        let agreement = if majority_strength <= 0.5 {
            0.0
        } else {
            (majority_strength - 0.5) * 2.0
        };
        let consensus_candidate = candidate_scores
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|entry| entry.0);

        criteria.push(CriterionSignal {
            criterion,
            comparable_judgments,
            decisive_judgments,
            majority_strength: round3(majority_strength),
            agreement: round3(agreement),
            low_confidence_count,
            order_inconsistent_count,
            cycle_detected: has_cycle(&candidates, &preferences),
            consensus_candidate,
        });
    }

    let mean_agreement = if criteria.is_empty() {
        0.0
    } else {
        criteria.iter().map(|signal| signal.agreement).sum::<f64>() / criteria.len() as f64
    };
    let mean_majority_strength = if criteria.is_empty() {
        0.0
    } else {
        criteria
            .iter()
            .map(|signal| signal.majority_strength)
            .sum::<f64>()
            / criteria.len() as f64
    };
    let order_inconsistent_count = criteria
        .iter()
        .map(|signal| signal.order_inconsistent_count)
        .sum();
    let low_confidence_count = criteria
        .iter()
        .map(|signal| signal.low_confidence_count)
        .sum();
    let criteria_with_cycles = criteria
        .iter()
        .filter(|signal| signal.cycle_detected)
        .map(|signal| signal.criterion.clone())
        .collect::<Vec<_>>();
    let signal_strength = if mean_agreement >= 0.55
        && mean_majority_strength >= 0.78
        && order_inconsistent_count == 0
        && criteria_with_cycles.is_empty()
    {
        "strong"
    } else if mean_agreement >= 0.25 && mean_majority_strength >= 0.68 {
        "moderate"
    } else if mean_agreement >= 0.05 || mean_majority_strength >= 0.58 {
        "weak"
    } else {
        "noise"
    };

    AudienceSignalDiagnostics {
        response_count: responses.len(),
        human_response_count: responses
            .iter()
            .filter(|response| response.provenance == "human")
            .count(),
        agent_response_count: responses
            .iter()
            .filter(|response| response.provenance == "agent")
            .count(),
        imported_response_count: responses
            .iter()
            .filter(|response| response.provenance == "imported")
            .count(),
        mean_agreement: round3(mean_agreement),
        mean_majority_strength: round3(mean_majority_strength),
        low_confidence_count,
        order_inconsistent_count,
        criteria_with_cycles,
        signal_strength: signal_strength.to_string(),
        criteria,
    }
}

fn empty_diagnostics() -> AudienceSignalDiagnostics {
    summarize_signals(&[])
}

fn build_verification_summary(
    conn: &Connection,
    review_id: &str,
    run: Option<&AudienceValidationRun>,
    diagnostics: &AudienceSignalDiagnostics,
) -> Result<StagedVerificationSummary, String> {
    let review_row: Option<(String, i64, Option<f64>)> = conn
        .query_row(
            "SELECT status, COALESCE(findings_count, 0), score_composite
             FROM local_reviews WHERE id = ?1",
            params![review_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let (review_status, findings_count, review_score) =
        review_row.unwrap_or_else(|| ("missing".to_string(), 0, None));
    let review = VerificationStage {
        status: if review_status == "completed" {
            "completed".to_string()
        } else if review_status == "missing" {
            "not_run".to_string()
        } else {
            review_status.clone()
        },
        label: "Code review".to_string(),
        evidence: vec![format!(
            "{} finding(s) · score {}",
            findings_count,
            review_score
                .map(|score| format!("{score:.0}"))
                .unwrap_or_else(|| "unavailable".to_string())
        )],
        caveats: if findings_count > 0 {
            vec!["Review findings still require disposition and verification.".to_string()]
        } else {
            Vec::new()
        },
    };

    // The database-only summary cannot establish that browser evidence still
    // matches the current worktree/config/manifest/source identity. Review's
    // read-only warm-evidence adapter performs that qualification. Legacy
    // synthetic QA remains readable in its own surface but cannot satisfy or
    // block this exact-current executable stage.
    let executable_test = VerificationStage {
        status: "not_verified".to_string(),
        label: "Executable test".to_string(),
        evidence: Vec::new(),
        caveats: vec![
            "No exact-current warm verification evidence has been qualified.".to_string(),
        ],
    };

    let audience = match run {
        None => VerificationStage {
            status: "not_run".to_string(),
            label: "Audience validation".to_string(),
            evidence: Vec::new(),
            caveats: vec!["Audience validation has not been configured.".to_string()],
        },
        Some(run) if run.waived_reason.is_some() => VerificationStage {
            status: "waived".to_string(),
            label: "Audience validation".to_string(),
            evidence: vec![format!(
                "Not applicable: {}",
                run.waived_reason
                    .as_deref()
                    .unwrap_or("reason not recorded")
            )],
            caveats: vec!["No audience validation occurred.".to_string()],
        },
        Some(run) if diagnostics.response_count < run.min_responses as usize => VerificationStage {
            status: "incomplete".to_string(),
            label: "Audience validation".to_string(),
            evidence: vec![format!(
                "{} of {} required response(s) · {}",
                diagnostics.response_count, run.min_responses, run.audience
            )],
            caveats: vec!["Response threshold has not been met.".to_string()],
        },
        Some(run) => {
            let mode =
                if diagnostics.human_response_count > 0 && diagnostics.agent_response_count > 0 {
                    "mixed human + agent"
                } else if diagnostics.human_response_count > 0 {
                    "human"
                } else if diagnostics.agent_response_count > 0 {
                    "agent-simulated"
                } else {
                    "imported"
                };
            let mut caveats = Vec::new();
            if diagnostics.human_response_count == 0 {
                caveats.push(
                    "Human validation is not fulfilled; evidence is simulated or imported."
                        .to_string(),
                );
            }
            if diagnostics.order_inconsistent_count > 0 {
                caveats.push(format!(
                    "{} judgment(s) changed when candidate order changed.",
                    diagnostics.order_inconsistent_count
                ));
            }
            if !diagnostics.criteria_with_cycles.is_empty() {
                caveats.push(format!(
                    "Preference cycle detected for {}.",
                    diagnostics.criteria_with_cycles.join(", ")
                ));
            }
            VerificationStage {
                status: "completed".to_string(),
                label: "Audience validation".to_string(),
                evidence: vec![format!(
                    "{} response(s) · {mode} · {} signal · audience: {}",
                    diagnostics.response_count, diagnostics.signal_strength, run.audience
                )],
                caveats,
            }
        }
    };

    let aggregate_status = if review.status != "completed" {
        "incomplete"
    } else if executable_test.status == "failed" {
        "blocked"
    } else if executable_test.status != "passed"
        || (run.is_none_or(|run| run.required)
            && audience.status != "completed"
            && audience.status != "waived")
    {
        "incomplete"
    } else if findings_count > 0 {
        "needs_review"
    } else {
        "verified"
    };

    let human_validation_fulfilled =
        diagnostics.human_response_count > 0 && audience.status == "completed";
    let confidence = if aggregate_status == "blocked" || aggregate_status == "incomplete" {
        "low"
    } else if executable_test.status == "passed"
        && (human_validation_fulfilled || audience.status == "waived")
        && diagnostics.order_inconsistent_count == 0
        && diagnostics.criteria_with_cycles.is_empty()
    {
        "high"
    } else {
        "medium"
    };

    let audience_mode =
        if diagnostics.human_response_count > 0 && diagnostics.agent_response_count > 0 {
            "mixed"
        } else if diagnostics.human_response_count > 0 {
            "human"
        } else if diagnostics.agent_response_count > 0 {
            "agent-simulated"
        } else if diagnostics.imported_response_count > 0 {
            "imported"
        } else {
            "none"
        };
    let proof_markdown = format!(
        "### Staged verification\n\n- **Aggregate:** {aggregate_status} ({confidence} confidence)\n- **Code review:** {} — {}\n- **Executable test:** {} — {}\n- **Audience:** {} — mode: {audience_mode}; {} response(s); {} signal; human validation {}\n{}",
        review.status,
        review.evidence.join("; "),
        executable_test.status,
        executable_test.evidence.join("; "),
        audience.status,
        diagnostics.response_count,
        diagnostics.signal_strength,
        if human_validation_fulfilled { "fulfilled" } else { "not fulfilled" },
        if audience.caveats.is_empty() {
            String::new()
        } else {
            format!("- **Audience caveats:** {}", audience.caveats.join("; "))
        }
    );

    Ok(StagedVerificationSummary {
        review,
        executable_test,
        audience,
        aggregate_status: aggregate_status.to_string(),
        confidence: confidence.to_string(),
        human_validation_fulfilled,
        proof_markdown,
    })
}

pub(crate) fn load_bundle(
    conn: &Connection,
    review_id: &str,
) -> Result<AudienceValidationBundle, String> {
    let run = latest_run(conn, review_id).map_err(|error| error.to_string())?;
    let responses = match run.as_ref() {
        Some(run) => responses_for_run(conn, &run.id).map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    let diagnostics = if responses.is_empty() {
        empty_diagnostics()
    } else {
        summarize_signals(&responses)
    };
    let verification = build_verification_summary(conn, review_id, run.as_ref(), &diagnostics)?;
    Ok(AudienceValidationBundle {
        run,
        responses,
        diagnostics,
        verification,
    })
}

#[tauri::command]
pub async fn create_audience_validation_run(
    db: State<'_, DbState>,
    input: CreateAudienceValidationInput,
) -> Result<AudienceValidationBundle, String> {
    let review_id = clean_required(input.review_id, "review_id")?;
    let audience = clean_required(input.audience, "audience")?;
    let task = clean_required(input.task, "task")?;
    let candidate_a = clean_required(input.candidate_a, "candidate_a")?;
    let criteria = input
        .criteria
        .into_iter()
        .map(|criterion| criterion.trim().to_string())
        .filter(|criterion| !criterion.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if criteria.is_empty() {
        return Err("at least one criterion is required".to_string());
    }

    let conn = db.0.lock().map_err(|error| error.to_string())?;
    let review_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_reviews WHERE id = ?1)",
            params![review_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !review_exists {
        return Err("review not found".to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let criteria_json = serde_json::to_string(&criteria).map_err(|error| error.to_string())?;
    conn.execute(
        "INSERT INTO audience_validation_runs (
            id, review_id, repo_path, audience, task, candidate_a,
            candidate_a_artifact, candidate_b, candidate_b_artifact,
            criteria_json, min_responses, required, status, created_at, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'collecting',?13,?13)",
        params![
            id,
            review_id,
            clean_optional(input.repo_path),
            audience,
            task,
            candidate_a,
            clean_optional(input.candidate_a_artifact),
            clean_optional(input.candidate_b),
            clean_optional(input.candidate_b_artifact),
            criteria_json,
            input.min_responses.unwrap_or(3).clamp(1, 1000),
            if input.required.unwrap_or(true) { 1 } else { 0 },
            now,
        ],
    )
    .map_err(|error| error.to_string())?;
    load_bundle(&conn, &review_id)
}

#[tauri::command]
pub async fn add_audience_validation_response(
    db: State<'_, DbState>,
    input: AddAudienceResponseInput,
) -> Result<AudienceValidationBundle, String> {
    let run_id = clean_required(input.run_id, "run_id")?;
    let provenance = normalize_provenance(&input.provenance)?;
    let criterion = clean_required(input.criterion, "criterion")?;
    let candidate_a = clean_required(input.candidate_a, "candidate_a")?;
    let candidate_b = clean_optional(input.candidate_b);
    let preferred_candidate = clean_optional(input.preferred_candidate);
    let reverse_preferred_candidate = clean_optional(input.reverse_preferred_candidate);
    let confidence = input.confidence.unwrap_or(0.5).clamp(0.0, 1.0);
    if let Some(preferred) = preferred_candidate.as_deref() {
        if preferred != candidate_a && candidate_b.as_deref() != Some(preferred) {
            return Err("preferred_candidate must match candidate_a or candidate_b".to_string());
        }
    }

    let conn = db.0.lock().map_err(|error| error.to_string())?;
    let review_id: String = conn
        .query_row(
            "SELECT review_id FROM audience_validation_runs WHERE id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .map_err(|_| "audience validation run not found".to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let participant_id = clean_optional(input.participant_id)
        .unwrap_or_else(|| format!("anon-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO audience_validation_responses (
            id, run_id, participant_id, provenance, criterion, candidate_a,
            candidate_b, preferred_candidate, reverse_preferred_candidate,
            confidence, task_passed, feedback, evidence_ref, elapsed_ms, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            id,
            run_id,
            participant_id,
            provenance,
            criterion,
            candidate_a,
            candidate_b,
            preferred_candidate,
            reverse_preferred_candidate,
            confidence,
            input.task_passed.map(|passed| if passed { 1 } else { 0 }),
            clean_optional(input.feedback),
            clean_optional(input.evidence_ref),
            input.elapsed_ms.filter(|value| *value >= 0),
            now,
        ],
    )
    .map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE audience_validation_runs SET updated_at = ?2 WHERE id = ?1",
        params![run_id, now],
    )
    .map_err(|error| error.to_string())?;
    load_bundle(&conn, &review_id)
}

#[tauri::command]
pub async fn waive_audience_validation(
    db: State<'_, DbState>,
    review_id: String,
    reason: String,
) -> Result<AudienceValidationBundle, String> {
    let review_id = clean_required(review_id, "review_id")?;
    let reason = clean_required(reason, "reason")?;
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    let run = latest_run(&conn, &review_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "create an audience validation run before waiving it".to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE audience_validation_runs
         SET waived_reason = ?2, status = 'waived', updated_at = ?3
         WHERE id = ?1",
        params![run.id, reason, now],
    )
    .map_err(|error| error.to_string())?;
    load_bundle(&conn, &review_id)
}

#[tauri::command]
pub async fn get_audience_validation(
    db: State<'_, DbState>,
    review_id: String,
) -> Result<AudienceValidationBundle, String> {
    let review_id = clean_required(review_id, "review_id")?;
    let conn = db.0.lock().map_err(|error| error.to_string())?;
    load_bundle(&conn, &review_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn response(
        id: &str,
        criterion: &str,
        a: &str,
        b: &str,
        preferred: &str,
        reverse: Option<&str>,
        provenance: &str,
    ) -> AudienceValidationResponse {
        AudienceValidationResponse {
            id: id.to_string(),
            run_id: "run".to_string(),
            participant_id: id.to_string(),
            provenance: provenance.to_string(),
            criterion: criterion.to_string(),
            candidate_a: a.to_string(),
            candidate_b: Some(b.to_string()),
            preferred_candidate: Some(preferred.to_string()),
            reverse_preferred_candidate: reverse.map(ToOwned::to_owned),
            confidence: 0.8,
            task_passed: Some(true),
            feedback: None,
            evidence_ref: None,
            elapsed_ms: None,
            created_at: "2026-07-10T00:00:00Z".to_string(),
        }
    }

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        schema::run_migrations(&conn).expect("migrations");
        conn.execute(
            "INSERT INTO local_reviews (
                id, review_type, agent_used, score_composite, findings_count,
                status, created_at
             ) VALUES ('review-1', 'cli', 'codex', 100, 0, 'completed', '2026-07-10T00:00:00Z')",
            [],
        )
        .expect("review");
        conn
    }

    fn insert_run(conn: &Connection, min_responses: i64, waived_reason: Option<&str>) {
        conn.execute(
            "INSERT INTO audience_validation_runs (
                id, review_id, repo_path, audience, task, candidate_a,
                candidate_b, criteria_json, min_responses, required,
                waived_reason, status, created_at, updated_at
             ) VALUES (
                'run-1', 'review-1', '/tmp/repo', 'Target users', 'Complete onboarding',
                'A', 'B', '[\"clarity\"]', ?1, 1, ?2,
                CASE WHEN ?2 IS NULL THEN 'collecting' ELSE 'waived' END,
                '2026-07-10T00:02:00Z', '2026-07-10T00:02:00Z'
             )",
            params![min_responses, waived_reason],
        )
        .expect("audience run");
    }

    fn insert_db_response(conn: &Connection, id: &str, provenance: &str, preferred: &str) {
        conn.execute(
            "INSERT INTO audience_validation_responses (
                id, run_id, participant_id, provenance, criterion, candidate_a,
                candidate_b, preferred_candidate, reverse_preferred_candidate,
                confidence, task_passed, created_at
             ) VALUES (?1, 'run-1', ?1, ?2, 'clarity', 'A', 'B', ?3, ?3, 0.8, 1,
                '2026-07-10T00:03:00Z')",
            params![id, provenance, preferred],
        )
        .expect("audience response");
    }

    #[test]
    fn reversed_order_disagreement_is_indecisive() {
        let diagnostics =
            summarize_signals(&[response("1", "clarity", "A", "B", "A", Some("B"), "agent")]);
        assert_eq!(diagnostics.order_inconsistent_count, 1);
        assert_eq!(diagnostics.criteria[0].decisive_judgments, 0);
        assert_eq!(diagnostics.signal_strength, "noise");
    }

    #[test]
    fn majority_strength_and_provenance_are_preserved() {
        let diagnostics = summarize_signals(&[
            response("1", "trust", "A", "B", "A", Some("A"), "agent"),
            response("2", "trust", "A", "B", "A", Some("A"), "human"),
            response("3", "trust", "A", "B", "B", Some("B"), "imported"),
        ]);
        assert_eq!(diagnostics.agent_response_count, 1);
        assert_eq!(diagnostics.human_response_count, 1);
        assert_eq!(diagnostics.imported_response_count, 1);
        assert_eq!(
            diagnostics.criteria[0].consensus_candidate.as_deref(),
            Some("A")
        );
        assert_eq!(diagnostics.criteria[0].majority_strength, 0.667);
    }

    #[test]
    fn condorcet_cycle_is_reported() {
        let diagnostics = summarize_signals(&[
            response("1", "fit", "A", "B", "A", Some("A"), "human"),
            response("2", "fit", "B", "C", "B", Some("B"), "human"),
            response("3", "fit", "A", "C", "C", Some("C"), "human"),
        ]);
        assert_eq!(diagnostics.criteria_with_cycles, vec!["fit"]);
    }

    #[test]
    fn old_review_without_audience_data_remains_readable() {
        let conn = test_db();
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert!(bundle.run.is_none());
        assert_eq!(bundle.verification.review.status, "completed");
        assert_eq!(bundle.verification.executable_test.status, "not_verified");
        assert_eq!(bundle.verification.aggregate_status, "incomplete");
        assert_eq!(bundle.verification.audience.status, "not_run");
        assert!(!bundle.verification.human_validation_fulfilled);
    }

    #[test]
    fn legacy_executable_failure_cannot_block_exact_current_verification() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO synthetic_qa_runs (
                id, review_id, loop_id, runner_type, pass, duration_ms,
                console_errors, created_at
             ) VALUES ('qa-1', 'review-1', 'onboarding', 'playwright_builtin', 0, 10, 1, '2026-07-10T00:01:00Z')",
            [],
        )
        .expect("qa");
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert_eq!(bundle.verification.executable_test.status, "not_verified");
        assert_eq!(bundle.verification.aggregate_status, "incomplete");
        assert_eq!(bundle.verification.confidence, "low");
    }

    #[test]
    fn legacy_executable_pass_cannot_satisfy_exact_current_verification() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO synthetic_qa_runs (
                id, review_id, loop_id, runner_type, pass, duration_ms,
                console_errors, created_at
             ) VALUES ('qa-1', 'review-1', 'onboarding', 'playwright_builtin', 1, 10, 0, '2026-07-10T00:01:00Z')",
            [],
        )
        .expect("qa");
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert_eq!(bundle.verification.executable_test.status, "not_verified");
        assert_eq!(bundle.verification.aggregate_status, "incomplete");
    }

    #[test]
    fn audience_waiver_never_claims_human_validation() {
        let conn = test_db();
        insert_run(&conn, 3, Some("Backend-only schema repair"));
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert_eq!(bundle.verification.audience.status, "waived");
        assert_eq!(bundle.verification.aggregate_status, "incomplete");
        assert!(!bundle.verification.human_validation_fulfilled);
        assert!(bundle
            .verification
            .proof_markdown
            .contains("human validation not fulfilled"));
    }

    #[test]
    fn agent_only_panel_is_complete_but_not_human_validation() {
        let conn = test_db();
        insert_run(&conn, 1, None);
        insert_db_response(&conn, "agent-1", "agent", "A");
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert_eq!(bundle.verification.audience.status, "completed");
        assert_eq!(bundle.diagnostics.agent_response_count, 1);
        assert_eq!(bundle.diagnostics.human_response_count, 0);
        assert!(!bundle.verification.human_validation_fulfilled);
        assert_eq!(bundle.verification.confidence, "low");
    }

    #[test]
    fn mixed_panel_preserves_provenance_but_waits_for_exact_warm_evidence() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO synthetic_qa_runs (
                id, review_id, loop_id, runner_type, pass, duration_ms,
                console_errors, created_at
             ) VALUES ('qa-1', 'review-1', 'onboarding', 'playwright_builtin', 1, 10, 0,
                '2026-07-10T00:01:00Z')",
            [],
        )
        .expect("qa");
        insert_run(&conn, 2, None);
        insert_db_response(&conn, "agent-1", "agent", "A");
        insert_db_response(&conn, "human-1", "human", "A");
        let bundle = load_bundle(&conn, "review-1").expect("bundle");
        assert_eq!(bundle.diagnostics.agent_response_count, 1);
        assert_eq!(bundle.diagnostics.human_response_count, 1);
        assert!(bundle.verification.human_validation_fulfilled);
        assert_eq!(bundle.verification.aggregate_status, "incomplete");
        assert_eq!(bundle.verification.confidence, "low");
    }
}
