use super::adapter::{
    semantic_expression, ArchaeologyAdapterEvents, ArchaeologyAdapterInput,
    ArchaeologyAdapterLineage, ArchaeologyAdapterMetadata, ArchaeologyAdapterRegion,
    ArchaeologyAdapterRegionKind, ArchaeologyDialectEvidence, ArchaeologyLanguageAdapter,
    ArchaeologyLineageKind, SourcePositionIndex,
};
use super::contracts::{
    ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyFact, ArchaeologyFactEdge,
    ArchaeologyFactEdgeKind, ArchaeologyFactKind, ArchaeologyParserCapability,
    ArchaeologySourceClassification, ArchaeologyTrust,
};
use super::legacy::{
    archaeology_id, check_cancelled, checked_span, lines, tokens, LegacyFormat, LegacyLine,
};
use crate::commands::secret_policy::{contains_sensitive_path, looks_like_secret};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use std::collections::{BTreeMap, BTreeSet};

const PARSER_ID: &str = "codevetter-assembly-fallback";
const PARSER_VERSION: &str = "2";
const MAX_LOCAL_REFERENCES: usize = 4_096;

pub struct AssemblyAdapter {
    capability: ArchaeologyParserCapability,
}

#[rustfmt::skip]
impl Default for AssemblyAdapter {
    fn default() -> Self {
        Self { capability: ArchaeologyParserCapability {
                parser_id: PARSER_ID.into(), parser_version: PARSER_VERSION.into(), language: "assembly".into(),
                dialects: ["hlasm", "x86-64-gas-att"].map(str::to_string).to_vec(),
                constructs: vec![
                    ArchaeologyFactKind::Declaration, ArchaeologyFactKind::DataField, ArchaeologyFactKind::Predicate,
                    ArchaeologyFactKind::Calculation, ArchaeologyFactKind::Mutation, ArchaeologyFactKind::Call,
                    ArchaeologyFactKind::InputOutput, ArchaeologyFactKind::ControlFlow,
                    ArchaeologyFactKind::EntryPoint, ArchaeologyFactKind::Include, ArchaeologyFactKind::Unresolved,
                ],
                exact_spans: true, preprocessing: false, recovery: true,
        }}
    }
}

#[rustfmt::skip]
impl ArchaeologyLanguageAdapter for AssemblyAdapter {
    fn capability(&self) -> &ArchaeologyParserCapability { &self.capability }
    fn parse(&self, input: ArchaeologyAdapterInput<'_>, output: &mut dyn ArchaeologyAdapterEvents,
        positions: &SourcePositionIndex, cancellation: &StructuralGraphCancellation) -> Result<ArchaeologyAdapterMetadata, String> {
        check_cancelled(cancellation)?;
        let source = std::str::from_utf8(input.source)
            .map_err(|_| "Assembly archaeology adapter requires UTF-8 source".to_string())?;
        let mut extraction = Extraction::new(&input, source, output, positions, cancellation);
        let dialect = match input.unit.dialect.as_deref() {
            Some("hlasm") => Some(Dialect::Hlasm),
            Some("gas-att") => Some(Dialect::Gas),
            _ => None,
        };
        let gate = match dialect { Some(dialect) => gate(&input, source, dialect, cancellation)?, None => None };
        let Some(gate) = gate else {
            extraction
                .unsupported_unit("assembly dialect lacks non-conflicting positive evidence")?;
            return Ok(extraction.metadata(None, None));
        };
        for entry in &gate.entries { extraction.globals.insert(symbol_key(entry, input.unit.dialect.as_deref())); }
        let evidence = extraction.span(gate.evidence)?;
        extraction.parse(gate.dialect)?;
        extraction.resolve_targets()?;
        Ok(extraction.metadata(Some(gate.qualified), Some(evidence)))
    }
}

#[rustfmt::skip]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect { Hlasm, Gas }

