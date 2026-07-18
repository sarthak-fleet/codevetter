//! Compact append-only rule deltas anchored to exact archaeology revisions.
//!
//! The sidecar deliberately stores no source body, repository path, or
//! generation foreign key. Content-addressed snapshots therefore survive
//! normal generation cleanup while unchanged rules create no repeated event.

use super::contracts::{
    ArchaeologyCoverage, ArchaeologyCoverageState, ArchaeologyTemporalClausePayload,
    ArchaeologyTemporalEvidencePayload, ArchaeologyTemporalSnapshotPayload,
    ArchaeologyTemporalSpanPayload,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

const DIGEST_PREFIX: &str = "sha256:";
const MAX_REASON_COUNT: usize = 32;
const MAX_REASON_BYTES: usize = 256;
const MAX_TIMESTAMP_BYTES: usize = 128;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyTemporalLimits {
    pub max_rules: usize,
    pub max_clauses_per_rule: usize,
    pub max_evidence_per_clause: usize,
    pub max_spans_per_evidence: usize,
    pub max_snapshot_bytes: usize,
}

impl Default for ArchaeologyTemporalLimits {
    fn default() -> Self {
        Self {
            max_rules: 100_000,
            max_clauses_per_rule: 256,
            max_evidence_per_clause: 512,
            max_spans_per_evidence: 256,
            max_snapshot_bytes: 256 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyTemporalCoverageState {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalCoverageInput {
    pub state: ArchaeologyTemporalCoverageState,
    pub reasons: Vec<String>,
}

impl ArchaeologyTemporalCoverageInput {
    pub(crate) fn complete() -> Self {
        Self {
            state: ArchaeologyTemporalCoverageState::Complete,
            reasons: Vec::new(),
        }
    }
}

pub(crate) struct ArchaeologyTemporalProjection<'a> {
    pub repository_id: &'a str,
    pub generation_id: &'a str,
    pub prior_generation_id: Option<&'a str>,
    pub history_coverage: ArchaeologyTemporalCoverageInput,
    pub created_at: &'a str,
    pub limits: ArchaeologyTemporalLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyTemporalProjectionReport {
    pub temporal_generation_identity: String,
    pub catalog_identity: String,
    pub rule_count: usize,
    pub snapshot_count: usize,
    pub event_count: usize,
    pub coverage_state: ArchaeologyTemporalCoverageState,
    pub coverage_reasons: Vec<String>,
}

#[derive(Debug)]
struct Generation {
    id: String,
    repository_id: String,
    revision: String,
    parser_manifest: String,
    coverage: ArchaeologyCoverage,
}

#[derive(Debug, Clone)]
struct StoredRule {
    generated_rule_id: String,
    repository_id: String,
    stable_rule_identity: String,
    continuity_identity: String,
    kind: String,
    title: String,
    evidence_identity: String,
    parser_compatibility_identity: String,
    contradiction_identity: String,
    description_identity: String,
    clauses: Vec<StoredClause>,
}

#[derive(Debug, Clone)]
struct StoredClause {
    ordinal: u64,
    text: String,
    trust: String,
    confidence: String,
    caveats: serde_json::Value,
    evidence: BTreeMap<(String, String), StoredEvidence>,
}

#[derive(Debug, Clone)]
struct StoredEvidence {
    role: String,
    fact_identity: String,
    fact_kind: String,
    parser_identity: String,
    spans: Vec<ArchaeologyTemporalSpanPayload>,
}

#[derive(Debug, Clone)]
struct Snapshot {
    identity: String,
    rule: StoredRule,
    payload_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Observed,
    Introduced,
    Changed,
    Conflicted,
    Superseded,
    Removed,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Introduced => "introduced",
            Self::Changed => "changed",
            Self::Conflicted => "conflicted",
            Self::Superseded => "superseded",
            Self::Removed => "removed",
        }
    }
}

#[derive(Debug)]
struct ContinuityEdge {
    identity: String,
    continuity_identity: String,
    predecessor: String,
    successor: String,
    evidence_identity: String,
}

#[derive(Debug)]
struct TemporalEvent<'a> {
    kind: EventKind,
    stable_rule_identity: &'a str,
    continuity_identity: &'a str,
    predecessor_rule_identity: Option<&'a str>,
    successor_rule_identity: Option<&'a str>,
    before: Option<&'a Snapshot>,
    after: Option<&'a Snapshot>,
    continuity_edge_identity: Option<&'a str>,
    coverage_state: ArchaeologyTemporalCoverageState,
    coverage_reasons: EventCoverageReasons<'a>,
}

#[derive(Debug, Clone, Copy)]
struct EventCoverageReasons<'a> {
    common: &'a [String],
    extra: Option<&'static str>,
}

impl<'a> EventCoverageReasons<'a> {
    fn new(common: &'a [String], extra: Option<&'static str>) -> Self {
        Self {
            common,
            extra: extra.filter(|reason| !common.iter().any(|item| item == reason)),
        }
    }

    fn for_state(
        state: ArchaeologyTemporalCoverageState,
        common: &'a [String],
        extra: Option<&'static str>,
    ) -> Self {
        if state == ArchaeologyTemporalCoverageState::Complete {
            Self::new(&[], None)
        } else {
            Self::new(common, extra)
        }
    }
}

pub(crate) fn persist_temporal_projection(
    transaction: &Transaction<'_>,
    input: ArchaeologyTemporalProjection<'_>,
) -> Result<ArchaeologyTemporalProjectionReport, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    validate_token("repository", input.repository_id, 256)?;
    validate_token("generation", input.generation_id, 256)?;
    validate_timestamp(input.created_at)?;
    validate_coverage_input(&input.history_coverage)?;
    if input.prior_generation_id == Some(input.generation_id) {
        return Err("Temporal prior and current generations must differ".into());
    }

    let current = load_generation(
        transaction,
        input.repository_id,
        input.generation_id,
        &["staging", "ready"],
    )?
    .ok_or_else(|| "Exact archaeology temporal generation is unavailable".to_string())?;
    let prior = input
        .prior_generation_id
        .map(|generation_id| {
            validate_token("prior generation", generation_id, 256)?;
            load_generation(
                transaction,
                input.repository_id,
                generation_id,
                &["ready", "superseded"],
            )
        })
        .transpose()?
        .flatten();

    let current_rules = load_snapshots(transaction, &current, input.limits)?;
    profile_temporal_stage(profiling, "temporal.current_snapshots", started);
    let prior_rules = prior
        .as_ref()
        .map(|generation| load_snapshots(transaction, generation, input.limits))
        .transpose()?
        .unwrap_or_default();
    profile_temporal_stage(profiling, "temporal.prior_snapshots", started);
    // With no exact prior catalog, the temporal generation and its partial
    // coverage are sufficient. Persist both exact sides atomically only when
    // a real comparison becomes possible.
    if prior.is_some() {
        persist_snapshots(
            transaction,
            current_rules.values().chain(prior_rules.values()),
            input.created_at,
        )?;
    }
    profile_temporal_stage(profiling, "temporal.persist_snapshots", started);

    let catalog_identity = catalog_identity(input.repository_id, &current_rules);
    let prior_temporal_identity = input
        .prior_generation_id
        .map(|generation_id| {
            load_temporal_generation_identity(transaction, input.repository_id, generation_id)
        })
        .transpose()?
        .flatten();
    let (coverage_state, coverage_reasons) = projection_coverage(
        &input.history_coverage,
        &current,
        prior.as_ref(),
        input.prior_generation_id,
        prior_temporal_identity.as_deref(),
    )?;
    let coverage_json = encode_reasons(&coverage_reasons)?;
    let temporal_generation_identity = digest_fields(
        "archaeology-temporal-generation:v1",
        &[
            input.repository_id,
            &current.id,
            &current.revision,
            prior_temporal_identity.as_deref().unwrap_or("none"),
            &catalog_identity,
            coverage_name(coverage_state),
            &coverage_json,
        ],
    );
    persist_temporal_generation(
        transaction,
        input.repository_id,
        &current,
        prior_temporal_identity.as_deref(),
        &temporal_generation_identity,
        &catalog_identity,
        current_rules.len(),
        coverage_state,
        &coverage_json,
        input.created_at,
    )?;

    let exact = coverage_state == ArchaeologyTemporalCoverageState::Complete;
    let edges = if let Some(prior) = prior.as_ref() {
        load_continuity_edges(transaction, input.repository_id, &prior.id, &current.id)?
    } else {
        Vec::new()
    };
    let mut linked_predecessors = BTreeSet::new();
    let mut linked_successors = BTreeSet::new();
    let mut events = Vec::new();
    for edge in &edges {
        let before = prior_rules
            .get(&edge.predecessor)
            .ok_or("Temporal continuity predecessor snapshot is unavailable")?;
        let after = current_rules
            .get(&edge.successor)
            .ok_or("Temporal continuity successor snapshot is unavailable")?;
        if after.rule.evidence_identity != edge.evidence_identity {
            return Err("Temporal continuity evidence does not match its successor".into());
        }
        if !linked_predecessors.insert(edge.predecessor.clone())
            || !linked_successors.insert(edge.successor.clone())
        {
            return Err("Temporal continuity is ambiguous within one generation".into());
        }
        events.push(TemporalEvent {
            kind: EventKind::Superseded,
            stable_rule_identity: &edge.predecessor,
            continuity_identity: &edge.continuity_identity,
            predecessor_rule_identity: Some(&edge.predecessor),
            successor_rule_identity: Some(&edge.successor),
            before: Some(before),
            after: Some(after),
            continuity_edge_identity: Some(&edge.identity),
            coverage_state,
            coverage_reasons: EventCoverageReasons::for_state(
                coverage_state,
                &coverage_reasons,
                None,
            ),
        });
    }

    let identities = if prior.is_some() {
        prior_rules
            .keys()
            .chain(current_rules.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    for stable in &identities {
        if linked_predecessors.contains(stable) || linked_successors.contains(stable) {
            continue;
        }
        let before = prior_rules.get(stable);
        let after = current_rules.get(stable);
        let event = classify_event(
            stable,
            before,
            after,
            exact,
            coverage_state,
            &coverage_reasons,
        );
        if let Some(event) = event {
            events.push(event);
        }
    }
    events.sort_by(|left, right| {
        left.stable_rule_identity
            .cmp(right.stable_rule_identity)
            .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
    });
    for event in &events {
        persist_event(
            transaction,
            input.repository_id,
            &temporal_generation_identity,
            prior_temporal_identity.as_deref(),
            event,
            input.created_at,
        )?;
    }
    profile_temporal_stage(profiling, "temporal.persist_events", started);
    let event_count = events.len();
    drop(events);

    Ok(ArchaeologyTemporalProjectionReport {
        temporal_generation_identity,
        catalog_identity,
        rule_count: current_rules.len(),
        snapshot_count: referenced_snapshot_count(transaction, input.repository_id)?,
        event_count,
        coverage_state,
        coverage_reasons,
    })
}

fn profile_temporal_stage(enabled: bool, label: &str, started: Instant) {
    if enabled {
        eprintln!(
            "ARCHAEOLOGY_PROFILE\t{label}\t{:.3}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
}

fn classify_event<'a>(
    stable_rule_identity: &'a str,
    before: Option<&'a Snapshot>,
    after: Option<&'a Snapshot>,
    exact: bool,
    coverage_state: ArchaeologyTemporalCoverageState,
    coverage_reasons: &'a [String],
) -> Option<TemporalEvent<'a>> {
    let (kind, continuity_identity, event_before, event_after, state, extra_reason) =
        match (before, after) {
            (None, Some(after)) if exact => (
                EventKind::Introduced,
                after.rule.continuity_identity.as_str(),
                None,
                Some(after),
                ArchaeologyTemporalCoverageState::Complete,
                None,
            ),
            (Some(before), None) if exact => (
                EventKind::Removed,
                before.rule.continuity_identity.as_str(),
                Some(before),
                None,
                ArchaeologyTemporalCoverageState::Complete,
                None,
            ),
            (None, Some(after)) => (
                EventKind::Observed,
                after.rule.continuity_identity.as_str(),
                None,
                Some(after),
                coverage_state,
                None,
            ),
            (Some(before), None) => (
                EventKind::Observed,
                before.rule.continuity_identity.as_str(),
                Some(before),
                None,
                ArchaeologyTemporalCoverageState::Partial,
                Some("absence_not_proven"),
            ),
            (Some(before), Some(after)) if before.identity == after.identity => return None,
            (Some(before), Some(after))
                if before.rule.parser_compatibility_identity
                    != after.rule.parser_compatibility_identity =>
            {
                (
                    EventKind::Observed,
                    after.rule.continuity_identity.as_str(),
                    Some(before),
                    Some(after),
                    ArchaeologyTemporalCoverageState::Partial,
                    Some("parser_incompatible"),
                )
            }
            (Some(before), Some(after)) if !exact => (
                EventKind::Observed,
                after.rule.continuity_identity.as_str(),
                Some(before),
                Some(after),
                coverage_state,
                None,
            ),
            (Some(before), Some(after))
                if before.rule.contradiction_identity != after.rule.contradiction_identity =>
            {
                (
                    EventKind::Conflicted,
                    after.rule.continuity_identity.as_str(),
                    Some(before),
                    Some(after),
                    ArchaeologyTemporalCoverageState::Complete,
                    None,
                )
            }
            (Some(before), Some(after))
                if before.rule.evidence_identity != after.rule.evidence_identity =>
            {
                (
                    EventKind::Changed,
                    after.rule.continuity_identity.as_str(),
                    Some(before),
                    Some(after),
                    ArchaeologyTemporalCoverageState::Complete,
                    None,
                )
            }
            (Some(before), Some(after)) => (
                EventKind::Observed,
                after.rule.continuity_identity.as_str(),
                Some(before),
                Some(after),
                ArchaeologyTemporalCoverageState::Complete,
                None,
            ),
            (None, None) => return None,
        };
    Some(TemporalEvent {
        kind,
        stable_rule_identity,
        continuity_identity,
        predecessor_rule_identity: event_before
            .map(|snapshot| snapshot.rule.stable_rule_identity.as_str()),
        successor_rule_identity: None,
        before: event_before,
        after: event_after,
        continuity_edge_identity: None,
        coverage_state: state,
        coverage_reasons: EventCoverageReasons::for_state(state, coverage_reasons, extra_reason),
    })
}

fn load_generation(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
    statuses: &[&str],
) -> Result<Option<Generation>, String> {
    let statuses_json = serde_json::to_string(statuses).map_err(|error| error.to_string())?;
    transaction
        .query_row(
            "SELECT generation_id,repository_id,revision_sha,parser_identity,coverage_json
             FROM archaeology_generations
             WHERE repository_id=?1 AND generation_id=?2 AND schema_version=2
               AND status IN (SELECT value FROM json_each(?3))",
            params![repository_id, generation_id, statuses_json],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("Load archaeology temporal generation: {error}"))?
        .map(|row| {
            validate_revision(&row.2)?;
            let coverage: ArchaeologyCoverage = serde_json::from_str(&row.4)
                .map_err(|_| "Stored archaeology temporal coverage is invalid".to_string())?;
            Ok(Generation {
                id: row.0,
                repository_id: row.1,
                revision: row.2,
                parser_manifest: row.3,
                coverage,
            })
        })
        .transpose()
}

fn load_snapshots(
    transaction: &Transaction<'_>,
    generation: &Generation,
    limits: ArchaeologyTemporalLimits,
) -> Result<BTreeMap<String, Snapshot>, String> {
    let mut rules = load_rules(transaction, &generation.id, limits.max_rules)?;
    let clause_index = load_clauses(transaction, &generation.id, &mut rules, limits)?;
    load_evidence(
        transaction,
        &generation.id,
        &mut rules,
        &clause_index,
        limits,
    )?;
    let mut snapshots = BTreeMap::new();
    for rule in rules.into_values() {
        if rule.repository_id != generation.repository_id {
            return Err("Temporal rule crosses repository scope".into());
        }
        if rule.clauses.is_empty() {
            return Err("Temporal rule has no clauses".into());
        }
        let payload = ArchaeologyTemporalSnapshotPayload {
            title: rule.title.clone(),
            clauses: rule
                .clauses
                .iter()
                .map(|clause| {
                    Ok(ArchaeologyTemporalClausePayload {
                        ordinal: clause.ordinal,
                        text: clause.text.clone(),
                        trust: clause.trust.clone(),
                        confidence: clause.confidence.clone(),
                        caveats: serde_json::from_value(clause.caveats.clone())
                            .map_err(|_| "Temporal clause caveats are invalid".to_string())?,
                        evidence: clause
                            .evidence
                            .values()
                            .map(|evidence| ArchaeologyTemporalEvidencePayload {
                                role: evidence.role.clone(),
                                fact_identity: evidence.fact_identity.clone(),
                                fact_kind: evidence.fact_kind.clone(),
                                parser_identity: evidence.parser_identity.clone(),
                                spans: evidence.spans.clone(),
                            })
                            .collect(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        let payload_json = serde_json::to_string(&payload)
            .map_err(|error| format!("Encode archaeology temporal snapshot: {error}"))?;
        if payload_json.len() > limits.max_snapshot_bytes || payload_json.len() > 256 * 1024 {
            return Err("Archaeology temporal snapshot byte bound exceeded".into());
        }
        let identity = digest_fields(
            "archaeology-rule-temporal-snapshot:v1",
            &[
                &rule.stable_rule_identity,
                &rule.continuity_identity,
                &rule.kind,
                &rule.evidence_identity,
                &rule.parser_compatibility_identity,
                &rule.contradiction_identity,
                &rule.description_identity,
                &payload_json,
            ],
        );
        let stable = rule.stable_rule_identity.clone();
        if snapshots
            .insert(
                stable,
                Snapshot {
                    identity,
                    rule,
                    payload_json,
                },
            )
            .is_some()
        {
            return Err("Temporal generation has duplicate stable rule identities".into());
        }
    }
    Ok(snapshots)
}

fn load_rules(
    transaction: &Transaction<'_>,
    generation_id: &str,
    max_rules: usize,
) -> Result<BTreeMap<String, StoredRule>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT rule_id,repository_id,stable_rule_identity,continuity_identity,kind,title,
                    evidence_identity,parser_compatibility_identity,contradiction_identity,
                    description_identity
             FROM archaeology_rules rule
             WHERE generation_id=?1 AND identity_schema_version=2
               AND NOT EXISTS (
                 SELECT 1 FROM archaeology_rule_relations alias
                 WHERE alias.generation_id=rule.generation_id AND alias.kind='aliases'
                   AND alias.from_rule_id=rule.rule_id)
             ORDER BY stable_rule_identity,rule_id LIMIT ?2",
        )
        .map_err(|error| format!("Prepare archaeology temporal rules: {error}"))?;
    let rows = statement
        .query_map(params![generation_id, max_rules.saturating_add(1)], |row| {
            Ok(StoredRule {
                generated_rule_id: row.get(0)?,
                repository_id: row.get(1)?,
                stable_rule_identity: row.get(2)?,
                continuity_identity: row.get(3)?,
                kind: row.get(4)?,
                title: row.get(5)?,
                evidence_identity: row.get(6)?,
                parser_compatibility_identity: row.get(7)?,
                contradiction_identity: row.get(8)?,
                description_identity: row.get(9)?,
                clauses: Vec::new(),
            })
        })
        .map_err(|error| format!("Query archaeology temporal rules: {error}"))?;
    let mut result = BTreeMap::new();
    for row in rows {
        let rule = row.map_err(|error| format!("Read archaeology temporal rule: {error}"))?;
        for value in [
            &rule.stable_rule_identity,
            &rule.continuity_identity,
            &rule.evidence_identity,
            &rule.parser_compatibility_identity,
            &rule.contradiction_identity,
            &rule.description_identity,
        ] {
            validate_digest(value)?;
        }
        if result
            .insert(rule.generated_rule_id.clone(), rule)
            .is_some()
        {
            return Err("Temporal generation has duplicate rule occurrences".into());
        }
        if result.len() > max_rules {
            return Err("Archaeology temporal rule bound exceeded".into());
        }
    }
    Ok(result)
}

fn load_clauses(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rules: &mut BTreeMap<String, StoredRule>,
    limits: ArchaeologyTemporalLimits,
) -> Result<HashMap<String, (String, usize)>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json
             FROM archaeology_rule_clauses clause WHERE generation_id=?1
               AND NOT EXISTS (
                 SELECT 1 FROM archaeology_rule_relations alias
                 WHERE alias.generation_id=clause.generation_id AND alias.kind='aliases'
                   AND alias.from_rule_id=clause.rule_id)
             ORDER BY rule_id,ordinal,clause_id",
        )
        .map_err(|error| format!("Prepare archaeology temporal clauses: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(|error| format!("Query archaeology temporal clauses: {error}"))?;
    let mut clause_index = HashMap::new();
    for row in rows {
        let (rule_id, clause_id, ordinal, text, trust, confidence, caveats_json) =
            row.map_err(|error| format!("Read archaeology temporal clause: {error}"))?;
        let rule = rules
            .get_mut(&rule_id)
            .ok_or("Temporal clause references an unknown rule")?;
        if rule.clauses.len() >= limits.max_clauses_per_rule {
            return Err("Archaeology temporal clause bound exceeded".into());
        }
        let caveats: serde_json::Value = serde_json::from_str(&caveats_json)
            .map_err(|_| "Temporal clause caveats are invalid".to_string())?;
        if !caveats.is_array() {
            return Err("Temporal clause caveats must be an array".into());
        }
        let clause_ordinal = rule.clauses.len();
        rule.clauses.push(StoredClause {
            ordinal,
            text,
            trust,
            confidence,
            caveats,
            evidence: BTreeMap::new(),
        });
        if clause_index
            .insert(clause_id, (rule_id, clause_ordinal))
            .is_some()
        {
            return Err("Temporal generation has duplicate clause identities".into());
        }
    }
    Ok(clause_index)
}

fn load_evidence(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rules: &mut BTreeMap<String, StoredRule>,
    clause_index: &HashMap<String, (String, usize)>,
    limits: ArchaeologyTemporalLimits,
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "WITH evidence AS MATERIALIZED (
                SELECT generation.generation_id,link.owner_kind_code,
                       owner.identity AS owner_id,link.evidence_kind_code,
                       referenced.identity AS evidence_id,link.role_code
                FROM archaeology_evidence_links_compact link
                JOIN archaeology_generation_keys generation
                  ON generation.generation_key=link.generation_key
                 AND generation.generation_id=?1
                JOIN archaeology_evidence_identities owner
                  ON owner.generation_key=link.generation_key
                 AND owner.identity_key=link.owner_identity_key
                JOIN archaeology_evidence_identities referenced
                  ON referenced.generation_key=link.generation_key
                 AND referenced.identity_key=link.evidence_identity_key
             )
             SELECT clause.rule_id,clause.clause_id,
                    CASE clause_fact.role_code WHEN 1 THEN 'supporting' ELSE 'contradicting' END,
                    fact.fact_id,fact.kind,
                    fact.parser_id || '@' || unit.parser_version,unit.path_identity,
                    unit.content_hash,unit.hash_algorithm,fact.parser_id,unit.parser_id,
                    span.start_byte,span.end_byte,span.start_line,span.start_column,
                    span.end_line,span.end_column
             FROM archaeology_rule_clauses clause
             JOIN evidence clause_fact
               ON clause_fact.generation_id=clause.generation_id
              AND clause_fact.owner_kind_code=3 AND clause_fact.owner_id=clause.clause_id
              AND clause_fact.evidence_kind_code=2
              AND clause_fact.role_code IN (1,2)
             JOIN archaeology_facts fact
               ON fact.generation_id=clause_fact.generation_id
              AND fact.fact_id=clause_fact.evidence_id
             JOIN evidence fact_span
               ON fact_span.generation_id=fact.generation_id AND fact_span.owner_kind_code=1
               AND fact_span.owner_id=fact.fact_id AND fact_span.evidence_kind_code=1
              AND fact_span.role_code=1
             JOIN archaeology_source_spans span
               ON span.generation_id=fact_span.generation_id AND span.span_id=fact_span.evidence_id
             JOIN archaeology_source_units unit
               ON unit.generation_id=span.generation_id AND unit.source_unit_id=span.source_unit_id
             WHERE clause.generation_id=?1 AND unit.content_hash IS NOT NULL
               AND NOT EXISTS (
                 SELECT 1 FROM archaeology_rule_relations alias
                 WHERE alias.generation_id=clause.generation_id AND alias.kind='aliases'
                   AND alias.from_rule_id=clause.rule_id)
             ORDER BY clause.rule_id,clause.ordinal,clause.clause_id,clause_fact.role_code,
                      fact.fact_id,unit.path_identity,span.start_byte,span.end_byte",
        )
        .map_err(|error| format!("Prepare archaeology temporal evidence: {error}"))?;
    let rows = statement
        .query_map([generation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, u64>(11)?,
                row.get::<_, u64>(12)?,
                row.get::<_, u64>(13)?,
                row.get::<_, u64>(14)?,
                row.get::<_, u64>(15)?,
                row.get::<_, u64>(16)?,
            ))
        })
        .map_err(|error| format!("Query archaeology temporal evidence: {error}"))?;
    for row in rows {
        let row = row.map_err(|error| format!("Read archaeology temporal evidence: {error}"))?;
        if row.8.as_deref() != Some("sha256") || row.9 != row.10 {
            return Err("Temporal evidence parser or hash provenance is invalid".into());
        }
        validate_opaque_path_identity(&row.6)?;
        validate_hex(&row.7, 64, "temporal evidence content hash")?;
        let (rule_id, clause_ordinal) = clause_index
            .get(&row.1)
            .ok_or("Temporal evidence references an unknown clause")?;
        if rule_id != &row.0 {
            return Err("Temporal evidence crosses rule scope".into());
        }
        let clause = rules
            .get_mut(rule_id)
            .and_then(|rule| rule.clauses.get_mut(*clause_ordinal))
            .ok_or("Temporal evidence references an unknown rule")?;
        let key = (row.2.clone(), row.3.clone());
        if !clause.evidence.contains_key(&key)
            && clause.evidence.len() >= limits.max_evidence_per_clause
        {
            return Err("Archaeology temporal evidence bound exceeded".into());
        }
        let evidence = clause
            .evidence
            .entry(key)
            .or_insert_with(|| StoredEvidence {
                role: row.2.clone(),
                fact_identity: row.3.clone(),
                fact_kind: row.4.clone(),
                parser_identity: row.5.clone(),
                spans: Vec::new(),
            });
        if evidence.spans.len() >= limits.max_spans_per_evidence {
            return Err("Archaeology temporal span bound exceeded".into());
        }
        let span = ArchaeologyTemporalSpanPayload {
            path_identity: row.6,
            content_hash: row.7,
            start_byte: row.11,
            end_byte: row.12,
            start_line: row.13,
            start_column: row.14,
            end_line: row.15,
            end_column: row.16,
        };
        if span.end_byte < span.start_byte || !evidence.spans.iter().all(|prior| prior != &span) {
            return Err("Temporal evidence contains an invalid or duplicate span".into());
        }
        evidence.spans.push(span);
    }
    for rule in rules.values() {
        if rule.clauses.iter().any(|clause| {
            clause.evidence.is_empty() || clause.evidence.values().any(|item| item.spans.is_empty())
        }) {
            return Err("Temporal clause has no exact bounded evidence".into());
        }
    }
    Ok(())
}

fn projection_coverage(
    history: &ArchaeologyTemporalCoverageInput,
    current: &Generation,
    prior: Option<&Generation>,
    prior_generation_id: Option<&str>,
    prior_temporal_identity: Option<&str>,
) -> Result<(ArchaeologyTemporalCoverageState, Vec<String>), String> {
    let mut state = history.state;
    let mut reasons = history.reasons.clone();
    apply_generation_coverage(&mut state, &mut reasons, "current", &current.coverage);
    match (prior_generation_id, prior) {
        (None, None) => {
            weaken(&mut state, ArchaeologyTemporalCoverageState::Partial);
            push_reason(&mut reasons, "missing_prior_generation");
        }
        (Some(_), Some(prior)) => {
            apply_generation_coverage(&mut state, &mut reasons, "prior", &prior.coverage);
            if prior_temporal_identity.is_none() {
                weaken(&mut state, ArchaeologyTemporalCoverageState::Partial);
                push_reason(&mut reasons, "missing_prior_temporal_generation");
            }
            if prior.parser_manifest != current.parser_manifest {
                weaken(&mut state, ArchaeologyTemporalCoverageState::Partial);
                push_reason(&mut reasons, "parser_manifest_incompatible");
            }
        }
        (Some(_), None) => {
            weaken(&mut state, ArchaeologyTemporalCoverageState::Partial);
            push_reason(&mut reasons, "missing_prior_catalog");
            if prior_temporal_identity.is_none() {
                push_reason(&mut reasons, "missing_prior_temporal_generation");
            }
        }
        (None, Some(_)) => return Err("Temporal prior generation state is inconsistent".into()),
    }
    if state != ArchaeologyTemporalCoverageState::Complete && reasons.is_empty() {
        return Err("Partial temporal coverage requires a reason".into());
    }
    reasons.sort();
    reasons.dedup();
    validate_reasons(&reasons)?;
    Ok((state, reasons))
}

fn apply_generation_coverage(
    state: &mut ArchaeologyTemporalCoverageState,
    reasons: &mut Vec<String>,
    prefix: &str,
    coverage: &ArchaeologyCoverage,
) {
    for (name, value) in [
        ("catalog", &coverage.state),
        ("parser", &coverage.parser_coverage),
        ("repository", &coverage.repository_coverage),
    ] {
        match value {
            ArchaeologyCoverageState::Complete => {}
            ArchaeologyCoverageState::Partial => {
                weaken(state, ArchaeologyTemporalCoverageState::Partial);
                push_reason(reasons, &format!("{prefix}_{name}_coverage_partial"));
            }
            ArchaeologyCoverageState::Unavailable => {
                weaken(state, ArchaeologyTemporalCoverageState::Unavailable);
                push_reason(reasons, &format!("{prefix}_{name}_coverage_unavailable"));
            }
        }
    }
}

fn weaken(
    current: &mut ArchaeologyTemporalCoverageState,
    candidate: ArchaeologyTemporalCoverageState,
) {
    let rank = |state| match state {
        ArchaeologyTemporalCoverageState::Complete => 0,
        ArchaeologyTemporalCoverageState::Partial => 1,
        ArchaeologyTemporalCoverageState::Unavailable => 2,
    };
    if rank(candidate) > rank(*current) {
        *current = candidate;
    }
}

const SNAPSHOT_WRITE_BATCH: usize = 64;

/// Persist a bounded batch of content-addressed snapshots with the same exact
/// collision verification as the single-row path. A history comparison often
/// revisits hundreds of unchanged snapshots; set-wise reconciliation avoids a
/// write followed by a separate read round trip for every one of those rows.
fn persist_snapshots<'a>(
    transaction: &Transaction<'_>,
    snapshots: impl IntoIterator<Item = &'a Snapshot>,
    created_at: &str,
) -> Result<(), String> {
    let snapshots = snapshots.into_iter().collect::<Vec<_>>();
    for batch in snapshots.chunks(SNAPSHOT_WRITE_BATCH) {
        let rows = batch
            .iter()
            .map(|snapshot| {
                serde_json::json!({
                    "identity": snapshot.identity,
                    "repository_id": repository_id(snapshot),
                    "stable_rule_identity": snapshot.rule.stable_rule_identity,
                    "continuity_identity": snapshot.rule.continuity_identity,
                    "rule_kind": snapshot.rule.kind,
                    "evidence_identity": snapshot.rule.evidence_identity,
                    "parser_compatibility_identity": snapshot.rule.parser_compatibility_identity,
                    "contradiction_identity": snapshot.rule.contradiction_identity,
                    "description_identity": snapshot.rule.description_identity,
                    "payload_json": snapshot.payload_json,
                })
            })
            .collect::<Vec<_>>();
        let rows_json = serde_json::to_string(&rows)
            .map_err(|error| format!("Encode archaeology temporal snapshot batch: {error}"))?;
        transaction
            .execute(
                "INSERT OR IGNORE INTO archaeology_rule_temporal_snapshots
                 (snapshot_identity,repository_id,stable_rule_identity,continuity_identity,
                  rule_kind,evidence_identity,parser_compatibility_identity,
                  contradiction_identity,description_identity,payload_json,created_at)
                 SELECT json_extract(value,'$.identity'),
                        json_extract(value,'$.repository_id'),
                        json_extract(value,'$.stable_rule_identity'),
                        json_extract(value,'$.continuity_identity'),
                        json_extract(value,'$.rule_kind'),
                        json_extract(value,'$.evidence_identity'),
                        json_extract(value,'$.parser_compatibility_identity'),
                        json_extract(value,'$.contradiction_identity'),
                        json_extract(value,'$.description_identity'),
                        json_extract(value,'$.payload_json'),?2
                   FROM json_each(?1)",
                params![rows_json, created_at],
            )
            .map_err(|error| format!("Persist archaeology temporal snapshots: {error}"))?;
        let exact: usize = transaction
            .query_row(
                "SELECT COUNT(*) FROM json_each(?1) AS input
                 JOIN archaeology_rule_temporal_snapshots AS snapshot
                   ON snapshot.snapshot_identity=json_extract(input.value,'$.identity')
                 WHERE snapshot.repository_id=json_extract(input.value,'$.repository_id')
                   AND snapshot.stable_rule_identity=json_extract(input.value,'$.stable_rule_identity')
                   AND snapshot.continuity_identity=json_extract(input.value,'$.continuity_identity')
                   AND snapshot.rule_kind=json_extract(input.value,'$.rule_kind')
                   AND snapshot.evidence_identity=json_extract(input.value,'$.evidence_identity')
                   AND snapshot.parser_compatibility_identity=json_extract(input.value,'$.parser_compatibility_identity')
                   AND snapshot.contradiction_identity=json_extract(input.value,'$.contradiction_identity')
                   AND snapshot.description_identity=json_extract(input.value,'$.description_identity')
                   AND snapshot.payload_json=json_extract(input.value,'$.payload_json')",
                [rows_json],
                |row| row.get(0),
            )
            .map_err(|error| format!("Verify archaeology temporal snapshot retry: {error}"))?;
        if exact != batch.len() {
            return Err("Archaeology temporal snapshot identity collision".into());
        }
    }
    Ok(())
}

fn repository_id(snapshot: &Snapshot) -> &str {
    &snapshot.rule.repository_id
}

fn persist_temporal_generation(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation: &Generation,
    prior_identity: Option<&str>,
    identity: &str,
    catalog_identity: &str,
    rule_count: usize,
    coverage_state: ArchaeologyTemporalCoverageState,
    coverage_json: &str,
    created_at: &str,
) -> Result<(), String> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO archaeology_temporal_generations
             (temporal_generation_identity,repository_id,generation_id,revision_sha,
              prior_temporal_generation_identity,source_schema_version,catalog_identity,
              rule_count,coverage_state,coverage_reasons_json,created_at)
             VALUES (?1,?2,?3,?4,?5,2,?6,?7,?8,?9,?10)",
            params![
                identity,
                repository_id,
                generation.id,
                generation.revision,
                prior_identity,
                catalog_identity,
                rule_count,
                coverage_name(coverage_state),
                coverage_json,
                created_at,
            ],
        )
        .map_err(|error| format!("Persist archaeology temporal generation: {error}"))?;
    let exact = transaction
        .query_row(
            "SELECT temporal_generation_identity=?3 AND revision_sha=?4
                    AND prior_temporal_generation_identity IS ?5 AND catalog_identity=?6
                    AND rule_count=?7 AND coverage_state=?8 AND coverage_reasons_json=?9
             FROM archaeology_temporal_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![
                repository_id,
                generation.id,
                identity,
                generation.revision,
                prior_identity,
                catalog_identity,
                rule_count,
                coverage_name(coverage_state),
                coverage_json,
            ],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|error| format!("Verify archaeology temporal generation retry: {error}"))?
        .unwrap_or(false);
    if exact {
        Ok(())
    } else {
        Err("Archaeology temporal generation retry does not reconcile".into())
    }
}

