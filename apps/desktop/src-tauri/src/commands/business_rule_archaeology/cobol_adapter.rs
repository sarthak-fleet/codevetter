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
    LegacyToken,
};
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use std::collections::BTreeSet;

const PARSER_ID: &str = "codevetter-cobol-fallback";
#[rustfmt::skip]
const ACTIONS: &[&str] = &[
    "MOVE", "SET", "INITIALIZE", "COMPUTE", "ADD", "SUBTRACT", "MULTIPLY", "DIVIDE",
    "CALL", "PERFORM", "OPEN", "CLOSE", "READ", "WRITE", "REWRITE", "DELETE", "START", "DISPLAY", "ACCEPT",
];
#[rustfmt::skip]
const IO: &[&str] = &["SELECT", "FD", "OPEN", "CLOSE", "READ", "WRITE", "REWRITE", "DELETE", "START", "DISPLAY", "ACCEPT"];
const DIVISIONS: &[&str] = &["IDENTIFICATION", "ENVIRONMENT", "DATA", "PROCEDURE"];
#[rustfmt::skip]
const RESERVED_SENTENCES: &[&str] = &["STOP", "RUN", "GOBACK", "EXIT", "CONTINUE", "ELSE", "WHEN", "END-IF", "END-EVALUATE", "END-PERFORM"];

#[rustfmt::skip]
pub struct CobolAdapter { capability: ArchaeologyParserCapability }

#[rustfmt::skip]
impl Default for CobolAdapter {
    fn default() -> Self {
        Self { capability: ArchaeologyParserCapability {
            parser_id: PARSER_ID.into(), parser_version: "2".into(), language: "cobol".into(),
            dialects: ["fixed", "free", "copybook"].map(str::to_string).to_vec(),
            constructs: vec![
                ArchaeologyFactKind::Declaration, ArchaeologyFactKind::DataField,
                ArchaeologyFactKind::Constant, ArchaeologyFactKind::Predicate,
                ArchaeologyFactKind::Decision, ArchaeologyFactKind::Calculation,
                ArchaeologyFactKind::Mutation, ArchaeologyFactKind::Call,
                ArchaeologyFactKind::InputOutput, ArchaeologyFactKind::Transaction,
                ArchaeologyFactKind::ControlFlow,
                ArchaeologyFactKind::EntryPoint, ArchaeologyFactKind::Include,
                ArchaeologyFactKind::Unresolved,
            ],
            exact_spans: true, preprocessing: false, recovery: true,
        }}
    }
}

#[rustfmt::skip]
impl ArchaeologyLanguageAdapter for CobolAdapter {
    fn capability(&self) -> &ArchaeologyParserCapability { &self.capability }

    fn parse(&self, input: ArchaeologyAdapterInput<'_>, output: &mut dyn ArchaeologyAdapterEvents,
        positions: &SourcePositionIndex, cancellation: &StructuralGraphCancellation) -> Result<ArchaeologyAdapterMetadata, String> {
        check_cancelled(cancellation)?;
        let source = std::str::from_utf8(input.source)
            .map_err(|_| "COBOL archaeology adapter requires UTF-8 source".to_string())?;
        let mut extraction = Extraction::new(&input, source, output, positions, cancellation);
        let Some(gate) = dialect_gate(&input, source, cancellation)? else {
            let reason = dialect_gap(&input);
            extraction.region(whole_range(source)?, ArchaeologyAdapterRegionKind::Unsupported, &reason)?;
            return Ok(extraction.metadata(None, None));
        };
        let evidence = extraction.span(gate.evidence)?;
        extraction.parse(gate.format)?;
        Ok(extraction.metadata(Some(gate.dialect), Some(evidence)))
    }
}

struct DialectGate {
    dialect: &'static str,
    format: LegacyFormat,
    evidence: (usize, usize),
}

