//! Source-scoped structural metrics extracted from Tree-sitter scopes.
//!
//! The facts are deliberately descriptive. They are inputs to later calibrated
//! health/risk services, not findings or severity claims by themselves.

use super::language::SupportedLanguage;
use super::types::{
    stable_graph_id, GraphSourceAnchor, GraphTrust, StructuralBoundaryFact, StructuralCloneGroup,
    StructuralCloneRegion, StructuralCodeMetrics, StructuralControlFlowFact,
    StructuralGraphCancellation, StructuralGraphEdge, StructuralGraphMetricFact,
    STRUCTURAL_METRIC_SCHEMA_VERSION,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tree_sitter::Node;

const MAX_NAMES_PER_SCOPE: usize = 200;
const MAX_CONTROL_FLOW_FACTS: usize = 500;
const MAX_BOUNDARY_FACTS: usize = 200;
const MAX_NORMALIZED_TOKENS: usize = 50_000;
const MIN_CLONE_TOKENS: usize = 20;
const NORMALIZATION_METHOD: &str = "tree-sitter-leaf-kinds-v1";

#[cfg(test)]
fn extract_scope_metrics(
    path: &str,
    language: SupportedLanguage,
    source: &str,
    scope: Node<'_>,
    node_id: &str,
    scope_kind: &str,
    public_surface: bool,
    public_surface_reason: Option<String>,
) -> StructuralGraphMetricFact {
    extract_scope_metrics_with_cancellation(
        path,
        language,
        source,
        scope,
        node_id,
        scope_kind,
        public_surface,
        public_surface_reason,
        None,
    )
    .expect("uncancelled structural metric extraction")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn extract_scope_metrics_with_cancellation(
    path: &str,
    language: SupportedLanguage,
    source: &str,
    scope: Node<'_>,
    node_id: &str,
    scope_kind: &str,
    public_surface: bool,
    public_surface_reason: Option<String>,
    cancellation: Option<&StructuralGraphCancellation>,
) -> Option<StructuralGraphMetricFact> {
    let mut metrics = StructuralCodeMetrics {
        line_count: scope
            .end_position()
            .row
            .saturating_sub(scope.start_position().row)
            + 1,
        cyclomatic_complexity: 1,
        ..StructuralCodeMetrics::default()
    };
    let mut control_flow = Vec::new();
    let mut definitions = BTreeSet::new();
    let mut uses = BTreeSet::new();
    let mut boundaries = Vec::new();
    let mut stack = vec![(scope, 0_usize, None::<String>)];
    let mut visited = 0_usize;
    while let Some((node, nesting, parent_control_id)) = stack.pop() {
        visited += 1;
        if cancellation_due(cancellation, visited) {
            return None;
        }
        let is_root = same_node(node, scope);
        if !is_root && is_nested_scope(node.kind()) {
            continue;
        }

        let kind = node.kind();
        if is_statement(kind) {
            metrics.statement_count += 1;
        }
        let nesting_increment = usize::from(is_nesting_construct(kind));
        let effective_nesting = nesting + nesting_increment;
        metrics.max_nesting = metrics.max_nesting.max(effective_nesting);
        let mut child_control_id = parent_control_id.clone();
        if is_decision(kind) {
            metrics.cyclomatic_complexity += 1;
            metrics.cognitive_complexity += 1 + nesting;
            if control_flow.len() < MAX_CONTROL_FLOW_FACTS {
                let id = control_flow_id(path, node, kind);
                control_flow.push(StructuralControlFlowFact {
                    id: id.clone(),
                    kind: normalized_control_kind(kind).to_string(),
                    parent_id: parent_control_id.clone(),
                    nesting,
                    source: source_anchor(path, node, source),
                });
                child_control_id = Some(id);
            }
        } else if is_terminal_control(kind) && control_flow.len() < MAX_CONTROL_FLOW_FACTS {
            control_flow.push(StructuralControlFlowFact {
                id: control_flow_id(path, node, kind),
                kind: normalized_control_kind(kind).to_string(),
                parent_id: parent_control_id.clone(),
                nesting,
                source: source_anchor(path, node, source),
            });
        }
        if is_parameter(kind) {
            metrics.parameter_count += 1;
        }
        if is_identifier(kind) {
            if let Some(name) = node_text(node, source, 120) {
                if is_definition_identifier(node) {
                    definitions.insert(name);
                } else {
                    uses.insert(name);
                }
            }
        }
        if is_call(kind) && boundaries.len() < MAX_BOUNDARY_FACTS {
            if let Some(target) = call_target(node, source) {
                if let Some(boundary_kind) = classify_boundary(&target) {
                    boundaries.push(StructuralBoundaryFact {
                        kind: boundary_kind.to_string(),
                        target,
                        source: source_anchor(path, node, source),
                    });
                }
            }
        }

        let mut children = Vec::new();
        let mut cursor = node.walk();
        children.extend(node.children(&mut cursor));
        for child in children.into_iter().rev() {
            stack.push((child, effective_nesting, child_control_id.clone()));
        }
    }

    metrics.cohesion = calculate_cohesion(scope, source, cancellation)?;
    let (syntax_fingerprint, normalized_token_count, fingerprint_limited) =
        syntax_fingerprint(scope, source, cancellation)?;
    let mut limitations = Vec::new();
    let definitions_truncated = definitions.len() > MAX_NAMES_PER_SCOPE;
    let uses_truncated = uses.len() > MAX_NAMES_PER_SCOPE;
    if definitions_truncated {
        limitations.push("definitions_limited".to_string());
    }
    if uses_truncated {
        limitations.push("uses_limited".to_string());
    }
    if control_flow.len() >= MAX_CONTROL_FLOW_FACTS {
        limitations.push("control_flow_limited".to_string());
    }
    if boundaries.len() >= MAX_BOUNDARY_FACTS {
        limitations.push("boundaries_limited".to_string());
    }
    if !public_surface {
        limitations.push("public_surface_not_proven".to_string());
    }
    if fingerprint_limited {
        limitations.push("syntax_fingerprint_limited".to_string());
    }
    let definitions = definitions
        .into_iter()
        .take(MAX_NAMES_PER_SCOPE)
        .collect::<Vec<_>>();
    let uses = uses
        .into_iter()
        .take(MAX_NAMES_PER_SCOPE)
        .collect::<Vec<_>>();
    limitations.sort();

    Some(StructuralGraphMetricFact {
        schema_version: STRUCTURAL_METRIC_SCHEMA_VERSION,
        id: stable_graph_id("metric", node_id),
        node_id: node_id.to_string(),
        path: path.to_string(),
        scope_kind: scope_kind.to_string(),
        language: language.name().to_string(),
        public_surface,
        public_surface_reason,
        syntax_fingerprint,
        normalized_token_count,
        normalization_method: NORMALIZATION_METHOD.to_string(),
        metrics,
        control_flow,
        definitions,
        uses,
        boundaries,
        sources: vec![source_anchor(path, scope, source)],
        limitations,
    })
}

pub fn detect_clone_groups(facts: &[StructuralGraphMetricFact]) -> Vec<StructuralCloneGroup> {
    let mut by_fingerprint = BTreeMap::<&str, Vec<&StructuralGraphMetricFact>>::new();
    for fact in facts.iter().filter(|fact| {
        fact.normalized_token_count >= MIN_CLONE_TOKENS
            && matches!(
                fact.scope_kind.as_str(),
                "function" | "method" | "class" | "struct" | "impl"
            )
            && !fact
                .limitations
                .iter()
                .any(|value| value == "syntax_fingerprint_limited")
    }) {
        by_fingerprint
            .entry(fact.syntax_fingerprint.as_str())
            .or_default()
            .push(fact);
    }
    let mut groups = by_fingerprint
        .into_iter()
        .filter_map(|(fingerprint, mut facts)| {
            if facts.len() < 2 {
                return None;
            }
            facts.sort_by(|left, right| {
                left.path
                    .cmp(&right.path)
                    .then_with(|| left.node_id.cmp(&right.node_id))
            });
            let regions = facts
                .iter()
                .filter_map(|fact| {
                    fact.sources
                        .first()
                        .cloned()
                        .map(|source| StructuralCloneRegion {
                            metric_id: fact.id.clone(),
                            node_id: fact.node_id.clone(),
                            path: fact.path.clone(),
                            source,
                        })
                })
                .collect::<Vec<_>>();
            (regions.len() >= 2).then(|| StructuralCloneGroup {
                id: stable_graph_id("clone-group", fingerprint),
                syntax_fingerprint: fingerprint.to_string(),
                normalization_method: NORMALIZATION_METHOD.to_string(),
                normalized_token_count: facts[0].normalized_token_count,
                similarity: 1.0,
                regions,
                exclusions: vec![
                    "comments".to_string(),
                    "identifier_names".to_string(),
                    "literal_values".to_string(),
                    format!("scopes_under_{MIN_CLONE_TOKENS}_tokens"),
                ],
            })
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    groups
}

pub fn finalize_metric_degrees(
    facts: &mut [StructuralGraphMetricFact],
    edges: &[StructuralGraphEdge],
) {
    let index = facts
        .iter()
        .enumerate()
        .map(|(index, fact)| (fact.node_id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut incoming = vec![HashSet::<&str>::new(); facts.len()];
    let mut outgoing = vec![HashSet::<&str>::new(); facts.len()];
    for edge in edges.iter().filter(|edge| {
        matches!(edge.trust, GraphTrust::Extracted | GraphTrust::Inferred)
            && is_dependency_edge(&edge.kind)
    }) {
        if let Some(&from) = index.get(edge.from.as_str()) {
            outgoing[from].insert(edge.to.as_str());
        }
        if let Some(&to) = index.get(edge.to.as_str()) {
            incoming[to].insert(edge.from.as_str());
        }
    }
    for (index, fact) in facts.iter_mut().enumerate() {
        fact.metrics.fan_in = incoming[index].len();
        fact.metrics.fan_out = outgoing[index].len();
    }
}

fn is_dependency_edge(kind: &str) -> bool {
    !matches!(
        kind,
        "defines"
            | "contains"
            | "contains_test"
            | "declares"
            | "exports"
            | "exposes"
            | "documents"
            | "candidate_for"
            | "same_event"
            | "records_decision"
            | "configures"
            | "references"
    )
}

fn syntax_fingerprint(
    scope: Node<'_>,
    source: &str,
    cancellation: Option<&StructuralGraphCancellation>,
) -> Option<(String, usize, bool)> {
    let mut tokens = Vec::new();
    let mut total = 0_usize;
    let mut stack = vec![scope];
    let mut visited = 0_usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if cancellation_due(cancellation, visited) {
            return None;
        }
        if node.child_count() > 0 {
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor).collect::<Vec<_>>();
            children.reverse();
            stack.extend(children);
            continue;
        }
        if node.kind().contains("comment") {
            continue;
        }
        let token = normalized_token(node, source);
        if token.is_empty() {
            continue;
        }
        total += 1;
        if tokens.len() < MAX_NORMALIZED_TOKENS {
            tokens.push(token);
        }
    }
    Some((
        stable_graph_id("syntax-fingerprint", &tokens.join("\u{1f}")),
        total,
        total > MAX_NORMALIZED_TOKENS,
    ))
}

fn normalized_token(node: Node<'_>, source: &str) -> String {
    let kind = node.kind();
    if is_identifier(kind) || kind.contains("identifier") {
        return "$id".to_string();
    }
    if kind.contains("string") || kind.contains("char_literal") {
        return "$literal:string".to_string();
    }
    if kind.contains("number")
        || kind.contains("integer")
        || kind.contains("float")
        || kind.contains("decimal")
    {
        return "$literal:number".to_string();
    }
    node_text(node, source, 80)
        .map(|text| format!("{kind}:{text}"))
        .unwrap_or_else(|| kind.to_string())
}

fn same_node(left: Node<'_>, right: Node<'_>) -> bool {
    left.kind_id() == right.kind_id()
        && left.start_byte() == right.start_byte()
        && left.end_byte() == right.end_byte()
}

fn control_flow_id(path: &str, node: Node<'_>, kind: &str) -> String {
    stable_graph_id(
        "control-flow",
        &format!("{path}\0{kind}\0{}\0{}", node.start_byte(), node.end_byte()),
    )
}

fn is_nested_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "function_definition"
            | "function_item"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "method_declaration"
            | "constructor_declaration"
            | "class_declaration"
            | "class_definition"
            | "class_specifier"
            | "impl_item"
    )
}

fn is_statement(kind: &str) -> bool {
    kind.ends_with("_statement")
        || matches!(
            kind,
            "expression_statement"
                | "let_declaration"
                | "const_declaration"
                | "variable_declaration"
                | "local_variable_declaration"
                | "defer_statement"
        )
}

fn is_decision(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "if_expression"
            | "elif_clause"
            | "else_if_clause"
            | "for_statement"
            | "for_expression"
            | "for_in_statement"
            | "while_statement"
            | "while_expression"
            | "do_statement"
            | "case_statement"
            | "case_clause"
            | "switch_case"
            | "when_entry"
            | "catch_clause"
            | "except_clause"
            | "conditional_expression"
            | "ternary_expression"
            | "match_arm"
    ) || matches!(kind, "&&" | "||")
}

fn is_nesting_construct(kind: &str) -> bool {
    is_decision(kind)
        || matches!(
            kind,
            "try_statement"
                | "try_expression"
                | "switch_statement"
                | "switch_expression"
                | "match_expression"
                | "with_statement"
        )
}

fn is_terminal_control(kind: &str) -> bool {
    matches!(
        kind,
        "return_statement"
            | "break_statement"
            | "continue_statement"
            | "throw_statement"
            | "raise_statement"
            | "yield_expression"
    )
}

fn normalized_control_kind(kind: &str) -> &'static str {
    if kind.contains("if") || kind.contains("conditional") || kind.contains("ternary") {
        "branch"
    } else if kind.contains("for") || kind.contains("while") || kind.contains("do_statement") {
        "loop"
    } else if kind.contains("case") || kind.contains("when") || kind.contains("match_arm") {
        "case"
    } else if kind.contains("catch") || kind.contains("except") {
        "exception_branch"
    } else if kind.contains("return") {
        "return"
    } else if kind.contains("break") {
        "break"
    } else if kind.contains("continue") {
        "continue"
    } else if kind.contains("throw") || kind.contains("raise") {
        "throw"
    } else if kind.contains("yield") {
        "yield"
    } else {
        "decision"
    }
}

