//! Evidence-traced business-rule archaeology.
//!
//! The first layer is deliberately transport-neutral: indexing, desktop IPC,
//! exports, and MCP must share one vocabulary before any parser is trusted.

pub mod adapter;
pub mod assembly_adapter;
pub mod cleanup_command;
pub mod cobol_adapter;
pub mod contracts;
#[allow(dead_code)]
mod deterministic_rules;
pub(crate) mod evidence_store;
#[allow(dead_code)]
mod graph;
#[allow(dead_code)]
pub(crate) mod identity;
#[allow(dead_code)]
pub(crate) mod identity_store;
#[cfg(test)]
mod identity_store_tests;
#[allow(dead_code)]
pub(crate) mod invalidation;
#[allow(dead_code)]
pub(crate) mod invalidation_store;
#[cfg(test)]
mod invalidation_store_tests;
#[cfg(test)]
mod invalidation_tests;
pub mod inventory;
#[allow(dead_code)]
pub(crate) mod lifecycle;
#[allow(dead_code)]
pub(crate) mod lifecycle_store;
#[allow(dead_code)]
pub(crate) mod temporal_store;
#[cfg(test)]
mod temporal_store_tests;
// The durable stage engine is incrementally exposed as the archaeology
// lifecycle lands; every path is covered by its module tests meanwhile.
pub mod export;
#[allow(dead_code)]
pub(crate) mod jobs;
pub(crate) mod legacy;
pub mod modern_adapter;
#[cfg(test)]
mod qualification_comparison;
#[allow(dead_code)]
pub mod read;
pub mod refresh_command;
pub mod repository_resolution;
pub mod review_command;
mod synthesis;
#[cfg(test)]
mod synthesis_adversarial_tests;
pub mod synthesis_command;
mod synthesis_runtime;

