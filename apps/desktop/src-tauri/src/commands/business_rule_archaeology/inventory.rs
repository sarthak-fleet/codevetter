use super::contracts::{
    validate_revision_sha, ArchaeologyCoverage, ArchaeologyCoverageState,
    ArchaeologyRepositoryIdentity, ArchaeologySourceClassification, ArchaeologySourceUnitIdentity,
    ARCHAEOLOGY_SCHEMA_VERSION,
};
use crate::commands::secret_policy::is_sensitive_path;
use crate::commands::structural_graph::extract::{
    is_binary_path, is_generated_path, is_vendor_path,
};
use crate::commands::structural_graph::language::SupportedLanguage;
use crate::commands::structural_graph::types::StructuralGraphCancellation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;

// v2 retains the inventory-time coverage reasons after parsing. That durable
// proof is required before a later revision may reuse an unchanged manifest
// row without rereading its Git blob.
pub const INVENTORY_POLICY_VERSION: &str = "archaeology-inventory-v2";

#[derive(Debug, Clone, Copy)]
pub struct ArchaeologyInventoryLimits {
    pub max_files: usize,
    pub max_path_bytes: usize,
    pub max_source_unit_bytes: u64,
    pub max_candidate_scan_bytes: usize,
    pub max_candidates_per_unit: usize,
}

