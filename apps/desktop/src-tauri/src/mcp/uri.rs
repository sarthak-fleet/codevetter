use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::fmt;

pub const SCHEME: &str = "codevetter-history";

pub(crate) const RESOURCE_KINDS: &[&str] = &[
    "repository",
    "graph",
    "snapshot",
    "community",
    "release",
    "landmark-catalog",
    "contributor-summary",
    "commit",
    "episode",
    "entity-lineage",
    "causal-thread",
    "annotation",
    "evidence",
    "archaeology-catalog",
    "archaeology-rule",
    "archaeology-domain",
    "archaeology-source",
    "archaeology-relations",
    "archaeology-temporal",
    "archaeology-evidence",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryResourceUri {
    pub repo_id: String,
    pub kind: String,
    pub id: String,
}

impl HistoryResourceUri {
    pub fn new(repo_id: &str, kind: &str, id: &str) -> Result<Self, String> {
        validate_repo_id(repo_id)?;
        if !RESOURCE_KINDS.contains(&kind) {
            return Err("Unknown CodeVetter history resource kind".to_string());
        }
        if id.is_empty() || id.len() > 4_096 || id.chars().any(char::is_control) {
            return Err("Invalid CodeVetter history resource identifier".to_string());
        }
        Ok(Self {
            repo_id: repo_id.to_string(),
            kind: kind.to_string(),
            id: id.to_string(),
        })
    }

    pub fn parse(raw: &str, expected_repo_id: &str) -> Result<Self, String> {
        let prefix = format!("{SCHEME}://");
        let remainder = raw
            .strip_prefix(&prefix)
            .ok_or_else(|| "Invalid CodeVetter history resource scheme".to_string())?;
        if remainder.contains(['?', '#', '\\']) || remainder.contains("..") {
            return Err("Invalid CodeVetter history resource URI".to_string());
        }
        let mut segments = remainder.split('/');
        let repo_id = segments.next().unwrap_or_default();
        let kind = segments.next().unwrap_or_default();
        let encoded_id = segments.next().unwrap_or_default();
        if segments.next().is_some() || repo_id != expected_repo_id {
            return Err("CodeVetter history resource is outside this repository scope".to_string());
        }
        validate_repo_id(repo_id)?;
        if !RESOURCE_KINDS.contains(&kind) {
            return Err("Unknown CodeVetter history resource kind".to_string());
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded_id)
            .map_err(|_| "Malformed CodeVetter history resource identifier".to_string())?;
        let id = String::from_utf8(decoded)
            .map_err(|_| "Malformed CodeVetter history resource identifier".to_string())?;
        Self::new(repo_id, kind, &id)
    }
}

impl fmt::Display for HistoryResourceUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let encoded_id = URL_SAFE_NO_PAD.encode(self.id.as_bytes());
        write!(
            formatter,
            "{SCHEME}://{}/{}/{}",
            self.repo_id, self.kind, encoded_id
        )
    }
}

fn validate_repo_id(repo_id: &str) -> Result<(), String> {
    if repo_id.len() < 16
        || repo_id.len() > 128
        || !repo_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Invalid opaque CodeVetter repository identity".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
