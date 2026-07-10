//! chromiumoxide-backed browser driver. Drives the user's installed Chrome
//! via CDP. We do not bundle Chromium — if Chrome isn't found, `launch`
//! returns a clear error and the caller surfaces it to the user.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, CaptureScreenshotParams,
};
use chromiumoxide::{Browser as Cdp, BrowserConfig, Page};
use futures::StreamExt;
use tokio::task::JoinHandle;

pub const DEFAULT_MAX_ELEMENTS: usize = 80;

pub struct Browser {
    inner: Cdp,
    page: Page,
    handler: JoinHandle<()>,
}

#[derive(Debug, Clone)]
pub struct SnapshotOpts<'a> {
    /// Where to write the captured screenshot. `None` skips screenshot
    /// capture entirely — the right call for text-only providers (claude,
    /// gemini), where bytes get thrown away anyway.
    pub screenshot_path: Option<&'a Path>,
    /// Cap on interactable elements returned. Applied in the injected JS so
    /// huge pages don't waste serialization time.
    pub max_elements: usize,
}

impl Default for SnapshotOpts<'_> {
    fn default() -> Self {
        Self {
            screenshot_path: None,
            max_elements: DEFAULT_MAX_ELEMENTS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageState {
    pub url: String,
    pub title: String,
    /// One line per interactable element, prefixed by [idx]. Compact form
    /// the brain consumes alongside the screenshot (when available).
    pub element_list: String,
    pub screenshot_path: Option<PathBuf>,
    /// `data:image/jpeg;base64,…` form of the same screenshot. Filled when a
    /// screenshot was captured so the frontend can render it inline without
    /// configuring the asset:// protocol scope. JPEG q80 keeps payload small.
    pub screenshot_data_url: Option<String>,
}

/// Builds the element-extraction JS with a cap inlined as `MAX`. Capping in
/// the page avoids serializing hundreds of elements over CDP and paying the
/// JSON parse cost on the Rust side. We sort by viewport-top so the cap
/// keeps the most-visible elements rather than truncating randomly.
fn extract_elements_js(max_elements: usize) -> String {
    format!(
        r#"
(() => {{
  const MAX = {max_elements};
  const out = [];
  const sel = 'a, button, input, textarea, select, [role="button"], [role="link"], [role="tab"], [role="menuitem"], [contenteditable="true"]';
  const nodes = document.querySelectorAll(sel);
  nodes.forEach((el) => {{
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || parseFloat(style.opacity) === 0) return;

    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute('role') || tag;
    const aria = el.getAttribute('aria-label') || '';
    const placeholder = el.getAttribute('placeholder') || '';
    const value = (el.value || '').toString();
    const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 100);
    const label = (text || aria || placeholder || value).slice(0, 100);

    let sel = null;
    if (el.id) sel = '#' + CSS.escape(el.id);
    else if (el.getAttribute('data-testid')) sel = '[data-testid="' + el.getAttribute('data-testid') + '"]';
    else if (el.getAttribute('aria-label')) sel = tag + '[aria-label="' + el.getAttribute('aria-label') + '"]';
    else if (tag === 'a' && el.getAttribute('href')) sel = 'a[href="' + el.getAttribute('href') + '"]';

    out.push({{
      role,
      label,
      selector: sel,
      top: Math.round(rect.top),
    }});
  }});
  out.sort((a, b) => a.top - b.top);
  const clipped = out.slice(0, MAX);
  return JSON.stringify(clipped.map((it, idx) => ({{ idx, ...it }})));
}})()
"#,
    )
}

impl Browser {
    pub async fn launch() -> Result<Self, String> {
        // Unique profile dir per launch so concurrent Browser instances (e.g.
        // multiple agent runs queued back-to-back, or parallel test threads)
        // don't deadlock on chromiumoxide's default singleton lock.
        let profile_dir =
            std::env::temp_dir().join(format!("codevetter-chrome-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("create profile dir {profile_dir:?}: {e}"))?;

        let config = BrowserConfig::builder()
            .user_data_dir(&profile_dir)
            .build()
            .map_err(|e| format!("BrowserConfig build failed: {e}"))?;

        let (inner, mut handler) = Cdp::launch(config).await.map_err(|e| {
            format!(
                "Failed to launch Chrome: {e}. CodeVetter relies on the Chrome \
                 you already have installed — install it from chrome.com if missing."
            )
        })?;

        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });

        let page = inner
            .new_page("about:blank")
            .await
            .map_err(|e| format!("Failed to open new page: {e}"))?;

        Ok(Self {
            inner,
            page,
            handler: handler_task,
        })
    }