impl Default for ArchaeologyInventoryLimits {
    fn default() -> Self {
        Self {
            max_files: 250_000,
            max_path_bytes: 64 * 1024 * 1024,
            max_source_unit_bytes: 16 * 1024 * 1024,
            max_candidate_scan_bytes: 2 * 1024 * 1024,
            max_candidates_per_unit: 128,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchaeologyIncludeCandidate {
    pub kind: String,
    pub target: String,
    pub line: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchaeologyInventoryUnit {
    pub identity: ArchaeologySourceUnitIdentity,
    pub classification: ArchaeologySourceClassification,
    pub language: String,
    pub dialect: Option<String>,
    pub byte_count: u64,
    pub line_count: u64,
    pub include_candidates: Vec<ArchaeologyIncludeCandidate>,
    pub coverage_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchaeologyRepositoryInventory {
    pub schema_version: u32,
    pub policy_version: String,
    pub repository: ArchaeologyRepositoryIdentity,
    pub config_identity: String,
    pub source_units: Vec<ArchaeologyInventoryUnit>,
    pub coverage: ArchaeologyCoverage,
}

impl ArchaeologyRepositoryInventory {
    pub(crate) fn summary(&self) -> ArchaeologyInventorySummary {
        ArchaeologyInventorySummary {
            schema_version: self.schema_version,
            policy_version: self.policy_version.clone(),
            repository: self.repository.clone(),
            config_identity: self.config_identity.clone(),
            coverage: self.coverage.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchaeologyInventorySummary {
    pub schema_version: u32,
    pub policy_version: String,
    pub repository: ArchaeologyRepositoryIdentity,
    pub config_identity: String,
    pub coverage: ArchaeologyCoverage,
}

#[cfg(test)]
fn inventory_repository(
    root: &Path,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
) -> Result<ArchaeologyRepositoryInventory, String> {
    let mut source_units = Vec::new();
    let summary = inventory_repository_streaming_observed(
        root,
        cancellation,
        limits,
        &mut |unit| {
            source_units.push(unit);
            Ok(())
        },
        &mut |_| {},
    )?;
    Ok(ArchaeologyRepositoryInventory {
        schema_version: summary.schema_version,
        policy_version: summary.policy_version,
        repository: summary.repository,
        config_identity: summary.config_identity,
        source_units,
        coverage: summary.coverage,
    })
}

pub fn inventory_repository_streaming(
    root: &Path,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    emit: &mut impl FnMut(ArchaeologyInventoryUnit) -> Result<(), String>,
) -> Result<ArchaeologyInventorySummary, String> {
    inventory_repository_streaming_observed(root, cancellation, limits, emit, &mut |_| {})
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InventoryCheckpoint {
    PathDiscovered,
    HashChunkRead,
}

#[cfg(test)]
fn inventory_repository_observed(
    root: &Path,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    observer: &mut impl FnMut(InventoryCheckpoint),
) -> Result<ArchaeologyRepositoryInventory, String> {
    let mut source_units = Vec::new();
    let summary = inventory_repository_streaming_observed(
        root,
        cancellation,
        limits,
        &mut |unit| {
            source_units.push(unit);
            Ok(())
        },
        observer,
    )?;
    Ok(ArchaeologyRepositoryInventory {
        schema_version: summary.schema_version,
        policy_version: summary.policy_version,
        repository: summary.repository,
        config_identity: summary.config_identity,
        source_units,
        coverage: summary.coverage,
    })
}

fn inventory_repository_streaming_observed(
    root: &Path,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    emit: &mut impl FnMut(ArchaeologyInventoryUnit) -> Result<(), String>,
    observer: &mut impl FnMut(InventoryCheckpoint),
) -> Result<ArchaeologyInventorySummary, String> {
    check_cancelled(cancellation)?;
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("Resolve archaeology repository: {error}"))?;
    if !canonical.is_dir() {
        return Err("Archaeology repository must be a directory".to_string());
    }
    let revision_sha = git_head(&canonical)?;
    let repository_id = opaque_id(
        "archaeology-repository",
        canonical.to_string_lossy().as_bytes(),
    );
    let mut discovered_bytes = 0_u64;
    let mut indexed_bytes = 0_u64;
    let mut discovered_source_units = 0_u64;
    let mut indexed_source_units = 0_u64;
    let mut reasons = BTreeSet::new();
    let mut config_digest = Sha256::new();
    config_digest.update(INVENTORY_POLICY_VERSION.as_bytes());
    let mut blobs = GitBlobBatch::start(&canonical)?;
    discover_tree(
        &canonical,
        &revision_sha,
        cancellation,
        limits,
        &mut |entry| {
            observer(InventoryCheckpoint::PathDiscovered);
            check_cancelled(cancellation)?;
            if entry
                .relative_path
                .as_deref()
                .is_some_and(is_inventory_config_path)
            {
                update_config_identity(&mut config_digest, &entry);
            }
            let unit = match entry.relative_path.as_deref() {
                Some(relative_path) => inventory_tree_unit(
                    &repository_id,
                    &revision_sha,
                    &entry,
                    relative_path,
                    &mut blobs,
                    cancellation,
                    limits,
                    observer,
                )?,
                None => opaque_tree_unit(
                    &repository_id,
                    &revision_sha,
                    &entry,
                    None,
                    "non_utf8_path_excluded",
                ),
            };
            discovered_source_units = discovered_source_units.saturating_add(1);
            discovered_bytes = discovered_bytes.saturating_add(unit.byte_count);
            if unit.identity.content_hash.is_some() {
                indexed_source_units = indexed_source_units.saturating_add(1);
                indexed_bytes = indexed_bytes.saturating_add(unit.byte_count);
            }
            reasons.extend(unit.coverage_reasons.iter().cloned());
            emit(unit)
        },
    )?;
    check_cancelled(cancellation)?;
    blobs.finish()?;
    check_cancelled(cancellation)?;
    if git_head(&canonical)? != revision_sha {
        return Err("Archaeology HEAD changed during inventory".to_string());
    }
    let config_identity = format!("sha256:{}", hex(&config_digest.finalize()));
    let repository = ArchaeologyRepositoryIdentity {
        repository_id,
        revision_sha: revision_sha.clone(),
        source_identity: opaque_id(
            "archaeology-source",
            format!("{revision_sha}\0{config_identity}").as_bytes(),
        ),
    };
    let state = if reasons.is_empty() {
        ArchaeologyCoverageState::Complete
    } else {
        ArchaeologyCoverageState::Partial
    };
    Ok(ArchaeologyInventorySummary {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        policy_version: INVENTORY_POLICY_VERSION.to_string(),
        repository,
        config_identity,
        coverage: ArchaeologyCoverage {
            state: state.clone(),
            parser_coverage: ArchaeologyCoverageState::Unavailable,
            repository_coverage: state,
            temporal_coverage: ArchaeologyCoverageState::Unavailable,
            discovered_source_units,
            indexed_source_units,
            discovered_bytes,
            indexed_bytes,
            reasons: reasons.into_iter().collect(),
        },
    })
}

struct GitTreeEntry {
    mode: String,
    object_type: String,
    object_id: String,
    size: Option<u64>,
    identity: String,
    relative_path: Option<String>,
}

fn discover_tree(
    root: &Path,
    revision_sha: &str,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    emit: &mut impl FnMut(GitTreeEntry) -> Result<(), String>,
) -> Result<(), String> {
    discover_tree_paths(root, revision_sha, None, cancellation, limits, emit)
}

fn discover_tree_paths(
    root: &Path,
    revision_sha: &str,
    paths: Option<&[String]>,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    emit: &mut impl FnMut(GitTreeEntry) -> Result<(), String>,
) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(root)
        .args(["ls-tree", "-rlz", "--full-tree", revision_sha]);
    if let Some(paths) = paths {
        command.arg("--").args(paths);
    }
    let child = command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Start archaeology Git inventory: {error}"))?;
    let mut child = ManagedChild::new(child);
    let stdout = child
        .child
        .stdout
        .take()
        .ok_or("Archaeology Git stdout unavailable")?;
    let mut reader = BufReader::new(stdout);
    let mut path_bytes = 0_usize;
    let mut path_count = 0_usize;
    loop {
        if cancellation.is_cancelled() {
            return Err("Archaeology inventory cancelled".to_string());
        }
        let mut encoded = Vec::new();
        let record_bound = limits
            .max_path_bytes
            .saturating_sub(path_bytes)
            .saturating_add(256);
        let count = reader
            .by_ref()
            .take(record_bound as u64)
            .read_until(0, &mut encoded)
            .map_err(|error| format!("Read archaeology Git inventory: {error}"))?;
        if count == 0 {
            break;
        }
        if encoded.pop() != Some(0) {
            return Err("Archaeology Git tree record exceeds its path bound".to_string());
        }
        let entry = parse_tree_record(&encoded)?;
        path_bytes = path_bytes
            .checked_add(entry.path.len())
            .ok_or("Archaeology path bytes overflowed")?;
        if path_bytes > limits.max_path_bytes || path_count == limits.max_files {
            return Err("Archaeology repository inventory exceeds its path bound".to_string());
        }
        path_count += 1;
        let relative_path = String::from_utf8(entry.path.clone())
            .ok()
            .map(|path| path.replace('\\', "/"));
        if let Some(path) = relative_path.as_deref() {
            validate_relative_path(path)?;
        }
        emit(GitTreeEntry {
            mode: entry.mode,
            object_type: entry.object_type,
            object_id: entry.object_id,
            size: entry.size,
            identity: opaque_id("archaeology-git-path", &entry.path),
            relative_path,
        })?;
    }
    let (status, stderr) = child.finish()?;
    if !status.success() {
        return Err(format!(
            "Archaeology Git inventory failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    Ok(())
}

struct ParsedTreeRecord {
    mode: String,
    object_type: String,
    object_id: String,
    size: Option<u64>,
    path: Vec<u8>,
}

fn parse_tree_record(record: &[u8]) -> Result<ParsedTreeRecord, String> {
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or("Archaeology Git tree record is missing its path delimiter")?;
    let header = std::str::from_utf8(&record[..tab])
        .map_err(|_| "Archaeology Git tree header is not UTF-8")?;
    let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
    let valid_mode_type = matches!(
        (fields.first().copied(), fields.get(1).copied()),
        (Some("100644" | "100755" | "120000"), Some("blob")) | (Some("160000"), Some("commit"))
    );
    if fields.len() != 4
        || fields[0].len() != 6
        || !fields[0].bytes().all(|byte| matches!(byte, b'0'..=b'7'))
        || !valid_mode_type
        || validate_object_id(fields[2]).is_err()
        || record[tab + 1..].is_empty()
    {
        return Err("Archaeology Git tree record is invalid".to_string());
    }
    let size = if fields[3] == "-" {
        None
    } else {
        Some(
            fields[3]
                .parse::<u64>()
                .map_err(|_| "Archaeology Git tree size is invalid")?,
        )
    };
    if (fields[1] == "blob") != size.is_some() {
        return Err("Archaeology Git tree type and size disagree".to_string());
    }
    Ok(ParsedTreeRecord {
        mode: fields[0].to_string(),
        object_type: fields[1].to_string(),
        object_id: fields[2].to_string(),
        size,
        path: record[tab + 1..].to_vec(),
    })
}

struct ManagedChild {
    child: Child,
    finished: bool,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl ManagedChild {
    fn new(mut child: Child) -> Self {
        let stderr = child.stderr.take().map(|mut stderr| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.read_to_end(&mut bytes);
                bytes
            })
        });
        Self {
            child,
            finished: false,
            stderr,
        }
    }

    fn finish(mut self) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
        let status = self
            .child
            .wait()
            .map_err(|error| format!("Wait for archaeology Git inventory: {error}"))?;
        self.finished = true;
        let stderr = self
            .stderr
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default();
        Ok((status, stderr))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

fn check_cancelled(cancellation: &StructuralGraphCancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Archaeology inventory cancelled".to_string())
    } else {
        Ok(())
    }
}

struct GitBlobBatch {
    child: ManagedChild,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl GitBlobBatch {
    fn start(root: &Path) -> Result<Self, String> {
        let child = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "--batch"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Start archaeology Git blob stream: {error}"))?;
        let mut child = ManagedChild::new(child);
        let stdin = child
            .child
            .stdin
            .take()
            .ok_or("Archaeology Git blob stdin unavailable")?;
        let stdout = child
            .child
            .stdout
            .take()
            .ok_or("Archaeology Git blob stdout unavailable")?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
        })
    }

    fn read_blob(
        &mut self,
        object_id: &str,
        expected_size: u64,
        cancellation: &StructuralGraphCancellation,
        observer: &mut impl FnMut(InventoryCheckpoint),
    ) -> Result<Vec<u8>, String> {
        check_cancelled(cancellation)?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or("Archaeology Git blob stream is closed")?;
        stdin
            .write_all(format!("{object_id}\n").as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("Request archaeology Git blob: {error}"))?;
        let mut header = Vec::new();
        let count = self
            .stdout
            .by_ref()
            .take(257)
            .read_until(b'\n', &mut header)
            .map_err(|error| format!("Read archaeology Git blob header: {error}"))?;
        if count == 0 || count > 256 || header.pop() != Some(b'\n') {
            return Err("Archaeology Git blob header is invalid".to_string());
        }
        parse_batch_header(&header, object_id, expected_size)?;
        let size = usize::try_from(expected_size)
            .map_err(|_| "Archaeology Git blob size exceeds this platform")?;
        let mut content = vec![0_u8; size];
        for chunk in content.chunks_mut(64 * 1024) {
            check_cancelled(cancellation)?;
            self.stdout
                .read_exact(chunk)
                .map_err(|error| format!("Read archaeology Git blob: {error}"))?;
            observer(InventoryCheckpoint::HashChunkRead);
            check_cancelled(cancellation)?;
        }
        check_cancelled(cancellation)?;
        let mut delimiter = [0_u8; 1];
        self.stdout
            .read_exact(&mut delimiter)
            .map_err(|error| format!("Read archaeology Git blob delimiter: {error}"))?;
        if delimiter != *b"\n" {
            return Err("Archaeology Git blob delimiter is invalid".to_string());
        }
        Ok(content)
    }

    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let (status, stderr) = self.child.finish()?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "Archaeology Git blob stream failed: {}",
                String::from_utf8_lossy(&stderr).trim()
            ))
        }
    }
}

fn parse_batch_header(header: &[u8], object_id: &str, expected_size: u64) -> Result<(), String> {
    let header =
        std::str::from_utf8(header).map_err(|_| "Archaeology Git blob header is not UTF-8")?;
    let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
    let size = fields.get(2).and_then(|size| size.parse::<u64>().ok());
    if fields.len() != 3
        || fields[0] != object_id
        || fields[1] != "blob"
        || size != Some(expected_size)
    {
        Err("Archaeology Git blob identity, type, or size disagrees with the tree".to_string())
    } else {
        Ok(())
    }
}

fn opaque_tree_unit(
    repository_id: &str,
    revision_sha: &str,
    entry: &GitTreeEntry,
    relative_path: Option<&str>,
    reason: &str,
) -> ArchaeologyInventoryUnit {
    let protected = relative_path.is_some_and(is_sensitive_path);
    let path_key = relative_path.unwrap_or(&entry.identity);
    let path_identity = opaque_id(
        "archaeology-path",
        format!("{repository_id}\0{path_key}").as_bytes(),
    );
    let change_identity = source_change_identity(repository_id, &path_identity, &entry.object_id);
    ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id: opaque_id(
                "archaeology-source-unit",
                format!(
                    "{repository_id}\0{revision_sha}\0{path_identity}\0{}",
                    entry.object_id
                )
                .as_bytes(),
            ),
            repository_id: repository_id.to_string(),
            revision_sha: revision_sha.to_string(),
            path_identity,
            relative_path: relative_path.filter(|_| !protected).map(str::to_string),
            content_hash: None,
            hash_algorithm: None,
            change_identity: Some(change_identity),
        },
        classification: if protected {
            ArchaeologySourceClassification::Protected
        } else {
            ArchaeologySourceClassification::Opaque
        },
        language: "unknown".to_string(),
        dialect: None,
        byte_count: entry.size.unwrap_or(0),
        line_count: 0,
        include_candidates: Vec::new(),
        coverage_reasons: vec![if protected {
            "protected_source_content_excluded".to_string()
        } else {
            reason.to_string()
        }],
    }
}

fn inventory_tree_unit(
    repository_id: &str,
    revision_sha: &str,
    entry: &GitTreeEntry,
    relative_path: &str,
    blobs: &mut GitBlobBatch,
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
    observer: &mut impl FnMut(InventoryCheckpoint),
) -> Result<ArchaeologyInventoryUnit, String> {
    let path_identity = opaque_id(
        "archaeology-path",
        format!("{repository_id}\0{relative_path}").as_bytes(),
    );
    let protected = is_sensitive_path(relative_path);
    let vendor = is_vendor_path(relative_path);
    let generated = !vendor && is_generated_path(relative_path);
    let regular = entry.object_type == "blob" && matches!(entry.mode.as_str(), "100644" | "100755");
    let opaque = !regular || is_binary_path(relative_path);
    let classification = if protected {
        ArchaeologySourceClassification::Protected
    } else if vendor {
        ArchaeologySourceClassification::Vendor
    } else if generated {
        ArchaeologySourceClassification::Generated
    } else if opaque {
        ArchaeologySourceClassification::Opaque
    } else {
        ArchaeologySourceClassification::Source
    };
    let mut coverage_reasons = Vec::new();
    let byte_count = entry.size.unwrap_or(0);
    let readable = !protected && !opaque && byte_count <= limits.max_source_unit_bytes;
    if protected {
        coverage_reasons.push("protected_source_content_excluded".to_string());
    } else if opaque {
        coverage_reasons.push("non_regular_or_binary_source_excluded".to_string());
    } else if byte_count > limits.max_source_unit_bytes {
        coverage_reasons.push("source_unit_exceeds_byte_bound".to_string());
    }
    let content = readable
        .then(|| blobs.read_blob(&entry.object_id, byte_count, cancellation, observer))
        .transpose()?;
    if content
        .as_deref()
        .is_some_and(|content| content.contains(&0) || std::str::from_utf8(content).is_err())
    {
        return Ok(opaque_tree_unit(
            repository_id,
            revision_sha,
            entry,
            Some(relative_path),
            "non_utf8_or_nul_source_excluded",
        ));
    }
    let sample = content
        .as_deref()
        .map(|content| &content[..content.len().min(limits.max_candidate_scan_bytes)]);
    let (language, dialect) = detect_language(Path::new(relative_path), sample);
    let (include_candidates, candidate_limit_reached) = sample
        .map(|bytes| find_include_candidates(bytes, limits.max_candidates_per_unit))
        .unwrap_or_default();
    if content
        .as_ref()
        .is_some_and(|content| content.len() > limits.max_candidate_scan_bytes)
    {
        coverage_reasons.push("include_candidate_scan_byte_bound_reached".to_string());
    }
    if candidate_limit_reached {
        coverage_reasons.push("include_candidate_count_bound_reached".to_string());
    }
    let content_hash = content
        .as_ref()
        .map(|content| hex(&Sha256::digest(content)));
    let source_unit_id = opaque_id(
        "archaeology-source-unit",
        format!(
            "{repository_id}\0{revision_sha}\0{path_identity}\0{}",
            content_hash.as_deref().unwrap_or("content-unavailable")
        )
        .as_bytes(),
    );
    let change_identity = source_change_identity(repository_id, &path_identity, &entry.object_id);
    Ok(ArchaeologyInventoryUnit {
        identity: ArchaeologySourceUnitIdentity {
            source_unit_id,
            repository_id: repository_id.to_string(),
            revision_sha: revision_sha.to_string(),
            path_identity,
            relative_path: (!protected).then(|| relative_path.to_string()),
            content_hash,
            hash_algorithm: content.as_ref().map(|_| "sha256".to_string()),
            change_identity: Some(change_identity),
        },
        classification,
        language,
        dialect,
        byte_count,
        line_count: content
            .as_ref()
            .map(|content| line_count(content))
            .unwrap_or(0),
        include_candidates,
        coverage_reasons,
    })
}

fn line_count(content: &[u8]) -> u64 {
    content.iter().filter(|byte| **byte == b'\n').count() as u64
        + u64::from(content.last().is_some_and(|byte| *byte != b'\n'))
}

fn detect_language(path: &Path, sample: Option<&[u8]>) -> (String, Option<String>) {
    if let Some(language) = SupportedLanguage::from_path(path) {
        return (language.name().to_string(), None);
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = sample
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(extension.as_str(), "cbl" | "cob" | "cobol" | "cpy") {
        let dialect = if extension == "cpy" {
            "copybook"
        } else if text.contains(">>SOURCE FORMAT FREE") {
            "free"
        } else if text
            .lines()
            .any(|line| line.len() >= 7 && line.as_bytes()[6] == b'*')
            || text.lines().any(|line| {
                line.get(..6).is_some_and(|prefix| {
                    prefix
                        .chars()
                        .all(|character| character.is_ascii_digit() || character == ' ')
                })
            })
        {
            "fixed"
        } else {
            "ambiguous"
        };
        return ("cobol".to_string(), Some(dialect.to_string()));
    }
    if matches!(extension.as_str(), "asm" | "s" | "hlasm") {
        let hlasm_section = text.lines().any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            words
                .get(1)
                .is_some_and(|word| matches!(*word, "CSECT" | "DSECT"))
        });
        let hlasm_specific = [" USING ", " MVC ", " CLC ", " R14", " R15"]
            .iter()
            .any(|marker| text.contains(marker));
        let gas_global = text
            .lines()
            .any(|line| matches!(line.split_whitespace().next(), Some(".GLOBL" | ".GLOBAL")));
        let gas_att = text.contains('%') && (text.contains('$') || text.contains("(%"));
        let hlasm = hlasm_section && hlasm_specific;
        let gas = gas_global && gas_att;
        let nasm =
            text.contains("SECTION .TEXT") || text.contains("GLOBAL ") || text.contains("[RAX]");
        let dialect = match (hlasm, gas, nasm) {
            (true, false, false) => "hlasm",
            (false, true, false) => "gas-att",
            (false, false, true) => "nasm",
            _ => "ambiguous",
        };
        return ("assembly".to_string(), Some(dialect.to_string()));
    }
    ("unknown".to_string(), None)
}

