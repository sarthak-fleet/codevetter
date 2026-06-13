//! Blast-radius analyzer.
//!
//! Given a repo path and a diff range, extracts symbols (functions, classes,
//! methods, exported consts) introduced or modified by the diff and uses
//! `git grep` to find every other place in the repo that references them.
//!
//! Deterministic — no LLM. The output is a graph-aware view of what the PR
//! actually touches downstream: "this function has 47 callers, here are 10
//! of them."

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::process::Command as StdCommand;

// ─── Public types (mirrored on the TS side) ─────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct CallerSite {
    pub file: String,
    pub line: usize,
    pub snippet: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct BlastRadiusSymbol {
    pub name: String,
    pub kind: String,
    pub language: String,
    #[serde(rename = "definedIn")]
    pub defined_in: String,
    pub callers: Vec<CallerSite>,
    #[serde(rename = "callerCount")]
    pub caller_count: usize,
    /// "safe" (0) | "medium" (1-5) | "high" (6+)
    pub risk: String,
}

#[derive(Debug, Serialize)]
pub struct BlastRadiusReport {
    pub symbols: Vec<BlastRadiusSymbol>,
    #[serde(rename = "totalSymbols")]
    pub total_symbols: usize,
    #[serde(rename = "totalCallers")]
    pub total_callers: usize,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    #[serde(rename = "changedFiles")]
    pub changed_files: usize,
}

// ─── Language detection ─────────────────────────────────────────────────────

fn language_for(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    Some(match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => "ts",
        "py" => "py",
        "rs" => "rs",
        "go" => "go",
        _ => return None,
    })
}

// ─── Symbol extraction ──────────────────────────────────────────────────────

/// A lightweight pattern match for a symbol definition on a single line.
/// Returns (name, kind) if the line defines something interesting.
fn extract_definitions_from_line(lang: &str, line: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let trimmed = line.trim_start();

    match lang {
        "ts" => {
            // `function foo(` / `export function foo(` / `export async function foo(`
            if let Some(rest) = strip_prefix_any(
                trimmed,
                &[
                    "export default async function ",
                    "export default function ",
                    "export async function ",
                    "export function ",
                    "async function ",
                    "function ",
                ],
            ) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "function"));
                }
            }
            // `class Foo` / `export class Foo`
            if let Some(rest) = strip_prefix_any(
                trimmed,
                &[
                    "export default class ",
                    "export abstract class ",
                    "export class ",
                    "abstract class ",
                    "class ",
                ],
            ) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "class"));
                }
            }
            // `export const foo = ` / `const foo = (...) =>`
            if let Some(rest) =
                strip_prefix_any(trimmed, &["export const ", "export let ", "const ", "let "])
            {
                if let Some(name) = take_ident(rest) {
                    // Only capture if it's assigned to a function/arrow
                    // (otherwise every local variable matches — too noisy)
                    if rest.contains("=>") || rest.contains("function") || rest.contains("= async")
                    {
                        out.push((name, "const-fn"));
                    }
                }
            }
            // class method: `  methodName(args) {` — must look like a member
            // Heuristic: starts with identifier, has `(`, ends with `{` (roughly)
            if let Some(name) = maybe_method(trimmed) {
                out.push((name, "method"));
            }
        }
        "py" => {
            if let Some(rest) = strip_prefix_any(trimmed, &["def ", "async def "]) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "function"));
                }
            }
            if let Some(rest) = strip_prefix_any(trimmed, &["class "]) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "class"));
                }
            }
        }
        "rs" => {
            if let Some(rest) =
                strip_prefix_any(trimmed, &["pub async fn ", "pub fn ", "async fn ", "fn "])
            {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "function"));
                }
            }
            if let Some(rest) = strip_prefix_any(trimmed, &["pub struct ", "struct "]) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "struct"));
                }
            }
            if let Some(rest) = strip_prefix_any(trimmed, &["pub enum ", "enum "]) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "enum"));
                }
            }
        }
        "go" => {
            // `func Foo(` or `func (r *Recv) Foo(`
            if let Some(rest) = strip_prefix_any(trimmed, &["func "]) {
                let after_recv = if rest.starts_with('(') {
                    // skip receiver `(r *T) `
                    rest.find(')')
                        .map(|i| rest[i + 1..].trim_start())
                        .unwrap_or(rest)
                } else {
                    rest
                };
                if let Some(name) = take_ident(after_recv) {
                    out.push((name, "function"));
                }
            }
            if let Some(rest) = strip_prefix_any(trimmed, &["type "]) {
                if let Some(name) = take_ident(rest) {
                    out.push((name, "type"));
                }
            }
        }
        _ => {}
    }

    out
}