    pub async fn goto(&self, url: &str) -> Result<(), String> {
        self.page
            .goto(url)
            .await
            .map_err(|e| format!("goto({url}) failed: {e}"))?;
        // Best-effort settle. Some sites never fire load; cap the wait.
        let _ = tokio::time::timeout(Duration::from_secs(8), self.page.wait_for_navigation()).await;
        Ok(())
    }

    pub async fn snapshot(&self, opts: SnapshotOpts<'_>) -> Result<PageState, String> {
        let url = self
            .page
            .url()
            .await
            .map_err(|e| format!("page.url() failed: {e}"))?
            .unwrap_or_else(|| "about:blank".into());

        let title = self
            .page
            .get_title()
            .await
            .map_err(|e| format!("page.get_title() failed: {e}"))?
            .unwrap_or_default();

        let js = extract_elements_js(opts.max_elements);
        let raw_value = self
            .page
            .evaluate(js.as_str())
            .await
            .map_err(|e| format!("element-extraction JS failed: {e}"))?;
        let json_str: String = raw_value
            .into_value()
            .map_err(|e| format!("element-extraction value not a string: {e}"))?;
        let element_list = format_element_list(&json_str);

        let (screenshot_path, screenshot_data_url) = match opts.screenshot_path {
            Some(p) => {
                // JPEG q80 is ~5-10x smaller than PNG for typical screenshots
                // and the model doesn't care about exact pixel fidelity for
                // GUI navigation purposes.
                let params = CaptureScreenshotParams::builder()
                    .format(CaptureScreenshotFormat::Jpeg)
                    .quality(80)
                    .build();
                let bytes = self
                    .page
                    .screenshot(params)
                    .await
                    .map_err(|e| format!("screenshot failed: {e}"))?;
                tokio::fs::write(p, &bytes)
                    .await
                    .map_err(|e| format!("write screenshot {p:?}: {e}"))?;
                let encoded = general_purpose::STANDARD.encode(&bytes);
                (
                    Some(p.to_path_buf()),
                    Some(format!("data:image/jpeg;base64,{encoded}")),
                )
            }
            None => (None, None),
        };

        Ok(PageState {
            url,
            title,
            element_list,
            screenshot_path,
            screenshot_data_url,
        })
    }

    pub async fn click(&self, selector: &str) -> Result<(), String> {
        let el = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| format!("find_element({selector}): {e}"))?;
        el.click()
            .await
            .map_err(|e| format!("click({selector}): {e}"))?;
        Ok(())
    }

    pub async fn type_into(&self, selector: &str, text: &str) -> Result<(), String> {
        let el = self
            .page
            .find_element(selector)
            .await
            .map_err(|e| format!("find_element({selector}): {e}"))?;
        el.focus()
            .await
            .map_err(|e| format!("focus({selector}): {e}"))?;
        el.type_str(text)
            .await
            .map_err(|e| format!("type_str({selector}): {e}"))?;
        Ok(())
    }

    pub async fn press_key(&self, key: &str) -> Result<(), String> {
        // CDP Input.dispatchKeyEvent via evaluate is simplest for v0.
        let js = format!(
            r#"document.activeElement && document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {{ key: {key:?}, bubbles: true }}))"#
        );
        self.page
            .evaluate(js.as_str())
            .await
            .map_err(|e| format!("press_key({key}): {e}"))?;
        Ok(())
    }

    pub async fn scroll(&self, delta: i32) -> Result<(), String> {
        let js = format!("window.scrollBy(0, {delta})");
        self.page
            .evaluate(js.as_str())
            .await
            .map_err(|e| format!("scroll({delta}): {e}"))?;
        Ok(())
    }

    pub async fn close(mut self) -> Result<(), String> {
        let _ = self.inner.close().await;
        self.handler.abort();
        Ok(())
    }
}

