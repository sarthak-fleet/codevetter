use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpCursor {
    version: u8,
    repo_id: String,
    operation: String,
    offset: usize,
    fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<Value>,
}

impl McpCursor {
    pub fn new(repo_id: &str, operation: &str, offset: usize, fingerprint: &str) -> Self {
        Self {
            version: 1,
            repo_id: repo_id.to_string(),
            operation: operation.to_string(),
            offset,
            fingerprint: fingerprint.to_string(),
            position: None,
        }
    }

    pub fn with_position(mut self, position: Value) -> Self {
        self.position = Some(position);
        self
    }

    pub fn encode(&self) -> Result<String, String> {
        serde_json::to_vec(self)
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .map_err(|error| format!("Encode MCP cursor: {error}"))
    }

    pub fn decode(
        raw: &str,
        repo_id: &str,
        operation: &str,
        fingerprint: &str,
    ) -> Result<Self, String> {
        if raw.len() > 2_048 {
            return Err("Invalid MCP cursor".to_string());
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| "Invalid MCP cursor".to_string())?;
        let cursor: Self =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid MCP cursor".to_string())?;
        if cursor.version != 1
            || cursor.repo_id != repo_id
            || cursor.operation != operation
            || cursor.fingerprint != fingerprint
        {
            return Err("MCP cursor does not belong to this request".to_string());
        }
        Ok(cursor)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn position<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>, String> {
        self.position
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| "Invalid MCP cursor position".to_string())
    }
}

#[cfg(test)]
mod tests;
