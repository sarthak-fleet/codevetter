use super::*;
use crate::commands::business_rule_archaeology::adapter::{
    assert_no_duplicated_source_body, compose_captured_events, run_archaeology_adapter,
    ArchaeologyAdapterLimits, ArchaeologyAdapterOutcome, ArchaeologyAdapterOutput, CapturedEvents,
};
use crate::commands::business_rule_archaeology::contracts::{
    ArchaeologyFactEdge, ArchaeologySourceClassification, ArchaeologySourceUnitIdentity,
};
use crate::commands::business_rule_archaeology::inventory::ArchaeologyInventoryUnit;
use sha2::{Digest, Sha256};

const SOURCE: &[u8] = include_bytes!("fixtures/sources/modern/payment.ts");
const REVISION: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn labeled_typescript_fixture_is_exact_deterministic_and_honest_about_gaps() {
    let first = run(false).expect("first modern reference extraction");
    let second = run(false).expect("second modern reference extraction");
    assert_eq!(first.spans, second.spans);
    assert_eq!(first.facts, second.facts);
    assert_eq!(first.edges, second.edges);
    assert_eq!(first.outcome(), second.outcome());
    assert_no_duplicated_source_body(&first.events, SOURCE);

    let entry = first
        .facts
        .iter()
        .find(|fact| fact.kind == ArchaeologyFactKind::EntryPoint)
        .expect("entry-point fact");
    assert_eq!(entry.label, "approvePayment");
    let entry_span = first
        .spans
        .iter()
        .find(|span| span.span_id == entry.span_ids[0])
        .expect("entry-point span");
    assert_eq!((entry_span.start.byte, entry_span.end.byte), (16, 30));
    assert_eq!((entry_span.start.line, entry_span.start.column), (1, 17));
    assert_eq!((entry_span.end.line, entry_span.end.column), (1, 31));
    assert_eq!(&SOURCE[16..30], b"approvePayment");

    assert!(first
        .facts
        .iter()
        .any(|fact| fact.kind == ArchaeologyFactKind::ControlFlow));
    assert_eq!(first.outcome().metadata.regions.len(), 2);
    assert!(first
        .outcome()
        .metadata
        .regions
        .iter()
        .all(|region| region.kind == ArchaeologyAdapterRegionKind::Unsupported));
    assert!(first
        .outcome()
        .metadata
        .coverage_reasons
        .iter()
        .any(|reason| { reason.contains("predicate semantics") }));
    assert!(first
        .outcome()
        .metadata
        .coverage_reasons
        .iter()
        .any(|reason| { reason.contains("mutation and calculation semantics") }));
    assert!(first.outcome().metadata.lineage.is_empty());
    assert_eq!(first.outcome().edge_count, 0);
}

#[test]
fn cancellation_aborts_the_transaction_without_partial_publication() {
    let error = run(true).expect_err("adapter cancellation");
    assert!(error.contains("cancelled"), "{error}");
}

#[test]
fn declaration_identifier_anchors_come_from_tree_sitter_across_languages_and_unicode() {
    for (path, language, source, label, bytes, columns) in [
        (
            "tiny.ts",
            SupportedLanguage::TypeScript,
            "function f(){}",
            "f",
            (9, 10),
            (10, 11),
        ),
        (
            "tiny.py",
            SupportedLanguage::Python,
            "def d():\n    pass\n",
            "d",
            (4, 5),
            (5, 6),
        ),
        (
            "unicode.ts",
            SupportedLanguage::TypeScript,
            "function résumé(){}",
            "résumé",
            (9, 17),
            (10, 16),
        ),
    ] {
        let result = run_source(
            source.as_bytes(),
            path,
            language,
            StructuralGraphCancellation::default(),
            false,
        )
        .unwrap_or_else(|error| panic!("{path}: {error}"));
        let fact = result
            .facts
            .iter()
            .find(|fact| fact.kind == ArchaeologyFactKind::EntryPoint && fact.label == label)
            .unwrap_or_else(|| panic!("{path}: missing {label}"));
        let span = result
            .spans
            .iter()
            .find(|span| span.span_id == fact.span_ids[0])
            .expect("entry span");
        assert_eq!((span.start.byte, span.end.byte), bytes, "{path}");
        assert_eq!((span.start.column, span.end.column), columns, "{path}");
        assert_eq!(
            &source.as_bytes()[bytes.0 as usize..bytes.1 as usize],
            label.as_bytes(),
            "{path}"
        );
    }
}

#[test]
fn syntax_recovery_fails_before_any_high_confidence_fact_is_published() {
    let error = run_source(
        b"export function broken( {",
        "broken.ts",
        SupportedLanguage::TypeScript,
        StructuralGraphCancellation::default(),
        false,
    )
    .expect_err("syntax recovery must fail closed");
    assert!(error.contains("syntax recovery"), "{error}");
}

