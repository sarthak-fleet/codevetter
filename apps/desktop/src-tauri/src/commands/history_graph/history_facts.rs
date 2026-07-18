use super::*;
use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

pub(super) const HISTORY_FACTS_SCHEMA_VERSION: i64 = 1;
pub(super) const HISTORY_FACT_CLASSIFICATION_VERSION: i64 = 1;
const MARKER: &[u8] = b"\x1eCODEVETTER_HISTORY_FACTS_V1";
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
const MAX_REVISIONS: usize = 100_000;
const MAX_PATHS: usize = 1_000_000;
const MAX_MAILMAP_BYTES: u64 = 1024 * 1024;
const MAX_MAILMAP_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoryFactsBatch {
    pub(super) schema_version: i64,
    pub(super) classification_version: i64,
    pub(super) git_process_count: usize,
    pub(super) mailmap_fingerprint: String,
    pub(super) facts_fingerprint: String,
    pub(super) revisions: Vec<HistoryRevisionFact>,
}

impl HistoryFactsBatch {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema_version != HISTORY_FACTS_SCHEMA_VERSION
            || self.classification_version != HISTORY_FACT_CLASSIFICATION_VERSION
            || self.git_process_count != 1
            || self.mailmap_fingerprint.is_empty()
            || self.facts_fingerprint.is_empty()
        {
            return Err("Batched history facts have an unsupported identity".to_string());
        }
        if self
            .revisions
            .iter()
            .filter(|revision| revision.is_head)
            .count()
            != 1
        {
            return Err("Batched history facts must identify exactly one HEAD".to_string());
        }
        for revision in &self.revisions {
            if revision.is_merge != (revision.parents.len() > 1)
                || revision.subject.contains('\0')
                || revision.primary.contributor_id.is_empty()
                || revision.primary.display_name.is_empty()
                || revision.tags.iter().any(String::is_empty)
                || revision.malformed_coauthor_count > 10_000
            {
                return Err("Batched history facts contain an invalid revision".to_string());
            }
            let identities = std::iter::once(&revision.primary).chain(&revision.coauthors);
            if identities
                .into_iter()
                .any(|identity| match identity.automation {
                    HistoryAutomationKind::Human
                    | HistoryAutomationKind::Automation
                    | HistoryAutomationKind::Unknown => identity.contributor_id.is_empty(),
                })
            {
                return Err("Batched history facts contain an invalid identity".to_string());
            }
            for path in &revision.paths {
                let _classification = (path.generated, path.vendored);
                if path.path.is_empty()
                    || path.old_path.as_deref() == Some("")
                    || (path.binary && (path.additions.is_some() || path.deletions.is_some()))
                    || matches!(path.status, HistoryPathStatus::Unknown)
                {
                    return Err("Batched history facts contain an invalid path".to_string());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoryRevisionFact {
    pub(super) sha: String,
    pub(super) parents: Vec<String>,
    pub(super) committed_at: String,
    pub(super) subject: String,
    pub(super) primary: HistoryIdentityFact,
    pub(super) coauthors: Vec<HistoryIdentityFact>,
    pub(super) malformed_coauthor_count: usize,
    pub(super) tags: Vec<String>,
    pub(super) paths: Vec<HistoryPathFact>,
    pub(super) is_merge: bool,
    pub(super) is_head: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct HistoryIdentityFact {
    pub(super) contributor_id: String,
    pub(super) display_name: String,
    pub(super) automation: HistoryAutomationKind,
    pub(super) alias_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum HistoryAutomationKind {
    Human,
    Automation,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HistoryPathFact {
    pub(super) path: String,
    pub(super) old_path: Option<String>,
    pub(super) status: HistoryPathStatus,
    pub(super) additions: Option<u64>,
    pub(super) deletions: Option<u64>,
    pub(super) binary: bool,
    pub(super) generated: bool,
    pub(super) vendored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum HistoryPathStatus {
    Added,
    Copied,
    Deleted,
    Modified,
    Renamed,
    TypeChanged,
    Unmerged,
    Unknown,
}

#[derive(Clone, Copy)]
struct Limits {
    output_bytes: usize,
    revisions: usize,
    paths: usize,
}

#[derive(Default)]
struct Mailmap {
    entries: Vec<MailmapEntry>,
}

struct MailmapEntry {
    canonical_name: Option<String>,
    canonical_email: String,
    alias_name: Option<String>,
    alias_email: String,
}

impl Mailmap {
    fn resolve<'a>(&'a self, name: &'a str, email: &'a str) -> (&'a str, &'a str) {
        self.entries
            .iter()
            .find(|entry| {
                entry.alias_email.eq_ignore_ascii_case(email)
                    && entry
                        .alias_name
                        .as_deref()
                        .is_none_or(|alias| alias.eq_ignore_ascii_case(name))
            })
            .map(|entry| {
                (
                    entry.canonical_name.as_deref().unwrap_or(name),
                    entry.canonical_email.as_str(),
                )
            })
            .unwrap_or((name, email))
    }

    fn alias_count(&self, name: &str, email: &str) -> usize {
        self.entries
            .iter()
            .filter(|entry| {
                entry.canonical_email.eq_ignore_ascii_case(email)
                    && entry
                        .canonical_name
                        .as_deref()
                        .is_none_or(|canonical| canonical.eq_ignore_ascii_case(name))
            })
            .count()
    }
}

fn read_mailmap(root: &Path) -> Result<(Mailmap, String), String> {
    let path = root.join(".mailmap");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((
                Mailmap::default(),
                stable_graph_id("history-mailmap-v1", "absent"),
            ));
        }
        Err(error) => return Err(format!("Inspect repository .mailmap: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Repository .mailmap must be a regular non-symlink file".to_string());
    }
    if metadata.len() > MAX_MAILMAP_BYTES {
        return Err(format!(
            "Repository .mailmap exceeds {MAX_MAILMAP_BYTES} bytes"
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("Read repository .mailmap as UTF-8: {error}"))?;
    let mut entries = contents
        .lines()
        .filter_map(parse_mailmap_entry)
        .take(MAX_MAILMAP_ENTRIES + 1)
        .collect::<Vec<_>>();
    if entries.len() > MAX_MAILMAP_ENTRIES {
        return Err(format!(
            "Repository .mailmap exceeds {MAX_MAILMAP_ENTRIES} entries"
        ));
    }
    entries.sort_by(|a, b| {
        a.alias_email
            .cmp(&b.alias_email)
            .then_with(|| a.alias_name.cmp(&b.alias_name))
    });
    Ok((
        Mailmap { entries },
        stable_graph_id("history-mailmap-v1", &contents),
    ))
}

/// Computes the same `.mailmap` identity used by the full fact reader without
/// starting the all-history Git process.
pub(super) fn current_mailmap_fingerprint(root: &Path) -> Result<String, String> {
    read_mailmap(root).map(|(_, fingerprint)| fingerprint)
}

fn parse_mailmap_entry(line: &str) -> Option<MailmapEntry> {
    let line = line.split('#').next()?.trim();
    let ranges = line
        .match_indices('<')
        .filter_map(|(start, _)| {
            line[start + 1..]
                .find('>')
                .map(|length| (start, start + 1 + length))
        })
        .take(2)
        .collect::<Vec<_>>();
    let &(canonical_start, canonical_end) = ranges.first()?;
    let canonical_name = line[..canonical_start].trim();
    let canonical_email = line[canonical_start + 1..canonical_end].trim().to_string();
    if canonical_email.is_empty() {
        return None;
    }
    let (alias_name, alias_email) = if let Some(&(alias_start, alias_end)) = ranges.get(1) {
        let name = line[canonical_end + 1..alias_start].trim();
        (
            (!name.is_empty()).then(|| name.to_string()),
            line[alias_start + 1..alias_end].trim().to_string(),
        )
    } else {
        (None, canonical_email.clone())
    };
    if alias_email.is_empty() {
        return None;
    }
    Some(MailmapEntry {
        canonical_name: (!canonical_name.is_empty()).then(|| canonical_name.to_string()),
        canonical_email,
        alias_name,
        alias_email,
    })
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            output_bytes: MAX_OUTPUT_BYTES,
            revisions: MAX_REVISIONS,
            paths: MAX_PATHS,
        }
    }
}

pub(super) fn read_all_history_facts(
    root: &Path,
    cancellation: &StructuralGraphCancellation,
) -> Result<HistoryFactsBatch, String> {
    read_history_facts(root, "HEAD", cancellation)
}

/// Reads only commits introduced after an already-indexed, exact revision.
///
/// The caller must first prove this revision is an ancestor of `HEAD`; this
/// reader deliberately does not widen a rewrite into an all-history scan.
pub(super) fn read_history_facts_since(
    root: &Path,
    from_exclusive: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<HistoryFactsBatch, String> {
    validate_full_sha(from_exclusive, None)?;
    read_history_facts(root, &format!("{from_exclusive}..HEAD"), cancellation)
}

fn read_history_facts(
    root: &Path,
    revision_range: &str,
    cancellation: &StructuralGraphCancellation,
) -> Result<HistoryFactsBatch, String> {
    if cancellation.is_cancelled() {
        return Err("History facts read cancelled".to_string());
    }
    let limits = Limits::default();
    let (mailmap, mailmap_fingerprint) = read_mailmap(root)?;
    let repository_scope = stable_graph_id("history-repository-v1", &root.to_string_lossy());
    let output = run_git_once(root, revision_range, cancellation, limits.output_bytes)?;
    let mut batch =
        parse_history_facts(&output, limits, cancellation, &mailmap, &repository_scope)?;
    batch.mailmap_fingerprint = mailmap_fingerprint;
    Ok(batch)
}

fn git_arguments(revision_range: &str) -> Vec<String> {
    let format = concat!(
        "%x1eCODEVETTER_HISTORY_FACTS_V1%x00",
        "%H%x00%P%x00%cI%x00%aN%x00%aE%x00",
        "%(decorate:prefix=,suffix=,separator=%x1f,tag=tag:%x20)%x00",
        "%s%x00",
        "%(trailers:key=Co-authored-by,valueonly,separator=%x1f)%x00"
    );
    [
        "log",
        revision_range,
        "--topo-order",
        "--reverse",
        "--decorate=full",
        "--no-abbrev",
        "--root",
        "--diff-merges=first-parent",
        "-M",
        "-C",
        "--raw",
        "--numstat",
        "-z",
    ]
    .into_iter()
    .map(str::to_string)
    .chain([format!("--format={format}")])
    .collect()
}

fn run_git_once(
    root: &Path,
    revision_range: &str,
    cancellation: &StructuralGraphCancellation,
    output_limit: usize,
) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(git_arguments(revision_range))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Start batched history Git reader: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("History Git stdout unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("History Git stderr unavailable")?;
    let output_exceeded = Arc::new(AtomicBool::new(false));
    let stdout_overflow = Arc::clone(&output_exceeded);
    let stdout_reader =
        thread::spawn(move || read_bounded_notifying(stdout, output_limit, &stdout_overflow));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, 64 * 1024));
    let status = loop {
        if output_exceeded.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(format!(
                "Batched history Git output exceeds {output_limit} bytes"
            ));
        }
        if cancellation.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err("History facts read cancelled".to_string());
        }
        match child
            .try_wait()
            .map_err(|error| format!("Poll batched history Git reader: {error}"))?
        {
            Some(status) => break status,
            None => thread::sleep(Duration::from_millis(2)),
        }
    };
    let (stdout, truncated) = stdout_reader
        .join()
        .map_err(|_| "History Git stdout reader panicked".to_string())??;
    let (stderr, _) = stderr_reader
        .join()
        .map_err(|_| "History Git stderr reader panicked".to_string())??;
    if !status.success() {
        return Err(format!(
            "Batched history Git reader failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    if truncated {
        return Err(format!(
            "Batched history Git output exceeds {output_limit} bytes"
        ));
    }
    Ok(stdout)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<(Vec<u8>, bool), String> {
    read_bounded_notifying(reader, limit, &AtomicBool::new(false))
}

fn read_bounded_notifying(
    mut reader: impl Read,
    limit: usize,
    overflow: &AtomicBool,
) -> Result<(Vec<u8>, bool), String> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("Read batched history Git stream: {error}"))?;
        if count == 0 {
            break;
        }
        let available = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(available)]);
        if count > available {
            truncated = true;
            overflow.store(true, Ordering::Release);
            break;
        }
    }
    Ok((retained, truncated))
}

fn parse_history_facts(
    output: &[u8],
    limits: Limits,
    cancellation: &StructuralGraphCancellation,
    mailmap: &Mailmap,
    repository_scope: &str,
) -> Result<HistoryFactsBatch, String> {
    if output.len() > limits.output_bytes {
        return Err("Batched history Git output exceeds its byte bound".to_string());
    }
    let fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut revisions = Vec::new();
    let mut total_paths = 0_usize;
    let mut index = 0;
    while index < fields.len() {
        if cancellation.is_cancelled() {
            return Err("History facts read cancelled".to_string());
        }
        if trim_newlines(fields[index]) != MARKER {
            if fields[index].iter().all(u8::is_ascii_whitespace) {
                index += 1;
                continue;
            }
            return Err("Malformed batched history output before revision marker".to_string());
        }
        if revisions.len() == limits.revisions {
            return Err("Batched history output exceeds its revision bound".to_string());
        }
        let header = fields
            .get(index + 1..index + 9)
            .ok_or("Batched history output ended inside a revision header")?;
        let sha = utf8(header[0], "revision SHA")?.to_string();
        validate_full_sha(&sha, None)?;
        let parents = utf8(header[1], "revision parents")?
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        for parent in &parents {
            validate_full_sha(parent, Some(sha.len()))?;
        }
        let committed_at = utf8(header[2], "commit time")?.to_string();
        if chrono::DateTime::parse_from_rfc3339(&committed_at).is_err() {
            return Err("Batched history output contains an invalid commit time".to_string());
        }
        let primary_name = utf8(header[3], "primary author name")?;
        let primary_email = utf8(header[4], "primary author email")?;
        let primary = identity_fact(
            repository_scope,
            primary_name,
            primary_email,
            mailmap.alias_count(primary_name, primary_email),
        );
        let mut tags = parse_tags(utf8(header[5], "revision decorations")?);
        tags.sort();
        tags.dedup();
        let subject = utf8(header[6], "commit subject")?.to_string();
        let (mut coauthors, malformed_coauthor_count) =
            parse_coauthors(header[7], mailmap, repository_scope)?;
        coauthors.sort();
        coauthors.dedup_by(|a, b| a.contributor_id == b.contributor_id);
        let is_head = utf8(header[5], "revision decorations")?
            .split('\u{1f}')
            .any(|item| item.trim() == "HEAD" || item.trim().starts_with("HEAD -> "));
        index += 9;
        let end = fields[index..]
            .iter()
            .position(|field| trim_newlines(field) == MARKER)
            .map(|offset| index + offset)
            .unwrap_or(fields.len());
        let paths = parse_paths(&fields[index..end], cancellation)?;
        total_paths = total_paths
            .checked_add(paths.len())
            .ok_or("Batched history path count overflowed")?;
        if total_paths > limits.paths {
            return Err("Batched history output exceeds its path bound".to_string());
        }
        revisions.push(HistoryRevisionFact {
            sha,
            is_merge: parents.len() > 1,
            parents,
            committed_at,
            subject,
            primary,
            coauthors,
            malformed_coauthor_count,
            tags,
            paths,
            is_head,
        });
        index = end;
    }
    let facts_fingerprint = history_facts_fingerprint(&revisions);
    Ok(HistoryFactsBatch {
        schema_version: HISTORY_FACTS_SCHEMA_VERSION,
        classification_version: HISTORY_FACT_CLASSIFICATION_VERSION,
        git_process_count: 1,
        mailmap_fingerprint: String::new(),
        facts_fingerprint,
        revisions,
    })
}

pub(super) fn history_facts_fingerprint(revisions: &[HistoryRevisionFact]) -> String {
    let mut identity = String::new();
    for revision in revisions {
        identity.push_str(&revision.sha);
        identity.push('\0');
        identity.push_str(&revision.parents.join(" "));
        identity.push('\0');
        identity.push_str(&revision.committed_at);
        identity.push('\0');
        identity.push_str(&revision.subject);
        identity.push('\0');
        identity.push_str(&revision.primary.contributor_id);
        identity.push('\0');
        for coauthor in &revision.coauthors {
            identity.push_str(&coauthor.contributor_id);
            identity.push('\0');
        }
        for tag in &revision.tags {
            identity.push_str(tag);
            identity.push('\0');
        }
        for path in &revision.paths {
            identity.push_str(&path.path);
            identity.push('\0');
            identity.push_str(path.old_path.as_deref().unwrap_or_default());
            identity.push('\0');
            identity.push_str(match path.status {
                HistoryPathStatus::Added => "a",
                HistoryPathStatus::Copied => "c",
                HistoryPathStatus::Deleted => "d",
                HistoryPathStatus::Modified => "m",
                HistoryPathStatus::Renamed => "r",
                HistoryPathStatus::TypeChanged => "t",
                HistoryPathStatus::Unmerged => "u",
                HistoryPathStatus::Unknown => "?",
            });
            identity.push_str(&format!(
                ":{:?}:{:?}:{}:{}:{}\0",
                path.additions, path.deletions, path.binary, path.generated, path.vendored
            ));
        }
    }
    stable_graph_id("history-facts-v1", &identity)
}

fn parse_paths(
    fields: &[&[u8]],
    cancellation: &StructuralGraphCancellation,
) -> Result<Vec<HistoryPathFact>, String> {
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        if cancellation.is_cancelled() {
            return Err("History facts read cancelled".to_string());
        }
        let field = trim_newlines(fields[index]);
        if field.is_empty() {
            index += 1;
            continue;
        }
        if field[0] == b':' {
            let token = field
                .split(|byte| byte.is_ascii_whitespace())
                .rfind(|part| !part.is_empty())
                .ok_or("Malformed raw history path record")?;
            let status = parse_status(token.first().copied().unwrap_or_default());
            let old = utf8(
                fields
                    .get(index + 1)
                    .ok_or("Raw history path missing path")?,
                "path",
            )?
            .to_string();
            let (path, old_path, consumed) = if matches!(
                status,
                HistoryPathStatus::Renamed | HistoryPathStatus::Copied
            ) {
                (
                    utf8(
                        fields
                            .get(index + 2)
                            .ok_or("Raw rename/copy missing destination")?,
                        "destination path",
                    )?
                    .to_string(),
                    Some(old),
                    3,
                )
            } else {
                (old, None, 2)
            };
            let (generated, vendored) = classify_history_path(&path);
            paths.push(HistoryPathFact {
                path,
                old_path,
                status,
                additions: None,
                deletions: None,
                binary: false,
                generated,
                vendored,
            });
            index += consumed;
            continue;
        }
        let Some((additions, deletions, inline_path)) = parse_numstat(field)? else {
            return Err("Malformed batched history path payload".to_string());
        };
        let (path, old_path, consumed) = if inline_path.is_empty() {
            (
                utf8(
                    fields
                        .get(index + 2)
                        .ok_or("Numstat rename/copy missing destination")?,
                    "numstat destination",
                )?,
                Some(utf8(
                    fields
                        .get(index + 1)
                        .ok_or("Numstat rename/copy missing source")?,
                    "numstat source",
                )?),
                3,
            )
        } else {
            (utf8(inline_path, "numstat path")?, None, 1)
        };
        let fact = paths
            .iter_mut()
            .find(|fact| fact.path == path && fact.old_path.as_deref() == old_path)
            .ok_or("Numstat record has no matching raw path record")?;
        fact.binary = additions.is_none() && deletions.is_none();
        fact.additions = additions;
        fact.deletions = deletions;
        index += consumed;
    }
    paths.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.old_path.cmp(&b.old_path))
            .then_with(|| a.status.cmp(&b.status))
    });
    Ok(paths)
}

