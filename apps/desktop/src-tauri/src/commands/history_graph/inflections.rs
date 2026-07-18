//! Pure candidate-inflection derivation over normalized SQLite history facts.
//!
//! This module never invokes Git, reconstructs graphs, or publishes landmarks.
//! Its scores describe unusual observed change size, not intent or quality.

use super::stable_graph_id;
use serde::Serialize;
use std::collections::BTreeSet;

pub(crate) const ALGORITHM: &str = "robust-churn-files";
pub(crate) const ALGORITHM_VERSION: u32 = 1;
// Keep the detector aligned with the normalized history reader ceiling instead
// of disabling landmarks for repositories between 10k and 100k revisions.
pub(crate) const MAX_REVISIONS: usize = 100_000;
pub(crate) const MIN_BASELINE: usize = 12;
pub(crate) const MIN_CHURN: u64 = 200;
pub(crate) const MIN_CHANGED_FILES: u64 = 8;
pub(crate) const SCORE_THRESHOLD_MILLI: i64 = 3_500;

const MAD_NORMALIZATION: f64 = 1.4826;
const LOG_SCALE_FLOOR: f64 = 0.05;
const MAX_NOISE_DISCOUNT_MILLI: u64 = 650;
const RELEASE_ONLY_WEIGHT_MILLI: i64 = 750;