fn strip_prefix_any<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    for p in prefixes {
        if let Some(rest) = s.strip_prefix(p) {
            return Some(rest);
        }
    }
    None
}

/// Take the leading identifier from a string (alphanumeric + underscore).
fn take_ident(s: &str) -> Option<String> {
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let name = &s[..end];
    // Skip reserved-ish / too-short / generic names
    if name.len() < 2 {
        return None;
    }
    Some(name.to_string())
}

/// Heuristic for "this line looks like a class method".
/// e.g. `  handleClick(e: Event) {` or `  async submit() {`
fn maybe_method(line: &str) -> Option<String> {
    // Skip obvious non-method starts
    if line.starts_with("if ")
        || line.starts_with("for ")
        || line.starts_with("while ")
        || line.starts_with("switch ")
        || line.starts_with("return ")
        || line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
        || line.starts_with('@')
    {
        return None;
    }
    let mut work = line;
    // Allow leading modifiers
    for kw in [
        "public ",
        "private ",
        "protected ",
        "static ",
        "readonly ",
        "async ",
        "get ",
        "set ",
    ] {
        if let Some(rest) = work.strip_prefix(kw) {
            work = rest;
        }
    }
    let name = take_ident(work)?;
    let after = &work[name.len()..];
    // Must be followed by `(` (possibly with generics: `<T>(...)`)
    let after = after.trim_start();
    let after = after
        .strip_prefix('<')
        .and_then(|r| r.find('>').map(|i| r[i + 1..].trim_start()))
        .unwrap_or(after);
    if !after.starts_with('(') {
        return None;
    }
    // Must end with `{` (open body) — avoid matching function calls
    if !line.trim_end().ends_with('{') && !line.trim_end().ends_with("=>") {
        return None;
    }
    Some(name)
}

// ─── Diff parsing ───────────────────────────────────────────────────────────

/// Parse unified diff output into `(file, added_lines)` pairs.
/// We only look at lines that start with `+` (added) since those are what
/// introduced or modified a definition.
fn parse_diff_added_lines(diff: &str) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            current_file = Some(rest.to_string());
            continue;
        }
        if line.starts_with("+++ ") || line.starts_with("--- ") || line.starts_with("diff --git") {
            continue;
        }
        if let Some(content) = line.strip_prefix('+') {
            if let Some(ref f) = current_file {
                out.entry(f.clone()).or_default().push(content.to_string());
            }
        }
    }
    out
}

// ─── git grep wrapper ───────────────────────────────────────────────────────

const MAX_CALLERS_PER_SYMBOL: usize = 25;