#[rustfmt::skip]
struct DialectGate { dialect: Dialect, qualified: &'static str, evidence: (usize, usize), entries: Vec<String> }

#[rustfmt::skip]
fn gate(input: &ArchaeologyAdapterInput<'_>, source: &str, dialect: Dialect,
    cancellation: &StructuralGraphCancellation) -> Result<Option<DialectGate>, String> {
    if input.unit.classification != ArchaeologySourceClassification::Source { return Ok(None); }
    let (mut section, mut globals, mut labels) = (None, BTreeSet::new(), BTreeSet::new());
    let (mut hlasm_signal, mut percent, mut addressing, mut conflict) = (false, false, false, false);
    for line in lines(source, LegacyFormat::Free) {
        check_cancelled(cancellation)?;
        let Some(range) = code_range(line, dialect) else { continue };
        let code = &source[range.0..range.1];
        let mut words = code.split_whitespace();
        let (first, second, third) = (words.next(), words.next(), words.next());
        hlasm_signal |= code.split_whitespace().any(|word| matches!(word.to_ascii_uppercase().as_str(),
            "USING" | "MVC" | "CLC" | "R14" | "R15"));
        if range.0 == line.start && second.is_some_and(|word|
            matches!(word.to_ascii_uppercase().as_str(), "CSECT" | "DSECT")) { section.get_or_insert(range); }
        if matches!(first, Some(".globl" | ".global" | ".GLOBL" | ".GLOBAL"))
            && third.is_none()
            && second.is_some_and(valid_symbol) { globals.insert((second.expect("checked").to_string(), range)); }
        if let Some((label, _)) = leading_label(code) {
            if labels.len() == MAX_LOCAL_REFERENCES { return Err("assembly gate label bound exceeded".into()); }
            labels.insert(label);
        }
        percent |= code.contains('%'); addressing |= code.contains('$') || code.contains("(%");
        let upper = code.to_ascii_uppercase();
        conflict |= match dialect {
            Dialect::Hlasm => code.contains(['%', '['])
                || matches!(first, Some(".globl" | ".global" | ".GLOBL" | ".GLOBAL")),
            Dialect::Gas => upper.contains(" CSECT") || upper.contains(" DSECT") || upper.contains("[RAX]"),
        };
    }
    let gas_evidence = globals.iter().find(|(target, _)| labels.contains(target.as_str()) && percent && addressing && !conflict).map(|(_, evidence)| *evidence);
    let gate = match dialect {
        Dialect::Hlasm => section.filter(|_| hlasm_signal && !conflict).map(|evidence|
            DialectGate { dialect, qualified: "hlasm", evidence, entries: vec![] }),
        Dialect::Gas => gas_evidence
            .map(|evidence| DialectGate { dialect, qualified: "x86-64-gas-att", evidence,
                entries: globals.into_iter().map(|(target, _)| target).collect() }),
    };
    Ok(gate)
}

#[rustfmt::skip]
#[derive(Clone)]
struct FactRef { id: String, span_id: String, range: (usize, usize) }

#[rustfmt::skip]
struct PendingTarget { from: FactRef, target: String, kind: ArchaeologyFactEdgeKind }

#[rustfmt::skip]
struct Extraction<'a, 'b> {
    input: &'a ArchaeologyAdapterInput<'b>, source: &'a str,
    output: &'a mut dyn ArchaeologyAdapterEvents, positions: &'a SourcePositionIndex,
    cancellation: &'a StructuralGraphCancellation, spans: BTreeSet<String>,
    labels: BTreeMap<String, FactRef>, globals: BTreeSet<String>, targets: Vec<PendingTarget>,
    compare: Option<FactRef>, lineage: Vec<ArchaeologyAdapterLineage>,
    regions: Vec<ArchaeologyAdapterRegion>, reasons: BTreeSet<String>, in_macro: Option<(usize, String)>,
}

#[rustfmt::skip]
impl<'a, 'b> Extraction<'a, 'b> {
    fn new(input: &'a ArchaeologyAdapterInput<'b>, source: &'a str,
        output: &'a mut dyn ArchaeologyAdapterEvents, positions: &'a SourcePositionIndex,
        cancellation: &'a StructuralGraphCancellation) -> Self {
        Self { input, source, output, positions, cancellation, spans: BTreeSet::new(),
            labels: BTreeMap::new(), globals: BTreeSet::new(), targets: vec![], compare: None, lineage: vec![],
            regions: vec![], reasons: BTreeSet::new(), in_macro: None }
    }

