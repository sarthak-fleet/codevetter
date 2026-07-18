use super::*;
use crate::commands::business_rule_archaeology::assembly_adapter::AssemblyAdapter;
use crate::commands::business_rule_archaeology::cobol_adapter::CobolAdapter;
use crate::commands::business_rule_archaeology::contracts::{
    ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyFactEdgeKind, ArchaeologyFactKind,
    ArchaeologyPosition, ArchaeologySourceUnitIdentity,
};
use crate::commands::business_rule_archaeology::modern_adapter::ModernLanguageAdapter;
use crate::commands::structural_graph::language::SupportedLanguage;
use std::cell::Cell;

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SOURCE: &[u8] = b"CHECK\nWRITE\nBROKEN";

#[test]
fn semantic_expression_is_bounded_private_and_token_boundary_safe() {
    let spaced = semantic_expression("AMOUNT   >\n 0", true).unwrap();
    let canonical = semantic_expression("amount>0", true).unwrap();
    assert_eq!(spaced, canonical);
    assert_ne!(
        semantic_expression("AB C", true).unwrap(),
        semantic_expression("A BC", true).unwrap()
    );
    assert_eq!(
        semantic_expression("cmpq $0, %rdi", false).unwrap(),
        semantic_expression("cmpq $0,%rdi", false).unwrap()
    );
    assert_ne!(
        semantic_expression("A-B", true).unwrap(),
        semantic_expression("A - B", true).unwrap()
    );
    assert_ne!(
        semantic_expression("AMOUNT > 0", true).unwrap(),
        semantic_expression("AMOUNT > 100", true).unwrap()
    );
    assert!(canonical.starts_with("v1:sha256:") && !canonical.contains("AMOUNT"));
    assert!(semantic_expression("", true).is_err());
    assert!(semantic_expression(&"X".repeat(64 * 1024 + 1), true).is_err());
}

#[test]
fn fixture_adapter_streams_exact_cited_output_and_parser_metadata() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected::default();
    let outcome = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect("valid fixture adapter");

    assert_eq!(outcome.parser_identity, "fixture-parser@1");
    assert_eq!(
        (outcome.span_count, outcome.fact_count, outcome.edge_count),
        (3, 2, 1)
    );
    assert!(outcome.output_bytes > 0);
    assert_eq!(outcome.metadata.dialect.as_deref(), Some("fixture-dialect"));
    assert_eq!(outcome.metadata.lineage.len(), 2);
    assert_eq!(outcome.metadata.regions.len(), 3);
    assert_eq!(
        outcome.metadata.regions[2].kind,
        ArchaeologyAdapterRegionKind::Unsupported
    );
    assert_eq!(output.spans[0].start, position(0, 1, 1));
    assert_eq!(output.spans[0].end, position(5, 1, 6));
    assert_eq!(output.spans[2].end, position(18, 3, 7));
    assert_eq!(output.facts[0].span_ids, ["span-check"]);
    assert_eq!(output.edges[0].evidence_span_ids, ["span-check"]);
    assert!(output.committed);
}

#[test]
fn stream_enforces_count_and_byte_bounds_without_buffering_a_batch() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits {
            max_facts: 1,
            ..ArchaeologyAdapterLimits::default()
        },
    )
    .expect_err("second fact exceeds bound");
    assert!(error.contains("fact count"));
    assert!(
        output.emitted > 0,
        "validated events reached the transactional sink"
    );
    assert!(output.facts.is_empty(), "failed output was rolled back");

    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits {
            max_output_bytes: 1,
            ..ArchaeologyAdapterLimits::default()
        },
    )
    .expect_err("serialized output exceeds bound");
    assert!(error.contains("byte bound"));
    assert!(output.spans.is_empty());
}