fn is_parameter(kind: &str) -> bool {
    matches!(
        kind,
        "parameter"
            | "formal_parameter"
            | "required_parameter"
            | "optional_parameter"
            | "typed_parameter"
            | "default_parameter"
            | "variadic_parameter"
    )
}

fn is_identifier(kind: &str) -> bool {
    matches!(
        kind,
        "identifier" | "field_identifier" | "property_identifier" | "type_identifier" | "constant"
    )
}

fn is_definition_identifier(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent
        .child_by_field_name("name")
        .is_some_and(|name| same_node(name, node))
        && (is_parameter(parent.kind())
            || parent.kind().contains("declarator")
            || parent.kind().contains("declaration")
            || parent.kind().contains("pattern"))
    {
        return true;
    }
    is_parameter(parent.kind()) || parent.kind().contains("pattern")
}

fn is_call(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"
            | "call"
            | "method_invocation"
            | "invocation_expression"
            | "function_call_expression"
    )
}

fn call_target(node: Node<'_>, source: &str) -> Option<String> {
    for field in ["function", "name", "method"] {
        if let Some(target) = node.child_by_field_name(field) {
            return node_text(target, source, 200);
        }
    }
    let mut cursor = node.walk();
    let target = node
        .named_children(&mut cursor)
        .next()
        .and_then(|target| node_text(target, source, 200));
    target
}

