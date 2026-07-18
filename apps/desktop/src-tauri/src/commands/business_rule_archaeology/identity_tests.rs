use super::*;

const PREDICATE: ArchaeologyFactKind = ArchaeologyFactKind::Predicate;
const MUTATION: ArchaeologyFactKind = ArchaeologyFactKind::Mutation;
const DECISION: ArchaeologyFactKind = ArchaeologyFactKind::Decision;
const SEMANTIC_A: &str =
    "v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SEMANTIC_B: &str =
    "v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SEMANTIC_C: &str =
    "v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const CONTENT_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const CONTENT_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn span<'a>(
    path_identity: &'a str,
    content_hash: &'a str,
    start_byte: u64,
    end_byte: u64,
) -> ArchaeologyIdentitySpan<'a> {
    ArchaeologyIdentitySpan {
        path_identity,
        content_hash,
        start_byte,
        end_byte,
    }
}

fn fact<'a>(
    kind: &'a ArchaeologyFactKind,
    semantic_expression: &'a str,
    parser_identity: &'a str,
    spans: &'a [ArchaeologyIdentitySpan<'a>],
) -> ArchaeologyIdentityFact<'a> {
    ArchaeologyIdentityFact {
        kind,
        semantic_expression,
        parser_identity,
        spans,
    }
}