fn find_include_candidates(bytes: &[u8], limit: usize) -> (Vec<ArchaeologyIncludeCandidate>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut candidates = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let logical = line.get(6..).filter(|_| {
            line.as_bytes().get(..6).is_some_and(|prefix| {
                prefix
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || *byte == b' ')
            })
        });
        let trimmed = logical.unwrap_or(line).trim();
        let upper = trimmed.to_ascii_uppercase();
        let candidate = if let Some(rest) = upper.strip_prefix("COPY ") {
            token(rest).map(|target| ("copybook", target))
        } else if let Some(rest) = trimmed.strip_prefix(".include ") {
            token(rest).map(|target| ("include", target))
        } else if let Some(rest) = trimmed.strip_prefix("%include ") {
            token(rest).map(|target| ("include", target))
        } else if upper == "MACRO" || upper.ends_with(" MACRO") {
            Some((
                "macro",
                trimmed
                    .split_whitespace()
                    .next()
                    .unwrap_or("anonymous")
                    .to_string(),
            ))
        } else {
            None
        };
        if let Some((kind, target)) = candidate {
            if candidates.len() == limit {
                return (candidates, true);
            }
            candidates.push(ArchaeologyIncludeCandidate {
                kind: kind.to_string(),
                target,
                line: index as u64 + 1,
            });
        }
    }
    (candidates, false)
}

