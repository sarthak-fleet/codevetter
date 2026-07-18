//! SQLite materialization for stable rule identities.

use super::contracts::{ArchaeologyFactKind, ArchaeologyRuleKind};
use super::identity::{
    build_rule_identities, ArchaeologyIdentityFact, ArchaeologyIdentityLimits,
    ArchaeologyIdentitySpan, ArchaeologyRuleIdentityInput, PARSER_COMPATIBILITY_TAG,
};
use super::inventory::hex;
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use rusqlite::{params, Transaction};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const MAX_RULE_IDENTITIES_PER_GENERATION: usize = 100_000;
const RULE_IDENTITY_BATCH_SIZE: usize = 512;
const MAX_RULE_IDENTITY_SELECTION_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSpan {
    path_identity: String,
    content_hash: String,
    start_byte: u64,
    end_byte: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFact {
    fact_id: String,
    kind: ArchaeologyFactKind,
    semantic_expression: String,
    parser_identity: String,
    spans: Vec<StoredSpan>,
}

struct StoredRule {
    rule_id: String,
    repository_id: String,
    kind: ArchaeologyRuleKind,
    title: String,
    description_source_identity: String,
    clauses: Vec<String>,
    supporting_fact_ids: BTreeSet<String>,
    contradicting_fact_ids: BTreeSet<String>,
}

/// Rebuild one or all rule identity projections from persisted normalized
/// facts and exact spans. Generated row IDs and revision SHAs never enter the
/// digest inputs.
pub(crate) fn refresh_rule_identities(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids: &[String],
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    process_rule_identities(transaction, generation_id, rule_ids, cancellation, true)
}

pub(crate) fn validate_rule_identities(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids: &[String],
    cancellation: &StructuralGraphCancellation,
) -> Result<usize, String> {
    process_rule_identities(transaction, generation_id, rule_ids, cancellation, false)
}

fn process_rule_identities(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids: &[String],
    cancellation: &StructuralGraphCancellation,
    apply: bool,
) -> Result<usize, String> {
    if rule_ids.is_empty() {
        return Ok(0);
    }
    if rule_ids.len() > MAX_RULE_IDENTITIES_PER_GENERATION
        || rule_ids
            .iter()
            .try_fold(0usize, |total, id| total.checked_add(id.len()).ok_or(()))
            .map_or(true, |bytes| bytes > MAX_RULE_IDENTITY_SELECTION_BYTES)
        || rule_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 256 || id.contains('\0'))
        || rule_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .len()
            != rule_ids.len()
    {
        return Err("Archaeology rule identity selection is invalid or over bound".into());
    }
    let mut changed = 0usize;
    for batch in rule_ids.chunks(RULE_IDENTITY_BATCH_SIZE) {
        if cancellation.is_cancelled() {
            return Err("Archaeology rule identity refresh cancelled".into());
        }
        let batch_changed =
            process_identity_batch(transaction, generation_id, batch, cancellation, apply)?;
        if batch_changed != batch.len() {
            return Err("Archaeology rule identity selection did not reconcile".into());
        }
        changed = changed.saturating_add(batch_changed);
    }
    Ok(changed)
}

