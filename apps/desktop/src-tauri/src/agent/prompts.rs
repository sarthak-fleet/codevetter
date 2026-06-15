//! System prompts + action-schema instructions for the agent brain.
//! Output tokens cost ~215x more time than input tokens (browser-use
//! benchmark), so the prompt actively discourages verbose `reasoning`.

const ACTION_SCHEMA: &str = r#"Emit exactly one JSON object on its own line, no prose. Shapes:

  { "type":"click",   "selector":"...", "reasoning":"..." }
  { "type":"type",    "selector":"...", "text":"...", "reasoning":"..." }
  { "type":"key",     "key":"Enter|Tab|...", "reasoning":"..." }
  { "type":"scroll",  "delta":600, "reasoning":"..." }
  { "type":"goto",    "url":"https://...", "reasoning":"..." }
  { "type":"done",    "reasoning":"goal completed because ..." }
  { "type":"give_up", "reasoning":"stuck because ..." }

Constraints:
  - `reasoning`: <= 60 chars. One short clause. No restating the action.
  - Selectors: prefer the stable CSS in the visible elements list (e.g.
    `#dl`, `a[href="/pricing"]`). If no stable selector, use `text=Label`.
  - Return `done` as soon as the goal is unambiguously met.
  - Return `give_up` if you've made no progress for several steps."#;

pub fn system_prompt_for_goal(goal: &str, persona: Option<&str>) -> String {
    let persona_block = match persona {
        Some(p) if !p.trim().is_empty() => format!("\n\nPersona:\n{}\n", p.trim()),
        _ => String::new(),
    };
    format!(
        "You are CodeVetter's live browser agent. You drive a real Chrome \
         page step by step to accomplish a goal a user gave you. You see \
         the page through a numbered list of visible interactable elements \
         (and sometimes a screenshot). You pick exactly one action per turn.\
         \n\nGoal:\n{goal}\
         {persona_block}\
         \n\n{ACTION_SCHEMA}"
    )
}