    fn parse(&mut self, dialect: Dialect) -> Result<(), String> {
        for line in lines(self.source, LegacyFormat::Free) {
            check_cancelled(self.cancellation)?;
            if line.text.is_empty() {
                continue;
            }
            let Some(range) = code_range(line, dialect) else { continue };
            if tokens(self.source, line).is_err() {
                self.region(
                    line.range(),
                    ArchaeologyAdapterRegionKind::Unsupported,
                    "assembly line exceeds lexical bounds or has an unterminated literal",
                )?;
                self.compare = None;
                continue;
            }
            if dialect == Dialect::Hlasm
                && line.text.len() > 71
                && line
                    .text
                    .as_bytes()
                    .get(71)
                    .is_some_and(|byte| *byte != b' ')
            {
                self.region(
                    line.range(),
                    ArchaeologyAdapterRegionKind::Unsupported,
                    "HLASM continuation requires preprocessing",
                )?;
                self.compare = None;
                continue;
            }
            if self.in_macro.is_some() {
                let code = &self.source[range.0..range.1];
                let ended = match dialect { Dialect::Hlasm => code.split_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("MEND")), Dialect::Gas => code
                    .split_whitespace().next().is_some_and(|word| word.eq_ignore_ascii_case(".endm")) };
                if ended {
                    let (start, name) = self.in_macro.take().expect("checked macro start");
                    self.include((start, range.1), &name, ArchaeologyLineageKind::Macro, "macro-include")?;
                }
                continue;
            }
            match dialect {
                Dialect::Hlasm => self.hlasm(range)?,
                Dialect::Gas => self.gas(range)?,
            }
        }
        if let Some((start, name)) = self.in_macro.take() {
            self.region(
                (start, self.source.len()),
                ArchaeologyAdapterRegionKind::Error,
                &format!("unterminated assembly macro {name}"),
            )?;
        }
        Ok(())
    }

    fn hlasm(&mut self, range: (usize, usize)) -> Result<(), String> {
        let text = &self.source[range.0..range.1];
        let words = text.split_whitespace().collect::<Vec<_>>();
        let known = |word: &str| {
            hlasm_kind(word).is_some()
                || matches!(word, "CSECT" | "DSECT" | "COPY" | "MACRO" | "MEND")
        };
        let column_one = range.0 == 0 || self.source.as_bytes().get(range.0 - 1) == Some(&b'\n');
        let (label, opcode_index) = if words
            .first()
            .is_some_and(|word| known(&word.to_ascii_uppercase()))
        {
            (None, 0)
        } else if column_one {
            (words.first().copied(), 1)
        } else {
            (None, 0)
        };
        let Some(opcode) = words
            .get(opcode_index)
            .map(|word| word.to_ascii_uppercase())
        else {
            return self.region(
                range,
                ArchaeologyAdapterRegionKind::Error,
                "HLASM label has no statement",
            );
        };
        let operand_text = words.get(opcode_index + 1..).unwrap_or_default().join(" ");
        if matches!(opcode.as_str(), "CSECT" | "DSECT") {
            let Some(label) = label else {
                return self.malformed(range, "HLASM section requires a label");
            };
            return self.label(label, range, "label", "section", opcode == "CSECT");
        }
        if opcode == "COPY" {
            return self.include(
                range,
                operand_text.trim(),
                ArchaeologyLineageKind::Include,
                "macro-include",
            );
        }
        if opcode == "MACRO" {
            self.in_macro = Some((range.0, label.unwrap_or("anonymous").to_string()));
            self.compare = None;
            return Ok(());
        }
        if let Some(label) = label {
            if matches!(opcode.as_str(), "DC" | "DS") {
                self.label(label, range, "label", "data-definition", false)?;
            } else {
                self.label(label, (range.0, range.0 + label.len()), "label", "code", false)?;
            }
        }
        let Some((kind, construct, operands)) = hlasm_kind(&opcode) else {
            self.compare = None;
            return self.region(
                range,
                ArchaeologyAdapterRegionKind::Unsupported,
                "unsupported or unexpanded HLASM opcode",
            );
        };
        self.instruction(
            range,
            &opcode,
            &operand_text,
            kind,
            construct,
            operands,
            Dialect::Hlasm,
        )
    }

