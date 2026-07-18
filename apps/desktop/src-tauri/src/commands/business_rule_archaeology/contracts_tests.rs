use super::*;

fn span() -> ArchaeologySourceSpan {
    ArchaeologySourceSpan {
        span_id: "span:eligibility".to_string(),
        source_unit_id: "unit:program".to_string(),
        revision_sha: "a".repeat(40),
        start: ArchaeologyPosition {
            byte: 20,
            line: 3,
            column: 5,
        },
        end: ArchaeologyPosition {
            byte: 48,
            line: 3,
            column: 33,
        },
    }
}

fn clause() -> ArchaeologyRuleClause {
    ArchaeologyRuleClause {
        clause_id: "clause:eligible".to_string(),
        text: "A claim is eligible when the covered amount is positive.".to_string(),
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::High,
        supporting_fact_ids: vec!["fact:predicate".to_string()],
        contradicting_fact_ids: Vec::new(),
        evidence_span_ids: vec!["span:eligibility".to_string()],
        caveats: vec!["Source-derived behavior is not legal-policy validation.".to_string()],
    }
}

#[test]
fn revision_identity_is_exact_lowercase_sha1_or_sha256() {
    for valid in ["a".repeat(40), "b".repeat(64)] {
        assert!(validate_revision_sha(&valid).is_ok(), "{valid}");
    }
    for invalid in [
        "a".repeat(39),
        "b".repeat(63),
        "A".repeat(40),
        "B".repeat(64),
        format!("{}g", "a".repeat(39)),
    ] {
        assert!(validate_revision_sha(&invalid).is_err(), "{invalid}");
    }
}

#[test]
fn legacy_empty_payloads_are_explicitly_unavailable() {
    let page: ArchaeologyCatalogPage = serde_json::from_str("{}").expect("legacy page");
    assert_eq!(page.schema_version, ARCHAEOLOGY_SCHEMA_VERSION);
    assert_eq!(page.contract_id, ARCHAEOLOGY_CONTRACT_ID);
    assert!(page.rules.is_empty());
    assert_eq!(page.coverage.state, ArchaeologyCoverageState::Unavailable);
    assert_eq!(page.freshness, ArchaeologyFreshness::default());

    let job: ArchaeologyJobStatus = serde_json::from_str("{}").expect("legacy job");
    assert_eq!(job.stage, ArchaeologyJobStage::Idle);
    assert_eq!(job.state, ArchaeologyJobState::Unavailable);
    assert!(job.owner_id.is_none());
}

#[test]
fn exact_source_spans_reject_ambiguous_or_reversed_coordinates() {
    span().validate().expect("exact span");
    let mut invalid = span();
    invalid.start.line = 0;
    assert!(invalid.validate().unwrap_err().contains("one-based"));
    let mut reversed = span();
    reversed.end.byte = 10;
    assert!(reversed.validate().unwrap_err().contains("precedes"));
    let mut abbreviated = span();
    abbreviated.revision_sha = "abcdef12".to_string();
    assert!(abbreviated
        .validate()
        .unwrap_err()
        .contains("full revision"));
}

