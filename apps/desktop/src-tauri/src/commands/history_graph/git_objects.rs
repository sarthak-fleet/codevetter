use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GitTreeEntry {
    pub(super) object_id: String,
    pub(super) path: String,
}

pub(super) struct HistoricalBlobBatch {
    pub(super) blobs: Vec<HistoricalFileBlob>,
    pub(super) discovered_files: usize,
    pub(super) truncated: bool,
}

pub(super) struct GitObjectReader<'a> {
    pub(super) root: &'a Path,
}

impl<'a> GitObjectReader<'a> {
    pub(super) fn new(root: &'a Path) -> Self {
        Self { root }
    }

    #[cfg(test)]
    pub(super) fn blobs_at(&self, revision: &str) -> Result<Vec<HistoricalFileBlob>, String> {
        Ok(self.blobs_at_with_coverage(revision)?.blobs)
    }

    pub(super) fn blobs_at_with_coverage(
        &self,
        revision: &str,
    ) -> Result<HistoricalBlobBatch, String> {
        let revision = resolve_revision(self.root, revision)?;
        let tree = git_bytes(self.root, &["ls-tree", "-r", "-z", &revision])?;
        let mut entries = tree
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .filter_map(parse_tree_entry)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let discovered_files = entries.len();
        let truncated = discovered_files > MAX_HISTORICAL_FILES;
        entries.truncate(MAX_HISTORICAL_FILES);
        Ok(HistoricalBlobBatch {
            blobs: self.read_batch(&entries)?,
            discovered_files,
            truncated,
        })
    }

    pub(super) fn blobs_for_paths(
        &self,
        revision: &str,
        paths: &[String],
    ) -> Result<Vec<HistoricalFileBlob>, String> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let revision = resolve_revision(self.root, revision)?;
        let mut arguments = vec!["ls-tree", "-r", "-z", revision.as_str(), "--"];
        arguments.extend(paths.iter().map(String::as_str));
        let tree = git_bytes(self.root, &arguments)?;
        let mut entries = tree
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .filter_map(parse_tree_entry)
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries.dedup_by(|left, right| left.path == right.path);
        self.read_batch(&entries)
    }

    pub(super) fn read_batch(
        &self,
        entries: &[GitTreeEntry],
    ) -> Result<Vec<HistoricalFileBlob>, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(self.root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Start Git object reader: {error}"))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| "Git object reader stdin is unavailable".to_string())?;
            for entry in entries {
                writeln!(stdin, "{}", entry.object_id)
                    .map_err(|error| format!("Queue Git object: {error}"))?;
            }
        }
        drop(child.stdin.take());
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Git object reader stdout is unavailable".to_string())?;
        let mut reader = BufReader::new(stdout);
        let mut blobs = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut header = String::new();
            reader
                .read_line(&mut header)
                .map_err(|error| format!("Read Git object header: {error}"))?;
            let fields = header.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || fields[1] != "blob" {
                return Err(format!(
                    "Git object {} is unavailable or is not a blob",
                    entry.object_id
                ));
            }
            let size = fields[2]
                .parse::<usize>()
                .map_err(|error| format!("Invalid Git object size: {error}"))?;
            let bytes = if size <= MAX_HISTORICAL_BLOB_BYTES {
                let mut bytes = vec![0; size];
                reader
                    .read_exact(&mut bytes)
                    .map_err(|error| format!("Read Git object content: {error}"))?;
                bytes
            } else {
                std::io::copy(&mut reader.by_ref().take(size as u64), &mut std::io::sink())
                    .map_err(|error| format!("Skip oversized Git object: {error}"))?;
                vec![0; MAX_HISTORICAL_BLOB_BYTES + 1]
            };
            let mut newline = [0_u8; 1];
            reader
                .read_exact(&mut newline)
                .map_err(|error| format!("Read Git object delimiter: {error}"))?;
            blobs.push(HistoricalFileBlob {
                path: entry.path.clone(),
                bytes,
            });
        }
        let status = child
            .wait()
            .map_err(|error| format!("Wait for Git object reader: {error}"))?;
        if !status.success() {
            return Err("Git object reader failed".to_string());
        }
        Ok(blobs)
    }
}

pub(super) fn parse_tree_entry(record: &[u8]) -> Option<GitTreeEntry> {
    let tab = record.iter().position(|byte| *byte == b'\t')?;
    let header = String::from_utf8_lossy(&record[..tab]);
    let fields = header.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 3 || fields[1] != "blob" {
        return None;
    }
    Some(GitTreeEntry {
        object_id: fields[2].to_string(),
        path: String::from_utf8_lossy(&record[tab + 1..]).replace('\\', "/"),
    })
}