    fn gas(&mut self, range: (usize, usize)) -> Result<(), String> {
        let text = &self.source[range.0..range.1];
        if let Some((label, rest)) = leading_label(text) {
            let label_range = (range.0, range.0 + label.len() + 1);
            let entry = self.globals.contains(&symbol_key(label, self.input.unit.dialect.as_deref()));
            self.label(label, label_range, "label", "code", entry)?;
            if rest.trim().is_empty() {
                self.compare = None;
                return Ok(());
            }
            let offset = text.find(rest.trim()).expect("substring");
            return self.gas((range.0 + offset, range.1));
        }
        if text.split_whitespace().next().is_some_and(|word| word.contains(':')) {
            return self.malformed(range, "GAS label is invalid");
        }
        let mut parts = text.splitn(2, char::is_whitespace);
        let opcode = parts.next().unwrap_or_default().to_ascii_lowercase();
        let operands = parts.next().unwrap_or_default().trim();
        if matches!(opcode.as_str(), ".globl" | ".global") {
            self.compare = None;
            let fields = operand_fields(operands, Dialect::Gas).filter(|fields| fields.len() == 1 && valid_symbol(fields[0]));
            let Some(fields) = fields else { return self.malformed(range, "GAS global directive is invalid") };
            self.globals.insert(symbol_key(fields[0], self.input.unit.dialect.as_deref()));
            return Ok(());
        }
        if matches!(opcode.as_str(), ".text" | ".data") {
            self.compare = None;
            return if operands.is_empty() { Ok(()) } else { self.malformed(range, "GAS section directive has operands") };
        }
        if opcode == ".include" {
            return self.include(
                range,
                operands.trim_matches(['\'', '"']),
                ArchaeologyLineageKind::Include,
                "macro-include",
            );
        }
        if opcode == ".macro" {
            self.in_macro = Some((
                range.0,
                operands
                    .split_whitespace()
                    .next()
                    .unwrap_or("anonymous")
                    .to_string(),
            ));
            self.compare = None;
            return Ok(());
        }
        if matches!(opcode.as_str(), ".byte" | ".word" | ".long" | ".quad" | ".asciz" | ".zero") {
            let count = if matches!(opcode.as_str(), ".asciz" | ".zero") { 1 } else { usize::MAX };
            return self.instruction(
                range,
                &opcode,
                operands,
                ArchaeologyFactKind::DataField,
                "data-definition",
                count,
                Dialect::Gas,
            );
        }
        let Some((kind, construct, count)) = gas_kind(&opcode) else {
            self.compare = None;
            return self.region(
                range,
                ArchaeologyAdapterRegionKind::Unsupported,
                "unsupported or unexpanded x86/GAS opcode",
            );
        };
        self.instruction(
            range,
            &opcode,
            operands,
            kind,
            construct,
            count,
            Dialect::Gas,
        )
    }

    fn instruction(
        &mut self,
        range: (usize, usize),
        opcode: &str,
        operands: &str,
        kind: ArchaeologyFactKind,
        construct: &'static str,
        expected_operands: usize,
        dialect: Dialect,
    ) -> Result<(), String> {
        let values = operand_fields(operands, dialect);
        let valid_count = values.as_ref().is_some_and(|values| {
            (expected_operands == usize::MAX && !values.is_empty()) || values.len() == expected_operands
        });
        if !valid_count {
            self.compare = None;
            return self.malformed(range, "assembly instruction has invalid operand shape");
        }
        let values = values.expect("validated operands");
        let qualified_effect = matches!(opcode, "in" | "out" | "syscall") || values.iter().any(|value|
            value.contains('(') || (!value.starts_with(['%', '$']) && valid_symbol(value)));
        if construct == "memory-io" && dialect == Dialect::Gas && !qualified_effect {
            self.compare = None;
            return self.malformed(range, "x86 memory/I/O instruction lacks a qualified effect");
        }
        let direct = if matches!(construct, "branch" | "call") {
            match direct_target(opcode, construct, dialect, &values) {
                Ok(target) => target,
                Err(()) => { self.compare = None; return self.malformed(range, "assembly target is invalid"); }
            }
        } else { None };
        let fact = self.fact(kind, opcode, range, construct,
            relationship_hints(opcode, construct, dialect, &values, direct))?;
        if construct == "comparison" {
            self.compare = Some(fact);
            return Ok(());
        }
        if construct == "branch" {
            let conditional = match dialect { Dialect::Hlasm => !matches!(opcode, "B" | "BR" | "BCR"),
                Dialect::Gas => opcode.starts_with('j') && opcode != "jmp" };
            if conditional {
                if let Some(compare) = self.compare.take() {
                    self.edge(&compare, &fact, ArchaeologyFactEdgeKind::Controls, None)?;
                }
            } else {
                self.compare = None;
            }
            if let Some(target) = direct {
                self.pending(fact, target, ArchaeologyFactEdgeKind::BranchesTo)?;
            }
            return Ok(());
        }
        self.compare = None;
        if construct == "call" {
            if let Some(target) = direct {
                self.pending(fact, target, ArchaeologyFactEdgeKind::Calls)?;
            }
        }
        Ok(())
    }