#[rustfmt::skip]
fn dialect_gate(input: &ArchaeologyAdapterInput<'_>, source: &str,
    cancellation: &StructuralGraphCancellation) -> Result<Option<DialectGate>, String> {
    if input.unit.classification != ArchaeologySourceClassification::Source { return Ok(None); }
    let dialect = input.unit.dialect.as_deref();
    let format = match dialect {
        Some("free") => LegacyFormat::Free,
        Some("fixed" | "copybook") => LegacyFormat::Fixed,
        _ => return Ok(None),
    };
    let copybook_path = input.unit.identity.relative_path.as_deref()
        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".cpy"));
    if dialect == Some("copybook") && !copybook_path { return Ok(None); }
    for line in lines(source, format) {
        check_cancelled(cancellation)?;
        let logical = line.logical().trim();
        if dialect == Some("free") && logical.eq_ignore_ascii_case(">>SOURCE FORMAT FREE") {
            let start = line.logical_start + line.logical().find('>').unwrap_or(0);
            return Ok(Some(DialectGate { dialect: "free", format, evidence: (start, line.end) }));
        }
        if format != LegacyFormat::Fixed || line.logical_start - line.start != 7 { continue; }
        let Ok(words) = tokens(source, line) else { continue; };
        let qualified = words.first().is_some_and(|token| token.is(source, "IF")
            || token.is(source, "EVALUATE") || token.is(source, "IDENTIFICATION")
            || token.text(source).parse::<u8>().is_ok());
        if qualified && (dialect != Some("copybook") || is_layout(source, &words)) {
            let dialect = if dialect == Some("copybook") { "copybook" } else { "fixed" };
            return Ok(Some(DialectGate { dialect, format, evidence: token_range(&words, 0, words.len()) }));
        }
    }
    Ok(None)
}

#[rustfmt::skip]
fn dialect_gap(input: &ArchaeologyAdapterInput<'_>) -> String {
    if input.unit.classification == ArchaeologySourceClassification::Generated {
        "generated COBOL listings are retained as unsupported evidence, not semantic facts".into()
    } else { format!("COBOL dialect lacks positive fixed, free, or copybook evidence (inventory={})",
        input.unit.dialect.as_deref().unwrap_or("unknown")) } }

#[derive(Clone)]
#[rustfmt::skip]
struct FactRef { id: String, span_id: String }

#[rustfmt::skip]
struct Extraction<'a, 'b> {
    input: &'a ArchaeologyAdapterInput<'b>, source: &'a str,
    output: &'a mut dyn ArchaeologyAdapterEvents, cancellation: &'a StructuralGraphCancellation,
    positions: &'a SourcePositionIndex, spans: BTreeSet<String>, regions: Vec<ArchaeologyAdapterRegion>,
    reasons: BTreeSet<String>, lineage: Vec<ArchaeologyAdapterLineage>, controller: Option<FactRef>,
    evaluate: Option<FactRef>, evaluate_subject: Option<String>, in_procedure: bool,
    sql_start: Option<usize>,
}

#[rustfmt::skip]
impl<'a, 'b> Extraction<'a, 'b> {
    fn new(input: &'a ArchaeologyAdapterInput<'b>, source: &'a str,
        output: &'a mut dyn ArchaeologyAdapterEvents, positions: &'a SourcePositionIndex,
        cancellation: &'a StructuralGraphCancellation) -> Self {
        Self {
            input, source, output, cancellation, positions,
            spans: BTreeSet::new(), regions: vec![], reasons: BTreeSet::new(), lineage: vec![],
            controller: None, evaluate: None, evaluate_subject: None, in_procedure: false,
            sql_start: None,
        }
    }

    fn parse(&mut self, format: LegacyFormat) -> Result<(), String> {
        for line in lines(self.source, format) {
            check_cancelled(self.cancellation)?;
            if line.text.is_empty() || matches!(line.indicator, Some(b'*' | b'/')) { continue; }
            if line.indicator == Some(b'-') {
                self.region(line.range(), ArchaeologyAdapterRegionKind::Unsupported,
                    "fixed-format continuation requires preprocessing and is not expanded")?;
                continue;
            }
            if line.indicator.is_some_and(|indicator| indicator != b' ') {
                self.region(line.range(), ArchaeologyAdapterRegionKind::Unsupported,
                    "fixed-format conditional or invalid indicator is unsupported")?;
                continue;
            }
            let words = match tokens(self.source, line) {
                Ok(words) => words,
                Err(reason) => { self.region(line.range(), ArchaeologyAdapterRegionKind::Unsupported, reason)?; continue; }
            };
            if words.is_empty() { continue; }
            if self.sql_start.is_some() {
                if position(self.source, &words, "END-EXEC").is_some() {
                    let start = self.sql_start.take().expect("checked SQL start");
                    let fact = self.sql_fact((start, statement_end(self.source, &words)))?;
                    self.control(&fact)?;
                }
                continue;
            }
            if line.logical().trim_start().starts_with(">>") {
                if !line.logical().trim().eq_ignore_ascii_case(">>SOURCE FORMAT FREE") {
                    self.region(line.range(), ArchaeologyAdapterRegionKind::Unsupported,
                        "unsupported COBOL compiler directive")?;
                }
                continue;
            }
            self.parse_line(line, &words)?;
            if words.last().is_some_and(|token| token.text(self.source) == ".") {
                self.controller = None;
                self.evaluate = None;
                self.evaluate_subject = None;
            }
        }
        if let Some(start) = self.sql_start.take() {
            self.region((start, self.source.len()), ArchaeologyAdapterRegionKind::Error,
                "unterminated EXEC SQL region")?;
        }
        Ok(())
    }

