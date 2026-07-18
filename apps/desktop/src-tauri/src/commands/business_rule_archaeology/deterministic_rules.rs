//! Shared zero-model rule pipeline: evidence packets first, prose later.

use super::adapter::canonical_semantic_digest;
use super::contracts::{
    validate_revision_sha, ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyEvidencePacket,
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
    ArchaeologyRuleClause, ArchaeologyRuleKind, ArchaeologyRuleLifecycle, ArchaeologyRulePacket,
    ArchaeologySourceClassification, ArchaeologyTrust,
};
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::{stable_graph_id, StructuralGraphCancellation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyDeterministicLimits {
    pub max_facts: usize,
    pub max_edges: usize,
    pub max_packets: usize,
    pub max_facts_per_packet: usize,
    pub max_edges_per_packet: usize,
    pub max_examined_edges_per_packet: usize,
    pub max_spans_per_packet: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_clauses_per_rule: usize,
    pub max_clause_text_bytes: usize,
    pub max_rule_output_bytes: usize,
    pub max_cluster_members: usize,
    pub max_cluster_relations: usize,
    pub max_cluster_domains: usize,
    pub max_cluster_output_bytes: usize,
}

impl Default for ArchaeologyDeterministicLimits {
    fn default() -> Self {
        Self {
            max_facts: 100_000,
            max_edges: 100_000,
            max_packets: 100_000,
            max_facts_per_packet: 64,
            max_edges_per_packet: 128,
            max_examined_edges_per_packet: 512,
            max_spans_per_packet: 256,
            max_input_bytes: 256 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024,
            max_clauses_per_rule: 256,
            max_clause_text_bytes: 1_024,
            max_rule_output_bytes: 64 * 1024 * 1024,
            max_cluster_members: 1_024,
            max_cluster_relations: 200_000,
            max_cluster_domains: 100_000,
            max_cluster_output_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchaeologyFactOrigin {
    pub fact_id: String,
    pub source_unit_id: String,
    pub path_identity: String,
    /// Repository-invariant digest of the normalized repository-relative path
    /// and exact fact byte range. Used only for deterministic ranking; never
    /// exposed or persisted.
    pub ranking_path_identity: String,
    pub classification: ArchaeologySourceClassification,
}

pub(crate) fn derive_evidence_packets(
    repository_id: &str,
    revision_sha: &str,
    facts: &[ArchaeologyFact],
    edges: &[ArchaeologyFactEdge],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyDeterministicLimits,
) -> Result<Vec<ArchaeologyEvidencePacket>, String> {
    cancelled(cancellation)?;
    if !safe_scope_id(repository_id)
        || validate_revision_sha(revision_sha).is_err()
        || limits.max_facts_per_packet == 0
        || limits.max_examined_edges_per_packet == 0
    {
        return Err("Archaeology packet scope is invalid".into());
    }
    if facts.len() > limits.max_facts || edges.len() > limits.max_edges {
        return Err("Archaeology packet input bound exceeded".into());
    }
    if packet_input_bytes(facts, edges, cancellation)? > limits.max_input_bytes {
        return Err("Archaeology packet input byte bound exceeded".into());
    }
    let mut by_id = BTreeMap::new();
    for (index, fact) in facts.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&fact.fact_id)
            || !safe_id(&fact.parser_id)
            || !deterministic_source_trust(&fact.trust)
            || fact.span_ids.is_empty()
            || fact.span_ids.iter().any(|id| !safe_id(id))
            || !valid_fact_semantic_expression(fact)
            || by_id.insert(fact.fact_id.as_str(), fact).is_some()
        {
            return Err("Archaeology packets require unique cited facts".into());
        }
    }
    let mut ordered_edges = edges.iter().collect::<Vec<_>>();
    ordered_edges.sort_by_key(|edge| edge.edge_id.as_str());
    let mut edges_by_id = BTreeMap::new();
    for (index, edge) in ordered_edges.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&edge.edge_id)
            || !deterministic_source_trust(&edge.trust)
            || edge.evidence_span_ids.is_empty()
            || edge.evidence_span_ids.iter().any(|id| !safe_id(id))
            || !by_id.contains_key(edge.from_fact_id.as_str())
            || !by_id.contains_key(edge.to_fact_id.as_str())
            || (edge.kind == ArchaeologyFactEdgeKind::Contradicts
                && edge.from_fact_id == edge.to_fact_id)
            || !edge_evidence_matches_endpoints(edge, &by_id)
            || edges_by_id.insert(edge.edge_id.as_str(), *edge).is_some()
        {
            return Err("Archaeology packets require unique cited relationships".into());
        }
    }
    let mut outgoing = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    let mut outgoing_contradiction = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    let mut reverse_control = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    let mut reverse_contradiction = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    for edge in &ordered_edges {
        if edge.kind == ArchaeologyFactEdgeKind::Contradicts {
            outgoing_contradiction
                .entry(edge.from_fact_id.as_str())
                .or_default()
                .push(edge);
        } else if !matches!(
            edge.kind,
            ArchaeologyFactEdgeKind::Aliases | ArchaeologyFactEdgeKind::Supports
        ) {
            outgoing
                .entry(edge.from_fact_id.as_str())
                .or_default()
                .push(edge);
        }
        if reverse_at_anchor(&edge.kind) {
            reverse_control
                .entry(edge.to_fact_id.as_str())
                .or_default()
                .push(edge);
        }
        if edge.kind == ArchaeologyFactEdgeKind::Contradicts {
            reverse_contradiction
                .entry(edge.to_fact_id.as_str())
                .or_default()
                .push(edge);
        }
    }
    let anchors = facts
        .iter()
        .filter(|fact| is_anchor(fact))
        .collect::<Vec<_>>();
    if anchors.len() > limits.max_packets {
        return Err("Archaeology packet count bound exceeded".into());
    }
    let mut packets = Vec::with_capacity(anchors.len());
    let mut output_bytes = 2usize;
    for anchor in anchors {
        cancelled(cancellation)?;
        let packet = packet_for_anchor(
            repository_id,
            revision_sha,
            anchor,
            &by_id,
            &edges_by_id,
            &outgoing,
            &outgoing_contradiction,
            &reverse_control,
            &reverse_contradiction,
            limits,
        )?;
        output_bytes = output_bytes
            .saturating_add(
                serde_json::to_vec(&packet)
                    .map_err(|_| "Archaeology packet is not serializable")?
                    .len(),
            )
            .saturating_add(1);
        if output_bytes > limits.max_output_bytes {
            return Err("Archaeology packet output byte bound exceeded".into());
        }
        packets.push(packet);
    }
    packets.sort_by(|left, right| left.packet_id.cmp(&right.packet_id));
    cancelled(cancellation)?;
    Ok(packets)
}

