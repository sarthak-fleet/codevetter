//! Reproducible correctness qualification over the checked hand-labeled corpus.
//!
//! This is deliberately a production-pipeline measurement, not an inventory of
//! labels: two fixed Git revisions pass through inventory, adapters, linking,
//! deterministic derivation, publication, and canonical SQLite reads.

use super::*;
use crate::commands::business_rule_archaeology::adapter::{
    run_archaeology_adapter, ArchaeologyAdapterEvents, ArchaeologyAdapterInput,
    ArchaeologyAdapterLimits, ArchaeologyAdapterOutcome, ArchaeologyAdapterOutput,
    ArchaeologyLanguageAdapter,
};
use crate::commands::business_rule_archaeology::assembly_adapter::AssemblyAdapter;
use crate::commands::business_rule_archaeology::cobol_adapter::CobolAdapter;
use crate::commands::business_rule_archaeology::contracts::{
    ArchaeologyFact, ArchaeologyFactEdge, ArchaeologySourceClassification, ArchaeologySourceSpan,
    ArchaeologySourceUnitIdentity, ArchaeologyTemporalSnapshotPayload,
};
use crate::commands::business_rule_archaeology::export::{
    export_core, ArchaeologyExportFormat, ArchaeologyExportInput,
};
use crate::commands::business_rule_archaeology::inventory::{
    ArchaeologyIncludeCandidate, ArchaeologyInventoryUnit,
};
use crate::commands::business_rule_archaeology::modern_adapter::ModernLanguageAdapter;
use crate::commands::business_rule_archaeology::read::{
    ArchaeologyReadRequest, ArchaeologyReadResponse, ArchaeologyReadService, ArchaeologyRuleFilter,
    ArchaeologySourceSelector, ArchaeologyTemporalSelector, ArchaeologyTemporalSnapshot,
};
use crate::commands::structural_graph::language::SupportedLanguage;
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CORPUS: &[u8] = include_bytes!("fixtures/expected.json.fixture");
const POLICY: &[u8] = include_bytes!(
    "../../../../tests/fixtures/business-rule-archaeology/qualification-policy-v1.json"
);
const CHECKED: &[u8] = include_bytes!(
    "../../../../tests/fixtures/business-rule-archaeology/real-pipeline-correctness-v1.json"
);
const FIXTURE_ROOT: &str = "src/commands/business_rule_archaeology/fixtures/sources";

#[derive(Debug, Deserialize)]
struct Corpus {
    corpus_id: String,
    source_units: Vec<GoldenUnit>,
    spans: Vec<GoldenSpan>,
    facts: Vec<GoldenFact>,
    edges: Vec<GoldenEdge>,
    rules: Vec<GoldenRule>,
    conflicts: Vec<GoldenRelationCase>,
    duplicate_groups: Vec<GoldenDuplicateGroup>,
    history_changes: Vec<GoldenHistoryChange>,
}

