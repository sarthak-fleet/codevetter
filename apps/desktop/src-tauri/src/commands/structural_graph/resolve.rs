use super::types::{
    stable_graph_id, GraphOrigin, GraphTrust, StructuralGraphEdge, StructuralGraphNode,
};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

#[derive(Debug, Default)]
struct ImportContext {
    candidate_paths: HashSet<String>,
    bindings: HashMap<String, String>,
}

#[derive(Debug, Default)]
struct FileIndex<'a> {
    by_normalized_path: HashMap<String, Vec<&'a StructuralGraphNode>>,
    by_stem: HashMap<String, Vec<&'a StructuralGraphNode>>,
}

impl<'a> FileIndex<'a> {
    fn new(files: &[&'a StructuralGraphNode]) -> Self {
        let mut index = Self::default();
        for file in files {
            let Some(path) = file.path.as_deref() else {
                continue;
            };
            let normalized = normalize_path(Path::new(path));
            index
                .by_normalized_path
                .entry(normalized.trim_matches('/').to_string())
                .or_default()
                .push(*file);
            if let Some(stem) = Path::new(&normalized)
                .file_stem()
                .and_then(|stem| stem.to_str())
            {
                index
                    .by_stem
                    .entry(stem.to_string())
                    .or_default()
                    .push(*file);
            }
        }
        for candidates in index
            .by_normalized_path
            .values_mut()
            .chain(index.by_stem.values_mut())
        {
            candidates.sort_by(|left, right| left.id.cmp(&right.id));
        }
        index
    }
}

pub fn resolve_cross_file(nodes: &[StructuralGraphNode], edges: &mut Vec<StructuralGraphEdge>) {
    let node_by_id = nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut symbols_by_label: HashMap<String, Vec<&StructuralGraphNode>> = HashMap::new();
    let mut files = Vec::new();
    for node in nodes {
        if node.kind == "file" {
            files.push(node);
        } else if !node.kind.ends_with("_reference") {
            symbols_by_label
                .entry(node.label.clone())
                .or_default()
                .push(node);
        }
    }
    for candidates in symbols_by_label.values_mut() {
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
    }
    files.sort_by(|left, right| left.id.cmp(&right.id));
    let file_index = FileIndex::new(&files);

    let reference_edges = edges
        .iter()
        .filter(|edge| matches!(edge.origin, GraphOrigin::Syntax | GraphOrigin::Metadata))
        .cloned()
        .collect::<Vec<_>>();
    let mut imports_by_source_path: HashMap<String, Vec<ImportContext>> = HashMap::new();
    for edge in reference_edges.iter().filter(|edge| edge.kind == "imports") {
        let Some(reference) = node_by_id.get(edge.to.as_str()).copied() else {
            continue;
        };
        let Some(source_path) = reference.path.as_ref() else {
            continue;
        };
        let candidate_paths = resolve_module_candidates(reference, &file_index)
            .into_iter()
            .filter_map(|candidate| candidate.path.clone())
            .collect::<HashSet<_>>();
        imports_by_source_path
            .entry(source_path.clone())
            .or_default()
            .push(ImportContext {
                candidate_paths,
                bindings: parse_import_bindings(reference.detail.as_deref().unwrap_or_default()),
            });
    }
    let mut additions = Vec::new();
    for edge in reference_edges {
        let Some(reference) = node_by_id.get(edge.to.as_str()).copied() else {
            continue;
        };
        if !reference.kind.ends_with("_reference") {
            continue;
        }
        let mut candidates = if reference.kind == "module_reference" {
            resolve_module_candidates(reference, &file_index)
        } else {
            resolve_symbol_candidates(reference, &symbols_by_label, &imports_by_source_path)
        };
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        candidates.dedup_by(|left, right| left.id == right.id);

        if reference.kind == "dynamic_reference" && !candidates.is_empty() {
            additions.push(StructuralGraphEdge {
                id: stable_graph_id(
                    "edge",
                    &format!("dynamic_candidate_for\0{}\0{}", edge.from, reference.id),
                ),
                from: edge.from.clone(),
                to: reference.id.clone(),
                kind: "candidate_for".to_string(),
                evidence: format!(
                    "Runtime lookup `{}` may resolve to {} source-backed candidate(s); none was promoted to a verified edge",
                    reference.label,
                    candidates.len()
                ),
                trust: GraphTrust::Ambiguous,
                origin: GraphOrigin::Resolution,
                sources: edge.sources.clone(),
                candidates: candidates
                    .into_iter()
                    .map(|candidate| candidate.id.clone())
                    .collect(),
            });
            continue;
        }

        if candidates.len() == 1 {
            let target = candidates[0];
            additions.push(StructuralGraphEdge {
                id: stable_graph_id(
                    "edge",
                    &format!("{}\0{}\0{}", edge.kind, edge.from, target.id),
                ),
                from: edge.from.clone(),
                to: target.id.clone(),
                kind: edge.kind.clone(),
                evidence: format!(
                    "Resolved `{}` to `{}` using deterministic path/name and source-context rules",
                    reference.label,
                    target.qualified_name.as_deref().unwrap_or(&target.label)
                ),
                trust: GraphTrust::Inferred,
                origin: GraphOrigin::Resolution,
                sources: edge.sources.clone(),
                candidates: Vec::new(),
            });
        } else if !candidates.is_empty() {
            let candidate_ids = candidates
                .iter()
                .map(|candidate| candidate.id.clone())
                .collect::<Vec<_>>();
            additions.push(StructuralGraphEdge {
                id: stable_graph_id(
                    "edge",
                    &format!("candidate_for\0{}\0{}", edge.from, reference.id),
                ),
                from: edge.from.clone(),
                to: reference.id.clone(),
                kind: "candidate_for".to_string(),
                evidence: format!(
                    "`{}` has {} plausible targets; no target was selected silently",
                    reference.label,
                    candidate_ids.len()
                ),
                trust: GraphTrust::Ambiguous,
                origin: GraphOrigin::Resolution,
                sources: edge.sources.clone(),
                candidates: candidate_ids,
            });
        }
    }
    resolve_document_links(nodes, &files, &mut additions);
    resolve_test_targets(nodes, &files, &mut additions);
    relate_analytics_events(nodes, &mut additions);
    edges.extend(additions);
}

fn resolve_document_links(
    nodes: &[StructuralGraphNode],
    files: &[&StructuralGraphNode],
    additions: &mut Vec<StructuralGraphEdge>,
) {
    let files_by_path = files
        .iter()
        .filter_map(|file| {
            file.path
                .as_deref()
                .map(|path| (normalize_path(Path::new(path)), *file))
        })
        .collect::<HashMap<_, _>>();
    for link in nodes
        .iter()
        .filter(|node| node.kind == "documentation_link")
    {
        let Some(source_path) = link.path.as_deref() else {
            continue;
        };
        let target = link.label.split('#').next().unwrap_or_default();
        if target.starts_with("http://") || target.starts_with("https://") || target.is_empty() {
            continue;
        }
        let source_directory = Path::new(source_path)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        let resolved = normalize_path(&source_directory.join(target));
        let Some(target_file) = files_by_path.get(&resolved) else {
            continue;
        };
        let source_file_id = stable_graph_id("file", source_path);
        additions.push(StructuralGraphEdge {
            id: stable_graph_id(
                "edge",
                &format!("documents\0{source_file_id}\0{}", target_file.id),
            ),
            from: source_file_id,
            to: target_file.id.clone(),
            kind: "documents".to_string(),
            evidence: format!(
                "Documentation link resolves `{}` to `{resolved}`",
                link.label
            ),
            trust: GraphTrust::Inferred,
            origin: GraphOrigin::Resolution,
            sources: link.sources.clone(),
            candidates: Vec::new(),
        });
    }
}

fn resolve_test_targets(
    nodes: &[StructuralGraphNode],
    files: &[&StructuralGraphNode],
    additions: &mut Vec<StructuralGraphEdge>,
) {
    for test in nodes.iter().filter(|node| node.kind == "test") {
        let Some(test_path) = test.path.as_deref() else {
            continue;
        };
        let test_stem = normalized_test_stem(test_path);
        if test_stem.is_empty() {
            continue;
        }
        let mut candidates = files
            .iter()
            .copied()
            .filter(|file| file.path.as_deref() != Some(test_path))
            .filter(|file| {
                file.path.as_deref().map(normalized_test_stem).as_deref()
                    == Some(test_stem.as_str())
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        if candidates.len() == 1 {
            let target = candidates[0];
            additions.push(StructuralGraphEdge {
                id: stable_graph_id("edge", &format!("tests\0{}\0{}", test.id, target.id)),
                from: test.id.clone(),
                to: target.id.clone(),
                kind: "tests".to_string(),
                evidence: "Unique production file matched by conventional test filename"
                    .to_string(),
                trust: GraphTrust::Inferred,
                origin: GraphOrigin::Resolution,
                sources: test.sources.clone(),
                candidates: Vec::new(),
            });
        } else if !candidates.is_empty() {
            additions.push(StructuralGraphEdge {
                id: stable_graph_id("edge", &format!("candidate_test_target\0{}", test.id)),
                from: test.id.clone(),
                to: test.id.clone(),
                kind: "candidate_for".to_string(),
                evidence: "Test filename matches multiple possible production files".to_string(),
                trust: GraphTrust::Ambiguous,
                origin: GraphOrigin::Resolution,
                sources: test.sources.clone(),
                candidates: candidates
                    .into_iter()
                    .map(|candidate| candidate.id.clone())
                    .collect(),
            });
        }
    }
}

fn normalized_test_stem(path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    stem.trim_start_matches("test_")
        .trim_end_matches(".test")
        .trim_end_matches(".spec")
        .trim_end_matches("_test")
        .to_string()
}

fn relate_analytics_events(
    nodes: &[StructuralGraphNode],
    additions: &mut Vec<StructuralGraphEdge>,
) {
    let mut by_label: HashMap<&str, Vec<&StructuralGraphNode>> = HashMap::new();
    for node in nodes.iter().filter(|node| node.kind == "analytics_event") {
        by_label.entry(node.label.as_str()).or_default().push(node);
    }
    for occurrences in by_label.values_mut() {
        occurrences.sort_by(|left, right| left.id.cmp(&right.id));
        for pair in occurrences.windows(2) {
            additions.push(StructuralGraphEdge {
                id: stable_graph_id(
                    "edge",
                    &format!("same_event\0{}\0{}", pair[0].id, pair[1].id),
                ),
                from: pair[0].id.clone(),
                to: pair[1].id.clone(),
                kind: "same_event".to_string(),
                evidence: format!(
                    "Source-backed analytics callsites share exact event label `{}`",
                    pair[0].label
                ),
                trust: GraphTrust::Inferred,
                origin: GraphOrigin::Resolution,
                sources: pair
                    .iter()
                    .flat_map(|node| node.sources.iter().cloned())
                    .collect(),
                candidates: Vec::new(),
            });
        }
    }
}

fn resolve_symbol_candidates<'a>(
    reference: &StructuralGraphNode,
    symbols_by_label: &'a HashMap<String, Vec<&'a StructuralGraphNode>>,
    imports_by_source_path: &HashMap<String, Vec<ImportContext>>,
) -> Vec<&'a StructuralGraphNode> {
    let terminal = terminal_reference_name(&reference.label);
    let source_path = reference.path.as_deref().unwrap_or_default();
    let import_contexts = imports_by_source_path
        .get(source_path)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let imported_binding = import_contexts
        .iter()
        .find_map(|context| context.bindings.get(&terminal))
        .cloned();
    let lookup = imported_binding.as_deref().unwrap_or(&terminal);
    let Some(all) = symbols_by_label.get(lookup) else {
        return Vec::new();
    };
    if all.len() <= 1 {
        return all.clone();
    }
    let same_file = all
        .iter()
        .copied()
        .filter(|candidate| candidate.path.as_deref() == Some(source_path))
        .collect::<Vec<_>>();
    if same_file.len() == 1 {
        return same_file;
    }

    let imported = all
        .iter()
        .copied()
        .filter(|candidate| {
            candidate.path.as_ref().is_some_and(|candidate_path| {
                import_contexts.iter().any(|context| {
                    context.candidate_paths.contains(candidate_path)
                        && (context.bindings.is_empty()
                            || context
                                .bindings
                                .get(&terminal)
                                .is_some_and(|name| name == lookup)
                            || context.bindings.values().any(|name| name == lookup))
                })
            })
        })
        .collect::<Vec<_>>();
    if imported.len() == 1 {
        return imported;
    }

    let source_directory = Path::new(source_path).parent();
    let same_directory = all
        .iter()
        .copied()
        .filter(|candidate| {
            candidate
                .path
                .as_deref()
                .and_then(|path| Path::new(path).parent())
                == source_directory
        })
        .collect::<Vec<_>>();
    if same_directory.len() == 1 {
        return same_directory;
    }

    all.clone()
}

fn parse_import_bindings(statement: &str) -> HashMap<String, String> {
    let normalized = statement
        .replace(['{', '}', '(', ')', ';', ','], " ")
        .replace("::", " ")
        .replace('.', " ");
    let words = normalized
        .split_whitespace()
        .map(|word| word.trim_matches(['\'', '"', '`']))
        .collect::<Vec<_>>();
    let mut bindings = HashMap::new();
    for window in words.windows(3) {
        if window[1].eq_ignore_ascii_case("as")
            && is_binding_identifier(window[0])
            && is_binding_identifier(window[2])
        {
            bindings.insert(window[2].to_string(), window[0].to_string());
        }
    }
    if let Some(import_position) = words
        .iter()
        .position(|word| word.eq_ignore_ascii_case("import"))
    {
        for word in words.iter().skip(import_position + 1) {
            if word.eq_ignore_ascii_case("from") || word.eq_ignore_ascii_case("as") {
                break;
            }
            if is_binding_identifier(word) {
                bindings
                    .entry((*word).to_string())
                    .or_insert_with(|| (*word).to_string());
            }
        }
    }
    bindings
}

fn is_binding_identifier(value: &str) -> bool {
    !value.is_empty()
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "import" | "from" | "use" | "export" | "type" | "pub" | "crate" | "self" | "super"
        )
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_' || character == '$')
}