pub(crate) fn render_template_rules(
    repository_id: &str,
    generation_id: &str,
    revision_sha: &str,
    packets: &[ArchaeologyEvidencePacket],
    facts: &[ArchaeologyFact],
    edges: &[ArchaeologyFactEdge],
    coverage: &ArchaeologyCoverage,
    parser_identity: &str,
    algorithm_identity: &str,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyDeterministicLimits,
) -> Result<Vec<ArchaeologyRulePacket>, String> {
    cancelled(cancellation)?;
    if !safe_scope_id(repository_id)
        || !safe_scope_id(generation_id)
        || !safe_scope_id(parser_identity)
        || !safe_scope_id(algorithm_identity)
        || validate_revision_sha(revision_sha).is_err()
        || packets.len() > limits.max_packets
        || limits.max_clauses_per_rule == 0
        || limits.max_clause_text_bytes == 0
        || limits.max_rule_output_bytes == 0
        || coverage.reasons.len() > 32
        || coverage
            .reasons
            .iter()
            .any(|reason| reason.len() > 512 || unsafe_text(reason))
    {
        return Err("Archaeology template rule scope or bounds are invalid".into());
    }
    if facts.len() > limits.max_facts
        || edges.len() > limits.max_edges
        || packet_input_bytes(facts, edges, cancellation)?.saturating_add(
            evidence_packet_input_bytes(packets, coverage, cancellation)?,
        ) > limits.max_input_bytes
    {
        return Err("Archaeology template rule input bound exceeded".into());
    }
    let mut facts_by_id = BTreeMap::new();
    let mut known_spans = BTreeSet::new();
    for (index, fact) in facts.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&fact.fact_id)
            || !safe_id(&fact.parser_id)
            || !deterministic_source_trust(&fact.trust)
            || fact.span_ids.is_empty()
            || fact.span_ids.iter().any(|id| !safe_id(id))
            || facts_by_id.insert(fact.fact_id.as_str(), fact).is_some()
        {
            return Err("Archaeology template rules require unique cited facts".into());
        }
        known_spans.extend(fact.span_ids.iter().map(String::as_str));
    }
    let mut edges_by_id = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&edge.edge_id)
            || !deterministic_source_trust(&edge.trust)
            || edge.evidence_span_ids.is_empty()
            || edge.evidence_span_ids.iter().any(|id| !safe_id(id))
            || !edge_evidence_matches_endpoints(edge, &facts_by_id)
            || edges_by_id.insert(edge.edge_id.as_str(), edge).is_some()
        {
            return Err("Archaeology template rules require exact cited relationships".into());
        }
    }
    let mut packet_ids = BTreeSet::new();
    let mut rules = Vec::with_capacity(packets.len());
    let mut output_bytes = 2usize;
    for packet in packets {
        cancelled(cancellation)?;
        if packet.packet_id != expected_packet_id(repository_id, revision_sha, packet)
            || !packet_ids.insert(packet.packet_id.as_str())
            || packet.supporting_fact_ids.is_empty()
            || !packet.supporting_fact_ids.contains(&packet.anchor_fact_id)
            || packet.supporting_fact_ids.len() > limits.max_facts_per_packet
            || packet.relationship_ids.len() > limits.max_edges_per_packet
            || packet.evidence_span_ids.len() > limits.max_spans_per_packet
            || !packet_ids_are_known(packet, &facts_by_id, &edges_by_id, &known_spans)
            || !packet_metadata_is_categorical(packet)
        {
            return Err("Archaeology template packet is invalid".into());
        }
        let anchor = facts_by_id[packet.anchor_fact_id.as_str()];
        let rule_id = expected_rule_id(packet);
        let mut clauses = vec![anchor_clause(&rule_id, packet, anchor, limits)?];
        let mut relationship_ids = packet.relationship_ids.clone();
        relationship_ids.sort();
        for (index, relationship_id) in relationship_ids.iter().enumerate() {
            if index % 128 == 0 {
                cancelled(cancellation)?;
            }
            let edge = edges_by_id[relationship_id.as_str()];
            if edge.kind == ArchaeologyFactEdgeKind::Contradicts {
                if let Some(clause) =
                    contradiction_clause(&rule_id, packet, edge, &facts_by_id, limits)?
                {
                    merge_rendered_clause(&mut clauses, clause)?;
                }
                continue;
            }
            if let Some(clause) = relationship_clause(&rule_id, packet, edge, &facts_by_id, limits)?
            {
                merge_rendered_clause(&mut clauses, clause)?;
            }
            if clauses.len() > limits.max_clauses_per_rule {
                return Err("Archaeology template clause count bound exceeded".into());
            }
        }
        let title = bounded_text(
            &format!(
                "{} candidate: {}",
                rule_kind_name(&packet.kind),
                display_fact(anchor)
            ),
            limits.max_clause_text_bytes,
        )?;
        let rule = ArchaeologyRulePacket {
            rule_id,
            repository_id: repository_id.into(),
            generation_id: generation_id.into(),
            revision_sha: revision_sha.into(),
            kind: packet.kind.clone(),
            title,
            domain_ids: vec![],
            lifecycle: ArchaeologyRuleLifecycle::Candidate,
            trust: ArchaeologyTrust::Deterministic,
            confidence: packet.confidence.clone(),
            clauses,
            dependency_rule_ids: vec![],
            conflict_rule_ids: vec![],
            alias_rule_ids: vec![],
            coverage: coverage.clone(),
            parser_identity: parser_identity.into(),
            algorithm_identity: algorithm_identity.into(),
            synthesis_identity: None,
        };
        rule.validate()?;
        output_bytes = output_bytes
            .saturating_add(
                serde_json::to_vec(&rule)
                    .map_err(|_| "Archaeology template rule is not serializable")?
                    .len(),
            )
            .saturating_add(1);
        if output_bytes > limits.max_rule_output_bytes {
            return Err("Archaeology template rule output byte bound exceeded".into());
        }
        rules.push(rule);
    }
    rules.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    cancelled(cancellation)?;
    Ok(rules)
}

/// Multiple parser relationships can encode the same human-readable claim.
/// Keep one clause for that claim while retaining the complete, exact evidence
/// union; the final catalog intentionally forbids duplicate text per rule.
fn merge_rendered_clause(
    clauses: &mut Vec<ArchaeologyRuleClause>,
    incoming: ArchaeologyRuleClause,
) -> Result<(), String> {
    let Some(existing) = clauses
        .iter_mut()
        .find(|clause| clause.text == incoming.text)
    else {
        clauses.push(incoming);
        return Ok(());
    };
    if existing.trust != incoming.trust {
        return Err("Archaeology duplicate rendered clauses have incompatible trust".into());
    }
    existing.confidence = conservative_confidence(&existing.confidence, &incoming.confidence);
    existing
        .supporting_fact_ids
        .extend(incoming.supporting_fact_ids);
    existing
        .contradicting_fact_ids
        .extend(incoming.contradicting_fact_ids);
    existing
        .evidence_span_ids
        .extend(incoming.evidence_span_ids);
    existing.caveats.extend(incoming.caveats);
    existing.supporting_fact_ids.sort();
    existing.supporting_fact_ids.dedup();
    existing.contradicting_fact_ids.sort();
    existing.contradicting_fact_ids.dedup();
    existing.evidence_span_ids.sort();
    existing.evidence_span_ids.dedup();
    existing.caveats.sort();
    existing.caveats.dedup();
    if existing
        .supporting_fact_ids
        .iter()
        .any(|id| existing.contradicting_fact_ids.binary_search(id).is_ok())
    {
        return Err("Archaeology duplicate rendered clauses have conflicting evidence".into());
    }
    Ok(())
}

fn conservative_confidence(
    left: &ArchaeologyConfidence,
    right: &ArchaeologyConfidence,
) -> ArchaeologyConfidence {
    match (left, right) {
        (ArchaeologyConfidence::Unavailable, _) | (_, ArchaeologyConfidence::Unavailable) => {
            ArchaeologyConfidence::Unavailable
        }
        (ArchaeologyConfidence::Low, _) | (_, ArchaeologyConfidence::Low) => {
            ArchaeologyConfidence::Low
        }
        (ArchaeologyConfidence::Medium, _) | (_, ArchaeologyConfidence::Medium) => {
            ArchaeologyConfidence::Medium
        }
        (ArchaeologyConfidence::High, ArchaeologyConfidence::High) => ArchaeologyConfidence::High,
    }
}

