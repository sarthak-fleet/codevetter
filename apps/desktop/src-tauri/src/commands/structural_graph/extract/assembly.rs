use super::*;

pub(super) fn metadata_file_contribution(
    path: String,
    language: Option<SupportedLanguage>,
    disposition: FileDisposition,
) -> FileContribution {
    let language_name = language.map(|language| language.name().to_string());
    let (diagnostic_code, diagnostic_message) = match disposition {
        FileDisposition::Unsupported => (
            "unsupported_language",
            "File is retained as metadata because no syntax grammar is bundled",
        ),
        FileDisposition::Generated => (
            "generated_file_skipped",
            "Generated file is retained as metadata and excluded from syntax extraction",
        ),
        FileDisposition::TooLarge => (
            "file_too_large",
            "File exceeds the configured syntax extraction byte limit",
        ),
        _ => ("metadata_only", "File is indexed as metadata only"),
    };
    FileContribution {
        path: path.clone(),
        language: language_name.clone(),
        content_hash: None,
        byte_size: 0,
        nodes: vec![StructuralGraphNode {
            id: stable_graph_id("file", &path),
            kind: "file".to_string(),
            label: path.clone(),
            qualified_name: Some(path.clone()),
            path: Some(path.clone()),
            detail: Some(
                match disposition {
                    FileDisposition::Unsupported => "metadata-only unsupported file",
                    FileDisposition::Generated => "metadata-only generated file",
                    FileDisposition::TooLarge => "metadata-only oversized source file",
                    _ => "metadata-only file",
                }
                .to_string(),
            ),
            language: language_name.clone(),
            community_id: None,
            trust: GraphTrust::Extracted,
            origin: GraphOrigin::Metadata,
            sources: vec![GraphSourceAnchor::path(path.clone())],
        }],
        edges: Vec::new(),
        metrics: Vec::new(),
        diagnostics: vec![StructuralGraphDiagnostic {
            severity: "info".to_string(),
            code: diagnostic_code.to_string(),
            message: diagnostic_message.to_string(),
            path: Some(path),
            language: language_name,
        }],
        disposition,
    }
}

pub(super) fn skipped_contribution(
    path: String,
    language: Option<SupportedLanguage>,
    disposition: FileDisposition,
) -> FileContribution {
    let (code, message) = match disposition {
        FileDisposition::Sensitive => (
            "sensitive_file_skipped",
            "Sensitive file content and original path were excluded from the graph",
        ),
        FileDisposition::Binary => (
            "binary_file_skipped",
            "Binary file content was excluded from the graph",
        ),
        _ => (
            "file_skipped",
            "File was excluded from structural extraction",
        ),
    };
    FileContribution {
        path: path.clone(),
        language: language.map(|language| language.name().to_string()),
        content_hash: None,
        byte_size: 0,
        nodes: Vec::new(),
        edges: Vec::new(),
        metrics: Vec::new(),
        diagnostics: vec![StructuralGraphDiagnostic {
            severity: "info".to_string(),
            code: code.to_string(),
            message: message.to_string(),
            path: Some(path),
            language: language.map(|language| language.name().to_string()),
        }],
        disposition,
    }
}

pub(super) fn parse_error_contribution(
    path: &str,
    language: SupportedLanguage,
    message: String,
) -> FileContribution {
    FileContribution {
        path: path.to_string(),
        language: Some(language.name().to_string()),
        content_hash: None,
        byte_size: 0,
        nodes: Vec::new(),
        edges: Vec::new(),
        metrics: Vec::new(),
        diagnostics: vec![StructuralGraphDiagnostic {
            severity: "error".to_string(),
            code: "parser_failed".to_string(),
            message,
            path: Some(path.to_string()),
            language: Some(language.name().to_string()),
        }],
        disposition: FileDisposition::Error,
    }
}

pub(super) fn file_record_from_contribution(
    contribution: &FileContribution,
) -> StructuralGraphFileRecord {
    StructuralGraphFileRecord {
        path: contribution.path.clone(),
        language: contribution.language.clone(),
        content_hash: contribution.content_hash.clone(),
        disposition: contribution.disposition.as_str().to_string(),
        byte_size: contribution.byte_size,
        node_count: contribution.nodes.len(),
        edge_count: contribution.edges.len(),
    }
}