fn process_identity_batch(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids: &[String],
    cancellation: &StructuralGraphCancellation,
    apply: bool,
) -> Result<usize, String> {
    let profiling = std::env::var_os("CODEVETTER_ARCHAEOLOGY_PROFILE").is_some();
    let started = Instant::now();
    let selected = serde_json::to_string(rule_ids)
        .map_err(|error| format!("Encode archaeology identity rule selection: {error}"))?;
    let mut rules = load_rules(transaction, generation_id, &selected)?;
    if rules.is_empty() {
        return Ok(0);
    }
    load_rule_evidence(transaction, generation_id, &selected, &mut rules)?;
    if profiling {
        eprintln!(
            "ARCHAEOLOGY_PROFILE\tidentity.load_rules\t{:.3}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    let facts = load_facts(transaction, generation_id, &selected)?;
    if profiling {
        eprintln!(
            "ARCHAEOLOGY_PROFILE\tidentity.load_facts\t{:.3}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    let provenance = serde_json::to_string(&super::identity::identity_provenance())
        .map_err(|error| format!("Encode archaeology identity provenance: {error}"))?;
    let limits = ArchaeologyIdentityLimits::default();
    let projection_sql = if apply {
        "UPDATE archaeology_rules
         SET identity_schema_version=2,stable_rule_identity=?3,evidence_identity=?4,
             contradiction_identity=?5,description_identity=?6,continuity_identity=?7,
             identity_provenance_json=?8,parser_compatibility_identity=?9
         WHERE generation_id=?1 AND rule_id=?2"
    } else {
        "SELECT COUNT(*) FROM archaeology_rules
         WHERE generation_id=?1 AND rule_id=?2 AND identity_schema_version=2
           AND stable_rule_identity=?3 AND evidence_identity=?4
           AND contradiction_identity=?5 AND description_identity=?6
           AND continuity_identity=?7 AND identity_provenance_json=?8
           AND parser_compatibility_identity=?9"
    };
    let mut projection_statement = transaction
        .prepare_cached(projection_sql)
        .map_err(|error| format!("Prepare archaeology rule identity projection: {error}"))?;

    let mut changed = 0usize;
    for rule in rules.values() {
        if cancellation.is_cancelled() {
            return Err("Archaeology rule identity refresh cancelled".into());
        }
        if rule.supporting_fact_ids.is_empty() || rule.clauses.is_empty() {
            return Err("Archaeology rule identity requires cited clauses".into());
        }
        let supporting_stored = resolve_facts(&facts, &rule.supporting_fact_ids)?;
        let contradicting_stored = resolve_facts(&facts, &rule.contradicting_fact_ids)?;
        let supporting_spans = supporting_stored
            .iter()
            .map(|fact| borrowed_spans(fact))
            .collect::<Vec<_>>();
        let contradicting_spans = contradicting_stored
            .iter()
            .map(|fact| borrowed_spans(fact))
            .collect::<Vec<_>>();
        let supporting = borrowed_facts(&supporting_stored, &supporting_spans);
        let contradicting = borrowed_facts(&contradicting_stored, &contradicting_spans);
        let anchor = supporting
            .iter()
            .min_by(|left, right| {
                (fact_kind_key(left.kind), left.semantic_expression)
                    .cmp(&(fact_kind_key(right.kind), right.semantic_expression))
            })
            .ok_or("Archaeology rule identity has no supporting anchor")?;
        let clauses = rule.clauses.iter().map(String::as_str).collect::<Vec<_>>();
        let identities = build_rule_identities(
            &ArchaeologyRuleIdentityInput {
                repository_id: &rule.repository_id,
                kind: &rule.kind,
                anchor,
                supporting_facts: &supporting,
                contradicting_facts: &contradicting,
                title: &rule.title,
                clauses: &clauses,
                description_source_identity: &rule.description_source_identity,
            },
            limits,
        )?;
        let parser_compatibility_identity =
            parser_compatibility_identity(&rule.repository_id, &supporting, &contradicting)?;
        let reconciled = if apply {
            projection_statement
                .execute(params![
                    generation_id,
                    rule.rule_id,
                    identities.stable_rule_identity,
                    identities.evidence_identity,
                    identities.contradiction_identity,
                    identities.description_identity,
                    identities.continuity_identity,
                    provenance,
                    parser_compatibility_identity,
                ])
                .map_err(|error| format!("Persist archaeology rule identity: {error}"))?
        } else {
            projection_statement
                .query_row(
                    params![
                        generation_id,
                        rule.rule_id,
                        identities.stable_rule_identity,
                        identities.evidence_identity,
                        identities.contradiction_identity,
                        identities.description_identity,
                        identities.continuity_identity,
                        provenance,
                        parser_compatibility_identity,
                    ],
                    |row| row.get::<_, usize>(0),
                )
                .map_err(|error| format!("Validate archaeology rule identity: {error}"))?
        };
        if reconciled != 1 {
            return Err("Archaeology rule identity projection does not reconcile".into());
        }
        changed += 1;
    }
    if profiling {
        eprintln!(
            "ARCHAEOLOGY_PROFILE\tidentity.project\t{:.3}",
            started.elapsed().as_secs_f64() * 1_000.0
        );
    }
    Ok(changed)
}

fn load_rules(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids_json: &str,
) -> Result<BTreeMap<String, StoredRule>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT rule.rule_id,rule.repository_id,rule.kind,rule.title,
                    COALESCE(rule.synthesis_identity,rule.algorithm_identity),clause.clause_text
             FROM archaeology_rules rule
             JOIN archaeology_rule_clauses clause
               ON clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id
             WHERE rule.generation_id=?1 AND rule.rule_id IN (SELECT value FROM json_each(?2))
             ORDER BY rule.rule_id,clause.ordinal,clause.clause_id",
        )
        .map_err(|error| format!("Prepare archaeology identity rules: {error}"))?;
    let rows = statement
        .query_map(params![generation_id, rule_ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| format!("Query archaeology identity rules: {error}"))?;
    let mut result = BTreeMap::new();
    for row in rows {
        let (id, repository_id, kind, title, description_source_identity, clause) =
            row.map_err(|error| format!("Read archaeology identity rule: {error}"))?;
        let kind = serde_json::from_value(serde_json::Value::String(kind))
            .map_err(|_| "Stored archaeology rule kind is invalid".to_string())?;
        let entry = result.entry(id.clone()).or_insert_with(|| StoredRule {
            rule_id: id,
            repository_id,
            kind,
            title,
            description_source_identity,
            clauses: Vec::new(),
            supporting_fact_ids: BTreeSet::new(),
            contradicting_fact_ids: BTreeSet::new(),
        });
        entry.clauses.push(clause);
    }
    Ok(result)
}

fn load_rule_evidence(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids_json: &str,
    rules: &mut BTreeMap<String, StoredRule>,
) -> Result<(), String> {
    let mut statement = transaction
        .prepare(
            "SELECT clause.rule_id,evidence.evidence_id,evidence.role
             FROM archaeology_rule_clauses clause
             JOIN archaeology_evidence_links evidence
               ON evidence.generation_id=clause.generation_id
              AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
              AND evidence.evidence_kind='fact'
             WHERE clause.generation_id=?1 AND clause.rule_id IN (SELECT value FROM json_each(?2))
             ORDER BY clause.rule_id,evidence.role,evidence.evidence_id",
        )
        .map_err(|error| format!("Prepare archaeology identity evidence: {error}"))?;
    let rows = statement
        .query_map(params![generation_id, rule_ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Query archaeology identity evidence: {error}"))?;
    for row in rows {
        let (rule, fact, role) =
            row.map_err(|error| format!("Read archaeology identity evidence: {error}"))?;
        let target = rules
            .get_mut(&rule)
            .ok_or("Archaeology identity evidence references an unknown rule")?;
        match role.as_str() {
            "supporting" => {
                target.supporting_fact_ids.insert(fact);
            }
            "contradicting" => {
                target.contradicting_fact_ids.insert(fact);
            }
            _ => return Err("Archaeology identity evidence has an invalid role".into()),
        }
    }
    Ok(())
}

fn load_facts(
    transaction: &Transaction<'_>,
    generation_id: &str,
    rule_ids_json: &str,
) -> Result<BTreeMap<String, StoredFact>, String> {
    let mut statement = transaction
        .prepare(
            "WITH selected AS (
               SELECT DISTINCT evidence.evidence_id fact_id
               FROM archaeology_rule_clauses clause
               JOIN archaeology_evidence_links evidence
                 ON evidence.generation_id=clause.generation_id
                AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
                AND evidence.evidence_kind='fact'
               WHERE clause.generation_id=?1 AND clause.rule_id IN (SELECT value FROM json_each(?2))
             ), rows AS (
               SELECT fact.fact_id,fact.kind,
                 json_extract((SELECT value FROM json_each(fact.attributes_json)
                   WHERE json_extract(value,'$.key')='semantic_expr' LIMIT 1),'$.value') semantic_expression,
                 fact.parser_id || '@' || unit.parser_version parser_identity,
                 fact.parser_id fact_parser_id,unit.parser_id unit_parser_id,
                 unit.hash_algorithm,unit.path_identity,unit.content_hash,
                 span.start_byte,span.end_byte
               FROM selected
               JOIN archaeology_facts fact ON fact.generation_id=?1 AND fact.fact_id=selected.fact_id
               JOIN archaeology_evidence_links evidence
                 ON evidence.generation_id=fact.generation_id AND evidence.owner_kind='fact'
                AND evidence.owner_id=fact.fact_id AND evidence.evidence_kind='span'
                AND evidence.role='supporting'
               JOIN archaeology_source_spans span
                 ON span.generation_id=evidence.generation_id AND span.span_id=evidence.evidence_id
               JOIN archaeology_source_units unit
                 ON unit.generation_id=span.generation_id AND unit.source_unit_id=span.source_unit_id
               WHERE unit.content_hash IS NOT NULL
               ORDER BY fact.fact_id,unit.path_identity,span.start_byte,span.end_byte
             )
             SELECT json_object('fact_id',fact_id,'kind',MIN(kind),
               'semantic_expression',MIN(semantic_expression),
               'parser_identity',MIN(parser_identity),'spans',json_group_array(json_object(
                 'path_identity',path_identity,'content_hash',content_hash,
                 'start_byte',start_byte,'end_byte',end_byte)))
             FROM rows GROUP BY fact_id
             HAVING COUNT(DISTINCT parser_identity)=1
                AND MIN(fact_parser_id)=MIN(unit_parser_id)
                AND MIN(hash_algorithm)='sha256' AND MAX(hash_algorithm)='sha256'
             ORDER BY fact_id",
        )
        .map_err(|error| format!("Prepare archaeology identity facts: {error}"))?;
    let rows = statement
        .query_map(params![generation_id, rule_ids_json], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|error| format!("Query archaeology identity facts: {error}"))?;
    let mut result = BTreeMap::new();
    for row in rows {
        let json = row.map_err(|error| format!("Read archaeology identity fact: {error}"))?;
        let fact: StoredFact = serde_json::from_str(&json)
            .map_err(|_| "Stored archaeology identity fact is invalid".to_string())?;
        if result.insert(fact.fact_id.clone(), fact).is_some() {
            return Err("Stored archaeology identity fact is duplicated".into());
        }
    }
    Ok(result)
}

fn resolve_facts<'a>(
    facts: &'a BTreeMap<String, StoredFact>,
    ids: &BTreeSet<String>,
) -> Result<Vec<&'a StoredFact>, String> {
    ids.iter()
        .map(|id| {
            facts
                .get(id)
                .ok_or_else(|| "Archaeology rule identity fact is unavailable".to_string())
        })
        .collect()
}

fn borrowed_spans(fact: &StoredFact) -> Vec<ArchaeologyIdentitySpan<'_>> {
    fact.spans
        .iter()
        .map(|span| ArchaeologyIdentitySpan {
            path_identity: &span.path_identity,
            content_hash: &span.content_hash,
            start_byte: span.start_byte,
            end_byte: span.end_byte,
        })
        .collect()
}

fn borrowed_facts<'a>(
    facts: &[&'a StoredFact],
    spans: &'a [Vec<ArchaeologyIdentitySpan<'a>>],
) -> Vec<ArchaeologyIdentityFact<'a>> {
    facts
        .iter()
        .zip(spans)
        .map(|(fact, spans)| ArchaeologyIdentityFact {
            kind: &fact.kind,
            semantic_expression: &fact.semantic_expression,
            parser_identity: &fact.parser_identity,
            spans,
        })
        .collect()
}