fn resolve_module_candidates<'a>(
    reference: &StructuralGraphNode,
    files: &FileIndex<'a>,
) -> Vec<&'a StructuralGraphNode> {
    let target = clean_module_reference(&reference.label);
    if target.is_empty() {
        return Vec::new();
    }
    let source_path = reference.path.as_deref().unwrap_or_default();
    let source_directory = Path::new(source_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let relative_target = if target.starts_with('.') {
        normalize_path(&source_directory.join(&target))
    } else {
        target.replace("::", "/").replace('.', "/")
    };
    let mut expected = HashSet::new();
    let trimmed = relative_target.trim_matches('/').to_string();
    let mut bases = vec![trimmed.clone()];
    if let Some(rest) = trimmed.strip_prefix("crate/") {
        bases.push(format!("src/{rest}"));
    }
    if let Some(rest) = trimmed.strip_prefix("self/") {
        bases.push(normalize_path(&source_directory.join(rest)));
    }
    if let Some((parent, _symbol)) = trimmed.rsplit_once('/') {
        if !parent.is_empty() {
            bases.push(parent.to_string());
            if let Some(rest) = parent.strip_prefix("crate/") {
                bases.push(format!("src/{rest}"));
            }
        }
    }
    bases.sort();
    bases.dedup();
    for base in &bases {
        expected.insert(base.to_string());
        for extension in [
            "ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "c", "h", "cpp", "hpp", "cs", "rb",
            "php", "kt", "swift",
        ] {
            expected.insert(format!("{base}.{extension}"));
            expected.insert(format!("{base}/index.{extension}"));
            expected.insert(format!("{base}/mod.{extension}"));
        }
    }

    let terminal = terminal_reference_name(&target);
    let mut candidates = expected
        .iter()
        .filter_map(|path| files.by_normalized_path.get(path))
        .flatten()
        .copied()
        .chain(files.by_stem.get(&terminal).into_iter().flatten().copied())
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.id.cmp(&right.id));
    candidates.dedup_by(|left, right| left.id == right.id);
    candidates
}