fn persist_event(
    transaction: &Transaction<'_>,
    repository_id: &str,
    temporal_generation_identity: &str,
    prior_temporal_generation_identity: Option<&str>,
    event: &TemporalEvent<'_>,
    created_at: &str,
) -> Result<(), String> {
    let reasons_json = encode_event_reasons(event.coverage_reasons)?;
    let identity = digest_fields(
        "archaeology-rule-temporal-event:v1",
        &[
            repository_id,
            temporal_generation_identity,
            event.kind.as_str(),
            event.stable_rule_identity,
            event.continuity_identity,
            event.before.map_or("none", |value| &value.identity),
            event.after.map_or("none", |value| &value.identity),
            event.continuity_edge_identity.unwrap_or("none"),
            coverage_name(event.coverage_state),
            &reasons_json,
        ],
    );
    transaction
        .execute(
            "INSERT OR IGNORE INTO archaeology_rule_temporal_events
             (event_identity,repository_id,temporal_generation_identity,
              prior_temporal_generation_identity,event_kind,stable_rule_identity,
              continuity_identity,predecessor_rule_identity,successor_rule_identity,
              before_snapshot_identity,after_snapshot_identity,continuity_edge_identity,
              coverage_state,coverage_reasons_json,created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                identity,
                repository_id,
                temporal_generation_identity,
                prior_temporal_generation_identity,
                event.kind.as_str(),
                event.stable_rule_identity,
                event.continuity_identity,
                event.predecessor_rule_identity,
                event.successor_rule_identity,
                event.before.map(|value| value.identity.as_str()),
                event.after.map(|value| value.identity.as_str()),
                event.continuity_edge_identity,
                coverage_name(event.coverage_state),
                reasons_json,
                created_at,
            ],
        )
        .map_err(|error| format!("Persist archaeology temporal event: {error}"))?;
    let exact = transaction
        .query_row(
            "SELECT repository_id=?2 AND temporal_generation_identity=?3
                    AND prior_temporal_generation_identity IS ?4 AND event_kind=?5
                    AND stable_rule_identity=?6 AND continuity_identity=?7
                    AND predecessor_rule_identity IS ?8 AND successor_rule_identity IS ?9
                    AND before_snapshot_identity IS ?10 AND after_snapshot_identity IS ?11
                    AND continuity_edge_identity IS ?12 AND coverage_state=?13
                    AND coverage_reasons_json=?14
             FROM archaeology_rule_temporal_events WHERE event_identity=?1",
            params![
                identity,
                repository_id,
                temporal_generation_identity,
                prior_temporal_generation_identity,
                event.kind.as_str(),
                event.stable_rule_identity,
                event.continuity_identity,
                event.predecessor_rule_identity,
                event.successor_rule_identity,
                event.before.map(|value| value.identity.as_str()),
                event.after.map(|value| value.identity.as_str()),
                event.continuity_edge_identity,
                coverage_name(event.coverage_state),
                reasons_json,
            ],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|error| format!("Verify archaeology temporal event retry: {error}"))?
        .unwrap_or(false);
    if exact {
        Ok(())
    } else {
        Err("Archaeology temporal event retry does not reconcile".into())
    }
}