    fn parse_line(&mut self, line: LegacyLine<'_>, words: &[LegacyToken]) -> Result<(), String> {
        // A standalone period is a valid sentence terminator in real COBOL
        // sources. It closes the active control context in `parse` but has no
        // statement range of its own.
        if trimmed_len(self.source, words) == 0 { return Ok(()); }
        if words.get(1).is_some_and(|token| token.is(self.source, "DIVISION")) {
            if !DIVISIONS.iter().any(|name| words[0].is(self.source, name)) {
                return self.malformed(line, "unsupported COBOL DIVISION name");
            }
            let label = format!("{} DIVISION", words[0].text(self.source));
            self.fact(ArchaeologyFactKind::Declaration, &label, statement_range(self.source, words, 0), vec![])?;
            self.in_procedure = words[0].is(self.source, "PROCEDURE");
            return Ok(());
        }
        if words[0].is(self.source, "PROGRAM-ID") {
            let Some(name) = words.iter().skip(1).find(|token| valid_identifier(token.text(self.source))) else {
                return self.malformed(line, "PROGRAM-ID is missing a program name");
            };
            self.fact(ArchaeologyFactKind::EntryPoint, name.text(self.source), (name.start, name.end),
                vec![("declaration", "program_id"), ("exported", "true")])?;
            return Ok(());
        }
        if words[0].text(self.source).parse::<u8>().is_ok() && !is_layout(self.source, words) {
            return self.malformed(line, "invalid COBOL data level or identifier");
        }
        if is_layout(self.source, words) {
            let level = words[0].text(self.source);
            let name = words[1].text(self.source);
            let kind = if matches!(level, "78" | "88") { ArchaeologyFactKind::Constant } else { ArchaeologyFactKind::DataField };
            let mut attributes = vec![("level", level)];
            if matches!(level, "78" | "88") {
                let Some(value) = value_after(self.source, words, "VALUE").filter(|value| valid_operand(value)) else {
                    return self.malformed(line, "level-78/88 constant is missing VALUE");
                };
                attributes.push(("value", value));
            }
            self.fact(kind, name, statement_range(self.source, words, 0), attributes)?;
            return Ok(());
        }
        if words[0].is(self.source, "COPY") { return self.copybook(line, words); }
        if words[0].is(self.source, "IF") {
            let logical_end = trimmed_len(self.source, words);
            if logical_end == 2 {
                if !valid_identifier(words[1].text(self.source)) {
                    return self.malformed(line, "IF condition-name is invalid");
                }
                self.controller = Some(self.fact(ArchaeologyFactKind::Predicate, "condition-name predicate",
                    (words[1].start, words[1].end), vec![("form", "condition_name"), ("reads", words[1].text(self.source))])?);
                return Ok(());
            }
            let Some(operator) = relational_operator(self.source, words) else {
                self.controller = None;
                return self.malformed(line, "IF predicate is incomplete or unsupported");
            };
            let action = words.iter().enumerate().skip(operator + 2)
                .find_map(|(index, token)| is_action(self.source, *token).then_some(index));
            let end = action.unwrap_or_else(|| trimmed_len(self.source, words));
            if operator == 1 || operator + 1 >= end || !valid_condition(self.source, &words[1..end]) {
                self.controller = None;
                return self.malformed(line, "IF predicate is missing an operand");
            }
            let comparison_rhs_expr =
                semantic_expression(words[operator + 1].text(self.source), true)?;
            let mut attributes = vec![
                ("operator", words[operator].text(self.source)),
                ("comparison_rhs_expr", comparison_rhs_expr.as_str()),
            ];
            attributes.extend(symbol_hints(self.source, &words[1..end], "reads"));
            self.controller = Some(self.fact(ArchaeologyFactKind::Predicate, "IF predicate",
                token_range(words, 1, end), attributes)?);
            if let Some(start) = action { self.statement(line, words, start)?; }
            return Ok(());
        }
        if words[0].is(self.source, "EVALUATE") {
            if trimmed_len(self.source, words) != 2 || !valid_operand(words[1].text(self.source)) {
                self.controller = None; self.evaluate = None; self.evaluate_subject = None;
                return self.malformed(line, "EVALUATE is missing its subject");
            }
            let subject = words[1].text(self.source);
            let mut attributes = vec![("subject", subject)];
            if valid_identifier(subject) { attributes.push(("reads", subject)); }
            let fact = self.fact(ArchaeologyFactKind::Decision, "EVALUATE decision",
                statement_range(self.source, words, 0), attributes)?;
            self.evaluate_subject = Some(semantic_expression(subject, true)?);
            self.evaluate = Some(fact.clone()); self.controller = Some(fact);
            return Ok(());
        }
        if words[0].is(self.source, "WHEN") {
            let action = words.iter().position(|token| is_action(self.source, *token));
            let end = action.unwrap_or_else(|| trimmed_len(self.source, words));
            let condition = &words[1..end];
            if condition.is_empty() || !(condition.len() == 1
                && valid_operand(condition[0].text(self.source)) || valid_condition(self.source, condition)) {
                return self.malformed(line, "WHEN condition is malformed or unsupported");
            }
            let Some(context) = self.evaluate_subject.clone() else {
                return self.malformed(line, "WHEN condition has no active EVALUATE subject");
            };
            let mut attributes = symbol_hints(self.source, condition, "reads");
            attributes.push(("semantic_context", context.as_str()));
            let predicate = self.fact(ArchaeologyFactKind::Predicate, "WHEN condition",
                token_range(words, 0, end), attributes)?;
            if let Some(evaluate) = self.evaluate.clone() {
                self.edge(&evaluate, &predicate, ArchaeologyFactEdgeKind::Controls, None)?;
            }
            self.controller = Some(predicate);
            if let Some(start) = action { self.statement(line, words, start)?; }
            return Ok(());
        }
        if words[0].is(self.source, "ELSE") {
            return if trimmed_len(self.source, words) == 1 {
                Ok(())
            } else if is_action(self.source, words[1]) {
                self.statement(line, words, 1)
            } else {
                self.malformed(line, "ELSE branch action is unsupported")
            };
        }
        if words[0].is(self.source, "END-IF") { self.controller = None; return Ok(()); }
        if words[0].is(self.source, "END-EVALUATE") {
            self.controller = None; self.evaluate = None; self.evaluate_subject = None; return Ok(());
        }
        if self.in_procedure && is_paragraph(self.source, words) {
            self.fact(ArchaeologyFactKind::EntryPoint, words[0].text(self.source),
                (words[0].start, words[0].end), vec![("declaration", "paragraph")])?;
            return Ok(());
        }
        self.statement(line, words, 0)
    }