#[derive(Debug, Deserialize)]
struct GoldenHistoryChange {
    #[serde(rename = "id")]
    _id: String,
    from_revision: String,
    to_revision: String,
    #[serde(rename = "before_rule_id")]
    _before_rule_id: String,
    #[serde(rename = "after_rule_id")]
    _after_rule_id: String,
    classification: String,
    span_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenUnit {
    id: String,
    path: String,
    #[serde(rename = "revision")]
    _revision: String,
    language: String,
    dialect: String,
    #[serde(rename = "protected")]
    _protected: bool,
}

#[derive(Debug, Deserialize)]
struct GoldenSpan {
    id: String,
    source_unit_id: String,
    start: [u64; 3],
    end: [u64; 3],
}

#[derive(Debug, Deserialize)]
struct GoldenFact {
    id: String,
    kind: String,
    span_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenEdge {
    from: String,
    to: String,
    kind: String,
}

#[derive(Debug, Deserialize)]
struct GoldenRule {
    id: String,
    clauses: Vec<GoldenClause>,
}

#[derive(Debug, Deserialize)]
struct GoldenClause {
    supporting_fact_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenRelationCase {
    rule_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GoldenDuplicateGroup {
    primary_rule_id: String,
    rule_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SpanKey {
    path: String,
    start_byte: u64,
    end_byte: u64,
    start_line: u64,
    start_column: u64,
    end_line: u64,
    end_column: u64,
}

#[derive(Debug, Clone)]
struct ActualFact {
    id: String,
    kind: String,
    trust: String,
    path: String,
    spans: BTreeSet<SpanKey>,
}

#[derive(Debug, Clone)]
struct ActualEdge {
    from: String,
    to: String,
    kind: String,
}

#[derive(Default)]
struct AdapterCapture {
    spans: Vec<ArchaeologySourceSpan>,
    facts: Vec<ArchaeologyFact>,
    edges: Vec<ArchaeologyFactEdge>,
}

impl ArchaeologyAdapterEvents for AdapterCapture {
    fn emit_span(&mut self, value: ArchaeologySourceSpan) -> Result<(), String> {
        self.spans.push(value);
        Ok(())
    }
    fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> {
        self.facts.push(value);
        Ok(())
    }
    fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> {
        self.edges.push(value);
        Ok(())
    }
}

impl ArchaeologyAdapterOutput for AdapterCapture {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn commit_unit(&mut self, _: &ArchaeologyAdapterOutcome) -> Result<(), String> {
        Ok(())
    }
    fn abort_unit(&mut self) -> Result<(), String> {
        self.spans.clear();
        self.facts.clear();
        self.edges.clear();
        Ok(())
    }
}

impl ActualFact {
    fn signature(&self) -> String {
        let spans = self
            .spans
            .iter()
            .map(span_signature)
            .collect::<Vec<_>>()
            .join("|");
        format!("{}\0{}\0{spans}", self.path, self.kind)
    }
}

#[derive(Default)]
struct Counts {
    expected: u64,
    observed: u64,
    matched: u64,
}

struct PipelineFixture {
    _root: tempfile::TempDir,
    connection: Connection,
    repository_id: String,
    before_generation: String,
    after_generation: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct TemporalDiagnostic {
    before_rules: Vec<DiagnosticRuleSnapshot>,
    after_rules: Vec<DiagnosticRuleSnapshot>,
    events: Vec<DiagnosticEvent>,
}

type DiagnosticRuleSnapshot = (String, String, String, String);
type DiagnosticEvent = (
    String,
    Option<DiagnosticRuleSnapshot>,
    Option<DiagnosticRuleSnapshot>,
);

#[derive(Debug)]
struct DiagnosticRule {
    semantic_key: String,
    identity_input_key: String,
    label: String,
    details: String,
}

#[test]
fn real_pipeline_correctness_measurement_is_exact_and_reproducible() {
    let first = evaluate().expect("evaluate real archaeology pipeline");
    let second = evaluate().expect("repeat real archaeology pipeline");
    assert_eq!(
        first, second,
        "correctness measurement is not deterministic"
    );
    let encoded = encode(&first).expect("encode correctness measurement");
    if std::env::var_os("UPDATE_ARCHAEOLOGY_CORRECTNESS_MEASUREMENTS").is_some() {
        fs::write(report_path(), &encoded).expect("write correctness measurement");
        return;
    }
    assert_eq!(
        encoded, CHECKED,
        "regenerate with UPDATE_ARCHAEOLOGY_CORRECTNESS_MEASUREMENTS=1"
    );
    assert_eq!(first["model_usage"]["external_model_calls"], 0);
    assert_eq!(first["reviewer_correction_effort"]["human_reviewers"], 0);
    assert!(first["reviewer_correction_effort"]["measured_minutes"].is_null());
}

#[test]
#[ignore = "writes an explicit private reviewer export"]
fn write_private_human_review_export() {
    let path = std::env::var_os("CODEVETTER_ARCHAEOLOGY_REVIEW_EXPORT")
        .map(PathBuf::from)
        .expect("set CODEVETTER_ARCHAEOLOGY_REVIEW_EXPORT to an explicit output path");
    let fixture = PipelineFixture::new().expect("publish labeled archaeology fixture");
    let result = export_core(
        &fixture.connection,
        ArchaeologyExportInput {
            repository_id: fixture.repository_id,
            format: ArchaeologyExportFormat::Json,
            limit: Some(1_000),
            cursor: None,
        },
    )
    .expect("export labeled archaeology fixture");
    assert!(
        !result.truncated,
        "review qualification requires a complete export"
    );
    assert!(result.next_cursor.is_none());
    assert!(result.rule_count > 0, "review export must contain rules");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create private reviewer export directory");
    }
    fs::write(path, format!("{}\n", result.content)).expect("write private reviewer export");
}

#[test]
fn relation_case_metric_respects_alias_direction_and_conflict_symmetry() {
    let cases = vec![(
        BTreeSet::from(["rule:a".to_owned()]),
        BTreeSet::from(["rule:b".to_owned()]),
    )];
    let exact = vec![("rule:a".to_owned(), "rule:b".to_owned())];
    let reversed = vec![("rule:b".to_owned(), "rule:a".to_owned())];

    assert_eq!(relation_case_metric(&cases, &exact, true), (1, 1));
    assert_eq!(relation_case_metric(&cases, &reversed, true), (0, 0));
    assert_eq!(relation_case_metric(&cases, &reversed, false), (1, 1));
}

#[test]
fn duplicate_equivalence_closure_credits_consolidation_and_inherited_conflicts() {
    let groups = vec![GoldenDuplicateGroup {
        primary_rule_id: "rule:duplicate".into(),
        rule_ids: vec!["rule:duplicate".into(), "rule:source".into()],
    }];
    let mut candidates = BTreeMap::from([
        (
            "rule:duplicate".into(),
            BTreeSet::from(["actual:merged".into()]),
        ),
        (
            "rule:source".into(),
            BTreeSet::from(["actual:source".into(), "actual:merged".into()]),
        ),
    ]);

    close_duplicate_candidates(&groups, &mut candidates).expect("close exact duplicates");

    assert_eq!(
        candidates["rule:duplicate"],
        BTreeSet::from(["actual:merged".into(), "actual:source".into()])
    );
    assert_eq!(candidates["rule:duplicate"], candidates["rule:source"]);
    assert!(duplicate_group_is_consolidated((
        &candidates["rule:duplicate"],
        &candidates["rule:source"],
    )));
}

fn evaluate() -> Result<Value, String> {
    let corpus: Corpus = serde_json::from_slice(CORPUS)
        .map_err(|error| format!("Decode labeled archaeology corpus: {error}"))?;
    let fixture = PipelineFixture::new()?;
    if std::env::var_os("CODEVETTER_ARCHAEOLOGY_TEMPORAL_DIAGNOSTICS").is_some() {
        eprintln!(
            "ARCHAEOLOGY_TEMPORAL_DIAGNOSTIC {}",
            serde_json::to_string(&temporal_diagnostic(&fixture)?)
                .map_err(|error| format!("Encode temporal diagnostic: {error}"))?
        );
    }
    let publication_facts = load_facts(&fixture.connection, &fixture.after_generation)?;
    let (facts, adapter_edges) = run_source_aligned_adapters(&corpus)?;
    let units = corpus
        .source_units
        .iter()
        .map(|unit| (unit.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let spans = corpus
        .spans
        .iter()
        .map(|span| (span.id.as_str(), span))
        .collect::<BTreeMap<_, _>>();

    let mut golden_signatures = BTreeMap::<String, String>::new();
    let mut dialect_constructs = BTreeMap::<String, BTreeMap<String, Counts>>::new();
    let mut labeled_kinds = BTreeMap::<String, BTreeSet<String>>::new();
    for fact in &corpus.facts {
        let signature = golden_fact_signature(fact, &spans, &units)?;
        golden_signatures.insert(fact.id.clone(), signature.clone());
        let first_span = spans
            .get(
                fact.span_ids
                    .first()
                    .map(String::as_str)
                    .unwrap_or_default(),
            )
            .ok_or_else(|| format!("Golden fact {} has no span", fact.id))?;
        let unit = units
            .get(first_span.source_unit_id.as_str())
            .ok_or_else(|| format!("Golden fact {} has no unit", fact.id))?;
        let key = format!("{}/{}", unit.language, unit.dialect);
        dialect_constructs
            .entry(key)
            .or_default()
            .entry(fact.kind.clone())
            .or_default()
            .expected += 1;
        labeled_kinds
            .entry(unit.path.clone())
            .or_default()
            .insert(fact.kind.clone());
    }
    let actual_extracted = facts
        .iter()
        .filter(|fact| fact.trust == "extracted")
        .collect::<Vec<_>>();
    for fact in &actual_extracted {
        let Some(kinds) = labeled_kinds.get(&fact.path) else {
            continue;
        };
        if !kinds.contains(&fact.kind) {
            continue;
        }
        let unit = corpus
            .source_units
            .iter()
            .find(|unit| unit.path == fact.path)
            .ok_or_else(|| format!("Observed labeled path {} has no unit", fact.path))?;
        dialect_constructs
            .entry(format!("{}/{}", unit.language, unit.dialect))
            .or_default()
            .entry(fact.kind.clone())
            .or_default()
            .observed += 1;
    }
    let actual_signatures = actual_extracted
        .iter()
        .map(|fact| fact.signature())
        .collect::<BTreeSet<_>>();
    let mut golden_to_actual = BTreeMap::<String, Vec<String>>::new();
    for fact in &corpus.facts {
        let signature = golden_signatures
            .get(&fact.id)
            .ok_or_else(|| format!("Golden signature missing for {}", fact.id))?;
        let matches = actual_extracted
            .iter()
            .filter(|actual| actual.signature() == *signature)
            .map(|actual| actual.id.clone())
            .collect::<Vec<_>>();
        if !matches.is_empty() {
            let first_span = spans
                .get(fact.span_ids[0].as_str())
                .ok_or_else(|| format!("Golden span missing for {}", fact.id))?;
            let unit = units
                .get(first_span.source_unit_id.as_str())
                .ok_or_else(|| format!("Golden unit missing for {}", fact.id))?;
            dialect_constructs
                .entry(format!("{}/{}", unit.language, unit.dialect))
                .or_default()
                .entry(fact.kind.clone())
                .or_default()
                .matched += 1;
        }
        golden_to_actual.insert(fact.id.clone(), matches);
    }

    let adapter_matrix = dialect_constructs
        .into_iter()
        .map(|(dialect, constructs)| {
            let constructs = constructs
                .into_iter()
                .map(|(construct, counts)| {
                    let metric = metric(&counts);
                    (construct, metric)
                })
                .collect::<serde_json::Map<_, _>>();
            (dialect, json!({ "constructs": constructs }))
        })
        .collect::<serde_json::Map<_, _>>();

    let dependency = dependency_metrics(&corpus.edges, &golden_to_actual, &adapter_edges)?;
    let clause_support = clause_support(&fixture.connection, &fixture.after_generation)?;
    let golden_to_published =
        map_golden_facts(&corpus.facts, &golden_signatures, &publication_facts);
    let relation_metrics = relation_metrics(
        &fixture.connection,
        &fixture.after_generation,
        &corpus,
        &golden_to_published,
    )?;
    let canonical = canonical_read_metrics(&fixture)?;
    let temporal = temporal_metrics(&fixture, &corpus)?;
    let pipeline_identity = pipeline_identity(
        &fixture.connection,
        &fixture.after_generation,
        &publication_facts,
        &publication_facts
            .iter()
            .map(|fact| (fact.id.as_str(), fact))
            .collect(),
    )?;
    let mut qualification_blockers = Vec::new();
    for (key, label) in [
        ("contradictions", "labeled contradiction handling"),
        (
            "duplicate_reconciliation",
            "labeled duplicate reconciliation",
        ),
    ] {
        let metric = &relation_metrics[key];
        if metric["precision"].as_f64().unwrap_or(0.0) < 1.0
            || metric["recall"].as_f64().unwrap_or(0.0) < 1.0
        {
            qualification_blockers.push(format!(
                "{label} is below the exact precision and recall threshold"
            ));
        }
    }
    if temporal["precision"].as_f64().unwrap_or(0.0) < 1.0
        || temporal["recall"].as_f64().unwrap_or(0.0) < 1.0
    {
        qualification_blockers.push(
            "the labeled temporal condition change is below the exact evidence threshold"
                .to_string(),
        );
    }
    qualification_blockers.push("no recorded human review sample exists".to_string());

    let mut report = json!({
        "schema_version": 1,
        "report_id": "codevetter.business-rule-archaeology.real-pipeline-correctness.v1",
        "corpus_id": corpus.corpus_id,
        "input_identities": {
            "labeled_corpus": hash(CORPUS),
            "qualification_policy": hash(POLICY),
            "source_fixture_bundle": source_bundle_hash()?,
        },
        "pipeline": {
            "stages": ["inventory", "adapter", "link", "derive", "publish", "canonical_read"],
            "revision_count": 2,
            "zero_model": true,
            "normalized_output_identity": pipeline_identity,
        },
        "adapter_correctness": {
            "source_alignment": true,
            "status": "measured_directly_through_production_language_adapters",
            "match_contract": "fact kind plus exact original path/byte/line/column span; human paraphrase labels are not compared",
            "dialects": adapter_matrix,
            "labeled_fact_count": corpus.facts.len(),
            "observed_extracted_fact_count": actual_extracted.len(),
            "exactly_matched_fact_count": golden_signatures.values().filter(|signature| actual_signatures.contains(*signature)).count(),
        },
        "catalog_correctness": {
            "clause_support": clause_support,
            "contradictions": relation_metrics["contradictions"].clone(),
            "duplicate_reconciliation": relation_metrics["duplicate_reconciliation"].clone(),
            "retrieval": canonical["retrieval"].clone(),
            "reverse_lookup": canonical["reverse_lookup"].clone(),
            "dependency_paths": dependency,
            "temporal_diffs": temporal,
        },
        "reviewer_correction_effort": {
            "human_reviewers": 0,
            "reviewed_rule_sample": 0,
            "measured_minutes": Value::Null,
            "measured_edits": Value::Null,
            "status": "unavailable_no_recorded_human_review; edit distance is not substituted",
        },
        "model_usage": {
            "external_model_calls": synthesis_attempts(&fixture.connection)?,
            "input_tokens": 0,
            "output_tokens": 0,
            "reported_cost_microusd": 0,
        },
        "limitations": [
            "The checked corpus is small and is not a repository-scale qualification.",
            "The full labeled source bundle publishes after isolating unsupported units, deduplicating evidence and clauses, reconciling prose-only rules, filtering temporal aliases, accepting exact EOF spans, and rebuilding revision-scoped lineage.",
            "Adapter facts are source-aligned and measured directly; catalog publication/read metrics use the same full labeled two-revision workload.",
            "Repeated full-corpus publication is deterministic. Labeled contradiction, source-duplicate, and temporal condition-change cases are measured directly with full recall; temporal classification remains fail-closed while history or parser coverage is partial, and the generated-listing negative case remains excluded from semantic facts by contract.",
            "No human reviewed this generated sample, so correction minutes and edits remain unavailable.",
            "A failing metric is retained as measured evidence and is not converted into a supported-language claim.",
        ],
        "qualification": {
            "full_correctness_qualification": false,
            "passing_claim": Value::Null,
            "blockers": qualification_blockers,
        },
    });
    let payload = serde_json::to_vec(&report)
        .map_err(|error| format!("Encode correctness payload: {error}"))?;
    report["report_payload_sha256"] = json!(hash(&payload));
    Ok(report)
}

fn temporal_diagnostic(fixture: &PipelineFixture) -> Result<TemporalDiagnostic, String> {
    let before = diagnostic_rules(&fixture.connection, &fixture.before_generation)?;
    let after = diagnostic_rules(&fixture.connection, &fixture.after_generation)?;
    let temporal_generation: String = fixture
        .connection
        .query_row(
            "SELECT temporal_generation_identity FROM archaeology_temporal_generations
             WHERE repository_id=?1 AND generation_id=?2",
            (&fixture.repository_id, &fixture.after_generation),
            |row| row.get(0),
        )
        .map_err(|error| format!("Load diagnostic temporal generation: {error}"))?;
    let mut statement = fixture
        .connection
        .prepare(
            "SELECT event_kind,predecessor_rule_identity,successor_rule_identity
             FROM archaeology_rule_temporal_events
             WHERE repository_id=?1 AND temporal_generation_identity=?2
             ORDER BY event_kind,predecessor_rule_identity,successor_rule_identity,event_identity",
        )
        .map_err(|error| format!("Prepare temporal diagnostic events: {error}"))?;
    let rows = statement
        .query_map((&fixture.repository_id, &temporal_generation), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| format!("Query temporal diagnostic events: {error}"))?;
    let mut events = Vec::new();
    for row in rows {
        let (kind, predecessor, successor) =
            row.map_err(|error| format!("Read temporal diagnostic event: {error}"))?;
        events.push((
            kind,
            predecessor.map(|identity| {
                before
                    .get(&identity)
                    .map(|rule| {
                        (
                            rule.semantic_key.clone(),
                            rule.identity_input_key.clone(),
                            rule.label.clone(),
                            rule.details.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            "missing-before".into(),
                            "missing-before".into(),
                            "missing-before".into(),
                            "missing-before".into(),
                        )
                    })
            }),
            successor.map(|identity| {
                after
                    .get(&identity)
                    .map(|rule| {
                        (
                            rule.semantic_key.clone(),
                            rule.identity_input_key.clone(),
                            rule.label.clone(),
                            rule.details.clone(),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            "missing-after".into(),
                            "missing-after".into(),
                            "missing-after".into(),
                            "missing-after".into(),
                        )
                    })
            }),
        ));
    }
    events.sort();
    let mut before_rules = before
        .values()
        .map(|rule| {
            (
                rule.semantic_key.clone(),
                rule.identity_input_key.clone(),
                rule.label.clone(),
                rule.details.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut after_rules = after
        .values()
        .map(|rule| {
            (
                rule.semantic_key.clone(),
                rule.identity_input_key.clone(),
                rule.label.clone(),
                rule.details.clone(),
            )
        })
        .collect::<Vec<_>>();
    before_rules.sort();
    after_rules.sort();
    Ok(TemporalDiagnostic {
        before_rules,
        after_rules,
        events,
    })
}

fn diagnostic_rules(
    connection: &Connection,
    generation: &str,
) -> Result<BTreeMap<String, DiagnosticRule>, String> {
    let mut statement = connection
        .prepare(
            "SELECT rule.rule_id,rule.stable_rule_identity,rule.kind,rule.title
             FROM archaeology_rules rule
             WHERE rule.generation_id=?1 AND NOT EXISTS (
               SELECT 1 FROM archaeology_rule_relations alias
               WHERE alias.generation_id=rule.generation_id AND alias.kind='aliases'
                 AND alias.from_rule_id=rule.rule_id)
             ORDER BY rule.stable_rule_identity,rule.rule_id",
        )
        .map_err(|error| format!("Prepare diagnostic rules: {error}"))?;
    let rows = statement
        .query_map([generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("Query diagnostic rules: {error}"))?;
    let mut rules = BTreeMap::new();
    for row in rows {
        let (rule_id, stable_identity, kind, title) =
            row.map_err(|error| format!("Read diagnostic rule: {error}"))?;
        let clauses = query_diagnostic_strings(
            connection,
            "SELECT clause_text || char(0) || trust || char(0) || confidence || char(0) || caveats_json
             FROM archaeology_rule_clauses WHERE generation_id=?1 AND rule_id=?2
             ORDER BY ordinal,clause_id",
            generation,
            &rule_id,
        )?;
        let facts = query_diagnostic_strings(
            connection,
            "SELECT DISTINCT fact.kind || char(0) || fact.label || char(0) || fact.attributes_json
                    || char(0) || unit.relative_path || char(0) || span.start_byte || char(0)
                    || span.end_byte
             FROM archaeology_rule_clauses clause
             JOIN archaeology_evidence_links clause_fact
               ON clause_fact.generation_id=clause.generation_id
              AND clause_fact.owner_kind='rule_clause' AND clause_fact.owner_id=clause.clause_id
              AND clause_fact.evidence_kind='fact'
             JOIN archaeology_facts fact
               ON fact.generation_id=clause_fact.generation_id AND fact.fact_id=clause_fact.evidence_id
             JOIN archaeology_evidence_links fact_span
               ON fact_span.generation_id=fact.generation_id AND fact_span.owner_kind='fact'
              AND fact_span.owner_id=fact.fact_id AND fact_span.evidence_kind='span'
             JOIN archaeology_source_spans span
               ON span.generation_id=fact_span.generation_id AND span.span_id=fact_span.evidence_id
             JOIN archaeology_source_units unit
               ON unit.generation_id=span.generation_id AND unit.source_unit_id=span.source_unit_id
             WHERE clause.generation_id=?1 AND clause.rule_id=?2
             ORDER BY 1",
            generation,
            &rule_id,
        )?;
        let identity_facts = query_diagnostic_strings(
            connection,
            "SELECT DISTINCT clause_fact.role || char(0) || fact.kind || char(0)
                    || json_extract((SELECT value FROM json_each(fact.attributes_json)
                       WHERE json_extract(value,'$.key')='semantic_expr' LIMIT 1),'$.value')
             FROM archaeology_rule_clauses clause
             JOIN archaeology_evidence_links clause_fact
               ON clause_fact.generation_id=clause.generation_id
              AND clause_fact.owner_kind='rule_clause' AND clause_fact.owner_id=clause.clause_id
              AND clause_fact.evidence_kind='fact'
             JOIN archaeology_facts fact
               ON fact.generation_id=clause_fact.generation_id AND fact.fact_id=clause_fact.evidence_id
             WHERE clause.generation_id=?1 AND clause.rule_id=?2
             ORDER BY 1",
            generation,
            &rule_id,
        )?;
        let mut supporting = BTreeSet::new();
        for identity_fact in &identity_facts {
            let mut components = identity_fact.splitn(3, '\0');
            let role = components.next().unwrap_or_default();
            let fact_kind = components.next().unwrap_or_default();
            let semantic_expression = components.next().unwrap_or_default();
            if fact_kind.is_empty() || semantic_expression.is_empty() {
                return Err("Diagnostic rule identity fact is invalid".into());
            }
            if role == "supporting" {
                supporting.insert(format!("{fact_kind}\0{semantic_expression}"));
            }
        }
        let anchor = supporting
            .first()
            .ok_or("Diagnostic rule identity has no supporting anchor")?;
        let identity_payload = serde_json::to_vec(&(&kind, anchor, &supporting))
            .map_err(|error| format!("Encode diagnostic identity input: {error}"))?;
        let identity_input_key = hash(&identity_payload);
        let label = format!("{kind}:{title}");
        let details = serde_json::to_string(&(&clauses, &facts, &identity_facts))
            .map_err(|error| format!("Encode diagnostic rule details: {error}"))?;
        let payload = serde_json::to_vec(&(&kind, &title, &clauses, &facts))
            .map_err(|error| format!("Encode diagnostic rule: {error}"))?;
        let semantic_key = hash(&payload);
        if rules
            .insert(
                stable_identity,
                DiagnosticRule {
                    semantic_key,
                    identity_input_key,
                    label,
                    details,
                },
            )
            .is_some()
        {
            return Err("Diagnostic generation has duplicate stable rule identities".into());
        }
    }
    Ok(rules)
}

fn query_diagnostic_strings(
    connection: &Connection,
    query: &str,
    generation: &str,
    rule_id: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("Prepare diagnostic strings: {error}"))?;
    let rows = statement
        .query_map((generation, rule_id), |row| row.get::<_, String>(0))
        .map_err(|error| format!("Query diagnostic strings: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("Read diagnostic string: {error}")))
        .collect()
}

fn run_source_aligned_adapters(
    corpus: &Corpus,
) -> Result<(Vec<ActualFact>, Vec<ActualEdge>), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let mut actual_facts = Vec::new();
    let mut actual_edges = Vec::new();
    for unit in &corpus.source_units {
        if unit._protected {
            continue;
        }
        let source = fs::read(root.join(&unit.path))
            .map_err(|error| format!("Read labeled source {}: {error}", unit.path))?;
        let dialect = match unit.dialect.as_str() {
            "ibm-fixed" => "fixed",
            "ibm-copybook" => "copybook",
            "x86-64-gas-att" => "gas-att",
            value => value,
        };
        let classification = if unit.path.starts_with("generated/") {
            ArchaeologySourceClassification::Generated
        } else {
            ArchaeologySourceClassification::Source
        };
        let include_candidates = String::from_utf8_lossy(&source)
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let target = line.trim().strip_prefix("COPY ")?.trim_end_matches('.');
                Some(ArchaeologyIncludeCandidate {
                    kind: "copybook".into(),
                    target: target.into(),
                    line: index as u64 + 1,
                })
            })
            .collect();
        let revision_sha = if unit._revision == "previous" {
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        } else {
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        };
        let inventory = ArchaeologyInventoryUnit {
            identity: ArchaeologySourceUnitIdentity {
                source_unit_id: unit.id.clone(),
                repository_id: "repository:labeled-correctness".into(),
                revision_sha: revision_sha.into(),
                path_identity: format!("path:{}", unit.path),
                relative_path: Some(unit.path.clone()),
                content_hash: Some(format!("{:x}", Sha256::digest(&source))),
                hash_algorithm: Some("sha256".into()),
                change_identity: None,
            },
            classification,
            language: unit.language.clone(),
            dialect: Some(dialect.into()),
            byte_count: source.len() as u64,
            line_count: source.iter().filter(|byte| **byte == b'\n').count() as u64,
            include_candidates,
            coverage_reasons: Vec::new(),
        };
        let adapter: Box<dyn ArchaeologyLanguageAdapter> = match unit.language.as_str() {
            "typescript" => Box::new(ModernLanguageAdapter::new(SupportedLanguage::TypeScript)),
            "cobol" => Box::new(CobolAdapter::default()),
            "assembly" => Box::new(AssemblyAdapter::default()),
            language => {
                return Err(format!(
                    "No source-aligned adapter for labeled language {language}"
                ));
            }
        };
        let mut capture = AdapterCapture::default();
        run_archaeology_adapter(
            adapter.as_ref(),
            ArchaeologyAdapterInput {
                unit: &inventory,
                source: &source,
            },
            &mut capture,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .map_err(|error| format!("Parse labeled source {}: {error}", unit.path))?;
        let spans = capture
            .spans
            .iter()
            .map(|span| (span.span_id.as_str(), span))
            .collect::<BTreeMap<_, _>>();
        for fact in &capture.facts {
            let fact_spans = fact
                .span_ids
                .iter()
                .map(|span_id| {
                    let span = spans.get(span_id.as_str()).ok_or_else(|| {
                        format!("Adapter fact {} has no source span", fact.fact_id)
                    })?;
                    Ok(SpanKey {
                        path: unit.path.clone(),
                        start_byte: span.start.byte,
                        end_byte: span.end.byte,
                        start_line: span.start.line,
                        start_column: span.start.column,
                        end_line: span.end.line,
                        end_column: span.end.column,
                    })
                })
                .collect::<Result<BTreeSet<_>, String>>()?;
            actual_facts.push(ActualFact {
                id: fact.fact_id.clone(),
                kind: serde_json::to_value(&fact.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or("Serialize adapter fact kind")?,
                trust: "extracted".into(),
                path: unit.path.clone(),
                spans: fact_spans,
            });
        }
        for edge in capture.edges {
            actual_edges.push(ActualEdge {
                from: edge.from_fact_id,
                to: edge.to_fact_id,
                kind: serde_json::to_value(edge.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or("Serialize adapter edge kind")?,
            });
        }
    }
    Ok((actual_facts, actual_edges))
}

impl PipelineFixture {
    fn new() -> Result<Self, String> {
        let root = tempfile::tempdir().map_err(|error| format!("Create fixture repo: {error}"))?;
        let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
        copy_tree(&source_root, root.path())?;
        let current = root.path().join("modern/payment.ts");
        fs::copy(root.path().join("history/payment_v1.ts"), &current)
            .map_err(|error| format!("Install prior labeled source: {error}"))?;
        git(root.path(), &["init", "-q"])?;
        git(
            root.path(),
            &["config", "user.email", "qualification@example.invalid"],
        )?;
        git(
            root.path(),
            &["config", "user.name", "CodeVetter Qualification"],
        )?;
        git(root.path(), &["add", "."])?;
        git(root.path(), &["commit", "-qm", "labeled prior"])?;

        let connection = Connection::open_in_memory()
            .map_err(|error| format!("Open correctness database: {error}"))?;
        crate::db::archaeology_schema::run_migration(&connection)
            .map_err(|error| format!("Migrate archaeology database: {error}"))?;
        crate::db::history_graph_schema::run_migration(&connection)
            .map_err(|error| format!("Migrate history database: {error}"))?;
        let before_generation = refresh(&connection, root.path())
            .map_err(|error| format!("Publish prior correctness revision: {error}"))?;

        fs::copy(source_root.join("modern/payment.ts"), &current)
            .map_err(|error| format!("Install current labeled source: {error}"))?;
        git(root.path(), &["add", "modern/payment.ts"])?;
        git(root.path(), &["commit", "-qm", "labeled condition change"])?;
        let after_generation = refresh(&connection, root.path())
            .map_err(|error| format!("Publish current correctness revision: {error}"))?;
        let repository_id = connection
            .query_row(
                "SELECT repository_id FROM archaeology_repositories",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("Load correctness repository identity: {error}"))?;
        Ok(Self {
            _root: root,
            connection,
            repository_id,
            before_generation,
            after_generation,
        })
    }
}

fn refresh(connection: &Connection, root: &Path) -> Result<String, String> {
    let started = run_refresh(
        connection,
        ArchaeologyRefreshCommandInput {
            repo_path: root.to_string_lossy().into_owned(),
        },
    )?;
    let job_id = started
        .job_id
        .ok_or("Correctness refresh unexpectedly reused a generation")?;
    let completed = continue_refresh(
        connection,
        ArchaeologyRefreshContinueInput {
            job_id,
            max_steps: 64,
        },
    )?;
    if !completed.ready {
        return Err("Correctness refresh did not publish a ready generation".into());
    }
    Ok(started.repository_generation_id)
}

fn load_facts(connection: &Connection, generation: &str) -> Result<Vec<ActualFact>, String> {
    let mut facts = BTreeMap::<String, ActualFact>::new();
    let mut statement = connection
        .prepare(
            "SELECT fact.fact_id,fact.kind,fact.trust,unit.relative_path,
                    span.start_byte,span.end_byte,span.start_line,span.start_column,
                    span.end_line,span.end_column
             FROM archaeology_facts fact
             JOIN archaeology_evidence_links link
               ON link.generation_id=fact.generation_id AND link.owner_kind='fact'
              AND link.owner_id=fact.fact_id AND link.evidence_kind='span'
             JOIN archaeology_source_spans span
               ON span.generation_id=link.generation_id AND span.span_id=link.evidence_id
             JOIN archaeology_source_units unit
               ON unit.generation_id=span.generation_id AND unit.source_unit_id=span.source_unit_id
             WHERE fact.generation_id=?1
             ORDER BY fact.fact_id,span.start_byte,span.span_id",
        )
        .map_err(|error| format!("Prepare correctness facts: {error}"))?;
    let rows = statement
        .query_map([generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                SpanKey {
                    path: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    start_byte: row.get(4)?,
                    end_byte: row.get(5)?,
                    start_line: row.get(6)?,
                    start_column: row.get(7)?,
                    end_line: row.get(8)?,
                    end_column: row.get(9)?,
                },
            ))
        })
        .map_err(|error| format!("Read correctness facts: {error}"))?;
    for row in rows {
        let (id, kind, trust, path, span) =
            row.map_err(|error| format!("Decode correctness fact: {error}"))?;
        facts
            .entry(id.clone())
            .or_insert_with(|| ActualFact {
                id,
                kind,
                trust,
                path,
                spans: BTreeSet::new(),
            })
            .spans
            .insert(span);
    }
    Ok(facts.into_values().collect())
}

fn golden_fact_signature(
    fact: &GoldenFact,
    spans: &BTreeMap<&str, &GoldenSpan>,
    units: &BTreeMap<&str, &GoldenUnit>,
) -> Result<String, String> {
    let mut signatures = Vec::new();
    let mut path = None;
    for span_id in &fact.span_ids {
        let span = spans
            .get(span_id.as_str())
            .ok_or_else(|| format!("Golden fact {} references an unknown span", fact.id))?;
        let unit = units
            .get(span.source_unit_id.as_str())
            .ok_or_else(|| format!("Golden span {} references an unknown unit", span.id))?;
        if path
            .replace(unit.path.as_str())
            .is_some_and(|prior| prior != unit.path)
        {
            return Err(format!("Golden fact {} crosses source units", fact.id));
        }
        signatures.push(span_signature(&SpanKey {
            path: unit.path.clone(),
            start_byte: span.start[0],
            end_byte: span.end[0],
            start_line: span.start[1],
            start_column: span.start[2],
            end_line: span.end[1],
            end_column: span.end[2],
        }));
    }
    signatures.sort();
    Ok(format!(
        "{}\0{}\0{}",
        path.unwrap_or_default(),
        fact.kind,
        signatures.join("|")
    ))
}

fn dependency_metrics(
    golden: &[GoldenEdge],
    matches: &BTreeMap<String, Vec<String>>,
    adapter_edges: &[ActualEdge],
) -> Result<Value, String> {
    let scoped = matches.values().flatten().cloned().collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for edge in adapter_edges {
        if scoped.contains(&edge.from) && scoped.contains(&edge.to) {
            actual.insert((edge.from.clone(), edge.to.clone(), edge.kind.clone()));
        }
    }
    let mut expected = BTreeSet::new();
    for edge in golden {
        for from in matches.get(&edge.from).into_iter().flatten() {
            for to in matches.get(&edge.to).into_iter().flatten() {
                expected.insert((from.clone(), to.clone(), edge.kind.clone()));
            }
        }
    }
    let matched = expected.intersection(&actual).count() as u64;
    let evaluable = golden
        .iter()
        .filter(|edge| {
            matches
                .get(&edge.from)
                .is_some_and(|items| !items.is_empty())
                && matches.get(&edge.to).is_some_and(|items| !items.is_empty())
        })
        .count() as u64;
    Ok(json!({
        "labeled_paths": golden.len(),
        "evaluable_paths": evaluable,
        "observed_scoped_paths": actual.len(),
        "correct_paths": matched,
        "precision": ratio(matched, actual.len() as u64),
        "recall": ratio(matched, golden.len() as u64),
        "status": "measured_on_source_aligned_adapter_edges; cross_unit_linker_publication_is_blocked",
    }))
}

fn clause_support(connection: &Connection, generation: &str) -> Result<Value, String> {
    let total: u64 = scalar(
        connection,
        "SELECT COUNT(*) FROM archaeology_rule_clauses WHERE generation_id=?1",
        generation,
    )?;
    let supported: u64 = scalar(
        connection,
        "SELECT COUNT(*) FROM archaeology_rule_clauses clause
         WHERE clause.generation_id=?1 AND EXISTS (
           SELECT 1 FROM archaeology_evidence_links link
           WHERE link.generation_id=clause.generation_id AND link.owner_kind='rule_clause'
             AND link.owner_id=clause.clause_id AND link.evidence_kind='fact'
             AND link.role='supporting')",
        generation,
    )?;
    Ok(json!({
        "clause_count": total,
        "supported_clause_count": supported,
        "unsupported_clause_count": total.saturating_sub(supported),
        "supported_clause_rate": ratio(supported, total),
    }))
}

fn relation_metrics(
    connection: &Connection,
    generation: &str,
    corpus: &Corpus,
    golden_to_actual: &BTreeMap<String, Vec<String>>,
) -> Result<Value, String> {
    let actual_to_rules = actual_fact_rule_index(connection, generation)?;
    let mut rule_candidates = corpus
        .rules
        .iter()
        .map(|rule| {
            Ok((
                rule.id.clone(),
                golden_rule_candidates(rule, golden_to_actual, &actual_to_rules)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    // A canonical publication may consolidate exact duplicate occurrences
    // instead of retaining an alias edge. Close every labeled duplicate group
    // over the same candidate set so inherited relations are scored once.
    close_duplicate_candidates(&corpus.duplicate_groups, &mut rule_candidates)?;
    let conflicts = corpus
        .conflicts
        .iter()
        .map(|case| {
            if case.rule_ids.len() != 2 {
                return Err("Labeled contradiction must identify exactly two rules".into());
            }
            Ok((
                rule_candidates
                    .get(&case.rule_ids[0])
                    .ok_or("Labeled contradiction rule is unavailable")?
                    .clone(),
                rule_candidates
                    .get(&case.rule_ids[1])
                    .ok_or("Labeled contradiction rule is unavailable")?
                    .clone(),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let duplicates = corpus
        .duplicate_groups
        .iter()
        .map(|group| {
            let primary = rule_candidates
                .get(&group.primary_rule_id)
                .ok_or("Labeled duplicate primary rule is unavailable")?
                .clone();
            let mut aliases = BTreeSet::new();
            for rule_id in &group.rule_ids {
                if rule_id != &group.primary_rule_id {
                    aliases.extend(
                        rule_candidates
                            .get(rule_id)
                            .ok_or("Labeled duplicate rule is unavailable")?
                            .iter()
                            .cloned(),
                    );
                }
            }
            Ok((aliases, primary))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let conflict_relations = load_rule_relations(connection, generation, "conflicts_with")?;
    let alias_relations = load_rule_relations(connection, generation, "aliases")?;
    let conflict_metric = relation_case_metric(&conflicts, &conflict_relations, false);
    let duplicate_metric = relation_case_metric(&duplicates, &alias_relations, true);
    let consolidated_groups = duplicates
        .iter()
        .filter(|(aliases, primary)| duplicate_group_is_consolidated((aliases, primary)))
        .count();
    let matched_duplicate_groups = duplicates
        .iter()
        .filter(|case| {
            duplicate_group_is_consolidated((&case.0, &case.1))
                || alias_relations
                    .iter()
                    .any(|relation| case.0.contains(&relation.0) && case.1.contains(&relation.1))
        })
        .count();
    Ok(json!({
        "contradictions": {
            "labeled_cases": conflicts.len(),
            "observed_relations": conflict_relations.len(),
            "matched_cases": conflict_metric.0,
            "matched_relations": conflict_metric.1,
            "false_positive_relations": conflict_relations.len().saturating_sub(conflict_metric.1),
            "false_negative_cases": conflicts.len().saturating_sub(conflict_metric.0),
            "precision": ratio(conflict_metric.1 as u64, conflict_relations.len() as u64),
            "recall": ratio(conflict_metric.0 as u64, conflicts.len() as u64),
            "status": "measured_against_exact_labeled_fact_to_published_rule_mappings",
        },
        "duplicate_reconciliation": {
            "labeled_groups": duplicates.len(),
            "observed_alias_relations": alias_relations.len(),
            "matched_groups": matched_duplicate_groups,
            "consolidated_groups": consolidated_groups,
            "matched_alias_relations": duplicate_metric.1,
            "unmatched_alias_relations": alias_relations.len().saturating_sub(duplicate_metric.1),
            "false_positive_alias_relations": Value::Null,
            "false_negative_groups": duplicates.len().saturating_sub(matched_duplicate_groups),
            "precision": ratio(matched_duplicate_groups as u64, duplicates.len() as u64),
            "recall": ratio(matched_duplicate_groups as u64, duplicates.len() as u64),
            "alias_relation_precision": Value::Null,
            "status": "group_accuracy_measured_with_canonical_consolidation; global_alias_precision_not_evaluable_from_non_exhaustive_labels",
        }
    }))
}

fn close_duplicate_candidates(
    groups: &[GoldenDuplicateGroup],
    rule_candidates: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    for group in groups {
        let mut equivalent = BTreeSet::new();
        for rule_id in &group.rule_ids {
            equivalent.extend(
                rule_candidates
                    .get(rule_id)
                    .ok_or("Labeled duplicate rule is unavailable")?
                    .iter()
                    .cloned(),
            );
        }
        for rule_id in &group.rule_ids {
            rule_candidates.insert(rule_id.clone(), equivalent.clone());
        }
    }
    Ok(())
}

fn duplicate_group_is_consolidated(group: (&BTreeSet<String>, &BTreeSet<String>)) -> bool {
    !group.0.is_disjoint(group.1)
}

fn map_golden_facts(
    golden: &[GoldenFact],
    signatures: &BTreeMap<String, String>,
    published: &[ActualFact],
) -> BTreeMap<String, Vec<String>> {
    golden
        .iter()
        .map(|fact| {
            let signature = signatures
                .get(&fact.id)
                .expect("every decoded golden fact has a signature");
            let matches = published
                .iter()
                .filter(|actual| actual.trust == "extracted" && actual.signature() == *signature)
                .map(|actual| actual.id.clone())
                .collect();
            (fact.id.clone(), matches)
        })
        .collect()
}

fn actual_fact_rule_index(
    connection: &Connection,
    generation: &str,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut statement = connection
        .prepare(
            "SELECT evidence.evidence_id,clause.rule_id
             FROM archaeology_rule_clauses clause
             JOIN archaeology_evidence_links evidence
               ON evidence.generation_id=clause.generation_id
              AND evidence.owner_kind='rule_clause' AND evidence.owner_id=clause.clause_id
              AND evidence.evidence_kind='fact' AND evidence.role='supporting'
             WHERE clause.generation_id=?1
             ORDER BY evidence.evidence_id,clause.rule_id",
        )
        .map_err(|error| format!("Prepare correctness rule fact index: {error}"))?;
    let rows = statement
        .query_map([generation], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Read correctness rule fact index: {error}"))?;
    let mut index = BTreeMap::<String, BTreeSet<String>>::new();
    for row in rows {
        let (fact, rule) =
            row.map_err(|error| format!("Decode correctness rule fact index: {error}"))?;
        index.entry(fact).or_default().insert(rule);
    }
    Ok(index)
}

fn golden_rule_candidates(
    rule: &GoldenRule,
    golden_to_actual: &BTreeMap<String, Vec<String>>,
    actual_to_rules: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    let mut candidates = BTreeSet::new();
    for fact in rule
        .clauses
        .iter()
        .flat_map(|clause| &clause.supporting_fact_ids)
    {
        let actual = golden_to_actual
            .get(fact)
            .ok_or("Labeled rule references an unknown fact")?;
        for fact_id in actual {
            candidates.extend(actual_to_rules.get(fact_id).into_iter().flatten().cloned());
        }
    }
    Ok(candidates)
}

fn load_rule_relations(
    connection: &Connection,
    generation: &str,
    kind: &str,
) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT from_rule_id,to_rule_id FROM archaeology_rule_relations
             WHERE generation_id=?1 AND kind=?2 ORDER BY from_rule_id,to_rule_id",
        )
        .map_err(|error| format!("Prepare correctness rule relations: {error}"))?;
    let relations = statement
        .query_map((generation, kind), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("Read correctness rule relations: {error}"))?
        .map(|row| row.map_err(|error| format!("Decode correctness rule relation: {error}")))
        .collect();
    relations
}

fn relation_case_metric(
    cases: &[(BTreeSet<String>, BTreeSet<String>)],
    observed: &[(String, String)],
    directed: bool,
) -> (usize, usize) {
    let matches = |case: &(BTreeSet<String>, BTreeSet<String>), relation: &(String, String)| {
        (case.0.contains(&relation.0) && case.1.contains(&relation.1))
            || (!directed && case.0.contains(&relation.1) && case.1.contains(&relation.0))
    };
    let matched_cases = cases
        .iter()
        .filter(|case| observed.iter().any(|relation| matches(case, relation)))
        .count();
    let matched_relations = observed
        .iter()
        .filter(|relation| cases.iter().any(|case| matches(case, relation)))
        .count();
    (matched_cases, matched_relations)
}

fn canonical_read_metrics(fixture: &PipelineFixture) -> Result<Value, String> {
    let service = ArchaeologyReadService::new(&fixture.connection);
    let response = service
        .execute(ArchaeologyReadRequest::ListRules {
            repository_id: fixture.repository_id.clone(),
            filter: ArchaeologyRuleFilter::default(),
            limit: Some(500),
            cursor: None,
        })
        .map_err(|error| format!("Measure canonical catalog retrieval: {error}"))?;
    let ArchaeologyReadResponse::ListRules(page) = response else {
        return Err("Canonical catalog returned the wrong response kind".into());
    };
    let observed = page
        .items
        .iter()
        .map(|rule| rule.rule_id.clone())
        .collect::<BTreeSet<_>>();
    let expected = query_strings(
        &fixture.connection,
        "SELECT stable_rule_identity FROM archaeology_rules
         WHERE generation_id=?1 ORDER BY stable_rule_identity",
        &fixture.after_generation,
    )?;
    let retrieval = set_metric(&expected, &observed);

    let mut reverse_counts = Counts::default();
    let mut statement = fixture
        .connection
        .prepare(
            "SELECT DISTINCT path_identity FROM archaeology_source_units
             WHERE generation_id=?1 AND classification NOT IN ('protected','opaque')
             ORDER BY path_identity",
        )
        .map_err(|error| format!("Prepare reverse lookup paths: {error}"))?;
    let rows = statement
        .query_map([&fixture.after_generation], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Read reverse lookup paths: {error}"))?;
    for row in rows {
        let path_identity = row.map_err(|error| format!("Decode reverse lookup path: {error}"))?;
        let expected = expected_reverse_rules(
            &fixture.connection,
            &fixture.after_generation,
            &path_identity,
        )?;
        if expected.is_empty() {
            continue;
        }
        let response = service
            .execute(ArchaeologyReadRequest::ReverseSource {
                repository_id: fixture.repository_id.clone(),
                source: ArchaeologySourceSelector::Path {
                    path_identity: path_identity.clone(),
                },
                limit: Some(500),
                cursor: None,
            })
            .map_err(|error| format!("Measure reverse lookup for {path_identity}: {error}"))?;
        let ArchaeologyReadResponse::ReverseSource(page) = response else {
            return Err("Canonical reverse lookup returned the wrong response kind".into());
        };
        let observed = page
            .items
            .iter()
            .map(|rule| rule.rule_id.clone())
            .collect::<BTreeSet<_>>();
        reverse_counts.expected += expected.len() as u64;
        reverse_counts.observed += observed.len() as u64;
        reverse_counts.matched += expected.intersection(&observed).count() as u64;
    }
    Ok(json!({
        "retrieval": retrieval,
        "reverse_lookup": metric(&reverse_counts),
    }))
}

fn temporal_metrics(fixture: &PipelineFixture, corpus: &Corpus) -> Result<Value, String> {
    let service = ArchaeologyReadService::new(&fixture.connection);
    let response = service
        .execute(ArchaeologyReadRequest::CompareTemporal {
            repository_id: fixture.repository_id.clone(),
            before: ArchaeologyTemporalSelector::Generation {
                generation_id: fixture.before_generation.clone(),
            },
            after: ArchaeologyTemporalSelector::Generation {
                generation_id: fixture.after_generation.clone(),
            },
            limit: Some(500),
            cursor: None,
        })
        .map_err(|error| format!("Measure canonical temporal comparison: {error}"))?;
    let ArchaeologyReadResponse::CompareTemporal(result) = response else {
        return Err("Canonical temporal read returned the wrong response kind".into());
    };
    if result.value.before.generation_id != fixture.before_generation
        || result.value.after.generation_id != fixture.after_generation
    {
        return Err("Canonical temporal selectors resolved the wrong generations".into());
    }
    let (before_revision, before_path, before_hash) = published_source_version(
        &fixture.connection,
        &fixture.before_generation,
        "modern/payment.ts",
    )?;
    let (after_revision, after_path, after_hash) = published_source_version(
        &fixture.connection,
        &fixture.after_generation,
        "modern/payment.ts",
    )?;
    if result.value.before.revision_sha != before_revision
        || result.value.after.revision_sha != after_revision
    {
        return Err("Canonical temporal points do not carry exact published revisions".into());
    }
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let expected_before_hash = raw_hash(
        &fs::read(source_root.join("history/payment_v1.ts"))
            .map_err(|error| format!("Read prior temporal fixture: {error}"))?,
    );
    let expected_after_hash = raw_hash(
        &fs::read(source_root.join("modern/payment.ts"))
            .map_err(|error| format!("Read current temporal fixture: {error}"))?,
    );
    if before_hash != expected_before_hash || after_hash != expected_after_hash {
        return Err("Published temporal source versions do not match the labeled fixture".into());
    }

    let spans = corpus
        .spans
        .iter()
        .map(|span| (span.id.as_str(), span))
        .collect::<BTreeMap<_, _>>();
    let units = corpus
        .source_units
        .iter()
        .map(|unit| (unit.id.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut exact_candidates = BTreeSet::new();
    for label in &corpus.history_changes {
        if label.classification != "condition_changed"
            || label.from_revision != "previous"
            || label.to_revision != "current"
        {
            continue;
        }
        let before_span = labeled_temporal_span(label, "previous", &spans, &units)?;
        let after_span = labeled_temporal_span(label, "current", &spans, &units)?;
        for change in &result.value.changes {
            if temporal_snapshot_cites(
                change.before.as_ref(),
                &before_path,
                before_span.start[0],
                before_span.end[0],
            ) && temporal_snapshot_cites(
                change.after.as_ref(),
                &after_path,
                after_span.start[0],
                after_span.end[0],
            ) && persisted_event_cites_versions(
                &fixture.connection,
                &change.event_id,
                &before_path,
                &before_hash,
                &after_path,
                &after_hash,
            )? {
                exact_candidates.insert(change.event_id.clone());
            }
        }
    }
    let expected_changes = corpus.history_changes.len() as u64;
    let observed = result.value.changes.len() as u64;
    let changed = result
        .value
        .changes
        .iter()
        .filter(|change| change.classification == "changed")
        .count() as u64;
    let exact_candidate_count = exact_candidates.len() as u64;
    let matched = exact_candidate_count.min(expected_changes);
    Ok(json!({
        "labeled_changes": expected_changes,
        "observed_changes": observed,
        "observed_changed_classifications": changed,
        "exact_evidence_candidates": exact_candidate_count,
        "correct_changes": matched,
        "precision": ratio(matched, exact_candidate_count),
        "recall": ratio(matched, expected_changes),
        "status": "measured_through_canonical_temporal_read_with_exact_revisions_source_hashes_and_labeled_byte_ranges; classification_remains_fail_closed_under_partial_coverage",
        "before_revision_sha": before_revision,
        "after_revision_sha": after_revision,
        "before_source_identity": format!("sha256:{before_hash}"),
        "after_source_identity": format!("sha256:{after_hash}"),
        "coverage": result.value.coverage,
        "coverage_reasons": result.value.reasons,
    }))
}

fn published_source_version(
    connection: &Connection,
    generation: &str,
    path: &str,
) -> Result<(String, String, String), String> {
    connection
        .query_row(
            "SELECT generation.revision_sha,unit.path_identity,unit.content_hash
             FROM archaeology_source_units unit
             JOIN archaeology_generations generation
               ON generation.generation_id=unit.generation_id
             WHERE unit.generation_id=?1 AND unit.relative_path=?2",
            (generation, path),
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("Load exact published temporal source {path}: {error}"))
}

fn labeled_temporal_span<'a>(
    change: &GoldenHistoryChange,
    revision: &str,
    spans: &BTreeMap<&str, &'a GoldenSpan>,
    units: &BTreeMap<&str, &'a GoldenUnit>,
) -> Result<&'a GoldenSpan, String> {
    change
        .span_ids
        .iter()
        .filter_map(|id| spans.get(id.as_str()).copied())
        .find(|span| {
            units
                .get(span.source_unit_id.as_str())
                .is_some_and(|unit| unit._revision == revision)
        })
        .ok_or_else(|| format!("Temporal label has no {revision} evidence span"))
}

fn temporal_snapshot_cites(
    snapshot: Option<&ArchaeologyTemporalSnapshot>,
    path_identity: &str,
    labeled_start: u64,
    labeled_end: u64,
) -> bool {
    snapshot.is_some_and(|snapshot| {
        snapshot
            .payload
            .clauses
            .iter()
            .flat_map(|clause| &clause.evidence)
            .flat_map(|evidence| &evidence.spans)
            .any(|span| {
                span.path_identity == path_identity
                    && span.start_byte >= labeled_start
                    && span.end_byte <= labeled_end
            })
    })
}

fn persisted_event_cites_versions(
    connection: &Connection,
    event_id: &str,
    before_path: &str,
    before_hash: &str,
    after_path: &str,
    after_hash: &str,
) -> Result<bool, String> {
    let (before, after): (String, String) = connection
        .query_row(
            "SELECT before.payload_json,after.payload_json
             FROM archaeology_rule_temporal_events event
             JOIN archaeology_rule_temporal_snapshots before
               ON before.snapshot_identity=event.before_snapshot_identity
             JOIN archaeology_rule_temporal_snapshots after
               ON after.snapshot_identity=event.after_snapshot_identity
             WHERE event.event_identity=?1",
            [event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("Load exact persisted temporal evidence: {error}"))?;
    let before: ArchaeologyTemporalSnapshotPayload = serde_json::from_str(&before)
        .map_err(|error| format!("Decode prior temporal snapshot: {error}"))?;
    let after: ArchaeologyTemporalSnapshotPayload = serde_json::from_str(&after)
        .map_err(|error| format!("Decode current temporal snapshot: {error}"))?;
    Ok(
        snapshot_payload_cites_version(&before, before_path, before_hash)
            && snapshot_payload_cites_version(&after, after_path, after_hash),
    )
}

fn snapshot_payload_cites_version(
    snapshot: &ArchaeologyTemporalSnapshotPayload,
    path_identity: &str,
    content_hash: &str,
) -> bool {
    snapshot
        .clauses
        .iter()
        .flat_map(|clause| &clause.evidence)
        .flat_map(|evidence| &evidence.spans)
        .any(|span| span.path_identity == path_identity && span.content_hash == content_hash)
}

fn pipeline_identity(
    connection: &Connection,
    generation: &str,
    facts: &[ActualFact],
    fact_by_id: &BTreeMap<&str, &ActualFact>,
) -> Result<String, String> {
    let normalized_facts = facts
        .iter()
        .map(ActualFact::signature)
        .collect::<BTreeSet<_>>();
    let mut normalized_edges = BTreeSet::new();
    let mut statement = connection
        .prepare(
            "SELECT from_fact_id,to_fact_id,kind FROM archaeology_fact_edges
             WHERE generation_id=?1 ORDER BY from_fact_id,to_fact_id,kind",
        )
        .map_err(|error| format!("Prepare pipeline identity edges: {error}"))?;
    let rows = statement
        .query_map([generation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("Read pipeline identity edges: {error}"))?;
    for row in rows {
        let (from, to, kind) = row.map_err(|error| format!("Decode pipeline edge: {error}"))?;
        let Some(from) = fact_by_id.get(from.as_str()) else {
            continue;
        };
        let Some(to) = fact_by_id.get(to.as_str()) else {
            continue;
        };
        normalized_edges.insert(format!(
            "{}\0{}\0{}",
            from.signature(),
            kind,
            to.signature()
        ));
    }
    let normalized_rules = query_strings(
        connection,
        "SELECT kind || char(0) || title || char(0) || lifecycle
         FROM archaeology_rules WHERE generation_id=?1 ORDER BY kind,title,lifecycle",
        generation,
    )?;
    let bytes = serde_json::to_vec(&json!({
        "facts": normalized_facts,
        "edges": normalized_edges,
        "rules": normalized_rules,
    }))
    .map_err(|error| format!("Encode normalized pipeline identity: {error}"))?;
    Ok(hash(&bytes))
}

fn expected_reverse_rules(
    connection: &Connection,
    generation: &str,
    path_identity: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT rule.stable_rule_identity
             FROM archaeology_rules rule
             JOIN archaeology_rule_clauses clause
               ON clause.generation_id=rule.generation_id AND clause.rule_id=rule.rule_id
             JOIN archaeology_evidence_links clause_fact
               ON clause_fact.generation_id=clause.generation_id
              AND clause_fact.owner_kind='rule_clause' AND clause_fact.owner_id=clause.clause_id
              AND clause_fact.evidence_kind='fact'
             JOIN archaeology_evidence_links fact_span
               ON fact_span.generation_id=clause_fact.generation_id
              AND fact_span.owner_kind='fact' AND fact_span.owner_id=clause_fact.evidence_id
              AND fact_span.evidence_kind='span'
             JOIN archaeology_source_spans span
               ON span.generation_id=fact_span.generation_id AND span.span_id=fact_span.evidence_id
             JOIN archaeology_source_units unit
               ON unit.generation_id=span.generation_id AND unit.source_unit_id=span.source_unit_id
             WHERE rule.generation_id=?1 AND unit.path_identity=?2
             ORDER BY rule.stable_rule_identity",
        )
        .map_err(|error| format!("Prepare reverse oracle: {error}"))?;
    let rows = statement
        .query_map((generation, path_identity), |row| row.get::<_, String>(0))
        .map_err(|error| format!("Read reverse oracle: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("Decode reverse oracle: {error}")))
        .collect()
}

fn set_metric(expected: &BTreeSet<String>, observed: &BTreeSet<String>) -> Value {
    let counts = Counts {
        expected: expected.len() as u64,
        observed: observed.len() as u64,
        matched: expected.intersection(observed).count() as u64,
    };
    metric(&counts)
}

fn metric(counts: &Counts) -> Value {
    json!({
        "expected": counts.expected,
        "observed": counts.observed,
        "matched": counts.matched,
        "false_positives": counts.observed.saturating_sub(counts.matched),
        "false_negatives": counts.expected.saturating_sub(counts.matched),
        "precision": ratio(counts.matched, counts.observed),
        "recall": ratio(counts.matched, counts.expected),
    })
}

fn ratio(numerator: u64, denominator: u64) -> Value {
    if denominator == 0 {
        Value::Null
    } else {
        json!((numerator as f64 / denominator as f64 * 1_000_000.0).round() / 1_000_000.0)
    }
}

fn scalar(connection: &Connection, query: &str, generation: &str) -> Result<u64, String> {
    connection
        .query_row(query, [generation], |row| row.get(0))
        .map_err(|error| format!("Read correctness scalar: {error}"))
}

fn query_strings(
    connection: &Connection,
    query: &str,
    generation: &str,
) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("Prepare correctness strings: {error}"))?;
    let rows = statement
        .query_map([generation], |row| row.get::<_, String>(0))
        .map_err(|error| format!("Read correctness strings: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("Decode correctness string: {error}")))
        .collect()
}

fn synthesis_attempts(connection: &Connection) -> Result<u64, String> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM archaeology_synthesis_attempts",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Count correctness model attempts: {error}"))
}

fn span_signature(span: &SpanKey) -> String {
    format!(
        "{}:{}-{}:{}:{}-{}:{}",
        span.path,
        span.start_byte,
        span.end_byte,
        span.start_line,
        span.start_column,
        span.end_line,
        span.end_column
    )
}

fn source_bundle_hash() -> Result<String, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let mut files = Vec::new();
    collect_files(&root, &root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    for (path, bytes) in files {
        digest.update(path.as_bytes());
        digest.update(b"\0");
        digest.update(bytes);
        digest.update(b"\0");
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("Read fixture directory: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read fixture entry: {error}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "Fixture path escaped its root".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.push((
                relative,
                fs::read(&path).map_err(|error| format!("Read fixture source: {error}"))?,
            ));
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    collect_files(source, source, &mut files)?;
    for (relative, bytes) in files {
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Create fixture directory: {error}"))?;
        }
        fs::write(target, bytes).map_err(|error| format!("Write fixture source: {error}"))?;
    }
    Ok(())
}

fn git(root: &Path, arguments: &[&str]) -> Result<(), String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .output()
        .map_err(|error| format!("Run fixture Git: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Fixture Git {:?}: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn raw_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Encode correctness report: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn report_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/business-rule-archaeology/real-pipeline-correctness-v1.json")
}
