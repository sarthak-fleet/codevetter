//! Brain layer: given the current page state + goal + history, return the
//! next AgentAction. The `Brain` trait abstracts the model provider so the
//! runner is generic across CliBrain (the shipped path — spawns `claude` /
//! `codex` directly via tokio::process) and any future impls (Ollama,
//! Anthropic API direct, etc.).

use super::types::AgentAction;

pub struct BrainContext<'a> {
    pub goal: &'a str,
    pub persona: Option<&'a str>,
    /// Compact summaries of prior steps for short-term memory.
    pub history: &'a [String],
    pub url: &'a str,
    pub page_title: &'a str,
    pub accessibility_tree: &'a str,
    /// Path to a screenshot of the current viewport, if captured.
    pub screenshot_path: Option<&'a std::path::Path>,
}

pub trait Brain: Send + Sync {
    fn next_action(
        &self,
        ctx: BrainContext<'_>,
    ) -> impl std::future::Future<Output = Result<AgentAction, String>> + Send;
}

/// Find the last well-formed JSON object in `text` that has a `type` field
/// matching one of our action shapes, and deserialize it as AgentAction.
/// Tolerates prose around the JSON (which CLIs often emit).
pub fn extract_action(text: &str) -> Result<AgentAction, String> {
    let candidates = scan_json_objects(text);
    if candidates.is_empty() {
        return Err(format!(
            "no JSON action found in brain response (length {}): {}",
            text.len(),
            preview(text, 240)
        ));
    }
    // Walk last-to-first; the action is typically the final block.
    for blob in candidates.iter().rev() {
        match serde_json::from_str::<AgentAction>(blob) {
            Ok(action) => return Ok(action),
            Err(_) => continue,
        }
    }
    Err(format!(
        "found {} JSON blocks but none matched the AgentAction schema. Last block: {}",
        candidates.len(),
        candidates.last().map(|s| preview(s, 240)).unwrap_or_default(),
    ))
}

/// Scan `text` for balanced `{...}` substrings. Naive but robust to prose.
fn scan_json_objects(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0_i32;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            if let Ok(slice) = std::str::from_utf8(&bytes[s..=i]) {
                                out.push(slice.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn preview(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_clean_done_action() {
        let action = extract_action(r#"{"type":"done","reasoning":"goal met"}"#).unwrap();
        match action {
            AgentAction::Done { reasoning } => assert_eq!(reasoning, "goal met"),
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn extracts_click_from_prose_wrapped_json() {
        let txt = r##"Looking at the page, the Download button is clearly the next step.

{"type":"click","selector":"#dl","reasoning":"primary CTA"}

That should take us to the install page."##;
        let action = extract_action(txt).unwrap();
        match action {
            AgentAction::Click { selector, reasoning } => {
                assert_eq!(selector, "#dl");
                assert_eq!(reasoning, "primary CTA");
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    #[test]
    fn picks_last_valid_block_when_multiple_exist() {
        let txt = r##"
        Earlier I considered: {"note":"not a real action"}
        Now I'll do: {"type":"scroll","delta":600,"reasoning":"see more"}
        "##;
        let action = extract_action(txt).unwrap();
        assert!(matches!(action, AgentAction::Scroll { delta: 600, .. }));
    }

    #[test]
    fn errors_on_no_json() {
        let err = extract_action("I have no idea what to do.").unwrap_err();
        assert!(err.contains("no JSON action found"), "{err}");
    }

    #[test]
    fn errors_on_unknown_action_type() {
        let err = extract_action(r#"{"type":"teleport","reasoning":"hmm"}"#).unwrap_err();
        assert!(err.contains("none matched"), "{err}");
    }
}
