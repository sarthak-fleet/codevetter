use super::*;
use std::ops::ControlFlow;
use tree_sitter::ParseOptions;

pub(crate) fn extract_source(
    path: &str,
    language: SupportedLanguage,
    source: &str,
) -> FileContribution {
    extract_source_with_cancellation(
        path,
        language,
        source,
        &StructuralGraphCancellation::default(),
    )
}

pub(crate) fn extract_source_with_cancellation(
    path: &str,
    language: SupportedLanguage,
    source: &str,
    cancellation: &StructuralGraphCancellation,
) -> FileContribution {
    if cancellation.is_cancelled() {
        return cancelled_contribution(path, language);
    }
    let mut parser = Parser::new();
    let ts_language = language.tree_sitter_language();
    if let Err(error) = parser.set_language(&ts_language) {
        return parse_error_contribution(path, language, format!("Parser setup failed: {error}"));
    }
    let bytes = source.as_bytes();
    let mut read = |offset: usize, _| {
        if cancellation.is_cancelled() {
            &bytes[bytes.len()..]
        } else {
            bytes.get(offset..).unwrap_or_default()
        }
    };
    let mut progress = |_: &tree_sitter::ParseState| {
        if cancellation.is_cancelled() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut progress);
    let tree = parser.parse_with_options(&mut read, None, Some(options));
    if cancellation.is_cancelled() {
        return cancelled_contribution(path, language);
    }
    let Some(tree) = tree else {
        return parse_error_contribution(path, language, "Parser returned no tree".to_string());
    };

    let file_id = stable_graph_id("file", path);
    let mut nodes = vec![StructuralGraphNode {
        id: file_id.clone(),
        kind: "file".to_string(),
        label: path.to_string(),
        qualified_name: Some(path.to_string()),
        path: Some(path.to_string()),
        detail: Some("syntax-indexed source file".to_string()),
        language: Some(language.name().to_string()),
        community_id: None,
        trust: GraphTrust::Extracted,
        origin: GraphOrigin::Syntax,
        sources: vec![GraphSourceAnchor::path(path)],
    }];
    let mut edges = Vec::new();
    let Some(root_metric) = extract_scope_metrics_with_cancellation(
        path,
        language,
        source,
        tree.root_node(),
        &file_id,
        "file",
        false,
        None,
        Some(cancellation),
    ) else {
        return cancelled_contribution(path, language);
    };
    let mut metrics = vec![root_metric];
    let mut identity_counts = HashMap::new();
    let mut visited = 0_usize;
    if !visit_node(
        tree.root_node(),
        source,
        path,
        language,
        &file_id,
        &[],
        &mut identity_counts,
        &mut nodes,
        &mut edges,
        &mut metrics,
        cancellation,
        &mut visited,
    ) {
        return cancelled_contribution(path, language);
    }
    if cancellation.is_cancelled() {
        return cancelled_contribution(path, language);
    }
    extract_metadata_signals(
        path,
        source,
        &file_id,
        Some(language.name()),
        &mut nodes,
        &mut edges,
    );
    attach_metadata_to_syntax_owners(&nodes, &mut edges);
    if cancellation.is_cancelled() {
        return cancelled_contribution(path, language);
    }
    let mut diagnostics = Vec::new();
    if tree.root_node().has_error() {
        diagnostics.push(StructuralGraphDiagnostic {
            severity: "warning".to_string(),
            code: "syntax_error".to_string(),
            message: "Tree-sitter recovered from one or more syntax errors; extracted nodes remain source-backed but coverage may be partial.".to_string(),
            path: Some(path.to_string()),
            language: Some(language.name().to_string()),
        });
    }

    FileContribution {
        path: path.to_string(),
        language: Some(language.name().to_string()),
        content_hash: Some(stable_graph_id("content", source)),
        byte_size: source.len() as u64,
        nodes,
        edges,
        metrics,
        diagnostics,
        disposition: FileDisposition::Indexed,
    }
}

fn cancelled_contribution(path: &str, language: SupportedLanguage) -> FileContribution {
    parse_error_contribution(
        path,
        language,
        "Structural extraction cancelled".to_string(),
    )
}