#[test]
fn published_rules_require_clause_level_fact_and_span_support() {
    clause().validate().expect("cited clause");
    let mut uncited = clause();
    uncited.evidence_span_ids.clear();
    assert!(uncited.validate().unwrap_err().contains("source spans"));

    let rule = ArchaeologyRulePacket {
        rule_id: "rule:eligibility".to_string(),
        repository_id: "repo:fixture".to_string(),
        generation_id: "generation:one".to_string(),
        revision_sha: "a".repeat(40),
        kind: ArchaeologyRuleKind::Eligibility,
        title: "Claim eligibility".to_string(),
        domain_ids: vec!["domain:claims".to_string()],
        lifecycle: ArchaeologyRuleLifecycle::Candidate,
        trust: ArchaeologyTrust::Deterministic,
        confidence: ArchaeologyConfidence::High,
        clauses: vec![clause()],
        dependency_rule_ids: Vec::new(),
        conflict_rule_ids: Vec::new(),
        alias_rule_ids: Vec::new(),
        coverage: ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Complete,
            parser_coverage: ArchaeologyCoverageState::Complete,
            repository_coverage: ArchaeologyCoverageState::Complete,
            temporal_coverage: ArchaeologyCoverageState::Unavailable,
            ..ArchaeologyCoverage::default()
        },
        parser_identity: "parser:fixture:v1".to_string(),
        algorithm_identity: "rules:v1".to_string(),
        synthesis_identity: None,
    };
    rule.validate().expect("valid rule");
    let encoded = serde_json::to_value(&rule).expect("serialize rule");
    assert_eq!(encoded["trust"], "deterministic");
    assert_eq!(encoded["lifecycle"], "candidate");
    assert_eq!(
        encoded["clauses"][0]["evidence_span_ids"][0],
        span().span_id
    );
}

#[test]
fn strict_fact_and_edge_contracts_reject_unknown_fields() {
    let fact = serde_json::json!({
        "fact_id": "fact:predicate",
        "kind": "predicate",
        "label": "COVERED-AMOUNT > 0",
        "span_ids": ["span:eligibility"],
        "parser_id": "parser:cobol:v1",
        "trust": "extracted",
        "confidence": "high",
        "attributes": [],
        "raw_email": "must-not-cross-contract"
    });
    assert!(serde_json::from_value::<ArchaeologyFact>(fact).is_err());

    let edge = serde_json::json!({
        "edge_id": "edge:controls",
        "from_fact_id": "fact:predicate",
        "to_fact_id": "fact:mutation",
        "kind": "controls",
        "trust": "extracted",
        "evidence_span_ids": ["span:eligibility"],
        "unresolved_reason": null
    });
    let edge: ArchaeologyFactEdge = serde_json::from_value(edge).expect("strict edge");
    assert_eq!(edge.kind, ArchaeologyFactEdgeKind::Controls);
}

#[test]
fn parser_job_and_page_contracts_keep_distinct_dimensions() {
    let capability = ArchaeologyParserCapability {
        parser_id: "parser:cobol:v1".to_string(),
        parser_version: "1.0.0".to_string(),
        language: "cobol".to_string(),
        dialects: vec!["ibm-enterprise".to_string()],
        constructs: vec![
            ArchaeologyFactKind::Predicate,
            ArchaeologyFactKind::Calculation,
        ],
        exact_spans: true,
        preprocessing: true,
        recovery: true,
    };
    assert!(capability.exact_spans && capability.preprocessing && capability.recovery);

    let page = ArchaeologyCatalogPage {
        repository_id: Some("repo:fixture".to_string()),
        generation_id: Some("generation:one".to_string()),
        coverage: ArchaeologyCoverage {
            state: ArchaeologyCoverageState::Partial,
            parser_coverage: ArchaeologyCoverageState::Partial,
            repository_coverage: ArchaeologyCoverageState::Complete,
            temporal_coverage: ArchaeologyCoverageState::Unavailable,
            reasons: vec!["unsupported_macro_region".to_string()],
            ..ArchaeologyCoverage::default()
        },
        freshness: ArchaeologyFreshness {
            stale: true,
            reasons: vec!["head_changed".to_string()],
            ..ArchaeologyFreshness::default()
        },
        page: ArchaeologyPageInfo {
            applied_limit: 100,
            total_rows: 100_000,
            truncated: true,
            next_cursor: Some("opaque:next".to_string()),
        },
        ..ArchaeologyCatalogPage::default()
    };
    assert_ne!(
        page.coverage.parser_coverage,
        page.coverage.repository_coverage
    );
    assert!(page.freshness.stale);
    assert_eq!(page.page.total_rows, 100_000);
}
