use super::*;

pub(super) fn extract_blob(path: &str, bytes: &[u8], max_bytes: usize) -> FileContribution {
    let normalized_path = path.replace('\\', "/");
    let relative_path = Path::new(&normalized_path);
    let language = SupportedLanguage::from_path(relative_path);
    if is_sensitive_path(&normalized_path) {
        return skipped_contribution(
            stable_graph_id("sensitive_path", &normalized_path),
            language,
            FileDisposition::Sensitive,
        );
    }
    if is_binary_path(&normalized_path) {
        return skipped_contribution(normalized_path, language, FileDisposition::Binary);
    }
    if is_generated_path(&normalized_path) {
        return metadata_file_contribution(normalized_path, language, FileDisposition::Generated);
    }
    if bytes.len() > max_bytes {
        return metadata_file_contribution(normalized_path, language, FileDisposition::TooLarge);
    }
    let Ok(source) = std::str::from_utf8(bytes) else {
        return skipped_contribution(normalized_path, language, FileDisposition::Binary);
    };
    if let Some(language) = language {
        return extract_source(&normalized_path, language, source);
    }
    if !is_metadata_text_path(relative_path) {
        return metadata_file_contribution(normalized_path, None, FileDisposition::Unsupported);
    }
    let file_id = stable_graph_id("file", &normalized_path);
    let mut nodes = vec![StructuralGraphNode {
        id: file_id.clone(),
        kind: "file".to_string(),
        label: normalized_path.clone(),
        qualified_name: Some(normalized_path.clone()),
        path: Some(normalized_path.clone()),
        detail: Some("historical metadata-indexed text file".to_string()),
        language: None,
        community_id: None,
        trust: GraphTrust::Extracted,
        origin: GraphOrigin::Metadata,
        sources: vec![GraphSourceAnchor::path(&normalized_path)],
    }];
    let mut edges = Vec::new();
    extract_metadata_signals(
        &normalized_path,
        source,
        &file_id,
        None,
        &mut nodes,
        &mut edges,
    );
    attach_metadata_to_syntax_owners(&nodes, &mut edges);
    FileContribution {
        path: normalized_path,
        language: None,
        content_hash: Some(stable_graph_id("content", source)),
        byte_size: bytes.len() as u64,
        nodes,
        edges,
        metrics: Vec::new(),
        diagnostics: Vec::new(),
        disposition: FileDisposition::Indexed,
    }
}

pub(super) fn discover_paths(root: &Path) -> Result<Vec<PathBuf>, StructuralGraphError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-co", "--exclude-standard", "-z"])
        .output()
        .map_err(|error| {
            StructuralGraphError::Io(format!("Failed to discover Git files: {error}"))
        })?;
    if !output.status.success() {
        return Err(StructuralGraphError::InvalidRepository(format!(
            "Git file discovery failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| PathBuf::from(String::from_utf8_lossy(bytes).into_owned()))
        .collect())
}

pub(super) fn extract_path(root: &Path, relative_path: &Path, max_bytes: u64) -> FileContribution {
    let normalized_path = relative_path.to_string_lossy().replace('\\', "/");
    let language = SupportedLanguage::from_path(relative_path);
    if is_sensitive_path(&normalized_path) {
        return skipped_contribution(
            stable_graph_id("sensitive_path", &normalized_path),
            language,
            FileDisposition::Sensitive,
        );
    }
    if is_binary_path(&normalized_path) {
        return skipped_contribution(normalized_path, language, FileDisposition::Binary);
    }
    if is_generated_path(&normalized_path) {
        return metadata_file_contribution(normalized_path, language, FileDisposition::Generated);
    }
    let Some(language) = language else {
        return extract_metadata_path(root, relative_path, &normalized_path, max_bytes);
    };

    let absolute_path = root.join(relative_path);
    let metadata = match std::fs::metadata(&absolute_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return FileContribution {
                path: normalized_path.clone(),
                language: Some(language.name().to_string()),
                content_hash: None,
                byte_size: 0,
                nodes: Vec::new(),
                edges: Vec::new(),
                metrics: Vec::new(),
                diagnostics: vec![StructuralGraphDiagnostic {
                    severity: "warning".to_string(),
                    code: "file_metadata_failed".to_string(),
                    message: error.to_string(),
                    path: Some(normalized_path),
                    language: Some(language.name().to_string()),
                }],
                disposition: FileDisposition::Error,
            };
        }
    };
    if metadata.len() > max_bytes {
        return metadata_file_contribution(
            normalized_path,
            Some(language),
            FileDisposition::TooLarge,
        );
    }
    let bytes = match std::fs::read(&absolute_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return FileContribution {
                path: normalized_path.clone(),
                language: Some(language.name().to_string()),
                content_hash: None,
                byte_size: metadata.len(),
                nodes: Vec::new(),
                edges: Vec::new(),
                metrics: Vec::new(),
                diagnostics: vec![StructuralGraphDiagnostic {
                    severity: "warning".to_string(),
                    code: "file_read_failed".to_string(),
                    message: error.to_string(),
                    path: Some(normalized_path),
                    language: Some(language.name().to_string()),
                }],
                disposition: FileDisposition::Error,
            };
        }
    };
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            return skipped_contribution(normalized_path, Some(language), FileDisposition::Binary);
        }
    };
    extract_source(&normalized_path, language, &source)
}