    fn statement(&mut self, line: LegacyLine<'_>, words: &[LegacyToken], start: usize) -> Result<(), String> {
        let keyword = words[start].text(self.source);
        let tail = &words[start..];
        let range = statement_range(self.source, words, start);
        if (is_action(self.source, words[start]) || IO.iter().any(|value| keyword.eq_ignore_ascii_case(value)))
            && !valid_action_shape(self.source, tail) {
            return self.malformed(line, "COBOL statement operands are malformed or unsupported");
        }
        let mut hints = statement_hints(self.source, tail);
        let fact = if ["MOVE", "SET", "INITIALIZE"].iter().any(|value| keyword.eq_ignore_ascii_case(value)) {
            hints.insert(0, ("operation", keyword));
            self.fact(ArchaeologyFactKind::Mutation, keyword, range, hints)?
        } else if ["COMPUTE", "ADD", "SUBTRACT", "MULTIPLY", "DIVIDE"]
            .iter().any(|value| keyword.eq_ignore_ascii_case(value)) {
            hints.insert(0, ("operation", keyword));
            self.fact(ArchaeologyFactKind::Calculation, keyword, range, hints)?
        } else if keyword.eq_ignore_ascii_case("CALL") || keyword.eq_ignore_ascii_case("PERFORM") {
            let kind = if keyword.eq_ignore_ascii_case("CALL") { ArchaeologyFactKind::Call }
                else { ArchaeologyFactKind::ControlFlow };
            let target = tail[1].text(self.source).trim_matches(['\'', '"']);
            let target = if keyword.eq_ignore_ascii_case("PERFORM")
                && matches!(target.to_ascii_uppercase().as_str(), "UNTIL" | "VARYING") {
                "inline"
            } else { target };
            self.fact(kind, if keyword.eq_ignore_ascii_case("CALL") { target } else { keyword },
                range, vec![("target", target)])?
        } else if keyword.eq_ignore_ascii_case("EXEC") && tail.get(1).is_some_and(|token| token.is(self.source, "SQL")) {
            if position(self.source, tail, "END-EXEC").is_none() {
                self.sql_start = Some(words[start].start); return Ok(());
            }
            self.sql_fact(range)?
        } else if IO.iter().any(|value| keyword.eq_ignore_ascii_case(value)) {
            let mut attributes = vec![("operation", keyword)]; attributes.append(&mut hints);
            self.fact(ArchaeologyFactKind::InputOutput, keyword, range, attributes)?
        } else { return Ok(()); };
        self.control(&fact)
    }

