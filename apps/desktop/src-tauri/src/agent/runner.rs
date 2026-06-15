//! Agent loop: launch browser → snapshot → ask brain → execute → emit
//! `agent:step` Tauri event → repeat until done / give_up / budget exhausted.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::brain::{Brain, BrainContext};
use super::browser::Browser;
use super::cli_brain::CliBrain;
use super::local_server::LocalServer;
use super::types::{AgentAction, AgentRunInput, AgentRunResult, AgentStep};

const DEFAULT_MAX_STEPS: u32 = 30;
const MAX_CONSECUTIVE_BRAIN_FAILURES: u32 = 2;
const STEP_EVENT: &str = "agent:step";

pub async fn run_agent_task(
    app: AppHandle,
    input: AgentRunInput,
) -> Result<AgentRunResult, String> {
    let brain = CliBrain::new(input.provider.clone(), input.model.clone());
    run_with_brain(app, input, brain).await
}

/// Generic over the brain so tests and future providers can swap in their
/// own `Brain` impl without touching the loop.
pub async fn run_with_brain<B: Brain>(
    app: AppHandle,
    input: AgentRunInput,
    brain: B,
) -> Result<AgentRunResult, String> {
    let run_id = Uuid::new_v4().to_string();
    let started = Instant::now();
    let max_steps = input.max_steps.unwrap_or(DEFAULT_MAX_STEPS);

    let tmp_dir = std::env::temp_dir().join(format!("codevetter-agent-{run_id}"));
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("create tempdir {tmp_dir:?}: {e}"))?;

    // Optionally launch the project's dev server before driving the browser.
    // `_dev_server` lives until end of function — Drop kills the process.
    let _dev_server = match input.project_dir.as_deref() {
        Some(dir) => Some(
            LocalServer::start(
                &PathBuf::from(dir),
                &input.url,
                Duration::from_secs(60),
            )
            .await?,
        ),
        None => None,
    };

    let browser = Browser::launch().await?;
    browser.goto(&input.url).await?;

    let mut steps: Vec<AgentStep> = Vec::new();
    let mut history: Vec<String> = Vec::new();
    let mut completed = false;
    let mut gave_up = false;
    let mut last_url = input.url.clone();
    let mut last_title = String::new();
    let mut consecutive_brain_failures: u32 = 0;

    for index in 0..max_steps {
        let step_start = Instant::now();
        let shot_path = tmp_dir.join(format!("step-{index:03}.png"));
        let state = browser.snapshot(Some(&shot_path)).await?;
        last_url = state.url.clone();
        last_title = state.title.clone();

        let ctx = BrainContext {
            goal: &input.goal,
            persona: input.persona.as_deref(),
            history: &history,
            url: &state.url,
            page_title: &state.title,
            accessibility_tree: &state.element_list,
            screenshot_path: state.screenshot_path.as_deref(),
        };

        let brain_result = brain.next_action(ctx).await;

        let action = match brain_result {
            Ok(a) => {
                consecutive_brain_failures = 0;
                a
            }
            Err(e) => {
                consecutive_brain_failures += 1;
                let still_have_budget =
                    consecutive_brain_failures < MAX_CONSECUTIVE_BRAIN_FAILURES;

                // Emit a step entry so the UI shows what happened during the
                // failed attempt, even on retry.
                let synthetic = if still_have_budget {
                    AgentAction::GiveUp {
                        reasoning: format!(
                            "brain error (retry {}/{}): {e}",
                            consecutive_brain_failures, MAX_CONSECUTIVE_BRAIN_FAILURES
                        ),
                    }
                } else {
                    AgentAction::GiveUp {
                        reasoning: format!(
                            "brain failed {MAX_CONSECUTIVE_BRAIN_FAILURES} consecutive times: {e}"
                        ),
                    }
                };
                let step = AgentStep {
                    index,
                    action: synthetic,
                    url: state.url,
                    page_title: state.title,
                    screenshot_path: state.screenshot_path.map(path_to_string),
                    screenshot_data_url: state.screenshot_data_url,
                    elapsed_ms: step_start.elapsed().as_millis() as u64,
                    error: Some(e),
                };
                let _ = app.emit(STEP_EVENT, &step);
                history.push(format_history_entry(&step));
                steps.push(step);
                if still_have_budget {
                    continue;
                }
                gave_up = true;
                break;
            }
        };

        let exec_err = execute_action(&browser, &action).await.err();

        let elapsed_ms = step_start.elapsed().as_millis() as u64;
        let step = AgentStep {
            index,
            action: action.clone(),
            url: state.url,
            page_title: state.title,
            screenshot_path: state.screenshot_path.map(path_to_string),
            screenshot_data_url: state.screenshot_data_url,
            elapsed_ms,
            error: exec_err,
        };

        let _ = app.emit(STEP_EVENT, &step);
        history.push(format_history_entry(&step));
        steps.push(step);

        match &action {
            AgentAction::Done { .. } => {
                completed = true;
                break;
            }
            AgentAction::GiveUp { .. } => {
                gave_up = true;
                break;
            }
            _ => {}
        }
    }

    let _ = browser.close().await;

    Ok(AgentRunResult {
        run_id,
        goal: input.goal,
        completed,
        gave_up,
        step_count: steps.len() as u32,
        final_url: last_url,
        final_title: last_title,
        duration_ms: started.elapsed().as_millis() as u64,
        steps,
        error: None,
    })
}