fn find_callers(repo_path: &str, symbol: &str, defined_in: &str) -> Vec<CallerSite> {
    // `git grep -n -w -- symbol` → file:line:content lines
    let output = StdCommand::new("git")
        .args(["grep", "-n", "-w", "-I", "--", symbol])
        .current_dir(repo_path)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        // git grep exits with 1 on no match — treat as empty result
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut sites = Vec::new();

    for raw in stdout.lines() {
        // Format: path:line:content
        let mut parts = raw.splitn(3, ':');
        let file = parts.next().unwrap_or("");
        let line_str = parts.next().unwrap_or("");
        let snippet = parts.next().unwrap_or("");

        if file.is_empty() || line_str.is_empty() {
            continue;
        }
        if file == defined_in {
            continue;
        }
        let Ok(line_no) = line_str.parse::<usize>() else {
            continue;
        };

        // Skip the definition line itself (e.g. if defined in another file
        // with the same name — which is unusual but possible for overloads)
        let snippet_trim = snippet.trim();
        if is_definition_line(snippet_trim) {
            continue;
        }

        sites.push(CallerSite {
            file: file.to_string(),
            line: line_no,
            snippet: snippet_trim.chars().take(200).collect(),
        });

        if sites.len() >= MAX_CALLERS_PER_SYMBOL {
            break;
        }
    }

    sites
}

fn is_definition_line(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with("function ")
        || s.starts_with("export function ")
        || s.starts_with("export async function ")
        || s.starts_with("async function ")
        || s.starts_with("class ")
        || s.starts_with("export class ")
        || s.starts_with("def ")
        || s.starts_with("async def ")
        || s.starts_with("fn ")
        || s.starts_with("pub fn ")
        || s.starts_with("pub async fn ")
        || s.starts_with("struct ")
        || s.starts_with("pub struct ")
        || s.starts_with("func ")
}

fn risk_bucket(caller_count: usize) -> &'static str {
    match caller_count {
        0 => "safe",
        1..=5 => "medium",
        _ => "high",
    }
}

// ─── Tauri command ──────────────────────────────────────────────────────────

/// Run the full blast-radius analysis and return the typed report.
/// Shared between the Tauri command and `run_cli_review` so the LLM prompt
/// can include graph context without the frontend having to thread the
/// report back through IPC.
pub fn compute_blast_radius(
    repo_path: &str,
    diff_range: &str,
) -> Result<BlastRadiusReport, String> {
    let start = std::time::Instant::now();

    let diff_out = StdCommand::new("git")
        .args(["diff", diff_range])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("git diff failed: {e}"))?;
    if !diff_out.status.success() {
        let stderr = String::from_utf8_lossy(&diff_out.stderr);
        return Err(format!("git diff failed: {stderr}"));
    }
    let diff_text = String::from_utf8_lossy(&diff_out.stdout).to_string();
    if diff_text.trim().is_empty() {
        return Ok(BlastRadiusReport {
            symbols: vec![],
            total_symbols: 0,
            total_callers: 0,
            duration_ms: 0,
            changed_files: 0,
        });
    }

    let added = parse_diff_added_lines(&diff_text);
    let changed_files = added.len();

    let mut by_symbol: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut _seen_names: HashSet<String> = HashSet::new();

    for (file, lines) in &added {
        let Some(lang) = language_for(file) else {
            continue;
        };
        for line in lines {
            let defs = extract_definitions_from_line(lang, line);
            for (name, kind) in defs {
                if name.len() < 3 {
                    continue;
                }
                let key = (name.clone(), file.clone());
                by_symbol
                    .entry(key)
                    .or_insert_with(|| (kind.to_string(), lang.to_string()));
                _seen_names.insert(name);
            }
        }
    }

    let mut symbols: Vec<BlastRadiusSymbol> = Vec::new();
    let mut total_callers = 0usize;

    for ((name, defined_in), (kind, language)) in by_symbol {
        let callers = find_callers(repo_path, &name, &defined_in);
        let caller_count = callers.len();
        total_callers += caller_count;
        let risk = risk_bucket(caller_count).to_string();
        symbols.push(BlastRadiusSymbol {
            name,
            kind,
            language,
            defined_in,
            callers,
            caller_count,
            risk,
        });
    }

    symbols.sort_by(|a, b| {
        let rank = |r: &str| match r {
            "high" => 0,
            "medium" => 1,
            _ => 2,
        };
        rank(&a.risk)
            .cmp(&rank(&b.risk))
            .then_with(|| b.caller_count.cmp(&a.caller_count))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(BlastRadiusReport {
        total_symbols: symbols.len(),
        total_callers,
        duration_ms: start.elapsed().as_millis() as u64,
        changed_files,
        symbols,
    })
}

/// Build a short prose summary of the most impactful symbols, for use in
/// an LLM review prompt. Returns `None` when there's nothing worth saying.
pub fn summarize_for_prompt(report: &BlastRadiusReport) -> Option<String> {
    if report.total_symbols == 0 {
        return None;
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "Graph context (blast-radius scan over {} file{}, {} symbol{}, {} caller{} found):",
        report.changed_files,
        if report.changed_files == 1 { "" } else { "s" },
        report.total_symbols,
        if report.total_symbols == 1 { "" } else { "s" },
        report.total_callers,
        if report.total_callers == 1 { "" } else { "s" },
    ));

    // Show up to 8 symbols: all high + as many medium/safe as fit
    let mut count = 0;
    for s in &report.symbols {
        if count >= 8 {
            break;
        }
        let risk_note = match s.risk.as_str() {
            "high" => "HIGH RISK — many callers, scrutinize for contract breaks",
            "medium" => "some callers — verify behavior is preserved",
            _ => "no callers — newly added or unused",
        };
        lines.push(format!(
            "  - {} ({} in {}): {} caller{} — {}",
            s.name,
            s.kind,
            s.defined_in,
            s.caller_count,
            if s.caller_count == 1 { "" } else { "s" },
            risk_note,
        ));
        count += 1;
    }

    if report.total_symbols > 8 {
        lines.push(format!("  (…and {} more)", report.total_symbols - 8));
    }

    lines.push(
        "When you comment, prefer issues that affect high-caller symbols; they carry the most risk.".to_string(),
    );

    Some(lines.join("\n"))
}