    fn sql_fact(&mut self, range: (usize, usize)) -> Result<FactRef, String> {
        match sql_transaction(&self.source[range.0..range.1]) {
            Some(operation) => self.fact(ArchaeologyFactKind::Transaction, operation, range,
                vec![("operation", operation)]),
            None => self.fact(ArchaeologyFactKind::InputOutput, "embedded SQL", range,
                vec![("operation", "exec_sql")]),
        }
    }

    fn copybook(&mut self, line: LegacyLine<'_>, words: &[LegacyToken]) -> Result<(), String> {
        let Some(target) = words.get(1).filter(|token| token.text(self.source) != ".") else {
            return self.malformed(line, "COPY is missing its target");
        };
        let name = target.text(self.source).trim_matches(['\'', '"']);
        if !valid_identifier(name) {
            return self.malformed(line, "COPY target is not a valid COBOL identifier");
        }
        let candidate = self.input.unit.include_candidates.iter().any(|candidate| candidate.kind == "copybook"
            && candidate.line == line.number && candidate.target.eq_ignore_ascii_case(name));
        let range = statement_range(self.source, words, 0);
        let include = self.fact(ArchaeologyFactKind::Include, name, range,
            vec![("lineage", "copybook"), ("inventory_candidate", if candidate { "matched" } else { "missing" })])?;
        let unresolved = self.fact(ArchaeologyFactKind::Unresolved, "unresolved copybook", range,
            vec![("target", name)])?;
        self.edge(&include, &unresolved, ArchaeologyFactEdgeKind::Unresolved,
            Some("copybook content is not expanded by the day-one local fallback"))?;
        self.lineage.push(ArchaeologyAdapterLineage {
            kind: ArchaeologyLineageKind::Copybook,
            source_unit_id: self.input.unit.identity.source_unit_id.clone(),
            target_source_unit_id: None, evidence_span_id: include.span_id.clone(),
            detail: format!("unresolved cross-unit include target={name}"),
        });
        self.region(range, ArchaeologyAdapterRegionKind::Unsupported,
            "COPY content is not expanded; unresolved source-map lineage is retained")
    }

    fn fact(&mut self, kind: ArchaeologyFactKind, label: &str, range: (usize, usize),
        attributes: Vec<(&str, &str)>) -> Result<FactRef, String> {
        check_cancelled(self.cancellation)?;
        let span_id = self.span(range)?;
        let fact_id = archaeology_id("fact", self.input, PARSER_ID,
            &format!("{kind:?}\0{}\0{}", range.0, range.1));
        let mut semantic_expr = semantic_expression(
            self.source
                .get(range.0..range.1)
                .ok_or("COBOL semantic expression range is invalid")?,
            true,
        )?;
        let mut contexts = attributes
            .iter()
            .filter(|(key, _)| *key == "semantic_context");
        if let Some((_, context)) = contexts.next() {
            if contexts.next().is_some() {
                return Err("COBOL semantic expression has duplicate context".into());
            }
            semantic_expr = semantic_expression(
                &format!("context {context} expression {semantic_expr}"),
                false,
            )?;
        }
        let mut attributes = attributes
            .into_iter()
            .filter(|(key, _)| *key != "semantic_context")
            .map(|(key, value)| ArchaeologyAttribute {
                key: key.into(), value: value.into(),
            })
            .collect::<Vec<_>>();
        attributes.push(ArchaeologyAttribute { key: "semantic_expr".into(), value: semantic_expr });
        self.output.emit_fact(ArchaeologyFact {
            fact_id: fact_id.clone(), kind, label: label.into(), span_ids: vec![span_id.clone()],
            parser_id: PARSER_ID.into(), trust: ArchaeologyTrust::Extracted,
            confidence: ArchaeologyConfidence::High,
            attributes,
        })?;
        Ok(FactRef { id: fact_id, span_id })
    }

    fn edge(&mut self, from: &FactRef, to: &FactRef, kind: ArchaeologyFactEdgeKind,
        unresolved_reason: Option<&str>) -> Result<(), String> {
        self.output.emit_edge(ArchaeologyFactEdge {
            edge_id: archaeology_id("edge", self.input, PARSER_ID,
                &format!("{}\0{}\0{kind:?}", from.id, to.id)),
            from_fact_id: from.id.clone(), to_fact_id: to.id.clone(), kind,
            trust: ArchaeologyTrust::Extracted,
            evidence_span_ids: vec![from.span_id.clone(), to.span_id.clone()],
            unresolved_reason: unresolved_reason.map(str::to_string),
        })
    }

