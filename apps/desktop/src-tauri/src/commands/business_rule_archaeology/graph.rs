//! Bounded projection of persisted archaeology evidence into the trusted graph vocabulary.
//!
//! Archaeology tables remain the source of truth. This module deliberately does not
//! materialize a second graph store; canonical desktop and MCP reads can project the
//! same bounded fragment later without losing archaeology-specific provenance.

use super::contracts::{
    validate_revision_sha, ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyFact,
    ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind, ArchaeologyRuleLifecycle,
    ArchaeologyRulePacket, ArchaeologySourceClassification, ArchaeologySourceSpan,
    ArchaeologyTrust, ARCHAEOLOGY_SCHEMA_VERSION,
};
use super::deterministic_rules::ArchaeologyFactOrigin;
use super::inventory::ArchaeologyInventoryUnit;
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::{
    stable_graph_id, GraphOrigin, GraphSourceAnchor, GraphTrust, StructuralGraphCancellation,
    StructuralGraphEdge, StructuralGraphNode,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

pub(crate) const ARCHAEOLOGY_GRAPH_CONTRACT_ID: &str =
    "codevetter.business-rule-archaeology.trusted-graph.v1";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyGraphLimits {
    pub max_source_units: usize,
    pub max_spans: usize,
    pub max_facts: usize,
    pub max_fact_edges: usize,
    pub max_rules: usize,
    pub max_rule_relations: usize,
    pub max_clauses: usize,
    pub max_domains: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_evidence_ids_per_item: usize,
    pub max_source_anchors_per_item: usize,
    pub max_metadata_items_per_item: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for ArchaeologyGraphLimits {
    fn default() -> Self {
        Self {
            max_source_units: 128,
            max_spans: 256,
            max_facts: 256,
            max_fact_edges: 512,
            max_rules: 64,
            max_rule_relations: 512,
            max_clauses: 256,
            max_domains: 64,
            max_nodes: 500,
            max_edges: 2_000,
            max_evidence_ids_per_item: 64,
            max_source_anchors_per_item: 64,
            max_metadata_items_per_item: 128,
            max_input_bytes: 8 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyGraphClaimRole {
    /// Graph context is never independently eligible to create a finding or verified claim.
    NavigationOnly,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyGraphEvidence {
    pub revision_sha: String,
    pub origin: ArchaeologyTrust,
    pub evidence_ids: Vec<String>,
    pub contradicting_evidence_ids: Vec<String>,
    pub coverage: ArchaeologyCoverage,
    pub lifecycle: Option<ArchaeologyRuleLifecycle>,
    pub confidence: Option<ArchaeologyConfidence>,
    pub parser_identity: Option<String>,
    pub algorithm_identity: Option<String>,
    pub synthesis_identity: Option<String>,
    pub limitations: Vec<String>,
    pub claim_role: ArchaeologyGraphClaimRole,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTrustedGraphNode {
    #[serde(flatten)]
    pub graph: StructuralGraphNode,
    pub archaeology: ArchaeologyGraphEvidence,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTrustedGraphEdge {
    #[serde(flatten)]
    pub graph: StructuralGraphEdge,
    pub archaeology: ArchaeologyGraphEvidence,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyGraphDomain {
    pub domain_id: String,
    pub label: String,
    pub parent_domain_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchaeologyGraphRuleRelationKind {
    DependsOn,
    Precedes,
    Overrides,
    Aliases,
    ConflictsWith,
    Supersedes,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyGraphRuleRelation {
    pub relation_id: String,
    pub from_rule_id: String,
    pub to_rule_id: String,
    pub kind: ArchaeologyGraphRuleRelationKind,
    pub trust: ArchaeologyTrust,
    pub evidence_ids: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyTrustedGraphFragment {
    pub schema_version: u32,
    pub contract_id: &'static str,
    pub repository_id: String,
    pub generation_id: String,
    pub revision_sha: String,
    pub nodes: Vec<ArchaeologyTrustedGraphNode>,
    pub edges: Vec<ArchaeologyTrustedGraphEdge>,
    pub coverage: ArchaeologyCoverage,
    /// Projection never silently drops evidence. A caller must request another bounded fragment.
    pub truncated: bool,
}

pub(crate) struct ArchaeologyGraphInput<'a> {
    pub repository_id: &'a str,
    pub generation_id: &'a str,
    pub revision_sha: &'a str,
    pub coverage: &'a ArchaeologyCoverage,
    pub source_units: &'a [ArchaeologyInventoryUnit],
    pub spans: &'a [ArchaeologySourceSpan],
    pub facts: &'a [ArchaeologyFact],
    pub fact_origins: &'a [ArchaeologyFactOrigin],
    pub fact_edges: &'a [ArchaeologyFactEdge],
    pub rules: &'a [ArchaeologyRulePacket],
    pub domains: &'a [ArchaeologyGraphDomain],
    pub rule_relations: &'a [ArchaeologyGraphRuleRelation],
}

pub(crate) fn project_archaeology_graph_fragment(
    input: ArchaeologyGraphInput<'_>,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyGraphLimits,
) -> Result<ArchaeologyTrustedGraphFragment, String> {
    cancelled(cancellation)?;
    validate_scope(&input, limits)?;

    let units = unique_by(
        input.source_units,
        |unit| unit.identity.source_unit_id.as_str(),
        "source unit",
    )?;
    let spans = unique_by(input.spans, |span| span.span_id.as_str(), "source span")?;
    let facts = unique_by(input.facts, |fact| fact.fact_id.as_str(), "fact")?;
    let origins = unique_by(
        input.fact_origins,
        |origin| origin.fact_id.as_str(),
        "fact origin",
    )?;
    let fact_edges = unique_by(input.fact_edges, |edge| edge.edge_id.as_str(), "fact edge")?;
    let rules = unique_by(input.rules, |rule| rule.rule_id.as_str(), "rule")?;
    let domains = unique_by(input.domains, |domain| domain.domain_id.as_str(), "domain")?;
    let rule_relations = unique_by(
        input.rule_relations,
        |relation| relation.relation_id.as_str(),
        "rule relation",
    )?;

    validate_references(
        &input,
        &units,
        &spans,
        &facts,
        &origins,
        &fact_edges,
        &rules,
        &domains,
        &rule_relations,
        limits,
    )?;

    let mut node_ids = BTreeMap::<(&str, &str), String>::new();
    for (kind, ids) in [
        ("source_unit", units.keys().copied().collect::<Vec<_>>()),
        ("span", spans.keys().copied().collect()),
        ("fact", facts.keys().copied().collect()),
        ("rule", rules.keys().copied().collect()),
        ("domain", domains.keys().copied().collect()),
    ] {
        for id in ids {
            node_ids.insert(
                (kind, id),
                graph_id(input.repository_id, input.generation_id, kind, id),
            );
        }
    }
    for rule in rules.values() {
        for clause in &rule.clauses {
            node_ids.insert(
                ("clause", clause.clause_id.as_str()),
                graph_id(
                    input.repository_id,
                    input.generation_id,
                    "clause",
                    &clause.clause_id,
                ),
            );
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut edge_ids = BTreeSet::new();

    for unit in units.values() {
        cancelled(cancellation)?;
        let dialect = unit.dialect.as_deref().unwrap_or("unspecified");
        push_node(
            &mut nodes,
            ArchaeologyTrustedGraphNode {
                graph: StructuralGraphNode {
                    id: node(&node_ids, "source_unit", &unit.identity.source_unit_id)?,
                    kind: "archaeology_source_unit".into(),
                    label: format!("{} source unit", unit.language),
                    qualified_name: None,
                    path: None,
                    detail: Some(format!(
                        "classification={}; dialect={dialect}",
                        classification_name(&unit.classification)
                    )),
                    language: Some(unit.language.clone()),
                    community_id: None,
                    trust: GraphTrust::Extracted,
                    origin: GraphOrigin::Extracted,
                    sources: Vec::new(),
                },
                archaeology: evidence(
                    input.revision_sha,
                    ArchaeologyTrust::Extracted,
                    vec![unit.identity.source_unit_id.clone()],
                    Vec::new(),
                    input.coverage,
                    None,
                    None,
                    Some(format!("{}:{}", unit.language, dialect)),
                    None,
                    None,
                    unit.coverage_reasons.clone(),
                    false,
                )?,
            },
            limits,
        )?;
    }

    for span in spans.values() {
        cancelled(cancellation)?;
        let anchor = span_anchor(span);
        push_node(
            &mut nodes,
            ArchaeologyTrustedGraphNode {
                graph: StructuralGraphNode {
                    id: node(&node_ids, "span", &span.span_id)?,
                    kind: "archaeology_source_span".into(),
                    label: "Exact source span".into(),
                    qualified_name: None,
                    path: None,
                    detail: Some(
                        "one-based Unicode position; byte identity remains authoritative".into(),
                    ),
                    language: units
                        .get(span.source_unit_id.as_str())
                        .map(|unit| unit.language.clone()),
                    community_id: None,
                    trust: GraphTrust::Extracted,
                    origin: GraphOrigin::Extracted,
                    sources: vec![anchor.clone()],
                },
                archaeology: evidence(
                    input.revision_sha,
                    ArchaeologyTrust::Extracted,
                    vec![span.span_id.clone()],
                    Vec::new(),
                    input.coverage,
                    None,
                    Some(ArchaeologyConfidence::High),
                    None,
                    None,
                    None,
                    Vec::new(),
                    false,
                )?,
            },
            limits,
        )?;
        push_edge(
            &mut edges,
            &mut edge_ids,
            typed_edge(
                &input,
                "contains_span",
                node(&node_ids, "source_unit", &span.source_unit_id)?,
                node(&node_ids, "span", &span.span_id)?,
                ArchaeologyTrust::Extracted,
                vec![span.span_id.clone()],
                Vec::new(),
                vec![anchor],
                input.coverage,
                Vec::new(),
                false,
            )?,
            limits,
        )?;
    }

    for fact in facts.values() {
        cancelled(cancellation)?;
        let origin = origins[fact.fact_id.as_str()];
        let fact_spans = exact_anchors(&fact.span_ids, &spans)?;
        let unresolved = fact.kind == ArchaeologyFactKind::Unresolved;
        push_node(
            &mut nodes,
            ArchaeologyTrustedGraphNode {
                graph: StructuralGraphNode {
                    id: node(&node_ids, "fact", &fact.fact_id)?,
                    kind: fact_node_kind(&fact.kind).into(),
                    label: fact.label.clone(),
                    qualified_name: None,
                    path: None,
                    detail: Some(format!("normalized {} fact", fact_kind_name(&fact.kind))),
                    language: units
                        .get(origin.source_unit_id.as_str())
                        .map(|unit| unit.language.clone()),
                    community_id: None,
                    trust: graph_trust(&fact.trust, unresolved),
                    origin: graph_origin(&fact.trust),
                    sources: fact_spans.clone(),
                },
                archaeology: evidence(
                    input.revision_sha,
                    fact.trust.clone(),
                    fact.span_ids
                        .iter()
                        .cloned()
                        .chain(std::iter::once(fact.fact_id.clone()))
                        .collect(),
                    Vec::new(),
                    input.coverage,
                    None,
                    Some(fact.confidence.clone()),
                    Some(fact.parser_id.clone()),
                    None,
                    None,
                    unresolved
                        .then(|| "normalized relationship target is unresolved".to_string())
                        .into_iter()
                        .collect(),
                    unresolved,
                )?,
            },
            limits,
        )?;
        push_edge(
            &mut edges,
            &mut edge_ids,
            typed_edge(
                &input,
                "contains_fact",
                node(&node_ids, "source_unit", &origin.source_unit_id)?,
                node(&node_ids, "fact", &fact.fact_id)?,
                fact.trust.clone(),
                fact.span_ids.clone(),
                Vec::new(),
                fact_spans.clone(),
                input.coverage,
                Vec::new(),
                unresolved,
            )?,
            limits,
        )?;
        for span_id in &fact.span_ids {
            push_edge(
                &mut edges,
                &mut edge_ids,
                typed_edge(
                    &input,
                    "located_at",
                    node(&node_ids, "fact", &fact.fact_id)?,
                    node(&node_ids, "span", span_id)?,
                    fact.trust.clone(),
                    vec![fact.fact_id.clone(), span_id.clone()],
                    Vec::new(),
                    vec![span_anchor(spans[span_id.as_str()])],
                    input.coverage,
                    Vec::new(),
                    unresolved,
                )?,
                limits,
            )?;
        }
    }

    for edge in fact_edges.values() {
        cancelled(cancellation)?;
        let unresolved =
            edge.kind == ArchaeologyFactEdgeKind::Unresolved || edge.unresolved_reason.is_some();
        let limitations = unresolved
            .then(|| "normalized relationship is unresolved or ambiguous".to_string())
            .into_iter()
            .collect();
        push_edge(
            &mut edges,
            &mut edge_ids,
            typed_edge(
                &input,
                fact_edge_kind_name(&edge.kind),
                node(&node_ids, "fact", &edge.from_fact_id)?,
                node(&node_ids, "fact", &edge.to_fact_id)?,
                edge.trust.clone(),
                edge.evidence_span_ids
                    .iter()
                    .cloned()
                    .chain(std::iter::once(edge.edge_id.clone()))
                    .collect(),
                Vec::new(),
                exact_anchors(&edge.evidence_span_ids, &spans)?,
                input.coverage,
                limitations,
                unresolved,
            )?,
            limits,
        )?;
    }

    for domain in domains.values() {
        cancelled(cancellation)?;
        push_node(
            &mut nodes,
            ArchaeologyTrustedGraphNode {
                graph: StructuralGraphNode {
                    id: node(&node_ids, "domain", &domain.domain_id)?,
                    kind: "archaeology_domain".into(),
                    label: domain.label.clone(),
                    qualified_name: None,
                    path: None,
                    detail: domain
                        .parent_domain_id
                        .as_ref()
                        .map(|parent| format!("parent={parent}")),
                    language: None,
                    community_id: Some(domain.domain_id.clone()),
                    trust: GraphTrust::Inferred,
                    origin: GraphOrigin::Deterministic,
                    sources: Vec::new(),
                },
                archaeology: evidence(
                    input.revision_sha,
                    ArchaeologyTrust::Deterministic,
                    vec![domain.domain_id.clone()],
                    Vec::new(),
                    input.coverage,
                    None,
                    None,
                    None,
                    None,
                    None,
                    (domain.domain_id == "domain:other")
                        .then(|| "deterministic fallback domain accounting".to_string())
                        .into_iter()
                        .collect(),
                    false,
                )?,
            },
            limits,
        )?;
    }

    for rule in rules.values() {
        cancelled(cancellation)?;
        let mut rule_evidence = vec![rule.rule_id.clone()];
        let mut contradictions = Vec::new();
        let mut limitations = rule.coverage.reasons.clone();
        for clause in &rule.clauses {
            rule_evidence.extend(clause.supporting_fact_ids.iter().cloned());
            rule_evidence.extend(clause.evidence_span_ids.iter().cloned());
            contradictions.extend(clause.contradicting_fact_ids.iter().cloned());
            limitations.extend(clause.caveats.iter().cloned());
        }
        let uncertain = matches!(
            rule.trust,
            ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown
        );
        let sources = rule
            .clauses
            .iter()
            .flat_map(|clause| clause.evidence_span_ids.iter())
            .map(|span_id| span_anchor(spans[span_id.as_str()]))
            .collect::<Vec<_>>();
        push_node(
            &mut nodes,
            ArchaeologyTrustedGraphNode {
                graph: StructuralGraphNode {
                    id: node(&node_ids, "rule", &rule.rule_id)?,
                    kind: format!("archaeology_rule_{}", rule_kind_name(&rule.kind)),
                    label: rule.title.clone(),
                    qualified_name: None,
                    path: None,
                    detail: Some(format!(
                        "lifecycle={}; confidence={}",
                        lifecycle_name(&rule.lifecycle),
                        confidence_name(&rule.confidence)
                    )),
                    language: None,
                    community_id: rule.domain_ids.first().cloned(),
                    trust: graph_trust(&rule.trust, uncertain),
                    origin: graph_origin(&rule.trust),
                    sources,
                },
                archaeology: evidence(
                    input.revision_sha,
                    rule.trust.clone(),
                    rule_evidence,
                    contradictions,
                    &rule.coverage,
                    Some(rule.lifecycle.clone()),
                    Some(rule.confidence.clone()),
                    Some(rule.parser_identity.clone()),
                    Some(rule.algorithm_identity.clone()),
                    rule.synthesis_identity.clone(),
                    limitations,
                    uncertain,
                )?,
            },
            limits,
        )?;

        for domain_id in &rule.domain_ids {
            push_edge(
                &mut edges,
                &mut edge_ids,
                typed_edge(
                    &input,
                    "classified_in",
                    node(&node_ids, "rule", &rule.rule_id)?,
                    node(&node_ids, "domain", domain_id)?,
                    ArchaeologyTrust::Deterministic,
                    vec![rule.rule_id.clone(), domain_id.clone()],
                    Vec::new(),
                    Vec::new(),
                    &rule.coverage,
                    Vec::new(),
                    false,
                )?,
                limits,
            )?;
        }

        for clause in &rule.clauses {
            cancelled(cancellation)?;
            let clause_uncertain = matches!(
                clause.trust,
                ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown
            );
            let clause_sources = exact_anchors(&clause.evidence_span_ids, &spans)?;
            push_node(
                &mut nodes,
                ArchaeologyTrustedGraphNode {
                    graph: StructuralGraphNode {
                        id: node(&node_ids, "clause", &clause.clause_id)?,
                        kind: "archaeology_rule_clause".into(),
                        label: clause.text.clone(),
                        qualified_name: None,
                        path: None,
                        detail: Some("atomic evidence-traced rule clause".into()),
                        language: None,
                        community_id: rule.domain_ids.first().cloned(),
                        trust: graph_trust(&clause.trust, clause_uncertain),
                        origin: graph_origin(&clause.trust),
                        sources: clause_sources.clone(),
                    },
                    archaeology: evidence(
                        input.revision_sha,
                        clause.trust.clone(),
                        clause
                            .supporting_fact_ids
                            .iter()
                            .chain(&clause.evidence_span_ids)
                            .cloned()
                            .chain(std::iter::once(clause.clause_id.clone()))
                            .collect(),
                        clause.contradicting_fact_ids.clone(),
                        &rule.coverage,
                        Some(rule.lifecycle.clone()),
                        Some(clause.confidence.clone()),
                        Some(rule.parser_identity.clone()),
                        Some(rule.algorithm_identity.clone()),
                        rule.synthesis_identity.clone(),
                        clause.caveats.clone(),
                        clause_uncertain,
                    )?,
                },
                limits,
            )?;
            push_edge(
                &mut edges,
                &mut edge_ids,
                typed_edge(
                    &input,
                    "has_clause",
                    node(&node_ids, "rule", &rule.rule_id)?,
                    node(&node_ids, "clause", &clause.clause_id)?,
                    clause.trust.clone(),
                    vec![rule.rule_id.clone(), clause.clause_id.clone()],
                    clause.contradicting_fact_ids.clone(),
                    clause_sources.clone(),
                    &rule.coverage,
                    clause.caveats.clone(),
                    clause_uncertain,
                )?,
                limits,
            )?;
            for fact_id in &clause.supporting_fact_ids {
                let fact_span_ids = clause_fact_span_ids(clause, facts[fact_id.as_str()])?;
                let fact_sources = exact_anchors(&fact_span_ids, &spans)?;
                push_edge(
                    &mut edges,
                    &mut edge_ids,
                    typed_edge(
                        &input,
                        "supported_by",
                        node(&node_ids, "clause", &clause.clause_id)?,
                        node(&node_ids, "fact", fact_id)?,
                        clause.trust.clone(),
                        vec![clause.clause_id.clone(), fact_id.clone()]
                            .into_iter()
                            .chain(fact_span_ids.iter().cloned())
                            .collect(),
                        Vec::new(),
                        fact_sources,
                        &rule.coverage,
                        clause.caveats.clone(),
                        clause_uncertain,
                    )?,
                    limits,
                )?;
            }
            for fact_id in &clause.contradicting_fact_ids {
                let fact_span_ids = clause_fact_span_ids(clause, facts[fact_id.as_str()])?;
                let fact_sources = exact_anchors(&fact_span_ids, &spans)?;
                push_edge(
                    &mut edges,
                    &mut edge_ids,
                    typed_edge(
                        &input,
                        "contradicted_by",
                        node(&node_ids, "clause", &clause.clause_id)?,
                        node(&node_ids, "fact", fact_id)?,
                        clause.trust.clone(),
                        vec![clause.clause_id.clone(), fact_id.clone()]
                            .into_iter()
                            .chain(fact_span_ids.iter().cloned())
                            .collect(),
                        vec![fact_id.clone()],
                        fact_sources,
                        &rule.coverage,
                        clause.caveats.clone(),
                        clause_uncertain,
                    )?,
                    limits,
                )?;
            }
            for span_id in &clause.evidence_span_ids {
                push_edge(
                    &mut edges,
                    &mut edge_ids,
                    typed_edge(
                        &input,
                        "cited_at",
                        node(&node_ids, "clause", &clause.clause_id)?,
                        node(&node_ids, "span", span_id)?,
                        clause.trust.clone(),
                        vec![clause.clause_id.clone(), span_id.clone()],
                        Vec::new(),
                        vec![span_anchor(spans[span_id.as_str()])],
                        &rule.coverage,
                        clause.caveats.clone(),
                        clause_uncertain,
                    )?,
                    limits,
                )?;
            }
        }
    }

    for relation in rule_relations.values() {
        cancelled(cancellation)?;
        let source = rules[relation.from_rule_id.as_str()];
        let kind = rule_relation_kind_name(&relation.kind);
        let uncertain = matches!(
            relation.trust,
            ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown
        );
        push_edge(
            &mut edges,
            &mut edge_ids,
            typed_edge(
                &input,
                kind,
                node(&node_ids, "rule", &relation.from_rule_id)?,
                node(&node_ids, "rule", &relation.to_rule_id)?,
                relation.trust.clone(),
                relation
                    .evidence_ids
                    .iter()
                    .cloned()
                    .chain(std::iter::once(relation.relation_id.clone()))
                    .collect(),
                (relation.kind == ArchaeologyGraphRuleRelationKind::ConflictsWith)
                    .then(|| relation.to_rule_id.clone())
                    .into_iter()
                    .collect(),
                Vec::new(),
                &source.coverage,
                relation.limitations.clone(),
                uncertain,
            )?,
            limits,
        )?;
    }

    let node_trust = nodes
        .iter()
        .map(|node| (node.graph.id.as_str(), node.graph.trust))
        .collect::<BTreeMap<_, _>>();
    for edge in &mut edges {
        let projected_trust = weakest_graph_trust([
            edge.graph.trust,
            *node_trust
                .get(edge.graph.from.as_str())
                .ok_or("Archaeology trusted graph edge source is missing")?,
            *node_trust
                .get(edge.graph.to.as_str())
                .ok_or("Archaeology trusted graph edge target is missing")?,
        ]);
        if projected_trust == GraphTrust::Ambiguous && edge.graph.trust != GraphTrust::Ambiguous {
            edge.archaeology.confidence = Some(ArchaeologyConfidence::Low);
            edge.archaeology
                .limitations
                .push("endpoint trust limits this relationship to ambiguous navigation".into());
            edge.archaeology.limitations.sort();
            edge.archaeology.limitations.dedup();
        }
        edge.graph.trust = projected_trust;
    }
    nodes.sort_by(|left, right| left.graph.id.cmp(&right.graph.id));
    edges.sort_by(|left, right| left.graph.id.cmp(&right.graph.id));
    cancelled(cancellation)?;
    let fragment = ArchaeologyTrustedGraphFragment {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        contract_id: ARCHAEOLOGY_GRAPH_CONTRACT_ID,
        repository_id: input.repository_id.to_string(),
        generation_id: input.generation_id.to_string(),
        revision_sha: input.revision_sha.to_string(),
        nodes,
        edges,
        coverage: input.coverage.clone(),
        truncated: false,
    };
    validate_output_bounds(&fragment, limits)?;
    let output_bytes = serde_json::to_vec(&fragment)
        .map_err(|_| "Archaeology trusted graph is not serializable")?
        .len();
    if output_bytes > limits.max_output_bytes {
        return Err("Archaeology trusted graph output byte bound exceeded".into());
    }
    cancelled(cancellation)?;
    Ok(fragment)
}

fn validate_scope(
    input: &ArchaeologyGraphInput<'_>,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    if !safe_id(input.repository_id)
        || !safe_id(input.generation_id)
        || validate_revision_sha(input.revision_sha).is_err()
    {
        return Err("Archaeology trusted graph scope is invalid".into());
    }
    let clause_count = input
        .rules
        .iter()
        .map(|rule| rule.clauses.len())
        .sum::<usize>();
    if input.source_units.len() > limits.max_source_units
        || input.spans.len() > limits.max_spans
        || input.facts.len() > limits.max_facts
        || input.fact_origins.len() > limits.max_facts
        || input.fact_edges.len() > limits.max_fact_edges
        || input.rules.len() > limits.max_rules
        || input.rule_relations.len() > limits.max_rule_relations
        || clause_count > limits.max_clauses
        || input.domains.len() > limits.max_domains
    {
        return Err("Archaeology trusted graph input bound exceeded".into());
    }
    bounded_input_bytes(input, limits.max_input_bytes)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_references<'a>(
    input: &ArchaeologyGraphInput<'a>,
    units: &BTreeMap<&'a str, &'a ArchaeologyInventoryUnit>,
    spans: &BTreeMap<&'a str, &'a ArchaeologySourceSpan>,
    facts: &BTreeMap<&'a str, &'a ArchaeologyFact>,
    origins: &BTreeMap<&'a str, &'a ArchaeologyFactOrigin>,
    fact_edges: &BTreeMap<&'a str, &'a ArchaeologyFactEdge>,
    rules: &BTreeMap<&'a str, &'a ArchaeologyRulePacket>,
    domains: &BTreeMap<&'a str, &'a ArchaeologyGraphDomain>,
    rule_relations: &BTreeMap<&'a str, &'a ArchaeologyGraphRuleRelation>,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    validate_coverage(input.coverage, limits)?;
    for unit in units.values() {
        if unit.identity.repository_id != input.repository_id
            || unit.identity.revision_sha != input.revision_sha
            || !safe_id(&unit.identity.source_unit_id)
            || !safe_public_text(&unit.language)
            || unit
                .dialect
                .as_deref()
                .is_some_and(|value| !safe_public_text(value))
            || unit
                .coverage_reasons
                .iter()
                .any(|value| !safe_public_text(value))
            || unit.coverage_reasons.len() > limits.max_metadata_items_per_item
        {
            return Err("Archaeology trusted graph source unit is invalid".into());
        }
    }
    for span in spans.values() {
        span.validate()?;
        if span.revision_sha != input.revision_sha
            || !units.contains_key(span.source_unit_id.as_str())
            || [
                span.start.line,
                span.start.column,
                span.end.line,
                span.end.column,
            ]
            .into_iter()
            .any(|value| u32::try_from(value).is_err())
        {
            return Err("Archaeology trusted graph span is outside its scope".into());
        }
    }
    if origins.len() != facts.len() {
        return Err("Archaeology trusted graph requires one fact origin per fact".into());
    }
    for fact in facts.values() {
        let Some(origin) = origins.get(fact.fact_id.as_str()) else {
            return Err("Archaeology trusted graph fact has no origin".into());
        };
        if !units.contains_key(origin.source_unit_id.as_str())
            || origin.fact_id != fact.fact_id
            || !safe_public_text(&fact.label)
            || !safe_id(&fact.parser_id)
            || fact.span_ids.is_empty()
            || fact.span_ids.len() > limits.max_evidence_ids_per_item
            || fact.span_ids.iter().any(|id| {
                spans
                    .get(id.as_str())
                    .is_none_or(|span| span.source_unit_id != origin.source_unit_id)
            })
        {
            return Err("Archaeology trusted graph fact evidence is invalid".into());
        }
    }
    for edge in fact_edges.values() {
        let from = facts.get(edge.from_fact_id.as_str());
        let to = facts.get(edge.to_fact_id.as_str());
        let from_spans = from
            .into_iter()
            .flat_map(|fact| fact.span_ids.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let to_spans = to
            .into_iter()
            .flat_map(|fact| fact.span_ids.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        if !facts.contains_key(edge.from_fact_id.as_str())
            || !facts.contains_key(edge.to_fact_id.as_str())
            || edge.evidence_span_ids.is_empty()
            || edge.evidence_span_ids.len() > limits.max_evidence_ids_per_item
            || edge.evidence_span_ids.iter().any(|id| {
                !spans.contains_key(id.as_str())
                    || (!from_spans.contains(id.as_str()) && !to_spans.contains(id.as_str()))
            })
            || !edge
                .evidence_span_ids
                .iter()
                .any(|id| from_spans.contains(id.as_str()))
            || !edge
                .evidence_span_ids
                .iter()
                .any(|id| to_spans.contains(id.as_str()))
        {
            return Err("Archaeology trusted graph fact edge is invalid".into());
        }
    }
    for domain in domains.values() {
        if !safe_id(&domain.domain_id)
            || !safe_public_text(&domain.label)
            || domain
                .parent_domain_id
                .as_deref()
                .is_some_and(|parent| !domains.contains_key(parent))
        {
            return Err("Archaeology trusted graph domain is invalid".into());
        }
    }
    let mut clauses = BTreeSet::new();
    let mut expected_relations = BTreeSet::new();
    for rule in rules.values() {
        rule.validate()?;
        validate_coverage(&rule.coverage, limits)?;
        if rule.repository_id != input.repository_id
            || rule.generation_id != input.generation_id
            || rule.revision_sha != input.revision_sha
            || !safe_public_text(&rule.title)
            || !safe_id(&rule.parser_identity)
            || !safe_id(&rule.algorithm_identity)
            || rule
                .synthesis_identity
                .as_deref()
                .is_some_and(|identity| !safe_id(identity))
            || rule
                .domain_ids
                .iter()
                .any(|id| !domains.contains_key(id.as_str()))
            || rule
                .dependency_rule_ids
                .iter()
                .chain(&rule.conflict_rule_ids)
                .chain(&rule.alias_rule_ids)
                .any(|id| !rules.contains_key(id.as_str()))
        {
            return Err("Archaeology trusted graph rule scope is invalid".into());
        }
        for (kind, targets) in [
            (
                ArchaeologyGraphRuleRelationKind::DependsOn,
                &rule.dependency_rule_ids,
            ),
            (
                ArchaeologyGraphRuleRelationKind::ConflictsWith,
                &rule.conflict_rule_ids,
            ),
            (
                ArchaeologyGraphRuleRelationKind::Aliases,
                &rule.alias_rule_ids,
            ),
        ] {
            for target in targets {
                expected_relations.insert((rule.rule_id.as_str(), target.as_str(), kind.clone()));
            }
        }
        for clause in &rule.clauses {
            if !clauses.insert(clause.clause_id.as_str())
                || !safe_public_text(&clause.text)
                || clause.caveats.iter().any(|value| !safe_public_text(value))
                || clause.caveats.len() > limits.max_metadata_items_per_item
                || clause
                    .supporting_fact_ids
                    .iter()
                    .chain(&clause.contradicting_fact_ids)
                    .any(|id| !facts.contains_key(id.as_str()))
                || clause
                    .evidence_span_ids
                    .iter()
                    .any(|id| !spans.contains_key(id.as_str()))
                || clause
                    .supporting_fact_ids
                    .len()
                    .saturating_add(clause.contradicting_fact_ids.len())
                    .saturating_add(clause.evidence_span_ids.len())
                    > limits.max_evidence_ids_per_item
                || clause
                    .supporting_fact_ids
                    .iter()
                    .chain(&clause.contradicting_fact_ids)
                    .any(|fact_id| {
                        !clause
                            .evidence_span_ids
                            .iter()
                            .any(|span_id| facts[fact_id.as_str()].span_ids.contains(span_id))
                    })
            {
                return Err("Archaeology trusted graph clause evidence is invalid".into());
            }
        }
    }
    let mut actual_relations = BTreeSet::new();
    for relation in rule_relations.values() {
        if !rules.contains_key(relation.from_rule_id.as_str())
            || !rules.contains_key(relation.to_rule_id.as_str())
            || relation.evidence_ids.is_empty()
            || relation.evidence_ids.len() > limits.max_evidence_ids_per_item
            || relation.limitations.len() > limits.max_metadata_items_per_item
            || relation.evidence_ids.iter().any(|id| {
                !safe_id(id)
                    || (!facts.contains_key(id.as_str())
                        && !spans.contains_key(id.as_str())
                        && !rules.contains_key(id.as_str()))
            })
            || relation
                .limitations
                .iter()
                .any(|value| !safe_public_text(value))
            || (relation.kind == ArchaeologyGraphRuleRelationKind::DependsOn
                && !matches!(
                    relation.trust,
                    ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown
                )
                && !relation
                    .evidence_ids
                    .iter()
                    .any(|id| facts.contains_key(id.as_str()) || spans.contains_key(id.as_str())))
            || !actual_relations.insert((
                relation.from_rule_id.as_str(),
                relation.to_rule_id.as_str(),
                relation.kind.clone(),
            ))
        {
            return Err("Archaeology trusted graph rule relation is invalid".into());
        }
    }
    let materialized_relations = actual_relations
        .into_iter()
        .filter(|(_, _, kind)| {
            matches!(
                kind,
                ArchaeologyGraphRuleRelationKind::DependsOn
                    | ArchaeologyGraphRuleRelationKind::ConflictsWith
                    | ArchaeologyGraphRuleRelationKind::Aliases
            )
        })
        .collect::<BTreeSet<_>>();
    if expected_relations != materialized_relations {
        return Err("Archaeology trusted graph rule relation parity failed".into());
    }
    Ok(())
}

fn unique_by<'a, T>(
    values: &'a [T],
    id: impl Fn(&'a T) -> &'a str,
    label: &str,
) -> Result<BTreeMap<&'a str, &'a T>, String> {
    let mut output = BTreeMap::new();
    for value in values {
        let identity = id(value);
        if !safe_id(identity) || output.insert(identity, value).is_some() {
            return Err(format!(
                "Archaeology trusted graph {label} identity is invalid or duplicate"
            ));
        }
    }
    Ok(output)
}

fn push_node(
    nodes: &mut Vec<ArchaeologyTrustedGraphNode>,
    node: ArchaeologyTrustedGraphNode,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    if nodes.len() == limits.max_nodes {
        return Err("Archaeology trusted graph node bound exceeded".into());
    }
    nodes.push(node);
    Ok(())
}

fn push_edge(
    edges: &mut Vec<ArchaeologyTrustedGraphEdge>,
    ids: &mut BTreeSet<String>,
    edge: ArchaeologyTrustedGraphEdge,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    if edges.len() == limits.max_edges {
        return Err("Archaeology trusted graph edge bound exceeded".into());
    }
    if !ids.insert(edge.graph.id.clone()) {
        return Err("Archaeology trusted graph edge identity is duplicate".into());
    }
    edges.push(edge);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn typed_edge(
    input: &ArchaeologyGraphInput<'_>,
    kind: &str,
    from: String,
    to: String,
    trust: ArchaeologyTrust,
    evidence_ids: Vec<String>,
    contradicting_evidence_ids: Vec<String>,
    sources: Vec<GraphSourceAnchor>,
    coverage: &ArchaeologyCoverage,
    limitations: Vec<String>,
    force_navigation_only: bool,
) -> Result<ArchaeologyTrustedGraphEdge, String> {
    let graph_trust = graph_trust(&trust, force_navigation_only);
    let graph_origin = graph_origin(&trust);
    let confidence = trust_confidence(&trust);
    let mut identity_evidence = evidence_ids.clone();
    identity_evidence.sort();
    identity_evidence.dedup();
    let id = stable_graph_id(
        "archaeology-trusted-edge",
        &format!(
            "{}\0{}\0{}\0{}\0{}\0{}",
            input.repository_id,
            input.generation_id,
            kind,
            from,
            to,
            identity_evidence.join("\0")
        ),
    );
    Ok(ArchaeologyTrustedGraphEdge {
        graph: StructuralGraphEdge {
            id,
            from,
            to,
            kind: format!("archaeology_{kind}"),
            evidence: format!("Evidence-traced archaeology relationship: {kind}"),
            trust: graph_trust,
            origin: graph_origin,
            sources,
            candidates: Vec::new(),
        },
        archaeology: evidence(
            input.revision_sha,
            trust,
            evidence_ids,
            contradicting_evidence_ids,
            coverage,
            None,
            Some(confidence),
            None,
            None,
            None,
            limitations,
            force_navigation_only,
        )?,
    })
}

#[allow(clippy::too_many_arguments)]
fn evidence(
    revision_sha: &str,
    origin: ArchaeologyTrust,
    mut evidence_ids: Vec<String>,
    mut contradicting_evidence_ids: Vec<String>,
    coverage: &ArchaeologyCoverage,
    lifecycle: Option<ArchaeologyRuleLifecycle>,
    confidence: Option<ArchaeologyConfidence>,
    parser_identity: Option<String>,
    algorithm_identity: Option<String>,
    synthesis_identity: Option<String>,
    mut limitations: Vec<String>,
    _force_navigation_only: bool,
) -> Result<ArchaeologyGraphEvidence, String> {
    evidence_ids.sort();
    evidence_ids.dedup();
    contradicting_evidence_ids.sort();
    contradicting_evidence_ids.dedup();
    limitations.sort();
    limitations.dedup();
    if evidence_ids.is_empty()
        || evidence_ids.iter().any(|value| !safe_id(value))
        || contradicting_evidence_ids
            .iter()
            .any(|value| !safe_id(value))
        || limitations.iter().any(|value| !safe_public_text(value))
    {
        return Err("Archaeology trusted graph provenance is invalid".into());
    }
    Ok(ArchaeologyGraphEvidence {
        revision_sha: revision_sha.to_string(),
        origin,
        evidence_ids,
        contradicting_evidence_ids,
        coverage: coverage.clone(),
        lifecycle,
        confidence,
        parser_identity,
        algorithm_identity,
        synthesis_identity,
        limitations,
        claim_role: ArchaeologyGraphClaimRole::NavigationOnly,
    })
}

fn validate_output_bounds(
    fragment: &ArchaeologyTrustedGraphFragment,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    let within = |value: &ArchaeologyGraphEvidence, sources: &[GraphSourceAnchor]| {
        value.evidence_ids.len() <= limits.max_evidence_ids_per_item
            && value.contradicting_evidence_ids.len() <= limits.max_evidence_ids_per_item
            && sources.len() <= limits.max_source_anchors_per_item
    };
    if fragment
        .nodes
        .iter()
        .any(|node| !within(&node.archaeology, &node.graph.sources))
        || fragment
            .edges
            .iter()
            .any(|edge| !within(&edge.archaeology, &edge.graph.sources))
    {
        return Err("Archaeology trusted graph evidence or source-anchor bound exceeded".into());
    }
    Ok(())
}

fn weakest_graph_trust(values: impl IntoIterator<Item = GraphTrust>) -> GraphTrust {
    values
        .into_iter()
        .max_by_key(|trust| match trust {
            GraphTrust::Extracted => 0,
            GraphTrust::Inferred => 1,
            GraphTrust::Ambiguous => 2,
            GraphTrust::Legacy => 3,
        })
        .unwrap_or(GraphTrust::Ambiguous)
}

fn graph_trust(trust: &ArchaeologyTrust, force_ambiguous: bool) -> GraphTrust {
    if force_ambiguous {
        return GraphTrust::Ambiguous;
    }
    match trust {
        ArchaeologyTrust::Extracted => GraphTrust::Extracted,
        ArchaeologyTrust::Deterministic | ArchaeologyTrust::HumanConfirmed => GraphTrust::Inferred,
        ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown => GraphTrust::Ambiguous,
    }
}

fn graph_origin(trust: &ArchaeologyTrust) -> GraphOrigin {
    match trust {
        ArchaeologyTrust::Extracted => GraphOrigin::Extracted,
        ArchaeologyTrust::Deterministic => GraphOrigin::Deterministic,
        ArchaeologyTrust::ModelSynthesized => GraphOrigin::ModelSynthesized,
        ArchaeologyTrust::HumanConfirmed => GraphOrigin::HumanConfirmed,
        ArchaeologyTrust::Unknown => GraphOrigin::LegacyMetadata,
    }
}

fn trust_confidence(trust: &ArchaeologyTrust) -> ArchaeologyConfidence {
    match trust {
        ArchaeologyTrust::Extracted => ArchaeologyConfidence::High,
        ArchaeologyTrust::Deterministic | ArchaeologyTrust::HumanConfirmed => {
            ArchaeologyConfidence::Medium
        }
        ArchaeologyTrust::ModelSynthesized | ArchaeologyTrust::Unknown => {
            ArchaeologyConfidence::Low
        }
    }
}

fn node(ids: &BTreeMap<(&str, &str), String>, kind: &str, id: &str) -> Result<String, String> {
    ids.get(&(kind, id))
        .cloned()
        .ok_or_else(|| "Archaeology trusted graph endpoint is missing".into())
}

fn graph_id(repository: &str, generation: &str, kind: &str, id: &str) -> String {
    stable_graph_id(
        "archaeology-trusted-node",
        &format!("{repository}\0{generation}\0{kind}\0{id}"),
    )
}

fn exact_anchors(
    ids: &[String],
    spans: &BTreeMap<&str, &ArchaeologySourceSpan>,
) -> Result<Vec<GraphSourceAnchor>, String> {
    ids.iter()
        .map(|id| {
            spans
                .get(id.as_str())
                .map(|span| span_anchor(span))
                .ok_or_else(|| "Archaeology trusted graph exact span is missing".into())
        })
        .collect()
}

fn clause_fact_span_ids(
    clause: &super::contracts::ArchaeologyRuleClause,
    fact: &ArchaeologyFact,
) -> Result<Vec<String>, String> {
    let fact_spans = fact
        .span_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let output = clause
        .evidence_span_ids
        .iter()
        .filter(|span_id| fact_spans.contains(span_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if output.is_empty() {
        Err("Archaeology trusted graph clause-to-fact evidence is not exact".into())
    } else {
        Ok(output)
    }
}

fn span_anchor(span: &ArchaeologySourceSpan) -> GraphSourceAnchor {
    GraphSourceAnchor {
        // This is an opaque source identity, not a filesystem path.
        path: span.source_unit_id.clone(),
        start_line: u32::try_from(span.start.line).ok(),
        start_column: u32::try_from(span.start.column).ok(),
        end_line: u32::try_from(span.end.line).ok(),
        end_column: u32::try_from(span.end.column).ok(),
        excerpt: None,
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains('\0')
        && !value.chars().any(char::is_whitespace)
        && !std::path::Path::new(value).is_absolute()
        && !contains_sensitive_path(value)
        && !looks_like_secret(value)
}

fn safe_public_text(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 2_048
        && !value.contains('\0')
        && !contains_sensitive_path(value)
        && !looks_like_secret(value)
}

fn validate_coverage(
    coverage: &ArchaeologyCoverage,
    limits: ArchaeologyGraphLimits,
) -> Result<(), String> {
    if coverage.reasons.len() > limits.max_metadata_items_per_item
        || coverage
            .reasons
            .iter()
            .any(|reason| !safe_public_text(reason))
    {
        return Err("Archaeology trusted graph coverage metadata is invalid".into());
    }
    Ok(())
}

fn bounded_input_bytes(input: &ArchaeologyGraphInput<'_>, max_bytes: usize) -> Result<(), String> {
    let mut writer = BoundedWriter {
        bytes: 0,
        max_bytes,
    };
    serde_json::to_writer(
        &mut writer,
        &(
            input.repository_id,
            input.generation_id,
            input.revision_sha,
            input.coverage,
            input.source_units,
            input.spans,
            input.facts,
            input.fact_origins,
            input.fact_edges,
            input.rules,
            input.domains,
            input.rule_relations,
        ),
    )
    .map_err(|_| "Archaeology trusted graph input byte bound exceeded".to_string())
}

struct BoundedWriter {
    bytes: usize,
    max_bytes: usize,
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(buffer.len())
            .ok_or_else(|| io::Error::other("archaeology graph input overflow"))?;
        if next > self.max_bytes {
            return Err(io::Error::other(
                "archaeology graph input byte bound exceeded",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology trusted graph projection cancelled".into())
    } else {
        Ok(())
    }
}

fn fact_node_kind(kind: &ArchaeologyFactKind) -> &'static str {
    match kind {
        ArchaeologyFactKind::Declaration | ArchaeologyFactKind::EntryPoint => "archaeology_program",
        ArchaeologyFactKind::DataField | ArchaeologyFactKind::Constant => "archaeology_data",
        ArchaeologyFactKind::Call => "archaeology_call",
        ArchaeologyFactKind::Transaction => "archaeology_transaction",
        ArchaeologyFactKind::Predicate => "archaeology_predicate",
        ArchaeologyFactKind::Decision => "archaeology_decision",
        ArchaeologyFactKind::Calculation => "archaeology_calculation",
        ArchaeologyFactKind::Mutation => "archaeology_mutation",
        ArchaeologyFactKind::InputOutput => "archaeology_input_output",
        ArchaeologyFactKind::ControlFlow => "archaeology_control_flow",
        ArchaeologyFactKind::Include => "archaeology_include",
        ArchaeologyFactKind::Unresolved => "archaeology_unresolved",
    }
}

fn fact_kind_name(kind: &ArchaeologyFactKind) -> &'static str {
    match kind {
        ArchaeologyFactKind::Declaration => "declaration",
        ArchaeologyFactKind::DataField => "data_field",
        ArchaeologyFactKind::Constant => "constant",
        ArchaeologyFactKind::Predicate => "predicate",
        ArchaeologyFactKind::Decision => "decision",
        ArchaeologyFactKind::Calculation => "calculation",
        ArchaeologyFactKind::Mutation => "mutation",
        ArchaeologyFactKind::Call => "call",
        ArchaeologyFactKind::InputOutput => "input_output",
        ArchaeologyFactKind::Transaction => "transaction",
        ArchaeologyFactKind::ControlFlow => "control_flow",
        ArchaeologyFactKind::EntryPoint => "entry_point",
        ArchaeologyFactKind::Include => "include",
        ArchaeologyFactKind::Unresolved => "unresolved",
    }
}

fn fact_edge_kind_name(kind: &ArchaeologyFactEdgeKind) -> &'static str {
    match kind {
        ArchaeologyFactEdgeKind::Defines => "defines",
        ArchaeologyFactEdgeKind::Reads => "reads",
        ArchaeologyFactEdgeKind::Writes => "writes",
        ArchaeologyFactEdgeKind::Calls => "calls",
        ArchaeologyFactEdgeKind::Includes => "includes",
        ArchaeologyFactEdgeKind::Controls => "controls",
        ArchaeologyFactEdgeKind::BranchesTo => "branches_to",
        ArchaeologyFactEdgeKind::Calculates => "calculates",
        ArchaeologyFactEdgeKind::BeginsTransaction => "begins_transaction",
        ArchaeologyFactEdgeKind::CommitsTransaction => "commits_transaction",
        ArchaeologyFactEdgeKind::RollsBackTransaction => "rolls_back_transaction",
        ArchaeologyFactEdgeKind::Supports => "supports",
        ArchaeologyFactEdgeKind::Contradicts => "contradicts",
        ArchaeologyFactEdgeKind::Aliases => "aliases",
        ArchaeologyFactEdgeKind::Unresolved => "unresolved",
    }
}

fn rule_relation_kind_name(kind: &ArchaeologyGraphRuleRelationKind) -> &'static str {
    match kind {
        ArchaeologyGraphRuleRelationKind::DependsOn => "depends_on",
        ArchaeologyGraphRuleRelationKind::Precedes => "precedes",
        ArchaeologyGraphRuleRelationKind::Overrides => "overrides",
        ArchaeologyGraphRuleRelationKind::Aliases => "aliases",
        ArchaeologyGraphRuleRelationKind::ConflictsWith => "conflicts_with",
        ArchaeologyGraphRuleRelationKind::Supersedes => "supersedes",
    }
}

fn classification_name(value: &ArchaeologySourceClassification) -> &'static str {
    match value {
        ArchaeologySourceClassification::Source => "source",
        ArchaeologySourceClassification::Generated => "generated",
        ArchaeologySourceClassification::Vendor => "vendor",
        ArchaeologySourceClassification::Protected => "protected",
        ArchaeologySourceClassification::Opaque => "opaque",
        ArchaeologySourceClassification::Unavailable => "unavailable",
    }
}

fn rule_kind_name(value: &super::contracts::ArchaeologyRuleKind) -> &'static str {
    use super::contracts::ArchaeologyRuleKind;
    match value {
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

fn lifecycle_name(value: &ArchaeologyRuleLifecycle) -> &'static str {
    match value {
        ArchaeologyRuleLifecycle::Candidate => "candidate",
        ArchaeologyRuleLifecycle::ReviewNeeded => "review_needed",
        ArchaeologyRuleLifecycle::Accepted => "accepted",
        ArchaeologyRuleLifecycle::Rejected => "rejected",
        ArchaeologyRuleLifecycle::Superseded => "superseded",
        ArchaeologyRuleLifecycle::Conflicted => "conflicted",
        ArchaeologyRuleLifecycle::Unavailable => "unavailable",
    }
}

fn confidence_name(value: &super::contracts::ArchaeologyConfidence) -> &'static str {
    use super::contracts::ArchaeologyConfidence;
    match value {
        ArchaeologyConfidence::High => "high",
        ArchaeologyConfidence::Medium => "medium",
        ArchaeologyConfidence::Low => "low",
        ArchaeologyConfidence::Unavailable => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::business_rule_archaeology::contracts::{
        ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyCoverageState, ArchaeologyPosition,
        ArchaeologyRuleClause, ArchaeologyRuleKind, ArchaeologySourceUnitIdentity,
    };

    #[derive(Clone)]
    struct Fixture {
        repository_id: String,
        generation_id: String,
        revision_sha: String,
        coverage: ArchaeologyCoverage,
        source_units: Vec<ArchaeologyInventoryUnit>,
        spans: Vec<ArchaeologySourceSpan>,
        facts: Vec<ArchaeologyFact>,
        origins: Vec<ArchaeologyFactOrigin>,
        fact_edges: Vec<ArchaeologyFactEdge>,
        rules: Vec<ArchaeologyRulePacket>,
        domains: Vec<ArchaeologyGraphDomain>,
        rule_relations: Vec<ArchaeologyGraphRuleRelation>,
    }

    impl Fixture {
        fn input(&self) -> ArchaeologyGraphInput<'_> {
            ArchaeologyGraphInput {
                repository_id: &self.repository_id,
                generation_id: &self.generation_id,
                revision_sha: &self.revision_sha,
                coverage: &self.coverage,
                source_units: &self.source_units,
                spans: &self.spans,
                facts: &self.facts,
                fact_origins: &self.origins,
                fact_edges: &self.fact_edges,
                rules: &self.rules,
                domains: &self.domains,
                rule_relations: &self.rule_relations,
            }
        }
    }

    #[test]
    fn rule_clause_fact_span_and_source_paths_preserve_exact_typed_evidence() {
        let fixture = fixture();
        let graph = project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default(),
        )
        .expect("trusted graph");

        for kind in [
            "archaeology_rule_validation",
            "archaeology_rule_clause",
            "archaeology_predicate",
            "archaeology_mutation",
            "archaeology_data",
            "archaeology_source_span",
            "archaeology_source_unit",
            "archaeology_domain",
        ] {
            assert!(
                graph.nodes.iter().any(|node| node.graph.kind == kind),
                "{kind}"
            );
        }
        for kind in [
            "archaeology_has_clause",
            "archaeology_supported_by",
            "archaeology_controls",
            "archaeology_writes",
            "archaeology_located_at",
            "archaeology_contains_fact",
            "archaeology_contains_span",
            "archaeology_classified_in",
        ] {
            assert!(
                graph.edges.iter().any(|edge| edge.graph.kind == kind),
                "{kind}"
            );
        }
        assert!(graph.nodes.iter().all(|node| {
            !node.archaeology.evidence_ids.is_empty()
                && node.archaeology.claim_role == ArchaeologyGraphClaimRole::NavigationOnly
        }));
        assert!(graph.edges.iter().all(|edge| {
            !edge.archaeology.evidence_ids.is_empty()
                && edge.archaeology.claim_role == ArchaeologyGraphClaimRole::NavigationOnly
                && edge
                    .graph
                    .sources
                    .iter()
                    .all(|source| source.excerpt.is_none())
        }));
        let clause = graph
            .nodes
            .iter()
            .find(|node| node.graph.kind == "archaeology_rule_clause")
            .expect("clause");
        assert_eq!(
            clause.archaeology.lifecycle,
            Some(ArchaeologyRuleLifecycle::Candidate)
        );
        assert_eq!(
            clause.archaeology.confidence,
            Some(ArchaeologyConfidence::High)
        );
        assert!(clause
            .archaeology
            .evidence_ids
            .contains(&"fact:predicate".into()));
        assert!(clause
            .archaeology
            .evidence_ids
            .contains(&"span:predicate".into()));
        assert!(clause
            .archaeology
            .limitations
            .contains(&"fixture caveat".into()));
        assert!(graph
            .nodes
            .iter()
            .flat_map(|node| &node.graph.sources)
            .all(|source| source.path == "unit:payments"));
        let predicate_id = graph
            .nodes
            .iter()
            .find(|node| node.graph.kind == "archaeology_predicate")
            .expect("predicate")
            .graph
            .id
            .clone();
        let predicate_support = graph
            .edges
            .iter()
            .find(|edge| {
                edge.graph.kind == "archaeology_supported_by" && edge.graph.to == predicate_id
            })
            .expect("predicate support");
        assert_eq!(
            predicate_support
                .graph
                .sources
                .iter()
                .map(|source| source.start_line)
                .collect::<Vec<_>>(),
            vec![Some(2)]
        );
        assert!(!predicate_support
            .archaeology
            .evidence_ids
            .contains(&"span:data".into()));
    }

    #[test]
    fn model_unresolved_and_accepted_state_never_upgrade_graph_trust() {
        let mut fixture = fixture();
        let mut model = fixture.rules[0].clone();
        model.rule_id = "rule:model".into();
        model.title = "Model-authored dependency candidate".into();
        model.lifecycle = ArchaeologyRuleLifecycle::Accepted;
        model.trust = ArchaeologyTrust::ModelSynthesized;
        model.confidence = ArchaeologyConfidence::High;
        model.synthesis_identity = Some("synthesis:v1".into());
        model.dependency_rule_ids = vec![fixture.rules[0].rule_id.clone()];
        model.clauses[0].clause_id = "clause:model".into();
        model.clauses[0].trust = ArchaeologyTrust::ModelSynthesized;
        fixture.rules.push(model);
        fixture.rule_relations.push(ArchaeologyGraphRuleRelation {
            relation_id: "relation:model".into(),
            from_rule_id: "rule:model".into(),
            to_rule_id: "rule:payment".into(),
            kind: ArchaeologyGraphRuleRelationKind::DependsOn,
            trust: ArchaeologyTrust::ModelSynthesized,
            evidence_ids: vec!["rule:model".into(), "rule:payment".into()],
            limitations: vec!["model-only dependency is unsupported by normalized facts".into()],
        });
        fixture.facts[0].kind = ArchaeologyFactKind::Unresolved;
        fixture.facts[0].trust = ArchaeologyTrust::Unknown;
        fixture.fact_edges[0].kind = ArchaeologyFactEdgeKind::Unresolved;
        fixture.fact_edges[0].unresolved_reason = Some("target was not uniquely resolved".into());

        let graph = project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default(),
        )
        .expect("trusted graph");
        let model_node = graph
            .nodes
            .iter()
            .find(|node| node.archaeology.origin == ArchaeologyTrust::ModelSynthesized)
            .expect("model node");
        assert_eq!(model_node.graph.trust, GraphTrust::Ambiguous);
        assert_eq!(model_node.graph.origin, GraphOrigin::ModelSynthesized);
        assert_eq!(
            model_node.archaeology.lifecycle,
            Some(ArchaeologyRuleLifecycle::Accepted)
        );
        assert_eq!(
            model_node.archaeology.claim_role,
            ArchaeologyGraphClaimRole::NavigationOnly
        );
        assert!(graph
            .edges
            .iter()
            .filter(|edge| {
                edge.graph.kind == "archaeology_depends_on"
                    || edge.graph.kind == "archaeology_unresolved"
            })
            .all(|edge| edge.graph.trust == GraphTrust::Ambiguous));
    }

    #[test]
    fn projection_is_order_stable_scoped_private_cancellable_and_bounded() {
        let fixture = fixture();
        let expected = project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default(),
        )
        .expect("trusted graph");
        let mut shuffled = fixture.clone();
        shuffled.source_units.reverse();
        shuffled.spans.reverse();
        shuffled.facts.reverse();
        shuffled.origins.reverse();
        shuffled.fact_edges.reverse();
        shuffled.rules.reverse();
        shuffled.domains.reverse();
        shuffled.rule_relations.reverse();
        let actual = project_archaeology_graph_fragment(
            shuffled.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default(),
        )
        .expect("shuffled trusted graph");
        assert_eq!(
            serde_json::to_vec(&expected).expect("expected JSON"),
            serde_json::to_vec(&actual).expect("actual JSON")
        );
        assert_ne!(
            graph_id("repository:one", "generation:one", "rule", "same"),
            graph_id("repository:two", "generation:one", "rule", "same")
        );
        assert_ne!(
            graph_id("repository:one", "generation:one", "rule", "same"),
            graph_id("repository:one", "generation:two", "rule", "same")
        );
        let json = serde_json::to_string(&expected).expect("JSON");
        assert!(!json.contains("/Users/private/source.cbl"));
        assert!(!json.contains("PROTECTED SOURCE BODY"));

        let cancellation = StructuralGraphCancellation::default();
        cancellation.cancel();
        assert!(project_archaeology_graph_fragment(
            fixture.input(),
            &cancellation,
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("cancelled"));

        let limits = ArchaeologyGraphLimits {
            max_nodes: 1,
            ..Default::default()
        };
        assert!(project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            limits
        )
        .unwrap_err()
        .contains("node bound"));

        let mut private = fixture.clone();
        private.rules[0].title = "token=sk-proj-prohibited-secret-value".into();
        assert!(project_archaeology_graph_fragment(
            private.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("rule scope"));
    }

    #[test]
    fn projection_rejects_citation_laundering_and_unbounded_or_private_metadata() {
        let fixture = fixture();

        let mut unrelated = fixture.clone();
        unrelated.fact_edges[0].evidence_span_ids = vec!["span:data".into()];
        assert!(project_archaeology_graph_fragment(
            unrelated.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("fact edge"));

        let mut missing_fact_span = fixture.clone();
        missing_fact_span.rules[0].clauses[0]
            .evidence_span_ids
            .retain(|span| span != "span:data");
        assert!(project_archaeology_graph_fragment(
            missing_fact_span.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("clause evidence"));

        let mut secret = fixture.clone();
        secret.coverage.reasons = vec!["password=correct-horse-battery-staple".into()];
        assert!(project_archaeology_graph_fragment(
            secret.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("coverage metadata"));

        let mut private_identity = fixture.clone();
        private_identity.rules[0].parser_identity = "/Users/private/parser".into();
        assert!(project_archaeology_graph_fragment(
            private_identity.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("rule scope"));

        let mut private_fact_parser = fixture.clone();
        private_fact_parser.facts[0].parser_id = "sk-proj-prohibited-parser-secret".into();
        assert!(project_archaeology_graph_fragment(
            private_fact_parser.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("fact evidence"));

        let mut oversized = fixture.clone();
        oversized.coverage.reasons = (0..129).map(|index| format!("reason:{index}")).collect();
        assert!(project_archaeology_graph_fragment(
            oversized.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("coverage metadata"));

        let byte_limits = ArchaeologyGraphLimits {
            max_input_bytes: 32,
            ..Default::default()
        };
        assert!(project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            byte_limits
        )
        .unwrap_err()
        .contains("input byte bound"));

        let mut uppercase_sha = fixture.clone();
        uppercase_sha.revision_sha = "A".repeat(40);
        assert!(project_archaeology_graph_fragment(
            uppercase_sha.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("scope"));
    }

    #[test]
    fn persisted_relation_trust_and_endpoint_downgrades_are_not_laundered() {
        let mut fixture = fixture();
        let mut target = fixture.rules[0].clone();
        target.rule_id = "rule:target".into();
        target.title = "Target rule".into();
        target.clauses[0].clause_id = "clause:target".into();
        target.trust = ArchaeologyTrust::ModelSynthesized;
        target.clauses[0].trust = ArchaeologyTrust::ModelSynthesized;
        target.synthesis_identity = Some("synthesis:v1".into());
        fixture.rules[0].dependency_rule_ids = vec![target.rule_id.clone()];
        fixture.rules.push(target);
        fixture.rule_relations.push(ArchaeologyGraphRuleRelation {
            relation_id: "relation:dependency".into(),
            from_rule_id: "rule:payment".into(),
            to_rule_id: "rule:target".into(),
            kind: ArchaeologyGraphRuleRelationKind::DependsOn,
            trust: ArchaeologyTrust::Deterministic,
            evidence_ids: vec!["fact:predicate".into(), "span:predicate".into()],
            limitations: Vec::new(),
        });

        let graph = project_archaeology_graph_fragment(
            fixture.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default(),
        )
        .expect("trusted graph");
        let relation = graph
            .edges
            .iter()
            .find(|edge| edge.graph.kind == "archaeology_depends_on")
            .expect("dependency");
        assert_eq!(relation.archaeology.origin, ArchaeologyTrust::Deterministic);
        assert_eq!(relation.graph.origin, GraphOrigin::Deterministic);
        assert_eq!(relation.graph.trust, GraphTrust::Ambiguous);
        assert_eq!(
            relation.archaeology.confidence,
            Some(ArchaeologyConfidence::Low)
        );
        assert!(relation.archaeology.limitations.iter().any(|limitation| {
            limitation == "endpoint trust limits this relationship to ambiguous navigation"
        }));
        assert!(relation
            .archaeology
            .evidence_ids
            .contains(&"relation:dependency".into()));

        let mut missing_persisted_relation = fixture.clone();
        missing_persisted_relation.rule_relations.clear();
        assert!(project_archaeology_graph_fragment(
            missing_persisted_relation.input(),
            &StructuralGraphCancellation::default(),
            ArchaeologyGraphLimits::default()
        )
        .unwrap_err()
        .contains("relation parity"));
    }

    #[test]
    fn graph_origins_round_trip_without_upgrading_unknown_values() {
        for (origin, stored) in [
            (GraphOrigin::Extracted, "extracted"),
            (GraphOrigin::Deterministic, "deterministic"),
            (GraphOrigin::ModelSynthesized, "model_synthesized"),
            (GraphOrigin::HumanConfirmed, "human_confirmed"),
        ] {
            assert_eq!(origin.as_str(), stored);
            assert_eq!(GraphOrigin::from_storage(stored), origin);
        }
        assert_eq!(
            GraphOrigin::from_storage("future-origin"),
            GraphOrigin::LegacyMetadata
        );
    }

    fn fixture() -> Fixture {
        let repository_id = "repository:payments".to_string();
        let generation_id = "generation:one".to_string();
        let revision_sha = "a".repeat(40);
        let coverage = ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Partial,
            parser_coverage: ArchaeologyCoverageState::Complete,
            repository_coverage: ArchaeologyCoverageState::Partial,
            temporal_coverage: ArchaeologyCoverageState::Unavailable,
            discovered_source_units: 1,
            indexed_source_units: 1,
            discovered_bytes: 120,
            indexed_bytes: 120,
            reasons: vec!["fixture is partial".into()],
        };
        let source_units = vec![ArchaeologyInventoryUnit {
            identity: ArchaeologySourceUnitIdentity {
                source_unit_id: "unit:payments".into(),
                repository_id: repository_id.clone(),
                revision_sha: revision_sha.clone(),
                path_identity: "path:opaque".into(),
                relative_path: Some("/Users/private/source.cbl".into()),
                content_hash: Some("hash:one".into()),
                hash_algorithm: Some("sha256".into()),
                change_identity: None,
            },
            classification: ArchaeologySourceClassification::Source,
            language: "cobol".into(),
            dialect: Some("fixed".into()),
            byte_count: 120,
            line_count: 6,
            include_candidates: Vec::new(),
            coverage_reasons: vec!["exact parser coverage".into()],
        }];
        let span = |id: &str, line: u64| ArchaeologySourceSpan {
            span_id: id.into(),
            source_unit_id: "unit:payments".into(),
            revision_sha: revision_sha.clone(),
            start: ArchaeologyPosition {
                byte: line * 10,
                line,
                column: 1,
            },
            end: ArchaeologyPosition {
                byte: line * 10 + 8,
                line,
                column: 9,
            },
        };
        let spans = vec![
            span("span:predicate", 2),
            span("span:mutation", 3),
            span("span:data", 4),
        ];
        let fact =
            |id: &str, kind: ArchaeologyFactKind, label: &str, span_id: &str| ArchaeologyFact {
                fact_id: id.into(),
                kind,
                label: label.into(),
                span_ids: vec![span_id.into()],
                parser_id: "parser:cobol-v2".into(),
                trust: ArchaeologyTrust::Extracted,
                confidence: ArchaeologyConfidence::High,
                attributes: Vec::<ArchaeologyAttribute>::new(),
            };
        let facts = vec![
            fact(
                "fact:predicate",
                ArchaeologyFactKind::Predicate,
                "positive amount",
                "span:predicate",
            ),
            fact(
                "fact:mutation",
                ArchaeologyFactKind::Mutation,
                "schedule payment",
                "span:mutation",
            ),
            fact(
                "fact:data",
                ArchaeologyFactKind::DataField,
                "payment amount",
                "span:data",
            ),
        ];
        let origins = [
            ("fact:predicate", "path:predicate"),
            ("fact:mutation", "path:mutation"),
            ("fact:data", "path:data"),
        ]
        .into_iter()
        .map(|(fact_id, path_identity)| ArchaeologyFactOrigin {
            fact_id: fact_id.into(),
            source_unit_id: "unit:payments".into(),
            path_identity: path_identity.into(),
            ranking_path_identity: stable_graph_id("archaeology-ranking-path", path_identity),
            classification: ArchaeologySourceClassification::Source,
        })
        .collect();
        let fact_edges = vec![
            ArchaeologyFactEdge {
                edge_id: "edge:controls".into(),
                from_fact_id: "fact:predicate".into(),
                to_fact_id: "fact:mutation".into(),
                kind: ArchaeologyFactEdgeKind::Controls,
                trust: ArchaeologyTrust::Extracted,
                evidence_span_ids: vec!["span:predicate".into(), "span:mutation".into()],
                unresolved_reason: None,
            },
            ArchaeologyFactEdge {
                edge_id: "edge:writes".into(),
                from_fact_id: "fact:mutation".into(),
                to_fact_id: "fact:data".into(),
                kind: ArchaeologyFactEdgeKind::Writes,
                trust: ArchaeologyTrust::Extracted,
                evidence_span_ids: vec!["span:mutation".into(), "span:data".into()],
                unresolved_reason: None,
            },
        ];
        let rules = vec![ArchaeologyRulePacket {
            rule_id: "rule:payment".into(),
            repository_id: repository_id.clone(),
            generation_id: generation_id.clone(),
            revision_sha: revision_sha.clone(),
            kind: ArchaeologyRuleKind::Validation,
            title: "Positive payments are scheduled".into(),
            domain_ids: vec!["domain:other".into()],
            lifecycle: ArchaeologyRuleLifecycle::Candidate,
            trust: ArchaeologyTrust::Deterministic,
            confidence: ArchaeologyConfidence::High,
            clauses: vec![ArchaeologyRuleClause {
                clause_id: "clause:payment".into(),
                text: "A positive payment amount schedules a payment.".into(),
                trust: ArchaeologyTrust::Deterministic,
                confidence: ArchaeologyConfidence::High,
                supporting_fact_ids: vec![
                    "fact:predicate".into(),
                    "fact:mutation".into(),
                    "fact:data".into(),
                ],
                contradicting_fact_ids: Vec::new(),
                evidence_span_ids: vec![
                    "span:predicate".into(),
                    "span:mutation".into(),
                    "span:data".into(),
                ],
                caveats: vec!["fixture caveat".into()],
            }],
            dependency_rule_ids: Vec::new(),
            conflict_rule_ids: Vec::new(),
            alias_rule_ids: Vec::new(),
            coverage: coverage.clone(),
            parser_identity: "parser:cobol-v2".into(),
            algorithm_identity: "algorithm:v1".into(),
            synthesis_identity: None,
        }];
        Fixture {
            repository_id,
            generation_id,
            revision_sha,
            coverage,
            source_units,
            spans,
            facts,
            origins,
            fact_edges,
            rules,
            domains: vec![ArchaeologyGraphDomain {
                domain_id: "domain:other".into(),
                label: "Other".into(),
                parent_domain_id: None,
            }],
            rule_relations: Vec::new(),
        }
    }
}