pub(super) fn node_belongs_to_paths(node: &StructuralGraphNode, paths: &HashSet<String>) -> bool {
    node.path.as_ref().is_some_and(|path| paths.contains(path))
        || sources_touch_paths(&node.sources, paths)
}

pub(super) fn sources_touch_paths(sources: &[GraphSourceAnchor], paths: &HashSet<String>) -> bool {
    sources.iter().any(|source| paths.contains(&source.path))
}

pub(super) fn coverage_from_file_records(
    files: &[StructuralGraphFileRecord],
) -> StructuralGraphCoverage {
    let mut coverage = StructuralGraphCoverage {
        discovered_files: files.len(),
        ..StructuralGraphCoverage::default()
    };
    let mut languages: BTreeMap<String, LanguageCoverage> = BTreeMap::new();
    for file in files {
        let language = file
            .language
            .clone()
            .unwrap_or_else(|| "unsupported".to_string());
        let entry = languages
            .entry(language.clone())
            .or_insert(LanguageCoverage {
                language,
                supported: file.language.is_some(),
                discovered_files: 0,
                indexed_files: 0,
                skipped_files: 0,
                error_files: 0,
            });
        entry.discovered_files += 1;
        match file.disposition.as_str() {
            "indexed" => {
                coverage.indexed_files += 1;
                entry.indexed_files += 1;
            }
            "error" => {
                coverage.error_files += 1;
                entry.error_files += 1;
            }
            "generated" => {
                coverage.generated_files += 1;
                coverage.skipped_files += 1;
                entry.skipped_files += 1;
            }
            "sensitive" => {
                coverage.sensitive_files += 1;
                coverage.skipped_files += 1;
                entry.skipped_files += 1;
            }
            "binary" => {
                coverage.binary_files += 1;
                coverage.skipped_files += 1;
                entry.skipped_files += 1;
            }
            _ => {
                coverage.skipped_files += 1;
                entry.skipped_files += 1;
            }
        }
    }
    coverage.languages = languages.into_values().collect();
    coverage
}

pub(super) fn deduplicate_nodes(nodes: &mut Vec<StructuralGraphNode>) {
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    nodes.dedup_by(|left, right| left.id == right.id);
    nodes.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub(super) fn deduplicate_edges(edges: &mut Vec<StructuralGraphEdge>) {
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);
    edges.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.from.cmp(&right.from))
            .then_with(|| left.to.cmp(&right.to))
    });
}

pub(super) fn deduplicate_metrics(metrics: &mut Vec<StructuralGraphMetricFact>) {
    metrics.sort_by(|left, right| left.id.cmp(&right.id));
    metrics.dedup_by(|left, right| left.id == right.id);
    metrics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
}

pub(crate) fn is_sensitive_path(path: &str) -> bool {
    crate::commands::secret_policy::is_sensitive_path(path)
}

pub(crate) fn is_vendor_path(path: &str) -> bool {
    let lower = format!("/{}/", path.to_ascii_lowercase().trim_matches('/'));
    ["/node_modules/", "/vendor/", "/.venv/", "/site-packages/"]
        .iter()
        .any(|segment| lower.contains(segment))
}

pub(crate) fn is_generated_path(path: &str) -> bool {
    let lower = format!("/{}/", path.to_ascii_lowercase().trim_matches('/'));
    [
        "/node_modules/",
        "/target/",
        "/dist/",
        "/build/",
        "/out/",
        "/coverage/",
        "/.next/",
        "/.turbo/",
    ]
    .iter()
    .any(|segment| lower.contains(segment))
        || is_vendor_path(path)
        || path.ends_with(".min.js")
        || path.ends_with(".generated.ts")
        || path.ends_with(".g.cs")
}

pub(crate) fn is_binary_path(path: &str) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "ico"
            | "pdf"
            | "zip"
            | "gz"
            | "tar"
            | "7z"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "mp3"
            | "mp4"
            | "mov"
            | "wasm"
            | "dylib"
            | "so"
            | "dll"
            | "exe"
    )
}
