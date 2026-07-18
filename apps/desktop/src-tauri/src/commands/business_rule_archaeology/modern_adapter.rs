use super::adapter::{
    semantic_expression, ArchaeologyAdapterEvents, ArchaeologyAdapterInput,
    ArchaeologyAdapterMetadata, ArchaeologyAdapterRegion, ArchaeologyAdapterRegionKind,
    ArchaeologyDialectEvidence, ArchaeologyLanguageAdapter, SourcePositionIndex,
};
use super::contracts::{
    ArchaeologyAttribute, ArchaeologyConfidence, ArchaeologyFact, ArchaeologyFactKind,
    ArchaeologyParserCapability, ArchaeologySourceSpan, ArchaeologyTrust,
};
use crate::commands::structural_graph::extract::extract_source_with_cancellation;
use crate::commands::structural_graph::language::SupportedLanguage;
use crate::commands::structural_graph::types::{
    stable_graph_id, GraphOrigin, GraphSourceAnchor, GraphTrust, StructuralGraphCancellation,
    BUNDLED_ENGINE_ID, BUNDLED_ENGINE_VERSION,
};
use std::collections::{BTreeMap, BTreeSet};

/// A thin archaeology projection over CodeVetter's existing Tree-sitter extraction.
///
/// It deliberately publishes only constructs already represented by the structural
/// graph. More detailed expression semantics remain explicit coverage gaps rather
/// than being guessed by a second parser.
pub struct ModernLanguageAdapter {
    language: SupportedLanguage,
    capability: ArchaeologyParserCapability,
}

impl ModernLanguageAdapter {
    pub fn new(language: SupportedLanguage) -> Self {
        Self {
            language,
            capability: ArchaeologyParserCapability {
                parser_id: BUNDLED_ENGINE_ID.to_string(),
                parser_version: format!("{BUNDLED_ENGINE_VERSION}.archaeology2"),
                language: language.name().to_string(),
                dialects: vec![language.name().to_string()],
                constructs: vec![
                    ArchaeologyFactKind::Declaration,
                    ArchaeologyFactKind::DataField,
                    ArchaeologyFactKind::Call,
                    ArchaeologyFactKind::ControlFlow,
                    ArchaeologyFactKind::EntryPoint,
                    ArchaeologyFactKind::Include,
                ],
                exact_spans: true,
                preprocessing: false,
                recovery: false,
            },
        }
    }
}

impl ArchaeologyLanguageAdapter for ModernLanguageAdapter {
    fn capability(&self) -> &ArchaeologyParserCapability {
        &self.capability
    }

    fn parse(
        &self,
        input: ArchaeologyAdapterInput<'_>,
        output: &mut dyn ArchaeologyAdapterEvents,
        positions: &SourcePositionIndex,
        cancellation: &StructuralGraphCancellation,
    ) -> Result<ArchaeologyAdapterMetadata, String> {
        check_cancelled(cancellation)?;
        let source = std::str::from_utf8(input.source)
            .map_err(|_| "Modern archaeology adapter requires UTF-8 source".to_string())?;
        let path = input
            .unit
            .identity
            .relative_path
            .as_deref()
            .ok_or("Modern archaeology adapter requires a repository-relative path")?;
        let contribution =
            extract_source_with_cancellation(path, self.language, source, cancellation);
        check_cancelled(cancellation)?;
        if !contribution.diagnostics().is_empty() {
            return Err(
                "Modern archaeology adapter refused syntax recovery or parse diagnostics"
                    .to_string(),
            );
        }

        let mut spans = BTreeSet::<String>::new();
        let mut facts = BTreeSet::<String>::new();
        let mut dialect_span = None;
        for node in contribution.nodes().iter().filter(|node| {
            node.trust == GraphTrust::Extracted && node.origin == GraphOrigin::Syntax
        }) {
            let Some(kind) = node_fact_kind(&node.kind) else {
                continue;
            };
            let Some(anchor) = node.sources.first() else {
                continue;
            };
            let span = span_from_anchor(
                &input,
                source,
                anchor,
                &self.capability.parser_id,
                positions,
            )?;
            let span_id = span.span_id.clone();
            let semantic_expr =
                semantic_expression(&format!("{} {}", node.kind, node.label), false)?;
            emit_span_once(output, cancellation, &mut spans, span)?;
            dialect_span.get_or_insert_with(|| span_id.clone());
            let fact_id =
                archaeology_id("fact", &input, &format!("{kind:?}\0{}\0{span_id}", node.id));
            if facts.insert(fact_id.clone()) {
                let mut attributes = node
                    .qualified_name
                    .iter()
                    .map(|value| ArchaeologyAttribute {
                        key: "qualified_name".to_string(),
                        value: value.clone(),
                    })
                    .collect::<Vec<_>>();
                attributes.push(ArchaeologyAttribute {
                    key: "semantic_expr".into(),
                    value: semantic_expr,
                });
                output.emit_fact(ArchaeologyFact {
                    fact_id,
                    kind,
                    label: node.label.clone(),
                    span_ids: vec![span_id],
                    parser_id: self.capability.parser_id.clone(),
                    trust: ArchaeologyTrust::Extracted,
                    confidence: ArchaeologyConfidence::High,
                    attributes,
                })?;
            }
        }

        let mut unsupported = BTreeMap::<String, String>::new();
        for metric in contribution.metrics() {
            for flow in &metric.control_flow {
                let span = span_from_anchor(
                    &input,
                    source,
                    &flow.source,
                    &self.capability.parser_id,
                    positions,
                )?;
                let span_id = span.span_id.clone();
                let semantic_expr =
                    semantic_expression(&format!("{} {}", flow.kind, flow.nesting), false)?;
                emit_span_once(output, cancellation, &mut spans, span)?;
                dialect_span.get_or_insert_with(|| span_id.clone());
                let fact_id = archaeology_id(
                    "fact",
                    &input,
                    &format!("control_flow\0{}\0{span_id}", flow.id),
                );
                if facts.insert(fact_id.clone()) {
                    output.emit_fact(ArchaeologyFact {
                        fact_id,
                        kind: ArchaeologyFactKind::ControlFlow,
                        label: flow.kind.clone(),
                        span_ids: vec![span_id.clone()],
                        parser_id: self.capability.parser_id.clone(),
                        trust: ArchaeologyTrust::Extracted,
                        confidence: ArchaeologyConfidence::High,
                        attributes: vec![
                            ArchaeologyAttribute {
                                key: "nesting".to_string(),
                                value: flow.nesting.to_string(),
                            },
                            ArchaeologyAttribute {
                                key: "semantic_expr".into(),
                                value: semantic_expr,
                            },
                        ],
                    })?;
                }
                let reason = match flow.kind.as_str() {
                    "branch" | "case" => Some(
                        "atomic predicate semantics are not exposed by the structural projection",
                    ),
                    "return" => Some(
                        "returned mutation and calculation semantics are not exposed by the structural projection",
                    ),
                    _ => None,
                };
                if let Some(reason) = reason {
                    unsupported.entry(reason.to_string()).or_insert(span_id);
                }
            }
        }

        let regions = unsupported
            .iter()
            .map(|(reason, span_id)| ArchaeologyAdapterRegion {
                kind: ArchaeologyAdapterRegionKind::Unsupported,
                span_id: span_id.clone(),
                reason: reason.clone(),
            })
            .collect::<Vec<_>>();
        let coverage_reasons = unsupported.keys().cloned().collect::<Vec<_>>();
        let dialect_span = dialect_span
            .ok_or("Modern archaeology adapter found no source-backed structural facts")?;
        Ok(ArchaeologyAdapterMetadata {
            dialect: Some(self.language.name().to_string()),
            dialect_evidence: vec![ArchaeologyDialectEvidence {
                signal: "tree_sitter_grammar".to_string(),
                value: self.language.name().to_string(),
                span_ids: vec![dialect_span],
            }],
            lineage: Vec::new(),
            regions,
            coverage_reasons,
        })
    }
}