/// Format the element list emitted by the injected JS into a compact
/// textual representation the brain consumes. Tight format ("0 button
/// \"Download\" #dl") saves ~50 chars/element vs the older fixed-width
/// layout — meaningful on dense pages where output tokens are 215x more
/// expensive than input tokens. Falls back to the raw JSON on parse error
/// so we don't lose information.
fn format_element_list(json_str: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Item {
        idx: u32,
        role: String,
        label: String,
        selector: Option<String>,
        #[allow(dead_code)]
        top: i32,
    }
    let parsed: Result<Vec<Item>, _> = serde_json::from_str(json_str);
    match parsed {
        Ok(items) => {
            let mut out = String::new();
            for it in items {
                // "0 button \"Download for macOS\" #dl\n"
                // Selector is optional — omitted when we don't have a stable one.
                let label = it.label.replace('"', "'");
                match it.selector.as_deref() {
                    Some(sel) => {
                        out.push_str(&format!("{} {} \"{}\" {}\n", it.idx, it.role, label, sel))
                    }
                    None => out.push_str(&format!("{} {} \"{}\"\n", it.idx, it.role, label)),
                }
            }
            if out.is_empty() {
                "(no interactable elements visible)".into()
            } else {
                out
            }
        }
        Err(_) => json_str.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{format_element_list, Browser, SnapshotOpts};

    #[test]
    fn format_element_list_renders_compact_lines() {
        let json = r##"[
            {"idx":0,"role":"button","label":"Download","selector":"#dl","top":420},
            {"idx":1,"role":"a","label":"Pricing","selector":"a[href=\"/pricing\"]","top":80}
        ]"##;
        let out = format_element_list(json);
        // New compact format: `0 button "Download" #dl\n1 a "Pricing" a[href=...]`
        assert!(out.contains("0 button \"Download\" #dl"), "{out}");
        assert!(out.contains("1 a \"Pricing\""), "{out}");
    }

    #[test]
    fn format_element_list_omits_selector_when_absent() {
        let json = r##"[{"idx":0,"role":"button","label":"Click","selector":null,"top":0}]"##;
        let out = format_element_list(json);
        assert_eq!(out.trim(), "0 button \"Click\"");
    }

    #[test]
    fn format_element_list_escapes_inner_quotes() {
        let json =
            r##"[{"idx":0,"role":"button","label":"Press \"Go\"","selector":"#g","top":0}]"##;
        let out = format_element_list(json);
        assert!(out.contains("\"Press 'Go'\""), "{out}");
    }

    #[test]
    fn format_element_list_handles_empty_list() {
        let out = format_element_list("[]");
        assert_eq!(out, "(no interactable elements visible)");
    }

    #[test]
    fn format_element_list_falls_back_on_bad_json() {
        let out = format_element_list("not-json-here");
        assert_eq!(out, "not-json-here");
    }

    /// End-to-end integration test against a real Chrome. Ignored by default;
    /// run with `cargo test -- --ignored agent::browser::tests::e2e` when you
    /// want to verify chromiumoxide wiring against the installed browser.
    #[tokio::test]
    #[ignore]
    async fn e2e_snapshot_and_click_against_real_chrome() {
        // Inline data: URL so the test is self-contained — no fixture files
        // and no network access required.
        let url = concat!(
            "data:text/html;charset=utf-8,",
            "%3Ctitle%3ETest%20Page%3C%2Ftitle%3E",
            "%3Cbutton%20id=%22b1%22%3EClick%20me%3C%2Fbutton%3E",
            "%3Ca%20id=%22home%22%20href=%22%2F%22%3EHome%3C%2Fa%3E",
        );
        let browser = Browser::launch().await.expect("launch chrome");
        browser.goto(url).await.expect("goto data:url");
        let state = browser
            .snapshot(SnapshotOpts::default())
            .await
            .expect("snapshot");
        assert_eq!(state.title, "Test Page", "title should populate");
        assert!(
            state.element_list.contains("Click me"),
            "expected button text in element list: {}",
            state.element_list,
        );
        assert!(
            state.element_list.contains("Home"),
            "expected link text in element list",
        );
        browser.click("#b1").await.expect("click button");
        browser.close().await.expect("close");
    }
}