    fn label(
        &mut self,
        label: &str,
        range: (usize, usize),
        construct: &'static str,
        role: &str,
        entry: bool,
    ) -> Result<(), String> {
        let normalized = symbol_key(label, self.input.unit.dialect.as_deref());
        if !valid_symbol(label) || self.labels.len() == MAX_LOCAL_REFERENCES || self.labels.contains_key(&normalized) {
            return self.malformed(
                range,
                "assembly label is invalid or exceeds the local reference bound",
            );
        }
        let fact = self.fact(
            if entry { ArchaeologyFactKind::EntryPoint } else { ArchaeologyFactKind::Declaration },
            label,
            range,
            construct,
            if entry { vec![("role", role), ("exported", "true")] } else { vec![("role", role)] },
        )?;
        self.labels.insert(normalized, fact);
        Ok(())
    }

    fn pending(&mut self, from: FactRef, target: &str, kind: ArchaeologyFactEdgeKind) -> Result<(), String> {
        if self.targets.len() == MAX_LOCAL_REFERENCES { return Err("assembly target reference bound exceeded".into()); }
        self.targets.push(PendingTarget { from, target: symbol_key(target, self.input.unit.dialect.as_deref()), kind });
        Ok(())
    }

    fn resolve_targets(&mut self) -> Result<(), String> {
        for pending in std::mem::take(&mut self.targets) {
            if let Some(target) = self.labels.get(&pending.target).cloned() {
                self.edge(&pending.from, &target, pending.kind, None)?;
            } else {
                let unresolved = self.fact(ArchaeologyFactKind::Unresolved, &pending.target,
                    pending.from.range, "unresolved", vec![("target", &pending.target)])?;
                self.edge(&pending.from, &unresolved, ArchaeologyFactEdgeKind::Unresolved,
                    Some("assembly target is not defined in this source unit"))?;
            }
        }
        Ok(())
    }

    fn include(&mut self, range: (usize, usize), target: &str, kind: ArchaeologyLineageKind,
        construct: &'static str) -> Result<(), String> {
        self.compare = None;
        if !valid_include_target(target) { return self.malformed(range, "assembly include is missing a bounded target"); }
        let include = self.fact(ArchaeologyFactKind::Include, target, range, construct, vec![("target", target)])?;
        let unresolved = self.fact(ArchaeologyFactKind::Unresolved, target, range, "unresolved", vec![("target", target)])?;
        self.edge(&include, &unresolved, ArchaeologyFactEdgeKind::Unresolved,
            Some("assembly include or macro is not expanded by the local fallback"))?;
        self.lineage.push(ArchaeologyAdapterLineage {
            kind, source_unit_id: self.input.unit.identity.source_unit_id.clone(),
            target_source_unit_id: None, evidence_span_id: include.span_id.clone(),
            detail: format!("unresolved unexpanded assembly target={target}"),
        });
        self.region(range, ArchaeologyAdapterRegionKind::Unsupported, "assembly include or macro expansion is unavailable")
    }