fn fact_kind_key(kind: &ArchaeologyFactKind) -> &'static str {
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

fn parser_compatibility_identity(
    repository_id: &str,
    supporting: &[ArchaeologyIdentityFact<'_>],
    contradicting: &[ArchaeologyIdentityFact<'_>],
) -> Result<String, String> {
    let identities = supporting
        .iter()
        .chain(contradicting)
        .map(|fact| fact.parser_identity)
        .collect::<BTreeSet<_>>();
    if identities.is_empty() || identities.len() > ArchaeologyIdentityLimits::default().max_facts {
        return Err("Archaeology parser compatibility identity bound is invalid".into());
    }
    let mut digest = Sha256::new();
    digest.update(PARSER_COMPATIBILITY_TAG.as_bytes());
    digest.update([0]);
    let repository_length = u64::try_from(repository_id.len())
        .map_err(|_| "Archaeology parser compatibility repository is too large")?;
    digest.update(repository_length.to_be_bytes());
    digest.update(repository_id.as_bytes());
    for identity in identities {
        if identity.is_empty() || identity.len() > 256 || identity.contains('\0') {
            return Err("Archaeology parser compatibility input is invalid".into());
        }
        let length = u64::try_from(identity.len())
            .map_err(|_| "Archaeology parser compatibility input is too large")?;
        digest.update(length.to_be_bytes());
        digest.update(identity.as_bytes());
    }
    Ok(format!("sha256:{}", hex(&digest.finalize())))
}
