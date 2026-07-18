use super::*;
use crate::commands::business_rule_archaeology::adapter::{
    assert_no_duplicated_source_body, compose_captured_events, run_archaeology_adapter,
    ArchaeologyAdapterLimits, ArchaeologyAdapterOutcome, ArchaeologyAdapterOutput, CapturedEvents,
};
use crate::commands::business_rule_archaeology::contracts::{
    ArchaeologyCoverage, ArchaeologySourceSpan, ArchaeologySourceUnitIdentity,
};
use crate::commands::business_rule_archaeology::deterministic_rules::{
    cluster_evidence_compatible_rules, derive_evidence_packets, render_template_rules,
    ArchaeologyFactOrigin,
};
use crate::commands::business_rule_archaeology::inventory::ArchaeologyInventoryUnit;
use crate::commands::business_rule_archaeology::{
    link_archaeology_facts, ArchaeologyLinkFact, ArchaeologyLinkLimits, ArchaeologyLinkUnit,
};
use crate::commands::structural_graph::types::stable_graph_id;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const REVISION: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const REQUIRED: [&str; 8] = [
    "label",
    "data-definition",
    "branch",
    "call",
    "comparison",
    "arithmetic",
    "memory-io",
    "macro-include",
];

#[test]
fn real_gas_comparisons_keep_ordered_operands_and_literals_through_clustering() {
    let source = b".globl compare_values\ncompare_values:\n  cmpq $0, %rdi\n  cmpq $100, %rdi\n  cmpq $0, %rsi\n  testq %rdi, %rdi\n  ret\n";
    let parsed = run(
        source,
        "semantic-comparisons.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        None,
        Default::default(),
    )
    .unwrap();
    let predicates = parsed
        .facts
        .iter()
        .filter(|fact| fact.kind == ArchaeologyFactKind::Predicate)
        .collect::<Vec<_>>();
    assert_eq!(
        predicates.len(),
        4,
        "predicate labels: {:?}",
        predicates
            .iter()
            .map(|fact| &fact.label)
            .collect::<Vec<_>>()
    );
    let expressions = predicates
        .iter()
        .map(|fact| {
            fact.attributes
                .iter()
                .find(|attribute| attribute.key == "semantic_expr")
                .map(|attribute| attribute.value.as_str())
                .unwrap()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(expressions.len(), 4);
    assert!(expressions
        .iter()
        .all(|value| value.starts_with("v1:sha256:")
            && !value.contains("rdi")
            && !value.contains("100")));

    let cancellation = StructuralGraphCancellation::default();
    let packets = derive_evidence_packets(
        "repository:assembly-cluster",
        REVISION,
        &parsed.facts,
        &parsed.edges,
        &cancellation,
        Default::default(),
    )
    .unwrap();
    let rules = render_template_rules(
        "repository:assembly-cluster",
        "generation:assembly-cluster",
        REVISION,
        &packets,
        &parsed.facts,
        &parsed.edges,
        &ArchaeologyCoverage::default(),
        "parser:manifest",
        "algorithm:v1",
        &cancellation,
        Default::default(),
    )
    .unwrap();
    let origins = parsed
        .facts
        .iter()
        .map(|fact| ArchaeologyFactOrigin {
            fact_id: fact.fact_id.clone(),
            source_unit_id: format!("unit:{}", fact.fact_id),
            path_identity: format!("path:{}", fact.fact_id),
            ranking_path_identity: stable_graph_id(
                "archaeology-ranking-path",
                &format!("src/{}.asm", fact.fact_id),
            ),
            classification: ArchaeologySourceClassification::Source,
        })
        .collect::<Vec<_>>();
    let clustered = cluster_evidence_compatible_rules(
        "repository:assembly-cluster",
        REVISION,
        &rules,
        &parsed.facts,
        &parsed.edges,
        &origins,
        &cancellation,
        Default::default(),
    )
    .unwrap();
    assert_eq!(clustered.len(), rules.len());
    assert!(clustered
        .iter()
        .all(|rule| rule.alias_rule_ids.is_empty() && rule.domain_ids == ["domain:other"]));
}

#[test]
fn real_hlasm_comparisons_hash_opcode_literal_and_operand_order() {
    let source = b"SEMTEST  CSECT\nSTATUS   DC X'00'\nOTHER    DC X'00'\n         CLI STATUS,0\n         CLI STATUS,1\n         CLI OTHER,0\n         CLC STATUS,OTHER\n         CLC OTHER,STATUS\n         CR R2,R3\n         CR R3,R2\n         BR R14\n";
    let parsed = run(
        source,
        "semantic-comparisons.asm",
        "hlasm",
        ArchaeologySourceClassification::Source,
        None,
        Default::default(),
    )
    .unwrap();
    let expressions = parsed
        .facts
        .iter()
        .filter(|fact| fact.kind == ArchaeologyFactKind::Predicate)
        .map(|fact| {
            fact.attributes
                .iter()
                .find(|attribute| attribute.key == "semantic_expr")
                .map(|attribute| attribute.value.as_str())
                .unwrap()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(expressions.len(), 7);
    assert!(expressions
        .iter()
        .all(|value| value.starts_with("v1:sha256:")
            && !value.contains("STATUS")
            && !value.contains("R2")));
}

const HLASM: [&str; 3] = [
    "HONE     CSECT\nDATA1    DC F'1'\n         CLC R1,DATA1\n         BNE REJECT1\n         BAL R14,WORK1\n         A R1,DATA1\n         MVC OUT1,DATA1\n         COPY COMMON1\nREJECT1  MVC OUT1,DATA1\nWORK1    BR R14\n",
    "HTWO     CSECT\nDATA2    DS F\n         CR R2,R3\n         BE REJECT2\n         BAS R14,WORK2\n         S R2,DATA2\n         ST R2,DATA2\n         COPY COMMON2\nREJECT2  MVC OUT2,DATA2\nWORK2    BR R14\n",
    "HTHREE   CSECT\nDATA3    DC H'2'\n         CLI DATA3,0\n         BNH REJECT3\n         BAL R14,WORK3\n         AR R4,R5\n         L R4,DATA3\n         COPY COMMON3\nREJECT3  MVC OUT3,DATA3\nWORK3    BR R14\n",
];

const GAS: [&str; 3] = [
    ".globl route_one\nroute_one:\ndata_one: .quad 1\n  cmpq $0,%rdi\n  jle .Lreject_one\n  call work_one\n  addq $1,%rdi\n  movq %rdi,data_one(%rip)\n  .include \"defs-one.inc\"\nwork_one:\n  ret\n.Lreject_one:\n  ret\n",
    ".global route_two\nroute_two:\ndata_two: .long 2\n  testq %rsi,%rsi\n  jne .Lreject_two\n  callq work_two\n  subq $1,%rsi\n  movq data_two(%rip),%rax\n  .include \"defs-two.inc\"\nwork_two:\n  retq\n.Lreject_two:\n  retq\n",
    ".globl route_three\nroute_three:\ndata_three: .quad 3\n  cmpq $3,%rdx\n  jg .Lreject_three\n  call work_three\n  imulq $2,%rdx\n  movq %rdx,data_three(%rip)\n  .include \"defs-three.inc\"\nwork_three:\n  ret\n.Lreject_three:\n  ret\n",
];

#[test]
fn three_dense_units_per_dialect_cross_every_construct_floor_with_exact_spans() {
    for (dialect, qualified, sources) in [
        ("hlasm", "hlasm", HLASM.as_slice()),
        ("gas-att", "x86-64-gas-att", GAS.as_slice()),
    ] {
        let mut totals = BTreeMap::<String, usize>::new();
        for (index, source) in sources.iter().enumerate() {
            let result = run(
                source.as_bytes(),
                &format!("asm/{dialect}-{index}"),
                dialect,
                ArchaeologySourceClassification::Source,
                None,
                ArchaeologyAdapterLimits::default(),
            )
            .unwrap();
            assert_no_duplicated_source_body(&result.events, source.as_bytes());
            assert_eq!(
                result.outcome().metadata.dialect.as_deref(),
                Some(qualified)
            );
            for construct in REQUIRED {
                let facts = result
                    .facts
                    .iter()
                    .filter(|fact| assembly_construct(fact) == construct)
                    .collect::<Vec<_>>();
                assert!(!facts.is_empty(), "{dialect}/{index} missing {construct}");
                *totals.entry(construct.into()).or_default() += facts.len();
                for fact in facts {
                    assert_exact_fact(&result, source.as_bytes(), fact);
                }
            }
            assert_eq!(result.outcome().metadata.lineage.len(), 1);
            assert_eq!(
                result
                    .edges
                    .iter()
                    .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
                    .count(),
                1
            );
            assert!(result
                .edges
                .iter()
                .any(|edge| edge.kind == ArchaeologyFactEdgeKind::BranchesTo));
            assert!(result
                .edges
                .iter()
                .any(|edge| edge.kind == ArchaeologyFactEdgeKind::Calls));
        }
        for construct in REQUIRED {
            assert!(totals[construct] >= 3, "{dialect}/{construct}");
        }
    }
}

#[test]
fn ambiguity_cross_dialect_and_generated_sources_emit_only_honest_gaps() {
    for (source, dialect, classification) in [
        (
            "START MOV AX,VALUE\n      JNZ ACCEPT\nACCEPT DC F'1'\n",
            "ambiguous",
            ArchaeologySourceClassification::Source,
        ),
        (HLASM[0], "gas-att", ArchaeologySourceClassification::Source),
        (GAS[0], "hlasm", ArchaeologySourceClassification::Source),
        (
            ".globl missing\nother:\n  cmpq $0,%rdi\n",
            "gas-att",
            ArchaeologySourceClassification::Source,
        ),
        (
            "FAKE CSECT\n* MVC R1,R2\n",
            "hlasm",
            ArchaeologySourceClassification::Source,
        ),
        (
            ".globl fake\nfake:\n  # movq $1,%rax\n",
            "gas-att",
            ArchaeologySourceClassification::Source,
        ),
        (
            HLASM[0],
            "hlasm",
            ArchaeologySourceClassification::Generated,
        ),
    ] {
        let result = run(
            source.as_bytes(),
            "asm/gap.asm",
            dialect,
            classification,
            None,
            ArchaeologyAdapterLimits::default(),
        )
        .unwrap();
        assert!(result.facts.is_empty());
        assert_eq!(result.outcome().metadata.dialect, None);
        assert!(result
            .outcome()
            .metadata
            .regions
            .iter()
            .any(|region| region.kind == ArchaeologyAdapterRegionKind::Unsupported));
    }
}

#[test]
#[rustfmt::skip]
fn exported_targets_and_exact_symbol_effects_are_hinted() {
    let hlasm = "PAYASM   CSECT\nDATA     DC F'1'\nOUT      DS F\n         CLC OUT,DATA\n         MVC OUT,DATA\n         A R1,DATA\n         ST R2,OUT\n         L R4,DATA\n         BAL R14,WORK\nWORK     BR R14\n";
    let result = run(hlasm.as_bytes(), "asm/hints.asm", "hlasm", ArchaeologySourceClassification::Source, None, ArchaeologyAdapterLimits::default()).unwrap();
    assert_eq!(attributes(result.facts.iter().find(|fact| fact.label == "PAYASM").unwrap(), "exported"), ["true"]);
    assert!(attributes(result.facts.iter().find(|fact| fact.label == "WORK").unwrap(), "exported").is_empty());
    assert_eq!(attributes(opcode(&result, "CLC"), "reads"), ["OUT", "DATA"]);
    assert_eq!(attributes(opcode(&result, "MVC"), "writes"), ["OUT"]);
    assert_eq!(attributes(opcode(&result, "MVC"), "reads"), ["DATA"]);
    assert_eq!(attributes(opcode(&result, "A"), "reads"), ["DATA"]);
    assert!(attributes(opcode(&result, "A"), "writes").is_empty());
    assert_eq!(attributes(opcode(&result, "ST"), "writes"), ["OUT"]);
    assert_eq!(attributes(opcode(&result, "L"), "reads"), ["DATA"]);
    assert_eq!(attributes(opcode(&result, "BAL"), "target"), ["WORK"]);

    let gas = ".globl route\nroute:\ndata: .quad 1\n  cmpq data(%rip),%rax\n  addq data(%rip),%rax\n  movq %rax,data(%rip)\n  out %rax,PORT\n  call work\n  .include \"defs.inc\"\nwork:\n  ret\nlate:\n  ret\n.globl late\n";
    let result = run(gas.as_bytes(), "asm/hints.s", "gas-att", ArchaeologySourceClassification::Source, None, ArchaeologyAdapterLimits::default()).unwrap();
    assert_eq!(attributes(result.facts.iter().find(|fact| fact.label == "route").unwrap(), "exported"), ["true"]);
    assert_eq!(attributes(result.facts.iter().find(|fact| fact.label == "late").unwrap(), "exported"), ["true"]);
    assert!(attributes(result.facts.iter().find(|fact| fact.label == "work").unwrap(), "exported").is_empty());
    assert_eq!(attributes(opcode(&result, "cmpq"), "reads"), ["data"]);
    assert_eq!(attributes(opcode(&result, "addq"), "reads"), ["data"]);
    assert_eq!(attributes(opcode(&result, "movq"), "writes"), ["data"]);
    assert!(attributes(opcode(&result, "out"), "writes").is_empty());
    assert_eq!(attributes(opcode(&result, "call"), "target"), ["work"]);
    let units = [
        ArchaeologyLinkUnit { source_unit_id: "unit:asm/hints.s", language: "assembly", dialect: Some("gas-att"), relative_path: Some("asm/hints.s"), lineage: &result.outcome().metadata.lineage },
        ArchaeologyLinkUnit { source_unit_id: "unit:asm/defs.inc", language: "assembly", dialect: Some("gas-att"), relative_path: Some("asm/defs.inc"), lineage: &[] },
    ];
    let facts = result.facts.iter().map(|fact| ArchaeologyLinkFact { source_unit_id: units[0].source_unit_id, fact, evidence_spans: &result.spans }).collect::<Vec<_>>();
    let patch = link_archaeology_facts("repository:assembly", REVISION, &units, &facts, &result.edges,
        &StructuralGraphCancellation::default(), ArchaeologyLinkLimits::default()).unwrap();
    assert_eq!(patch.lineage[0].target_source_unit_id.as_deref(), Some(units[1].source_unit_id));
}

#[test]
fn malformed_comments_continuations_and_non_adjacent_branches_fail_closed() {
    let malformed = ".globl bad\nbad:\n  cmpq $0,%rdi # exact comment\n  addq $1,%rdi\n  jle bad\n  movq %rax\n  # call invented\n";
    let result = run(
        malformed.as_bytes(),
        "asm/bad.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert!(result
        .outcome()
        .metadata
        .regions
        .iter()
        .any(|region| region.kind == ArchaeologyAdapterRegionKind::Error));
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        0
    );
    assert!(result.facts.iter().all(|fact| fact.label != "invented"));

    let continued = format!("CONT     CSECT\n         MVC OUT,IN{}X\n", " ".repeat(52));
    let result = run(
        continued.as_bytes(),
        "asm/continued.asm",
        "hlasm",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert!(result
        .outcome()
        .metadata
        .coverage_reasons
        .iter()
        .any(|reason| reason.contains("continuation")));

    for (source, dialect, rejected) in [
        (
            ".globl bad\nbad:\n  movq $1,(%rax)\n  cmpq $0,,%rdi\n",
            "gas-att",
            "cmpq",
        ),
        (
            ".globl bad\nbad:\n  movq $1,(%rax)\n  call !!!\n",
            "gas-att",
            "call",
        ),
        (
            ".globl bad\nbad:\n  movq $1,(%rax)\n  jne *%rax\n",
            "gas-att",
            "jne",
        ),
        (
            "BAD      CSECT\nDATA     DC F'1'\n         MVC OUT,DATA\n         CLC R1,,DATA\n",
            "hlasm",
            "CLC",
        ),
        (
            "BAD      CSECT\nDATA     DC F'1'\n         MVC OUT,DATA\n         BNE ???\n",
            "hlasm",
            "BNE",
        ),
        (
            "BAD      CSECT\nDATA     DC F'1'\n         MVC OUT,DATA\n         BAL R14,,WORK\n",
            "hlasm",
            "BAL",
        ),
    ] {
        let result = run(
            source.as_bytes(),
            "asm/operand-negative.asm",
            dialect,
            ArchaeologySourceClassification::Source,
            None,
            ArchaeologyAdapterLimits::default(),
        )
        .unwrap();
        assert!(result.facts.iter().all(|fact| fact.label != rejected));
        assert!(result
            .outcome()
            .metadata
            .regions
            .iter()
            .any(|region| { region.kind == ArchaeologyAdapterRegionKind::Error }));
    }

    let linked = "linked:\n.globl linked\nData: .quad 1\ndata: .asciz \"key#value,ok:yes\"\n  cmpq $0,%rdi\n  jne missing\n  call Target\n  call target\n  call *%rax\n  jmp *table(,%rax,8)\nTarget:\n  ret\n";
    let result = run(
        linked.as_bytes(),
        "asm/linked.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert_eq!(kind_of(&result, "linked"), ArchaeologyFactKind::EntryPoint);
    assert_eq!(kind_of(&result, "Data"), ArchaeologyFactKind::Declaration);
    assert_eq!(kind_of(&result, "data"), ArchaeologyFactKind::Declaration);
    assert_eq!(kind_of(&result, "Target"), ArchaeologyFactKind::Declaration);
    assert_eq!(kind_of(&result, "target"), ArchaeologyFactKind::Unresolved);
    assert!(result.facts.iter().any(|fact| fact.label == ".asciz"));
    assert!(result.facts.iter().any(|fact| fact.label == "missing"));
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Calls)
            .count(),
        1
    );
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::BranchesTo)
            .count(),
        0
    );

    let gaps = ".globl gaps\ngaps:\n  cmpq $0,%rdi\n  .include \"defs.inc\"\n  jne gaps\n  cmpq $1,%rdi\n  .macro HIDDEN\n  call hidden\n  .endm\n  jne gaps\n";
    let result = run(
        gaps.as_bytes(),
        "asm/adjacency.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        0
    );

    let columns = "MAIN     CSECT\nDATA     DC F'1'\n         CLC R1,DATA\n         COPY SAFECPY\n         BNE MISSING\n         FAKE DC F'2'\n         BR R14\n";
    let result = run(
        columns.as_bytes(),
        "asm/columns.asm",
        "hlasm",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert_eq!(kind_of(&result, "MAIN"), ArchaeologyFactKind::EntryPoint);
    assert_eq!(kind_of(&result, "DATA"), ArchaeologyFactKind::Declaration);
    assert!(result.facts.iter().all(|fact| fact.label != "FAKE"));
    assert_eq!(
        result
            .edges
            .iter()
            .filter(|edge| edge.kind == ArchaeologyFactEdgeKind::Controls)
            .count(),
        0
    );
}

fn kind_of(result: &Collected, label: &str) -> ArchaeologyFactKind {
    result
        .facts
        .iter()
        .find(|fact| fact.label == label)
        .unwrap()
        .kind
        .clone()
}

#[test]
fn include_targets_reject_paths_and_secrets_before_lineage() {
    for target in [
        "/etc/defs.inc",
        r"C:\Users\person\defs.inc",
        r"\\server\share\defs.inc",
        "file:///workspace/defs.inc",
        "../defs.inc",
        "secrets/provider.json",
        "password=secret-value",
    ] {
        let source = format!(".globl safe\nsafe:\n  movq $1,%rax\n  .include \"{target}\"\n");
        let result = run(
            source.as_bytes(),
            "asm/include-policy.s",
            "gas-att",
            ArchaeologySourceClassification::Source,
            None,
            ArchaeologyAdapterLimits::default(),
        )
        .unwrap();
        assert!(result.outcome().metadata.lineage.is_empty(), "{target}");
        assert!(
            result.facts.iter().all(|fact| fact.label != target),
            "{target}"
        );
        assert!(
            result
                .outcome()
                .metadata
                .regions
                .iter()
                .any(|region| { region.kind == ArchaeologyAdapterRegionKind::Error }),
            "{target}"
        );
    }
}

#[test]
fn macro_blocks_unicode_positions_cancellation_and_spi_rollback_are_bounded() {
    let macro_source = ".globl unicode\nunicode:\nvalue: .asciz \"é\"\n  cmpq $0,%rdi\n  jle unicode\n  .macro HIDDEN\n  call invented\n  .endm\n  ret\n";
    let result = run(
        macro_source.as_bytes(),
        "asm/unicode.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        None,
        ArchaeologyAdapterLimits::default(),
    )
    .unwrap();
    assert!(result.facts.iter().all(|fact| fact.label != "invented"));
    assert!(result
        .outcome()
        .metadata
        .lineage
        .iter()
        .any(|lineage| lineage.kind == ArchaeologyLineageKind::Macro));
    let data = result
        .facts
        .iter()
        .find(|fact| assembly_construct(fact) == "data-definition")
        .unwrap();
    assert_exact_fact(&result, macro_source.as_bytes(), data);

    let cancellation = StructuralGraphCancellation::default();
    cancellation.cancel_after_checks(5);
    assert!(run(
        GAS[0].as_bytes(),
        "asm/cancel.s",
        "gas-att",
        ArchaeologySourceClassification::Source,
        Some(&cancellation),
        ArchaeologyAdapterLimits::default()
    )
    .unwrap_err()
    .contains("cancelled"));
    let mut bounds = [
        (ArchaeologyAdapterLimits::default(), "byte contract"),
        (ArchaeologyAdapterLimits::default(), "span count"),
        (ArchaeologyAdapterLimits::default(), "fact count"),
        (ArchaeologyAdapterLimits::default(), "edge count"),
        (ArchaeologyAdapterLimits::default(), "byte bound"),
    ];
    bounds[0].0.max_source_bytes = GAS[0].len() - 1;
    bounds[1].0.max_spans = 1;
    bounds[2].0.max_facts = 1;
    bounds[3].0.max_edges = 0;
    bounds[4].0.max_output_bytes = 1;
    for (limits, expected) in bounds {
        let error = run(
            GAS[0].as_bytes(),
            "asm/bounded.s",
            "gas-att",
            ArchaeologySourceClassification::Source,
            None,
            limits,
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

fn assembly_construct(fact: &ArchaeologyFact) -> &str {
    fact.attributes
        .iter()
        .find(|attribute| attribute.key == "assembly_construct")
        .map(|attribute| attribute.value.as_str())
        .unwrap_or("")
}

fn opcode<'a>(result: &'a Collected, value: &str) -> &'a ArchaeologyFact {
    result
        .facts
        .iter()
        .find(|fact| fact.label == value)
        .unwrap()
}
fn attributes<'a>(fact: &'a ArchaeologyFact, key: &str) -> Vec<&'a str> {
    fact.attributes
        .iter()
        .filter(|item| item.key == key)
        .map(|item| item.value.as_str())
        .collect()
}

fn assert_exact_fact(result: &Collected, source: &[u8], fact: &ArchaeologyFact) {
    let span = result
        .spans
        .iter()
        .find(|span| span.span_id == fact.span_ids[0])
        .unwrap();
    assert!(span.start.byte < span.end.byte);
    let slice = std::str::from_utf8(&source[span.start.byte as usize..span.end.byte as usize])
        .unwrap()
        .to_ascii_lowercase();
    let opcode = fact
        .attributes
        .iter()
        .find(|attribute| attribute.key == "opcode");
    assert!(
        slice.contains(&fact.label.to_ascii_lowercase())
            || opcode
                .is_some_and(|attribute| slice.contains(&attribute.value.to_ascii_lowercase())),
        "fact {} is not source-labeled by {slice:?}",
        fact.label
    );
    assert_eq!(
        (span.start.line, span.start.column),
        position(source, span.start.byte as usize)
    );
    assert_eq!(
        (span.end.line, span.end.column),
        position(source, span.end.byte as usize)
    );
}

fn position(source: &[u8], byte: usize) -> (u64, u64) {
    let prefix = std::str::from_utf8(&source[..byte]).unwrap();
    let line = prefix.bytes().filter(|value| *value == b'\n').count() as u64 + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() as u64 + 1;
    (line, column)
}

fn run(
    source: &[u8],
    path: &str,
    dialect: &str,
    classification: ArchaeologySourceClassification,
    cancellation: Option<&StructuralGraphCancellation>,
    limits: ArchaeologyAdapterLimits,
) -> Result<Collected, String> {
    let unit = ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: format!("unit:{path}"),
            repository_id: "repository:assembly".into(),
            revision_sha: REVISION.into(),
            path_identity: format!("path:{path}"),
            relative_path: Some(path.into()),
            content_hash: Some(format!("{:x}", Sha256::digest(source))),
            hash_algorithm: Some("sha256".into()),
            change_identity: None,
        },
        classification,
        language: "assembly".into(),
        dialect: Some(dialect.into()),
        byte_count: source.len() as u64,
        line_count: source.iter().filter(|byte| **byte == b'\n').count() as u64,
        include_candidates: vec![],
        coverage_reasons: vec![],
    };
    let default_cancellation = StructuralGraphCancellation::default();
    let mut output = Collected::default();
    match run_archaeology_adapter(
        &AssemblyAdapter::default(),
        ArchaeologyAdapterInput {
            unit: &unit,
            source,
        },
        &mut output,
        cancellation.unwrap_or(&default_cancellation),
        limits,
    ) {
        Ok(outcome) => {
            output.outcome = Some(outcome);
            Ok(output)
        }
        Err(error) => {
            assert!(output.spans.is_empty());
            assert!(output.facts.is_empty());
            assert!(output.edges.is_empty());
            Err(error)
        }
    }
}

#[derive(Default, Debug)]
struct Collected {
    events: CapturedEvents,
    outcome: Option<ArchaeologyAdapterOutcome>,
}

impl Collected {
    fn outcome(&self) -> &ArchaeologyAdapterOutcome {
        self.outcome.as_ref().unwrap()
    }
}
compose_captured_events!(Collected, events);

#[rustfmt::skip]
impl ArchaeologyAdapterEvents for Collected {
    fn emit_span(&mut self, value: ArchaeologySourceSpan) -> Result<(), String> { self.events.emit_span(value) }
    fn emit_fact(&mut self, value: ArchaeologyFact) -> Result<(), String> { self.events.emit_fact(value) }
    fn emit_edge(&mut self, value: ArchaeologyFactEdge) -> Result<(), String> { self.events.emit_edge(value) }
}
#[rustfmt::skip]
impl ArchaeologyAdapterOutput for Collected {
    fn begin_unit(&mut self, _: &str) -> Result<(), String> { Ok(()) }
    fn commit_unit(&mut self, _: &ArchaeologyAdapterOutcome) -> Result<(), String> { Ok(()) }
    fn abort_unit(&mut self) -> Result<(), String> { self.events.clear(); Ok(()) }
}