    fn fact(&mut self, kind: ArchaeologyFactKind, label: &str, range: (usize, usize),
        construct: &'static str, attributes: Vec<(&str, &str)>) -> Result<FactRef, String> {
        check_cancelled(self.cancellation)?;
        let span_id = self.span(range)?;
        let fact_id = archaeology_id("fact", self.input, PARSER_ID, &format!("{kind:?}\0{}\0{}", range.0, range.1));
        let mut values = vec![ArchaeologyAttribute { key: "assembly_construct".into(), value: construct.into() }];
        values.extend(attributes.into_iter().map(|(key, value)| ArchaeologyAttribute { key: key.into(), value: value.into() }));
        values.push(ArchaeologyAttribute {
            key: "semantic_expr".into(),
            value: semantic_expression(
                self.source
                    .get(range.0..range.1)
                    .ok_or("Assembly semantic expression range is invalid")?,
                self.input.unit.dialect.as_deref() == Some("hlasm"),
            )?,
        });
        self.output.emit_fact(ArchaeologyFact {
            fact_id: fact_id.clone(), kind, label: label.into(),
            span_ids: vec![span_id.clone()],
            parser_id: PARSER_ID.into(), trust: ArchaeologyTrust::Extracted,
            confidence: ArchaeologyConfidence::High, attributes: values,
        })?;
        Ok(FactRef { id: fact_id, span_id, range })
    }

    fn edge(&mut self, from: &FactRef, to: &FactRef, kind: ArchaeologyFactEdgeKind,
        unresolved_reason: Option<&str>) -> Result<(), String> {
        self.output.emit_edge(ArchaeologyFactEdge {
            edge_id: archaeology_id("edge", self.input, PARSER_ID, &format!("{}\0{}\0{kind:?}", from.id, to.id)),
            from_fact_id: from.id.clone(), to_fact_id: to.id.clone(), kind, trust: ArchaeologyTrust::Extracted,
            evidence_span_ids: vec![from.span_id.clone(), to.span_id.clone()],
            unresolved_reason: unresolved_reason.map(str::to_string),
        })
    }

    fn span(&mut self, range: (usize, usize)) -> Result<String, String> {
        let span = checked_span(self.input, self.source, PARSER_ID, range, self.positions)?;
        let id = span.span_id.clone();
        if self.spans.insert(id.clone()) { self.output.emit_span(span)?; }
        Ok(id)
    }

    fn malformed(&mut self, range: (usize, usize), reason: &str) -> Result<(), String> {
        self.region(range, ArchaeologyAdapterRegionKind::Error, reason)
    }

    fn region(&mut self, range: (usize, usize), kind: ArchaeologyAdapterRegionKind,
        reason: &str) -> Result<(), String> {
        let span_id = self.span(range)?;
        self.regions.push(ArchaeologyAdapterRegion { kind, span_id, reason: reason.into() });
        self.reasons.insert(reason.into());
        Ok(())
    }

    fn unsupported_unit(&mut self, reason: &str) -> Result<(), String> {
        if !self.source.is_empty() {
            self.region((0, self.source.len()), ArchaeologyAdapterRegionKind::Unsupported, reason)
        } else {
            self.reasons.insert(reason.into());
            Ok(())
        }
    }

    fn metadata(self, dialect: Option<&str>, evidence: Option<String>) -> ArchaeologyAdapterMetadata {
        ArchaeologyAdapterMetadata {
            dialect: dialect.map(str::to_string),
            dialect_evidence: evidence.into_iter().map(|span_id| ArchaeologyDialectEvidence {
                signal: "positive_non_conflicting_source_evidence".into(),
                value: dialect.unwrap_or("unknown").into(), span_ids: vec![span_id],
            }).collect(),
            lineage: self.lineage, regions: self.regions,
            coverage_reasons: self.reasons.into_iter().collect(),
        }
    }
}