#[allow(dead_code)]
#[rustfmt::skip]
mod linker {
use super::adapter::{canonical_semantic_digest, ArchaeologyAdapterLineage, ArchaeologyLineageKind};
use super::contracts::{ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyFact, ArchaeologyFactEdge, ArchaeologyFactEdgeKind, ArchaeologyFactKind, ArchaeologySourceSpan, ArchaeologyTrust};
use crate::commands::structural_graph::types::{stable_graph_id, StructuralGraphCancellation};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchaeologyLinkLimits {
    pub max_units: usize, pub max_facts: usize, pub max_edges: usize, pub max_references: usize,
    pub max_candidates_per_reference: usize, pub max_output_edges: usize,
    pub max_input_bytes: usize, pub max_output_items: usize, pub max_output_bytes: usize,
}
impl Default for ArchaeologyLinkLimits {
    fn default() -> Self { Self { max_units: 250_000, max_facts: 100_000, max_edges: 100_000, max_references: 50_000,
        max_candidates_per_reference: 64, max_output_edges: 100_000, max_output_items: 500_000,
        max_input_bytes: 256 * 1024 * 1024, max_output_bytes: 64 * 1024 * 1024 } }
}
pub(crate) struct ArchaeologyLinkUnit<'a> {
    pub source_unit_id: &'a str, pub language: &'a str, pub dialect: Option<&'a str>, pub relative_path: Option<&'a str>, pub lineage: &'a [ArchaeologyAdapterLineage],
}
pub(crate) struct ArchaeologyLinkFact<'a> {
    pub source_unit_id: &'a str, pub fact: &'a ArchaeologyFact, pub evidence_spans: &'a [ArchaeologySourceSpan],
}
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct ArchaeologyLinkPatch {
    pub remove_fact_ids: Vec<String>, pub remove_edge_ids: Vec<String>, pub upsert_facts: Vec<ArchaeologyFact>, pub upsert_edges: Vec<ArchaeologyFactEdge>,
    pub evidence: Vec<(String, String, String)>, pub lineage: Vec<ArchaeologyAdapterLineage>,
}
pub(crate) fn link_archaeology_facts(repository_id: &str, revision_sha: &str,
    units: &[ArchaeologyLinkUnit<'_>], facts: &[ArchaeologyLinkFact<'_>], extracted_edges: &[ArchaeologyFactEdge],
    cancellation: &StructuralGraphCancellation, limits: ArchaeologyLinkLimits) -> Result<ArchaeologyLinkPatch, String> {
    cancelled(cancellation)?;
    if units.len() > limits.max_units || facts.len() > limits.max_facts || extracted_edges.len() > limits.max_edges { return Err("Archaeology linker input bound exceeded".into()); }
    if link_input_bytes(units, facts, extracted_edges) > limits.max_input_bytes { return Err("Archaeology linker input byte bound exceeded".into()); }
    let unit_languages = units.iter().map(|unit| (unit.source_unit_id, (unit.language, unit.dialect))).collect::<BTreeMap<_, _>>();
    if unit_languages.len() != units.len() { return Err("Archaeology linker duplicate unit identity".into()); }
    let unit_paths = units.iter().map(|unit| (unit.source_unit_id, unit.relative_path.unwrap_or(unit.source_unit_id))).collect::<BTreeMap<_, _>>();
    let by_id = facts.iter().map(|item| (item.fact.fact_id.as_str(), item)).collect::<BTreeMap<_, _>>();
    if by_id.len() != facts.len() { return Err("Archaeology linker duplicate fact identity".into()); }
    let mut unresolved_by_source = BTreeMap::<&str, Vec<&ArchaeologyFactEdge>>::new();
    let mut incidents = BTreeMap::<&str, usize>::new();
    for edge in extracted_edges { *incidents.entry(&edge.from_fact_id).or_default() += 1; *incidents.entry(&edge.to_fact_id).or_default() += 1;
        if edge.kind == ArchaeologyFactEdgeKind::Unresolved || edge.unresolved_reason.is_some() { unresolved_by_source.entry(&edge.from_fact_id).or_default().push(edge); } }
    for item in facts {
        if !unit_languages.contains_key(item.source_unit_id) { return Err("Archaeology linker fact has no source unit".into()); }
        if item.fact.span_ids.iter().any(|id| !item.evidence_spans.iter().any(|span|
            span.span_id == *id && span.source_unit_id == item.source_unit_id && span.revision_sha == revision_sha)) {
            return Err("Archaeology linker requires exact scoped evidence spans".into());
        }
    }
    let mut includes = BTreeMap::<(&str, &str), &ArchaeologyFact>::new();
    for item in facts.iter().filter(|item| item.fact.kind == ArchaeologyFactKind::Include) {
        for span in &item.fact.span_ids { if includes.insert((item.source_unit_id, span), item.fact).is_some() {
            return Err("Archaeology linker duplicate include evidence".into());
        }}
    }
    let mut target_units = BTreeMap::<(String, String, String), Vec<&ArchaeologyLinkUnit<'_>>>::new();
    for unit in units { if let Some(path) = unit.relative_path.map(Path::new) {
        for target in [path.file_name(), path.file_stem()].into_iter().flatten().filter_map(|value| value.to_str()) {
            target_units.entry(lineage_key(unit.language, unit.dialect, target)).or_default().push(unit);
        }
    }}
    for candidates in target_units.values_mut() { candidates.sort_by_key(|unit| unit.source_unit_id); candidates.dedup_by_key(|unit| unit.source_unit_id); }
    let mut symbols = BTreeMap::<String, Vec<&ArchaeologyLinkFact<'_>>>::new();
    for item in facts {
        cancelled(cancellation)?;
        if candidate_kind(&item.fact.kind) {
            for key in std::iter::once(item.fact.label.as_str()).chain(
                item.fact.attributes.iter().filter(|a| a.key == "symbol").map(|a| a.value.as_str())) {
                symbols.entry(folded_key(key)).or_default().push(item);
            }
        }
    }
    let mut patch = ArchaeologyLinkPatch::default();
    let mut placeholder_candidates = BTreeSet::new();
    let mut removed_edges = BTreeSet::new();
    let mut removed_incidents = BTreeMap::new();
    let lineage_count = units.iter().map(|unit| unit.lineage.iter().filter(|lineage| lineage.kind != ArchaeologyLineageKind::Preprocessed).count()).sum::<usize>();
    if lineage_count > limits.max_references || lineage_count > limits.max_output_items { return Err("Archaeology linker lineage bound exceeded".into()); }
    link_lineage(units, &includes, &target_units, &unresolved_by_source, &by_id, cancellation, limits.max_candidates_per_reference,
        limits.max_output_items, &mut removed_edges, &mut removed_incidents, &mut placeholder_candidates, &mut patch)?;
    let mut reference_count = lineage_count;
    for source in facts {
        for (kind, target) in references(source.fact) {
            cancelled(cancellation)?;
            reference_count += 1;
            if reference_count > limits.max_references { return Err("Archaeology linker reference bound exceeded".into()); }
            let (source_language, source_dialect) = unit_languages.get(source.source_unit_id).copied().unwrap_or(("", None));
            let mut candidates = symbols.get(&folded_key(target)).cloned().unwrap_or_default();
            candidates.retain(|candidate| candidate.fact.fact_id != source.fact.fact_id
                && target_kind(&kind, &candidate.fact.kind)
                && unit_languages.get(candidate.source_unit_id).is_some_and(|(language, dialect)|
                    if *language == source_language { case_insensitive(source_language, source_dialect)
                        || exact_key(candidate.fact.label.as_str()) == exact_key(target)
                        || attribute(candidate.fact, "symbol").is_some_and(|value| exact_key(value) == exact_key(target))
                    } else { attribute(candidate.fact, "exported") == Some("true")
                        && (exact_key(candidate.fact.label.as_str()) == exact_key(target)
                            || attribute(candidate.fact, "symbol").is_some_and(|value| exact_key(value) == exact_key(target))) }
                    && (*language != "assembly" || source_language != "assembly" || *dialect == source_dialect)));
            candidates.sort_by_key(|candidate| candidate.fact.fact_id.as_str());
            candidates.dedup_by_key(|candidate| candidate.fact.fact_id.as_str());
            if candidates.len() > limits.max_candidates_per_reference {
                return Err("Archaeology linker candidate bound exceeded".into());
            }
            let evidence_items = source.fact.span_ids.len() + candidates.first().map_or(0, |candidate| candidate.fact.span_ids.len());
            let added_items = if candidates.len() == 1 { 1usize.saturating_add(evidence_items) }
                else { 2usize.saturating_add(source.fact.span_ids.len().saturating_mul(2)) };
            if output_items(&patch).saturating_add(added_items) > limits.max_output_items {
                return Err("Archaeology linker output item bound exceeded".into());
            }
            let (to_id, edge_kind, unresolved_reason, evidence_target) = if candidates.len() == 1 {
                (candidates[0].fact.fact_id.clone(), kind, None, Some(candidates[0]))
            } else {
                let reason = if candidates.is_empty() { "reference target is unavailable" }
                    else { "reference target is ambiguous" };
                let fact_id = link_id("fact", repository_id, revision_sha,
                    &format!("{:?}\0{}\0{}", kind, source.fact.fact_id, reference_key(source_language, source_dialect, target)));
                let candidate_count = candidates.len().to_string();
                patch.upsert_facts.push(ArchaeologyFact { fact_id: fact_id.clone(),
                    kind: ArchaeologyFactKind::Unresolved, label: "unresolved reference".into(),
                    span_ids: source.fact.span_ids.clone(), parser_id: source.fact.parser_id.clone(),
                    trust: ArchaeologyTrust::Deterministic, confidence: ArchaeologyConfidence::Low,
                    attributes: vec![ArchaeologyAttribute { key: "candidate_count".into(), value: candidate_count }] });
                for span in &source.fact.span_ids { patch.evidence.push(("fact".into(), fact_id.clone(), span.clone())); }
                let unresolved_kind = if source.fact.kind == ArchaeologyFactKind::Transaction { kind }
                    else { ArchaeologyFactEdgeKind::Unresolved };
                (fact_id, unresolved_kind, Some(reason), None)
            };
            let evidence = exact_evidence(source, evidence_target);
            let edge_id = link_id("edge", repository_id, revision_sha,
                &format!("{:?}\0{}\0{}", edge_kind, source.fact.fact_id, to_id));
            clear_placeholders(source.fact, &to_id, &unresolved_by_source, &by_id,
                &mut removed_edges, &mut removed_incidents, &mut placeholder_candidates);
            patch.upsert_edges.push(ArchaeologyFactEdge { edge_id: edge_id.clone(),
                from_fact_id: source.fact.fact_id.clone(), to_fact_id: to_id,
                kind: edge_kind, trust: ArchaeologyTrust::Deterministic,
                evidence_span_ids: evidence.clone(), unresolved_reason: unresolved_reason.map(str::to_string) });
            for span in evidence { patch.evidence.push(("fact_edge".into(), edge_id.clone(), span)); }
        }
    }
    let mut predicate_groups = BTreeMap::<(String, String, String, String), Vec<(&ArchaeologyLinkFact<'_>, &str)>>::new();
    for item in facts.iter().filter(|item| item.fact.kind == ArchaeologyFactKind::Predicate) {
        cancelled(cancellation)?;
        let Some(operator) = unique_attribute(item.fact, "operator").filter(|value| comparison_operator(value)) else { continue; };
        let Some(rhs) = unique_attribute(item.fact, "comparison_rhs_expr").filter(|value| canonical_semantic_digest(value)) else { continue; };
        let Some(subject) = unique_attribute(item.fact, "reads") else { continue; };
        let (language, dialect) = unit_languages.get(item.source_unit_id).copied().ok_or("Archaeology linker predicate has no source unit")?;
        predicate_groups.entry((language.into(), dialect.unwrap_or("").into(), folded_key(subject), rhs.into()))
            .or_default().push((item, operator));
    }
    let mut known_contradictions = extracted_edges.iter().filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Contradicts)
        .map(|edge| ordered_pair(edge.from_fact_id.as_str(), edge.to_fact_id.as_str())).collect::<BTreeSet<_>>();
    for group in predicate_groups.values_mut() {
        group.sort_by_key(|(item, _)| {
            let span = item.evidence_spans.iter()
                .filter(|span| item.fact.span_ids.contains(&span.span_id))
                .min_by_key(|span| (span.start.byte, span.end.byte));
            (unit_paths[item.source_unit_id], span.map_or(u64::MAX, |span| span.start.byte),
                span.map_or(u64::MAX, |span| span.end.byte), item.fact.label.as_str())
        });
        if group.len() > limits.max_candidates_per_reference { return Err("Archaeology linker complementary predicate candidate bound exceeded".into()); }
        for left in 0..group.len() { for right in left + 1..group.len() {
            cancelled(cancellation)?;
            if !complementary_operators(group[left].1, group[right].1) { continue; }
            reference_count = reference_count.saturating_add(1);
            if reference_count > limits.max_references { return Err("Archaeology linker reference bound exceeded".into()); }
            let identity_pair = ordered_pair(group[left].0.fact.fact_id.as_str(), group[right].0.fact.fact_id.as_str());
            if !known_contradictions.insert(identity_pair) { continue; }
            let pair = (group[left].0.fact.fact_id.clone(), group[right].0.fact.fact_id.clone());
            let evidence = exact_evidence(group[left].0, Some(group[right].0));
            if output_items(&patch).saturating_add(1 + evidence.len()) > limits.max_output_items {
                return Err("Archaeology linker output item bound exceeded".into());
            }
            let edge_id = link_id("edge", repository_id, revision_sha,
                &format!("Contradicts\0{}\0{}", pair.0, pair.1));
            patch.upsert_edges.push(ArchaeologyFactEdge { edge_id: edge_id.clone(),
                from_fact_id: pair.0, to_fact_id: pair.1, kind: ArchaeologyFactEdgeKind::Contradicts,
                trust: ArchaeologyTrust::Deterministic, evidence_span_ids: evidence.clone(), unresolved_reason: None });
            for span in evidence { patch.evidence.push(("fact_edge".into(), edge_id.clone(), span)); }
        }}
    }
    patch.remove_edge_ids = removed_edges.iter().cloned().collect();
    patch.remove_fact_ids = placeholder_candidates.into_iter().filter(|id|
        incidents.get(id.as_str()) == removed_incidents.get(id.as_str())).collect();
    patch.upsert_facts.sort_by(|a, b| a.fact_id.cmp(&b.fact_id));
    patch.upsert_facts.dedup_by(|a, b| a.fact_id == b.fact_id);
    patch.upsert_edges.sort_by(|a, b| a.edge_id.cmp(&b.edge_id));
    patch.upsert_edges.dedup_by(|a, b| a.edge_id == b.edge_id);
    patch.evidence.sort(); patch.evidence.dedup(); patch.lineage.sort_by(|a, b|
        (&a.source_unit_id, &a.evidence_span_id).cmp(&(&b.source_unit_id, &b.evidence_span_id)));
    if patch.upsert_edges.len() > limits.max_output_edges || output_items(&patch) > limits.max_output_items || serde_json::to_vec(&patch)
        .map_err(|_| "Archaeology linker output is not serializable")?.len() > limits.max_output_bytes {
        return Err("Archaeology linker output bound exceeded".into());
    }
    cancelled(cancellation)?;
    Ok(patch)
}
fn link_lineage(units: &[ArchaeologyLinkUnit<'_>], includes: &BTreeMap<(&str, &str), &ArchaeologyFact>,
    target_units: &BTreeMap<(String, String, String), Vec<&ArchaeologyLinkUnit<'_>>>,
    edges: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>, by_id: &BTreeMap<&str, &ArchaeologyLinkFact<'_>>,
    cancellation: &StructuralGraphCancellation, max_candidates: usize, max_output_items: usize,
    removed: &mut BTreeSet<String>, removed_incidents: &mut BTreeMap<String, usize>, placeholders: &mut BTreeSet<String>, patch: &mut ArchaeologyLinkPatch) -> Result<(), String> {
    for unit in units { for lineage in unit.lineage.iter().filter(|lineage| lineage.kind != ArchaeologyLineageKind::Preprocessed) {
        cancelled(cancellation)?;
        if output_items(patch) == max_output_items { return Err("Archaeology linker output item bound exceeded".into()); }
        let include = includes.get(&(unit.source_unit_id, lineage.evidence_span_id.as_str())).copied();
        let target = include.map(|fact| attribute(fact, "target").unwrap_or(&fact.label));
        let matches = target.and_then(|target| target_units.get(&lineage_key(unit.language, unit.dialect, target)))
            .map(|items| items.iter().copied().filter(|candidate| candidate.source_unit_id != unit.source_unit_id).collect::<Vec<_>>()).unwrap_or_default();
        if matches.len() > max_candidates { return Err("Archaeology linker lineage candidate bound exceeded".into()); }
        let resolved = (matches.len() == 1).then(|| matches[0].source_unit_id.to_string());
        if resolved.is_some() { if let Some(include) = include { clear_placeholders(include, "", edges, by_id, removed, removed_incidents, placeholders); }}
        patch.lineage.push(ArchaeologyAdapterLineage { kind: lineage.kind.clone(), source_unit_id: unit.source_unit_id.into(),
            evidence_span_id: lineage.evidence_span_id.clone(), target_source_unit_id: resolved,
            detail: if matches.len() == 1 { "resolved include target" } else if matches.is_empty() {
                "unresolved include target is unavailable" } else { "unresolved include target is ambiguous" }.into() });
    }}
    let pairs = patch.lineage.iter().filter_map(|item| item.target_source_unit_id.as_ref().map(|target| (item.source_unit_id.clone(), target.clone()))).collect::<BTreeSet<_>>();
    let cycles = pairs.iter().filter(|(source, target)| pairs.contains(&(target.clone(), source.clone()))).cloned().collect::<BTreeSet<_>>();
    for item in &mut patch.lineage { if item.target_source_unit_id.as_ref().is_some_and(|target| cycles.contains(&(item.source_unit_id.clone(), target.clone()))) {
        item.detail = "resolved include target with direct cycle".into();
    }}
    Ok(())
}
fn references(fact: &ArchaeologyFact) -> Vec<(ArchaeologyFactEdgeKind, &str)> {
    let mut result = Vec::new();
    for attribute in &fact.attributes {
        let kind = match attribute.key.as_str() {
            "reads" => Some(ArchaeologyFactEdgeKind::Reads), "writes" => Some(ArchaeologyFactEdgeKind::Writes),
            "controls" => Some(ArchaeologyFactEdgeKind::Controls), _ => None,
        };
        if let Some(kind) = kind { result.push((kind, attribute.value.as_str())); }
    }
    let implicit_transaction = (fact.kind == ArchaeologyFactKind::Transaction
        && matches!(attribute(fact, "operation"), Some("begin" | "commit" | "rollback")))
        .then_some("implicit transaction scope");
    if let Some(target) = attribute(fact, "target").or(implicit_transaction) {
        let kind = match fact.kind { ArchaeologyFactKind::Call => Some(ArchaeologyFactEdgeKind::Calls),
            ArchaeologyFactKind::ControlFlow => Some(ArchaeologyFactEdgeKind::BranchesTo),
            ArchaeologyFactKind::Transaction => match attribute(fact, "operation") {
                Some("begin") => Some(ArchaeologyFactEdgeKind::BeginsTransaction),
                Some("commit") => Some(ArchaeologyFactEdgeKind::CommitsTransaction),
                Some("rollback") => Some(ArchaeologyFactEdgeKind::RollsBackTransaction), _ => None }, _ => None };
        if let Some(kind) = kind { result.push((kind, target)); }
    }
    result
}
fn candidate_kind(kind: &ArchaeologyFactKind) -> bool { matches!(kind,
    ArchaeologyFactKind::Declaration | ArchaeologyFactKind::DataField | ArchaeologyFactKind::Constant
    | ArchaeologyFactKind::Predicate | ArchaeologyFactKind::Decision | ArchaeologyFactKind::Transaction
    | ArchaeologyFactKind::ControlFlow | ArchaeologyFactKind::EntryPoint) }
fn target_kind(edge: &ArchaeologyFactEdgeKind, fact: &ArchaeologyFactKind) -> bool { match edge {
    ArchaeologyFactEdgeKind::Calls | ArchaeologyFactEdgeKind::BranchesTo => matches!(fact, ArchaeologyFactKind::EntryPoint | ArchaeologyFactKind::Declaration),
    ArchaeologyFactEdgeKind::Reads | ArchaeologyFactEdgeKind::Writes => matches!(fact, ArchaeologyFactKind::DataField | ArchaeologyFactKind::Constant),
    ArchaeologyFactEdgeKind::Controls => matches!(fact, ArchaeologyFactKind::Predicate | ArchaeologyFactKind::Decision | ArchaeologyFactKind::ControlFlow),
    ArchaeologyFactEdgeKind::BeginsTransaction | ArchaeologyFactEdgeKind::CommitsTransaction | ArchaeologyFactEdgeKind::RollsBackTransaction => *fact == ArchaeologyFactKind::Transaction,
    _ => false } }
fn attribute<'a>(fact: &'a ArchaeologyFact, key: &str) -> Option<&'a str> { fact.attributes.iter().find(|item| item.key == key).map(|item| item.value.as_str()) }
fn unique_attribute<'a>(fact: &'a ArchaeologyFact, key: &str) -> Option<&'a str> {
    let mut values = fact.attributes.iter().filter(|item| item.key == key).map(|item| item.value.as_str());
    let value = values.next()?; values.next().is_none().then_some(value)
}
fn comparison_operator(value: &str) -> bool { matches!(value, ">" | "<" | "=" | ">=" | "<=") }
fn complementary_operators(left: &str, right: &str) -> bool {
    matches!((left, right), (">", "<=") | ("<=", ">") | ("<", ">=") | (">=", "<"))
}
fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right { (left.into(), right.into()) } else { (right.into(), left.into()) }
}
fn exact_key(value: &str) -> &str { value.trim().trim_matches(['\'', '"']).trim_end_matches(['.', ':']) }
fn folded_key(value: &str) -> String { exact_key(value).to_ascii_lowercase() }
fn case_insensitive(language: &str, dialect: Option<&str>) -> bool { language == "cobol" || language == "assembly" && dialect == Some("hlasm") }
fn reference_key(language: &str, dialect: Option<&str>, value: &str) -> String {
    if case_insensitive(language, dialect) { folded_key(value) } else { exact_key(value).into() }
}
fn lineage_key(language: &str, dialect: Option<&str>, target: &str) -> (String, String, String) {
    (language.into(), if language == "cobol" { "*".into() } else { dialect.unwrap_or("").into() },
        if case_insensitive(language, dialect) { folded_key(target) } else { exact_key(target).into() })
}
fn output_items(patch: &ArchaeologyLinkPatch) -> usize { patch.remove_fact_ids.len() + patch.remove_edge_ids.len() + patch.upsert_facts.len() + patch.upsert_edges.len() + patch.evidence.len() + patch.lineage.len() }
fn link_input_bytes(units: &[ArchaeologyLinkUnit<'_>], facts: &[ArchaeologyLinkFact<'_>], edges: &[ArchaeologyFactEdge]) -> usize {
    let mut total = 0usize;
    for unit in units { add_bytes(&mut total, unit.source_unit_id); add_bytes(&mut total, unit.language); if let Some(value) = unit.dialect { add_bytes(&mut total, value); }
        if let Some(value) = unit.relative_path { add_bytes(&mut total, value); }
        for item in unit.lineage { add_bytes(&mut total, &item.source_unit_id); if let Some(value) = &item.target_source_unit_id { add_bytes(&mut total, value); } add_bytes(&mut total, &item.evidence_span_id); add_bytes(&mut total, &item.detail); total = total.saturating_add(32); } }
    for item in facts { add_bytes(&mut total, item.source_unit_id); add_bytes(&mut total, &item.fact.fact_id); add_bytes(&mut total, &item.fact.label); add_bytes(&mut total, &item.fact.parser_id); for value in &item.fact.span_ids { add_bytes(&mut total, value); }
        for attribute in &item.fact.attributes { add_bytes(&mut total, &attribute.key); add_bytes(&mut total, &attribute.value); } for span in item.evidence_spans { add_bytes(&mut total, &span.span_id); add_bytes(&mut total, &span.source_unit_id); add_bytes(&mut total, &span.revision_sha); total = total.saturating_add(64); } total = total.saturating_add(32); }
    for edge in edges { add_bytes(&mut total, &edge.edge_id); add_bytes(&mut total, &edge.from_fact_id); add_bytes(&mut total, &edge.to_fact_id); for value in &edge.evidence_span_ids { add_bytes(&mut total, value); } if let Some(value) = &edge.unresolved_reason { add_bytes(&mut total, value); } total = total.saturating_add(32); }
    total
}
fn add_bytes(total: &mut usize, value: &str) { *total = (*total).saturating_add(value.len()); }
fn exact_evidence(source: &ArchaeologyLinkFact<'_>, target: Option<&ArchaeologyLinkFact<'_>>) -> Vec<String> {
    let mut ids = source.fact.span_ids.clone(); if let Some(target) = target { ids.extend(target.fact.span_ids.clone()); } ids.sort(); ids.dedup(); ids }