type Numstat<'a> = Option<(Option<u64>, Option<u64>, &'a [u8])>;

fn parse_numstat(field: &[u8]) -> Result<Numstat<'_>, String> {
    let mut parts = field.splitn(3, |byte| *byte == b'\t');
    let (Some(additions), Some(deletions), Some(path)) = (parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    let count = |value: &[u8]| -> Result<Option<u64>, String> {
        if value == b"-" {
            return Ok(None);
        }
        utf8(value, "numstat count")?
            .parse::<u64>()
            .map(Some)
            .map_err(|_| "Numstat count is not an unsigned integer".to_string())
    };
    Ok(Some((count(additions)?, count(deletions)?, path)))
}

fn parse_status(status: u8) -> HistoryPathStatus {
    match status {
        b'A' => HistoryPathStatus::Added,
        b'C' => HistoryPathStatus::Copied,
        b'D' => HistoryPathStatus::Deleted,
        b'M' => HistoryPathStatus::Modified,
        b'R' => HistoryPathStatus::Renamed,
        b'T' => HistoryPathStatus::TypeChanged,
        b'U' => HistoryPathStatus::Unmerged,
        _ => HistoryPathStatus::Unknown,
    }
}

fn validate_full_sha(value: &str, expected_len: Option<usize>) -> Result<(), String> {
    let valid_len = expected_len
        .map(|length| value.len() == length)
        .unwrap_or(matches!(value.len(), 40 | 64));
    if !valid_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Batched history output contains a non-full revision SHA".to_string());
    }
    Ok(())
}

