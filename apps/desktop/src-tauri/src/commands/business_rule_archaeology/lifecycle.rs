use super::contracts::ArchaeologyRuleLifecycle;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_LIFECYCLE_EVENTS_PER_RULE: usize = 10_000;
pub(crate) const MAX_LIFECYCLE_ID_BYTES: usize = 256;
pub(crate) const MAX_LIFECYCLE_REASON_BYTES: usize = 1_024;
pub(crate) const MAX_LIFECYCLE_ANNOTATION_BYTES: usize = 4_096;
pub(crate) const MAX_ALIASES_PER_REPOSITORY: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyReviewerKind {
    Human,
    DeterministicPolicy,
    Model,
}

/// Local provenance for a lifecycle action.
///
/// `authority_id` is the configured policy identity for deterministic actions
/// and the provider/model identity for model-authored notes. Human identities
/// are already carried by `actor_id` and therefore have no second authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyReviewerProvenance {
    pub kind: ArchaeologyReviewerKind,
    pub actor_id: String,
    pub authority_id: Option<String>,
}

impl ArchaeologyReviewerProvenance {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_id("reviewer actor", &self.actor_id)?;
        match self.kind {
            ArchaeologyReviewerKind::Human => {
                if self.authority_id.is_some() {
                    return Err("Human reviewer provenance cannot name a policy authority".into());
                }
            }
            ArchaeologyReviewerKind::DeterministicPolicy | ArchaeologyReviewerKind::Model => {
                validate_id(
                    "reviewer authority",
                    self.authority_id.as_deref().unwrap_or_default(),
                )?;
            }
        }
        Ok(())
    }

    fn can_decide(&self) -> bool {
        matches!(
            self.kind,
            ArchaeologyReviewerKind::Human | ArchaeologyReviewerKind::DeterministicPolicy
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ArchaeologyLifecycleAction {
    Candidate,
    ReviewNeeded { reason: String },
    Accept,
    Reject { reason: String },
    Conflict { reason: String },
    Supersede { successor_rule_id: String },
    Annotate { annotation: String },
}

impl ArchaeologyLifecycleAction {
    fn validate(
        &self,
        rule_id: &str,
        provenance: &ArchaeologyReviewerProvenance,
    ) -> Result<(), String> {
        match self {
            Self::Candidate | Self::Accept => {}
            Self::ReviewNeeded { reason } | Self::Reject { reason } | Self::Conflict { reason } => {
                validate_text("lifecycle reason", reason, MAX_LIFECYCLE_REASON_BYTES)?
            }
            Self::Supersede { successor_rule_id } => {
                validate_id("successor rule", successor_rule_id)?;
                if successor_rule_id == rule_id {
                    return Err("A rule cannot supersede itself".into());
                }
            }
            Self::Annotate { annotation } => validate_text(
                "lifecycle annotation",
                annotation,
                MAX_LIFECYCLE_ANNOTATION_BYTES,
            )?,
        }
        if matches!(self, Self::Accept | Self::Reject { .. }) && !provenance.can_decide() {
            return Err("Only a human or deterministic policy may accept or reject a rule".into());
        }
        if matches!(self, Self::Supersede { .. })
            && matches!(provenance.kind, ArchaeologyReviewerKind::Model)
        {
            return Err("A model cannot supersede a rule".into());
        }
        Ok(())
    }
}

/// One immutable event. `expected_previous_sequence` is the compare-and-swap
/// value supplied by the writer; it must equal `sequence - 1`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyLifecycleEvent {
    pub event_id: String,
    pub repository_id: String,
    pub rule_id: String,
    pub sequence: u64,
    pub expected_previous_sequence: u64,
    pub provenance: ArchaeologyReviewerProvenance,
    pub action: ArchaeologyLifecycleAction,
}

impl ArchaeologyLifecycleEvent {
    fn validate_shape(&self) -> Result<(), String> {
        validate_id("lifecycle event", &self.event_id)?;
        validate_id("repository", &self.repository_id)?;
        validate_id("rule", &self.rule_id)?;
        if self.sequence == 0 {
            return Err("Lifecycle event sequence is one-based".into());
        }
        if self.expected_previous_sequence != self.sequence - 1 {
            return Err("Lifecycle event compare-and-swap sequence is inconsistent".into());
        }
        self.provenance.validate()?;
        self.action.validate(&self.rule_id, &self.provenance)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyProjectedAnnotation {
    pub event_id: String,
    pub sequence: u64,
    pub annotation: String,
    pub provenance: ArchaeologyReviewerProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyLifecycleProjection {
    pub repository_id: String,
    pub rule_id: String,
    pub lifecycle: ArchaeologyRuleLifecycle,
    pub last_sequence: u64,
    pub last_state_event_id: String,
    pub decision_provenance: Option<ArchaeologyReviewerProvenance>,
    pub successor_rule_id: Option<String>,
    pub annotations: Vec<ArchaeologyProjectedAnnotation>,
}

/// Projects a complete append-only stream. Callers may supply rows in any
/// order; sequence is authoritative and gaps, duplicates, stale CAS values,
/// cross-rule rows, and illegal transitions fail closed.
pub(crate) fn project_lifecycle(
    events: &[ArchaeologyLifecycleEvent],
) -> Result<ArchaeologyLifecycleProjection, String> {
    if events.is_empty() {
        return Err("A lifecycle stream requires an initial candidate event".into());
    }
    if events.len() > MAX_LIFECYCLE_EVENTS_PER_RULE {
        return Err("Lifecycle event bound exceeded".into());
    }

    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });

    let repository_id = ordered[0].repository_id.clone();
    let rule_id = ordered[0].rule_id.clone();
    let mut event_ids = BTreeSet::new();
    let mut lifecycle = ArchaeologyRuleLifecycle::Unavailable;
    let mut last_state_event_id = String::new();
    let mut decision_provenance = None;
    let mut successor_rule_id = None;
    let mut annotations = Vec::new();

    for (offset, event) in ordered.into_iter().enumerate() {
        event.validate_shape()?;
        if event.repository_id != repository_id || event.rule_id != rule_id {
            return Err("Lifecycle stream crosses repository or rule scope".into());
        }
        let expected_sequence = u64::try_from(offset)
            .map_err(|_| "Lifecycle event sequence exceeds supported range")?
            + 1;
        if event.sequence != expected_sequence {
            return Err("Lifecycle event sequence is duplicated or has a gap".into());
        }
        if !event_ids.insert(event.event_id.as_str()) {
            return Err("Lifecycle event identity is duplicated".into());
        }

        match &event.action {
            ArchaeologyLifecycleAction::Annotate { annotation } => {
                if lifecycle == ArchaeologyRuleLifecycle::Unavailable {
                    return Err("A lifecycle annotation cannot precede the candidate event".into());
                }
                annotations.push(ArchaeologyProjectedAnnotation {
                    event_id: event.event_id.clone(),
                    sequence: event.sequence,
                    annotation: annotation.clone(),
                    provenance: event.provenance.clone(),
                });
            }
            action => {
                lifecycle = transition(&lifecycle, action)?;
                last_state_event_id.clone_from(&event.event_id);
                decision_provenance = matches!(
                    action,
                    ArchaeologyLifecycleAction::Accept | ArchaeologyLifecycleAction::Reject { .. }
                )
                .then(|| event.provenance.clone());
                successor_rule_id = match action {
                    ArchaeologyLifecycleAction::Supersede { successor_rule_id } => {
                        Some(successor_rule_id.clone())
                    }
                    _ => None,
                };
            }
        }
    }

    Ok(ArchaeologyLifecycleProjection {
        repository_id,
        rule_id,
        lifecycle,
        last_sequence: events.len() as u64,
        last_state_event_id,
        decision_provenance,
        successor_rule_id,
        annotations,
    })
}

/// Validates one append without mutating the existing stream. This is the pure
/// CAS gate used before an eventual transactional insert.
pub(crate) fn validate_lifecycle_append(
    existing: &[ArchaeologyLifecycleEvent],
    candidate: &ArchaeologyLifecycleEvent,
) -> Result<ArchaeologyLifecycleProjection, String> {
    if existing.len() >= MAX_LIFECYCLE_EVENTS_PER_RULE {
        return Err("Lifecycle event bound exceeded".into());
    }
    candidate.validate_shape()?;
    let expected_previous = if existing.is_empty() {
        0
    } else {
        project_lifecycle(existing)?.last_sequence
    };
    if candidate.expected_previous_sequence != expected_previous
        || candidate.sequence != expected_previous + 1
    {
        return Err("Lifecycle append compare-and-swap failed".into());
    }
    if let Some(first) = existing.first() {
        if candidate.repository_id != first.repository_id || candidate.rule_id != first.rule_id {
            return Err("Lifecycle append crosses repository or rule scope".into());
        }
    }
    let mut projected = Vec::with_capacity(existing.len() + 1);
    projected.extend_from_slice(existing);
    projected.push(candidate.clone());
    project_lifecycle(&projected)
}

fn transition(
    current: &ArchaeologyRuleLifecycle,
    action: &ArchaeologyLifecycleAction,
) -> Result<ArchaeologyRuleLifecycle, String> {
    use ArchaeologyLifecycleAction as Action;
    use ArchaeologyRuleLifecycle as State;

    let next = match (current, action) {
        (State::Unavailable, Action::Candidate) => State::Candidate,
        (State::Unavailable, _) => {
            return Err("The first lifecycle event must create a candidate".into())
        }
        (_, Action::Candidate) => return Err("A candidate event may only start a lifecycle".into()),
        (State::Superseded, _) => {
            return Err("A superseded rule cannot receive another state transition".into())
        }
        (State::ReviewNeeded, Action::ReviewNeeded { .. })
        | (State::Accepted, Action::Accept)
        | (State::Rejected, Action::Reject { .. })
        | (State::Conflicted, Action::Conflict { .. }) => {
            return Err("Lifecycle transition would not change state".into())
        }
        (_, Action::ReviewNeeded { .. }) => State::ReviewNeeded,
        (_, Action::Accept) => State::Accepted,
        (_, Action::Reject { .. }) => State::Rejected,
        (_, Action::Conflict { .. }) => State::Conflicted,
        (_, Action::Supersede { .. }) => State::Superseded,
        (_, Action::Annotate { .. }) => unreachable!("annotations are projected separately"),
    };
    Ok(next)
}

/// Identities needed to decide whether a prior review remains compatible.
/// These are hashes/opaque IDs; prose and source bodies do not cross this API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchaeologyRuleSnapshotIdentity {
    pub repository_id: String,
    pub rule_id: String,
    pub rule_kind_identity: String,
    pub continuity_identity: String,
    pub evidence_identity: String,
    pub parser_compatibility_identity: String,
    pub contradiction_identity: String,
    pub description_identity: String,
}

impl ArchaeologyRuleSnapshotIdentity {
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("repository", self.repository_id.as_str()),
            ("rule", self.rule_id.as_str()),
            ("rule kind", self.rule_kind_identity.as_str()),
            ("rule continuity", self.continuity_identity.as_str()),
            ("rule evidence", self.evidence_identity.as_str()),
            (
                "parser compatibility",
                self.parser_compatibility_identity.as_str(),
            ),
            ("rule contradiction", self.contradiction_identity.as_str()),
            ("rule description", self.description_identity.as_str()),
        ] {
            validate_id(label, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ArchaeologyCompatibilityMismatch {
    Evidence,
    Parser,
    Contradiction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArchaeologyCompatibilityOutcome {
    Compatible {
        lifecycle: ArchaeologyRuleLifecycle,
        description_changed: bool,
    },
    ReviewNeeded {
        reasons: Vec<ArchaeologyCompatibilityMismatch>,
    },
    Conflicted {
        reasons: Vec<ArchaeologyCompatibilityMismatch>,
    },
    Superseded {
        predecessor_rule_id: String,
        successor_rule_id: String,
        predecessor_lifecycle: ArchaeologyRuleLifecycle,
        successor_lifecycle: ArchaeologyRuleLifecycle,
    },
}

/// Compares two persisted rule snapshots without fuzzy matching. A changed
/// rule ID requires an explicit successor link; reused IDs must keep kind and
/// continuity identities. Contradiction drift invalidates accepted evidence
/// more strongly than ordinary parser/evidence drift.
pub(crate) fn evaluate_snapshot_compatibility(
    previous: &ArchaeologyRuleSnapshotIdentity,
    current: &ArchaeologyRuleSnapshotIdentity,
    previous_lifecycle: ArchaeologyRuleLifecycle,
    explicit_successor_rule_id: Option<&str>,
) -> Result<ArchaeologyCompatibilityOutcome, String> {
    previous.validate()?;
    current.validate()?;
    if previous.repository_id != current.repository_id {
        return Err("Rule compatibility cannot cross repository scope".into());
    }
    if previous.rule_kind_identity != current.rule_kind_identity {
        return Err("Rule compatibility kind changed; continuity is ambiguous".into());
    }

    if let Some(successor_rule_id) = explicit_successor_rule_id {
        validate_id("explicit successor rule", successor_rule_id)?;
        if previous.rule_id == current.rule_id || successor_rule_id != current.rule_id {
            return Err("Explicit successor must name a distinct current rule".into());
        }
        return Ok(ArchaeologyCompatibilityOutcome::Superseded {
            predecessor_rule_id: previous.rule_id.clone(),
            successor_rule_id: current.rule_id.clone(),
            predecessor_lifecycle: ArchaeologyRuleLifecycle::Superseded,
            successor_lifecycle: ArchaeologyRuleLifecycle::ReviewNeeded,
        });
    }
    if previous.continuity_identity != current.continuity_identity {
        return Err("Rule compatibility identity changed; continuity is ambiguous".into());
    }
    if previous.rule_id != current.rule_id {
        return Err("Changed rule identity requires an explicit successor link".into());
    }

    let mut mismatches = BTreeSet::new();
    if previous.evidence_identity != current.evidence_identity {
        mismatches.insert(ArchaeologyCompatibilityMismatch::Evidence);
    }
    if previous.parser_compatibility_identity != current.parser_compatibility_identity {
        mismatches.insert(ArchaeologyCompatibilityMismatch::Parser);
    }
    if previous.contradiction_identity != current.contradiction_identity {
        mismatches.insert(ArchaeologyCompatibilityMismatch::Contradiction);
    }
    let reasons = mismatches.into_iter().collect::<Vec<_>>();
    if reasons.is_empty() {
        return Ok(ArchaeologyCompatibilityOutcome::Compatible {
            lifecycle: previous_lifecycle,
            description_changed: previous.description_identity != current.description_identity,
        });
    }
    if previous_lifecycle == ArchaeologyRuleLifecycle::Accepted
        && reasons.contains(&ArchaeologyCompatibilityMismatch::Contradiction)
    {
        Ok(ArchaeologyCompatibilityOutcome::Conflicted { reasons })
    } else {
        Ok(ArchaeologyCompatibilityOutcome::ReviewNeeded { reasons })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyRuleAlias {
    pub event_id: String,
    pub alias_repository_id: String,
    pub alias_rule_id: String,
    pub canonical_repository_id: String,
    pub canonical_rule_id: String,
    pub provenance: ArchaeologyReviewerProvenance,
}

impl ArchaeologyRuleAlias {
    fn validate_shape(&self) -> Result<(), String> {
        for (label, value) in [
            ("alias event", self.event_id.as_str()),
            ("alias repository", self.alias_repository_id.as_str()),
            ("alias rule", self.alias_rule_id.as_str()),
            (
                "canonical repository",
                self.canonical_repository_id.as_str(),
            ),
            ("canonical rule", self.canonical_rule_id.as_str()),
        ] {
            validate_id(label, value)?;
        }
        self.provenance.validate()?;
        if matches!(self.provenance.kind, ArchaeologyReviewerKind::Model) {
            return Err("A model cannot create a rule alias".into());
        }
        if self.alias_repository_id != self.canonical_repository_id {
            return Err("A rule alias cannot cross repository scope".into());
        }
        if self.alias_rule_id == self.canonical_rule_id {
            return Err("A rule cannot alias itself".into());
        }
        Ok(())
    }
}

/// Validates a complete alias set. Canonical targets are stars: they may have
/// many direct aliases but can never themselves be aliases. This rejects
/// alias-to-alias chains and therefore all cycles, while retaining an explicit
/// cycle check as a fail-closed invariant for imported rows.
pub(crate) fn validate_rule_aliases(aliases: &[ArchaeologyRuleAlias]) -> Result<(), String> {
    if aliases.len() > MAX_ALIASES_PER_REPOSITORY {
        return Err("Rule alias bound exceeded".into());
    }
    let mut event_ids = BTreeSet::new();
    let mut targets = BTreeMap::<(&str, &str), (&str, &str)>::new();
    for alias in aliases {
        alias.validate_shape()?;
        if !event_ids.insert(alias.event_id.as_str()) {
            return Err("Rule alias event identity is duplicated".into());
        }
        let key = (
            alias.alias_repository_id.as_str(),
            alias.alias_rule_id.as_str(),
        );
        let value = (
            alias.canonical_repository_id.as_str(),
            alias.canonical_rule_id.as_str(),
        );
        if targets.insert(key, value).is_some() {
            return Err("A rule may have only one canonical alias target".into());
        }
    }
    for alias in targets.keys() {
        let mut cursor = *alias;
        let mut visited = BTreeSet::new();
        while let Some(next) = targets.get(&cursor).copied() {
            if !visited.insert(cursor) || visited.contains(&next) {
                return Err("Rule alias cycle detected".into());
            }
            cursor = next;
        }
    }
    if targets.values().any(|target| targets.contains_key(target)) {
        return Err("A canonical rule cannot itself be an alias".into());
    }
    Ok(())
}

pub(crate) fn validate_rule_alias_append(
    existing: &[ArchaeologyRuleAlias],
    candidate: &ArchaeologyRuleAlias,
) -> Result<(), String> {
    if existing.len() >= MAX_ALIASES_PER_REPOSITORY {
        return Err("Rule alias bound exceeded".into());
    }
    let mut aliases = Vec::with_capacity(existing.len() + 1);
    aliases.extend_from_slice(existing);
    aliases.push(candidate.clone());
    validate_rule_aliases(&aliases)
}

fn validate_id(label: &str, value: &str) -> Result<(), String> {
    validate_text(label, value, MAX_LIFECYCLE_ID_BYTES)
}

fn validate_text(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} is required"));
    }
    if value.len() > max_bytes {
        return Err(format!("{label} exceeds its byte bound"));
    }
    if value.chars().any(|character| {
        character == '\0'
            || character.is_control() && character != '\n' && character != '\r' && character != '\t'
    }) {
        return Err(format!("{label} contains unsupported control characters"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