fn identities(
    repository_id: &str,
    title: &str,
    clauses: &[&str],
    description_source_identity: &str,
    supporting_facts: &[ArchaeologyIdentityFact<'_>],
    contradicting_facts: &[ArchaeologyIdentityFact<'_>],
) -> ArchaeologyRuleIdentities {
    build_rule_identities(
        &ArchaeologyRuleIdentityInput {
            repository_id,
            kind: &ArchaeologyRuleKind::Validation,
            anchor: &supporting_facts[0],
            supporting_facts,
            contradicting_facts,
            title,
            clauses,
            description_source_identity,
        },
        ArchaeologyIdentityLimits::default(),
    )
    .expect("valid identity fixture")
}

fn assert_persisted_hash(value: &str) {
    let digest = value.strip_prefix("sha256:").expect("sha256 prefix");
    assert_eq!(digest.len(), 64);
    assert!(digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
}

#[test]
fn prose_only_change_preserves_semantics_evidence_contradictions_and_continuity() {
    let spans = [span("archaeology-path:a", CONTENT_A, 10, 20)];
    let supporting = [fact(&PREDICATE, SEMANTIC_A, "parser:v1", &spans)];
    let original = identities(
        "repository:one",
        "Account is eligible",
        &["The account must be active."],
        "template:v1",
        &supporting,
        &[],
    );
    let rewritten = identities(
        "repository:one",
        "Eligible account",
        &["An active account is required."],
        "template:v1",
        &supporting,
        &[],
    );

    for identity in [
        &original.stable_rule_identity,
        &original.evidence_identity,
        &original.contradiction_identity,
        &original.description_identity,
        &original.continuity_identity,
    ] {
        assert_persisted_hash(identity);
    }

    assert_eq!(
        original.stable_rule_identity,
        rewritten.stable_rule_identity
    );
    assert_eq!(original.evidence_identity, rewritten.evidence_identity);
    assert_eq!(
        original.contradiction_identity,
        rewritten.contradiction_identity
    );
    assert_eq!(original.continuity_identity, rewritten.continuity_identity);
    assert_ne!(
        original.description_identity,
        rewritten.description_identity
    );
}

#[test]
fn evidence_parser_contradiction_and_semantic_changes_are_partitioned() {
    let original_span = [span("archaeology-path:a", CONTENT_A, 10, 20)];
    let moved_span = [span("archaeology-path:b", CONTENT_A, 30, 40)];
    let contradiction_span = [span("archaeology-path:c", CONTENT_B, 50, 60)];
    let original = [fact(&PREDICATE, SEMANTIC_A, "parser:v1", &original_span)];
    let moved = [fact(&PREDICATE, SEMANTIC_A, "parser:v1", &moved_span)];
    let reparsed = [fact(&PREDICATE, SEMANTIC_A, "parser:v2", &original_span)];
    let semantic_change = [fact(&PREDICATE, SEMANTIC_B, "parser:v1", &original_span)];
    let contradiction = [fact(
        &DECISION,
        SEMANTIC_C,
        "parser:v1",
        &contradiction_span,
    )];

    let baseline = identities(
        "repository:one",
        "Rule",
        &["Clause"],
        "template:v1",
        &original,
        &[],
    );
    for changed in [&moved[..], &reparsed[..]] {
        let result = identities(
            "repository:one",
            "Rule",
            &["Clause"],
            "template:v1",
            changed,
            &[],
        );
        assert_eq!(baseline.stable_rule_identity, result.stable_rule_identity);
        assert_ne!(baseline.evidence_identity, result.evidence_identity);
        assert_eq!(baseline.continuity_identity, result.continuity_identity);
        assert_eq!(baseline.description_identity, result.description_identity);
        assert_eq!(
            baseline.contradiction_identity,
            result.contradiction_identity
        );
    }

    let contradicted = identities(
        "repository:one",
        "Rule",
        &["Clause"],
        "template:v1",
        &original,
        &contradiction,
    );
    assert_eq!(
        baseline.stable_rule_identity,
        contradicted.stable_rule_identity
    );
    assert_eq!(baseline.evidence_identity, contradicted.evidence_identity);
    assert_eq!(
        baseline.continuity_identity,
        contradicted.continuity_identity
    );
    assert_ne!(
        baseline.contradiction_identity,
        contradicted.contradiction_identity
    );

    let redefined = identities(
        "repository:one",
        "Rule",
        &["Clause"],
        "template:v1",
        &semantic_change,
        &[],
    );
    assert_ne!(
        baseline.stable_rule_identity,
        redefined.stable_rule_identity
    );
    assert_ne!(baseline.evidence_identity, redefined.evidence_identity);
    assert_ne!(baseline.continuity_identity, redefined.continuity_identity);
}

#[test]
fn input_order_does_not_change_any_identity() {
    let spans_a = [
        span("archaeology-path:b", CONTENT_B, 30, 40),
        span("archaeology-path:a", CONTENT_A, 10, 20),
    ];
    let spans_a_reversed = [spans_a[1], spans_a[0]];
    let spans_b = [span("archaeology-path:c", CONTENT_A, 50, 60)];
    let contradiction_spans = [span("archaeology-path:d", CONTENT_B, 70, 80)];
    let supporting = [
        fact(&PREDICATE, SEMANTIC_A, "parser:v1", &spans_a),
        fact(&MUTATION, SEMANTIC_B, "parser:v1", &spans_b),
    ];
    let supporting_reversed = [
        fact(&MUTATION, SEMANTIC_B, "parser:v1", &spans_b),
        fact(&PREDICATE, SEMANTIC_A, "parser:v1", &spans_a_reversed),
    ];
    let contradictions = [
        fact(&DECISION, SEMANTIC_C, "parser:v1", &contradiction_spans),
        fact(&PREDICATE, SEMANTIC_B, "parser:v1", &spans_b),
    ];
    let contradictions_reversed = [contradictions[1], contradictions[0]];

    let left = build_rule_identities(
        &ArchaeologyRuleIdentityInput {
            repository_id: "repository:one",
            kind: &ArchaeologyRuleKind::Validation,
            anchor: &supporting[0],
            supporting_facts: &supporting,
            contradicting_facts: &contradictions,
            title: "  Canonical   title ",
            clauses: &["Second clause", "First\nclause"],
            description_source_identity: "template:v1",
        },
        ArchaeologyIdentityLimits::default(),
    )
    .unwrap();
    let right = build_rule_identities(
        &ArchaeologyRuleIdentityInput {
            repository_id: "repository:one",
            kind: &ArchaeologyRuleKind::Validation,
            anchor: &supporting_reversed[1],
            supporting_facts: &supporting_reversed,
            contradicting_facts: &contradictions_reversed,
            title: "Canonical title",
            clauses: &["First clause", "Second clause"],
            description_source_identity: "template:v1",
        },
        ArchaeologyIdentityLimits::default(),
    )
    .unwrap();

    assert_eq!(left, right);
}

#[test]
fn every_identity_is_repository_scoped() {
    let spans = [span("archaeology-path:a", CONTENT_A, 10, 20)];
    let supporting = [fact(&PREDICATE, SEMANTIC_A, "parser:v1", &spans)];
    let origin = identities(
        "repository:origin",
        "Rule",
        &["Clause"],
        "template:v1",
        &supporting,
        &[],
    );
    let fork = identities(
        "repository:fork",
        "Rule",
        &["Clause"],
        "template:v1",
        &supporting,
        &[],
    );

    assert_ne!(origin.stable_rule_identity, fork.stable_rule_identity);
    assert_ne!(origin.evidence_identity, fork.evidence_identity);
    assert_ne!(origin.contradiction_identity, fork.contradiction_identity);
    assert_ne!(origin.description_identity, fork.description_identity);
    assert_ne!(origin.continuity_identity, fork.continuity_identity);
}

#[test]
fn contradiction_empty_set_and_provenance_are_explicit_and_versioned() {
    let limits = ArchaeologyIdentityLimits::default();
    let empty = contradiction_identity("repository:one", &[], limits).unwrap();
    assert!(empty.starts_with("sha256:"));
    assert_eq!(
        empty,
        contradiction_identity("repository:one", &[], limits).unwrap()
    );
    let provenance = identity_provenance();
    assert_eq!(
        provenance,
        ArchaeologyIdentityProvenance {
            schema: "codevetter.archaeology-rule-identities.v1".into(),
            hash_algorithm: "sha256".into(),
            stable_rule_version: "archaeology-stable-rule:v1".into(),
            evidence_version: "archaeology-rule-evidence:v1".into(),
            contradiction_version: "archaeology-rule-contradictions:v1".into(),
            description_version: "archaeology-rule-description:v1".into(),
            continuity_version: "archaeology-rule-continuity:v1".into(),
            parser_compatibility_version: "archaeology-rule-parser-compatibility:v1".into(),
        }
    );
    let json = serde_json::to_string(&provenance).unwrap();
    assert!(json.len() < 512);
    assert_eq!(
        serde_json::from_str::<ArchaeologyIdentityProvenance>(&json).unwrap(),
        provenance
    );
    assert!(
        serde_json::from_str::<ArchaeologyIdentityProvenance>(&json.replace(
            "\"continuity_version\":",
            "\"unknown\":true,\"continuity_version\":"
        ))
        .is_err()
    );
}

#[test]
fn malformed_and_over_bound_inputs_fail_closed() {
    let spans = [span("archaeology-path:a", CONTENT_A, 10, 20)];
    let valid = fact(&PREDICATE, SEMANTIC_A, "parser:v1", &spans);
    let uppercase_semantic =
        "v1:sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let malformed = fact(&PREDICATE, uppercase_semantic, "parser:v1", &spans);
    let limits = ArchaeologyIdentityLimits::default();

    assert!(stable_rule_identity(
        "repository:one",
        &ArchaeologyRuleKind::Validation,
        &malformed,
        &[malformed],
        limits,
    )
    .is_err());
    assert!(stable_rule_identity(
        "repository/unsafe",
        &ArchaeologyRuleKind::Validation,
        &valid,
        &[valid],
        limits,
    )
    .is_err());
    assert!(stable_rule_identity(
        "repository:one",
        &ArchaeologyRuleKind::Validation,
        &valid,
        &[],
        limits,
    )
    .is_err());

    let invalid_hash_spans = [span("archaeology-path:a", "abc", 10, 20)];
    let invalid_hash = fact(&PREDICATE, SEMANTIC_A, "parser:v1", &invalid_hash_spans);
    assert!(evidence_identity("repository:one", &[invalid_hash], limits).is_err());
    let reversed_spans = [span("archaeology-path:a", CONTENT_A, 20, 10)];
    let reversed = fact(&PREDICATE, SEMANTIC_A, "parser:v1", &reversed_spans);
    assert!(evidence_identity("repository:one", &[reversed], limits).is_err());
    let duplicate_spans = [spans[0], spans[0]];
    let duplicate = fact(&PREDICATE, SEMANTIC_A, "parser:v1", &duplicate_spans);
    assert!(evidence_identity("repository:one", &[duplicate], limits).is_err());

    assert!(description_identity("repository:one", "Rule", &[], "template:v1", limits).is_err());
    assert!(description_identity(
        "repository:one",
        "Rule",
        &["Clause"],
        "template:\0v1",
        limits,
    )
    .is_err());
    assert!(continuity_identity("repository:one", "rule:mutable-id", limits).is_err());

    let one_fact_only = ArchaeologyIdentityLimits {
        max_facts: 0,
        ..limits
    };
    assert!(evidence_identity("repository:one", &[valid], one_fact_only).is_err());
    let no_bytes = ArchaeologyIdentityLimits {
        max_identity_bytes: 0,
        ..limits
    };
    assert!(contradiction_identity("repository:one", &[], no_bytes).is_err());
}