fn identity_fact(
    repository_scope: &str,
    name: &str,
    email: &str,
    alias_count: usize,
) -> HistoryIdentityFact {
    let name = name.trim();
    let email = email.trim().to_ascii_lowercase();
    let display_name = if name.is_empty() || name.contains('@') {
        "Unknown"
    } else {
        name
    };
    HistoryIdentityFact {
        contributor_id: stable_graph_id(
            "history-contributor-v1",
            &format!(
                "{}\0{}\0{}",
                repository_scope,
                display_name.to_ascii_lowercase(),
                email
            ),
        ),
        display_name: display_name.to_string(),
        automation: classify_automation(display_name, &email),
        alias_count,
    }
}

fn parse_coauthors(
    value: &[u8],
    mailmap: &Mailmap,
    repository_scope: &str,
) -> Result<(Vec<HistoryIdentityFact>, usize), String> {
    let mut identities = Vec::new();
    let mut malformed = 0;
    for trailer in utf8(value, "co-author trailers")?
        .split('\u{1f}')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let Some(open) = trailer.rfind('<') else {
            malformed += 1;
            continue;
        };
        if !trailer.ends_with('>') || open == 0 {
            malformed += 1;
            continue;
        }
        let (name, email) = mailmap.resolve(
            trailer[..open].trim(),
            trailer[open + 1..trailer.len() - 1].trim(),
        );
        identities.push(identity_fact(
            repository_scope,
            name,
            email,
            mailmap.alias_count(name, email),
        ));
    }
    Ok((identities, malformed))
}