fn symbol_operand(value: &str, dialect: Dialect) -> Option<&str> {
    let value = value.trim_start_matches('*');
    if value.starts_with(['%', '$']) || value.parse::<i64>().is_ok() {
        return None;
    }
    if dialect == Dialect::Hlasm
        && value
            .strip_prefix(['R', 'r'])
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| number <= 15)
    {
        return None;
    }
    let symbol = value.split_once('(').map_or(value, |(prefix, _)| prefix);
    valid_symbol(symbol).then_some(symbol)
}
fn relationship_hints<'a>(
    opcode: &'a str,
    construct: &str,
    dialect: Dialect,
    operands: &[&'a str],
    direct: Option<&'a str>,
) -> Vec<(&'static str, &'a str)> {
    let mut result = vec![("opcode", opcode)];
    if let Some(target) = direct {
        result.push(("target", target));
    }
    let mut add = |index: usize, key| {
        if let Some(value) = operands
            .get(index)
            .and_then(|value| symbol_operand(value, dialect))
        {
            result.push((key, value));
        }
    };
    match (construct, dialect, opcode) {
        ("comparison", _, _) => {
            for index in 0..operands.len() {
                add(index, "reads");
            }
        }
        ("memory-io", Dialect::Hlasm, "MVC") => {
            add(0, "writes");
            add(1, "reads");
        }
        ("memory-io", Dialect::Hlasm, "MVI") => add(0, "writes"),
        ("memory-io", Dialect::Hlasm, "ST") => add(1, "writes"),
        ("memory-io", Dialect::Hlasm, "L") => add(1, "reads"),
        ("memory-io", Dialect::Gas, _) if opcode.starts_with("mov") => {
            add(0, "reads");
            add(1, "writes");
        }
        ("arithmetic", Dialect::Hlasm, _) => {
            add(0, "reads");
            add(0, "writes");
            add(1, "reads");
        }
        ("arithmetic", Dialect::Gas, _) if opcode.starts_with("idiv") => {}
        ("arithmetic", Dialect::Gas, _) if operands.len() == 1 => {
            add(0, "reads");
            add(0, "writes");
        }
        ("arithmetic", Dialect::Gas, _) => {
            for index in 0..operands.len() - 1 {
                add(index, "reads");
            }
            add(operands.len() - 1, "reads");
            add(operands.len() - 1, "writes");
        }
        _ => {}
    }
    result
}

#[rustfmt::skip]
fn hlasm_kind(opcode: &str) -> Option<(ArchaeologyFactKind, &'static str, usize)> {
    let value = match opcode {
        "DC" | "DS" => (ArchaeologyFactKind::DataField, "data-definition", 1),
        "C" | "CR" | "CLC" | "CLI" => (ArchaeologyFactKind::Predicate, "comparison", 2),
        "B" | "BE" | "BNE" | "BH" | "BL" | "BNH" | "BNL" | "BR" => (ArchaeologyFactKind::ControlFlow, "branch", 1),
        "BCR" => (ArchaeologyFactKind::ControlFlow, "branch", 2),
        "BAL" | "BAS" => (ArchaeologyFactKind::Call, "call", 2),
        "A" | "AR" | "S" | "SR" | "M" | "MR" | "D" | "DR" => (ArchaeologyFactKind::Calculation, "arithmetic", 2),
        "MVC" | "MVI" | "ST" | "L" => (ArchaeologyFactKind::Mutation, "memory-io", 2),
        _ => return None,
    };
    Some(value)
}

#[rustfmt::skip]
fn gas_kind(opcode: &str) -> Option<(ArchaeologyFactKind, &'static str, usize)> {
    if opcode == "jmp" || opcode == "ret" || opcode == "retq" || opcode.starts_with('j') {
        return Some((ArchaeologyFactKind::ControlFlow, "branch", usize::from(!opcode.starts_with("ret"))));
    }
    if matches!(opcode, "call" | "callq") { return Some((ArchaeologyFactKind::Call, "call", 1)); }
    if matches!(opcode, "in" | "out" | "syscall") {
        return Some((ArchaeologyFactKind::InputOutput, "memory-io", if opcode == "syscall" { 0 } else { 2 }));
    }
    let base = ["cmp", "test", "add", "sub", "imul", "idiv", "inc", "dec", "mov"].into_iter()
        .find(|base| opcode == *base || opcode.strip_prefix(base).is_some_and(|suffix| suffix.len() == 1 && "bwlq".contains(suffix)))?;
    Some(match base {
        "cmp" | "test" => (ArchaeologyFactKind::Predicate, "comparison", 2),
        "add" | "sub" | "imul" | "idiv" => (ArchaeologyFactKind::Calculation, "arithmetic", 2),
        "inc" | "dec" => (ArchaeologyFactKind::Calculation, "arithmetic", 1),
        "mov" => (ArchaeologyFactKind::Mutation, "memory-io", 2),
        _ => return None,
    })
}