fn load_continuity_edges(
    transaction: &Transaction<'_>,
    repository_id: &str,
    predecessor_generation_id: &str,
    successor_generation_id: &str,
) -> Result<Vec<ContinuityEdge>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT edge_identity,continuity_identity,predecessor_rule_identity,
                    successor_rule_identity,evidence_identity
             FROM archaeology_rule_continuity_edges
             WHERE repository_id=?1 AND predecessor_generation_id=?2
               AND successor_generation_id=?3 AND kind='supersedes'
             ORDER BY predecessor_rule_identity,successor_rule_identity,edge_identity",
        )
        .map_err(|error| format!("Prepare archaeology temporal continuity: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repository_id,
                predecessor_generation_id,
                successor_generation_id
            ],
            |row| {
                Ok(ContinuityEdge {
                    identity: row.get(0)?,
                    continuity_identity: row.get(1)?,
                    predecessor: row.get(2)?,
                    successor: row.get(3)?,
                    evidence_identity: row.get(4)?,
                })
            },
        )
        .map_err(|error| format!("Query archaeology temporal continuity: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read archaeology temporal continuity: {error}"))
}

fn load_temporal_generation_identity(
    transaction: &Transaction<'_>,
    repository_id: &str,
    generation_id: &str,
) -> Result<Option<String>, String> {
    transaction
        .query_row(
            "SELECT temporal_generation_identity FROM archaeology_temporal_generations
             WHERE repository_id=?1 AND generation_id=?2",
            params![repository_id, generation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Load prior archaeology temporal generation: {error}"))
}

fn referenced_snapshot_count(
    transaction: &Transaction<'_>,
    repository_id: &str,
) -> Result<usize, String> {
    transaction
        .query_row(
            "SELECT COUNT(*) FROM archaeology_rule_temporal_snapshots WHERE repository_id=?1",
            [repository_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Count archaeology temporal snapshots: {error}"))
}

fn catalog_identity(repository_id: &str, rules: &BTreeMap<String, Snapshot>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"archaeology-temporal-catalog:v1\0");
    digest_field(&mut digest, repository_id);
    for (stable, snapshot) in rules {
        digest_field(&mut digest, stable);
        digest_field(&mut digest, &snapshot.identity);
    }
    format!(
        "{DIGEST_PREFIX}{}",
        super::inventory::hex(&digest.finalize())
    )
}

fn digest_fields(tag: &str, fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(tag.as_bytes());
    digest.update([0]);
    for field in fields {
        digest_field(&mut digest, field);
    }
    format!(
        "{DIGEST_PREFIX}{}",
        super::inventory::hex(&digest.finalize())
    )
}

fn digest_field(digest: &mut Sha256, field: &str) {
    digest.update((field.len() as u64).to_be_bytes());
    digest.update(field.as_bytes());
}

fn coverage_name(state: ArchaeologyTemporalCoverageState) -> &'static str {
    match state {
        ArchaeologyTemporalCoverageState::Complete => "complete",
        ArchaeologyTemporalCoverageState::Partial => "partial",
        ArchaeologyTemporalCoverageState::Unavailable => "unavailable",
    }
}

fn validate_coverage_input(input: &ArchaeologyTemporalCoverageInput) -> Result<(), String> {
    validate_reasons(&input.reasons)?;
    if input.state == ArchaeologyTemporalCoverageState::Complete && !input.reasons.is_empty() {
        return Err("Complete temporal coverage cannot retain gap reasons".into());
    }
    if input.state != ArchaeologyTemporalCoverageState::Complete && input.reasons.is_empty() {
        return Err("Partial temporal coverage requires a reason".into());
    }
    Ok(())
}

fn validate_reasons(reasons: &[String]) -> Result<(), String> {
    if reasons.len() > MAX_REASON_COUNT {
        return Err("Temporal coverage reason bound exceeded".into());
    }
    for reason in reasons {
        validate_token("temporal coverage reason", reason, MAX_REASON_BYTES)?;
    }
    Ok(())
}

fn encode_reasons(reasons: &[String]) -> Result<String, String> {
    validate_reasons(reasons)?;
    serde_json::to_string(reasons).map_err(|error| format!("Encode temporal coverage: {error}"))
}

fn encode_event_reasons(reasons: EventCoverageReasons<'_>) -> Result<String, String> {
    validate_reasons(reasons.common)?;
    if let Some(extra) = reasons.extra {
        validate_token("temporal coverage reason", extra, MAX_REASON_BYTES)?;
    }
    if reasons.common.len() + usize::from(reasons.extra.is_some()) > MAX_REASON_COUNT {
        return Err("Temporal coverage reason bound exceeded".into());
    }
    let values = reasons
        .common
        .iter()
        .map(String::as_str)
        .chain(reasons.extra)
        .collect::<Vec<_>>();
    serde_json::to_string(&values).map_err(|error| format!("Encode temporal coverage: {error}"))
}

fn push_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

fn validate_digest(value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix(DIGEST_PREFIX) else {
        return Err("Temporal identity must use sha256".into());
    };
    validate_hex(hex, 64, "temporal identity")
}

fn validate_revision(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Temporal revision must be an exact lowercase Git SHA".into());
    }
    Ok(())
}

fn validate_hex(value: &str, length: usize, label: &str) -> Result<(), String> {
    if value.len() != length
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Err(format!("{label} is invalid"))
    } else {
        Ok(())
    }
}

fn validate_opaque_path_identity(value: &str) -> Result<(), String> {
    validate_token("temporal path identity", value, 256)?;
    if value.contains('/') || value.contains('\\') || value == "." || value == ".." {
        return Err("Temporal evidence requires an opaque path identity".into());
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    validate_token("temporal timestamp", value, MAX_TIMESTAMP_BYTES)
}

fn validate_token(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_bytes
        || value != value.trim()
        || value
            .bytes()
            .any(|byte| matches!(byte, 0 | b'\n' | b'\r' | b'\t'))
    {
        Err(format!("Archaeology {label} is invalid"))
    } else {
        Ok(())
    }
}