#[test]
fn cancellation_duplicate_ids_and_dangling_edges_fail_closed() {
    for (mode, expected) in [
        (Mode::CancelAfterSpan, "cancelled"),
        (Mode::DuplicateFact, "duplicate fact"),
        (Mode::DanglingEdge, "dangling edge"),
        (Mode::SwallowedError, "duplicate fact"),
        (Mode::InvalidMetadata, "unsupported dialect"),
        (Mode::UncitedDialect, "metadata contains an empty value"),
        (Mode::UncoveredRegion, "coverage reasons"),
        (Mode::ParserError, "fixture parser failed"),
        (Mode::Panic, "adapter panicked"),
    ] {
        let unit = unit(ArchaeologySourceClassification::Source);
        let adapter = FixtureAdapter::new(mode);
        let mut output = Collected::default();
        let error = run_archaeology_adapter(
            &adapter,
            ArchaeologyAdapterInput {
                unit: &unit,
                source: SOURCE,
            },
            &mut output,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .expect_err(expected);
        assert!(error.contains(expected), "{error}");
        assert!(output.spans.is_empty());
        assert!(output.facts.is_empty());
        assert!(output.edges.is_empty());
        assert!(!output.committed);
    }
}

#[test]
fn commit_failure_aborts_the_transactional_sink() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected {
        fail_commit: true,
        ..Collected::default()
    };
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect_err("commit failure");
    assert!(error.contains("fixture commit failed"));
    assert!(output.spans.is_empty());
    assert!(output.facts.is_empty());
    assert!(output.edges.is_empty());
    assert!(!output.committed);
}

#[test]
fn protected_source_is_refused_before_adapter_execution() {
    let unit = unit(ArchaeologySourceClassification::Protected);
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect_err("protected source refused");
    assert!(error.contains("protected source"));
    assert_eq!(adapter.calls.get(), 0);
}