fn token(value: &str) -> Option<String> {
    let token = value
        .split_whitespace()
        .next()?
        .trim_matches(['\'', '"', '.', ';', ',']);
    (!token.is_empty() && token.len() <= 256).then(|| token.to_string())
}

fn is_inventory_config_path(path: &str) -> bool {
    matches!(
        path,
        ".gitignore"
            | ".gitattributes"
            | ".codevetter/archaeology.json"
            | ".codevetter/archaeology.yaml"
            | ".codevetter/archaeology.yml"
    )
}

fn update_config_identity(digest: &mut Sha256, entry: &GitTreeEntry) {
    digest.update(
        entry
            .relative_path
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    digest.update([0]);
    digest.update(entry.mode.as_bytes());
    digest.update([0]);
    digest.update(entry.object_id.as_bytes());
    digest.update([0]);
    digest.update(entry.size.unwrap_or(0).to_string().as_bytes());
    digest.update([0]);
}

fn git_line(root: &Path, args: &[&str]) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&git_bytes(root, args)?)
        .trim()
        .to_string())
}

pub(crate) fn git_head(root: &Path) -> Result<String, String> {
    let revision_sha = git_line(root, &["rev-parse", "HEAD"])?;
    validate_revision_sha(&revision_sha)
        .map_err(|_| "Archaeology inventory requires an exact full HEAD revision".to_string())?;
    Ok(revision_sha)
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Run archaeology Git command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Archaeology Git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err("Archaeology Git inventory returned an unsafe path".to_string())
    } else {
        Ok(())
    }
}

fn validate_object_id(value: &str) -> Result<(), String> {
    validate_revision_sha(value)
        .map_err(|_| "Archaeology Git object identity is invalid".to_string())
}

fn opaque_id(kind: &str, identity: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(identity);
    format!("{kind}:{}", hex(&digest.finalize()))
}