fn parse_tags(decorations: &str) -> Vec<String> {
    decorations
        .split('\u{1f}')
        .filter_map(|item| item.trim().strip_prefix("tag: refs/tags/"))
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn classify_history_path(path: &str) -> (bool, bool) {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let parts = path.split('/').collect::<Vec<_>>();
    let vendored = parts.iter().any(|part| {
        matches!(
            *part,
            "vendor" | "vendors" | "third_party" | "node_modules" | ".pnpm"
        )
    });
    let file = parts.last().copied().unwrap_or_default();
    let generated = parts
        .iter()
        .any(|part| matches!(*part, "generated" | "gen" | "dist" | "build" | "coverage"))
        || matches!(
            file,
            "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock" | "cargo.lock"
        )
        || file.ends_with(".generated.rs")
        || file.ends_with(".generated.ts")
        || file.ends_with(".min.js")
        || file.ends_with(".min.css")
        || file.ends_with(".map");
    (generated, vendored)
}

pub(super) fn classify_automation(name: &str, email: &str) -> HistoryAutomationKind {
    let identity = format!("{name} {email}").to_ascii_lowercase();
    if identity.trim().is_empty() {
        HistoryAutomationKind::Unknown
    } else if identity.contains("[bot]")
        || identity.contains("dependabot")
        || identity.contains("renovate")
        || identity.contains("github-actions")
        || identity.contains("automation@")
        || identity.contains("bot@")
    {
        HistoryAutomationKind::Automation
    } else {
        HistoryAutomationKind::Human
    }
}

fn trim_newlines(mut value: &[u8]) -> &[u8] {
    while matches!(value.first(), Some(b'\n' | b'\r')) {
        value = &value[1..];
    }
    value
}

fn utf8<'a>(value: &'a [u8], label: &str) -> Result<&'a str, String> {
    std::str::from_utf8(value).map_err(|_| format!("Batched history {label} is not UTF-8"))
}

#[cfg(test)]
#[path = "history_facts_tests.rs"]
mod tests;