pub(super) fn extract_metadata_path(
    root: &Path,
    relative_path: &Path,
    normalized_path: &str,
    max_bytes: u64,
) -> FileContribution {
    if !is_metadata_text_path(relative_path) {
        return metadata_file_contribution(
            normalized_path.to_string(),
            None,
            FileDisposition::Unsupported,
        );
    }
    let absolute_path = root.join(relative_path);
    let metadata = match std::fs::metadata(&absolute_path) {
        Ok(metadata) if metadata.len() <= max_bytes => metadata,
        Ok(_) => {
            return metadata_file_contribution(
                normalized_path.to_string(),
                None,
                FileDisposition::TooLarge,
            )
        }
        Err(error) => {
            return metadata_read_error(normalized_path, "file_metadata_failed", error.to_string())
        }
    };
    let bytes = match std::fs::read(&absolute_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return metadata_read_error(normalized_path, "file_read_failed", error.to_string())
        }
    };
    let source = match String::from_utf8(bytes) {
        Ok(source) => source,
        Err(_) => {
            return skipped_contribution(normalized_path.to_string(), None, FileDisposition::Binary)
        }
    };
    let file_id = stable_graph_id("file", normalized_path);
    let mut nodes = vec![StructuralGraphNode {
        id: file_id.clone(),
        kind: "file".to_string(),
        label: normalized_path.to_string(),
        qualified_name: Some(normalized_path.to_string()),
        path: Some(normalized_path.to_string()),
        detail: Some("metadata-indexed text file".to_string()),
        language: None,
        community_id: None,
        trust: GraphTrust::Extracted,
        origin: GraphOrigin::Metadata,
        sources: vec![GraphSourceAnchor::path(normalized_path)],
    }];
    let mut edges = Vec::new();
    extract_metadata_signals(
        normalized_path,
        &source,
        &file_id,
        None,
        &mut nodes,
        &mut edges,
    );
    attach_metadata_to_syntax_owners(&nodes, &mut edges);
    FileContribution {
        path: normalized_path.to_string(),
        language: None,
        content_hash: Some(stable_graph_id("content", &source)),
        byte_size: metadata.len(),
        nodes,
        edges,
        metrics: Vec::new(),
        diagnostics: Vec::new(),
        disposition: FileDisposition::Indexed,
    }
}

fn metadata_read_error(path: &str, code: &str, message: String) -> FileContribution {
    FileContribution {
        path: path.to_string(),
        language: None,
        content_hash: None,
        byte_size: 0,
        nodes: Vec::new(),
        edges: Vec::new(),
        metrics: Vec::new(),
        diagnostics: vec![StructuralGraphDiagnostic {
            severity: "warning".to_string(),
            code: code.to_string(),
            message,
            path: Some(path.to_string()),
            language: None,
        }],
        disposition: FileDisposition::Error,
    }
}

fn is_metadata_text_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "md" | "mdx"
            | "sql"
            | "json"
            | "jsonc"
            | "toml"
            | "yaml"
            | "yml"
            | "ini"
            | "sh"
            | "proto"
            | "graphql"
            | "gql"
    ) || matches!(
        name.as_str(),
        "dockerfile" | "makefile" | "justfile" | "procfile"
    )
}