/// Deterministically annotate evidence-compatible rules without merging away
/// any member's exact clauses or citations.
pub(crate) fn cluster_evidence_compatible_rules(
    repository_id: &str,
    revision_sha: &str,
    rules: &[ArchaeologyRulePacket],
    facts: &[ArchaeologyFact],
    edges: &[ArchaeologyFactEdge],
    origins: &[ArchaeologyFactOrigin],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyDeterministicLimits,
) -> Result<Vec<ArchaeologyRulePacket>, String> {
    cancelled(cancellation)?;
    if !safe_scope_id(repository_id)
        || validate_revision_sha(revision_sha).is_err()
        || rules.len() > limits.max_packets
        || facts.len() > limits.max_facts
        || edges.len() > limits.max_edges
        || origins.len() > limits.max_facts
        || limits.max_input_bytes == 0
        || limits.max_clauses_per_rule == 0
        || limits.max_clause_text_bytes == 0
        || limits.max_facts_per_packet == 0
        || limits.max_examined_edges_per_packet == 0
        || limits.max_spans_per_packet == 0
        || limits.max_cluster_members == 0
        || limits.max_cluster_relations == 0
        || limits.max_cluster_domains == 0
        || limits.max_cluster_output_bytes == 0
    {
        return Err("Archaeology rule clustering scope or bounds are invalid".into());
    }
    for (index, rule) in rules.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        if rule.clauses.len() > limits.max_clauses_per_rule
            || rule.clauses.iter().any(|clause| {
                clause.text.len() > limits.max_clause_text_bytes
                    || clause
                        .supporting_fact_ids
                        .len()
                        .saturating_add(clause.contradicting_fact_ids.len())
                        > limits.max_facts_per_packet
                    || clause.evidence_span_ids.len() > limits.max_spans_per_packet
            })
        {
            return Err("Archaeology rule clustering rule input bound exceeded".into());
        }
    }
    if cluster_input_bytes(facts, edges, origins, rules, cancellation)? > limits.max_input_bytes {
        return Err("Archaeology rule clustering input byte bound exceeded".into());
    }
    let mut facts_by_id = BTreeMap::new();
    let mut known_spans = BTreeSet::new();
    for (index, fact) in facts.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&fact.fact_id)
            || !safe_id(&fact.parser_id)
            || !deterministic_source_trust(&fact.trust)
            || fact.span_ids.is_empty()
            || fact.span_ids.iter().any(|id| !safe_id(id))
            || !valid_fact_semantic_expression(fact)
            || !fact.label.bytes().any(|byte| byte.is_ascii_alphanumeric())
            || fact.attributes.iter().any(|attribute| {
                matches!(
                    attribute.key.as_str(),
                    "symbol"
                        | "target"
                        | "operation"
                        | "reads"
                        | "writes"
                        | "controls"
                        | "semantic_expr"
                ) && !attribute
                    .value
                    .bytes()
                    .any(|byte| byte.is_ascii_alphanumeric())
            })
            || cluster_fact_contains_secret(fact)
            || facts_by_id.insert(fact.fact_id.as_str(), fact).is_some()
        {
            return Err("Archaeology rule clustering facts are invalid".into());
        }
        known_spans.extend(fact.span_ids.iter().map(String::as_str));
    }
    let mut origins_by_fact = BTreeMap::new();
    for (index, origin) in origins.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        if !facts_by_id.contains_key(origin.fact_id.as_str())
            || !safe_scope_id(&origin.source_unit_id)
            || !safe_scope_id(&origin.path_identity)
            || !safe_scope_id(&origin.ranking_path_identity)
            || !matches!(
                origin.classification,
                ArchaeologySourceClassification::Source
                    | ArchaeologySourceClassification::Generated
                    | ArchaeologySourceClassification::Vendor
            )
            || origins_by_fact
                .insert(origin.fact_id.as_str(), origin)
                .is_some()
        {
            return Err("Archaeology rule clustering origins are invalid or private".into());
        }
    }
    if origins_by_fact.len() != facts_by_id.len() {
        return Err("Archaeology rule clustering requires one origin per fact".into());
    }
    let mut edges_by_id = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        if !safe_id(&edge.edge_id)
            || !deterministic_source_trust(&edge.trust)
            || edge
                .unresolved_reason
                .as_deref()
                .is_some_and(cluster_private_text)
            || !edge_evidence_matches_endpoints(edge, &facts_by_id)
            || edges_by_id.insert(edge.edge_id.as_str(), edge).is_some()
        {
            return Err("Archaeology rule clustering relationships are invalid".into());
        }
    }
    let mut edges_by_fact = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    for edge in edges_by_id.values() {
        edges_by_fact
            .entry(edge.from_fact_id.as_str())
            .or_default()
            .push(edge);
        if edge.to_fact_id != edge.from_fact_id {
            edges_by_fact
                .entry(edge.to_fact_id.as_str())
                .or_default()
                .push(edge);
        }
    }

    let mut clustered = rules.to_vec();
    clustered.sort_by(|left, right| left.rule_id.cmp(&right.rule_id));
    let scope = clustered.first().map(|rule| {
        (
            rule.generation_id.clone(),
            rule.parser_identity.clone(),
            rule.algorithm_identity.clone(),
            rule.coverage.clone(),
        )
    });
    let mut rule_ids = BTreeSet::new();
    let mut keys = BTreeMap::<String, Vec<usize>>::new();
    for (index, rule) in clustered.iter_mut().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        rule.validate()?;
        let invalid_scope = rule.repository_id != repository_id
            || rule.revision_sha != revision_sha
            || !safe_scope_id(&rule.rule_id)
            || !safe_scope_id(&rule.generation_id)
            || !safe_scope_id(&rule.parser_identity)
            || !safe_scope_id(&rule.algorithm_identity)
            || scope
                .as_ref()
                .is_some_and(|(generation, parser, algorithm, coverage)| {
                    rule.generation_id != *generation
                        || rule.parser_identity != *parser
                        || rule.algorithm_identity != *algorithm
                        || rule.coverage != *coverage
                })
            || rule.title.trim().is_empty()
            || cluster_rule_has_private_text(rule)
            || !rule.dependency_rule_ids.is_empty()
            || !rule.domain_ids.is_empty()
            || !rule.alias_rule_ids.is_empty()
            || !rule.conflict_rule_ids.is_empty()
            || rule.confidence == ArchaeologyConfidence::Unavailable
            || rule.clauses.len() > limits.max_clauses_per_rule
            || rule.clauses.iter().any(|clause| {
                !safe_scope_id(&clause.clause_id)
                    || clause.text.len() > limits.max_clause_text_bytes
                    || clause
                        .evidence_span_ids
                        .iter()
                        .any(|id| !safe_id(id) || !known_spans.contains(id.as_str()))
                    || clause
                        .supporting_fact_ids
                        .iter()
                        .chain(&clause.contradicting_fact_ids)
                        .any(|id| !safe_id(id))
                    || clause.confidence == ArchaeologyConfidence::Unavailable
                    || !clause_evidence_is_exact(clause, &facts_by_id)
                    || clause
                        .supporting_fact_ids
                        .len()
                        .saturating_add(clause.contradicting_fact_ids.len())
                        > limits.max_facts_per_packet
                    || clause.evidence_span_ids.len() > limits.max_spans_per_packet
            })
            || !rule_ids.insert(rule.rule_id.clone());
        if invalid_scope {
            return Err("Archaeology rule clustering rule scope is invalid".into());
        }
        remove_generated_only_caveat(rule);
        for clause in &mut rule.clauses {
            clause.supporting_fact_ids.sort();
            clause.contradicting_fact_ids.sort();
            clause.evidence_span_ids.sort();
            clause.caveats.sort();
        }
        let key = compatibility_key(
            rule,
            &facts_by_id,
            &edges_by_fact,
            cancellation,
            limits.max_examined_edges_per_packet,
        )?;
        let members = keys.entry(key).or_default();
        if members.len() == limits.max_cluster_members {
            return Err("Archaeology rule cluster member bound exceeded".into());
        }
        members.push(index);
    }

    let mut primary_indices = BTreeSet::new();
    let mut member_primary = vec![usize::MAX; clustered.len()];
    let mut relation_count = 0usize;
    for members in keys.values() {
        cancelled(cancellation)?;
        let primary = *members
            .iter()
            .min_by_key(|index| primary_rank(&clustered[**index], &facts_by_id, &origins_by_fact))
            .ok_or("Archaeology rule cluster is empty")?;
        primary_indices.insert(primary);
        let primary_id = clustered[primary].rule_id.clone();
        let generated_only = members.iter().all(|index| {
            rule_fact_ids(&clustered[*index]).iter().all(|id| {
                !matches!(
                    origins_by_fact[*id].classification,
                    ArchaeologySourceClassification::Source
                )
            })
        });
        for index in members {
            cancelled(cancellation)?;
            member_primary[*index] = primary;
            if *index == primary {
                clustered[*index].domain_ids = vec!["domain:other".into()];
            } else {
                clustered[*index].alias_rule_ids = vec![primary_id.clone()];
                relation_count = relation_count.saturating_add(1);
            }
            if generated_only {
                clustered[*index].confidence = ArchaeologyConfidence::Low;
                for clause in &mut clustered[*index].clauses {
                    clause.confidence = ArchaeologyConfidence::Low;
                }
                let caveat = "cluster contains only generated or vendor evidence".to_string();
                if !clustered[*index].clauses[0].caveats.contains(&caveat) {
                    clustered[*index].clauses[0].caveats.push(caveat);
                    clustered[*index].clauses[0].caveats.sort();
                }
            }
        }
    }
    if primary_indices.len() > limits.max_cluster_domains {
        return Err("Archaeology rule cluster domain bound exceeded".into());
    }

    let mut supporting_primaries = BTreeMap::<String, BTreeSet<usize>>::new();
    for (index, rule) in clustered.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        for id in supporting_rule_fact_ids(rule) {
            supporting_primaries
                .entry(fact_fingerprint(facts_by_id[id]))
                .or_default()
                .insert(member_primary[index]);
        }
    }
    let mut conflicts = BTreeSet::new();
    for (index, rule) in clustered.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        let primary = member_primary[index];
        for id in contradicting_rule_fact_ids(rule) {
            let fingerprint = fact_fingerprint(facts_by_id[id]);
            for other in supporting_primaries.get(&fingerprint).into_iter().flatten() {
                if primary != *other {
                    conflicts.insert((primary.min(*other), primary.max(*other)));
                }
            }
        }
    }
    relation_count = relation_count.saturating_add(conflicts.len().saturating_mul(2));
    if relation_count > limits.max_cluster_relations {
        return Err("Archaeology rule cluster relation bound exceeded".into());
    }
    for (left, right) in conflicts {
        let left_id = clustered[left].rule_id.clone();
        let right_id = clustered[right].rule_id.clone();
        clustered[left].conflict_rule_ids.push(right_id);
        clustered[right].conflict_rule_ids.push(left_id);
    }
    for rule in &mut clustered {
        rule.conflict_rule_ids.sort();
        rule.conflict_rule_ids.dedup();
    }
    reconcile_duplicate_canonical_occurrences(&mut clustered, &facts_by_id, limits)?;
    for rule in &mut clustered {
        rule.clauses
            .sort_by_key(|clause| canonical_clause_semantic_rank(clause, &facts_by_id));
    }
    cancelled(cancellation)?;
    if serde_json::to_vec(&clustered)
        .map_err(|_| "Archaeology clustered rules are not serializable")?
        .len()
        > limits.max_cluster_output_bytes
    {
        return Err("Archaeology rule cluster output byte bound exceeded".into());
    }
    cancelled(cancellation)?;
    Ok(clustered)
}