fn source_change_identity(repository_id: &str, path_identity: &str, object_id: &str) -> String {
    opaque_id(
        "archaeology-change",
        format!("{repository_id}\0{path_identity}\0{object_id}").as_bytes(),
    )
}

/// Rebuild an inventory from a prior ready manifest plus only the paths Git
/// reports as changed. Returning `None` is an intentional safe fallback: the
/// caller must perform the normal full-tree inventory in that case.
pub(crate) fn inventory_repository_delta(
    root: &Path,
    prior_revision: &str,
    prior_config_identity: &str,
    prior_units: &[ArchaeologyInventoryUnit],
    cancellation: &StructuralGraphCancellation,
    limits: ArchaeologyInventoryLimits,
) -> Result<Option<ArchaeologyRepositoryInventory>, String> {
    check_cancelled(cancellation)?;
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("Resolve archaeology repository: {error}"))?;
    let revision_sha = git_head(&canonical)?;
    if revision_sha == prior_revision || prior_units.len() > limits.max_files {
        return Ok(None);
    }
    let changes = git_delta_paths(&canonical, prior_revision, &revision_sha, limits)?;
    let Some(changes) = changes else {
        return Ok(None);
    };
    if changes
        .iter()
        .any(|path| is_inventory_config_path(path) || is_sensitive_path(path))
    {
        return Ok(None);
    }
    let repository_id = opaque_id(
        "archaeology-repository",
        canonical.to_string_lossy().as_bytes(),
    );
    if prior_units
        .iter()
        .any(|unit| unit.identity.repository_id != repository_id)
    {
        return Ok(None);
    }
    let changed = changes.into_iter().collect::<BTreeSet<_>>();
    let mut units = prior_units
        .iter()
        .filter(|unit| {
            unit.identity
                .relative_path
                .as_deref()
                .is_none_or(|path| !changed.contains(path))
        })
        .cloned()
        .map(|mut unit| {
            let content = unit
                .identity
                .content_hash
                .as_deref()
                .unwrap_or("content-unavailable");
            unit.identity.source_unit_id = opaque_id(
                "archaeology-source-unit",
                format!(
                    "{repository_id}\0{revision_sha}\0{}\0{content}",
                    unit.identity.path_identity
                )
                .as_bytes(),
            );
            unit.identity.revision_sha = revision_sha.clone();
            unit
        })
        .collect::<Vec<_>>();
    let changed_paths = changed.into_iter().collect::<Vec<_>>();
    let mut blobs = GitBlobBatch::start(&canonical)?;
    discover_tree_paths(
        &canonical,
        &revision_sha,
        Some(&changed_paths),
        cancellation,
        limits,
        &mut |entry| {
            check_cancelled(cancellation)?;
            let Some(path) = entry.relative_path.as_deref() else {
                return Err("Archaeology delta inventory returned a non-UTF-8 path".into());
            };
            if !changed_paths
                .binary_search_by(|candidate| candidate.as_str().cmp(path))
                .is_ok()
            {
                return Err("Archaeology delta inventory returned an unexpected path".into());
            }
            units.push(inventory_tree_unit(
                &repository_id,
                &revision_sha,
                &entry,
                path,
                &mut blobs,
                cancellation,
                limits,
                &mut |_| {},
            )?);
            Ok(())
        },
    )?;
    blobs.finish()?;
    check_cancelled(cancellation)?;
    if git_head(&canonical)? != revision_sha {
        return Err("Archaeology HEAD changed during delta inventory".to_string());
    }
    if units.len() > limits.max_files {
        return Err("Archaeology repository inventory exceeds its path bound".into());
    }
    units.sort_by(|left, right| {
        left.identity
            .path_identity
            .cmp(&right.identity.path_identity)
    });
    if units
        .windows(2)
        .any(|pair| pair[0].identity.path_identity == pair[1].identity.path_identity)
    {
        return Ok(None);
    }
    let mut reasons = BTreeSet::new();
    let mut discovered_bytes = 0_u64;
    let mut indexed_bytes = 0_u64;
    let mut indexed_source_units = 0_u64;
    for unit in &units {
        discovered_bytes = discovered_bytes.saturating_add(unit.byte_count);
        if unit.identity.content_hash.is_some() {
            indexed_source_units = indexed_source_units.saturating_add(1);
            indexed_bytes = indexed_bytes.saturating_add(unit.byte_count);
        }
        reasons.extend(unit.coverage_reasons.iter().cloned());
    }
    let repository = ArchaeologyRepositoryIdentity {
        repository_id,
        revision_sha: revision_sha.clone(),
        source_identity: opaque_id(
            "archaeology-source",
            format!("{revision_sha}\0{prior_config_identity}").as_bytes(),
        ),
    };
    let state = if reasons.is_empty() {
        ArchaeologyCoverageState::Complete
    } else {
        ArchaeologyCoverageState::Partial
    };
    let discovered_source_units = units.len() as u64;
    Ok(Some(ArchaeologyRepositoryInventory {
        schema_version: ARCHAEOLOGY_SCHEMA_VERSION,
        policy_version: INVENTORY_POLICY_VERSION.into(),
        repository,
        config_identity: prior_config_identity.into(),
        source_units: units,
        coverage: ArchaeologyCoverage {
            state: state.clone(),
            parser_coverage: ArchaeologyCoverageState::Unavailable,
            repository_coverage: state,
            temporal_coverage: ArchaeologyCoverageState::Unavailable,
            discovered_source_units,
            indexed_source_units,
            discovered_bytes,
            indexed_bytes,
            reasons: reasons.into_iter().collect(),
        },
    }))
}

fn git_delta_paths(
    root: &Path,
    prior_revision: &str,
    revision_sha: &str,
    limits: ArchaeologyInventoryLimits,
) -> Result<Option<Vec<String>>, String> {
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["merge-base", "--is-ancestor", prior_revision, revision_sha])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("Check archaeology Git ancestry: {error}"))?;
    if !ancestor.success() {
        return Ok(None);
    }
    let bytes = git_bytes(
        root,
        &[
            "diff-tree",
            "--no-commit-id",
            "-r",
            "--name-status",
            "-z",
            "--no-renames",
            prior_revision,
            revision_sha,
        ],
    )?;
    let mut fields = bytes.split(|byte| *byte == 0);
    let mut paths = BTreeSet::new();
    while let Some(status) = fields.next().filter(|field| !field.is_empty()) {
        let status =
            std::str::from_utf8(status).map_err(|_| "Archaeology Git delta status is not UTF-8")?;
        if !matches!(status, "A" | "M" | "D" | "T") {
            return Ok(None);
        }
        let path = fields
            .next()
            .ok_or("Archaeology Git delta record is incomplete")?;
        let path = std::str::from_utf8(path)
            .map_err(|_| "Archaeology Git delta path is not UTF-8")?
            .replace('\\', "/");
        validate_relative_path(&path)?;
        paths.insert(path);
        if paths.len() > limits.max_files {
            return Ok(None);
        }
    }
    Ok(Some(paths.into_iter().collect()))
}