#[allow(clippy::too_many_arguments)]
fn visit_node(
    node: Node<'_>,
    source: &str,
    path: &str,
    language: SupportedLanguage,
    owner_id: &str,
    containers: &[String],
    identity_counts: &mut HashMap<String, usize>,
    nodes: &mut Vec<StructuralGraphNode>,
    edges: &mut Vec<StructuralGraphEdge>,
    metrics: &mut Vec<StructuralGraphMetricFact>,
    cancellation: &StructuralGraphCancellation,
    visited: &mut usize,
) -> bool {
    *visited += 1;
    if (*visited).is_multiple_of(256) && cancellation.is_cancelled() {
        return false;
    }
    let mut child_owner = owner_id.to_string();
    let mut child_containers = containers.to_vec();

    if let Some(kind) = declaration_kind(node.kind()) {
        if let Some(name_node) = declaration_name_node(node) {
            let Some(name) = compact_node_text(name_node, source, 120) else {
                return true;
            };
            let qualified_name = if containers.is_empty() {
                name.clone()
            } else {
                format!("{}::{name}", containers.join("::"))
            };
            let identity = format!("{path}\0{kind}\0{qualified_name}");
            let ordinal = identity_counts.entry(identity.clone()).or_insert(0);
            let node_id = stable_graph_id(kind, &format!("{identity}\0{ordinal}"));
            *ordinal += 1;
            let anchor = source_anchor(path, name_node, source);
            nodes.push(StructuralGraphNode {
                id: node_id.clone(),
                kind: kind.to_string(),
                label: name.clone(),
                qualified_name: Some(format!("{path}::{qualified_name}")),
                path: Some(path.to_string()),
                detail: Some(declaration_detail(node, source)),
                language: Some(language.name().to_string()),
                community_id: None,
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![anchor.clone()],
            });
            if is_metric_scope(kind) {
                let (public_surface, public_surface_reason) =
                    public_surface(node, source, language);
                let Some(metric) = extract_scope_metrics_with_cancellation(
                    path,
                    language,
                    source,
                    node,
                    &node_id,
                    kind,
                    public_surface,
                    public_surface_reason,
                    Some(cancellation),
                ) else {
                    return false;
                };
                metrics.push(metric);
            }
            edges.push(make_edge(
                owner_id,
                &node_id,
                "defines",
                GraphTrust::Extracted,
                GraphOrigin::Syntax,
                format!("{} declaration", node.kind()),
                vec![anchor],
                Vec::new(),
            ));
            if is_explicitly_exported(node) {
                edges.push(make_edge(
                    owner_id,
                    &node_id,
                    "exports",
                    GraphTrust::Extracted,
                    GraphOrigin::Syntax,
                    "declaration is wrapped by an explicit export syntax node".to_string(),
                    vec![source_anchor(path, node, source)],
                    Vec::new(),
                ));
            }
            if kind == "field" {
                if let Some(type_node) = declaration_type_node(node) {
                    if let Some(target) = compact_node_text(type_node, source, 160) {
                        add_reference_edge(
                            path,
                            language,
                            &node_id,
                            type_node,
                            source,
                            &target,
                            "type_reference",
                            "has_type",
                            None,
                            nodes,
                            edges,
                        );
                    }
                }
            }
            child_owner = node_id;
            if is_container_kind(kind) {
                child_containers.push(name);
            }
        }
    }

    if is_call_node(node.kind()) {
        if let Some(target) = call_target(node, source) {
            add_reference_edge(
                path,
                language,
                &child_owner,
                node,
                source,
                &target,
                "symbol_reference",
                "calls",
                None,
                nodes,
                edges,
            );
        }
    }
    if is_import_node(node.kind()) {
        if let Some(target) = import_target(node, source) {
            add_reference_edge(
                path,
                language,
                owner_id,
                node,
                source,
                &target,
                "module_reference",
                "imports",
                compact_node_text(node, source, 500),
                nodes,
                edges,
            );
        }
    }
    if is_inheritance_node(node.kind()) {
        if let Some(target) = compact_node_text(node, source, 160) {
            add_reference_edge(
                path,
                language,
                &child_owner,
                node,
                source,
                &target,
                "type_reference",
                if node.kind().contains("implement") {
                    "implements"
                } else {
                    "inherits"
                },
                None,
                nodes,
                edges,
            );
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if !visit_node(
            child,
            source,
            path,
            language,
            &child_owner,
            &child_containers,
            identity_counts,
            nodes,
            edges,
            metrics,
            cancellation,
            visited,
        ) {
            return false;
        }
    }
    true
}

fn is_explicitly_exported(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    for _ in 0..3 {
        let Some(current) = parent else {
            return false;
        };
        if matches!(
            current.kind(),
            "export_statement" | "export_declaration" | "exported_declaration"
        ) {
            return true;
        }
        parent = current.parent();
    }
    false
}

fn is_metric_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function"
            | "method"
            | "class"
            | "interface"
            | "struct"
            | "trait"
            | "module"
            | "impl"
            | "enum"
            | "object"
    )
}