/// Rules with the same kind and normalized supporting semantics have one
/// stable identity even when exact fact occurrences, contradiction evidence,
/// or clause partitioning differ.
/// Consolidate those prose-only occurrences before persistence so lifecycle
/// projection sees one canonical rule while retaining every unique clause and
/// citation. Existing aliases and conflict references are deterministically
/// reparented to the repository-invariant semantic primary.
fn reconcile_duplicate_canonical_occurrences(
    rules: &mut Vec<ArchaeologyRulePacket>,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    limits: ArchaeologyDeterministicLimits,
) -> Result<(), String> {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, rule) in rules.iter().enumerate() {
        if !rule.alias_rule_ids.is_empty() {
            continue;
        }
        let supporting = supporting_rule_fact_ids(rule)
            .into_iter()
            .map(|id| stable_fact_semantic_key(facts[id]))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let key = serde_json::to_string(&(rule_kind_name(&rule.kind), supporting))
            .map_err(|_| "Archaeology canonical occurrence key is not serializable")?;
        groups.entry(key).or_default().push(index);
    }

    let mut replacements = BTreeMap::<String, String>::new();
    let mut removed = BTreeSet::<usize>::new();
    for members in groups.values().filter(|members| members.len() > 1) {
        let primary = *members
            .iter()
            .min_by_key(|index| canonical_rule_semantic_rank(&rules[**index], facts))
            .ok_or("Archaeology canonical occurrence group is empty")?;
        let primary_id = rules[primary].rule_id.clone();
        for secondary in members.iter().copied().filter(|index| *index != primary) {
            let secondary_rule = rules[secondary].clone();
            replacements.insert(secondary_rule.rule_id.clone(), primary_id.clone());
            removed.insert(secondary);
            rules[primary].confidence =
                conservative_confidence(&rules[primary].confidence, &secondary_rule.confidence);
            for clause in secondary_rule.clauses {
                merge_rendered_clause(&mut rules[primary].clauses, clause)?;
            }
            rules[primary]
                .conflict_rule_ids
                .extend(secondary_rule.conflict_rule_ids);
            rules[primary]
                .dependency_rule_ids
                .extend(secondary_rule.dependency_rule_ids);
        }
        rules[primary]
            .clauses
            .sort_by_key(|clause| canonical_clause_semantic_rank(clause, facts));
        if rules[primary].clauses.len() > limits.max_clauses_per_rule
            || rules[primary].clauses.iter().any(|clause| {
                clause
                    .supporting_fact_ids
                    .len()
                    .saturating_add(clause.contradicting_fact_ids.len())
                    > limits.max_facts_per_packet
                    || clause.evidence_span_ids.len() > limits.max_spans_per_packet
            })
        {
            return Err("Archaeology canonical occurrence merge bound exceeded".into());
        }
    }

    if replacements.is_empty() {
        return Ok(());
    }
    for rule in rules.iter_mut() {
        for id in rule
            .alias_rule_ids
            .iter_mut()
            .chain(&mut rule.conflict_rule_ids)
            .chain(&mut rule.dependency_rule_ids)
        {
            if let Some(replacement) = replacements.get(id) {
                *id = replacement.clone();
            }
        }
        rule.alias_rule_ids.sort();
        rule.alias_rule_ids.dedup();
        rule.conflict_rule_ids.retain(|id| id != &rule.rule_id);
        rule.conflict_rule_ids.sort();
        rule.conflict_rule_ids.dedup();
        rule.dependency_rule_ids.retain(|id| id != &rule.rule_id);
        rule.dependency_rule_ids.sort();
        rule.dependency_rule_ids.dedup();
    }
    let mut index = 0usize;
    rules.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    for rule in rules.iter() {
        rule.validate()?;
    }
    Ok(())
}

fn stable_fact_semantic_key(fact: &ArchaeologyFact) -> Result<(String, String), String> {
    let mut expressions = fact
        .attributes
        .iter()
        .filter(|attribute| attribute.key == "semantic_expr");
    let expression = expressions
        .next()
        .ok_or("Archaeology canonical occurrence fact lacks semantic identity")?;
    if expressions.next().is_some() || !canonical_semantic_digest(&expression.value) {
        return Err("Archaeology canonical occurrence fact has invalid semantic identity".into());
    }
    Ok((format!("{:?}", fact.kind), expression.value.clone()))
}

fn compatibility_key(
    rule: &ArchaeologyRulePacket,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    edges: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>,
    cancellation: &StructuralGraphCancellation,
    max_examined_edges: usize,
) -> Result<String, String> {
    if rule.trust != ArchaeologyTrust::Deterministic
        || rule.lifecycle != ArchaeologyRuleLifecycle::Candidate
        || rule.synthesis_identity.is_some()
    {
        return Err("Archaeology rule clustering requires deterministic candidates".into());
    }
    let mut clauses = Vec::with_capacity(rule.clauses.len());
    for clause in &rule.clauses {
        cancelled(cancellation)?;
        if clause.trust != ArchaeologyTrust::Deterministic {
            return Err("Archaeology rule clustering clause trust is invalid".into());
        }
        let supporting = exact_fact_fingerprints(&clause.supporting_fact_ids, facts)?;
        let contradicting = exact_fact_fingerprints(&clause.contradicting_fact_ids, facts)?;
        let referenced = clause
            .supporting_fact_ids
            .iter()
            .chain(&clause.contradicting_fact_ids)
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut seen_edges = BTreeSet::new();
        let mut relationships = Vec::new();
        let mut examined_edges = 0usize;
        for fact_id in &referenced {
            cancelled(cancellation)?;
            for edge in edges.get(fact_id).into_iter().flatten() {
                cancelled(cancellation)?;
                if examined_edges == max_examined_edges {
                    return Err("Archaeology rule cluster relationship bound exceeded".into());
                }
                examined_edges += 1;
                if referenced.contains(edge.from_fact_id.as_str())
                    && referenced.contains(edge.to_fact_id.as_str())
                    && seen_edges.insert(edge.edge_id.as_str())
                {
                    relationships.push(format!(
                        "{:?}\0{}\0{}",
                        edge.kind,
                        fact_fingerprint(facts[edge.from_fact_id.as_str()]),
                        fact_fingerprint(facts[edge.to_fact_id.as_str()])
                    ));
                }
            }
        }
        relationships.sort();
        let mut caveats = clause
            .caveats
            .iter()
            .filter(|value| value.as_str() != "cluster contains only generated or vendor evidence")
            .map(|value| categorical_cluster_caveat(value).map(str::to_string))
            .collect::<Result<Vec<_>, _>>()?;
        caveats.sort();
        clauses.push(
            serde_json::to_string(&(supporting, contradicting, relationships, caveats))
                .map_err(|_| "Archaeology rule cluster signature is not serializable")?,
        );
    }
    clauses.sort();
    serde_json::to_string(&(format!("{:?}", rule.kind), clauses))
        .map_err(|_| "Archaeology rule cluster key is not serializable".into())
}

fn exact_fact_fingerprints(
    ids: &[String],
    facts: &BTreeMap<&str, &ArchaeologyFact>,
) -> Result<Vec<String>, String> {
    let unique = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if unique.len() != ids.len() || unique.iter().any(|id| !facts.contains_key(id)) {
        return Err("Archaeology rule clustering cites unknown or duplicate facts".into());
    }
    let mut values = unique
        .into_iter()
        .map(|id| fact_fingerprint(facts[id]))
        .collect::<Vec<_>>();
    values.sort();
    Ok(values)
}

fn fact_fingerprint(fact: &ArchaeologyFact) -> String {
    let mut attributes = fact
        .attributes
        .iter()
        .filter(|attribute| {
            matches!(
                attribute.key.as_str(),
                "symbol"
                    | "target"
                    | "operation"
                    | "reads"
                    | "writes"
                    | "controls"
                    | "semantic_expr"
            )
        })
        .map(|attribute| {
            format!(
                "{}={}",
                attribute.key,
                normalized_semantic_text(&attribute.value)
            )
        })
        .collect::<Vec<_>>();
    attributes.sort();
    format!(
        "{:?}\0{}\0{}",
        fact.kind,
        normalized_semantic_text(&fact.label),
        attributes.join("\0")
    )
}