fn classify_boundary(target: &str) -> Option<&'static str> {
    let lower = target.to_ascii_lowercase();
    if [
        "fetch",
        "axios",
        "http",
        "request",
        "client.get",
        "client.post",
        "urlsession",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        Some("network")
    } else if [
        "query",
        "execute",
        "select",
        "insert",
        "update",
        "delete",
        "repository",
        "prisma",
        "sqlx",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        Some("database")
    } else if [
        "read_to_string",
        "read_file",
        "write_file",
        "fs.",
        "file.",
        "open(",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        Some("filesystem")
    } else if ["command", "spawn", "exec", "processbuilder", "subprocess"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        Some("process")
    } else if ["env::var", "getenv", "process.env", "import.meta.env"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        Some("configuration")
    } else {
        None
    }
}

fn calculate_cohesion(
    scope: Node<'_>,
    source: &str,
    cancellation: Option<&StructuralGraphCancellation>,
) -> Option<Option<f64>> {
    if !matches!(
        scope.kind(),
        "class_declaration" | "class_definition" | "class_specifier" | "struct_item" | "impl_item"
    ) {
        return Some(None);
    }
    let mut fields = BTreeSet::new();
    let mut methods = Vec::new();
    let mut stack = vec![scope];
    let mut visited = 0_usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if cancellation_due(cancellation, visited) {
            return None;
        }
        if !same_node(node, scope)
            && matches!(
                node.kind(),
                "method_definition" | "method_declaration" | "function_item"
            )
        {
            if let Some(text) = node_text(node, source, 20_000) {
                methods.push(text);
            }
            continue;
        }
        if matches!(
            node.kind(),
            "field_definition"
                | "public_field_definition"
                | "field_declaration"
                | "property_declaration"
        ) {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name| node_text(name, source, 120))
            {
                fields.insert(name);
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    if methods.len() < 2 || fields.is_empty() {
        return Some(None);
    }
    let mut field_sets = Vec::with_capacity(methods.len());
    for (index, method) in methods.iter().enumerate() {
        if cancellation_due(cancellation, index + 1) {
            return None;
        }
        field_sets.push(
            fields
                .iter()
                .filter(|field| contains_identifier(method, field))
                .collect::<HashSet<_>>(),
        );
    }
    let mut connected = 0_usize;
    let mut pairs = 0_usize;
    for left in 0..field_sets.len() {
        for right in left + 1..field_sets.len() {
            pairs += 1;
            if cancellation_due(cancellation, pairs) {
                return None;
            }
            if !field_sets[left].is_disjoint(&field_sets[right]) {
                connected += 1;
            }
        }
    }
    Some((pairs > 0).then(|| connected as f64 / pairs as f64))
}

fn cancellation_due(cancellation: Option<&StructuralGraphCancellation>, visited: usize) -> bool {
    visited.is_multiple_of(256) && cancellation.is_some_and(|token| token.is_cancelled())
}

fn contains_identifier(source: &str, identifier: &str) -> bool {
    source.match_indices(identifier).any(|(start, _)| {
        let before = source[..start].chars().next_back();
        let end = start + identifier.len();
        let after = source[end..].chars().next();
        before.is_none_or(|value| !value.is_alphanumeric() && value != '_')
            && after.is_none_or(|value| !value.is_alphanumeric() && value != '_')
    })
}

fn source_anchor(path: &str, node: Node<'_>, source: &str) -> GraphSourceAnchor {
    GraphSourceAnchor {
        path: path.to_string(),
        start_line: Some((node.start_position().row + 1) as u32),
        start_column: Some((node.start_position().column + 1) as u32),
        end_line: Some((node.end_position().row + 1) as u32),
        end_column: Some((node.end_position().column + 1) as u32),
        excerpt: node_text(node, source, 240),
    }
}

fn node_text(node: Node<'_>, source: &str, limit: usize) -> Option<String> {
    let text = source.get(node.byte_range())?.trim();
    (!text.is_empty()).then(|| text.chars().take(limit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn function_fact(
        path: &str,
        language: SupportedLanguage,
        source: &str,
    ) -> StructuralGraphMetricFact {
        let mut parser = Parser::new();
        parser
            .set_language(&language.tree_sitter_language())
            .expect("language");
        let tree = parser.parse(source, None).expect("tree");
        let mut stack = vec![tree.root_node()];
        let scope = loop {
            let node = stack.pop().expect("function scope");
            if is_nested_scope(node.kind()) {
                break node;
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        };
        extract_scope_metrics(
            path,
            language,
            source,
            scope,
            "function:fixture",
            "function",
            true,
            Some("explicit export".to_string()),
        )
    }

    #[test]
    fn complexity_def_use_control_flow_and_boundaries_are_source_scoped() {
        let fact = function_fact(
            "src/load.ts",
            SupportedLanguage::TypeScript,
            "export async function load(userId: string) { let result = 0; if (userId && result === 0) { for (const row of await db.query('users')) { result += row.id; } } return result; }",
        );
        assert!(fact.metrics.cyclomatic_complexity >= 4);
        assert!(fact.metrics.cognitive_complexity >= 3);
        assert!(fact.metrics.max_nesting >= 2);
        assert!(fact.definitions.contains(&"userId".to_string()));
        assert!(fact.uses.contains(&"result".to_string()));
        assert!(fact.control_flow.iter().any(|flow| flow.kind == "branch"));
        assert!(fact.control_flow.iter().any(|flow| flow.kind == "loop"));
        assert!(fact
            .boundaries
            .iter()
            .any(|boundary| boundary.kind == "database"));
        assert_eq!(fact.sources[0].path, "src/load.ts");
        assert!(fact.public_surface);
    }

    #[test]
    fn generic_metrics_cover_python_and_rust_tiers() {
        for (path, language, source) in [
            (
                "load.py",
                SupportedLanguage::Python,
                "def load(items):\n    for item in items:\n        if item:\n            return item\n",
            ),
            (
                "load.rs",
                SupportedLanguage::Rust,
                "pub fn load(items: Vec<u8>) -> u8 { for item in items { if item > 0 { return item; } } 0 }",
            ),
        ] {
            let fact = function_fact(path, language, source);
            assert!(fact.metrics.cyclomatic_complexity >= 3, "{path}");
            assert!(fact.metrics.line_count >= 1, "{path}");
            assert!(!fact.control_flow.is_empty(), "{path}");
        }
    }

    #[test]
    fn semantic_fan_in_and_out_ignore_containment_edges() {
        let mut facts = [
            function_fact(
                "a.ts",
                SupportedLanguage::TypeScript,
                "function a() { b(); }",
            ),
            function_fact("b.ts", SupportedLanguage::TypeScript, "function b() {}"),
        ];
        facts[0].node_id = "a".to_string();
        facts[1].node_id = "b".to_string();
        let edges = [
            StructuralGraphEdge {
                id: "call".to_string(),
                from: "a".to_string(),
                to: "b".to_string(),
                kind: "calls".to_string(),
                evidence: "fixture".to_string(),
                trust: GraphTrust::Inferred,
                origin: super::super::types::GraphOrigin::Resolution,
                sources: Vec::new(),
                candidates: Vec::new(),
            },
            StructuralGraphEdge {
                id: "contains".to_string(),
                from: "file".to_string(),
                to: "a".to_string(),
                kind: "defines".to_string(),
                evidence: "fixture".to_string(),
                trust: GraphTrust::Extracted,
                origin: super::super::types::GraphOrigin::Syntax,
                sources: Vec::new(),
                candidates: Vec::new(),
            },
        ];
        finalize_metric_degrees(&mut facts, &edges);
        assert_eq!(facts[0].metrics.fan_out, 1);
        assert_eq!(facts[0].metrics.fan_in, 0);
        assert_eq!(facts[1].metrics.fan_in, 1);
    }

    #[test]
    fn class_cohesion_is_a_separate_explainable_metric() {
        let source = "class Counter { value = 0; increment() { this.value += 1; } reset() { this.value = 0; } }";
        let language = SupportedLanguage::TypeScript;
        let mut parser = Parser::new();
        parser
            .set_language(&language.tree_sitter_language())
            .expect("language");
        let tree = parser.parse(source, None).expect("tree");
        let mut stack = vec![tree.root_node()];
        let class = loop {
            let node = stack.pop().expect("class scope");
            if node.kind() == "class_declaration" {
                break node;
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        };
        let fact = extract_scope_metrics(
            "counter.ts",
            language,
            source,
            class,
            "class:counter",
            "class",
            false,
            None,
        );
        assert_eq!(fact.metrics.cohesion, Some(1.0));
        assert!(fact
            .limitations
            .contains(&"public_surface_not_proven".to_string()));
    }

    #[test]
    fn normalized_clone_groups_cross_files_without_identifier_or_literal_bias() {
        let mut first = function_fact(
            "src/alpha.ts",
            SupportedLanguage::TypeScript,
            "function alpha(items: number[]) { let total = 0; for (const item of items) { if (item > 1) { total += item; } } return total; }",
        );
        first.node_id = "function:alpha".to_string();
        first.id = stable_graph_id("metric", &first.node_id);
        let mut second = function_fact(
            "src/beta.ts",
            SupportedLanguage::TypeScript,
            "function beta(values: number[]) { let sum = 9; for (const value of values) { if (value > 4) { sum += value; } } return sum; }",
        );
        second.node_id = "function:beta".to_string();
        second.id = stable_graph_id("metric", &second.node_id);

        assert_eq!(first.syntax_fingerprint, second.syntax_fingerprint);
        let groups = detect_clone_groups(&[first, second]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].similarity, 1.0);
        assert_eq!(groups[0].regions.len(), 2);
        assert_eq!(
            groups[0]
                .regions
                .iter()
                .map(|region| region.path.as_str())
                .collect::<Vec<_>>(),
            ["src/alpha.ts", "src/beta.ts"]
        );
        assert!(groups[0]
            .exclusions
            .contains(&"identifier_names".to_string()));
    }
}
