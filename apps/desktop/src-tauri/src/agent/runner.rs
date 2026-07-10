//! Agent loop: launch browser → snapshot → ask brain → execute → emit
//! `agent:step` Tauri event → repeat until done / give_up / budget exhausted.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::brain::{Brain, BrainContext};
use super::browser::{Browser, SnapshotOpts, DEFAULT_MAX_ELEMENTS};
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
    let app_for_emit = app.clone();
    run_with_brain(input, brain, move |step| {
        let _ = app_for_emit.emit(STEP_EVENT, step);
    })
    .await
}

/// Generic over both the `Brain` and the per-step emit sink so tests can
/// drive the loop with a scripted brain + an in-memory collector instead of
/// a Tauri `AppHandle`. `emit_step` is called once per step, after the
/// action has been executed (or after a brain error).
pub async fn run_with_brain<B, F>(
    input: AgentRunInput,
    brain: B,
    emit_step: F,
) -> Result<AgentRunResult, String>
where
    B: Brain,
    F: Fn(&AgentStep) + Send + Sync,
{
    let run_id = Uuid::new_v4().to_string();
    let started = Instant::now();
    let max_steps = input.max_steps.unwrap_or(DEFAULT_MAX_STEPS);

    // Codex is the only provider whose model actually consumes the
    // screenshot bytes. For claude/gemini we skip capture+encode entirely
    // to save ~150-300ms per step.
    let provider_wants_screenshot = input.provider == "codex";

    let tmp_dir = std::env::temp_dir().join(format!("codevetter-agent-{run_id}"));
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| format!("create tempdir {tmp_dir:?}: {e}"))?;

    // Optionally launch the project's dev server before driving the browser.
    // `_dev_server` lives until end of function — Drop kills the process.
    let _dev_server = match input.project_dir.as_deref() {
        Some(dir) => Some(
            LocalServer::start(&PathBuf::from(dir), &input.url, Duration::from_secs(60)).await?,
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
        let shot_path: Option<PathBuf> = if provider_wants_screenshot {
            Some(tmp_dir.join(format!("step-{index:03}.jpg")))
        } else {
            None
        };

        let snapshot_start = Instant::now();
        let state = browser
            .snapshot(SnapshotOpts {
                screenshot_path: shot_path.as_deref(),
                max_elements: DEFAULT_MAX_ELEMENTS,
            })
            .await?;
        let snapshot_ms = snapshot_start.elapsed().as_millis() as u64;
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

        let brain_start = Instant::now();
        let brain_result = brain.next_action(ctx).await;
        let brain_ms = brain_start.elapsed().as_millis() as u64;

        let action = match brain_result {
            Ok(a) => {
                consecutive_brain_failures = 0;
                a
            }
            Err(e) => {
                consecutive_brain_failures += 1;
                let still_have_budget = consecutive_brain_failures < MAX_CONSECUTIVE_BRAIN_FAILURES;

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
                    snapshot_ms,
                    brain_ms,
                    exec_ms: 0,
                    error: Some(e),
                };
                emit_step(&step);
                history.push(format_history_entry(&step));
                steps.push(step);
                if still_have_budget {
                    continue;
                }
                gave_up = true;
                break;
            }
        };

        let exec_start = Instant::now();
        let exec_err = execute_action(&browser, &action).await.err();
        let exec_ms = exec_start.elapsed().as_millis() as u64;

        let step = AgentStep {
            index,
            action: action.clone(),
            url: state.url,
            page_title: state.title,
            screenshot_path: state.screenshot_path.map(path_to_string),
            screenshot_data_url: state.screenshot_data_url,
            elapsed_ms: step_start.elapsed().as_millis() as u64,
            snapshot_ms,
            brain_ms,
            exec_ms,
            error: exec_err,
        };

        emit_step(&step);
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
    use std::sync::Mutex;

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
            snapshot_ms: 10,
            brain_ms: 80,
            exec_ms: 10,
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

    /// Scripted brain that returns a queued sequence of actions.
    struct ScriptedBrain {
        actions: Mutex<Vec<AgentAction>>,
    }
    impl ScriptedBrain {
        fn new(actions: Vec<AgentAction>) -> Self {
            Self {
                actions: Mutex::new(actions),
            }
        }
    }
    impl Brain for ScriptedBrain {
        async fn next_action(&self, _ctx: BrainContext<'_>) -> Result<AgentAction, String> {
            self.actions
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| "scripted brain exhausted".to_string())
        }
    }

    /// Brain that fails its first N calls then returns `done`. Used to
    /// verify the retry budget without bringing up a real CLI.
    struct FlakyBrain {
        fails_remaining: Mutex<u32>,
    }
    impl FlakyBrain {
        fn new(fails: u32) -> Self {
            Self {
                fails_remaining: Mutex::new(fails),
            }
        }
    }
    impl Brain for FlakyBrain {
        async fn next_action(&self, _ctx: BrainContext<'_>) -> Result<AgentAction, String> {
            let mut left = self.fails_remaining.lock().unwrap();
            if *left > 0 {
                *left -= 1;
                Err(format!("flaky failure ({} left)", *left))
            } else {
                Ok(AgentAction::Done {
                    reasoning: "ok".into(),
                })
            }
        }
    }

    fn data_url(html: &str) -> String {
        format!("data:text/html;charset=utf-8,{}", urlencoding_lite(html))
    }

    /// Minimal URL-encode for the few characters that matter inside an inline
    /// data: URL — keeps the test self-contained without pulling a dep in.
    fn urlencoding_lite(s: &str) -> String {
        let mut out = String::new();
        for ch in s.chars() {
            match ch {
                ' ' => out.push_str("%20"),
                '<' => out.push_str("%3C"),
                '>' => out.push_str("%3E"),
                '"' => out.push_str("%22"),
                '#' => out.push_str("%23"),
                _ => out.push(ch),
            }
        }
        out
    }

    /// End-to-end loop check: real chromiumoxide-driven Chrome, scripted brain
    /// (so no LLM credits), in-memory event sink. Verifies that snapshot →
    /// brain → execute composes correctly across multiple steps and that
    /// `done` terminates the loop. Ignored by default.
    #[tokio::test]
    #[ignore]
    async fn e2e_loop_with_scripted_brain() {
        let url = data_url(
            r##"<title>T</title><button id="b1">One</button><a id="home" href="/">Home</a>"##,
        );
        let brain = ScriptedBrain::new(vec![
            AgentAction::Done {
                reasoning: "got there".into(),
            },
            AgentAction::Click {
                selector: "#b1".into(),
                reasoning: "click one".into(),
            },
            AgentAction::Scroll {
                delta: 100,
                reasoning: "see more".into(),
            },
        ]);

        let collected: Mutex<Vec<AgentStep>> = Mutex::new(Vec::new());
        let input = AgentRunInput {
            url: url.clone(),
            goal: "test".into(),
            persona: None,
            provider: "claude".into(),
            model: None,
            max_steps: Some(10),
            project_dir: None,
        };

        let result = run_with_brain(input, brain, |s| {
            collected.lock().unwrap().push(s.clone());
        })
        .await
        .expect("run");

        assert!(result.completed, "expected loop to complete: {result:?}");
        assert_eq!(result.step_count, 3, "expected 3 scripted steps");
        let events = collected.into_inner().unwrap();
        assert_eq!(events.len(), 3, "expected one event per step");
        assert!(matches!(events[0].action, AgentAction::Scroll { .. }));
        assert!(matches!(events[1].action, AgentAction::Click { .. }));
        assert!(matches!(events[2].action, AgentAction::Done { .. }));
        // Phase timings populated.
        assert!(events[0].snapshot_ms > 0 || events[0].brain_ms > 0);
    }

    /// Verifies the retry budget: brain fails once, then succeeds — loop
    /// completes (does not give up) and shows two recorded steps.
    #[tokio::test]
    #[ignore]
    async fn e2e_retry_recovers_within_budget() {
        let url = data_url(r##"<title>T</title>"##);
        let brain = FlakyBrain::new(1);

        let input = AgentRunInput {
            url,
            goal: "test".into(),
            persona: None,
            provider: "claude".into(),
            model: None,
            max_steps: Some(5),
            project_dir: None,
        };

        let result = run_with_brain(input, brain, |_| {}).await.expect("run");
        assert!(
            result.completed,
            "retry should let the loop finish: {result:?}"
        );
        assert_eq!(result.step_count, 2, "1 retry step + 1 done");
    }
}