fn link_id(kind: &str, repository: &str, revision: &str, local: &str) -> String {
    stable_graph_id(&format!("archaeology-link-{kind}"), &format!("{repository}\0{revision}\0{local}")) }
fn clear_placeholders(source: &ArchaeologyFact, keep: &str, edges: &BTreeMap<&str, Vec<&ArchaeologyFactEdge>>, facts: &BTreeMap<&str, &ArchaeologyLinkFact<'_>>, removed: &mut BTreeSet<String>,
    removed_incidents: &mut BTreeMap<String, usize>, candidates: &mut BTreeSet<String>) {
    for edge in edges.get(source.fact_id.as_str()).into_iter().flatten().filter(|edge| edge.to_fact_id != keep
        && facts.get(edge.to_fact_id.as_str()).is_some_and(|item| item.fact.kind == ArchaeologyFactKind::Unresolved)) {
        if removed.insert(edge.edge_id.clone()) { *removed_incidents.entry(edge.from_fact_id.clone()).or_default() += 1;
            *removed_incidents.entry(edge.to_fact_id.clone()).or_default() += 1; }
        candidates.insert(edge.to_fact_id.clone());
    }
}
fn cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> { if cancellation.is_cancelled() { Err("Archaeology linker cancelled".into()) } else { Ok(()) } }
}
#[rustfmt::skip]
pub(crate) use linker::{link_archaeology_facts, ArchaeologyLinkFact, ArchaeologyLinkLimits, ArchaeologyLinkPatch, ArchaeologyLinkUnit};

#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures_tests;