    fn control(&mut self, fact: &FactRef) -> Result<(), String> {
        if let Some(controller) = self.controller.clone() {
            self.edge(&controller, fact, ArchaeologyFactEdgeKind::Controls, None)?;
        }
        Ok(())
    }

    fn span(&mut self, range: (usize, usize)) -> Result<String, String> {
        let span = checked_span(self.input, self.source, PARSER_ID, range, self.positions)?;
        let id = span.span_id.clone();
        if self.spans.insert(id.clone()) { self.output.emit_span(span)?; }
        Ok(id)
    }

    fn malformed(&mut self, line: LegacyLine<'_>, reason: &str) -> Result<(), String> {
        self.region(line.range(), ArchaeologyAdapterRegionKind::Error, reason)
    }

    fn region(&mut self, range: (usize, usize), kind: ArchaeologyAdapterRegionKind,
        reason: &str) -> Result<(), String> {
        let span_id = self.span(range)?;
        self.regions.push(ArchaeologyAdapterRegion { kind, span_id, reason: reason.into() });
        self.reasons.insert(reason.into());
        Ok(())
    }

    fn metadata(self, dialect: Option<&str>, evidence: Option<String>) -> ArchaeologyAdapterMetadata {
        ArchaeologyAdapterMetadata {
            dialect: dialect.map(str::to_string),
            dialect_evidence: evidence.into_iter().map(|span_id| ArchaeologyDialectEvidence {
                signal: "bounded_source_evidence".into(), value: dialect.unwrap_or("unknown").into(),
                span_ids: vec![span_id],
            }).collect(),
            lineage: self.lineage, regions: self.regions,
            coverage_reasons: self.reasons.into_iter().collect(),
        }
    }
}

#[rustfmt::skip]
fn is_layout(source: &str, words: &[LegacyToken]) -> bool { words.len() >= 2
    && words[0].text(source).parse::<u8>().is_ok_and(valid_level) && valid_identifier(words[1].text(source)) }
#[rustfmt::skip]
fn is_paragraph(source: &str, words: &[LegacyToken]) -> bool { words.len() == 2 && words[1].text(source) == "."
    && valid_identifier(words[0].text(source)) && !RESERVED_SENTENCES.iter().chain(ACTIONS).any(|word| words[0].is(source, word)) }
fn is_action(source: &str, token: LegacyToken) -> bool {
    ACTIONS.iter().any(|value| token.is(source, value))
}
#[rustfmt::skip]
fn symbol_hints<'a>(source: &'a str, words: &[LegacyToken], key: &'static str) -> Vec<(&'static str, &'a str)> {
    words.iter().filter_map(|token| { let value = token.text(source); valid_identifier(value).then_some((key, value)) }).collect() }
#[rustfmt::skip]
fn statement_hints<'a>(source: &'a str, words: &[LegacyToken]) -> Vec<(&'static str, &'a str)> {
    let words = &words[..trimmed_len(source, words)]; let keyword = words[0].text(source).to_ascii_uppercase();
    let separator = ["TO", "BY", "FROM", "INTO", "GIVING"].iter().find_map(|name| position(source, words, name).map(|index| (index, *name)));
    let mut result = vec![]; let mut add = |slice: &[LegacyToken], key| result.extend(symbol_hints(source, slice, key));
    match keyword.as_str() {
        "MOVE" => if let Some((at, _)) = separator { add(&words[1..at], "reads"); add(&words[at + 1..], "writes"); },
        "SET" => if let Some((at, mode)) = separator { add(&words[1..at], "writes"); if mode == "BY" { add(&words[1..at], "reads"); } add(&words[at + 1..], "reads"); },
        "INITIALIZE" => add(&words[1..], "writes"), "COMPUTE" => { add(&words[1..2], "writes"); add(words.get(3..).unwrap_or_default(), "reads"); },
        "ADD" | "SUBTRACT" | "MULTIPLY" => if let Some((at, mode)) = separator { add(&words[1..at], "reads"); add(&words[at + 1..], if mode == "GIVING" { "writes" } else { "reads" }); if mode != "GIVING" { add(&words[at + 1..], "writes"); } },
        "DIVIDE" => if let Some((at, mode)) = separator { add(&words[1..at], "reads"); add(&words[at + 1..], if mode == "GIVING" { "writes" } else { "reads" }); if mode == "INTO" { add(&words[at + 1..], "writes"); } else if mode == "BY" { add(&words[1..at], "writes"); } },
        "SELECT" | "FD" | "READ" | "WRITE" | "REWRITE" | "DELETE" | "START" => add(&words[1..2], "target"), "OPEN" => add(words.get(2..).unwrap_or_default(), "target"), "CLOSE" => add(&words[1..], "target"),
        "ACCEPT" => { add(&words[1..2], "target"); add(&words[1..2], "writes"); }, _ => {} }
    result }