#[rustfmt::skip]
fn code_range(line: LegacyLine<'_>, dialect: Dialect) -> Option<(usize, usize)> {
    let trimmed = line.text.trim_start();
    if dialect == Dialect::Hlasm && line.text.starts_with('*')
        || dialect == Dialect::Gas && (trimmed.starts_with('#') || trimmed.starts_with("//")) { return None; }
    let (mut end, mut quote, mut escaped) = (line.text.len(), None, false);
    if dialect == Dialect::Gas { for (index, character) in line.text.char_indices() {
        if let Some(delimiter) = quote {
            if escaped { escaped = false; } else if character == '\\' { escaped = true; }
            else if character == delimiter { quote = None; }
        } else if matches!(character, '\'' | '"') { quote = Some(character); }
        else if character == '#' { end = index; break; }
    }}
    let start = line.text[..end].find(|character: char| !character.is_whitespace())?;
    let length = line.text[..end].trim_end().len();
    Some((line.start + start, line.start + length))
}

#[rustfmt::skip]
fn leading_label(text: &str) -> Option<(&str, &str)> {
    let token = text.split_whitespace().next()?;
    let label = token.strip_suffix(':').filter(|label| valid_symbol(label))?;
    Some((label, &text[token.len()..]))
}

#[rustfmt::skip]
fn operand_fields(text: &str, dialect: Dialect) -> Option<Vec<&str>> {
    if text.is_empty() { return Some(vec![]); }
    let (mut fields, mut start, mut depth, mut quote, mut escaped, mut gap) = (vec![], 0, 0_u16, None, false, false);
    for (index, character) in text.char_indices() {
        if let Some(delimiter) = quote {
            if escaped { escaped = false; }
            else if character == '\\' { escaped = true; }
            else if character == delimiter { quote = None; }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' => depth = depth.checked_add(1)?,
            ')' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => {
                let field = text[start..index].trim();
                if field.is_empty() || field.len() > 256 { return None; }
                fields.push(field);
                start = index + 1; gap = false;
            }
            _ if character.is_whitespace() => gap = true,
            _ if character.is_ascii_alphanumeric() || matches!(character,
                '_' | '.' | '$' | '@' | '+' | '-' | '*' | '=' | ',')
                || (dialect == Dialect::Gas && character == '%') => {
                    if gap && !text[start..index].trim().is_empty() { return None; }
                    gap = false;
                }
            _ => return None,
        }
    }
    if quote.is_some() || depth != 0 { return None; }
    let field = text[start..].trim();
    if field.is_empty() || field.len() > 256 { return None; }
    fields.push(field);
    Some(fields)
}

#[rustfmt::skip]
fn direct_target<'a>(opcode: &str, construct: &str, dialect: Dialect,
    values: &[&'a str]) -> Result<Option<&'a str>, ()> {
    let Some(target) = values.last().copied() else { return Ok(None) };
    if dialect == Dialect::Hlasm {
        let register = target.strip_prefix(['R', 'r']).and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| number <= 15);
        if construct == "branch" && matches!(opcode, "BR" | "BCR") && register {
            return Ok(None);
        }
        return valid_symbol(target).then_some(Some(target)).ok_or(());
    }
    if let Some(indirect) = target.strip_prefix('*') {
        let valid = indirect.strip_prefix('%').is_some_and(|register| !register.is_empty()
                && register.chars().all(|character| character.is_ascii_alphanumeric()))
            || valid_symbol(indirect) || (indirect.contains('(') && operand_fields(indirect, Dialect::Gas).is_some());
        return (valid && (construct == "call" || opcode == "jmp")).then_some(None).ok_or(());
    }
    valid_symbol(target).then_some(Some(target)).ok_or(())
}

#[rustfmt::skip]
fn valid_symbol(value: &str) -> bool {
    let value = value.trim().trim_end_matches(':');
    !value.is_empty() && value.len() <= 256
        && value.chars().next()
            .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '.' | '$'))
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '@'))
}

#[rustfmt::skip]
fn valid_include_target(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    !value.is_empty() && value.len() <= 256
        && !value.contains(['\0', '\n', '\r'])
        && !looks_like_secret(value)
        && !contains_sensitive_path(value)
        && !value.starts_with(['/', '\\'])
        && !drive
        && !value.get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:"))
        && !value.split(['/', '\\']).any(|part| part == "..")
}

#[rustfmt::skip]
fn symbol_key(value: &str, dialect: Option<&str>) -> String {
    let value = value.trim().trim_end_matches(':');
    if dialect == Some("gas-att") { value.into() } else { value.to_ascii_uppercase() }
}

#[cfg(test)]
#[path = "assembly_adapter_tests.rs"]
mod tests;
