use super::*;

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
    Ok(tags_by_commit_from_records(&read_git_tags(root)?))
}

pub(in crate::commands::history_graph) fn tags_by_commit_from_records(
    records: &[GitTagRecord],
) -> HashMap<String, Vec<String>> {
    let mut tags = HashMap::<String, Vec<String>>::new();
    for tag in records {
        tags.entry(tag.commit_sha.clone())
            .or_default()
            .push(tag.name.clone());
    }
    for values in tags.values_mut() {
        values.sort();
    }
    tags
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

pub(in crate::commands::history_graph) fn git_is_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
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
