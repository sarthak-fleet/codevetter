use super::*;

pub fn load_history_revisions(
    connection: &Connection,
    repo_path: &str,
    query: Option<&str>,
    releases_only: bool,
    limit: usize,
) -> Result<HistorySearchResult, String> {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let mut statement = connection
        .prepare(
            "SELECT sha, substr(sha, 1, 8), parents_json, committed_at, author_name,
                    subject, tags_json, is_release, is_head
             FROM history_graph_revisions
             WHERE repo_path = ?1
               AND (?2 = 0 OR is_release = 1)
               AND (?3 = '' OR lower(subject) LIKE '%' || ?3 || '%'
                    OR lower(author_name) LIKE '%' || ?3 || '%'
                    OR lower(tags_json) LIKE '%' || ?3 || '%'
                    OR lower(sha) LIKE ?3 || '%')
             ORDER BY ordinal DESC
             LIMIT ?4",
        )
        .map_err(|error| format!("Prepare history query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                repo_path,
                i64::from(releases_only),
                query,
                (limit + 1) as i64
            ],
            |row| {
                let parents_json: String = row.get(2)?;
                let tags_json: String = row.get(6)?;
                Ok(HistoryRevision {
                    sha: row.get(0)?,
                    short_sha: row.get(1)?,
                    parents: serde_json::from_str(&parents_json).unwrap_or_default(),
                    committed_at: row.get(3)?,
                    author: row.get(4)?,
                    subject: row.get(5)?,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    is_release: row.get::<_, i64>(7)? != 0,
                    is_head: row.get::<_, i64>(8)? != 0,
                })
            },
        )
        .map_err(|error| format!("Query history revisions: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Read history revisions: {error}"))?;
    let truncated = rows.len() > limit;
    let mut revisions = rows;
    revisions.truncate(limit);
    Ok(HistorySearchResult {
        revisions,
        truncated,
    })
}

pub(super) fn build_timeline(root: &Path, limit: Option<usize>) -> Result<HistoryTimeline, String> {
    let limit = limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT);
    let head = git_text(root, &["rev-parse", "HEAD"])?;
    let tags = tags_by_commit(root)?;
    let ordinals = revision_ordinals(root)?;
    let total_commits = ordinals.len();
    let is_shallow = git_text(root, &["rev-parse", "--is-shallow-repository"])? == "true";
    let format = "%H%x1f%h%x1f%P%x1f%cI%x1f%an%x1f%s%x1e";
    let output = git_bytes(
        root,
        &[
            "log",
            "--topo-order",
            &format!("--max-count={limit}"),
            &format!("--format={format}"),
        ],
    )?;
    let mut revisions = String::from_utf8_lossy(&output)
        .split('\u{1e}')
        .filter_map(|record| parse_history_revision_record(record, &tags, &head))
        .collect::<Vec<_>>();
    let mut present = revisions
        .iter()
        .map(|revision| revision.sha.clone())
        .collect::<HashSet<_>>();
    let missing_releases = tags
        .iter()
        .filter(|(_, values)| values.iter().any(|tag| is_release_tag(tag)))
        .map(|(sha, _)| sha)
        .filter(|sha| !present.contains(*sha) && git_is_ancestor(root, sha, "HEAD"))
        .cloned()
        .collect::<Vec<_>>();
    for sha in missing_releases {
        let record = git_text(root, &["show", "-s", &format!("--format={format}"), &sha])?;
        if let Some(revision) = parse_history_revision_record(&record, &tags, &head) {
            present.insert(revision.sha.clone());
            revisions.push(revision);
        }
    }
    revisions.sort_by(|left, right| {
        ordinals
            .get(&left.sha)
            .cmp(&ordinals.get(&right.sha))
            .then_with(|| left.sha.cmp(&right.sha))
    });
    let release_ranges = release_ranges(&revisions, &head);
    let truncated = total_commits > revisions.len();
    Ok(HistoryTimeline {
        schema_version: 1,
        repo_path: root.to_string_lossy().to_string(),
        head,
        generated_at: Utc::now().to_rfc3339(),
        truncated,
        is_shallow,
        coverage_complete: !is_shallow && !truncated,
        release_ranges,
        total_commits,
        revisions,
    })
}

pub(super) fn parse_history_revision_record(
    record: &str,
    tags: &HashMap<String, Vec<String>>,
    head: &str,
) -> Option<HistoryRevision> {
    let fields = record
        .trim()
        .trim_end_matches('\u{1e}')
        .splitn(6, '\u{1f}')
        .collect::<Vec<_>>();
    if fields.len() != 6 || fields[0].is_empty() {
        return None;
    }
    let revision_tags = tags.get(fields[0]).cloned().unwrap_or_default();
    Some(HistoryRevision {
        sha: fields[0].to_string(),
        short_sha: fields[1].to_string(),
        parents: fields[2].split_whitespace().map(str::to_string).collect(),
        committed_at: fields[3].to_string(),
        author: fields[4].to_string(),
        subject: fields[5].to_string(),
        is_release: revision_tags.iter().any(|tag| is_release_tag(tag)),
        is_head: fields[0] == head,
        tags: revision_tags,
    })
}