pub(super) fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;
    use tempfile::TempDir;

    #[test]
    fn inventory_is_deterministic_dialect_aware_and_privacy_safe() {
        let fixture = repository();
        write(
            fixture.path(),
            "src/claim.cbl",
            "000100 IDENTIFICATION DIVISION.\n000200 DATA DIVISION.\n000300 COPY CLAIMREC.\n000400 PROCEDURE DIVISION.\n",
        );
        write(
            fixture.path(),
            "src/route.s",
            ".globl route_claim\nroute_claim:\n  cmpq $0, %rdi\n",
        );
        write(fixture.path(), ".env", "API_KEY=must-not-be-read\n");
        write(
            fixture.path(),
            "vendor/lib.ts",
            "export const vendor = true;\n",
        );
        write(
            fixture.path(),
            "src/client.generated.ts",
            "export const generated = true;\n",
        );
        commit_all(fixture.path());

        let first = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("inventory");
        let second = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("repeat inventory");
        assert_eq!(first, second);
        let cobol = unit(&first, "src/claim.cbl");
        assert_eq!(cobol.language, "cobol");
        assert_eq!(cobol.dialect.as_deref(), Some("fixed"));
        assert_eq!(cobol.include_candidates[0].target, "CLAIMREC");
        let assembly = unit(&first, "src/route.s");
        assert_eq!(assembly.dialect.as_deref(), Some("gas-att"));
        assert_eq!(
            unit(&first, "vendor/lib.ts").classification,
            ArchaeologySourceClassification::Vendor
        );
        assert_eq!(
            unit(&first, "src/client.generated.ts").classification,
            ArchaeologySourceClassification::Generated
        );
        let protected = first
            .source_units
            .iter()
            .find(|unit| unit.classification == ArchaeologySourceClassification::Protected)
            .expect("protected unit");
        assert!(protected.identity.relative_path.is_none());
        assert!(protected.identity.content_hash.is_none());
        assert!(!serde_json::to_string(&first)
            .expect("inventory json")
            .contains("must-not-be-read"));
        assert_eq!(first.coverage.state, ArchaeologyCoverageState::Partial);
    }

    #[test]
    fn identical_git_forks_keep_repository_source_and_path_identities_scoped() {
        let first = repository();
        write(
            first.path(),
            "src/rules.cbl",
            "       IDENTIFICATION DIVISION.\n       PROGRAM-ID. RULES.\n",
        );
        commit_all(first.path());
        let second = TempDir::new().expect("fork directory");
        let status = Command::new("git")
            .args(["clone", "-q"])
            .arg(first.path())
            .arg(second.path())
            .status()
            .expect("clone fixture fork");
        assert!(status.success());

        let first_inventory = inventory_repository(
            first.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("first fork inventory");
        let second_inventory = inventory_repository(
            second.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("second fork inventory");
        assert_eq!(
            first_inventory.repository.revision_sha,
            second_inventory.repository.revision_sha
        );
        assert_ne!(
            first_inventory.repository.repository_id,
            second_inventory.repository.repository_id
        );
        let first_unit = unit(&first_inventory, "src/rules.cbl");
        let second_unit = unit(&second_inventory, "src/rules.cbl");
        assert_ne!(
            first_unit.identity.path_identity,
            second_unit.identity.path_identity
        );
        assert_ne!(
            first_unit.identity.source_unit_id,
            second_unit.identity.source_unit_id
        );
    }

    #[test]
    fn inventory_bounds_oversized_units_and_cancellation_without_partial_hashes() {
        let fixture = repository();
        write(fixture.path(), "src/large.cbl", &"A".repeat(1024));
        write(
            fixture.path(),
            "src/includes.cbl",
            "COPY FIRST.\nCOPY SECOND.\n",
        );
        commit_all(fixture.path());
        let inventory = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits {
                max_source_unit_bytes: 128,
                max_candidates_per_unit: 1,
                ..ArchaeologyInventoryLimits::default()
            },
        )
        .expect("bounded inventory");
        let large = unit(&inventory, "src/large.cbl");
        assert!(large.identity.content_hash.is_none());
        assert!(large
            .coverage_reasons
            .contains(&"source_unit_exceeds_byte_bound".to_string()));
        let includes = unit(&inventory, "src/includes.cbl");
        assert_eq!(includes.include_candidates.len(), 1);
        assert!(includes
            .coverage_reasons
            .contains(&"include_candidate_count_bound_reached".to_string()));

        let cancellation = StructuralGraphCancellation::default();
        cancellation.cancel();
        assert!(inventory_repository(
            fixture.path(),
            &cancellation,
            ArchaeologyInventoryLimits::default()
        )
        .unwrap_err()
        .contains("cancelled"));
    }

    #[test]
    fn exact_head_inventory_ignores_every_workspace_byte_and_is_read_only() {
        let sandbox = TempDir::new().expect("sandbox");
        let root = sandbox.path().join("repository");
        let linked = sandbox.path().join("linked-worktree");
        fs::create_dir(&root).expect("repository directory");
        init_repository(&root);
        write(&root, "src/main.ts", "export const value = 1;\n");
        write(&root, "src/large.cbl", &"A".repeat(192 * 1024));
        write(
            &root,
            ".env",
            "API_KEY=credential-sentinel-must-not-be-read\n",
        );
        commit_all(&root);
        run(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                linked.to_str().expect("linked path"),
                "-b",
                "fixture-linked",
            ],
        );
        let committed = inventory_repository(
            &root,
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("committed-tree baseline");
        run(
            &root,
            &["update-index", "--assume-unchanged", "src/main.ts"],
        );
        run(&root, &["update-index", "--skip-worktree", "src/large.cbl"]);
        write(&root, "src/main.ts", "export const value = 999;\n");
        write(&root, "src/large.cbl", "workspace-only\n");
        write(
            &root,
            "src/untracked.ts",
            "export const untracked = true;\n",
        );

        let source_paths = [
            root.join("src/main.ts"),
            root.join("src/large.cbl"),
            root.join("src/untracked.ts"),
            root.join(".env"),
            linked.join("src/main.ts"),
            linked.join("src/large.cbl"),
            linked.join(".env"),
        ];
        let before = RepositorySnapshot::capture(&root, &linked, &source_paths);
        let inventory = inventory_repository(
            &root,
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("read-only inventory");
        assert_eq!(inventory, committed);
        assert!(inventory
            .source_units
            .iter()
            .all(|unit| unit.identity.relative_path.as_deref() != Some("src/untracked.ts")));
        let protected = inventory
            .source_units
            .iter()
            .find(|unit| unit.classification == ArchaeologySourceClassification::Protected)
            .expect("protected credential unit");
        assert!(protected.identity.relative_path.is_none());
        assert!(protected.identity.content_hash.is_none());
        assert!(!serde_json::to_string(&inventory)
            .expect("inventory JSON")
            .contains("credential-sentinel"));
        assert_eq!(
            RepositorySnapshot::capture(&root, &linked, &source_paths),
            before
        );

        let cancellation = StructuralGraphCancellation::default();
        let cancellation_at_discovery = cancellation.clone();
        let mut discovered = 0;
        let error = inventory_repository_observed(
            &root,
            &cancellation,
            ArchaeologyInventoryLimits::default(),
            &mut |checkpoint| {
                if checkpoint == InventoryCheckpoint::PathDiscovered {
                    discovered += 1;
                    cancellation_at_discovery.cancel();
                }
            },
        )
        .expect_err("mid-discovery cancellation");
        assert_eq!(discovered, 1);
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(
            RepositorySnapshot::capture(&root, &linked, &source_paths),
            before
        );

        let cancellation = StructuralGraphCancellation::default();
        let cancellation_at_hash = cancellation.clone();
        let mut chunks = 0;

        let error = inventory_repository_observed(
            &root,
            &cancellation,
            ArchaeologyInventoryLimits::default(),
            &mut |checkpoint| {
                if checkpoint == InventoryCheckpoint::HashChunkRead {
                    chunks += 1;
                    cancellation_at_hash.cancel();
                }
            },
        )
        .expect_err("mid-hash cancellation");

        assert_eq!(chunks, 1, "cancellation must stop before a second chunk");
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(
            RepositorySnapshot::capture(&root, &linked, &source_paths),
            before
        );
    }

    #[test]
    fn config_identity_changes_only_with_relevant_config_content() {
        let fixture = repository();
        write(fixture.path(), "src/main.ts", "export const value = 1;\n");
        write(fixture.path(), ".gitignore", ".codevetter/\n");
        commit_all(fixture.path());
        let before = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("before");
        write(
            fixture.path(),
            ".codevetter/archaeology.json",
            "{\"dialect\":\"cobol\"}\n",
        );
        let workspace_only = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("workspace config is not HEAD config");
        assert_eq!(before.config_identity, workspace_only.config_identity);
        assert_eq!(
            before.repository.source_identity,
            workspace_only.repository.source_identity
        );
        run(
            fixture.path(),
            &["add", "-f", ".codevetter/archaeology.json"],
        );
        run(fixture.path(), &["commit", "-qm", "config"]);
        let after = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("committed config");
        assert_ne!(
            before.repository.revision_sha,
            after.repository.revision_sha
        );
        assert_ne!(before.config_identity, after.config_identity);
        assert_ne!(
            before.repository.source_identity,
            after.repository.source_identity
        );
    }

    #[test]
    fn legacy_dialects_and_candidate_bounds_use_content_evidence() {
        assert_eq!(
            detect_language(
                Path::new("billing.asm"),
                Some(b"BILLING CSECT\n         USING *,R15\n")
            ),
            ("assembly".to_string(), Some("hlasm".to_string()))
        );
        assert_eq!(
            detect_language(Path::new("billing.asm"), Some(b"entry:\n  db 0\n")),
            ("assembly".to_string(), Some("ambiguous".to_string()))
        );
        assert_eq!(
            detect_language(Path::new("record.cpy"), Some(b"  05 CLAIM-ID PIC X(10).\n")),
            ("cobol".to_string(), Some("copybook".to_string()))
        );
        let (candidates, truncated) =
            find_include_candidates(b"COPY FIRST.\nCOPY SECOND.\nCOPY THIRD.\n", 2);
        assert_eq!(candidates.len(), 2);
        assert!(truncated);
        assert_eq!(candidates[0].line, 1);
        assert_eq!(candidates[1].target, "SECOND");
        assert!(!find_include_candidates(b"COPY FIRST.\nCOPY SECOND.\n", 2).1);
        assert!(find_include_candidates(b"COPY FIRST.\n", 0).1);
    }

    #[test]
    fn non_text_source_blobs_and_non_utf8_paths_are_opaque_gaps() {
        let fixture = repository();
        write_bytes(
            fixture.path(),
            "src/nul.cbl",
            b"IDENTIFICATION\0DIVISION.\n",
        );
        write_bytes(fixture.path(), "src/invalid.cbl", &[0xff, b'\n']);
        #[cfg(unix)]
        let non_utf8_path_written = {
            use std::os::unix::ffi::OsStringExt;
            let path = fixture.path().join(std::ffi::OsString::from_vec(
                b"src/non-utf8-\xff.cbl".to_vec(),
            ));
            fs::write(path, b"       IDENTIFICATION DIVISION.\n").is_ok()
        };
        commit_all(fixture.path());
        let inventory = inventory_repository(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
        )
        .expect("opaque inventory");
        for path in ["src/nul.cbl", "src/invalid.cbl"] {
            let unit = unit(&inventory, path);
            assert_eq!(unit.classification, ArchaeologySourceClassification::Opaque);
            assert!(unit.identity.content_hash.is_none());
            assert_eq!(unit.line_count, 0);
            assert_eq!(unit.coverage_reasons, ["non_utf8_or_nul_source_excluded"]);
        }
        #[cfg(unix)]
        if non_utf8_path_written {
            assert!(inventory.source_units.iter().any(|unit| {
                unit.identity.relative_path.is_none()
                    && unit.coverage_reasons == ["non_utf8_path_excluded"]
            }));
        }
    }

    #[test]
    fn git_stream_protocol_parsers_reject_identity_type_size_and_shape_drift() {
        let object = "a".repeat(40);
        let record = format!("100644 blob {object} 3\tsrc/a.ts");
        let parsed = parse_tree_record(record.as_bytes()).expect("tree record");
        assert_eq!(parsed.object_id, object);
        assert_eq!(parsed.size, Some(3));
        assert!(parse_tree_record(format!("100644 tree {object} -\tsrc").as_bytes()).is_err());
        assert!(parse_tree_record(format!("100644 blob {object} 3 src/a.ts").as_bytes()).is_err());
        let mut non_utf8_record = format!("100644 blob {object} 3\tsrc/").into_bytes();
        non_utf8_record.extend_from_slice(&[0xff]);
        assert!(String::from_utf8(
            parse_tree_record(&non_utf8_record)
                .expect("opaque path record")
                .path
        )
        .is_err());
        assert!(parse_batch_header(format!("{object} blob 3").as_bytes(), &object, 3).is_ok());
        assert!(parse_batch_header(format!("{object} commit 3").as_bytes(), &object, 3).is_err());
        assert!(
            parse_batch_header(format!("{} blob 3", "b".repeat(40)).as_bytes(), &object, 3)
                .is_err()
        );
        assert!(parse_batch_header(format!("{object} blob 4").as_bytes(), &object, 3).is_err());
    }

    #[test]
    fn streaming_inventory_emits_units_without_retaining_a_catalog() {
        let fixture = repository();
        write(fixture.path(), "src/a.ts", "export const a = 1;\n");
        write(fixture.path(), "src/b.ts", "export const b = 2;\n");
        commit_all(fixture.path());
        let mut emitted = Vec::new();
        let summary = inventory_repository_streaming(
            fixture.path(),
            &StructuralGraphCancellation::default(),
            ArchaeologyInventoryLimits::default(),
            &mut |unit| {
                emitted.push(unit.identity.source_unit_id);
                Ok(())
            },
        )
        .expect("streamed inventory");
        assert_eq!(emitted.len(), 2);
        assert_eq!(summary.coverage.discovered_source_units, 2);
        assert_eq!(summary.coverage.indexed_source_units, 2);
    }

    #[test]
    fn delta_inventory_matches_full_scan_for_add_modify_and_delete() {
        let fixture = repository();
        write(
            fixture.path(),
            "src/stable.ts",
            "export const stable = 1;\n",
        );
        write(
            fixture.path(),
            "src/changed.cbl",
            "000100 PROCEDURE DIVISION.\n",
        );
        write(fixture.path(), "src/deleted.s", ".global old\nold:\n ret\n");
        commit_all(fixture.path());
        let cancellation = StructuralGraphCancellation::default();
        let prior = inventory_repository(
            fixture.path(),
            &cancellation,
            ArchaeologyInventoryLimits::default(),
        )
        .expect("baseline inventory");
        write(fixture.path(), "src/changed.cbl", "000100 COPY CLAIMREC.\n");
        fs::remove_file(fixture.path().join("src/deleted.s")).expect("remove fixture");
        write(
            fixture.path(),
            "src/added.ts",
            "export const added = true;\n",
        );
        commit_all(fixture.path());

        let delta = inventory_repository_delta(
            fixture.path(),
            &prior.repository.revision_sha,
            &prior.config_identity,
            &prior.source_units,
            &cancellation,
            ArchaeologyInventoryLimits::default(),
        )
        .expect("delta inventory")
        .expect("eligible delta inventory");
        let full = inventory_repository(
            fixture.path(),
            &cancellation,
            ArchaeologyInventoryLimits::default(),
        )
        .expect("full inventory");
        assert_eq!(delta.summary(), full.summary());
        let mut delta_units = delta.source_units.clone();
        let mut full_units = full.source_units.clone();
        let order = |left: &ArchaeologyInventoryUnit, right: &ArchaeologyInventoryUnit| {
            left.identity
                .path_identity
                .cmp(&right.identity.path_identity)
        };
        delta_units.sort_by(order);
        full_units.sort_by(order);
        assert_eq!(delta_units, full_units);
        assert_ne!(
            unit(&delta, "src/stable.ts").identity.source_unit_id,
            unit(&prior, "src/stable.ts").identity.source_unit_id,
            "revision-scoped source IDs must advance even when content is reused"
        );
    }

    #[test]
    fn delta_inventory_falls_back_when_inventory_config_changes() {
        let fixture = repository();
        write(fixture.path(), "src/main.ts", "export const value = 1;\n");
        commit_all(fixture.path());
        let cancellation = StructuralGraphCancellation::default();
        let prior = inventory_repository(
            fixture.path(),
            &cancellation,
            ArchaeologyInventoryLimits::default(),
        )
        .expect("baseline inventory");
        write(fixture.path(), ".gitignore", "dist/\n");
        commit_all(fixture.path());
        assert!(inventory_repository_delta(
            fixture.path(),
            &prior.repository.revision_sha,
            &prior.config_identity,
            &prior.source_units,
            &cancellation,
            ArchaeologyInventoryLimits::default(),
        )
        .expect("delta fallback")
        .is_none());
    }

    fn repository() -> TempDir {
        let directory = TempDir::new().expect("temp repo");
        init_repository(directory.path());
        directory
    }

    fn init_repository(root: &Path) {
        run(root, &["init", "-q"]);
        run(root, &["config", "user.name", "Fixture"]);
        run(root, &["config", "user.email", "fixture@example.test"]);
    }

    fn write(root: &Path, relative: &str, content: &str) {
        write_bytes(root, relative, content.as_bytes());
    }

    fn write_bytes(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write fixture");
    }

    fn commit_all(root: &Path) {
        run(root, &["add", "-A", "-f"]);
        run(root, &["commit", "-qm", "fixture"]);
    }

    fn run(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?}");
    }

    #[derive(Debug, PartialEq, Eq)]
    struct RepositorySnapshot {
        heads: Vec<Vec<u8>>,
        refs: Vec<u8>,
        worktrees: Vec<u8>,
        indexes: Vec<FileSnapshot>,
        sources: Vec<FileSnapshot>,
    }

    impl RepositorySnapshot {
        fn capture(root: &Path, linked: &Path, source_paths: &[std::path::PathBuf]) -> Self {
            Self {
                heads: [root, linked]
                    .iter()
                    .map(|worktree| git_output(worktree, &["rev-parse", "HEAD"]))
                    .collect(),
                refs: git_output(
                    root,
                    &[
                        "for-each-ref",
                        "--format=%(refname)%00%(objectname)%00%(symref)",
                    ],
                ),
                worktrees: git_output(root, &["worktree", "list", "--porcelain"]),
                indexes: [root, linked]
                    .iter()
                    .map(|worktree| {
                        let encoded = git_output(worktree, &["rev-parse", "--git-path", "index"]);
                        let value = String::from_utf8(encoded).expect("UTF-8 Git index path");
                        let path = Path::new(value.trim());
                        FileSnapshot::capture(if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            worktree.join(path)
                        })
                    })
                    .collect(),
                sources: source_paths
                    .iter()
                    .map(|path| FileSnapshot::capture(path.clone()))
                    .collect(),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct FileSnapshot {
        bytes: Option<Vec<u8>>,
        modified: SystemTime,
    }

    impl FileSnapshot {
        fn capture(path: std::path::PathBuf) -> Self {
            let metadata = fs::metadata(&path)
                .unwrap_or_else(|error| panic!("read metadata for {}: {error}", path.display()));
            Self {
                bytes: fs::read(&path).ok(),
                modified: metadata.modified().expect("modified timestamp"),
            }
        }
    }

    fn git_output(root: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("run Git snapshot command");
        assert!(output.status.success(), "git {args:?}");
        output.stdout
    }

    fn unit<'a>(
        inventory: &'a ArchaeologyRepositoryInventory,
        path: &str,
    ) -> &'a ArchaeologyInventoryUnit {
        inventory
            .source_units
            .iter()
            .find(|unit| unit.identity.relative_path.as_deref() == Some(path))
            .unwrap_or_else(|| panic!("missing {path}"))
    }
}