fn public_surface(
    node: Node<'_>,
    source: &str,
    language: SupportedLanguage,
) -> (bool, Option<String>) {
    if is_explicitly_exported(node) {
        return (true, Some("explicit export syntax".to_string()));
    }
    let declaration = source
        .get(node.byte_range())
        .unwrap_or_default()
        .trim_start()
        .chars()
        .take(240)
        .collect::<String>();
    let lower = declaration.to_ascii_lowercase();
    if lower.starts_with("pub ")
        || lower.starts_with("pub(")
        || lower.starts_with("public ")
        || lower.starts_with("export ")
        || lower.starts_with("open ")
    {
        return (true, Some("explicit public visibility".to_string()));
    }
    let name = declaration_name(node, source).unwrap_or_default();
    if language == SupportedLanguage::Go && name.chars().next().is_some_and(char::is_uppercase) {
        return (true, Some("Go exported-name convention".to_string()));
    }
    if matches!(
        language,
        SupportedLanguage::Python | SupportedLanguage::Ruby
    ) && node
        .parent()
        .is_some_and(|parent| parent.parent().is_none())
        && !name.starts_with('_')
    {
        return (
            true,
            Some("module-level public naming convention".to_string()),
        );
    }
    (false, None)
}

fn declaration_type_node(node: Node<'_>) -> Option<Node<'_>> {
    for field in ["type", "return_type", "type_annotation"] {
        if let Some(candidate) = node.child_by_field_name(field) {
            return Some(candidate);
        }
    }
    let mut cursor = node.walk();
    let candidate = node.named_children(&mut cursor).find(|child| {
        child.kind().contains("type")
            && !matches!(child.kind(), "type_identifier" | "predefined_type")
    });
    candidate
}

#[allow(clippy::too_many_arguments)]
fn add_reference_edge(
    path: &str,
    language: SupportedLanguage,
    owner_id: &str,
    node: Node<'_>,
    source: &str,
    target: &str,
    reference_kind: &str,
    edge_kind: &str,
    reference_detail: Option<String>,
    nodes: &mut Vec<StructuralGraphNode>,
    edges: &mut Vec<StructuralGraphEdge>,
) {
    let normalized_target = normalize_reference(target);
    if normalized_target.is_empty() {
        return;
    }
    let reference_id = stable_graph_id(
        reference_kind,
        &format!("{path}\0{edge_kind}\0{normalized_target}"),
    );
    let anchor = source_anchor(path, node, source);
    nodes.push(StructuralGraphNode {
        id: reference_id.clone(),
        kind: reference_kind.to_string(),
        label: normalized_target.clone(),
        qualified_name: None,
        path: Some(path.to_string()),
        detail: Some(reference_detail.unwrap_or_else(|| format!("unresolved {edge_kind} target"))),
        language: Some(language.name().to_string()),
        community_id: None,
        trust: GraphTrust::Extracted,
        origin: GraphOrigin::Syntax,
        sources: vec![anchor.clone()],
    });
    edges.push(make_edge(
        owner_id,
        &reference_id,
        edge_kind,
        GraphTrust::Extracted,
        GraphOrigin::Syntax,
        format!("{} syntax references `{normalized_target}`", node.kind()),
        vec![anchor],
        Vec::new(),
    ));
}

fn declaration_kind(node_kind: &str) -> Option<&'static str> {
    match node_kind {
        "function_declaration"
        | "function_definition"
        | "function_item"
        | "function_signature"
        | "local_function_statement" => Some("function"),
        "method_definition"
        | "method_declaration"
        | "method_signature"
        | "method"
        | "singleton_method"
        | "method_declaration_with_body" => Some("method"),
        "constructor_declaration" | "init_declaration" => Some("constructor"),
        "class_declaration" | "class_definition" | "class_specifier" | "class" => Some("class"),
        "interface_declaration" | "protocol_declaration" | "trait_item" | "trait_declaration" => {
            Some("interface")
        }
        "struct_item" | "struct_specifier" | "struct_declaration" => Some("struct"),
        "enum_item" | "enum_specifier" | "enum_declaration" => Some("enum"),
        "union_item" | "union_specifier" => Some("union"),
        "type_alias_declaration" | "type_item" | "type_definition" | "type_declaration" => {
            Some("type")
        }
        "field_declaration"
        | "property_declaration"
        | "property_signature"
        | "public_field_definition"
        | "field_definition"
        | "struct_field" => Some("field"),
        "module"
        | "module_declaration"
        | "module_definition"
        | "mod_item"
        | "namespace_definition" => Some("module"),
        "object_declaration" => Some("object"),
        _ => None,
    }
}