fn clean_module_reference(value: &str) -> String {
    let value = value.trim().trim_end_matches(';').trim();
    let value = value
        .strip_prefix("use ")
        .or_else(|| value.strip_prefix("import "))
        .or_else(|| value.strip_prefix("from "))
        .or_else(|| value.strip_prefix("#include"))
        .unwrap_or(value)
        .trim();
    value
        .split_whitespace()
        .last()
        .unwrap_or(value)
        .trim_matches(|character| matches!(character, '"' | '\'' | '`' | '<' | '>' | '{' | '}'))
        .to_string()
}

fn terminal_reference_name(value: &str) -> String {
    clean_module_reference(value)
        .rsplit(['.', ':', '/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(value)
        .trim_end_matches(['(', ')', '!', '?'])
        .to_string()
}

fn normalize_path(path: &Path) -> String {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                components.pop();
            }
            Component::CurDir => {}
            Component::Normal(value) => components.push(value.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    components.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::structural_graph::types::GraphSourceAnchor;

    fn node(id: &str, kind: &str, label: &str, path: &str) -> StructuralGraphNode {
        StructuralGraphNode {
            id: id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            qualified_name: Some(format!("{path}::{label}")),
            path: Some(path.to_string()),
            detail: None,
            language: Some("typescript".to_string()),
            community_id: None,
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Syntax,
            sources: vec![GraphSourceAnchor::path(path)],
        }
    }

    #[test]
    fn unique_same_file_symbol_is_preferred() {
        let nodes = vec![
            node("file:a", "file", "a.ts", "a.ts"),
            node("function:local", "function", "run", "a.ts"),
            node("function:other", "function", "run", "b.ts"),
            node("ref:run", "symbol_reference", "run", "a.ts"),
        ];
        let mut edges = vec![StructuralGraphEdge {
            id: "syntax".to_string(),
            from: "function:local".to_string(),
            to: "ref:run".to_string(),
            kind: "calls".to_string(),
            evidence: "call".to_string(),
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Syntax,
            sources: vec![GraphSourceAnchor::path("a.ts")],
            candidates: Vec::new(),
        }];
        resolve_cross_file(&nodes, &mut edges);
        assert!(edges.iter().any(|edge| {
            edge.origin == GraphOrigin::Resolution
                && edge.to == "function:local"
                && edge.trust == GraphTrust::Inferred
        }));
    }

    #[test]
    fn ambiguous_symbols_retain_all_candidates() {
        let nodes = vec![
            node("function:a", "function", "run", "a.ts"),
            node("function:b", "function", "run", "b.ts"),
            node("ref:run", "symbol_reference", "run", "c.ts"),
        ];
        let mut edges = vec![StructuralGraphEdge {
            id: "syntax".to_string(),
            from: "file:c".to_string(),
            to: "ref:run".to_string(),
            kind: "calls".to_string(),
            evidence: "call".to_string(),
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Syntax,
            sources: vec![GraphSourceAnchor::path("c.ts")],
            candidates: Vec::new(),
        }];
        resolve_cross_file(&nodes, &mut edges);
        let ambiguous = edges
            .iter()
            .find(|edge| edge.trust == GraphTrust::Ambiguous)
            .expect("ambiguous edge");
        assert_eq!(ambiguous.candidates, vec!["function:a", "function:b"]);
    }

    #[test]
    fn import_alias_and_module_path_disambiguate_cross_file_calls() {
        let mut module_reference = node("ref:module", "module_reference", "./a", "src/caller.ts");
        module_reference.detail = Some("import { run as importedRun } from './a';".to_string());
        let nodes = vec![
            node("file:a", "file", "src/a.ts", "src/a.ts"),
            node("file:b", "file", "src/b.ts", "src/b.ts"),
            node("file:caller", "file", "src/caller.ts", "src/caller.ts"),
            node("function:a", "function", "run", "src/a.ts"),
            node("function:b", "function", "run", "src/b.ts"),
            module_reference,
            node(
                "ref:call",
                "symbol_reference",
                "importedRun",
                "src/caller.ts",
            ),
        ];
        let mut edges = vec![
            StructuralGraphEdge {
                id: "import".to_string(),
                from: "file:caller".to_string(),
                to: "ref:module".to_string(),
                kind: "imports".to_string(),
                evidence: "import".to_string(),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![GraphSourceAnchor::path("src/caller.ts")],
                candidates: Vec::new(),
            },
            StructuralGraphEdge {
                id: "call".to_string(),
                from: "file:caller".to_string(),
                to: "ref:call".to_string(),
                kind: "calls".to_string(),
                evidence: "call".to_string(),
                trust: GraphTrust::Extracted,
                origin: GraphOrigin::Syntax,
                sources: vec![GraphSourceAnchor::path("src/caller.ts")],
                candidates: Vec::new(),
            },
        ];
        resolve_cross_file(&nodes, &mut edges);
        assert!(edges.iter().any(|edge| {
            edge.origin == GraphOrigin::Resolution
                && edge.kind == "calls"
                && edge.to == "function:a"
        }));
        assert!(!edges.iter().any(|edge| {
            edge.origin == GraphOrigin::Resolution
                && edge.kind == "calls"
                && edge.to == "function:b"
        }));
    }

    #[test]
    fn docs_tests_and_events_gain_qualified_cross_file_relationships() {
        let nodes = vec![
            node("file:readme", "file", "README.md", "README.md"),
            node("file:guide", "file", "guide.md", "docs/guide.md"),
            node("file:user", "file", "user.ts", "src/user.ts"),
            node(
                "file:user-test",
                "file",
                "user.test.ts",
                "tests/user.test.ts",
            ),
            node(
                "doc:guide",
                "documentation_link",
                "docs/guide.md",
                "README.md",
            ),
            node("test:user", "test", "loads user", "tests/user.test.ts"),
            node("event:a", "analytics_event", "user_loaded", "src/user.ts"),
            node(
                "event:b",
                "analytics_event",
                "user_loaded",
                "tests/user.test.ts",
            ),
        ];
        let mut edges = Vec::new();
        resolve_cross_file(&nodes, &mut edges);
        assert!(edges
            .iter()
            .any(|edge| edge.kind == "documents" && edge.to == "file:guide"));
        assert!(edges
            .iter()
            .any(|edge| edge.kind == "tests" && edge.to == "file:user"));
        assert!(edges.iter().any(|edge| {
            edge.kind == "same_event"
                && edge.trust == GraphTrust::Inferred
                && edge.evidence.contains("user_loaded")
        }));
    }

    #[test]
    fn multi_language_cycles_cross_packages_and_unresolved_calls_remain_honest() {
        let mut rust_entry = node(
            "function:rust",
            "function",
            "rust_entry",
            "crates/core/src/lib.rs",
        );
        rust_entry.language = Some("rust".to_string());
        let mut python_entry = node(
            "function:python",
            "function",
            "python_entry",
            "packages/api/main.py",
        );
        python_entry.language = Some("python".to_string());
        let mut rust_reference = node(
            "ref:rust",
            "symbol_reference",
            "rust_entry",
            "packages/api/main.py",
        );
        rust_reference.language = Some("python".to_string());
        let mut python_reference = node(
            "ref:python",
            "symbol_reference",
            "python_entry",
            "crates/core/src/lib.rs",
        );
        python_reference.language = Some("rust".to_string());
        let unresolved = node(
            "ref:missing",
            "symbol_reference",
            "not_declared",
            "packages/api/main.py",
        );
        let nodes = vec![
            rust_entry,
            python_entry,
            rust_reference,
            python_reference,
            unresolved,
            node(
                "function:dup-a",
                "function",
                "duplicate",
                "packages/a/mod.ts",
            ),
            node(
                "function:dup-b",
                "function",
                "duplicate",
                "packages/b/mod.ts",
            ),
            node(
                "ref:duplicate",
                "symbol_reference",
                "duplicate",
                "packages/c/mod.ts",
            ),
        ];
        let syntax_edge = |id: &str, from: &str, to: &str| StructuralGraphEdge {
            id: id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            kind: "calls".to_string(),
            evidence: "source-located call fixture".to_string(),
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Syntax,
            sources: Vec::new(),
            candidates: Vec::new(),
        };
        let mut edges = vec![
            syntax_edge("call:rust", "function:python", "ref:rust"),
            syntax_edge("call:python", "function:rust", "ref:python"),
            syntax_edge("call:missing", "function:python", "ref:missing"),
            syntax_edge("call:ambiguous", "function:python", "ref:duplicate"),
        ];

        resolve_cross_file(&nodes, &mut edges);

        assert!(edges.iter().any(|edge| {
            edge.from == "function:python"
                && edge.to == "function:rust"
                && edge.trust == GraphTrust::Inferred
        }));
        assert!(edges.iter().any(|edge| {
            edge.from == "function:rust"
                && edge.to == "function:python"
                && edge.trust == GraphTrust::Inferred
        }));
        let ambiguous = edges
            .iter()
            .find(|edge| edge.to == "ref:duplicate" && edge.trust == GraphTrust::Ambiguous)
            .expect("ambiguous relationship");
        assert_eq!(
            ambiguous.candidates,
            vec!["function:dup-a", "function:dup-b"]
        );
        assert!(!edges.iter().any(|edge| {
            edge.origin == GraphOrigin::Resolution
                && (edge.to == "ref:missing" || edge.evidence.contains("not_declared"))
        }));
    }
}