#[test]
fn extraction_honors_pre_and_mid_parse_cancellation_without_timing() {
    let pre_cancelled = StructuralGraphCancellation::default();
    pre_cancelled.cancel();
    let error = run_source(
        SOURCE,
        "modern/payment.ts",
        SupportedLanguage::TypeScript,
        pre_cancelled,
        false,
    )
    .expect_err("pre-cancelled extraction");
    assert!(error.contains("cancelled"), "{error}");

    let mid_cancelled = StructuralGraphCancellation::default();
    mid_cancelled.cancel_after_checks(4);
    let error = run_source(
        SOURCE,
        "modern/payment.ts",
        SupportedLanguage::TypeScript,
        mid_cancelled.clone(),
        false,
    )
    .expect_err("mid-parse cancellation");
    assert!(error.contains("cancelled"), "{error}");
    assert!(mid_cancelled.check_count() >= 4);
}

fn run(cancel_after_first_span: bool) -> Result<Collected, String> {
    let cancellation = StructuralGraphCancellation::default();
    run_source(
        SOURCE,
        "modern/payment.ts",
        SupportedLanguage::TypeScript,
        cancellation,
        cancel_after_first_span,
    )
}

fn run_source(
    source: &[u8],
    path: &str,
    language: SupportedLanguage,
    cancellation: StructuralGraphCancellation,
    cancel_after_first_span: bool,
) -> Result<Collected, String> {
    let adapter = ModernLanguageAdapter::new(language);
    let unit = unit_for(source, path, language.name());
    let mut output = Collected::new(cancel_after_first_span.then_some(cancellation.clone()));
    let outcome = run_archaeology_adapter(
        &adapter,
        ArchaeologyAdapterInput {
            unit: &unit,
            source,
        },
        &mut output,
        &cancellation,
        ArchaeologyAdapterLimits::default(),
    );
    match outcome {
        Ok(outcome) => {
            output.outcome = Some(outcome);
            Ok(output)
        }
        Err(error) => {
            assert!(output.spans.is_empty());
            assert!(output.facts.is_empty());
            assert!(!output.committed);
            Err(error)
        }
    }
}

fn unit_for(source: &[u8], path: &str, language: &str) -> ArchaeologyInventoryUnit {
    ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: "unit:modern".to_string(),
            repository_id: "repository:fixture".to_string(),
            revision_sha: REVISION.to_string(),
            path_identity: "path:modern".to_string(),
            relative_path: Some(path.to_string()),
            content_hash: Some(format!("{:x}", Sha256::digest(source))),
            hash_algorithm: Some("sha256".to_string()),
            change_identity: None,
        },
        classification: ArchaeologySourceClassification::Source,
        language: language.to_string(),
        dialect: Some(language.to_string()),
        byte_count: source.len() as u64,
        line_count: source.iter().filter(|byte| **byte == b'\n').count() as u64,
        include_candidates: Vec::new(),
        coverage_reasons: Vec::new(),
    }
}

#[derive(Debug)]
struct Collected {
    events: CapturedEvents,
    outcome: Option<ArchaeologyAdapterOutcome>,
    cancellation: Option<StructuralGraphCancellation>,
    committed: bool,
}

#[rustfmt::skip]
impl Collected {
    fn new(cancellation: Option<StructuralGraphCancellation>) -> Self {
        Self { events: CapturedEvents::default(), outcome: None, cancellation, committed: false }
    }
    fn outcome(&self) -> &ArchaeologyAdapterOutcome { self.outcome.as_ref().expect("committed adapter outcome") }
}

compose_captured_events!(Collected, events);

#[rustfmt::skip]
impl ArchaeologyAdapterEvents for Collected {
    fn emit_span(&mut self, span: ArchaeologySourceSpan) -> Result<(), String> {
        self.events.emit_span(span)?;
        if self.spans.len() == 1 { if let Some(cancellation) = &self.cancellation { cancellation.cancel(); } }
        Ok(())
    }
    fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> { self.events.emit_fact(value) }
    fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> { self.events.emit_edge(value) }
}

#[rustfmt::skip]
impl ArchaeologyAdapterOutput for Collected {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> { Ok(()) }
    fn commit_unit(&mut self, outcome: &ArchaeologyAdapterOutcome) -> Result<(), String> {
        self.outcome = Some(outcome.clone()); self.committed = true; Ok(())
    }
    fn abort_unit(&mut self) -> Result<(), String> {
        self.events.clear(); self.outcome = None; self.committed = false; Ok(())
    }
}