fn declaration_name(node: Node<'_>, source: &str) -> Option<String> {
    declaration_name_node(node).and_then(|name| compact_node_text(name, source, 120))
}

fn declaration_name_node(node: Node<'_>) -> Option<Node<'_>> {
    for field in ["name", "declarator", "type", "identifier"] {
        if let Some(candidate) = node.child_by_field_name(field) {
            if let Some(name) = first_identifier_node(candidate, 0) {
                return Some(name);
            }
        }
    }
    first_identifier_node(node, 0)
}

fn first_identifier_node(node: Node<'_>, depth: usize) -> Option<Node<'_>> {
    if depth > 5 {
        return None;
    }
    if is_identifier_kind(node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(value) = first_identifier_node(child, depth + 1) {
            return Some(value);
        }
    }
    None
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "name"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "namespace_identifier"
            | "constant"
            | "simple_identifier"
    )
}

fn is_container_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class" | "interface" | "struct" | "enum" | "union" | "module" | "object"
    )
}

fn is_call_node(kind: &str) -> bool {
    matches!(
        kind,
        "call_expression"
            | "invocation_expression"
            | "method_invocation"
            | "function_call_expression"
            | "call"
    )
}

fn call_target(node: Node<'_>, source: &str) -> Option<String> {
    for field in ["function", "name", "method", "callee"] {
        if let Some(candidate) = node.child_by_field_name(field) {
            return compact_node_text(candidate, source, 160);
        }
    }
    node.named_child(0)
        .and_then(|candidate| compact_node_text(candidate, source, 160))
}

fn is_import_node(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "import_declaration"
            | "import_from_statement"
            | "use_declaration"
            | "using_directive"
            | "namespace_use_declaration"
            | "preproc_include"
    )
}

fn import_target(node: Node<'_>, source: &str) -> Option<String> {
    for field in ["source", "path", "module", "argument"] {
        if let Some(candidate) = node.child_by_field_name(field) {
            return compact_node_text(candidate, source, 240);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "string" | "string_literal" | "interpreted_string_literal" | "scoped_identifier"
        ) {
            return compact_node_text(child, source, 240);
        }
    }
    compact_node_text(node, source, 240)
}

fn is_inheritance_node(kind: &str) -> bool {
    matches!(
        kind,
        "extends_clause"
            | "implements_clause"
            | "superclass"
            | "super_interfaces"
            | "base_list"
            | "delegation_specifiers"
    )
}

fn compact_node_text(node: Node<'_>, source: &str, max_chars: usize) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(max_chars).collect())
}

fn declaration_detail(node: Node<'_>, source: &str) -> String {
    let signature_end = node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| node.end_byte());
    let signature = source
        .get(node.start_byte()..signature_end)
        .unwrap_or(node.kind())
        .trim();
    format!(
        "{} · {}",
        node.kind(),
        stable_graph_id("declaration-shape", signature)
    )
}

fn normalize_reference(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| matches!(character, '"' | '\'' | '`' | '<' | '>'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn source_anchor(path: &str, node: Node<'_>, source: &str) -> GraphSourceAnchor {
    let start = node.start_position();
    let end = node.end_position();
    GraphSourceAnchor {
        path: path.to_string(),
        start_line: Some(start.row as u32 + 1),
        start_column: Some(start.column as u32 + 1),
        end_line: Some(end.row as u32 + 1),
        end_column: Some(end.column as u32 + 1),
        excerpt: compact_node_text(node, source, 240),
    }
}

pub(super) fn make_edge(
    from: &str,
    to: &str,
    kind: &str,
    trust: GraphTrust,
    origin: GraphOrigin,
    evidence: String,
    sources: Vec<GraphSourceAnchor>,
    candidates: Vec<String>,
) -> StructuralGraphEdge {
    StructuralGraphEdge {
        id: stable_graph_id("edge", &format!("{kind}\0{from}\0{to}")),
        from: from.to_string(),
        to: to.to_string(),
        kind: kind.to_string(),
        evidence,
        trust,
        origin,
        sources,
        candidates,
    }
}