fn normalized_semantic_text(value: &str) -> String {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_ascii_alphanumeric() {
            word.push(character.to_ascii_lowercase());
            continue;
        }
        if !word.is_empty() {
            tokens.push(std::mem::take(&mut word));
        }
        if matches!(
            character,
            '<' | '>' | '=' | '!' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '~'
        ) {
            let mut operator = character.to_string();
            if (matches!(character, '<' | '>' | '=' | '!')
                && characters.peek().is_some_and(|next| *next == '='))
                || (matches!(character, '&' | '|')
                    && characters.peek().is_some_and(|next| *next == character))
            {
                operator.push(characters.next().unwrap_or(character));
            }
            tokens.push(operator);
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    tokens.join("_")
}

fn categorical_cluster_caveat(value: &str) -> Result<&str, String> {
    match value {
        "kind is identifier-derived and requires review"
        | "packet has unresolved relationships"
        | "packet has contradicting evidence"
        | "packet relationship bound was truncated"
        | "relationship target is unresolved" => Ok(value),
        _ => Err("Archaeology rule clustering caveat is not categorical".into()),
    }
}

fn primary_rank(
    rule: &ArchaeologyRulePacket,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    origins: &BTreeMap<&str, &ArchaeologyFactOrigin>,
) -> (usize, usize, usize, u8, usize, String, String) {
    let fact_ids = rule_fact_ids(rule);
    let non_source_facts = fact_ids
        .iter()
        .filter(|id| {
            !matches!(
                origins[**id].classification,
                ArchaeologySourceClassification::Source
            )
        })
        .count();
    let unresolved = fact_ids
        .iter()
        .filter(|id| facts[**id].kind == ArchaeologyFactKind::Unresolved)
        .count()
        .saturating_add(
            rule.clauses
                .iter()
                .flat_map(|clause| &clause.caveats)
                .filter(|value| {
                    matches!(
                        value.as_str(),
                        "packet has unresolved relationships" | "relationship target is unresolved"
                    )
                })
                .count(),
        );
    let contradictions = contradicting_rule_fact_ids(rule).len();
    let confidence = match rule.confidence {
        ArchaeologyConfidence::High => 0,
        ArchaeologyConfidence::Medium => 1,
        ArchaeologyConfidence::Low => 2,
        ArchaeologyConfidence::Unavailable => 3,
    };
    let source_units = fact_ids
        .iter()
        .map(|id| origins[id].source_unit_id.as_str())
        .collect::<BTreeSet<_>>();
    let ranking_paths = fact_ids
        .iter()
        .map(|id| origins[id].ranking_path_identity.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\0");
    (
        non_source_facts,
        unresolved,
        contradictions,
        confidence,
        source_units.len(),
        ranking_paths,
        canonical_rule_semantic_rank(rule, facts),
    )
}

fn canonical_rule_semantic_rank(
    rule: &ArchaeologyRulePacket,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
) -> String {
    let mut clauses = rule
        .clauses
        .iter()
        .map(|clause| canonical_clause_semantic_rank(clause, facts))
        .collect::<Vec<_>>();
    clauses.sort();
    serde_json::to_string(&(
        rule_kind_name(&rule.kind),
        normalized_semantic_text(&rule.title),
        clauses,
    ))
    .expect("canonical rule semantic rank is serializable")
}

fn canonical_clause_semantic_rank(
    clause: &ArchaeologyRuleClause,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
) -> String {
    let supporting = clause
        .supporting_fact_ids
        .iter()
        .map(|id| fact_fingerprint(facts[id.as_str()]))
        .collect::<BTreeSet<_>>();
    let contradicting = clause
        .contradicting_fact_ids
        .iter()
        .map(|id| fact_fingerprint(facts[id.as_str()]))
        .collect::<BTreeSet<_>>();
    let caveats = clause
        .caveats
        .iter()
        .map(|value| normalized_semantic_text(value))
        .collect::<BTreeSet<_>>();
    serde_json::to_string(&(
        normalized_semantic_text(&clause.text),
        supporting,
        contradicting,
        caveats,
    ))
    .expect("canonical clause semantic rank is serializable")
}

fn rule_fact_ids(rule: &ArchaeologyRulePacket) -> BTreeSet<&str> {
    rule.clauses
        .iter()
        .flat_map(|clause| {
            clause
                .supporting_fact_ids
                .iter()
                .chain(&clause.contradicting_fact_ids)
        })
        .map(String::as_str)
        .collect()
}

fn clause_evidence_is_exact(
    clause: &ArchaeologyRuleClause,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
) -> bool {
    let supporting = clause
        .supporting_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let contradicting = clause
        .contradicting_fact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let evidence = clause
        .evidence_span_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = supporting
        .iter()
        .chain(&contradicting)
        .filter_map(|id| facts.get(*id))
        .flat_map(|fact| fact.span_ids.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    supporting.len() == clause.supporting_fact_ids.len()
        && contradicting.len() == clause.contradicting_fact_ids.len()
        && evidence.len() == clause.evidence_span_ids.len()
        && supporting.is_disjoint(&contradicting)
        && supporting.iter().all(|id| facts.contains_key(*id))
        && contradicting.iter().all(|id| facts.contains_key(*id))
        && evidence == expected
}

fn supporting_rule_fact_ids(rule: &ArchaeologyRulePacket) -> BTreeSet<&str> {
    rule.clauses
        .iter()
        .flat_map(|clause| clause.supporting_fact_ids.iter().map(String::as_str))
        .collect()
}

fn contradicting_rule_fact_ids(rule: &ArchaeologyRulePacket) -> BTreeSet<&str> {
    rule.clauses
        .iter()
        .flat_map(|clause| clause.contradicting_fact_ids.iter().map(String::as_str))
        .collect()
}

fn remove_generated_only_caveat(rule: &mut ArchaeologyRulePacket) {
    for clause in &mut rule.clauses {
        clause
            .caveats
            .retain(|value| value != "cluster contains only generated or vendor evidence");
    }
}

fn cluster_fact_contains_secret(fact: &ArchaeologyFact) -> bool {
    cluster_private_text(&fact.label)
        || fact.attributes.iter().any(|attribute| {
            matches!(
                attribute.key.as_str(),
                "symbol"
                    | "target"
                    | "operation"
                    | "reads"
                    | "writes"
                    | "controls"
                    | "semantic_expr"
            ) && cluster_private_text(&attribute.value)
        })
}

fn valid_fact_semantic_expression(fact: &ArchaeologyFact) -> bool {
    let mut expressions = fact
        .attributes
        .iter()
        .filter(|attribute| attribute.key == "semantic_expr");
    let required = fact.kind != ArchaeologyFactKind::Unresolved;
    let expression = expressions.next();
    (!required || expression.is_some())
        && expression.is_none_or(|attribute| canonical_semantic_digest(&attribute.value))
        && expressions.next().is_none()
}

fn cluster_rule_has_private_text(rule: &ArchaeologyRulePacket) -> bool {
    cluster_private_text(&rule.title)
        || rule
            .coverage
            .reasons
            .iter()
            .any(|value| cluster_private_text(value))
        || rule.clauses.iter().any(|clause| {
            cluster_private_text(&clause.text)
                || clause
                    .caveats
                    .iter()
                    .any(|value| cluster_private_text(value))
        })
}

fn cluster_private_text(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    value.contains('\0')
        || looks_like_secret(value)
        || value.starts_with(['/', '\\'])
        || drive
        || value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
}

fn packet_ids_are_known(
    packet: &ArchaeologyEvidencePacket,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    edges: &BTreeMap<&str, &ArchaeologyFactEdge>,
    spans: &BTreeSet<&str>,
) -> bool {
    let supporting = packet.supporting_fact_ids.iter().collect::<BTreeSet<_>>();
    let contradicting = packet
        .contradicting_fact_ids
        .iter()
        .collect::<BTreeSet<_>>();
    let unresolved = packet.unresolved_fact_ids.iter().collect::<BTreeSet<_>>();
    let relationships = packet.relationship_ids.iter().collect::<BTreeSet<_>>();
    let evidence = packet.evidence_span_ids.iter().collect::<BTreeSet<_>>();
    let all_packet_facts = supporting
        .iter()
        .chain(&contradicting)
        .chain(&unresolved)
        .map(|id| id.as_str())
        .collect::<BTreeSet<_>>();
    let selected_edges = relationships
        .iter()
        .filter_map(|id| edges.get(id.as_str()).copied())
        .collect::<Vec<_>>();
    let expected_evidence = all_packet_facts
        .iter()
        .filter_map(|id| facts.get(id).copied())
        .flat_map(|fact| fact.span_ids.iter().map(String::as_str))
        .chain(
            selected_edges
                .iter()
                .flat_map(|edge| edge.evidence_span_ids.iter().map(String::as_str)),
        )
        .collect::<BTreeSet<_>>();
    let selected_support = supporting
        .iter()
        .filter_map(|id| facts.get(id.as_str()).copied())
        .filter(|fact| {
            fact.fact_id == packet.anchor_fact_id
                || !selected_edges.iter().any(|edge| {
                    edge.kind == ArchaeologyFactEdgeKind::Contradicts
                        && (edge.from_fact_id == fact.fact_id || edge.to_fact_id == fact.fact_id)
                })
        })
        .collect::<Vec<_>>();
    let Some(anchor) = facts.get(packet.anchor_fact_id.as_str()).copied() else {
        return false;
    };
    let (classified_kind, identifier_derived) = classify(anchor, &selected_support);
    let has_identifier_caveat = packet
        .caveats
        .iter()
        .any(|value| value == "kind is identifier-derived and requires review");
    let has_unresolved_caveat = packet
        .caveats
        .iter()
        .any(|value| value == "packet has unresolved relationships");
    let has_contradiction_caveat = packet
        .caveats
        .iter()
        .any(|value| value == "packet has contradicting evidence");
    let truncated = packet
        .caveats
        .iter()
        .any(|value| value == "packet relationship bound was truncated");
    let expected_confidence = if truncated || !unresolved.is_empty() || !contradicting.is_empty() {
        ArchaeologyConfidence::Low
    } else if identifier_derived {
        ArchaeologyConfidence::Medium
    } else {
        ArchaeologyConfidence::High
    };
    supporting.len() == packet.supporting_fact_ids.len()
        && contradicting.len() == packet.contradicting_fact_ids.len()
        && unresolved.len() == packet.unresolved_fact_ids.len()
        && relationships.len() == packet.relationship_ids.len()
        && evidence.len() == packet.evidence_span_ids.len()
        && supporting.is_disjoint(&contradicting)
        && supporting.is_disjoint(&unresolved)
        && contradicting.is_disjoint(&unresolved)
        && supporting.iter().all(|id| facts.contains_key(id.as_str()))
        && contradicting
            .iter()
            .all(|id| facts.contains_key(id.as_str()))
        && unresolved.iter().all(|id| {
            facts
                .get(id.as_str())
                .is_some_and(|fact| fact.kind == ArchaeologyFactKind::Unresolved)
        })
        && packet
            .relationship_ids
            .iter()
            .all(|id| edges.contains_key(id.as_str()))
        && packet
            .evidence_span_ids
            .iter()
            .all(|id| spans.contains(id.as_str()))
        && selected_edges.iter().all(|edge| {
            all_packet_facts.contains(edge.from_fact_id.as_str())
                && all_packet_facts.contains(edge.to_fact_id.as_str())
        })
        && supporting.iter().all(|id| {
            id.as_str() == packet.anchor_fact_id
                || selected_edges
                    .iter()
                    .any(|edge| edge.from_fact_id == id.as_str() || edge.to_fact_id == id.as_str())
        })
        && contradicting.iter().all(|id| {
            selected_edges.iter().any(|edge| {
                edge.kind == ArchaeologyFactEdgeKind::Contradicts
                    && (edge.from_fact_id == id.as_str() || edge.to_fact_id == id.as_str())
            })
        })
        && unresolved.iter().all(|id| {
            selected_edges.iter().any(|edge| {
                (edge.kind == ArchaeologyFactEdgeKind::Unresolved
                    || edge.unresolved_reason.is_some())
                    && (edge.from_fact_id == id.as_str() || edge.to_fact_id == id.as_str())
            })
        })
        && expected_evidence == evidence.iter().map(|id| id.as_str()).collect()
        && classified_kind == packet.kind
        && identifier_derived == has_identifier_caveat
        && unresolved.is_empty() != has_unresolved_caveat
        && contradicting.is_empty() != has_contradiction_caveat
        && packet.unresolved_reasons.is_empty() == unresolved.is_empty()
        && packet.confidence == expected_confidence
}

pub(crate) fn packet_metadata_is_categorical(packet: &ArchaeologyEvidencePacket) -> bool {
    packet.caveats.iter().collect::<BTreeSet<_>>().len() == packet.caveats.len()
        && packet
            .unresolved_reasons
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            == packet.unresolved_reasons.len()
        && packet.caveats.iter().all(|value| {
            matches!(
                value.as_str(),
                "kind is identifier-derived and requires review"
                    | "packet has unresolved relationships"
                    | "packet has contradicting evidence"
                    | "packet relationship bound was truncated"
            )
        })
        && packet.unresolved_reasons.iter().all(|value| {
            matches!(
                value.as_str(),
                "ambiguous_reference" | "unavailable_reference" | "unresolved_reference"
            )
        })
}

fn anchor_clause(
    rule_id: &str,
    packet: &ArchaeologyEvidencePacket,
    anchor: &ArchaeologyFact,
    limits: ArchaeologyDeterministicLimits,
) -> Result<ArchaeologyRuleClause, String> {
    let evidence = anchor.span_ids.iter().cloned().collect::<BTreeSet<_>>();
    let text = bounded_text(
        &format!(
            "This {} candidate is anchored by {}.",
            rule_kind_name(&packet.kind),
            display_fact(anchor)
        ),
        limits.max_clause_text_bytes,
    )?;
    let mut caveats = packet.caveats.clone();
    caveats.sort();
    Ok(ArchaeologyRuleClause {
        clause_id: stable_graph_id("archaeology-clause", &format!("{rule_id}\0anchor")),
        text,
        trust: ArchaeologyTrust::Deterministic,
        confidence: packet.confidence.clone(),
        supporting_fact_ids: vec![anchor.fact_id.clone()],
        contradicting_fact_ids: vec![],
        evidence_span_ids: evidence.into_iter().collect(),
        caveats,
    })
}

fn contradiction_clause(
    rule_id: &str,
    packet: &ArchaeologyEvidencePacket,
    edge: &ArchaeologyFactEdge,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    limits: ArchaeologyDeterministicLimits,
) -> Result<Option<ArchaeologyRuleClause>, String> {
    let (supporting, contradicting) = if packet.supporting_fact_ids.contains(&edge.from_fact_id)
        && packet.contradicting_fact_ids.contains(&edge.to_fact_id)
    {
        (&edge.from_fact_id, &edge.to_fact_id)
    } else if packet.supporting_fact_ids.contains(&edge.to_fact_id)
        && packet.contradicting_fact_ids.contains(&edge.from_fact_id)
    {
        (&edge.to_fact_id, &edge.from_fact_id)
    } else {
        return Ok(None);
    };
    let supporting_fact = facts[supporting.as_str()];
    let contradicting_fact = facts[contradicting.as_str()];
    let mut evidence = supporting_fact
        .span_ids
        .iter()
        .chain(&contradicting_fact.span_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    if evidence.len() > limits.max_spans_per_packet {
        return Err("Archaeology contradiction clause evidence bound exceeded".into());
    }
    let text = bounded_text(
        &format!(
            "{} contradicts {}.",
            display_fact(supporting_fact),
            display_fact(contradicting_fact),
        ),
        limits.max_clause_text_bytes,
    )?;
    Ok(Some(ArchaeologyRuleClause {
        clause_id: stable_graph_id(
            "archaeology-clause",
            &format!("{rule_id}\0contradiction\0{}", edge.edge_id),
        ),
        text,
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::Low,
        supporting_fact_ids: vec![supporting.clone()],
        contradicting_fact_ids: vec![contradicting.clone()],
        evidence_span_ids: std::mem::take(&mut evidence).into_iter().collect(),
        caveats: vec!["packet has contradicting evidence".into()],
    }))
}

fn relationship_clause(
    rule_id: &str,
    packet: &ArchaeologyEvidencePacket,
    edge: &ArchaeologyFactEdge,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    limits: ArchaeologyDeterministicLimits,
) -> Result<Option<ArchaeologyRuleClause>, String> {
    let supporting = [&edge.from_fact_id, &edge.to_fact_id]
        .into_iter()
        .filter(|id| packet.supporting_fact_ids.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    if supporting.is_empty() {
        return Ok(None);
    }
    let contradicting = [&edge.from_fact_id, &edge.to_fact_id]
        .into_iter()
        .filter(|id| packet.contradicting_fact_ids.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let unresolved = [&edge.from_fact_id, &edge.to_fact_id]
        .into_iter()
        .any(|id| packet.unresolved_fact_ids.contains(id));
    let from = facts[edge.from_fact_id.as_str()];
    let to = facts[edge.to_fact_id.as_str()];
    let text = if unresolved {
        format!(
            "{} has an unresolved {} relationship.",
            display_fact(from),
            relationship_name(&edge.kind)
        )
    } else {
        format!(
            "{} {} {}.",
            display_fact(from),
            relationship_verb(&edge.kind),
            display_fact(to)
        )
    };
    Ok(Some(ArchaeologyRuleClause {
        clause_id: stable_graph_id(
            "archaeology-clause",
            &format!("{rule_id}\0relationship\0{}", edge.edge_id),
        ),
        text: bounded_text(&text, limits.max_clause_text_bytes)?,
        trust: ArchaeologyTrust::Deterministic,
        confidence: if unresolved {
            ArchaeologyConfidence::Low
        } else {
            packet.confidence.clone()
        },
        supporting_fact_ids: supporting,
        contradicting_fact_ids: contradicting,
        evidence_span_ids: edge
            .evidence_span_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        caveats: unresolved
            .then(|| "relationship target is unresolved".into())
            .into_iter()
            .collect(),
    }))
}

fn packet_for_anchor(
    repository_id: &str,
    revision_sha: &str,
    anchor: &ArchaeologyFact,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
    edges: &BTreeMap<&str, &ArchaeologyFactEdge>,
    outgoing: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>,
    outgoing_contradiction: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>,
    reverse_control: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>,
    reverse_contradiction: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>,
    limits: ArchaeologyDeterministicLimits,
) -> Result<ArchaeologyEvidencePacket, String> {
    let mut selected_facts = BTreeSet::from([anchor.fact_id.as_str()]);
    let mut selected_depth = BTreeMap::from([(anchor.fact_id.as_str(), 0usize)]);
    let mut contradiction_ids = BTreeSet::new();
    let mut contradiction_terminal_ids = BTreeSet::new();
    let mut selected_edges = BTreeSet::new();
    let mut frontier = BTreeSet::from([anchor.fact_id.as_str()]);
    let mut truncated = false;
    let mut examined_edges = 0usize;
    'depths: for depth in 0..2 {
        let mut next = BTreeSet::new();
        let selected_at_depth = selected_facts.clone();
        for current in &frontier {
            if contradiction_ids.contains(current) {
                continue;
            }
            for reverse in [false, true] {
                let adjacent = if reverse {
                    reverse_contradiction.get(current)
                } else {
                    outgoing_contradiction.get(current)
                };
                for edge in adjacent.into_iter().flatten() {
                    if examined_edges == limits.max_examined_edges_per_packet {
                        truncated = true;
                        break 'depths;
                    }
                    examined_edges += 1;
                    if selected_edges.contains(edge.edge_id.as_str()) {
                        continue;
                    }
                    if selected_edges.len() == limits.max_edges_per_packet {
                        truncated = true;
                        break 'depths;
                    }
                    selected_edges.insert(edge.edge_id.as_str());
                    for endpoint in [edge.from_fact_id.as_str(), edge.to_fact_id.as_str()] {
                        if endpoint != anchor.fact_id {
                            contradiction_terminal_ids.insert(endpoint);
                        }
                    }
                    let from_selected = selected_at_depth.contains(edge.from_fact_id.as_str());
                    let to_selected = selected_at_depth.contains(edge.to_fact_id.as_str());
                    let opposing = if from_selected && to_selected {
                        let from_depth = selected_depth
                            .get(edge.from_fact_id.as_str())
                            .copied()
                            .unwrap_or(usize::MAX);
                        let to_depth = selected_depth
                            .get(edge.to_fact_id.as_str())
                            .copied()
                            .unwrap_or(usize::MAX);
                        let contradicting = if from_depth != to_depth {
                            if from_depth > to_depth {
                                edge.from_fact_id.as_str()
                            } else {
                                edge.to_fact_id.as_str()
                            }
                        } else if fact_fingerprint(facts[edge.from_fact_id.as_str()])
                            > fact_fingerprint(facts[edge.to_fact_id.as_str()])
                        {
                            edge.from_fact_id.as_str()
                        } else {
                            edge.to_fact_id.as_str()
                        };
                        [Some(contradicting), None]
                    } else if from_selected {
                        [Some(edge.to_fact_id.as_str()), None]
                    } else {
                        [Some(edge.from_fact_id.as_str()), None]
                    };
                    for fact_id in opposing.into_iter().flatten() {
                        if fact_id != anchor.fact_id {
                            contradiction_ids.insert(fact_id);
                            selected_facts.remove(fact_id);
                        }
                    }
                }
            }
        }
        for current in frontier {
            if contradiction_terminal_ids.contains(current) {
                continue;
            }
            for reverse in [false, true] {
                if reverse && depth != 0 {
                    continue;
                }
                let adjacent = if reverse {
                    reverse_control.get(current)
                } else {
                    outgoing.get(current)
                };
                for edge in adjacent.into_iter().flatten() {
                    if examined_edges == limits.max_examined_edges_per_packet {
                        truncated = true;
                        break 'depths;
                    }
                    examined_edges += 1;
                    if selected_edges.contains(edge.edge_id.as_str()) {
                        continue;
                    }
                    let other = if reverse {
                        edge.from_fact_id.as_str()
                    } else {
                        edge.to_fact_id.as_str()
                    };
                    if selected_edges.len() == limits.max_edges_per_packet
                        || (!selected_facts.contains(other)
                            && selected_facts.len() == limits.max_facts_per_packet)
                    {
                        truncated = true;
                        break 'depths;
                    }
                    selected_edges.insert(edge.edge_id.as_str());
                    if !contradiction_ids.contains(other) && selected_facts.insert(other) {
                        selected_depth.insert(other, depth + 1);
                        next.insert(other);
                    }
                }
            }
        }
        frontier = next;
    }
    let unresolved_fact_ids = selected_facts
        .iter()
        .filter(|id| {
            facts
                .get(**id)
                .is_some_and(|fact| fact.kind == ArchaeologyFactKind::Unresolved)
        })
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let mut unresolved_reasons = selected_edges
        .iter()
        .filter_map(|id| {
            edges
                .get(id)
                .and_then(|edge| edge.unresolved_reason.as_deref())
        })
        .map(categorical_unresolved_reason)
        .collect::<BTreeSet<_>>();
    if !unresolved_fact_ids.is_empty() && unresolved_reasons.is_empty() {
        unresolved_reasons.insert("unresolved_reference".into());
    }
    let unresolved_reasons = unresolved_reasons.into_iter().collect::<Vec<_>>();
    let contradicting_fact_ids = contradiction_ids
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let supporting_fact_ids = selected_facts
        .iter()
        .filter(|id| {
            !contradiction_ids.contains(**id)
                && !unresolved_fact_ids.iter().any(|item| item == **id)
        })
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let relationship_ids = selected_edges
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let selected = supporting_fact_ids
        .iter()
        .filter_map(|id| facts.get(id.as_str()).copied())
        .filter(|fact| {
            fact.fact_id == anchor.fact_id
                || !selected_edges.iter().any(|edge_id| {
                    edges.get(edge_id).is_some_and(|edge| {
                        edge.kind == ArchaeologyFactEdgeKind::Contradicts
                            && (edge.from_fact_id == fact.fact_id
                                || edge.to_fact_id == fact.fact_id)
                    })
                })
        })
        .collect::<Vec<_>>();
    let mut evidence = BTreeSet::new();
    for fact in selected_facts
        .iter()
        .filter_map(|id| facts.get(*id).copied())
        .chain(
            contradiction_ids
                .iter()
                .filter_map(|id| facts.get(id).copied()),
        )
    {
        extend_evidence(&mut evidence, &fact.span_ids, limits.max_spans_per_packet)?;
    }
    for edge_id in &selected_edges {
        if let Some(edge) = edges.get(edge_id).copied() {
            extend_evidence(
                &mut evidence,
                &edge.evidence_span_ids,
                limits.max_spans_per_packet,
            )?;
        }
    }
    let (kind, identifier_derived) = classify(anchor, &selected);
    let mut caveats = Vec::new();
    if identifier_derived {
        caveats.push("kind is identifier-derived and requires review".into());
    }
    if !unresolved_fact_ids.is_empty() {
        caveats.push("packet has unresolved relationships".into());
    }
    if !contradicting_fact_ids.is_empty() {
        caveats.push("packet has contradicting evidence".into());
    }
    if truncated {
        caveats.push("packet relationship bound was truncated".into());
    }
    let confidence =
        if truncated || !unresolved_fact_ids.is_empty() || !contradicting_fact_ids.is_empty() {
            ArchaeologyConfidence::Low
        } else if identifier_derived {
            ArchaeologyConfidence::Medium
        } else {
            ArchaeologyConfidence::High
        };
    let mut packet = ArchaeologyEvidencePacket {
        packet_id: String::new(),
        kind,
        anchor_fact_id: anchor.fact_id.clone(),
        supporting_fact_ids,
        contradicting_fact_ids,
        relationship_ids,
        evidence_span_ids: evidence.into_iter().collect(),
        unresolved_fact_ids,
        unresolved_reasons,
        confidence,
        caveats,
    };
    packet.packet_id = expected_packet_id(repository_id, revision_sha, &packet);
    Ok(packet)
}

fn is_anchor(fact: &ArchaeologyFact) -> bool {
    matches!(
        fact.kind,
        ArchaeologyFactKind::Predicate
            | ArchaeologyFactKind::Decision
            | ArchaeologyFactKind::Calculation
            | ArchaeologyFactKind::Mutation
            | ArchaeologyFactKind::Transaction
            | ArchaeologyFactKind::ControlFlow
    )
}

fn reverse_at_anchor(kind: &ArchaeologyFactEdgeKind) -> bool {
    matches!(
        kind,
        ArchaeologyFactEdgeKind::Controls
            | ArchaeologyFactEdgeKind::Calculates
            | ArchaeologyFactEdgeKind::BranchesTo
    )
}

fn classify(
    anchor: &ArchaeologyFact,
    selected: &[&ArchaeologyFact],
) -> (ArchaeologyRuleKind, bool) {
    let identifiers = selected
        .iter()
        .flat_map(|fact| {
            std::iter::once(fact.label.as_str()).chain(
                fact.attributes
                    .iter()
                    .filter(|attribute| {
                        matches!(attribute.key.as_str(), "writes" | "target" | "symbol")
                    })
                    .map(|attribute| attribute.value.as_str()),
            )
        })
        .flat_map(identifier_tokens)
        .collect::<BTreeSet<_>>();
    let tagged = |names: &[&str]| names.iter().any(|name| identifiers.contains(*name));
    if anchor.kind == ArchaeologyFactKind::Transaction {
        return (ArchaeologyRuleKind::Transaction, false);
    }
    if anchor.kind == ArchaeologyFactKind::Calculation {
        return (ArchaeologyRuleKind::Calculation, false);
    }
    if tagged(&["eligible", "eligibility"]) {
        return (ArchaeologyRuleKind::Eligibility, true);
    }
    if tagged(&["entitle", "entitled", "entitlement"]) {
        return (ArchaeologyRuleKind::Entitlement, true);
    }
    if tagged(&["state", "status", "stage", "lifecycle", "phase"]) {
        return (ArchaeologyRuleKind::Lifecycle, true);
    }
    if anchor.kind == ArchaeologyFactKind::ControlFlow
        && tagged(&[
            "deny",
            "error",
            "exception",
            "fail",
            "failed",
            "invalid",
            "reject",
        ])
    {
        return (ArchaeologyRuleKind::Exception, true);
    }
    match anchor.kind {
        ArchaeologyFactKind::Decision | ArchaeologyFactKind::ControlFlow => {
            (ArchaeologyRuleKind::Routing, false)
        }
        ArchaeologyFactKind::Predicate => (ArchaeologyRuleKind::Validation, false),
        ArchaeologyFactKind::Mutation => (ArchaeologyRuleKind::Mutation, false),
        _ => (ArchaeologyRuleKind::Other, false),
    }
}

fn identifier_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
}

fn categorical_unresolved_reason(value: &str) -> String {
    if value.contains("ambiguous") {
        "ambiguous_reference".into()
    } else if value.contains("unavailable") || value.contains("not defined") {
        "unavailable_reference".into()
    } else {
        "unresolved_reference".into()
    }
}

fn display_fact(fact: &ArchaeologyFact) -> String {
    let kind = fact_kind_name(&fact.kind);
    if unsafe_text(&fact.label) || fact.label.contains(['/', '\\']) {
        return format!("the cited {kind}");
    }
    let normalized = fact.label.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return format!("the cited {kind}");
    }
    let label = truncate_utf8(&normalized, 160);
    format!("the cited {kind} \"{label}\"")
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

fn rule_kind_name(kind: &ArchaeologyRuleKind) -> &'static str {
    match kind {
        ArchaeologyRuleKind::Validation => "validation",
        ArchaeologyRuleKind::Calculation => "calculation",
        ArchaeologyRuleKind::Eligibility => "eligibility",
        ArchaeologyRuleKind::Entitlement => "entitlement",
        ArchaeologyRuleKind::Routing => "routing",
        ArchaeologyRuleKind::Mutation => "mutation",
        ArchaeologyRuleKind::Exception => "exception",
        ArchaeologyRuleKind::Lifecycle => "lifecycle",
        ArchaeologyRuleKind::Transaction => "transaction",
        ArchaeologyRuleKind::Other => "other",
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

fn relationship_name(kind: &ArchaeologyFactEdgeKind) -> &'static str {
    match kind {
        ArchaeologyFactEdgeKind::BeginsTransaction
        | ArchaeologyFactEdgeKind::CommitsTransaction
        | ArchaeologyFactEdgeKind::RollsBackTransaction => "transaction",
        ArchaeologyFactEdgeKind::Calls => "call",
        ArchaeologyFactEdgeKind::Reads | ArchaeologyFactEdgeKind::Writes => "data",
        ArchaeologyFactEdgeKind::Controls | ArchaeologyFactEdgeKind::BranchesTo => "control-flow",
        ArchaeologyFactEdgeKind::Includes => "include",
        _ => "source",
    }
}

fn bounded_text(value: &str, limit: usize) -> Result<String, String> {
    if value.trim().is_empty() || value.len() > limit || unsafe_text(value) {
        Err("Archaeology template text violates privacy or byte bounds".into())
    } else {
        Ok(value.into())
    }
}

fn unsafe_text(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    looks_like_secret(value)
        || contains_sensitive_path(value)
        || value.starts_with(['/', '\\'])
        || drive
        || value
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
}

fn truncate_utf8(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit.saturating_sub(3).min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn packet_input_bytes(
    facts: &[ArchaeologyFact],
    edges: &[ArchaeologyFactEdge],
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    let mut total = 0usize;
    for (index, fact) in facts.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        for value in [&fact.fact_id, &fact.label, &fact.parser_id]
            .into_iter()
            .chain(fact.span_ids.iter())
            .chain(
                fact.attributes
                    .iter()
                    .flat_map(|item| [&item.key, &item.value]),
            )
        {
            total = total.saturating_add(value.len());
        }
        total = total.saturating_add(32);
    }
    for (index, edge) in edges.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        for value in [&edge.edge_id, &edge.from_fact_id, &edge.to_fact_id]
            .into_iter()
            .chain(edge.evidence_span_ids.iter())
            .chain(edge.unresolved_reason.iter())
        {
            total = total.saturating_add(value.len());
        }
        total = total.saturating_add(32);
    }
    Ok(total)
}

fn cluster_input_bytes(
    facts: &[ArchaeologyFact],
    edges: &[ArchaeologyFactEdge],
    origins: &[ArchaeologyFactOrigin],
    rules: &[ArchaeologyRulePacket],
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    let mut total = packet_input_bytes(facts, edges, cancellation)?;
    for (index, origin) in origins.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        total = total
            .saturating_add(origin.fact_id.len())
            .saturating_add(origin.source_unit_id.len())
            .saturating_add(origin.path_identity.len())
            .saturating_add(origin.ranking_path_identity.len())
            .saturating_add(32);
    }
    for (index, rule) in rules.iter().enumerate() {
        if index % 128 == 0 {
            cancelled(cancellation)?;
        }
        total = total.saturating_add(128);
        for value in [
            &rule.rule_id,
            &rule.repository_id,
            &rule.generation_id,
            &rule.revision_sha,
            &rule.title,
            &rule.parser_identity,
            &rule.algorithm_identity,
        ]
        .into_iter()
        .chain(rule.synthesis_identity.iter())
        .chain(rule.domain_ids.iter())
        .chain(rule.dependency_rule_ids.iter())
        .chain(rule.conflict_rule_ids.iter())
        .chain(rule.alias_rule_ids.iter())
        .chain(rule.coverage.reasons.iter())
        {
            total = total.saturating_add(value.len());
        }
        for clause in &rule.clauses {
            cancelled(cancellation)?;
            total = total.saturating_add(64);
            for value in [&clause.clause_id, &clause.text]
                .into_iter()
                .chain(clause.supporting_fact_ids.iter())
                .chain(clause.contradicting_fact_ids.iter())
                .chain(clause.evidence_span_ids.iter())
                .chain(clause.caveats.iter())
            {
                total = total.saturating_add(value.len());
            }
        }
    }
    Ok(total)
}

fn evidence_packet_input_bytes(
    packets: &[ArchaeologyEvidencePacket],
    coverage: &ArchaeologyCoverage,
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    let mut total = coverage
        .reasons
        .iter()
        .fold(32usize, |sum, value| sum.saturating_add(value.len()));
    for (index, packet) in packets.iter().enumerate() {
        if index % 1_024 == 0 {
            cancelled(cancellation)?;
        }
        total = total.saturating_add(64);
        for value in std::iter::once(&packet.packet_id)
            .chain(std::iter::once(&packet.anchor_fact_id))
            .chain(packet.supporting_fact_ids.iter())
            .chain(packet.contradicting_fact_ids.iter())
            .chain(packet.relationship_ids.iter())
            .chain(packet.evidence_span_ids.iter())
            .chain(packet.unresolved_fact_ids.iter())
            .chain(packet.unresolved_reasons.iter())
            .chain(packet.caveats.iter())
        {
            total = total.saturating_add(value.len());
        }
    }
    Ok(total)
}

fn deterministic_source_trust(trust: &ArchaeologyTrust) -> bool {
    matches!(
        trust,
        ArchaeologyTrust::Extracted | ArchaeologyTrust::Deterministic
    )
}

fn edge_evidence_matches_endpoints(
    edge: &ArchaeologyFactEdge,
    facts: &BTreeMap<&str, &ArchaeologyFact>,
) -> bool {
    let Some(from) = facts.get(edge.from_fact_id.as_str()) else {
        return false;
    };
    let Some(to) = facts.get(edge.to_fact_id.as_str()) else {
        return false;
    };
    let evidence = edge
        .evidence_span_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = from
        .span_ids
        .iter()
        .chain(&to.span_ids)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    evidence == expected
}

pub(crate) fn expected_packet_id(
    repository_id: &str,
    revision_sha: &str,
    packet: &ArchaeologyEvidencePacket,
) -> String {
    let sorted = |values: &[String]| {
        let mut values = values.to_vec();
        values.sort();
        values.join("\0")
    };
    let local_identity = format!(
        "{:?}\0{:?}\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        packet.kind,
        packet.confidence,
        packet.anchor_fact_id,
        sorted(&packet.supporting_fact_ids),
        sorted(&packet.contradicting_fact_ids),
        sorted(&packet.unresolved_fact_ids),
        sorted(&packet.unresolved_reasons),
        sorted(&packet.relationship_ids),
        sorted(&packet.evidence_span_ids),
        sorted(&packet.caveats),
    );
    stable_graph_id(
        "archaeology-packet",
        &format!("{repository_id}\0{revision_sha}\0{local_identity}"),
    )
}

/// Stable canonical rule identity for both deterministic and model-assisted
/// wording. Optional synthesis may change trust and clause projection, but it
/// must never fork the evidence-derived rule identity.
pub(crate) fn expected_rule_id(packet: &ArchaeologyEvidencePacket) -> String {
    stable_graph_id(
        "archaeology-rule",
        &format!("{}\0template-v1", packet.packet_id),
    )
}

fn safe_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}

fn safe_scope_id(value: &str) -> bool {
    safe_id(value) && !unsafe_text(value) && !value.contains(['/', '\\'])
}

fn extend_evidence(
    evidence: &mut BTreeSet<String>,
    span_ids: &[String],
    limit: usize,
) -> Result<(), String> {
    for span_id in span_ids {
        if !evidence.contains(span_id) && evidence.len() == limit {
            return Err("Archaeology packet evidence span bound exceeded".into());
        }
        evidence.insert(span_id.clone());
    }
    Ok(())
}

fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology packet derivation cancelled".into())
    } else {
        Ok(())
    }
}