/// One aggregate produced from persisted revision and revision-path rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryInflectionFact {
    pub(crate) revision_sha: String,
    pub(crate) ordinal: i64,
    pub(crate) churn: Option<u64>,
    pub(crate) changed_files: u64,
    pub(crate) binary_files: u64,
    pub(crate) generated_files: u64,
    pub(crate) vendored_files: u64,
    /// Derived only from persisted path facts, not merely from a Git tag.
    pub(crate) release_only: bool,
    pub(crate) merge: bool,
    pub(crate) coverage_complete: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoverageStatus {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct InflectionCoverage {
    pub(crate) status: CoverageStatus,
    pub(crate) input_revisions: usize,
    pub(crate) comparable_revisions: usize,
    pub(crate) churn_revisions: usize,
    pub(crate) required_revisions: usize,
    pub(crate) reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct InflectionThresholds {
    pub(crate) max_revisions: usize,
    pub(crate) minimum_baseline: usize,
    pub(crate) minimum_churn: u64,
    pub(crate) minimum_changed_files: u64,
    pub(crate) score_milli: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BaselineComponent {
    pub(crate) population: usize,
    pub(crate) median_log_micros: i64,
    pub(crate) mad_log_micros: i64,
    pub(crate) scale_log_micros: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct InflectionBaseline {
    pub(crate) churn: BaselineComponent,
    pub(crate) changed_files: BaselineComponent,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ComponentScore {
    pub(crate) observed: u64,
    pub(crate) robust_deviations_milli: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct InflectionCandidate {
    pub(crate) id: String,
    pub(crate) revision_sha: String,
    pub(crate) ordinal: i64,
    pub(crate) aggregate_score_milli: i64,
    pub(crate) noise_weight_milli: i64,
    pub(crate) churn: Option<ComponentScore>,
    pub(crate) changed_files: ComponentScore,
    pub(crate) binary_files: u64,
    pub(crate) generated_files: u64,
    pub(crate) vendored_files: u64,
    pub(crate) release_only: bool,
    pub(crate) merge: bool,
    pub(crate) structural: Option<StructuralChangeMeasurements>,
    pub(crate) reasons: Vec<String>,
    pub(crate) caveats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct StructuralChangeMeasurements {
    pub(crate) node_changes: u64,
    pub(crate) edge_changes: u64,
    pub(crate) community_changes: u64,
    pub(crate) hub_changes: u64,
    pub(crate) bridge_changes: u64,
    pub(crate) coverage_gap: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct InflectionDerivation {
    pub(crate) algorithm: &'static str,
    pub(crate) algorithm_version: u32,
    pub(crate) thresholds: InflectionThresholds,
    pub(crate) coverage: InflectionCoverage,
    pub(crate) baseline: Option<InflectionBaseline>,
    pub(crate) candidates: Vec<InflectionCandidate>,
}

pub(crate) fn derive_history_inflections(facts: &[HistoryInflectionFact]) -> InflectionDerivation {
    if facts.len() > MAX_REVISIONS {
        return unavailable(
            facts.len(),
            0,
            0,
            CoverageStatus::Unavailable,
            format!(
                "The input contains {} revisions; the bounded detector accepts at most {MAX_REVISIONS}.",
                facts.len()
            ),
        );
    }

    let mut ordered = facts.to_vec();
    ordered.sort_by(|a, b| {
        a.ordinal
            .cmp(&b.ordinal)
            .then_with(|| a.revision_sha.cmp(&b.revision_sha))
    });
    let unique = ordered
        .iter()
        .map(|fact| fact.revision_sha.as_str())
        .collect::<BTreeSet<_>>();
    let invalid_counts = ordered.iter().any(|fact| {
        fact.binary_files > fact.changed_files
            || fact.generated_files > fact.changed_files
            || fact.vendored_files > fact.changed_files
    });
    if unique.len() != ordered.len() || unique.contains("") || invalid_counts {
        return unavailable(
            facts.len(),
            0,
            0,
            CoverageStatus::Unavailable,
            "Normalized facts contain invalid revision identities or path counts.".to_string(),
        );
    }

    // Bounded or incomplete rows do not influence a baseline or become a marker.
    let comparable = ordered
        .iter()
        .filter(|fact| fact.coverage_complete)
        .collect::<Vec<_>>();
    let churn_logs = comparable
        .iter()
        .filter_map(|fact| fact.churn.map(log_value))
        .collect::<Vec<_>>();
    if comparable.len() < MIN_BASELINE || churn_logs.len() < MIN_BASELINE {
        let status = if ordered.is_empty() || comparable.len() == ordered.len() {
            CoverageStatus::Unavailable
        } else {
            CoverageStatus::Partial
        };
        return unavailable(
            facts.len(),
            comparable.len(),
            churn_logs.len(),
            status,
            format!(
                "The detector requires {MIN_BASELINE} complete revisions with churn; found {} complete and {} with churn.",
                comparable.len(),
                churn_logs.len()
            ),
        );
    }

    let churn_baseline = robust_baseline(churn_logs);
    let files_baseline = robust_baseline(
        comparable
            .iter()
            .map(|fact| log_value(fact.changed_files))
            .collect(),
    );
    let missing_churn = comparable
        .iter()
        .filter(|fact| fact.churn.is_none())
        .count();
    let incomplete = ordered.len() - comparable.len();
    let mut coverage_reasons = Vec::new();
    if incomplete > 0 {
        coverage_reasons.push(format!(
            "{incomplete} incomplete revisions were excluded from the baseline and candidates."
        ));
    }
    if missing_churn > 0 {
        coverage_reasons.push(format!(
            "{missing_churn} comparable revisions have no line churn; file-count scoring remains available."
        ));
    }
    let coverage_status = if coverage_reasons.is_empty() {
        CoverageStatus::Complete
    } else {
        CoverageStatus::Partial
    };

    let mut candidates = comparable
        .iter()
        .filter_map(|fact| candidate(fact, &churn_baseline, &files_baseline, coverage_status))
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.aggregate_score_milli
            .cmp(&a.aggregate_score_milli)
            .then_with(|| a.ordinal.cmp(&b.ordinal))
            .then_with(|| a.revision_sha.cmp(&b.revision_sha))
    });

    InflectionDerivation {
        algorithm: ALGORITHM,
        algorithm_version: ALGORITHM_VERSION,
        thresholds: thresholds(),
        coverage: InflectionCoverage {
            status: coverage_status,
            input_revisions: facts.len(),
            comparable_revisions: comparable.len(),
            churn_revisions: churn_baseline.population,
            required_revisions: MIN_BASELINE,
            reasons: coverage_reasons,
        },
        baseline: Some(InflectionBaseline {
            churn: churn_baseline.contract(),
            changed_files: files_baseline.contract(),
        }),
        candidates,
    }
}

fn candidate(
    fact: &HistoryInflectionFact,
    churn_baseline: &RobustBaseline,
    files_baseline: &RobustBaseline,
    coverage_status: CoverageStatus,
) -> Option<InflectionCandidate> {
    if fact.churn.unwrap_or(0) < MIN_CHURN && fact.changed_files < MIN_CHANGED_FILES {
        return None;
    }
    let churn = fact
        .churn
        .map(|value| score_component(value, churn_baseline));
    let changed_files = score_component(fact.changed_files, files_baseline);
    let scores = std::iter::once(changed_files.robust_deviations_milli)
        .chain(churn.as_ref().map(|score| score.robust_deviations_milli))
        .map(|score| score.max(0) as f64 / 1_000.0)
        .collect::<Vec<_>>();
    let rms = (scores.iter().map(|score| score * score).sum::<f64>() / scores.len() as f64).sqrt();
    let noise_weight_milli = noise_weight_milli(fact);
    let aggregate_score_milli = round_milli(rms * noise_weight_milli as f64 / 1_000.0);
    if aggregate_score_milli < SCORE_THRESHOLD_MILLI {
        return None;
    }

    let mut reasons = vec![match fact.churn {
        Some(churn) => format!(
            "Observed {churn} changed lines across {} files.",
            fact.changed_files
        ),
        None => format!(
            "Observed {} changed files; line churn is unavailable.",
            fact.changed_files
        ),
    }];
    if let Some(score) = &churn {
        reasons.push(format!(
            "Log-scaled churn is {} robust deviations above the repository median.",
            display_milli(score.robust_deviations_milli)
        ));
    }
    reasons.push(format!(
        "Log-scaled file count is {} robust deviations above the repository median; the noise-adjusted aggregate is {} (threshold {}).",
        display_milli(changed_files.robust_deviations_milli),
        display_milli(aggregate_score_milli),
        display_milli(SCORE_THRESHOLD_MILLI)
    ));

    let mut caveats = Vec::new();
    push_count_caveat(&mut caveats, fact.generated_files, "generated");
    push_count_caveat(&mut caveats, fact.vendored_files, "vendored");
    if fact.release_only {
        caveats.push(
            "Persisted paths classify this as release-only change noise; it receives an additional score discount."
                .to_string(),
        );
    }
    if fact.merge {
        caveats.push(
            "This is a merge revision; observed change size is not attributed to one parent or author."
                .to_string(),
        );
    }
    if fact.binary_files > 0 {
        caveats.push(format!(
            "{} binary files have no comparable line churn.",
            fact.binary_files
        ));
    }
    if fact.churn.is_none() {
        caveats.push("This candidate uses changed-file deviation only.".to_string());
    }
    if coverage_status == CoverageStatus::Partial {
        caveats.push("The repository baseline has partial normalized-fact coverage.".to_string());
    }
    caveats.push(
        "Statistical change size does not establish intent, causation, impact, or quality."
            .to_string(),
    );

    Some(InflectionCandidate {
        id: stable_graph_id(
            "candidate-inflection-v1",
            &format!("{ALGORITHM_VERSION}\0{}", fact.revision_sha),
        ),
        revision_sha: fact.revision_sha.clone(),
        ordinal: fact.ordinal,
        aggregate_score_milli,
        noise_weight_milli,
        churn,
        changed_files,
        binary_files: fact.binary_files,
        generated_files: fact.generated_files,
        vendored_files: fact.vendored_files,
        release_only: fact.release_only,
        merge: fact.merge,
        structural: None,
        reasons,
        caveats,
    })
}

pub(crate) fn enrich_candidate_with_structural_delta(
    candidate: &mut InflectionCandidate,
    structural: StructuralChangeMeasurements,
) {
    candidate.reasons.push(format!(
        "Persisted structural delta observed {} node, {} edge, {} community, {} hub, and {} bridge changes.",
        structural.node_changes,
        structural.edge_changes,
        structural.community_changes,
        structural.hub_changes,
        structural.bridge_changes
    ));
    if let Some(gap) = structural.coverage_gap.as_deref() {
        candidate
            .caveats
            .push(format!("Structural-delta coverage is partial: {gap}."));
    }
    candidate.structural = Some(structural);
}

fn push_count_caveat(caveats: &mut Vec<String>, count: u64, kind: &str) {
    if count > 0 {
        caveats.push(format!(
            "{count} {kind} files contribute to the observed change and down-weight the score."
        ));
    }
}

fn noise_weight_milli(fact: &HistoryInflectionFact) -> i64 {
    let noisy = fact
        .generated_files
        .saturating_add(fact.vendored_files)
        .min(fact.changed_files);
    let discount = MAX_NOISE_DISCOUNT_MILLI
        .saturating_mul(noisy)
        .saturating_add(fact.changed_files / 2)
        .checked_div(fact.changed_files)
        .unwrap_or_default();
    let content_weight = 1_000_i64 - discount as i64;
    if fact.release_only {
        (content_weight * RELEASE_ONLY_WEIGHT_MILLI + 500) / 1_000
    } else {
        content_weight
    }
}

#[derive(Debug)]
struct RobustBaseline {
    population: usize,
    median: f64,
    mad: f64,
    scale: f64,
}

impl RobustBaseline {
    fn contract(&self) -> BaselineComponent {
        BaselineComponent {
            population: self.population,
            median_log_micros: round_micros(self.median),
            mad_log_micros: round_micros(self.mad),
            scale_log_micros: round_micros(self.scale),
        }
    }
}

fn robust_baseline(mut values: Vec<f64>) -> RobustBaseline {
    let population = values.len();
    let center = median(&mut values);
    let mut deviations = values
        .into_iter()
        .map(|value| (value - center).abs())
        .collect::<Vec<_>>();
    let mad = median(&mut deviations);
    RobustBaseline {
        population,
        median: center,
        mad,
        scale: (mad * MAD_NORMALIZATION).max(LOG_SCALE_FLOOR),
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn score_component(observed: u64, baseline: &RobustBaseline) -> ComponentScore {
    ComponentScore {
        observed,
        robust_deviations_milli: round_milli(
            (log_value(observed) - baseline.median) / baseline.scale,
        ),
    }
}

fn log_value(value: u64) -> f64 {
    (value as f64).ln_1p()
}

fn round_milli(value: f64) -> i64 {
    (value * 1_000.0).round() as i64
}

fn round_micros(value: f64) -> i64 {
    (value * 1_000_000.0).round() as i64
}

fn display_milli(value: i64) -> String {
    let sign = if value < 0 { "-" } else { "" };
    format!("{sign}{}.{:03}", value.abs() / 1_000, value.abs() % 1_000)
}

fn thresholds() -> InflectionThresholds {
    InflectionThresholds {
        max_revisions: MAX_REVISIONS,
        minimum_baseline: MIN_BASELINE,
        minimum_churn: MIN_CHURN,
        minimum_changed_files: MIN_CHANGED_FILES,
        score_milli: SCORE_THRESHOLD_MILLI,
    }
}

fn unavailable(
    input_revisions: usize,
    comparable_revisions: usize,
    churn_revisions: usize,
    status: CoverageStatus,
    reason: String,
) -> InflectionDerivation {
    InflectionDerivation {
        algorithm: ALGORITHM,
        algorithm_version: ALGORITHM_VERSION,
        thresholds: thresholds(),
        coverage: InflectionCoverage {
            status,
            input_revisions,
            comparable_revisions,
            churn_revisions,
            required_revisions: MIN_BASELINE,
            reasons: vec![reason],
        },
        baseline: None,
        candidates: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(ordinal: i64, sha: &str, churn: Option<u64>, files: u64) -> HistoryInflectionFact {
        HistoryInflectionFact {
            revision_sha: sha.into(),
            ordinal,
            churn,
            changed_files: files,
            binary_files: 0,
            generated_files: 0,
            vendored_files: 0,
            release_only: false,
            merge: false,
            coverage_complete: true,
        }
    }

    fn normal_history() -> Vec<HistoryInflectionFact> {
        (0..20)
            .map(|i| {
                fact(
                    i,
                    &format!("normal-{i:02}"),
                    Some(80 + i as u64 * 7),
                    2 + i as u64 % 3,
                )
            })
            .collect()
    }

    #[test]
    fn insufficient_baseline_is_explicit_and_emits_nothing() {
        let result = derive_history_inflections(&normal_history()[..11]);
        assert_eq!(result.coverage.status, CoverageStatus::Unavailable);
        assert_eq!(result.coverage.comparable_revisions, 11);
        assert!(result.baseline.is_none());
        assert!(result.candidates.is_empty());
        assert!(result.coverage.reasons[0].contains("requires 12"));
    }

    #[test]
    fn detects_extremes_with_stable_tie_ordering() {
        let mut facts = normal_history();
        facts.push(fact(51, "extreme-b", Some(80_000), 70));
        facts.push(fact(50, "extreme-a", Some(80_000), 70));
        let first = derive_history_inflections(&facts);
        facts.reverse();
        let second = derive_history_inflections(&facts);

        assert_eq!(first, second);
        assert_eq!(first.coverage.status, CoverageStatus::Complete);
        assert_eq!(
            first
                .candidates
                .iter()
                .take(2)
                .map(|point| point.revision_sha.as_str())
                .collect::<Vec<_>>(),
            ["extreme-a", "extreme-b"]
        );
    }

    #[test]
    fn down_weights_and_caveats_generated_vendor_release_noise() {
        let mut facts = normal_history();
        facts.push(fact(30, "clean", Some(100_000), 100));
        let mut noisy = fact(31, "noisy", Some(1_000_000_000), 1_000);
        noisy.generated_files = 700;
        noisy.vendored_files = 300;
        noisy.release_only = true;
        facts.push(noisy);

        let result = derive_history_inflections(&facts);
        let find = |sha| {
            result
                .candidates
                .iter()
                .find(|point| point.revision_sha == sha)
                .unwrap()
        };
        let (clean, noisy) = (find("clean"), find("noisy"));
        assert_eq!(noisy.noise_weight_milli, 263);
        assert!(noisy.aggregate_score_milli < clean.aggregate_score_milli);
        for expected in [
            "generated",
            "vendored",
            "release-only",
            "does not establish intent",
        ] {
            assert!(noisy.caveats.iter().any(|caveat| caveat.contains(expected)));
        }
    }

    #[test]
    fn binary_candidate_can_use_files_against_a_complete_churn_baseline() {
        let mut facts = normal_history();
        let mut binary = fact(30, "binary-extreme", None, 80);
        binary.binary_files = 80;
        facts.push(binary);

        let result = derive_history_inflections(&facts);
        assert_eq!(result.coverage.status, CoverageStatus::Partial);
        let point = result
            .candidates
            .iter()
            .find(|point| point.revision_sha == "binary-extreme")
            .unwrap();
        assert!(point.churn.is_none());
        assert!(point.caveats.iter().any(|caveat| caveat.contains("binary")));
        assert!(point
            .caveats
            .iter()
            .any(|caveat| caveat.contains("partial")));
    }

    #[test]
    fn partial_or_small_inputs_do_not_invent_candidates() {
        let mut partial = normal_history();
        for fact in partial.iter_mut().take(10) {
            fact.coverage_complete = false;
        }
        let result = derive_history_inflections(&partial);
        assert_eq!(result.coverage.status, CoverageStatus::Partial);
        assert!(result.candidates.is_empty());

        let mut small = normal_history();
        small.push(fact(30, "too-small", Some(199), 7));
        let result = derive_history_inflections(&small);
        assert!(!result
            .candidates
            .iter()
            .any(|point| point.revision_sha == "too-small"));
    }

    #[test]
    fn rejects_unbounded_or_invalid_facts() {
        let result =
            derive_history_inflections(&vec![fact(0, "same", Some(1), 1); MAX_REVISIONS + 1]);
        assert_eq!(result.coverage.status, CoverageStatus::Unavailable);
        assert!(result.coverage.reasons[0].contains("at most 100000"));

        let duplicate = [fact(0, "same", Some(1), 1), fact(1, "same", Some(2), 2)];
        assert!(derive_history_inflections(&duplicate).candidates.is_empty());
    }
}