async fn execute_action(browser: &Browser, action: &AgentAction) -> Result<(), String> {
    match action {
        AgentAction::Click { selector, .. } => browser.click(selector).await,
        AgentAction::Type { selector, text, .. } => browser.type_into(selector, text).await,
        AgentAction::Key { key, .. } => browser.press_key(key).await,
        AgentAction::Scroll { delta, .. } => browser.scroll(*delta).await,
        AgentAction::Goto { url, .. } => browser.goto(url).await,
        AgentAction::Done { .. } | AgentAction::GiveUp { .. } => Ok(()),
    }
}

fn format_history_entry(step: &AgentStep) -> String {
    let desc = match &step.action {
        AgentAction::Click { selector, .. } => format!("clicked {selector}"),
        AgentAction::Type { selector, text, .. } => format!("typed into {selector}: {text:?}"),
        AgentAction::Key { key, .. } => format!("pressed {key}"),
        AgentAction::Scroll { delta, .. } => format!("scrolled {delta}px"),
        AgentAction::Goto { url, .. } => format!("navigated to {url}"),
        AgentAction::Done { reasoning } => format!("done: {reasoning}"),
        AgentAction::GiveUp { reasoning } => format!("gave up: {reasoning}"),
    };
    if let Some(err) = &step.error {
        format!("{desc} (error: {err})")
    } else {
        desc
    }
}

fn path_to_string(p: std::path::PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step_with(action: AgentAction, error: Option<&str>) -> AgentStep {
        AgentStep {
            index: 0,
            action,
            url: "https://example.com".into(),
            page_title: "Example".into(),
            screenshot_path: None,
            screenshot_data_url: None,
            elapsed_ms: 100,
            error: error.map(String::from),
        }
    }

    #[test]
    fn history_entry_describes_click() {
        let s = step_with(
            AgentAction::Click {
                selector: "#dl".into(),
                reasoning: "primary CTA".into(),
            },
            None,
        );
        assert_eq!(format_history_entry(&s), "clicked #dl");
    }

    #[test]
    fn history_entry_appends_error() {
        let s = step_with(
            AgentAction::Click {
                selector: "#dl".into(),
                reasoning: "primary CTA".into(),
            },
            Some("not found"),
        );
        assert_eq!(format_history_entry(&s), "clicked #dl (error: not found)");
    }

    #[test]
    fn history_entry_describes_done() {
        let s = step_with(
            AgentAction::Done {
                reasoning: "found the price".into(),
            },
            None,
        );
        assert_eq!(format_history_entry(&s), "done: found the price");
    }
}
