use crate::{
    commands::secret_policy::{contains_sensitive_path, looks_like_secret},
    mcp::limits::{MAX_EXCERPT_BYTES, MAX_RESPONSE_BYTES},
};
use serde_json::{Map, Value};
use std::path::Path;

const OMITTED: &str = "[redacted]";

pub fn sanitize_response(mut value: Value) -> Result<Value, String> {
    sanitize_value(None, &mut value);
    let bytes =
        serde_json::to_vec(&value).map_err(|error| format!("Serialize MCP response: {error}"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(format!(
            "MCP response exceeds the {} byte limit; narrow the request",
            MAX_RESPONSE_BYTES
        ));
    }
    Ok(value)
}

fn sanitize_value(key: Option<&str>, value: &mut Value) {
    match value {
        Value::Object(map) => sanitize_map(map),
        Value::Array(values) => {
            for value in values {
                sanitize_value(key, value);
            }
        }
        Value::String(text) => {
            if (key.is_some_and(is_sensitive_reference_key) && contains_sensitive_path(text))
                || contains_absolute_local_path(text)
                || looks_like_secret(text)
                || looks_like_email(text)
            {
                *text = OMITTED.to_string();
            } else if key.is_some_and(is_excerpt_key) && text.len() > MAX_EXCERPT_BYTES {
                *text = truncate_utf8_bytes(text, MAX_EXCERPT_BYTES).to_string();
            }
        }
        _ => {}
    }
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn sanitize_map(map: &mut Map<String, Value>) {
    for key in [
        "repo_path",
        "repository_path",
        "database_path",
        "command",
        "raw_prompt",
        "prompt",
        "email",
        "author_email",
        "content_hash",
        "credential",
        "credentials",
    ] {
        map.remove(key);
    }
    for (key, value) in map.iter_mut() {
        sanitize_value(Some(key), value);
    }
}

fn is_sensitive_reference_key(key: &str) -> bool {
    matches!(
        key,
        "path" | "old_path" | "source_path" | "file" | "filename" | "label" | "detail"
    )
}

fn is_excerpt_key(key: &str) -> bool {
    matches!(
        key,
        "summary" | "detail" | "excerpt" | "text" | "subject" | "title" | "label"
    )
}

fn looks_like_email(value: &str) -> bool {
    value.split_whitespace().any(|part| {
        let part = part.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '@' | '.' | '_' | '-' | '+')
        });
        let Some((local, domain)) = part.split_once('@') else {
            return false;
        };
        !local.is_empty()
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    })
}

fn is_absolute_local_path(value: &str) -> bool {
    Path::new(value).is_absolute()
        || value.as_bytes().get(1) == Some(&b':')
            && value
                .as_bytes()
                .get(2)
                .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
}

fn contains_absolute_local_path(value: &str) -> bool {
    is_absolute_local_path(value)
        || value
            .split(|character: char| {
                character.is_whitespace()
                    || matches!(
                        character,
                        '`' | '\''
                            | '"'
                            | ','
                            | ';'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '<'
                            | '>'
                            | '='
                    )
            })
            .filter(|token| !token.is_empty())
            .any(is_absolute_local_path)
}

pub fn sanitize_error_message(message: &str, repo_path: &str) -> String {
    if contains_sensitive_path(message)
        || looks_like_secret(message)
        || contains_absolute_local_path(message)
    {
        return "Requested content is unavailable under CodeVetter redaction policy".to_string();
    }
    if repo_path.is_empty() {
        message.to_string()
    } else {
        message.replace(repo_path, "[repository]")
    }
}

#[cfg(test)]
mod tests;