#[rustfmt::skip]
fn relational_operator(source: &str, words: &[LegacyToken]) -> Option<usize> {
    words.iter().position(|token| matches!(token.text(source), ">" | "<" | "=" | ">=" | "<=")) }
fn position(source: &str, words: &[LegacyToken], value: &str) -> Option<usize> {
    words.iter().position(|token| token.is(source, value))
}
#[rustfmt::skip]
fn valid_level(level: u8) -> bool { matches!(level, 1..=49 | 66 | 77 | 78 | 88) }
#[rustfmt::skip]
fn valid_identifier(value: &str) -> bool {
    const RESERVED: &[&str] = &["TO", "BY", "FROM", "GIVING", "UNTIL", "VARYING", "VALUE", "PIC", "OTHER", "ZERO", "TRUE", "FALSE", "INPUT", "OUTPUT", "EXTEND", "I-O"];
    let value = value.trim_matches(['\'', '"']);
    !value.is_empty() && value.len() <= 30
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && value.bytes().any(|byte| byte.is_ascii_alphabetic())
        && !value.starts_with('-') && !value.ends_with('-')
        && !RESERVED.iter().chain(DIVISIONS).chain(ACTIONS).chain(IO).chain(RESERVED_SENTENCES)
            .any(|word| value.eq_ignore_ascii_case(word))
}
#[rustfmt::skip]
fn valid_operand(value: &str) -> bool {
    valid_identifier(value) || value.parse::<i64>().is_ok()
        || ["ZERO", "SPACE", "SPACES", "HIGH-VALUES", "LOW-VALUES", "TRUE", "FALSE", "OTHER"]
            .iter().any(|word| value.eq_ignore_ascii_case(word))
        || (value.len() >= 2 && matches!(value.as_bytes()[0], b'\'' | b'"')
            && value.as_bytes().last() == value.as_bytes().first())
}
#[rustfmt::skip]
fn valid_action_shape(source: &str, words: &[LegacyToken]) -> bool {
    let end = trimmed_len(source, words);
    let text = |index: usize| words.get(index).map(|token| token.text(source));
    let keyword = text(0).unwrap_or("");
    let keyword_at = |name: &str| (1..end).find(|index| text(*index).is_some_and(|value| value.eq_ignore_ascii_case(name)));
    let split = |name: &str| keyword_at(name).is_some_and(|index| index > 1 && index + 1 < end
        && valid_operand_list(source, &words[1..index]) && valid_operand_list(source, &words[index + 1..end]));
    match keyword.to_ascii_uppercase().as_str() {
        "MOVE" => split("TO"),
        "SET" => split("TO") || split("BY"),
        "INITIALIZE" => end > 1 && (1..end).all(|index| text(index).is_some_and(valid_identifier)),
        "COMPUTE" => keyword_at("=").is_some_and(|index| index == 2
            && text(1).is_some_and(valid_identifier) && valid_expression(source, &words[3..end])),
        "ADD" => split("TO") || split("GIVING"),
        "SUBTRACT" => split("FROM") || split("GIVING"),
        "MULTIPLY" => split("BY") || split("GIVING"),
        "DIVIDE" => split("INTO") || split("BY") || split("GIVING"),
        "CALL" => end >= 2 && text(1).is_some_and(valid_operand)
            && (end == 2 || text(2).is_some_and(|v| v.eq_ignore_ascii_case("USING"))
                && valid_operand_list(source, &words[3..end])),
        "PERFORM" => valid_perform(source, &words[..end]),
        "SELECT" => end == 5 && text(1).is_some_and(valid_identifier)
            && text(2).is_some_and(|v| v.eq_ignore_ascii_case("ASSIGN"))
            && text(3).is_some_and(|v| v.eq_ignore_ascii_case("TO")) && text(4).is_some_and(valid_operand),
        "FD" | "DELETE" | "START" | "ACCEPT" => end == 2 && text(1).is_some_and(valid_operand),
        "READ" => end == 2 && text(1).is_some_and(valid_operand) || end == 4
            && text(1).is_some_and(valid_operand) && text(2).is_some_and(|v| v.eq_ignore_ascii_case("INTO")) && text(3).is_some_and(valid_operand),
        "WRITE" | "REWRITE" => end == 2 && text(1).is_some_and(valid_operand) || end == 4
            && text(1).is_some_and(valid_operand) && text(2).is_some_and(|v| v.eq_ignore_ascii_case("FROM")) && text(3).is_some_and(valid_operand),
        "OPEN" => end > 2 && matches!(text(1).map(str::to_ascii_uppercase).as_deref(), Some("INPUT" | "OUTPUT" | "I-O" | "EXTEND")) && valid_operand_list(source, &words[2..end]),
        "CLOSE" | "DISPLAY" => valid_operand_list(source, &words[1..end]),
        _ => true,
    }
}
#[rustfmt::skip]
fn valid_perform(source: &str, words: &[LegacyToken]) -> bool {
    let text = |index: usize| words.get(index).map(|token| token.text(source));
    match text(1).map(str::to_ascii_uppercase).as_deref() {
        Some("UNTIL") => words.len() == 5 && valid_condition(source, &words[2..]),
        Some("VARYING") => words.len() == 11 && text(2).is_some_and(valid_identifier)
            && text(3).is_some_and(|v| v.eq_ignore_ascii_case("FROM")) && text(4).is_some_and(valid_operand)
            && text(5).is_some_and(|v| v.eq_ignore_ascii_case("BY")) && text(6).is_some_and(valid_operand)
            && text(7).is_some_and(|v| v.eq_ignore_ascii_case("UNTIL")) && valid_condition(source, &words[8..]),
        Some(target) if valid_identifier(target) => position(source, words, "UNTIL")
            .map_or(words.len() == 2, |index| index == 2 && valid_condition(source, &words[index + 1..])),
        _ => false,
    }
}
#[rustfmt::skip]
fn valid_operand_list(source: &str, words: &[LegacyToken]) -> bool {
    !words.is_empty() && words.len() % 2 == 1 && words.iter().enumerate().all(|(index, token)|
        if index % 2 == 0 { valid_operand(token.text(source)) } else { token.text(source) == "," })
}
#[rustfmt::skip]
fn valid_expression(source: &str, words: &[LegacyToken]) -> bool {
    words.len() == 1 && valid_operand(words[0].text(source)) || words.len() >= 3 && words.len() % 2 == 1 && words.iter().enumerate().all(|(index, token)|
        if index % 2 == 0 { valid_operand(token.text(source)) } else { matches!(token.text(source), "+" | "-" | "*" | "/") })
}
#[rustfmt::skip]
fn valid_condition(source: &str, words: &[LegacyToken]) -> bool {
    words.len() == 3 && relational_operator(source, words).is_some_and(|index| index == 1
        && valid_operand(words[0].text(source)) && valid_operand(words[2].text(source)))
}
#[rustfmt::skip]
fn sql_transaction(source: &str) -> Option<&'static str> {
    let mut words = source.split_ascii_whitespace();
    if !words.next()?.eq_ignore_ascii_case("EXEC") || !words.next()?.eq_ignore_ascii_case("SQL") { return None; }
    let operation = match words.next()? { value if value.eq_ignore_ascii_case("COMMIT") => "commit",
        value if value.eq_ignore_ascii_case("ROLLBACK") => "rollback", _ => return None };
    (words.next().is_some_and(|value| value.eq_ignore_ascii_case("END-EXEC")) && words.next().is_none()).then_some(operation)
}
#[rustfmt::skip]
fn value_after<'a>(source: &'a str, words: &[LegacyToken], keyword: &str) -> Option<&'a str> {
    words.get(position(source, words, keyword)? + 1).map(|token| token.text(source)) }
#[rustfmt::skip]
fn trimmed_len(source: &str, words: &[LegacyToken]) -> usize {
    words.len() - usize::from(words.last().is_some_and(|token| token.text(source) == ".")) }
#[rustfmt::skip]
fn token_range(words: &[LegacyToken], start: usize, end: usize) -> (usize, usize) { (words[start].start, words[end - 1].end) }
#[rustfmt::skip]
fn statement_range(source: &str, words: &[LegacyToken], start: usize) -> (usize, usize) { token_range(words, start, trimmed_len(source, words)) }
#[rustfmt::skip]
fn statement_end(source: &str, words: &[LegacyToken]) -> usize { words[trimmed_len(source, words) - 1].end }
#[rustfmt::skip]
fn whole_range(source: &str) -> Result<(usize, usize), String> { (!source.is_empty()).then_some((0, source.len()))
    .ok_or("COBOL source unit is empty and has no citable dialect evidence".into()) }

#[cfg(test)]
#[path = "cobol_adapter_tests.rs"]
mod tests;