fn node_fact_kind(kind: &str) -> Option<ArchaeologyFactKind> {
    match kind {
        "function" | "method" | "constructor" => Some(ArchaeologyFactKind::EntryPoint),
        "field" => Some(ArchaeologyFactKind::DataField),
        "class" | "interface" | "struct" | "enum" | "union" | "type" | "module" | "object" => {
            Some(ArchaeologyFactKind::Declaration)
        }
        "symbol_reference" => Some(ArchaeologyFactKind::Call),
        "module_reference" => Some(ArchaeologyFactKind::Include),
        _ => None,
    }
}

fn span_from_anchor(
    input: &ArchaeologyAdapterInput<'_>,
    source: &str,
    anchor: &GraphSourceAnchor,
    parser_id: &str,
    positions: &SourcePositionIndex,
) -> Result<ArchaeologySourceSpan, String> {
    let path = input.unit.identity.relative_path.as_deref();
    if path != Some(anchor.path.as_str()) {
        return Err("Structural fact crossed its inventoried source unit".to_string());
    }
    let start = positions
        .byte_at(
            source,
            anchor
                .start_line
                .map(u64::from)
                .ok_or("Structural fact is missing its exact line")?,
            anchor
                .start_column
                .map(u64::from)
                .ok_or("Structural fact is missing its exact column")?,
        )
        .ok_or("Structural fact start is outside the source unit")?;
    let end = positions
        .byte_at(
            source,
            anchor
                .end_line
                .map(u64::from)
                .ok_or("Structural fact is missing its exact line")?,
            anchor
                .end_column
                .map(u64::from)
                .ok_or("Structural fact is missing its exact column")?,
        )
        .ok_or("Structural fact end is outside the source unit")?;
    if start >= end || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
        return Err("Structural fact has an invalid exact source range".to_string());
    }
    let start_position = positions
        .position(source, start)
        .ok_or("Structural fact start is not a UTF-8 boundary")?;
    let end_position = positions
        .position(source, end)
        .ok_or("Structural fact end is not a UTF-8 boundary")?;
    let span_id = archaeology_id("span", input, &format!("{parser_id}\0{start}\0{end}"));
    Ok(ArchaeologySourceSpan {
        span_id,
        source_unit_id: input.unit.identity.source_unit_id.clone(),
        revision_sha: input.unit.identity.revision_sha.clone(),
        start: start_position,
        end: end_position,
    })
}

fn emit_span_once(
    output: &mut dyn ArchaeologyAdapterEvents,
    cancellation: &StructuralGraphCancellation,
    emitted: &mut BTreeSet<String>,
    span: ArchaeologySourceSpan,
) -> Result<(), String> {
    check_cancelled(cancellation)?;
    if emitted.insert(span.span_id.clone()) {
        output.emit_span(span)?;
    }
    Ok(())
}

fn archaeology_id(kind: &str, input: &ArchaeologyAdapterInput<'_>, local: &str) -> String {
    stable_graph_id(
        &format!("archaeology-{kind}"),
        &format!(
            "{}\0{}\0{}",
            input.unit.identity.repository_id, input.unit.identity.source_unit_id, local
        ),
    )
}

fn check_cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Modern archaeology adapter cancelled".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "modern_adapter_tests.rs"]
mod tests;