#[test]
fn invalid_utf8_and_wrong_language_are_isolated_before_execution() {
    let invalid_utf8 = [0xff, b'\n'];
    for (source, language, expected) in [
        (invalid_utf8.as_slice(), "fixture", "UTF-8"),
        (SOURCE, "cobol", "language does not match"),
    ] {
        let mut unit = unit(ArchaeologySourceClassification::Source);
        unit.identity.content_hash = Some(hex(&Sha256::digest(source)));
        unit.byte_count = source.len() as u64;
        unit.line_count = source.iter().filter(|byte| **byte == b'\n').count() as u64;
        unit.language = language.to_string();
        let adapter = FixtureAdapter::new(Mode::Valid);
        let mut output = Collected::default();
        let error = run_archaeology_adapter(
            &adapter,
            ArchaeologyAdapterInput {
                unit: &unit,
                source,
            },
            &mut output,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .expect_err(expected);
        assert!(error.contains(expected), "{error}");
        assert_eq!(adapter.calls.get(), 0);
        assert!(!output.begun && !output.committed);
        assert!(output.spans.is_empty() && output.facts.is_empty() && output.edges.is_empty());
    }
}

#[test]
fn adapter_matrix_declares_the_complete_normalized_fact_vocabulary() {
    let modern = ModernLanguageAdapter::new(SupportedLanguage::TypeScript);
    let cobol = CobolAdapter::default();
    let assembly = AssemblyAdapter::default();
    let matrix: [&dyn ArchaeologyLanguageAdapter; 3] = [&modern, &cobol, &assembly];
    for kind in [
        ArchaeologyFactKind::Declaration,
        ArchaeologyFactKind::DataField,
        ArchaeologyFactKind::Constant,
        ArchaeologyFactKind::Predicate,
        ArchaeologyFactKind::Decision,
        ArchaeologyFactKind::Calculation,
        ArchaeologyFactKind::Mutation,
        ArchaeologyFactKind::Call,
        ArchaeologyFactKind::InputOutput,
        ArchaeologyFactKind::Transaction,
        ArchaeologyFactKind::ControlFlow,
        ArchaeologyFactKind::EntryPoint,
        ArchaeologyFactKind::Include,
    ] {
        assert!(
            matrix
                .iter()
                .any(|adapter| adapter.capability().constructs.contains(&kind)),
            "normalized fact kind is not emitted by any adapter: {kind:?}"
        );
    }
}

#[test]
fn modern_semantic_expressions_ignore_opaque_repository_and_revision_identity() {
    const MODERN_SOURCE: &[u8] =
        b"export function authorize(amount: number) {\n  if (amount > 0) return amount;\n  return 0;\n}\n";
    let adapter = ModernLanguageAdapter::new(SupportedLanguage::TypeScript);
    let mut first_unit = unit(ArchaeologySourceClassification::Source);
    first_unit.identity.source_unit_id = "source-unit:modern-one".into();
    first_unit.identity.repository_id = "repository:modern-one".into();
    first_unit.identity.path_identity = "path:modern-one".into();
    first_unit.identity.relative_path = Some("src/authorize.ts".into());
    first_unit.identity.content_hash = Some(hex(&Sha256::digest(MODERN_SOURCE)));
    first_unit.language = "typescript".into();
    first_unit.dialect = Some("typescript".into());
    first_unit.byte_count = MODERN_SOURCE.len() as u64;
    first_unit.line_count = 4;
    let mut second_unit = first_unit.clone();
    second_unit.identity.source_unit_id = "source-unit:modern-two".into();
    second_unit.identity.repository_id = "repository:modern-two".into();
    second_unit.identity.revision_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    second_unit.identity.path_identity = "path:modern-two".into();

    let semantic_expressions = |unit: &ArchaeologyInventoryUnit| {
        let mut output = Collected::default();
        run_archaeology_adapter(
            &adapter,
            ArchaeologyAdapterInput {
                unit,
                source: MODERN_SOURCE,
            },
            &mut output,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .expect("modern adapter");
        output
            .facts
            .iter()
            .map(|fact| {
                (
                    fact.kind.clone(),
                    fact.label.clone(),
                    fact.attributes
                        .iter()
                        .find(|attribute| attribute.key == "semantic_expr")
                        .expect("semantic expression")
                        .value
                        .clone(),
                )
            })
            .collect::<Vec<_>>()
    };

    let first = semantic_expressions(&first_unit);
    assert!(!first.is_empty());
    assert_eq!(first, semantic_expressions(&second_unit));
}

#[test]
fn secret_shaped_fact_content_fails_closed_without_rejecting_benign_identifiers() {
    for mode in [Mode::SecretLabel, Mode::SecretAttribute] {
        let mut output = Collected::default();
        let error = run_archaeology_adapter(
            &FixtureAdapter::new(mode),
            ArchaeologyAdapterInput {
                unit: &unit(ArchaeologySourceClassification::Source),
                source: SOURCE,
            },
            &mut output,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .expect_err("secret-shaped fact content");
        assert!(error.contains("secret-shaped fact content"), "{error}");
        assert!(output.facts.is_empty(), "failed unit must roll back");
    }
    let mut output = Collected::default();
    run_archaeology_adapter(
        &FixtureAdapter::new(Mode::BenignCredentialIdentifier),
        ArchaeologyAdapterInput {
            unit: &unit(ArchaeologySourceClassification::Source),
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect("benign credential identifier");
    assert_eq!(output.facts[0].label, "credentials");
}

#[test]
fn forged_safe_classification_cannot_bypass_the_central_sensitive_path_policy() {
    let mut unit = unit(ArchaeologySourceClassification::Source);
    unit.identity.relative_path = Some("config/.env.production".to_string());
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect_err("central path policy must override forged classification");
    assert!(error.contains("protected source path"), "{error}");
    assert_eq!(adapter.calls.get(), 0);
    assert!(!output.begun);
}

#[test]
fn cancellation_after_validation_aborts_immediately_before_commit() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let adapter = FixtureAdapter::new(Mode::Empty);
    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel_after_checks(3);
    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &cancellation,
        ArchaeologyAdapterLimits::default(),
    )
    .expect_err("late cancellation must win over commit");
    assert!(error.contains("cancelled before commit"), "{error}");
    assert_eq!(cancellation.check_count(), 3);
    assert!(!output.committed);
    assert!(!output.begun);
}

#[test]
fn source_bytes_must_match_the_inventoried_content_hash() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let adapter = FixtureAdapter::new(Mode::Valid);
    let mut output = Collected::default();
    let error = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: b"CHOCK\nWRITE\nBROKEN",
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .expect_err("same-length stale source must fail");
    assert!(error.contains("inventoried hash"));
    assert_eq!(adapter.calls.get(), 0);
}

#[test]
fn capability_and_exact_span_contracts_reject_unqualified_output() {
    let unit = unit(ArchaeologySourceClassification::Source);
    let mut adapter = FixtureAdapter::new(Mode::Valid);
    adapter.capability.exact_spans = false;
    let mut output = Collected::default();
    assert!(run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap_err()
    .contains("capability"));

    let adapter = FixtureAdapter::new(Mode::ZeroBasedSpan);
    assert!(run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source: SOURCE,
        },
        &mut output,
        &StructuralGraphCancellation::default(),
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap_err()
    .contains("one-based"));
}

#[test]
fn emitted_facts_and_edges_cannot_exceed_the_declared_extracted_contract() {
    for (mode, expected) in [
        (Mode::UndeclaredFact, "invalid or duplicate fact"),
        (Mode::InvalidAttributeKey, "invalid or duplicate fact"),
        (Mode::UntrustedEdge, "invalid, duplicate, or dangling edge"),
    ] {
        let unit = unit(ArchaeologySourceClassification::Source);
        let adapter = FixtureAdapter::new(mode);
        let mut output = Collected::default();
        let error = run_archaeology_adapter(
            &adapter,
            ArchaeologyAdapterInput {
                unit: &unit,
                source: SOURCE,
            },
            &mut output,
            &StructuralGraphCancellation::default(),
            ArchaeologyAdapterLimits::default(),
        )
        .expect_err(expected);
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn position_index_is_unicode_exact_and_bounded_by_source_bytes() {
    let source = format!("{}\n{}tail", "é".repeat(1_024), "x".repeat(1_024));
    let index = SourcePositionIndex::new(&source);
    assert!(index.matches(&source, &position(2_048, 1, 1_025)));
    assert!(index.matches(&source, &position(2_049, 2, 1)));
    assert!(index.matches(&source, &position(3_077, 2, 1_029)));
    assert_eq!(index.byte_at(&source, 1, 2_049), Some(2_048));
    assert_eq!(index.byte_at(&source, 2, 1_029), Some(3_077));
    assert_eq!(index.position(&source, 2_049), Some(position(2_049, 2, 1)));
    assert_eq!(index.byte_at(&source, 1, 2_050), None);
    assert_eq!(index.byte_at(&source, 3, 1), None);
    assert!(index.checkpoints.len() <= source.len() / POSITION_STRIDE + 1);
}

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    Empty,
    CancelAfterSpan,
    DuplicateFact,
    DanglingEdge,
    ZeroBasedSpan,
    UndeclaredFact,
    InvalidAttributeKey,
    UntrustedEdge,
    SwallowedError,
    InvalidMetadata,
    ParserError,
    Panic,
    UncitedDialect,
    UncoveredRegion,
    SecretLabel,
    SecretAttribute,
    BenignCredentialIdentifier,
}

struct FixtureAdapter {
    capability: ArchaeologyParserCapability,
    mode: Mode,
    calls: Cell<usize>,
}

impl FixtureAdapter {
    fn new(mode: Mode) -> Self {
        Self {
            capability: ArchaeologyParserCapability {
                parser_id: "fixture-parser".to_string(),
                parser_version: "1".to_string(),
                language: "fixture".to_string(),
                dialects: vec!["fixture-dialect".to_string()],
                constructs: vec![
                    ArchaeologyFactKind::Predicate,
                    ArchaeologyFactKind::Mutation,
                ],
                exact_spans: true,
                preprocessing: true,
                recovery: true,
            },
            mode,
            calls: Cell::new(0),
        }
    }
}

impl ArchaeologyLanguageAdapter for FixtureAdapter {
    fn capability(&self) -> &ArchaeologyParserCapability {
        &self.capability
    }

    fn parse(
        &self,
        input: ArchaeologyAdapterInput<'_>,
        output: &mut dyn ArchaeologyAdapterEvents,
        _positions: &SourcePositionIndex,
        cancellation: &StructuralGraphCancellation,
    ) -> Result<ArchaeologyAdapterMetadata, String> {
        self.calls.set(self.calls.get() + 1);
        if matches!(self.mode, Mode::Empty) {
            return Ok(ArchaeologyAdapterMetadata {
                dialect: None,
                dialect_evidence: Vec::new(),
                lineage: Vec::new(),
                regions: Vec::new(),
                coverage_reasons: Vec::new(),
            });
        }
        output.emit_span(span(
            &input,
            "span-check",
            if matches!(self.mode, Mode::ZeroBasedSpan) {
                position(0, 0, 0)
            } else {
                position(0, 1, 1)
            },
            position(5, 1, 6),
        ))?;
        if matches!(self.mode, Mode::ParserError) {
            return Err("fixture parser failed".to_string());
        }
        if matches!(self.mode, Mode::Panic) {
            panic!("fixture parser panic");
        }
        if matches!(self.mode, Mode::CancelAfterSpan) {
            cancellation.cancel();
        }
        output.emit_span(span(
            &input,
            "span-write",
            position(6, 2, 1),
            position(11, 2, 6),
        ))?;
        output.emit_span(span(
            &input,
            "span-broken",
            position(12, 3, 1),
            position(18, 3, 7),
        ))?;
        let mut first = fact("fact-check", ArchaeologyFactKind::Predicate, "span-check");
        match self.mode {
            Mode::SecretLabel => first.label = "Authorization: Bearer fixture-runtime-token".into(),
            Mode::SecretAttribute => first.attributes.push(ArchaeologyAttribute {
                key: "password".into(),
                value: "correct-horse-battery-staple".into(),
            }),
            Mode::BenignCredentialIdentifier => first.label = "credentials".into(),
            Mode::InvalidAttributeKey => first.attributes.push(ArchaeologyAttribute {
                key: "Qualified Name".into(),
                value: "fixture".into(),
            }),
            _ => {}
        }
        output.emit_fact(first)?;
        let second_id = if matches!(self.mode, Mode::DuplicateFact) {
            "fact-check"
        } else {
            "fact-write"
        };
        output.emit_fact(fact(
            second_id,
            if matches!(self.mode, Mode::UndeclaredFact) {
                ArchaeologyFactKind::Call
            } else {
                ArchaeologyFactKind::Mutation
            },
            "span-write",
        ))?;
        if matches!(self.mode, Mode::SwallowedError) {
            let _ = output.emit_fact(fact(
                "fact-write",
                ArchaeologyFactKind::Mutation,
                "span-write",
            ));
            return Ok(metadata(&input));
        }
        output.emit_edge(ArchaeologyFactEdge {
            edge_id: "edge-controls".to_string(),
            from_fact_id: "fact-check".to_string(),
            to_fact_id: if matches!(self.mode, Mode::DanglingEdge) {
                "fact-missing".to_string()
            } else {
                "fact-write".to_string()
            },
            kind: ArchaeologyFactEdgeKind::Controls,
            trust: if matches!(self.mode, Mode::UntrustedEdge) {
                ArchaeologyTrust::ModelSynthesized
            } else {
                ArchaeologyTrust::Extracted
            },
            evidence_span_ids: vec!["span-check".to_string()],
            unresolved_reason: None,
        })?;
        let mut result = metadata(&input);
        if matches!(self.mode, Mode::InvalidMetadata) {
            result.dialect = Some("unsupported-dialect".to_string());
        }
        if matches!(self.mode, Mode::UncitedDialect) {
            result.dialect_evidence[0].span_ids.clear();
        }
        if matches!(self.mode, Mode::UncoveredRegion) {
            result.coverage_reasons.clear();
        }
        Ok(result)
    }
}

fn metadata(input: &ArchaeologyAdapterInput<'_>) -> ArchaeologyAdapterMetadata {
    ArchaeologyAdapterMetadata {
        dialect: Some("fixture-dialect".to_string()),
        dialect_evidence: vec![ArchaeologyDialectEvidence {
            signal: "marker".to_string(),
            value: "fixture".to_string(),
            span_ids: vec!["span-check".to_string()],
        }],
        lineage: vec![
            ArchaeologyAdapterLineage {
                kind: ArchaeologyLineageKind::Preprocessed,
                source_unit_id: input.unit.identity.source_unit_id.clone(),
                target_source_unit_id: None,
                evidence_span_id: "span-check".to_string(),
                detail: "normalized fixture input".to_string(),
            },
            ArchaeologyAdapterLineage {
                kind: ArchaeologyLineageKind::Include,
                source_unit_id: input.unit.identity.source_unit_id.clone(),
                target_source_unit_id: Some("source-unit:include".to_string()),
                evidence_span_id: "span-write".to_string(),
                detail: "resolved fixture include".to_string(),
            },
        ],
        regions: [
            ArchaeologyAdapterRegionKind::Recovered,
            ArchaeologyAdapterRegionKind::Error,
            ArchaeologyAdapterRegionKind::Unsupported,
        ]
        .into_iter()
        .map(|kind| ArchaeologyAdapterRegion {
            kind,
            span_id: "span-broken".to_string(),
            reason: "fixture unsupported range".to_string(),
        })
        .collect(),
        coverage_reasons: vec!["unsupported_fixture_range".to_string()],
    }
}

#[test]
fn lineage_targets_distinguish_resolved_from_explicitly_unresolved() {
    let mut lineage = ArchaeologyAdapterLineage {
        kind: ArchaeologyLineageKind::Copybook,
        source_unit_id: "source".into(),
        target_source_unit_id: None,
        evidence_span_id: "span".into(),
        detail: "unresolved COPY target".into(),
    };
    assert!(lineage.has_honest_target());
    lineage.detail = "candidate target".into();
    assert!(!lineage.has_honest_target());
    lineage.target_source_unit_id = Some("target".into());
    assert!(lineage.has_honest_target());
    lineage.detail = "unresolved but target populated".into();
    assert!(!lineage.has_honest_target());
    lineage.target_source_unit_id = None;
    for kind in [
        ArchaeologyLineageKind::Include,
        ArchaeologyLineageKind::Macro,
    ] {
        lineage.kind = kind;
        assert!(lineage.has_honest_target());
    }
    lineage.detail = "candidate target".into();
    assert!(!lineage.has_honest_target());
}

fn span(
    input: &ArchaeologyAdapterInput<'_>,
    id: &str,
    start: ArchaeologyPosition,
    end: ArchaeologyPosition,
) -> ArchaeologySourceSpan {
    ArchaeologySourceSpan {
        span_id: id.to_string(),
        source_unit_id: input.unit.identity.source_unit_id.clone(),
        revision_sha: input.unit.identity.revision_sha.clone(),
        start,
        end,
    }
}

fn fact(id: &str, kind: ArchaeologyFactKind, span: &str) -> ArchaeologyFact {
    ArchaeologyFact {
        fact_id: id.to_string(),
        kind,
        label: id.to_string(),
        span_ids: vec![span.to_string()],
        parser_id: "fixture-parser".to_string(),
        trust: ArchaeologyTrust::Extracted,
        confidence: ArchaeologyConfidence::High,
        attributes: Vec::new(),
    }
}

fn position(byte: u64, line: u64, column: u64) -> ArchaeologyPosition {
    ArchaeologyPosition { byte, line, column }
}

fn unit(classification: ArchaeologySourceClassification) -> ArchaeologyInventoryUnit {
    ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: "source-unit:fixture".to_string(),
            repository_id: "repository:fixture".to_string(),
            revision_sha: REVISION.to_string(),
            path_identity: "path:fixture".to_string(),
            relative_path: Some("src/fixture.txt".to_string()),
            content_hash: Some(hex(&Sha256::digest(SOURCE))),
            hash_algorithm: Some("sha256".to_string()),
            change_identity: None,
        },
        classification,
        language: "fixture".to_string(),
        dialect: Some("fixture-dialect".to_string()),
        byte_count: SOURCE.len() as u64,
        line_count: 3,
        include_candidates: Vec::new(),
        coverage_reasons: Vec::new(),
    }
}

#[derive(Default)]
struct Collected {
    events: CapturedEvents,
    emitted: usize,
    begun: bool,
    committed: bool,
    fail_commit: bool,
}

compose_captured_events!(Collected, events);

#[rustfmt::skip]
impl ArchaeologyAdapterEvents for Collected {
    fn emit_span(&mut self, value: ArchaeologySourceSpan) -> Result<(), String> { self.emitted += 1; self.events.emit_span(value) }
    fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> { self.emitted += 1; self.events.emit_fact(value) }
    fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> { self.emitted += 1; self.events.emit_edge(value) }
}

#[rustfmt::skip]
impl ArchaeologyAdapterOutput for Collected {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> { self.begun = true; Ok(()) }
    fn commit_unit(&mut self, _outcome: &ArchaeologyAdapterOutcome) -> Result<(), String> {
        if !self.begun { Err("unit not begun".into()) }
        else if self.fail_commit { Err("fixture commit failed".into()) }
        else { self.committed = true; Ok(()) }
    }
    fn abort_unit(&mut self) -> Result<(), String> {
        self.events.clear(); self.begun = false; self.committed = false; Ok(())
    }
}
