use super::*;

pub(in crate::commands::history_graph) fn build_topology(
    root: &Path,
    revision: &str,
    max_nodes: Option<usize>,
) -> Result<HistoryTopology, String> {
    let revision = resolve_revision(root, revision)?;
    let output = git_bytes(root, &["ls-tree", "-r", "--name-only", "-z", &revision])?;
    let mut files = output
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| String::from_utf8_lossy(bytes).replace('\\', "/"))
        .collect::<Vec<_>>();
    files.sort();
    let total_files = files.len();
    let limit = max_nodes
        .unwrap_or(DEFAULT_GRAPH_LIMIT)
        .clamp(20, MAX_GRAPH_LIMIT);
    let path_changes = changed_path_records(root, &revision)?;
    let changed_paths = path_changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<HashSet<_>>();

    let mut directory_counts = BTreeMap::<String, usize>::new();
    for path in &files {
        let mut current = String::new();
        for component in Path::new(path)
            .components()
            .take(path.split('/').count().saturating_sub(1))
        {
            let component = component.as_os_str().to_string_lossy();
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(&component);
            *directory_counts.entry(current.clone()).or_default() += 1;
        }
    }
    let mut selected_directories = directory_counts.into_iter().collect::<Vec<_>>();
    selected_directories.sort_by(|(left_path, left_count), (right_path, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_path.cmp(right_path))
    });
    let directory_budget = (limit / 3).max(8);
    selected_directories.truncate(directory_budget);
    let directory_ids = selected_directories
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<HashSet<_>>();

    files.sort_by(|left, right| {
        changed_paths
            .contains(right)
            .cmp(&changed_paths.contains(left))
            .then_with(|| left.cmp(right))
    });
    let file_budget = limit.saturating_sub(selected_directories.len());
    files.truncate(file_budget);
    let mut nodes = Vec::with_capacity(selected_directories.len() + files.len());
    for (path, count) in &selected_directories {
        nodes.push(HistoryTopologyNode {
            id: stable_graph_id("directory", path),
            kind: "directory".to_string(),
            label: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.clone(),
            detail: format!("{count} files at this revision"),
        });
    }
    for path in &files {
        nodes.push(HistoryTopologyNode {
            id: stable_graph_id("file", path),
            kind: if changed_paths.contains(path) {
                "changed_file"
            } else {
                "file"
            }
            .to_string(),
            label: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.clone(),
            detail: if changed_paths.contains(path) {
                "changed in this revision"
            } else {
                "present at this revision"
            }
            .to_string(),
        });
    }
    let node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let mut edges = Vec::new();
    for node in &nodes {
        let Some(parent) = Path::new(&node.path).parent().and_then(Path::to_str) else {
            continue;
        };
        if parent.is_empty() || !directory_ids.contains(parent) {
            continue;
        }
        let parent_id = stable_graph_id("directory", parent);
        if node_ids.contains(parent_id.as_str()) {
            edges.push(HistoryTopologyEdge {
                id: stable_graph_id("edge", &format!("contains\0{parent_id}\0{}", node.id)),
                from: parent_id,
                to: node.id.clone(),
                kind: "contains".to_string(),
            });
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(HistoryTopology {
        schema_version: 1,
        repo_path: root.to_string_lossy().to_string(),
        revision,
        nodes,
        edges,
        changed_paths: changed_paths.into_iter().collect(),
        path_changes,
        total_files,
        truncated: total_files > file_budget,
    })
}

pub(in crate::commands::history_graph) fn changed_path_records(
    root: &Path,
    revision: &str,
) -> Result<Vec<HistoryPathChange>, String> {
    let revision = resolve_revision(root, revision)?;
    let parent_line = git_text(root, &["rev-list", "--parents", "-n", "1", &revision])?;
    let parents = parent_line.split_whitespace().skip(1).collect::<Vec<_>>();
    if let Some(parent) = parents.first() {
        return changed_path_records_between(root, parent, &revision);
    }
    let output = git_bytes(
        root,
        &[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-status",
            "-M",
            "-C",
            "--find-copies-harder",
            "-r",
            "-z",
            &revision,
        ],
    )?;
    parse_changed_path_records(&output)
}

pub(in crate::commands::history_graph) fn changed_path_records_between(
    root: &Path,
    before_revision: &str,
    after_revision: &str,
) -> Result<Vec<HistoryPathChange>, String> {
    let before_revision = resolve_revision(root, before_revision)?;
    let after_revision = resolve_revision(root, after_revision)?;
    let output = git_bytes(
        root,
        &[
            "diff",
            "--name-status",
            "-M",
            "-C",
            "--find-copies-harder",
            "-z",
            &before_revision,
            &after_revision,
        ],
    )?;
    parse_changed_path_records(&output)
}

pub(in crate::commands::history_graph) fn parse_changed_path_records(
    output: &[u8],
) -> Result<Vec<HistoryPathChange>, String> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| String::from_utf8_lossy(bytes).replace('\\', "/"))
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = fields[index].clone();
        index += 1;
        let Some(first_path) = fields.get(index).cloned() else {
            return Err("Git history change output ended before a path".to_string());
        };
        index += 1;
        let kind = status.chars().next().unwrap_or('M');
        let (path, old_path) = if matches!(kind, 'R' | 'C') {
            let Some(new_path) = fields.get(index).cloned() else {
                return Err("Git history rename/copy output ended before a destination".to_string());
            };
            index += 1;
            (new_path, Some(first_path))
        } else {
            (first_path, None)
        };
        changes.push(HistoryPathChange {
            path,
            change_kind: match kind {
                'A' => "added",
                'D' => "deleted",
                'R' => "renamed",
                'C' => "copied",
                'T' => "type_changed",
                _ => "modified",
            }
            .to_string(),
            old_path,
            additions: None,
            deletions: None,
        });
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(changes)
}

pub(in crate::commands::history_graph) fn tags_by_commit(
    root: &Path,
) -> Result<HashMap<String, Vec<String>>, String> {
    let mut tags = HashMap::<String, Vec<String>>::new();
    for tag in read_git_tags(root)? {
        tags.entry(tag.commit_sha).or_default().push(tag.name);
    }
    for values in tags.values_mut() {
        values.sort();
    }
    Ok(tags)
}

pub(crate) fn resolve_revision(root: &Path, revision: &str) -> Result<String, String> {
    let revision = revision.trim();
    if revision.is_empty() || revision.len() > 128 || revision.starts_with('-') {
        return Err("A valid Git revision is required".to_string());
    }
    git_text(
        root,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
    )
}

pub(crate) fn canonical_repo_path(repo_path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(repo_path.trim())
        .canonicalize()
        .map_err(|error| format!("Cannot resolve repository path: {error}"))?;
    if !path.is_dir() {
        return Err("Repository path is not a directory".to_string());
    }
    Ok(path)
}

pub(crate) fn git_text(root: &Path, arguments: &[&str]) -> Result<String, String> {
    String::from_utf8(git_bytes(root, arguments)?)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("Git returned invalid UTF-8: {error}"))
}

pub(in crate::commands::history_graph) fn git_bytes(
    root: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .map_err(|error| format!("Failed to run git {}: {error}", arguments.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "Git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}
