//! Deterministic, zero-network template versus structured-synthesis qualification.

use super::adapter::semantic_expression;
use super::contracts::{
    ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyCoverage, ArchaeologyCoverageState,
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
    ArchaeologyRuleKind, ArchaeologyTrust, ARCHAEOLOGY_SCHEMA_VERSION,
    ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
};
use super::deterministic_rules::{
    derive_evidence_packets, render_template_rules, ArchaeologyDeterministicLimits,
};
use super::identity_store::refresh_rule_identities;
use super::jobs::{
    finalize_synthesis_catalog, ArchaeologyGenerationIdentity, ArchaeologyJobCheckpoint,
    ArchaeologySynthesisCatalogStage,
};
use super::synthesis::{
    build_synthesis_request, canonical_synthesis_clause_text, canonicalize_synthesis_response,
    ArchaeologySynthesisClause, ArchaeologySynthesisLimits, ArchaeologySynthesisQuantifier,
    ArchaeologySynthesisQuantifierKind, ArchaeologySynthesisRequest, ArchaeologySynthesisResponse,
    ArchaeologySynthesisSegment, ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID,
};
use super::synthesis_runtime::{
    invoke_synthesis_plan, permit_validated_qualification_fixture, prepare_synthesis_plan,
    ArchaeologyAttemptRecorder, ArchaeologyCostClass, ArchaeologyNetworkScope,
    ArchaeologyProviderDescriptor, ArchaeologyProviderExecutionBounds, ArchaeologyProviderKind,
    ArchaeologyProviderOutput, ArchaeologyProviderRequest, ArchaeologyProviderSelection,
    ArchaeologyProviderUsage, ArchaeologySynthesisAttempt, ArchaeologySynthesisProvider,
    ArchaeologyUsageSource, ProviderFuture,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use crate::db::archaeology_schema;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const REPO: &str = "repository:qualification-comparison";
const PARSER: &str = "qualification-corpus-v1";
const PARSER_MANIFEST: &str =
    "parser-manifest:v1:qualification-corpus-v1@1,unavailable@unavailable";
const ALGORITHM: &str = "algorithm:qualification-comparison-v1";
const SOURCE: &str = "source:qualification-comparison-v1";
const CONFIG: &str = "config:qualification-comparison-v1";
const NOW: &str = "2026-07-17T00:00:00.000Z";
const RATE: u64 = 1_000_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenFact {
    id: String,
    kind: ArchaeologyFactKind,
    label: String,
    span_ids: Vec<String>,
    trust: ArchaeologyTrust,
    confidence: ArchaeologyConfidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenEdge {
    id: String,
    from: String,
    to: String,
    kind: ArchaeologyFactEdgeKind,
    span_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenRule {
    id: String,
    revision: String,
    kind: ArchaeologyRuleKind,
    lifecycle: String,
    primary: bool,
    alias_of: Option<String>,
    clauses: Vec<GoldenClause>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenClause {
    id: String,
    kind: String,
    text: String,
    supporting_fact_ids: Vec<String>,
    contradicting_fact_ids: Vec<String>,
    span_ids: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    schema_version: u32,
    fixture_id: String,
    corpus_id: String,
    provider: ProviderFixture,
    scope: ScopeFixture,
    cases: Vec<CaseFixture>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderFixture {
    provider_kind: String,
    provider_identity: String,
    model_identity: String,
    network_scope: String,
    cost_class: String,
    pricing_identity: Option<String>,
    input_tokens_per_call: u64,
    output_tokens_per_call: u64,
    reported_cost_microusd_per_call: u64,
    max_output_tokens: u64,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeFixture {
    primary_current_rule_ids: Vec<String>,
    generated_alias_rule_id: String,
    generated_alias_of_rule_id: String,
    historical_rule_id: String,
    covered_clause_shapes: Vec<String>,
    missing_clause_shapes: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseFixture {
    case_id: String,
    golden_rule_id: String,
    anchor_fact_id: String,
    subject_fact_ids: Vec<String>,
    condition_fact_ids: Option<Vec<String>>,
    action_fact_ids: Vec<String>,
    exception_fact_ids: Option<Vec<String>>,
    relationship_ids: Vec<String>,
    contradicting_fact_ids: Vec<String>,
    quantifier: Option<QuantifierFixture>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuantifierFixture {
    kind: ArchaeologySynthesisQuantifierKind,
    fact_ids: Vec<String>,
}

#[derive(Default)]
struct Acc {
    cases: u64,
    clauses: u64,
    supported: u64,
    correction: [u64; 4],
    calls: u64,
    attempts: u64,
    input_tokens: u64,
    output_tokens: u64,
    reported_cost: u64,
}

#[derive(Default)]
struct Recorder(Mutex<Vec<ArchaeologySynthesisAttempt>>);

impl ArchaeologyAttemptRecorder for Recorder {
    fn begin(&self, _: u8) -> Result<(), String> {
        Ok(())
    }
    fn finish(&self, attempt: &ArchaeologySynthesisAttempt) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "Qualification recorder lock is unavailable".to_string())?
            .push(attempt.clone());
        Ok(())
    }
}

struct MockProvider {
    descriptor: ArchaeologyProviderDescriptor,
    output: Vec<u8>,
    usage: ArchaeologyProviderUsage,
    calls: Arc<AtomicUsize>,
}

impl ArchaeologySynthesisProvider for MockProvider {
    fn descriptor(&self) -> &ArchaeologyProviderDescriptor {
        &self.descriptor
    }
    fn invoke(&self, _: ArchaeologyProviderRequest) -> ProviderFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let output = ArchaeologyProviderOutput {
            raw_output: self.output.clone(),
            usage: self.usage.clone(),
        };
        Box::pin(async move { Ok(output) })
    }
}

pub(crate) async fn evaluate(
    corpus_bytes: &[u8],
    fixture_bytes: &[u8],
    policy_bytes: &[u8],
) -> Result<Value, String> {
    for (name, bytes) in [
        ("corpus", corpus_bytes),
        ("fixture", fixture_bytes),
        ("policy", policy_bytes),
    ] {
        if bytes.is_empty() || bytes.len() > 1024 * 1024 {
            return Err(format!(
                "Archaeology comparison {name} exceeds its byte bound"
            ));
        }
    }
    let corpus: Value = strict(corpus_bytes, "corpus")?;
    exact_keys(
        &corpus,
        &[
            "schema_version",
            "corpus_id",
            "revisions",
            "source_units",
            "spans",
            "facts",
            "edges",
            "rules",
            "duplicate_groups",
            "conflicts",
            "gaps",
            "history_changes",
            "negative_cases",
        ],
        "corpus",
    )?;
    let fixture: Fixture = strict(fixture_bytes, "fixture")?;
    let policy: Value = strict(policy_bytes, "policy")?;
    exact_keys(
        &policy,
        &[
            "schema_version",
            "policy_id",
            "policy_version",
            "status",
            "evidence_references",
            "required_dialect_constructs",
            "semantic_hard_gates",
            "named_machine_budgets",
            "safety_hard_gates",
            "claim_ceiling",
            "change_control",
        ],
        "policy",
    )?;
    let facts: Vec<GoldenFact> = value(&corpus, "facts")?;
    let edges: Vec<GoldenEdge> = value(&corpus, "edges")?;
    let rules: Vec<GoldenRule> = value(&corpus, "rules")?;
    let current = text(&corpus["revisions"], "current")?;
    validate_scope(&corpus, &fixture, &rules)?;
    let support_min = policy_rate(&policy, "clause_support_rate_min")?;
    let unsupported_max = policy_rate(&policy, "unsupported_clause_rate_max")?;

    let facts = facts
        .into_iter()
        .map(|fact| {
            let attributes = if fact.kind == ArchaeologyFactKind::Unresolved {
                vec![]
            } else {
                vec![ArchaeologyAttribute {
                    key: "semantic_expr".into(),
                    value: semantic_expression(&fact.label, false)?,
                }]
            };
            Ok((
                fact.id.clone(),
                ArchaeologyFact {
                    fact_id: fact.id,
                    kind: fact.kind,
                    label: fact.label,
                    span_ids: fact.span_ids,
                    parser_id: PARSER.into(),
                    trust: fact.trust,
                    confidence: fact.confidence,
                    attributes,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let edges = edges
        .into_iter()
        .map(|edge| {
            (
                edge.id.clone(),
                ArchaeologyFactEdge {
                    edge_id: edge.id,
                    from_fact_id: edge.from,
                    to_fact_id: edge.to,
                    kind: edge.kind,
                    trust: ArchaeologyTrust::Deterministic,
                    evidence_span_ids: edge.span_ids,
                    unresolved_reason: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let rules = rules
        .into_iter()
        .map(|rule| (rule.id.clone(), rule))
        .collect::<BTreeMap<_, _>>();
    let mut cases = fixture.cases.clone();
    cases.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    let mut deterministic = Acc::default();
    let mut synthesis = Acc::default();
    let mut case_reports = Vec::new();

    for case in &cases {
        let golden = rules
            .get(&case.golden_rule_id)
            .ok_or("Unknown golden rule")?;
        let case_edges = case
            .relationship_ids
            .iter()
            .map(|id| edges.get(id).cloned().ok_or("Unknown fixture relationship"))
            .collect::<Result<Vec<_>, _>>()?;
        let ids = case_fact_ids(case, &case_edges);
        let case_facts = ids
            .iter()
            .map(|id| facts.get(*id).cloned().ok_or("Unknown fixture fact"))
            .collect::<Result<Vec<_>, _>>()?;
        let cancellation = StructuralGraphCancellation::default();
        let packet = derive_evidence_packets(
            REPO,
            current,
            &case_facts,
            &case_edges,
            &cancellation,
            Default::default(),
        )?
        .into_iter()
        .find(|packet| packet.anchor_fact_id == case.anchor_fact_id)
        .ok_or("Fixture anchor produced no packet")?;
        let packet_ids = packet
            .supporting_fact_ids
            .iter()
            .chain(&packet.contradicting_fact_ids)
            .chain(&packet.unresolved_fact_ids)
            .collect::<BTreeSet<_>>();
        let packet_facts = case_facts
            .into_iter()
            .filter(|fact| packet_ids.contains(&fact.fact_id))
            .collect::<Vec<_>>();
        let packet_edges = case_edges
            .into_iter()
            .filter(|edge| packet.relationship_ids.contains(&edge.edge_id))
            .collect::<Vec<_>>();
        validate_case(
            case,
            golden,
            &packet.supporting_fact_ids,
            &packet.relationship_ids,
        )?;
        let generation = format!("generation:qualification:{}", case.case_id);
        let template = render_template_rules(
            REPO,
            &generation,
            current,
            std::slice::from_ref(&packet),
            &packet_facts,
            &packet_edges,
            &Default::default(),
            PARSER_MANIFEST,
            ALGORITHM,
            &cancellation,
            ArchaeologyDeterministicLimits::default(),
        )?
        .remove(0);
        let allowed = golden
            .clauses
            .iter()
            .flat_map(|clause| &clause.supporting_fact_ids)
            .collect::<BTreeSet<_>>();
        let template_supported = template
            .clauses
            .iter()
            .filter(|clause| {
                clause.validate().is_ok()
                    && clause
                        .supporting_fact_ids
                        .iter()
                        .all(|id| allowed.contains(id))
            })
            .count() as u64;
        let golden_text = golden
            .clauses
            .iter()
            .map(|clause| clause.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let template_text = template
            .clauses
            .iter()
            .map(|clause| clause.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let template_delta = edit_delta(&template_text, &golden_text);

        let request = build_synthesis_request(
            REPO,
            &generation,
            current,
            PARSER_MANIFEST,
            ALGORITHM,
            &packet,
            &packet_facts,
            &packet_edges,
            &cancellation,
            Default::default(),
        )?;
        let response = response(case, &request)?;
        canonicalize_synthesis_response(&request, &response, Default::default())
            .map_err(|error| format!("{}: {error}", case.case_id))?;
        let selection = selection(&fixture.provider);
        let descriptor = descriptor();
        let plan = prepare_synthesis_plan(&request, &selection, &descriptor, Default::default())?;
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(MockProvider {
            descriptor,
            output: serde_json::to_vec(&response).map_err(|_| "Encode mock response")?,
            usage: ArchaeologyProviderUsage {
                input_tokens: Some(fixture.provider.input_tokens_per_call),
                cached_input_tokens: Some(0),
                output_tokens: Some(fixture.provider.output_tokens_per_call),
                reported_cost_microusd: Some(0),
                estimated_cost_microusd: None,
                usage_source: ArchaeologyUsageSource::Reported,
                pricing_identity: None,
            },
            calls: calls.clone(),
        });
        let run = invoke_synthesis_plan(
            provider,
            &request,
            &plan,
            &permit_validated_qualification_fixture(&plan),
            Arc::new(Recorder::default()),
            &selection,
            1,
            &cancellation,
            ArchaeologySynthesisLimits::default(),
        )
        .await
        .map_err(|(error, _)| error)?;
        let canonical =
            canonicalize_synthesis_response(&request, &run.response, Default::default())?;
        let synthesis_text = canonical
            .clauses
            .iter()
            .map(|clause| canonical_synthesis_clause_text(&request, clause))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        let synthesis_delta = edit_delta(&synthesis_text, &golden_text);

        add(
            &mut deterministic,
            template.clauses.len() as u64,
            template_supported,
            template_delta,
            &[],
            0,
        );
        add(
            &mut synthesis,
            canonical.clauses.len() as u64,
            canonical.clauses.len() as u64,
            synthesis_delta,
            &run.attempts,
            calls.load(Ordering::SeqCst) as u64,
        );
        case_reports.push(json!({
            "case_id": case.case_id,
            "golden_rule_id": case.golden_rule_id,
            "golden_rule_kind": enum_text(&golden.kind)?,
            "deterministic_rule_kind": enum_text(&packet.kind)?,
            "rule_kind_match": packet.kind == golden.kind,
            "deterministic_clause_count": template.clauses.len(),
            "synthesis_clause_count": canonical.clauses.len(),
            "deterministic_supported_clause_count": template_supported,
            "synthesis_supported_clause_count": canonical.clauses.len(),
            "deterministic_correction": delta_json(template_delta),
            "synthesis_correction": delta_json(synthesis_delta),
        }));
    }
    let deterministic = variant("deterministic_template", deterministic, None);
    let synthesis = variant(
        "mock_structured_synthesis",
        synthesis,
        fixture.provider.pricing_identity.clone(),
    );
    let zero = zero_model_proof()?;
    let gates = gates(
        &deterministic,
        &synthesis,
        &zero,
        support_min,
        unsupported_max,
    )?;
    if !gates["comparison_gate_pass"].as_bool().unwrap_or(false) {
        return Err("Comparison clause gate failed".into());
    }
    Ok(json!({
        "schema_version": 1,
        "report_id": "business-rule-archaeology-template-model-comparison-v1",
        "input_identities": {
            "corpus": hash(corpus_bytes),
            "synthesis_fixture": hash(fixture_bytes),
            "qualification_policy": hash(policy_bytes),
        },
        "scope": {
            "corpus_id": fixture.corpus_id,
            "primary_current_rule_ids": fixture.scope.primary_current_rule_ids,
            "primary_current_cases": cases.len(),
            "generated_alias_rule_id": fixture.scope.generated_alias_rule_id,
            "generated_alias_of_rule_id": fixture.scope.generated_alias_of_rule_id,
            "generated_alias_cases": 1,
            "historical_rule_id": fixture.scope.historical_rule_id,
            "historical_cases": 1,
            "reconciled_rule_total": rules.len(),
            "covered_clause_shapes": fixture.scope.covered_clause_shapes,
            "missing_clause_shapes": fixture.scope.missing_clause_shapes,
        },
        "policy": {
            "policy_id": policy["policy_id"],
            "policy_version": policy["policy_version"],
            "clause_support_rate_min_millionths": support_min,
            "unsupported_clause_rate_max_millionths": unsupported_max,
        },
        "variants": [deterministic, synthesis],
        "cases": case_reports,
        "zero_model_catalog": zero,
        "gates": gates,
        "limitations": [
            "The synthesis variant is a deterministic no-network mock, not a live model evaluation.",
            "The corpus has no labeled quantifier case, so quantifier support is unqualified.",
            "The six mock cases measure cited-clause support precision, not contradiction completeness or recall.",
            "The generated listing is reconciled as an alias and is not counted as an independent primary rule.",
            "The prior payment rule is reconciled as historical and is not counted as a current rule.",
            "Text edit distance is deterministic comparison evidence, not measured human reviewer effort.",
            "This six-case fixture is not full correctness, scale, resource, retrieval, or external-model qualification.",
        ],
        "full_qualification": false,
    }))
}

fn validate_scope(corpus: &Value, fixture: &Fixture, rules: &[GoldenRule]) -> Result<(), String> {
    if fixture.schema_version != 1
        || fixture.fixture_id != "business-rule-archaeology-model-comparison-v1"
        || fixture.corpus_id != text(corpus, "corpus_id")?
        || fixture.provider.provider_kind != "mock"
        || fixture.provider.provider_identity != "local"
        || fixture.provider.network_scope != "none"
        || fixture.provider.cost_class != "free"
        || fixture.provider.pricing_identity.is_some()
        || fixture.provider.reported_cost_microusd_per_call != 0
        || fixture.provider.input_tokens_per_call == 0
        || fixture.provider.output_tokens_per_call == 0
        || fixture.provider.output_tokens_per_call > fixture.provider.max_output_tokens
        || fixture.scope.missing_clause_shapes != ["quantifier"]
        || fixture.scope.covered_clause_shapes != ["subject", "condition", "action", "exception"]
    {
        return Err("Invalid comparison identity or bounds".into());
    }
    let primary = fixture
        .scope
        .primary_current_rule_ids
        .iter()
        .collect::<BTreeSet<_>>();
    let cases = fixture
        .cases
        .iter()
        .map(|case| &case.golden_rule_id)
        .collect::<BTreeSet<_>>();
    if primary.len() != 6 || cases != primary || rules.len() != 9 {
        return Err("Comparison case accounting does not reconcile".into());
    }
    for rule in rules {
        if rule.lifecycle.is_empty()
            || rule.clauses.is_empty()
            || rule.clauses.iter().any(|clause| {
                clause.id.is_empty()
                    || clause.kind.is_empty()
                    || clause.text.is_empty()
                    || clause.supporting_fact_ids.is_empty()
                    || clause.span_ids.is_empty()
                    || clause
                        .contradicting_fact_ids
                        .iter()
                        .any(|id| clause.supporting_fact_ids.contains(id))
            })
        {
            return Err("Malformed golden rule".into());
        }
        if primary.contains(&rule.id)
            && (!rule.primary || rule.alias_of.is_some() || rule.revision != "current")
        {
            return Err("Primary rule is not current".into());
        }
    }
    let alias = rules
        .iter()
        .find(|rule| rule.id == fixture.scope.generated_alias_rule_id)
        .ok_or("Missing alias")?;
    let history = rules
        .iter()
        .find(|rule| rule.id == fixture.scope.historical_rule_id)
        .ok_or("Missing history")?;
    if alias.primary
        || alias.alias_of.as_ref() != Some(&fixture.scope.generated_alias_of_rule_id)
        || history.revision != "previous"
        || !history.primary
    {
        return Err("Alias/history accounting is invalid".into());
    }
    Ok(())
}

fn case_fact_ids<'a>(case: &'a CaseFixture, edges: &'a [ArchaeologyFactEdge]) -> BTreeSet<&'a str> {
    let mut ids = BTreeSet::from([case.anchor_fact_id.as_str()]);
    ids.extend(
        case.subject_fact_ids
            .iter()
            .chain(&case.action_fact_ids)
            .chain(case.condition_fact_ids.iter().flatten())
            .chain(case.exception_fact_ids.iter().flatten())
            .chain(&case.contradicting_fact_ids)
            .chain(case.quantifier.iter().flat_map(|q| &q.fact_ids))
            .map(String::as_str),
    );
    for edge in edges {
        ids.insert(&edge.from_fact_id);
        ids.insert(&edge.to_fact_id);
    }
    ids
}

fn validate_case(
    case: &CaseFixture,
    golden: &GoldenRule,
    packet_facts: &[String],
    packet_edges: &[String],
) -> Result<(), String> {
    let allowed = golden
        .clauses
        .iter()
        .flat_map(|clause| &clause.supporting_fact_ids)
        .collect::<BTreeSet<_>>();
    let cited = case
        .subject_fact_ids
        .iter()
        .chain(&case.action_fact_ids)
        .chain(case.condition_fact_ids.iter().flatten())
        .chain(case.exception_fact_ids.iter().flatten())
        .chain(case.quantifier.iter().flat_map(|q| &q.fact_ids));
    if cited.clone().any(|id| !allowed.contains(id))
        || cited.clone().any(|id| !packet_facts.contains(id))
        || case
            .relationship_ids
            .iter()
            .any(|id| !packet_edges.contains(id))
    {
        return Err(format!("{} is outside its golden packet", case.case_id));
    }
    Ok(())
}

fn response(
    case: &CaseFixture,
    request: &ArchaeologySynthesisRequest,
) -> Result<ArchaeologySynthesisResponse, String> {
    let segment = |ids: &[String]| -> Result<ArchaeologySynthesisSegment, String> {
        let mut ids = ids.to_vec();
        ids.sort();
        let labels = ids
            .iter()
            .map(|id| {
                request
                    .facts
                    .iter()
                    .find(|fact| fact.fact_id == *id)
                    .map(|fact| fact.label.as_str())
                    .ok_or("Response fact is outside request")
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ArchaeologySynthesisSegment {
            text: labels.join(" "),
            fact_ids: ids,
        })
    };
    let mut relationships = case.relationship_ids.clone();
    relationships.sort();
    let mut contradicting = case.contradicting_fact_ids.clone();
    contradicting.sort();
    Ok(ArchaeologySynthesisResponse {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        contract_id: ARCHAEOLOGY_SYNTHESIS_CONTRACT_ID.into(),
        request_id: request.request_id.clone(),
        packet_id: request.packet.packet_id.clone(),
        clauses: vec![ArchaeologySynthesisClause {
            subject: segment(&case.subject_fact_ids)?,
            condition: case
                .condition_fact_ids
                .as_deref()
                .map(segment)
                .transpose()?,
            action: segment(&case.action_fact_ids)?,
            exception: case
                .exception_fact_ids
                .as_deref()
                .map(segment)
                .transpose()?,
            quantifier: case.quantifier.as_ref().map(|q| {
                let mut ids = q.fact_ids.clone();
                ids.sort();
                ArchaeologySynthesisQuantifier {
                    kind: q.kind,
                    fact_ids: ids,
                }
            }),
            relationship_ids: relationships,
            contradicting_fact_ids: contradicting,
        }],
    })
}

fn descriptor() -> ArchaeologyProviderDescriptor {
    ArchaeologyProviderDescriptor {
        kind: ArchaeologyProviderKind::Local,
        provider_identity: "local".into(),
        endpoint: "http://127.0.0.1:1/v1/chat/completions".into(),
        network_scope: ArchaeologyNetworkScope::Loopback,
    }
}
fn selection(provider: &ProviderFixture) -> ArchaeologyProviderSelection {
    ArchaeologyProviderSelection {
        enabled: true,
        provider_identity: provider.provider_identity.clone(),
        model_identity: provider.model_identity.clone(),
        cost_class: ArchaeologyCostClass::Free,
        pricing: None,
        remote_approved: false,
        remote_disclosure_version: None,
        paid_approved: false,
        paid_disclosure_version: None,
        execution: ArchaeologyProviderExecutionBounds {
            total_timeout_ms: 1000,
            attempt_timeout_ms: 1000,
            max_attempts: 1,
            max_output_tokens: provider.max_output_tokens,
        },
    }
}

fn add(
    acc: &mut Acc,
    clauses: u64,
    supported: u64,
    delta: [u64; 4],
    attempts: &[ArchaeologySynthesisAttempt],
    calls: u64,
) {
    acc.cases += 1;
    acc.clauses += clauses;
    acc.supported += supported;
    for (i, value) in delta.into_iter().enumerate() {
        acc.correction[i] += value;
    }
    acc.calls += calls;
    acc.attempts += attempts.len() as u64;
    for attempt in attempts {
        acc.input_tokens += attempt.usage.input_tokens.unwrap_or(0);
        acc.output_tokens += attempt.usage.output_tokens.unwrap_or(0);
        acc.reported_cost += attempt.usage.reported_cost_microusd.unwrap_or(0);
    }
}
fn variant(name: &str, acc: Acc, pricing: Option<String>) -> Value {
    let unsupported = acc.clauses.saturating_sub(acc.supported);
    json!({
        "variant": name,
        "case_count": acc.cases,
        "clause_count": acc.clauses,
        "supported_clause_count": acc.supported,
        "unsupported_clause_count": unsupported,
        "supported_clause_rate_millionths": ratio(acc.supported, acc.clauses),
        "unsupported_clause_rate_millionths": ratio(unsupported, acc.clauses),
        "correction_insertions": acc.correction[0],
        "correction_deletions": acc.correction[1],
        "correction_substitutions": acc.correction[2],
        "text_edit_distance": acc.correction[3],
        "mock_provider_calls": acc.calls,
        "external_model_calls": 0,
        "attempts": acc.attempts,
        "input_tokens": acc.input_tokens,
        "output_tokens": acc.output_tokens,
        "reported_cost_microusd": acc.reported_cost,
        "estimated_cost_microusd": 0,
        "pricing_identity": pricing,
    })
}
fn delta_json(delta: [u64; 4]) -> Value {
    json!({"insertions":delta[0],"deletions":delta[1],"substitutions":delta[2],"text_edit_distance":delta[3]})
}

fn gates(d: &Value, s: &Value, z: &Value, min: u64, max: u64) -> Result<Value, String> {
    let check = |v: &Value| -> Result<(bool, bool), String> {
        Ok((
            number(v, "supported_clause_rate_millionths")? >= min,
            number(v, "unsupported_clause_rate_millionths")? <= max,
        ))
    };
    let (d1, d2) = check(d)?;
    let (s1, s2) = check(s)?;
    let z1 = z["exact_rerun_parity"] == true
        && z["canonical_rule_rows"] == 1
        && z["manifest_rows"] == 1
        && z["fts_rows"] == 1
        && z["manifest_fts_exact_parity"] == true
        && z["provider_calls"] == 0
        && z["synthesis_attempt_rows"] == 0;
    Ok(json!({
        "deterministic_clause_support_pass": d1,
        "deterministic_unsupported_clause_pass": d2,
        "synthesis_clause_support_pass": s1,
        "synthesis_unsupported_clause_pass": s2,
        "zero_model_catalog_pass": z1,
        "comparison_gate_pass": d1 && d2 && s1 && s2 && z1,
    }))
}

fn zero_model_proof() -> Result<Value, String> {
    let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    archaeology_schema::run_migration(&db).map_err(|e| e.to_string())?;
    seed_zero_model_catalog(&db)?;
    let cancellation = StructuralGraphCancellation::default();
    let transaction = db.unchecked_transaction().map_err(|e| e.to_string())?;
    refresh_rule_identities(
        &transaction,
        "generation:zero",
        &["rule:zero".to_string()],
        &cancellation,
    )?;
    transaction.commit().map_err(|e| e.to_string())?;

    let input = || ArchaeologySynthesisCatalogStage {
        job_id: "job:zero",
        repository_id: REPO,
        generation_id: "generation:zero",
        owner_id: "owner:zero",
        identity: ArchaeologyGenerationIdentity {
            revision_sha: "dddddddddddddddddddddddddddddddddddddddd",
            source: SOURCE,
            parser: PARSER_MANIFEST,
            algorithm: ALGORITHM,
            config: CONFIG,
        },
        cancellation: &cancellation,
        now: NOW,
    };
    let first = finalize_synthesis_catalog(&db, input())?;
    let rerun = finalize_synthesis_catalog(&db, input())?;
    let counts: (i64, i64, i64, i64) = db
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM archaeology_rules
                   WHERE generation_id='generation:zero'),
                (SELECT COUNT(*) FROM archaeology_rule_search_manifest
                   WHERE generation_id='generation:zero'),
                (SELECT COUNT(*) FROM archaeology_rule_fts
                   WHERE generation_id='generation:zero'),
                (SELECT COUNT(*) FROM archaeology_synthesis_attempts
                   WHERE generation_id='generation:zero')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|error| format!("Count zero-model catalog rows: {error}"))?;
    let parity = search_rows(&db, "archaeology_rule_search_manifest")?
        == search_rows(&db, "archaeology_rule_fts")?;

    Ok(json!({
        "first_receipt": first.checkpoint_identity,
        "rerun_receipt": rerun.checkpoint_identity,
        "exact_rerun_parity": first == rerun,
        "canonical_rule_rows": counts.0,
        "manifest_rows": counts.1,
        "fts_rows": counts.2,
        "manifest_fts_exact_parity": parity,
        "provider_calls": 0,
        "synthesis_attempt_rows": counts.3,
    }))
}

fn zero_model_coverage() -> Result<String, String> {
    serde_json::to_string(&ArchaeologyCoverage {
        state: ArchaeologyCoverageState::Complete,
        parser_coverage: ArchaeologyCoverageState::Complete,
        repository_coverage: ArchaeologyCoverageState::Complete,
        temporal_coverage: ArchaeologyCoverageState::Complete,
        discovered_source_units: 1,
        indexed_source_units: 1,
        discovered_bytes: 20,
        indexed_bytes: 20,
        reasons: vec![],
    })
    .map_err(|error| format!("Encode zero-model coverage: {error}"))
}

fn seed_zero_model_catalog(db: &Connection) -> Result<(), String> {
    const REVISION: &str = "dddddddddddddddddddddddddddddddddddddddd";
    const DERIVE_RECEIPT: &str = "checkpoint:derive:zero";

    let coverage = zero_model_coverage()?;
    let checkpoint = serde_json::to_string(&ArchaeologyJobCheckpoint {
        cursor_identity: Some(DERIVE_RECEIPT.into()),
        counters: BTreeMap::from([
            ("derive_complete".into(), 1),
            ("evidence_packets".into(), 1),
            ("deterministic_rules".into(), 1),
            ("deterministic_clauses".into(), 1),
            ("cluster_primary_rules".into(), 1),
            ("cluster_alias_rules".into(), 0),
            ("cluster_conflict_pairs".into(), 0),
            ("domain_other_rules".into(), 1),
        ]),
        ..ArchaeologyJobCheckpoint::default()
    })
    .map_err(|error| format!("Encode zero-model derive checkpoint: {error}"))?;

    db.execute(
        "INSERT INTO archaeology_repositories
         (repository_id,repo_path,source_identity,current_revision,created_at,updated_at)
         VALUES (?1,'qualification://zero',?2,?3,?4,?4)",
        params![REPO, SOURCE, REVISION, NOW],
    )
    .map_err(|error| format!("Seed zero-model repository: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_generations
         (generation_id,repository_id,schema_version,revision_sha,source_identity,
          parser_identity,algorithm_identity,config_identity,status,coverage_json,created_at)
         VALUES ('generation:zero',?1,?2,?3,?4,?5,?6,?7,'staging',?8,?9)",
        params![
            REPO,
            ARCHAEOLOGY_STORAGE_SCHEMA_VERSION,
            REVISION,
            SOURCE,
            PARSER_MANIFEST,
            ALGORITHM,
            CONFIG,
            coverage,
            NOW,
        ],
    )
    .map_err(|error| format!("Seed zero-model generation: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_jobs
         (job_id,repository_id,generation_id,owner_id,stage,state,checkpoint_identity,
          checkpoint_json,completed_units,total_units,updated_at)
         VALUES ('job:zero',?1,'generation:zero','owner:zero','synthesize','running',
                 ?2,?3,1,1,?4)",
        params![REPO, DERIVE_RECEIPT, checkpoint, NOW],
    )
    .map_err(|error| format!("Seed zero-model job: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_source_units
         (generation_id,source_unit_id,path_identity,relative_path,content_hash,hash_algorithm,
          language,parser_id,parser_version,classification,byte_count,line_count,coverage_json)
         VALUES ('generation:zero','unit:zero','path:zero','fixture/zero.cbl',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','sha256',
                 'cobol',?1,'1','source',20,1,?2)",
        params![PARSER, coverage],
    )
    .map_err(|error| format!("Seed zero-model source unit: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_source_spans
         (generation_id,span_id,source_unit_id,revision_sha,start_byte,end_byte,
          start_line,start_column,end_line,end_column)
         VALUES ('generation:zero','span:zero','unit:zero',?1,0,20,1,1,1,21)",
        [REVISION],
    )
    .map_err(|error| format!("Seed zero-model source span: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_facts
         (generation_id,fact_id,kind,label,parser_id,trust,confidence,attributes_json)
         VALUES ('generation:zero','fact:zero','predicate','AMOUNT POSITIVE',?1,
                 'extracted','high',
                 '[{\"key\":\"semantic_expr\",\"value\":\"v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}]')",
        [PARSER],
    )
    .map_err(|error| format!("Seed zero-model fact: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_evidence_links
         (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
         VALUES ('generation:zero','fact','fact:zero','span','span:zero','supporting')",
        [],
    )
    .map_err(|error| format!("Seed zero-model fact evidence: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_rules
         (generation_id,rule_id,repository_id,revision_sha,kind,title,lifecycle,trust,
          confidence,parser_identity,algorithm_identity,coverage_json,created_at)
         VALUES ('generation:zero','rule:zero',?1,?2,'validation','Positive amount',
                 'candidate','deterministic','high',?3,?4,?5,?6)",
        params![REPO, REVISION, PARSER_MANIFEST, ALGORITHM, coverage, NOW],
    )
    .map_err(|error| format!("Seed zero-model rule: {error}"))?;
    db.execute(
        "INSERT INTO archaeology_rule_clauses
         (generation_id,rule_id,clause_id,ordinal,clause_text,trust,confidence,caveats_json)
         VALUES ('generation:zero','rule:zero','clause:zero',0,'Amount is positive.',
                 'deterministic','high','[]')",
        [],
    )
    .map_err(|error| format!("Seed zero-model clause: {error}"))?;
    for (kind, evidence) in [("fact", "fact:zero"), ("span", "span:zero")] {
        db.execute(
            "INSERT INTO archaeology_evidence_links
             (generation_id,owner_kind,owner_id,evidence_kind,evidence_id,role)
             VALUES ('generation:zero','rule_clause','clause:zero',?1,?2,'supporting')",
            params![kind, evidence],
        )
        .map_err(|error| format!("Seed zero-model clause evidence: {error}"))?;
    }
    db.execute(
        "INSERT INTO archaeology_rule_domains
         (generation_id,rule_id,domain_id,domain_label)
         VALUES ('generation:zero','rule:zero','domain:other','Other')",
        [],
    )
    .map_err(|error| format!("Seed zero-model domain: {error}"))?;
    Ok(())
}

fn search_rows(
    db: &Connection,
    table: &str,
) -> Result<Vec<(String, String, String, String)>, String> {
    if !matches!(
        table,
        "archaeology_rule_search_manifest" | "archaeology_rule_fts"
    ) {
        return Err("Unknown zero-model search projection".into());
    }
    let sql = format!(
        "SELECT rule_id,title,clause_text,domain_text FROM {table}
         WHERE generation_id='generation:zero' ORDER BY rule_id"
    );
    let mut statement = db
        .prepare(&sql)
        .map_err(|error| format!("Prepare zero-model search projection: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|error| format!("Query zero-model search projection: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read zero-model search projection: {error}"))
}

fn edit_delta(actual: &str, expected: &str) -> [u64; 4] {
    let a = actual.chars().collect::<Vec<_>>();
    let b = expected.chars().collect::<Vec<_>>();
    let mut m = vec![vec![[0; 4]; b.len() + 1]; a.len() + 1];
    for (i, row) in m.iter_mut().enumerate().skip(1) {
        row[0] = [0, i as u64, 0, i as u64]
    }
    for (j, cell) in m[0].iter_mut().enumerate().skip(1) {
        *cell = [j as u64, 0, 0, j as u64]
    }
    let mut i = 1;
    while i <= a.len() {
        let mut j = 1;
        while j <= b.len() {
            if a[i - 1] == b[j - 1] {
                m[i][j] = m[i - 1][j - 1]
            } else {
                let mut ins = m[i][j - 1];
                ins[0] += 1;
                ins[3] += 1;
                let mut del = m[i - 1][j];
                del[1] += 1;
                del[3] += 1;
                let mut sub = m[i - 1][j - 1];
                sub[2] += 1;
                sub[3] += 1;
                m[i][j] = [sub, del, ins]
                    .into_iter()
                    .min_by_key(|v| (v[3], v[2], v[1], v[0]))
                    .unwrap()
            }
            j += 1;
        }
        i += 1;
    }
    m[a.len()][b.len()]
}

pub(crate) fn encode(report: &Value) -> Result<Vec<u8>, String> {
    validate_report(report)?;
    let mut bytes = serde_json::to_vec_pretty(report).map_err(|_| "Encode comparison report")?;
    bytes.push(b'\n');
    Ok(bytes)
}
pub(crate) fn validate_report(report: &Value) -> Result<(), String> {
    exact_keys(
        report,
        &[
            "schema_version",
            "report_id",
            "input_identities",
            "scope",
            "policy",
            "variants",
            "cases",
            "zero_model_catalog",
            "gates",
            "limitations",
            "full_qualification",
        ],
        "report",
    )?;
    if report["schema_version"] != 1
        || report["report_id"] != "business-rule-archaeology-template-model-comparison-v1"
        || report["full_qualification"] != false
    {
        return Err("Invalid comparison report identity".into());
    }
    Ok(())
}
fn exact_keys(value: &Value, expected: &[&str], label: &str) -> Result<(), String> {
    let actual = value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))?
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected.iter().copied().collect() {
        return Err(format!("Archaeology {label} has unknown or missing fields"));
    }
    Ok(())
}
fn strict<T: for<'de> Deserialize<'de>>(bytes: &[u8], label: &str) -> Result<T, String> {
    serde_json::from_slice(bytes).map_err(|_| format!("Archaeology {label} is not strict JSON"))
}
fn value<T: for<'de> Deserialize<'de>>(root: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(root[key].clone()).map_err(|_| format!("Invalid corpus {key}"))
}
fn text<'a>(root: &'a Value, key: &str) -> Result<&'a str, String> {
    root[key]
        .as_str()
        .ok_or_else(|| format!("Missing text {key}"))
}
fn number(root: &Value, key: &str) -> Result<u64, String> {
    root[key]
        .as_u64()
        .ok_or_else(|| format!("Missing number {key}"))
}
fn policy_rate(policy: &Value, key: &str) -> Result<u64, String> {
    let value = policy
        .pointer(&format!("/semantic_hard_gates/{key}"))
        .and_then(Value::as_f64)
        .ok_or("Invalid policy rate")?;
    if !(0.0..=1.0).contains(&value) {
        return Err("Policy rate outside zero to one".into());
    }
    Ok((value * RATE as f64).round() as u64)
}
fn ratio(a: u64, b: u64) -> u64 {
    if b == 0 {
        0
    } else {
        ((u128::from(a) * u128::from(RATE)) / u128::from(b)) as u64
    }
}
fn enum_text(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .ok_or("Invalid enum".into())
}
fn hash(bytes: &[u8]) -> String {
    format!("sha256:{}", super::inventory::hex(&Sha256::digest(bytes)))
}

#[path = "qualification_comparison_tests.rs"]
mod tests;