pub(super) fn revision_ordinals(root: &Path) -> Result<HashMap<String, i64>, String> {
    let output = git_text(root, &["rev-list", "--topo-order", "--reverse", "HEAD"])?;
    Ok(output
        .lines()
        .filter(|sha| !sha.is_empty())
        .enumerate()
        .map(|(ordinal, sha)| (sha.to_string(), ordinal as i64))
        .collect())
}

pub(super) fn release_ranges(
    revisions: &[HistoryRevision],
    head: &str,
) -> Vec<HistoryReleaseRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut previous_release = None::<String>;
    for (index, revision) in revisions.iter().enumerate() {
        if !revision.is_release {
            continue;
        }
        let tag = revision
            .tags
            .iter()
            .find(|tag| is_release_tag(tag))
            .cloned();
        let label = tag
            .clone()
            .unwrap_or_else(|| format!("Release {}", revision.short_sha));
        ranges.push(HistoryReleaseRange {
            id: stable_graph_id(
                "release-range",
                &format!("{}\0{}", revision.sha, tag.as_deref().unwrap_or_default()),
            ),
            label,
            tag,
            from_exclusive: previous_release.clone(),
            to_inclusive: revision.sha.clone(),
            commit_shas: revisions[start..=index]
                .iter()
                .map(|commit| commit.sha.clone())
                .collect(),
            is_unreleased: false,
        });
        start = index + 1;
        previous_release = Some(revision.sha.clone());
    }
    ranges.push(HistoryReleaseRange {
        id: stable_graph_id(
            "release-range",
            &format!(
                "unreleased\0{}",
                previous_release.as_deref().unwrap_or("root")
            ),
        ),
        label: "Unreleased".to_string(),
        tag: None,
        from_exclusive: previous_release,
        to_inclusive: head.to_string(),
        commit_shas: revisions[start..]
            .iter()
            .map(|commit| commit.sha.clone())
            .collect(),
        is_unreleased: true,
    });
    ranges
}

pub(super) fn timeline_tag_fingerprint(timeline: &HistoryTimeline) -> String {
    let tag_identity = timeline
        .revisions
        .iter()
        .flat_map(|revision| {
            revision
                .tags
                .iter()
                .map(move |tag| format!("{}\0{tag}", revision.sha))
        })
        .collect::<Vec<_>>()
        .join("\0");
    stable_graph_id("tags", &tag_identity)
}

pub(crate) fn repository_tag_fingerprint(root: &Path) -> Result<String, String> {
    let mut tag_identity = tags_by_commit(root)?
        .into_iter()
        .flat_map(|(sha, tags)| {
            tags.into_iter()
                .filter(|tag| is_release_tag(tag))
                .map(move |tag| format!("{sha}\0{tag}"))
        })
        .collect::<Vec<_>>();
    tag_identity.sort();
    Ok(stable_graph_id("tags", &tag_identity.join("\0")))
}

pub(super) fn classify_history_refresh(
    previous_head: Option<&str>,
    rewritten: bool,
    engine_incompatible: bool,
    fast_forward: bool,
    tags_changed: bool,
) -> &'static str {
    if previous_head.is_none() {
        "initial"
    } else if rewritten {
        "rewritten_history"
    } else if engine_incompatible {
        "engine_repair"
    } else if fast_forward {
        "fast_forward"
    } else if tags_changed {
        "tag_metadata"
    } else {
        "no_op"
    }
}

pub(super) fn has_incompatible_history_checkpoints(
    connection: &Connection,
    repo_path: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM history_graph_checkpoints
                WHERE repo_path = ?1
                  AND (engine_id != ?2 OR engine_version != ?3 OR schema_version != ?4)
             )",
            params![
                repo_path,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Inspect history checkpoint compatibility: {error}"))
}

pub(super) fn compatible_history_checkpoint_exists(
    connection: &Connection,
    repo_path: &str,
    revision: &str,
) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM history_graph_checkpoints
                WHERE repo_path = ?1 AND revision_sha = ?2
                  AND engine_id = ?3 AND engine_version = ?4 AND schema_version = ?5
                  AND status = 'ready'
             )",
            params![
                repo_path,
                revision,
                BUNDLED_ENGINE_ID,
                BUNDLED_ENGINE_VERSION,
                STRUCTURAL_GRAPH_SCHEMA_VERSION,
            ],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| format!("Inspect history checkpoint cache: {error}"))
}

pub(super) mod git;
pub(super) mod persistence;

pub(crate) use git::canonical_repo_path;
use git::*;