/// Analyze the blast radius of a diff: extract symbols from the diff and
/// count every caller in the repo.
#[tauri::command]
pub async fn analyze_blast_radius(repo_path: String, diff_range: String) -> Result<Value, String> {
    let report = compute_blast_radius(&repo_path, &diff_range)?;
    Ok(json!(report))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ts_function() {
        let defs = extract_definitions_from_line("ts", "export function fooBar(a: number) {");
        assert_eq!(defs, vec![("fooBar".to_string(), "function")]);
    }

    #[test]
    fn extracts_rs_fn() {
        let defs =
            extract_definitions_from_line("rs", "pub async fn run_review() -> Result<(), Error> {");
        assert_eq!(defs, vec![("run_review".to_string(), "function")]);
    }

    #[test]
    fn extracts_py_def() {
        let defs = extract_definitions_from_line("py", "def process_batch(items):");
        assert_eq!(defs, vec![("process_batch".to_string(), "function")]);
    }

    #[test]
    fn extracts_go_func_with_receiver() {
        let defs = extract_definitions_from_line(
            "go",
            "func (s *Server) HandleRequest(ctx context.Context) error {",
        );
        assert_eq!(defs, vec![("HandleRequest".to_string(), "function")]);
    }

    #[test]
    fn skips_short_names() {
        let defs = extract_definitions_from_line("rs", "fn x() {}");
        assert!(defs.is_empty());
    }

    #[test]
    fn skips_if_statements() {
        assert!(maybe_method("if (x > 0) {").is_none());
    }

    #[test]
    fn risk_buckets() {
        assert_eq!(risk_bucket(0), "safe");
        assert_eq!(risk_bucket(3), "medium");
        assert_eq!(risk_bucket(50), "high");
    }

    #[test]
    fn parses_added_lines() {
        let diff = "diff --git a/foo.ts b/foo.ts\n--- a/foo.ts\n+++ b/foo.ts\n@@ -1,1 +1,2 @@\n-old line\n+new line\n+another\n";
        let m = parse_diff_added_lines(diff);
        assert_eq!(
            m.get("foo.ts").unwrap(),
            &vec!["new line".to_string(), "another".to_string()]
        );
    }
}
