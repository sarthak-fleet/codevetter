//! Strict, model-agnostic wire contract for optional rule wording synthesis.
//!
//! Provider selection, prompts, cost, caching, retries, and timeouts belong to
//! the next layer. This module only allows one bounded cited packet in and
//! evidence-referencing structured clause segments out.

use super::contracts::{
    validate_revision_sha, ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyEvidencePacket,
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
    ArchaeologyTrust, ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION,
};
use super::deterministic_rules::{expected_packet_id, packet_metadata_is_categorical};
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID: &str =
    "codevetter.business-rule-archaeology.synthesis.v1";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologySynthesisLimits {
    pub max_facts: usize,
    pub max_relationships: usize,
    pub max_evidence_spans: usize,
    pub max_packet_caveats: usize,
    pub max_unresolved_reasons: usize,
    pub max_clauses: usize,
    pub max_fact_ids_per_segment: usize,
    pub max_relationship_ids_per_clause: usize,
    pub max_text_bytes: usize,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

impl Default for ArchaeologySynthesisLimits {
    fn default() -> Self {
        Self {
            max_facts: 64,
            max_relationships: 128,
            max_evidence_spans: 256,
            max_packet_caveats: 16,
            max_unresolved_reasons: 64,
            max_clauses: 256,
            max_fact_ids_per_segment: 64,
            max_relationship_ids_per_clause: 128,
            max_text_bytes: 1_024,
            max_request_bytes: 256 * 1024,
            max_response_bytes: 256 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisFact {
    pub fact_id: String,
    pub kind: ArchaeologyFactKind,
    pub label: String,
    pub trust: ArchaeologyTrust,
    pub confidence: ArchaeologyConfidence,
    #[serde(default)]
    pub quantifier_kinds: Vec<ArchaeologySynthesisQuantifierKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisRelationship {
    pub relationship_id: String,
    pub from_fact_id: String,
    pub to_fact_id: String,
    pub kind: ArchaeologyFactEdgeKind,
    pub trust: ArchaeologyTrust,
    pub unresolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisRequest {
    pub schema_version: u32,
    pub contract_id: String,
    pub request_id: String,
    pub repository_id: String,
    pub generation_id: String,
    pub revision_sha: String,
    pub parser_identity: String,
    pub algorithm_identity: String,
    pub packet: ArchaeologyEvidencePacket,
    pub facts: Vec<ArchaeologySynthesisFact>,
    pub relationships: Vec<ArchaeologySynthesisRelationship>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologySynthesisQuantifierKind {
    All,
    Any,
    None,
    ExactlyOne,
    AtLeastOne,
    AtMostOne,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisSegment {
    pub text: String,
    pub fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisQuantifier {
    pub kind: ArchaeologySynthesisQuantifierKind,
    pub fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisClause {
    pub subject: ArchaeologySynthesisSegment,
    pub condition: Option<ArchaeologySynthesisSegment>,
    pub action: ArchaeologySynthesisSegment,
    pub exception: Option<ArchaeologySynthesisSegment>,
    pub quantifier: Option<ArchaeologySynthesisQuantifier>,
    pub relationship_ids: Vec<String>,
    pub contradicting_fact_ids: Vec<String>,
}

impl ArchaeologySynthesisClause {
    /// The exact positive evidence projection shared by response validation and
    /// durable rule materialization. Keeping this in one place prevents a new
    /// clause segment from being validated but omitted from publication.
    pub(crate) fn supporting_fact_ids(&self) -> BTreeSet<&str> {
        self.subject
            .fact_ids
            .iter()
            .chain(&self.action.fact_ids)
            .chain(self.condition.iter().flat_map(|segment| &segment.fact_ids))
            .chain(self.exception.iter().flat_map(|segment| &segment.fact_ids))
            .chain(
                self.quantifier
                    .iter()
                    .flat_map(|quantifier| &quantifier.fact_ids),
            )
            .map(String::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologySynthesisResponse {
    pub schema_version: u32,
    pub contract_id: String,
    pub request_id: String,
    pub packet_id: String,
    pub clauses: Vec<ArchaeologySynthesisClause>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_synthesis_request(
    repository_id: &str,
    generation_id: &str,
    revision_sha: &str,
    parser_identity: &str,
    algorithm_identity: &str,
    packet: &ArchaeologyEvidencePacket,
    facts: &[ArchaeologyFact],
    relationships: &[ArchaeologyFactEdge],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologySynthesisLimits,
) -> Result<ArchaeologySynthesisRequest, String> {
    cancelled(cancellation)?;
    if !safe_scope_id(repository_id)
        || !safe_scope_id(generation_id)
        || !safe_scope_id(parser_identity)
        || !safe_scope_id(algorithm_identity)
        || validate_revision_sha(revision_sha).is_err()
        || facts.len() > limits.max_facts
        || relationships.len() > limits.max_relationships
    {
        return Err("Archaeology synthesis request scope or count bound is invalid".into());
    }
    validate_packet_shape(packet, limits)?;
    if packet.packet_id != expected_packet_id(repository_id, revision_sha, packet) {
        return Err("Archaeology synthesis packet identity does not match its evidence".into());
    }

    let expected_fact_ids = packet
        .supporting_fact_ids
        .iter()
        .chain(&packet.contradicting_fact_ids)
        .chain(&packet.unresolved_fact_ids)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let facts_by_id = unique_map(facts, |fact| fact.fact_id.as_str(), "fact")?;
    if expected_fact_ids != facts_by_id.keys().copied().collect() {
        return Err("Archaeology synthesis request fact set does not reconcile".into());
    }
    let expected_relationship_ids = packet
        .relationship_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let relationships_by_id =
        unique_map(relationships, |edge| edge.edge_id.as_str(), "relationship")?;
    if expected_relationship_ids != relationships_by_id.keys().copied().collect() {
        return Err("Archaeology synthesis request relationship set does not reconcile".into());
    }
    let used_span_ids = facts
        .iter()
        .flat_map(|fact| &fact.span_ids)
        .chain(
            relationships
                .iter()
                .flat_map(|relationship| &relationship.evidence_span_ids),
        )
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if used_span_ids
        != packet
            .evidence_span_ids
            .iter()
            .map(String::as_str)
            .collect()
    {
        return Err("Archaeology synthesis request evidence span set does not reconcile".into());
    }

    let mut request_facts = Vec::with_capacity(facts.len());
    for fact in facts_by_id.values() {
        cancelled(cancellation)?;
        if !safe_text(&fact.label, limits.max_text_bytes)
            || !matches!(
                fact.trust,
                ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
            )
            || fact.span_ids.is_empty()
            || fact
                .span_ids
                .iter()
                .any(|id| !packet.evidence_span_ids.contains(id))
        {
            return Err("Archaeology synthesis request fact is unsafe or unsupported".into());
        }
        request_facts.push(ArchaeologySynthesisFact {
            fact_id: fact.fact_id.clone(),
            kind: fact.kind.clone(),
            label: fact.label.clone(),
            trust: fact.trust.clone(),
            confidence: fact.confidence.clone(),
            quantifier_kinds: quantifier_kinds_from_evidence(&fact.label, &fact.attributes),
        });
    }

    let mut request_relationships = Vec::with_capacity(relationships.len());
    for edge in relationships_by_id.values() {
        cancelled(cancellation)?;
        if !expected_fact_ids.contains(edge.from_fact_id.as_str())
            || !expected_fact_ids.contains(edge.to_fact_id.as_str())
            || !matches!(
                edge.trust,
                ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
            )
            || edge.evidence_span_ids.is_empty()
            || edge
                .evidence_span_ids
                .iter()
                .any(|id| !packet.evidence_span_ids.contains(id))
        {
            return Err("Archaeology synthesis request relationship is unsafe or dangling".into());
        }
        request_relationships.push(ArchaeologySynthesisRelationship {
            relationship_id: edge.edge_id.clone(),
            from_fact_id: edge.from_fact_id.clone(),
            to_fact_id: edge.to_fact_id.clone(),
            kind: edge.kind.clone(),
            trust: edge.trust.clone(),
            unresolved: edge.kind == ArchaeologyFactEdgeKind::Unresolved
                || edge.unresolved_reason.is_some(),
        });
    }

    let mut request = ArchaeologySynthesisRequest {
        schema_version: ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION,
        contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID.into(),
        request_id: String::new(),
        repository_id: repository_id.into(),
        generation_id: generation_id.into(),
        revision_sha: revision_sha.into(),
        parser_identity: parser_identity.into(),
        algorithm_identity: algorithm_identity.into(),
        packet: packet.clone(),
        facts: request_facts,
        relationships: request_relationships,
    };
    let request_digest = Sha256::digest(
        serde_json::to_vec(&request)
            .map_err(|_| "Archaeology synthesis request is not serializable")?,
    );
    request.request_id = format!(
        "sha256:{}",
        super::inventory::hex(request_digest.as_slice())
    );
    validate_synthesis_request(&request, limits)?;
    cancelled(cancellation)?;
    Ok(request)
}

pub(crate) fn validate_synthesis_request(
    request: &ArchaeologySynthesisRequest,
    limits: ArchaeologySynthesisLimits,
) -> Result<(), String> {
    if request.schema_version != ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION
        || request.contract_id != ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID
        || !safe_scope_id(&request.repository_id)
        || !safe_scope_id(&request.generation_id)
        || !safe_scope_id(&request.parser_identity)
        || !safe_scope_id(&request.algorithm_identity)
        || validate_revision_sha(&request.revision_sha).is_err()
        || request.facts.len() > limits.max_facts
        || request.relationships.len() > limits.max_relationships
    {
        return Err("Archaeology synthesis request identity or count bound is invalid".into());
    }
    validate_packet_shape(&request.packet, limits)?;
    if request.packet.packet_id
        != expected_packet_id(
            &request.repository_id,
            &request.revision_sha,
            &request.packet,
        )
    {
        return Err("Archaeology synthesis packet identity does not match its evidence".into());
    }
    let expected_facts = request
        .packet
        .supporting_fact_ids
        .iter()
        .chain(&request.packet.contradicting_fact_ids)
        .chain(&request.packet.unresolved_fact_ids)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_facts = request
        .facts
        .iter()
        .map(|fact| fact.fact_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected_facts != actual_facts
        || actual_facts.len() != request.facts.len()
        || !request
            .facts
            .windows(2)
            .all(|pair| pair[0].fact_id < pair[1].fact_id)
        || request.facts.iter().any(|fact| {
            !safe_id(&fact.fact_id)
                || !safe_text(&fact.label, limits.max_text_bytes)
                || !matches!(
                    fact.trust,
                    ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
                )
                || fact.confidence == ArchaeologyConfidence::Unavailable
                || fact.quantifier_kinds.len() > 6
                || !fact
                    .quantifier_kinds
                    .windows(2)
                    .all(|pair| pair[0] < pair[1])
        })
    {
        return Err("Archaeology synthesis request fact projection is invalid".into());
    }
    let expected_relationships = request
        .packet
        .relationship_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_relationships = request
        .relationships
        .iter()
        .map(|relationship| relationship.relationship_id.as_str())
        .collect::<BTreeSet<_>>();
    if expected_relationships != actual_relationships
        || actual_relationships.len() != request.relationships.len()
        || !request
            .relationships
            .windows(2)
            .all(|pair| pair[0].relationship_id < pair[1].relationship_id)
        || request.relationships.iter().any(|relationship| {
            !safe_id(&relationship.relationship_id)
                || !expected_facts.contains(relationship.from_fact_id.as_str())
                || !expected_facts.contains(relationship.to_fact_id.as_str())
                || !matches!(
                    relationship.trust,
                    ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
                )
                || relationship.kind == ArchaeologyFactEdgeKind::Unresolved
                    && !relationship.unresolved
        })
    {
        return Err("Archaeology synthesis request relationship projection is invalid".into());
    }
    let mut identity = request.clone();
    identity.request_id.clear();
    let expected_request_id = format!(
        "sha256:{}",
        super::inventory::hex(
            Sha256::digest(
                serde_json::to_vec(&identity)
                    .map_err(|_| "Archaeology synthesis request is not serializable")?
            )
            .as_slice()
        )
    );
    if request.request_id != expected_request_id {
        return Err("Archaeology synthesis request identity does not match its payload".into());
    }
    if json_bytes(request)? > limits.max_request_bytes {
        return Err("Archaeology synthesis request byte bound exceeded".into());
    }
    Ok(())
}

pub(crate) fn parse_synthesis_response(
    raw: &[u8],
    request: &ArchaeologySynthesisRequest,
    limits: ArchaeologySynthesisLimits,
) -> Result<ArchaeologySynthesisResponse, String> {
    if raw.len() > limits.max_response_bytes {
        return Err("Archaeology synthesis response byte bound exceeded".into());
    }
    let raw_text = std::str::from_utf8(raw)
        .map_err(|_| "Archaeology synthesis response must be UTF-8 JSON")?;
    if unsafe_text(raw_text) {
        return Err("Archaeology synthesis response contains private or unsafe text".into());
    }
    let response: ArchaeologySynthesisResponse = serde_json::from_slice(raw)
        .map_err(|_| "Archaeology synthesis response is not strict contract JSON")?;
    validate_synthesis_response(request, &response, limits)?;
    Ok(response)
}

pub(crate) fn validate_synthesis_response(
    request: &ArchaeologySynthesisRequest,
    response: &ArchaeologySynthesisResponse,
    limits: ArchaeologySynthesisLimits,
) -> Result<(), String> {
    if response.schema_version != ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION
        || response.contract_id != ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID
        || response.request_id != request.request_id
        || response.packet_id != request.packet.packet_id
        || response.clauses.is_empty()
        || response.clauses.len() > limits.max_clauses
    {
        return Err("Archaeology synthesis response identity or clause count is invalid".into());
    }
    let supporting = request
        .packet
        .supporting_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let contradicting = request
        .packet
        .contradicting_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let relationships = request
        .packet
        .relationship_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let facts_by_id = request
        .facts
        .iter()
        .map(|fact| (fact.fact_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let relationships_by_id = request
        .relationships
        .iter()
        .map(|relationship| (relationship.relationship_id.as_str(), relationship))
        .collect::<BTreeMap<_, _>>();
    let mut clause_shapes = BTreeSet::new();
    // Contradictions are an exact response-wide partition: every packet
    // contradiction belongs to one clause, once. This prevents omission and
    // cross-clause laundering while retaining per-clause relationship checks.
    let mut reconciled_contradictions = BTreeSet::new();
    for clause in &response.clauses {
        validate_segment(&clause.subject, &supporting, limits)?;
        validate_segment(&clause.action, &supporting, limits)?;
        if let Some(condition) = &clause.condition {
            validate_segment(condition, &supporting, limits)?;
        }
        if let Some(exception) = &clause.exception {
            validate_segment(exception, &supporting, limits)?;
        }
        if let Some(quantifier) = &clause.quantifier {
            validate_ids(
                &quantifier.fact_ids,
                &supporting,
                limits.max_fact_ids_per_segment,
                false,
            )?;
        }
        validate_ids(
            &clause.relationship_ids,
            &relationships,
            limits.max_relationship_ids_per_clause,
            true,
        )?;
        validate_ids(
            &clause.contradicting_fact_ids,
            &contradicting,
            limits.max_fact_ids_per_segment,
            true,
        )?;
        let positive = clause.supporting_fact_ids();
        if clause
            .contradicting_fact_ids
            .iter()
            .any(|id| positive.contains(id.as_str()))
        {
            return Err(
                "Archaeology synthesis clause mixes positive and contradicting evidence".into(),
            );
        }
        for fact_id in &clause.contradicting_fact_ids {
            if !reconciled_contradictions.insert(fact_id.as_str()) {
                return Err(
                    "Archaeology synthesis contradiction is assigned to multiple clauses".into(),
                );
            }
        }
        validate_clause_semantics(clause, &facts_by_id, &relationships_by_id)?;
        validate_segment_text_support(&clause.subject, &facts_by_id)?;
        validate_segment_text_support(&clause.action, &facts_by_id)?;
        if let Some(condition) = &clause.condition {
            validate_segment_text_support(condition, &facts_by_id)?;
        }
        if let Some(exception) = &clause.exception {
            validate_segment_text_support(exception, &facts_by_id)?;
        }
        let shape = serde_json::to_string(clause)
            .map_err(|_| "Archaeology synthesis clause is not serializable")?;
        if !clause_shapes.insert(shape) {
            return Err("Archaeology synthesis response contains duplicate clauses".into());
        }
    }
    if reconciled_contradictions != contradicting {
        return Err(
            "Archaeology synthesis response does not reconcile every packet contradiction".into(),
        );
    }
    if json_bytes(response)? > limits.max_response_bytes {
        return Err("Archaeology synthesis response byte bound exceeded".into());
    }
    Ok(())
}

/// Convert a validated provider response into the only response shape that may
/// be retained or returned. Structure and evidence references survive; every
/// free-text segment is replaced with deterministic text from its cited fact
/// labels so provider prose never crosses the durable boundary.
pub(crate) fn canonicalize_synthesis_response(
    request: &ArchaeologySynthesisRequest,
    response: &ArchaeologySynthesisResponse,
    limits: ArchaeologySynthesisLimits,
) -> Result<ArchaeologySynthesisResponse, String> {
    validate_synthesis_response(request, response, limits)?;
    let facts = request
        .facts
        .iter()
        .map(|fact| (fact.fact_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let mut canonical = response.clone();
    for clause in &mut canonical.clauses {
        clause.subject.text = canonical_segment_text(&clause.subject.fact_ids, &facts)?;
        clause.action.text = canonical_segment_text(&clause.action.fact_ids, &facts)?;
        if let Some(condition) = &mut clause.condition {
            condition.text = canonical_segment_text(&condition.fact_ids, &facts)?;
        }
        if let Some(exception) = &mut clause.exception {
            exception.text = canonical_segment_text(&exception.fact_ids, &facts)?;
        }
    }
    validate_synthesis_response(request, &canonical, limits)?;
    Ok(canonical)
}

/// Render the only prose that may enter the canonical rule catalog from a
/// model-assisted response. Provider prose is validated as a conservative
/// paraphrase, but is never persisted. The published sentence is rebuilt from
/// cited fact labels, typed relationships, and the closed quantifier enum.
pub(crate) fn canonical_synthesis_clause_text(
    request: &ArchaeologySynthesisRequest,
    clause: &ArchaeologySynthesisClause,
) -> Result<String, String> {
    let facts = request
        .facts
        .iter()
        .map(|fact| (fact.fact_id.as_str(), fact))
        .collect::<BTreeMap<_, _>>();
    let relationships = request
        .relationships
        .iter()
        .map(|relationship| (relationship.relationship_id.as_str(), relationship))
        .collect::<BTreeMap<_, _>>();
    let mut parts = vec![format!(
        "Subject: {}.",
        canonical_fact_list(&clause.subject.fact_ids, &facts)?
    )];
    if let Some(condition) = &clause.condition {
        parts.push(format!(
            "Condition: {}.",
            canonical_fact_list(&condition.fact_ids, &facts)?
        ));
    }
    parts.push(format!(
        "Action: {}.",
        canonical_fact_list(&clause.action.fact_ids, &facts)?
    ));
    if let Some(exception) = &clause.exception {
        parts.push(format!(
            "Exception evidence: {}.",
            canonical_fact_list(&exception.fact_ids, &facts)?
        ));
    }
    if let Some(quantifier) = &clause.quantifier {
        let kind = match quantifier.kind {
            ArchaeologySynthesisQuantifierKind::All => "all",
            ArchaeologySynthesisQuantifierKind::Any => "any",
            ArchaeologySynthesisQuantifierKind::None => "none",
            ArchaeologySynthesisQuantifierKind::ExactlyOne => "exactly one",
            ArchaeologySynthesisQuantifierKind::AtLeastOne => "at least one",
            ArchaeologySynthesisQuantifierKind::AtMostOne => "at most one",
        };
        parts.push(format!(
            "Quantifier: {kind} of {}.",
            canonical_fact_list(&quantifier.fact_ids, &facts)?
        ));
    }
    for relationship_id in &clause.relationship_ids {
        let relationship = relationships
            .get(relationship_id.as_str())
            .ok_or("Archaeology synthesis clause cites an unknown relationship")?;
        let from = canonical_fact_list(std::slice::from_ref(&relationship.from_fact_id), &facts)?;
        let to = canonical_fact_list(std::slice::from_ref(&relationship.to_fact_id), &facts)?;
        parts.push(format!(
            "Relationship: {from} {} {to}.",
            relationship_verb(&relationship.kind)
        ));
    }
    if !clause.contradicting_fact_ids.is_empty() {
        parts.push(format!(
            "Contradicting evidence: {}.",
            canonical_fact_list(&clause.contradicting_fact_ids, &facts)?
        ));
    }
    let text = parts.join(" ");
    if text.len() > 64 * 1024 || unsafe_text(&text) {
        return Err("Archaeology canonical synthesis clause exceeds its safety bound".into());
    }
    Ok(text)
}

fn validate_packet_shape(
    packet: &ArchaeologyEvidencePacket,
    limits: ArchaeologySynthesisLimits,
) -> Result<(), String> {
    if !safe_id(&packet.packet_id)
        || !safe_id(&packet.anchor_fact_id)
        || !packet.supporting_fact_ids.contains(&packet.anchor_fact_id)
        || packet.supporting_fact_ids.len()
            + packet.contradicting_fact_ids.len()
            + packet.unresolved_fact_ids.len()
            > limits.max_facts
        || packet.relationship_ids.len() > limits.max_relationships
        || packet.evidence_span_ids.len() > limits.max_evidence_spans
        || packet.caveats.len() > limits.max_packet_caveats
        || packet.unresolved_reasons.len() > limits.max_unresolved_reasons
        || packet.unresolved_fact_ids.is_empty() != packet.unresolved_reasons.is_empty()
        || packet.evidence_span_ids.is_empty()
        || !sorted_unique(&packet.supporting_fact_ids)
        || !sorted_unique(&packet.contradicting_fact_ids)
        || !sorted_unique(&packet.unresolved_fact_ids)
        || !sorted_unique(&packet.relationship_ids)
        || !sorted_unique(&packet.evidence_span_ids)
        || !roles_are_disjoint(packet)
        || !packet_metadata_is_categorical(packet)
        || packet
            .supporting_fact_ids
            .iter()
            .chain(&packet.contradicting_fact_ids)
            .chain(&packet.unresolved_fact_ids)
            .chain(&packet.relationship_ids)
            .chain(&packet.evidence_span_ids)
            .any(|id| !safe_id(id))
        || packet
            .caveats
            .iter()
            .chain(&packet.unresolved_reasons)
            .any(|value| !safe_text(value, limits.max_text_bytes))
    {
        return Err("Archaeology synthesis packet shape is invalid".into());
    }
    Ok(())
}

fn validate_segment(
    segment: &ArchaeologySynthesisSegment,
    allowed: &BTreeSet<&str>,
    limits: ArchaeologySynthesisLimits,
) -> Result<(), String> {
    if !safe_text(&segment.text, limits.max_text_bytes) {
        return Err("Archaeology synthesis clause segment text is invalid".into());
    }
    validate_ids(
        &segment.fact_ids,
        allowed,
        limits.max_fact_ids_per_segment,
        false,
    )
}

fn validate_segment_text_support(
    segment: &ArchaeologySynthesisSegment,
    facts: &BTreeMap<&str, &ArchaeologySynthesisFact>,
) -> Result<(), String> {
    let supported = segment
        .fact_ids
        .iter()
        .filter_map(|id| facts.get(id.as_str()))
        .flat_map(|fact| semantic_tokens(&fact.label))
        .collect::<BTreeSet<_>>();
    let unsupported = semantic_tokens(&segment.text).into_iter().any(|token| {
        !supported.contains(&token)
            && !matches!(
                token.as_str(),
                "a" | "an"
                    | "are"
                    | "as"
                    | "at"
                    | "be"
                    | "been"
                    | "being"
                    | "by"
                    | "for"
                    | "from"
                    | "in"
                    | "is"
                    | "of"
                    | "on"
                    | "that"
                    | "the"
                    | "these"
                    | "this"
                    | "those"
                    | "to"
                    | "was"
                    | "were"
                    | "with"
            )
    });
    if unsupported {
        Err("Archaeology synthesis clause prose is not supported by its cited fact labels".into())
    } else {
        Ok(())
    }
}

fn semantic_tokens(value: &str) -> Vec<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

pub(crate) fn quantifier_kinds_from_evidence(
    label: &str,
    attributes: &[ArchaeologyAttribute],
) -> Vec<ArchaeologySynthesisQuantifierKind> {
    let mut kinds = attributes
        .iter()
        .filter(|attribute| matches!(attribute.key.as_str(), "quantifier" | "cardinality"))
        .filter_map(|attribute| match attribute.value.as_str() {
            "all" => Some(ArchaeologySynthesisQuantifierKind::All),
            "any" => Some(ArchaeologySynthesisQuantifierKind::Any),
            "none" => Some(ArchaeologySynthesisQuantifierKind::None),
            "exactly_one" => Some(ArchaeologySynthesisQuantifierKind::ExactlyOne),
            "at_least_one" => Some(ArchaeologySynthesisQuantifierKind::AtLeastOne),
            "at_most_one" => Some(ArchaeologySynthesisQuantifierKind::AtMostOne),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let tokens = semantic_tokens(label);
    let negated = tokens
        .iter()
        .any(|token| matches!(token.as_str(), "not" | "never" | "without"));
    if !negated && tokens.iter().any(|token| token == "all") {
        kinds.insert(ArchaeologySynthesisQuantifierKind::All);
    }
    if !negated && tokens.iter().any(|token| token == "any") {
        kinds.insert(ArchaeologySynthesisQuantifierKind::Any);
    }
    if !negated && tokens.iter().any(|token| token == "none") {
        kinds.insert(ArchaeologySynthesisQuantifierKind::None);
    }
    if !negated && contains_token_phrase(&tokens, &["exactly", "one"]) {
        kinds.insert(ArchaeologySynthesisQuantifierKind::ExactlyOne);
    }
    if !negated && contains_token_phrase(&tokens, &["at", "least", "one"]) {
        kinds.insert(ArchaeologySynthesisQuantifierKind::AtLeastOne);
    }
    if !negated && contains_token_phrase(&tokens, &["at", "most", "one"]) {
        kinds.insert(ArchaeologySynthesisQuantifierKind::AtMostOne);
    }
    kinds.into_iter().collect()
}

fn contains_token_phrase(tokens: &[String], phrase: &[&str]) -> bool {
    tokens
        .windows(phrase.len())
        .any(|window| window.iter().map(String::as_str).eq(phrase.iter().copied()))
}

fn canonical_fact_list(
    fact_ids: &[String],
    facts: &BTreeMap<&str, &ArchaeologySynthesisFact>,
) -> Result<String, String> {
    fact_ids
        .iter()
        .map(|id| {
            let fact = facts
                .get(id.as_str())
                .ok_or("Archaeology synthesis clause cites an unknown fact")?;
            let label = fact.label.split_whitespace().collect::<Vec<_>>().join(" ");
            let label = label.replace('"', "'");
            Ok(format!("{} \"{label}\"", fact_kind_name(&fact.kind)))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|values| values.join("; "))
}

fn canonical_segment_text(
    fact_ids: &[String],
    facts: &BTreeMap<&str, &ArchaeologySynthesisFact>,
) -> Result<String, String> {
    fact_ids
        .iter()
        .map(|id| {
            facts
                .get(id.as_str())
                .map(|fact| fact.label.split_whitespace().collect::<Vec<_>>().join(" "))
                .ok_or_else(|| "Archaeology synthesis segment cites an unknown fact".to_string())
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|labels| labels.join("; "))
}

fn fact_kind_name(kind: &ArchaeologyFactKind) -> &'static str {
    match kind {
        ArchaeologyFactKind::Declaration => "declaration",
        ArchaeologyFactKind::DataField => "data field",
        ArchaeologyFactKind::Constant => "constant",
        ArchaeologyFactKind::Predicate => "predicate",
        ArchaeologyFactKind::Decision => "decision",
        ArchaeologyFactKind::Calculation => "calculation",
        ArchaeologyFactKind::Mutation => "mutation",
        ArchaeologyFactKind::Call => "call",
        ArchaeologyFactKind::InputOutput => "I/O operation",
        ArchaeologyFactKind::Transaction => "transaction",
        ArchaeologyFactKind::ControlFlow => "control-flow operation",
        ArchaeologyFactKind::EntryPoint => "entry point",
        ArchaeologyFactKind::Include => "include",
        ArchaeologyFactKind::Unresolved => "unresolved reference",
    }
}

fn relationship_verb(kind: &ArchaeologyFactEdgeKind) -> &'static str {
    match kind {
        ArchaeologyFactEdgeKind::Defines => "defines",
        ArchaeologyFactEdgeKind::Reads => "reads",
        ArchaeologyFactEdgeKind::Writes => "writes",
        ArchaeologyFactEdgeKind::Calls => "calls",
        ArchaeologyFactEdgeKind::Includes => "includes",
        ArchaeologyFactEdgeKind::Controls => "controls",
        ArchaeologyFactEdgeKind::BranchesTo => "branches to",
        ArchaeologyFactEdgeKind::Calculates => "calculates",
        ArchaeologyFactEdgeKind::BeginsTransaction => "begins",
        ArchaeologyFactEdgeKind::CommitsTransaction => "commits",
        ArchaeologyFactEdgeKind::RollsBackTransaction => "rolls back",
        ArchaeologyFactEdgeKind::Supports => "supports",
        ArchaeologyFactEdgeKind::Contradicts => "contradicts",
        ArchaeologyFactEdgeKind::Aliases => "aliases",
        ArchaeologyFactEdgeKind::Unresolved => "has an unresolved link to",
    }
}

fn validate_ids(
    values: &[String],
    allowed: &BTreeSet<&str>,
    max: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && values.is_empty())
        || values.len() > max
        || !sorted_unique(values)
        || values
            .iter()
            .any(|value| !safe_id(value) || !allowed.contains(value.as_str()))
    {
        return Err("Archaeology synthesis evidence references are invalid".into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ClauseSegmentRole {
    Subject,
    Condition,
    Action,
    Exception,
    Quantifier,
}

fn validate_clause_semantics(
    clause: &ArchaeologySynthesisClause,
    facts: &BTreeMap<&str, &ArchaeologySynthesisFact>,
    relationships: &BTreeMap<&str, &ArchaeologySynthesisRelationship>,
) -> Result<(), String> {
    validate_segment_role(&clause.subject.fact_ids, facts, ClauseSegmentRole::Subject)?;
    validate_segment_role(&clause.action.fact_ids, facts, ClauseSegmentRole::Action)?;
    if let Some(condition) = &clause.condition {
        validate_segment_role(&condition.fact_ids, facts, ClauseSegmentRole::Condition)?;
    }
    if let Some(exception) = &clause.exception {
        validate_segment_role(&exception.fact_ids, facts, ClauseSegmentRole::Exception)?;
    }
    if let Some(quantifier) = &clause.quantifier {
        validate_segment_role(&quantifier.fact_ids, facts, ClauseSegmentRole::Quantifier)?;
        if !quantifier.fact_ids.iter().all(|fact_id| {
            facts.get(fact_id.as_str()).is_some_and(|fact| {
                fact_supports_role(&fact.kind, ClauseSegmentRole::Quantifier)
                    && fact.quantifier_kinds.contains(&quantifier.kind)
            })
        }) {
            return Err(
                "Archaeology synthesis quantifier lacks exact typed evidence support".into(),
            );
        }
    }

    let positive = clause
        .subject
        .fact_ids
        .iter()
        .chain(&clause.action.fact_ids)
        .chain(
            clause
                .condition
                .iter()
                .flat_map(|segment| &segment.fact_ids),
        )
        .chain(
            clause
                .exception
                .iter()
                .flat_map(|segment| &segment.fact_ids),
        )
        .chain(
            clause
                .quantifier
                .iter()
                .flat_map(|quantifier| &quantifier.fact_ids),
        )
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let contradicting = clause
        .contradicting_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut adjacency = BTreeMap::<&str, BTreeSet<&str>>::new();
    let mut supported_contradictions = BTreeMap::<&str, usize>::new();

    for relationship_id in &clause.relationship_ids {
        let relationship = relationships
            .get(relationship_id.as_str())
            .ok_or("Archaeology synthesis clause cites an unknown relationship")?;
        if relationship.unresolved
            || relationship.kind == ArchaeologyFactEdgeKind::Unresolved
            || !matches!(
                relationship.trust,
                ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
            )
        {
            return Err(
                "Archaeology synthesis clause cites an unresolved or untrusted relationship".into(),
            );
        }
        let from_positive = positive.contains(relationship.from_fact_id.as_str());
        let to_positive = positive.contains(relationship.to_fact_id.as_str());
        let from_contradicting = contradicting.contains(relationship.from_fact_id.as_str());
        let to_contradicting = contradicting.contains(relationship.to_fact_id.as_str());
        if relationship.kind == ArchaeologyFactEdgeKind::Contradicts {
            if !(from_positive && to_contradicting || to_positive && from_contradicting) {
                return Err(
                    "Archaeology synthesis contradiction relationship does not reconcile".into(),
                );
            }
            if from_contradicting {
                *supported_contradictions
                    .entry(relationship.from_fact_id.as_str())
                    .or_default() += 1;
            }
            if to_contradicting {
                *supported_contradictions
                    .entry(relationship.to_fact_id.as_str())
                    .or_default() += 1;
            }
        } else {
            if !from_positive || !to_positive {
                return Err(
                    "Archaeology synthesis relationship does not connect cited positive facts"
                        .into(),
                );
            }
            adjacency
                .entry(relationship.from_fact_id.as_str())
                .or_default()
                .insert(relationship.to_fact_id.as_str());
            adjacency
                .entry(relationship.to_fact_id.as_str())
                .or_default()
                .insert(relationship.from_fact_id.as_str());
        }
    }

    if supported_contradictions
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != contradicting
        || supported_contradictions.values().any(|count| *count != 1)
    {
        return Err(
            "Archaeology synthesis contradicting facts lack exactly one cited relationship support"
                .into(),
        );
    }
    if positive.len() > 1 {
        let first = *positive
            .first()
            .ok_or("Archaeology synthesis clause has no positive evidence")?;
        let mut reached = BTreeSet::from([first]);
        let mut pending = vec![first];
        while let Some(current) = pending.pop() {
            for adjacent in adjacency.get(current).into_iter().flatten() {
                if reached.insert(adjacent) {
                    pending.push(adjacent);
                }
            }
        }
        if reached != positive {
            return Err(
                "Archaeology synthesis clause facts lack one cited relationship path".into(),
            );
        }
    }
    Ok(())
}

fn validate_segment_role(
    fact_ids: &[String],
    facts: &BTreeMap<&str, &ArchaeologySynthesisFact>,
    role: ClauseSegmentRole,
) -> Result<(), String> {
    let supported = fact_ids.iter().all(|fact_id| {
        facts
            .get(fact_id.as_str())
            .is_some_and(|fact| fact_supports_role(&fact.kind, role))
    });
    if supported {
        Ok(())
    } else {
        Err("Archaeology synthesis clause segment lacks semantic fact support".into())
    }
}

fn fact_supports_role(kind: &ArchaeologyFactKind, role: ClauseSegmentRole) -> bool {
    match role {
        ClauseSegmentRole::Subject => !matches!(kind, ArchaeologyFactKind::Unresolved),
        ClauseSegmentRole::Condition | ClauseSegmentRole::Exception => matches!(
            kind,
            ArchaeologyFactKind::DataField
                | ArchaeologyFactKind::Constant
                | ArchaeologyFactKind::Predicate
                | ArchaeologyFactKind::Decision
                | ArchaeologyFactKind::Calculation
                | ArchaeologyFactKind::ControlFlow
        ),
        ClauseSegmentRole::Action => matches!(
            kind,
            ArchaeologyFactKind::Decision
                | ArchaeologyFactKind::Calculation
                | ArchaeologyFactKind::Mutation
                | ArchaeologyFactKind::Call
                | ArchaeologyFactKind::InputOutput
                | ArchaeologyFactKind::Transaction
                | ArchaeologyFactKind::ControlFlow
        ),
        ClauseSegmentRole::Quantifier => matches!(
            kind,
            ArchaeologyFactKind::DataField
                | ArchaeologyFactKind::Constant
                | ArchaeologyFactKind::Predicate
                | ArchaeologyFactKind::Decision
                | ArchaeologyFactKind::Calculation
        ),
    }
}

fn unique_map<'a, T>(
    values: &'a [T],
    id: impl Fn(&'a T) -> &'a str,
    label: &str,
) -> Result<BTreeMap<&'a str, &'a T>, String> {
    let mut output = BTreeMap::new();
    for value in values {
        let identity = id(value);
        if !safe_id(identity) || output.insert(identity, value).is_some() {
            return Err(format!(
                "Archaeology synthesis {label} identity is invalid or duplicate"
            ));
        }
    }
    Ok(output)
}

fn sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('\0')
        && !value.contains(['/', '\\'])
        && !value.chars().any(char::is_whitespace)
        && !contains_sensitive_path(value)
        && !looks_like_secret(value)
}

fn safe_scope_id(value: &str) -> bool {
    safe_id(value)
}

fn roles_are_disjoint(packet: &ArchaeologyEvidencePacket) -> bool {
    let supporting = packet.supporting_fact_ids.iter().collect::<BTreeSet<_>>();
    let contradicting = packet
        .contradicting_fact_ids
        .iter()
        .collect::<BTreeSet<_>>();
    let unresolved = packet.unresolved_fact_ids.iter().collect::<BTreeSet<_>>();
    supporting.is_disjoint(&contradicting)
        && supporting.is_disjoint(&unresolved)
        && contradicting.is_disjoint(&unresolved)
}

fn safe_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max_bytes
        && !unsafe_text(value)
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn unsafe_text(value: &str) -> bool {
    value.contains('\0')
        || looks_like_secret(value)
        || contains_sensitive_path(value)
        || contains_absolute_path(value)
}

fn contains_absolute_path(value: &str) -> bool {
    value
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '`' | '"'
                        | '\''
                        | ','
                        | ';'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '='
                )
        })
        .filter(|token| !token.is_empty())
        .any(|token| {
            let normalized = token.replace('\\', "/");
            let bytes = normalized.as_bytes();
            normalized.starts_with('/')
                || normalized.to_ascii_lowercase().starts_with("file:/")
                || (bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && bytes[2] == b'/')
        })
}

fn json_bytes(value: &impl Serialize) -> Result<usize, String> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|_| "Archaeology synthesis contract is not serializable".into())
}

fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology synthesis request cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::business_rule_archaeology::contracts::ArchaeologyRuleKind;

    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn one_packet_builds_a_sorted_private_request_and_accepts_only_cited_segments() {
        let (packet, mut facts, mut edges) = fixture();
        facts.reverse();
        edges.reverse();
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .expect("request");
        assert_eq!(
            request
                .facts
                .iter()
                .map(|fact| fact.fact_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "fact:action",
                "fact:condition",
                "fact:contradiction",
                "fact:unresolved"
            ]
        );
        assert!(request.request_id.starts_with("sha256:"));
        assert!(serde_json::to_string(&request)
            .expect("request JSON")
            .contains(ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID));
        let response = valid_response(&request);
        let raw = serde_json::to_vec(&response).unwrap();
        assert_eq!(
            parse_synthesis_response(&raw, &request, Default::default()).unwrap(),
            response
        );
        let canonical =
            canonicalize_synthesis_response(&request, &response, Default::default()).unwrap();
        assert_eq!(canonical.clauses[0].subject.text, "Positive payment");
        assert_eq!(
            canonical.clauses[0].condition.as_ref().unwrap().text,
            "Positive payment"
        );
        assert_eq!(canonical.clauses[0].action.text, "Schedule payment");
        assert!(!serde_json::to_string(&canonical)
            .unwrap()
            .contains("the payment is positive"));
        let mut forged = request.clone();
        forged.packet.caveats = vec!["organizational policy requires this action".into()];
        reidentify_request(&mut forged);
        assert!(validate_synthesis_request(&forged, Default::default()).is_err());
    }

    #[test]
    fn request_reconciles_exact_evidence_and_rejects_private_bounds_or_cancellation() {
        let (packet, facts, edges) = fixture();
        let build = |packet: &ArchaeologyEvidencePacket,
                     facts: &[ArchaeologyFact],
                     edges: &[ArchaeologyFactEdge],
                     cancellation: &StructuralGraphCancellation,
                     limits| {
            build_synthesis_request(
                "repository:one",
                "generation:one",
                REVISION,
                "parser:v1",
                "algorithm:v1",
                packet,
                facts,
                edges,
                cancellation,
                limits,
            )
        };
        assert!(build(
            &packet,
            &facts[..1],
            &edges,
            &Default::default(),
            Default::default()
        )
        .unwrap_err()
        .contains("fact set"));
        let mut tampered_identity = packet.clone();
        tampered_identity.packet_id = "packet:tampered".into();
        assert!(build(
            &tampered_identity,
            &facts,
            &edges,
            &Default::default(),
            Default::default()
        )
        .unwrap_err()
        .contains("identity"));
        let mut overlapping_roles = packet.clone();
        overlapping_roles
            .supporting_fact_ids
            .push("fact:contradiction".into());
        overlapping_roles.supporting_fact_ids.sort();
        overlapping_roles.packet_id =
            expected_packet_id("repository:one", REVISION, &overlapping_roles);
        assert!(build(
            &overlapping_roles,
            &facts,
            &edges,
            &Default::default(),
            Default::default()
        )
        .is_err());
        let mut extra_span = packet.clone();
        extra_span.evidence_span_ids.push("span:z-extra".into());
        extra_span.packet_id = expected_packet_id("repository:one", REVISION, &extra_span);
        assert!(build(
            &extra_span,
            &facts,
            &edges,
            &Default::default(),
            Default::default()
        )
        .unwrap_err()
        .contains("span set"));
        let mut private = facts.clone();
        private[0].label = "Authorization: Bearer private-runtime-token".into();
        assert!(build(
            &packet,
            &private,
            &edges,
            &Default::default(),
            Default::default()
        )
        .is_err());
        for absolute_path in [
            "See (/private/tmp/repository/rules.cbl)",
            r"See C:\Users\analyst\repository\rules.cbl",
            r"See \\server\share\rules.cbl",
        ] {
            let mut private = facts.clone();
            private[0].label = absolute_path.into();
            assert!(build(
                &packet,
                &private,
                &edges,
                &Default::default(),
                Default::default()
            )
            .is_err());
        }
        let limits = ArchaeologySynthesisLimits {
            max_request_bytes: 32,
            ..Default::default()
        };
        assert!(build(&packet, &facts, &edges, &Default::default(), limits).is_err());
        let cancellation = StructuralGraphCancellation::default();
        cancellation.cancel();
        assert!(
            build(&packet, &facts, &edges, &cancellation, Default::default())
                .unwrap_err()
                .contains("cancelled")
        );
    }

    #[test]
    fn response_rejects_unknown_duplicate_private_oversized_and_dangling_data() {
        let (packet, facts, edges) = fixture();
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &Default::default(),
            Default::default(),
        )
        .unwrap();
        let valid = serde_json::to_string(&valid_response(&request)).unwrap();
        let unknown = valid.replacen("\"clauses\":", "\"unknown\":true,\"clauses\":", 1);
        assert!(
            parse_synthesis_response(unknown.as_bytes(), &request, Default::default()).is_err()
        );
        let duplicate = valid.replacen(
            "\"schema_version\":1,",
            "\"schema_version\":1,\"schema_version\":1,",
            1,
        );
        assert!(
            parse_synthesis_response(duplicate.as_bytes(), &request, Default::default()).is_err()
        );
        let private = valid.replace("Payment", "password=correct-horse-battery-staple");
        assert!(
            parse_synthesis_response(private.as_bytes(), &request, Default::default()).is_err()
        );
        for absolute_path in [
            "See (/private/tmp/repository/rules.cbl)",
            r"See C:\Users\analyst\repository\rules.cbl",
            r"See \\server\share\rules.cbl",
        ] {
            let mut private = valid_response(&request);
            private.clauses[0].subject.text = absolute_path.into();
            assert!(parse_synthesis_response(
                &serde_json::to_vec(&private).unwrap(),
                &request,
                Default::default()
            )
            .is_err());
        }
        let limits = ArchaeologySynthesisLimits {
            max_response_bytes: 32,
            ..Default::default()
        };
        assert!(parse_synthesis_response(valid.as_bytes(), &request, limits).is_err());
        let injected_claim = valid.replacen(
            "\"subject\":",
            "\"clause_id\":\"model-owned\",\"trust\":\"human_confirmed\",\"subject\":",
            1,
        );
        assert!(
            parse_synthesis_response(injected_claim.as_bytes(), &request, Default::default())
                .is_err()
        );
        let invented_caveat = valid.replacen(
            "\"relationship_ids\":",
            "\"caveats\":[\"This is the organization's legal policy\"],\"relationship_ids\":",
            1,
        );
        assert!(
            parse_synthesis_response(invented_caveat.as_bytes(), &request, Default::default())
                .is_err()
        );
        let array_packet = valid.replacen(
            &format!("\"packet_id\":\"{}\"", request.packet.packet_id),
            "\"packet_id\":[\"packet:one\",\"packet:two\"]",
            1,
        );
        assert!(
            parse_synthesis_response(array_packet.as_bytes(), &request, Default::default())
                .is_err()
        );

        let mut dangling = valid_response(&request);
        dangling.clauses[0].action.fact_ids = vec!["fact:unknown".into()];
        assert!(validate_synthesis_response(&request, &dangling, Default::default()).is_err());
        let mut overlap = valid_response(&request);
        overlap.clauses[0].contradicting_fact_ids = vec!["fact:action".into()];
        assert!(validate_synthesis_response(&request, &overlap, Default::default()).is_err());
        let mut unresolved_support = valid_response(&request);
        unresolved_support.clauses[0].action.fact_ids = vec!["fact:unresolved".into()];
        assert!(
            validate_synthesis_response(&request, &unresolved_support, Default::default()).is_err()
        );
        let mut supporting_as_contradiction = valid_response(&request);
        supporting_as_contradiction.clauses[0].contradicting_fact_ids =
            vec!["fact:condition".into()];
        assert!(validate_synthesis_response(
            &request,
            &supporting_as_contradiction,
            Default::default()
        )
        .is_err());
        let mut unknown_relationship = valid_response(&request);
        unknown_relationship.clauses[0].relationship_ids = vec!["relationship:unknown".into()];
        assert!(
            validate_synthesis_response(&request, &unknown_relationship, Default::default())
                .is_err()
        );
        let mut duplicate_reference = valid_response(&request);
        duplicate_reference.clauses[0].subject.fact_ids =
            vec!["fact:condition".into(), "fact:condition".into()];
        assert!(
            validate_synthesis_response(&request, &duplicate_reference, Default::default())
                .is_err()
        );

        for semantic_reversal in [
            "Payment is not positive",
            "none Positive payment",
            "Positive payment without payment",
            "all Positive payment",
            "any Positive payment",
        ] {
            let mut reversed = valid_response(&request);
            reversed.clauses[0].condition.as_mut().unwrap().text = semantic_reversal.into();
            assert!(
                validate_synthesis_response(&request, &reversed, Default::default())
                    .unwrap_err()
                    .contains("prose is not supported"),
                "accepted semantic reversal: {semantic_reversal}"
            );
        }

        let mut unsupported_action = valid_response(&request);
        unsupported_action.clauses[0].action.fact_ids = vec!["fact:condition".into()];
        unsupported_action.clauses[0].relationship_ids.clear();
        assert!(
            validate_synthesis_response(&request, &unsupported_action, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let mut unsupported_exception = valid_response(&request);
        unsupported_exception.clauses[0].exception = Some(ArchaeologySynthesisSegment {
            text: "unless payment is scheduled".into(),
            fact_ids: vec!["fact:action".into()],
        });
        assert!(
            validate_synthesis_response(&request, &unsupported_exception, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let mut unsupported_quantifier = valid_response(&request);
        unsupported_quantifier.clauses[0].quantifier = Some(ArchaeologySynthesisQuantifier {
            kind: ArchaeologySynthesisQuantifierKind::ExactlyOne,
            fact_ids: vec!["fact:action".into()],
        });
        assert!(
            validate_synthesis_response(&request, &unsupported_quantifier, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let mut disconnected = valid_response(&request);
        disconnected.clauses[0].relationship_ids = vec!["relationship:contradicts".into()];
        assert!(
            validate_synthesis_response(&request, &disconnected, Default::default())
                .unwrap_err()
                .contains("relationship path")
        );

        let mut unresolved_relationship = valid_response(&request);
        unresolved_relationship.clauses[0].relationship_ids =
            vec!["relationship:unresolved".into()];
        assert!(validate_synthesis_response(
            &request,
            &unresolved_relationship,
            Default::default()
        )
        .unwrap_err()
        .contains("unresolved or untrusted"));

        let mut unsupported_contradiction = valid_response(&request);
        unsupported_contradiction.clauses[0].relationship_ids =
            vec!["relationship:controls".into()];
        assert!(validate_synthesis_response(
            &request,
            &unsupported_contradiction,
            Default::default()
        )
        .unwrap_err()
        .contains("lack exactly one cited relationship support"));

        let mut supported_contradiction = valid_response(&request);
        supported_contradiction.clauses[0].relationship_ids = vec![
            "relationship:contradicts".into(),
            "relationship:controls".into(),
        ];
        supported_contradiction.clauses[0].contradicting_fact_ids =
            vec!["fact:contradiction".into()];
        assert!(validate_synthesis_response(
            &request,
            &supported_contradiction,
            Default::default()
        )
        .is_ok());
    }

    #[test]
    fn quantifier_requires_exact_typed_evidence_after_role_validation() {
        let (packet, facts, edges) = fixture();
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap();

        let condition = request
            .facts
            .iter()
            .find(|fact| fact.fact_id == "fact:condition")
            .unwrap();
        assert_eq!(condition.kind, ArchaeologyFactKind::Predicate);
        assert_eq!(condition.label, "Positive payment");
        assert!(condition.quantifier_kinds.is_empty());

        for kind in [
            ArchaeologySynthesisQuantifierKind::All,
            ArchaeologySynthesisQuantifierKind::Any,
            ArchaeologySynthesisQuantifierKind::None,
            ArchaeologySynthesisQuantifierKind::ExactlyOne,
            ArchaeologySynthesisQuantifierKind::AtLeastOne,
            ArchaeologySynthesisQuantifierKind::AtMostOne,
        ] {
            let mut response = valid_response(&request);
            response.clauses[0].quantifier = Some(ArchaeologySynthesisQuantifier {
                kind,
                fact_ids: vec!["fact:condition".into()],
            });

            assert!(
                validate_synthesis_response(&request, &response, Default::default())
                    .unwrap_err()
                    .contains("lacks exact typed evidence support")
            );
        }
    }

    #[test]
    fn every_cited_fact_must_support_its_clause_segment_role() {
        let (packet, facts, edges) = fixture();
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap();
        let mixed = vec!["fact:action".into(), "fact:condition".into()];

        let mut mixed_action = valid_response(&request);
        mixed_action.clauses[0].action.fact_ids = mixed.clone();
        assert!(
            validate_synthesis_response(&request, &mixed_action, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let mut mixed_condition = valid_response(&request);
        mixed_condition.clauses[0]
            .condition
            .as_mut()
            .unwrap()
            .fact_ids = mixed.clone();
        assert!(
            validate_synthesis_response(&request, &mixed_condition, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let mut mixed_exception = valid_response(&request);
        mixed_exception.clauses[0].exception = Some(ArchaeologySynthesisSegment {
            text: "positive payment schedule".into(),
            fact_ids: mixed,
        });
        assert!(
            validate_synthesis_response(&request, &mixed_exception, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );

        let (mut packet, facts, edges) = fixture();
        packet.supporting_fact_ids.push("fact:unresolved".into());
        packet.supporting_fact_ids.sort();
        packet.unresolved_fact_ids.clear();
        packet.unresolved_reasons.clear();
        packet.packet_id = expected_packet_id("repository:one", REVISION, &packet);
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap();
        let mut mixed_subject = valid_response(&request);
        mixed_subject.clauses[0].subject.fact_ids =
            vec!["fact:condition".into(), "fact:unresolved".into()];
        assert!(
            validate_synthesis_response(&request, &mixed_subject, Default::default())
                .unwrap_err()
                .contains("semantic fact support")
        );
    }

    #[test]
    fn every_quantifier_fact_must_support_the_exact_selected_kind() {
        let (packet, mut facts, edges) = fixture();
        let condition = facts
            .iter_mut()
            .find(|fact| fact.fact_id == "fact:condition")
            .unwrap();
        condition.label = "All positive payment".into();
        let action = facts
            .iter_mut()
            .find(|fact| fact.fact_id == "fact:action")
            .unwrap();
        action.kind = ArchaeologyFactKind::Calculation;
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap();
        let mut response = valid_response(&request);
        response.clauses[0].quantifier = Some(ArchaeologySynthesisQuantifier {
            kind: ArchaeologySynthesisQuantifierKind::All,
            fact_ids: vec!["fact:action".into(), "fact:condition".into()],
        });

        assert!(
            validate_synthesis_response(&request, &response, Default::default())
                .unwrap_err()
                .contains("lacks exact typed evidence support")
        );
    }

    #[test]
    fn quantifier_accepts_only_exact_label_or_attribute_evidence() {
        let positive_labels = [
            (
                "All positive payment",
                ArchaeologySynthesisQuantifierKind::All,
            ),
            (
                "Any positive payment",
                ArchaeologySynthesisQuantifierKind::Any,
            ),
            (
                "None positive payment",
                ArchaeologySynthesisQuantifierKind::None,
            ),
            (
                "Exactly one positive payment",
                ArchaeologySynthesisQuantifierKind::ExactlyOne,
            ),
            (
                "At least one positive payment",
                ArchaeologySynthesisQuantifierKind::AtLeastOne,
            ),
            (
                "At most one positive payment",
                ArchaeologySynthesisQuantifierKind::AtMostOne,
            ),
        ];

        for (label, kind) in positive_labels {
            let (packet, mut facts, edges) = fixture();
            facts
                .iter_mut()
                .find(|fact| fact.fact_id == "fact:condition")
                .unwrap()
                .label = label.into();
            let request = build_synthesis_request(
                "repository:one",
                "generation:one",
                REVISION,
                "parser:v1",
                "algorithm:v1",
                &packet,
                &facts,
                &edges,
                &StructuralGraphCancellation::default(),
                Default::default(),
            )
            .unwrap();
            let mut response = valid_response(&request);
            response.clauses[0].quantifier = Some(ArchaeologySynthesisQuantifier {
                kind,
                fact_ids: vec!["fact:condition".into()],
            });

            assert!(validate_synthesis_response(&request, &response, Default::default()).is_ok());
            let canonical =
                canonicalize_synthesis_response(&request, &response, Default::default()).unwrap();
            assert_eq!(canonical.clauses[0].quantifier.as_ref().unwrap().kind, kind);
        }

        for negated_none in ["Not none positive payment", "None are not positive payment"] {
            assert!(!quantifier_kinds_from_evidence(negated_none, &[])
                .contains(&ArchaeologySynthesisQuantifierKind::None));
        }

        let (packet, mut facts, edges) = fixture();
        facts
            .iter_mut()
            .find(|fact| fact.fact_id == "fact:condition")
            .unwrap()
            .attributes
            .push(ArchaeologyAttribute {
                key: "cardinality".into(),
                value: "exactly_one".into(),
            });
        let request = build_synthesis_request(
            "repository:one",
            "generation:one",
            REVISION,
            "parser:v1",
            "algorithm:v1",
            &packet,
            &facts,
            &edges,
            &StructuralGraphCancellation::default(),
            Default::default(),
        )
        .unwrap();
        let mut response = valid_response(&request);
        response.clauses[0].quantifier = Some(ArchaeologySynthesisQuantifier {
            kind: ArchaeologySynthesisQuantifierKind::ExactlyOne,
            fact_ids: vec!["fact:condition".into()],
        });
        assert!(validate_synthesis_response(&request, &response, Default::default()).is_ok());
    }

    fn fixture() -> (
        ArchaeologyEvidencePacket,
        Vec<ArchaeologyFact>,
        Vec<ArchaeologyFactEdge>,
    ) {
        let fact = |id: &str, kind, label: &str| ArchaeologyFact {
            fact_id: id.into(),
            kind,
            label: label.into(),
            span_ids: vec![format!("span:{}", id.trim_start_matches("fact:"))],
            parser_id: "parser:v1".into(),
            trust: ArchaeologyTrust::Extracted,
            confidence: ArchaeologyConfidence::High,
            attributes: Vec::new(),
        };
        let mut packet = ArchaeologyEvidencePacket {
            packet_id: String::new(),
            kind: ArchaeologyRuleKind::Validation,
            anchor_fact_id: "fact:condition".into(),
            supporting_fact_ids: vec!["fact:action".into(), "fact:condition".into()],
            contradicting_fact_ids: vec!["fact:contradiction".into()],
            relationship_ids: vec![
                "relationship:contradicts".into(),
                "relationship:controls".into(),
                "relationship:unresolved".into(),
            ],
            evidence_span_ids: vec![
                "span:action".into(),
                "span:condition".into(),
                "span:contradiction".into(),
                "span:unresolved".into(),
            ],
            unresolved_fact_ids: vec!["fact:unresolved".into()],
            unresolved_reasons: vec!["unresolved_reference".into()],
            confidence: ArchaeologyConfidence::Low,
            caveats: vec![
                "packet has contradicting evidence".into(),
                "packet has unresolved relationships".into(),
            ],
        };
        packet.packet_id = expected_packet_id("repository:one", REVISION, &packet);
        (
            packet,
            vec![
                fact(
                    "fact:condition",
                    ArchaeologyFactKind::Predicate,
                    "Positive payment",
                ),
                fact(
                    "fact:action",
                    ArchaeologyFactKind::Mutation,
                    "Schedule payment",
                ),
                fact(
                    "fact:contradiction",
                    ArchaeologyFactKind::Predicate,
                    "Non-positive payment is allowed",
                ),
                fact(
                    "fact:unresolved",
                    ArchaeologyFactKind::Unresolved,
                    "Unresolved downstream routine",
                ),
            ],
            vec![
                ArchaeologyFactEdge {
                    edge_id: "relationship:controls".into(),
                    from_fact_id: "fact:condition".into(),
                    to_fact_id: "fact:action".into(),
                    kind: ArchaeologyFactEdgeKind::Controls,
                    trust: ArchaeologyTrust::Extracted,
                    evidence_span_ids: vec!["span:action".into(), "span:condition".into()],
                    unresolved_reason: None,
                },
                ArchaeologyFactEdge {
                    edge_id: "relationship:contradicts".into(),
                    from_fact_id: "fact:condition".into(),
                    to_fact_id: "fact:contradiction".into(),
                    kind: ArchaeologyFactEdgeKind::Contradicts,
                    trust: ArchaeologyTrust::Deterministic,
                    evidence_span_ids: vec!["span:condition".into(), "span:contradiction".into()],
                    unresolved_reason: None,
                },
                ArchaeologyFactEdge {
                    edge_id: "relationship:unresolved".into(),
                    from_fact_id: "fact:action".into(),
                    to_fact_id: "fact:unresolved".into(),
                    kind: ArchaeologyFactEdgeKind::Unresolved,
                    trust: ArchaeologyTrust::Extracted,
                    evidence_span_ids: vec!["span:action".into(), "span:unresolved".into()],
                    unresolved_reason: Some("unresolved_reference".into()),
                },
            ],
        )
    }

    fn valid_response(request: &ArchaeologySynthesisRequest) -> ArchaeologySynthesisResponse {
        ArchaeologySynthesisResponse {
            schema_version: ARCHAEOLOGY_SYNTHESIS_SCHEMA_VERSION,
            contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID.into(),
            request_id: request.request_id.clone(),
            packet_id: request.packet.packet_id.clone(),
            clauses: vec![ArchaeologySynthesisClause {
                subject: ArchaeologySynthesisSegment {
                    text: "Payment".into(),
                    fact_ids: vec!["fact:condition".into()],
                },
                condition: Some(ArchaeologySynthesisSegment {
                    text: "the payment is positive".into(),
                    fact_ids: vec!["fact:condition".into()],
                }),
                action: ArchaeologySynthesisSegment {
                    text: "schedule the payment".into(),
                    fact_ids: vec!["fact:action".into()],
                },
                exception: None,
                quantifier: None,
                relationship_ids: vec![
                    "relationship:contradicts".into(),
                    "relationship:controls".into(),
                ],
                contradicting_fact_ids: vec!["fact:contradiction".into()],
            }],
        }
    }

    fn reidentify_request(request: &mut ArchaeologySynthesisRequest) {
        request.packet.packet_id = expected_packet_id(
            &request.repository_id,
            &request.revision_sha,
            &request.packet,
        );
        request.request_id.clear();
        request.request_id = format!(
            "sha256:{}",
            super::super::inventory::hex(
                Sha256::digest(serde_json::to_vec(&request.clone()).unwrap()).as_slice()
            )
        );
    }
}
