use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_OUTPUT_CHARS: usize = 120_000;

#[derive(Clone)]
struct Candidate {
    tool: &'static str,
    label: Cow<'static, str>,
    path: PathBuf,
    source_kind: &'static str,
    note: &'static str,
}

#[derive(Clone, Serialize)]
pub struct AgentMemorySource {
    id: String,
    tool: String,
    label: String,
    path: String,
    exists: bool,
    readable: bool,
    file_size_bytes: Option<u64>,
    modified_at: Option<String>,
    source_kind: String,
    preview: String,
    note: String,
}

#[derive(Serialize)]
pub struct AgentMemoryDocument {
    source: AgentMemorySource,
    content: String,
    truncated: bool,
    extraction_note: String,
}

#[tauri::command]
pub fn list_agent_memory_sources() -> Result<Vec<AgentMemorySource>, String> {
    let mut out = Vec::new();
    for candidate in memory_candidates() {
        out.push(source_from_candidate(&candidate));
    }
    Ok(out)
}

#[tauri::command]
pub fn read_agent_memory_source(path: String) -> Result<AgentMemoryDocument, String> {
    let requested = PathBuf::from(&path);
    let candidate = find_allowed_candidate(&requested)
        .ok_or_else(|| "Path is not a known agent memory source.".to_string())?;

    if !candidate.path.is_file() {
        return Err(format!(
            "Memory source does not exist: {}",
            display_path(&candidate.path)
        ));
    }

    let mut file = fs::File::open(&candidate.path).map_err(|e| format!("Cannot open file: {e}"))?;
    let mut bytes = Vec::new();
    let read_limit = MAX_READ_BYTES.saturating_add(1);
    file.by_ref()
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read file: {e}"))?;

    let truncated_by_bytes = bytes.len() as u64 > MAX_READ_BYTES;
    if truncated_by_bytes {
        bytes.truncate(MAX_READ_BYTES as usize);
    }

    let raw = String::from_utf8_lossy(&bytes).to_string();
    let (mut content, extraction_note) = extract_memory_content(&candidate.path, &raw);
    let truncated_by_chars = content.chars().count() > MAX_OUTPUT_CHARS;
    if truncated_by_chars {
        content = content.chars().take(MAX_OUTPUT_CHARS).collect();
        content.push_str("\n\n[truncated]");
    }

    Ok(AgentMemoryDocument {
        source: source_from_candidate(&candidate),
        content,
        truncated: truncated_by_bytes || truncated_by_chars,
        extraction_note,
    })
}

fn memory_candidates() -> Vec<Candidate> {
    let mut candidates = Vec::new();

    if let Some(home) = home_dir() {
        for root in discover_profile_roots(&home, "claude", "CLAUDE_CONFIG_DIR") {
            add_claude_candidates(&mut candidates, root);
        }

        for root in discover_profile_roots(&home, "codex", "CODEX_HOME") {
            add_codex_candidates(&mut candidates, root);
        }

        add_cursor_candidates(&mut candidates);

        for root in discover_profile_roots(&home, "grok", "GROK_CONFIG_DIR") {
            add_grok_candidates(&mut candidates, root);
        }
        for root in discover_profile_roots(&home, "xai", "XAI_CONFIG_DIR") {
            add_grok_candidates(&mut candidates, root);
        }
    }

    dedupe_candidates(candidates)
}

fn discover_profile_roots(home: &Path, name: &str, env_var: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(config_dirs) = env::var(env_var) {
        for raw in config_dirs
            .split([',', ':'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            push_unique_path(&mut roots, PathBuf::from(raw));
        }
    }

    push_unique_path(&mut roots, home.join(format!(".{name}")));
    push_unique_path(&mut roots, home.join(".config").join(name));
    push_unique_path(
        &mut roots,
        home.join("Library")
            .join("Application Support")
            .join(capitalize_ascii(name)),
    );

    if let Ok(entries) = fs::read_dir(home) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();
            if name_str.starts_with(&format!(".{name}-")) {
                push_unique_path(&mut roots, path);
            }
        }
    }

    roots
}

fn add_claude_candidates(candidates: &mut Vec<Candidate>, root: PathBuf) {
    candidates.extend([
        Candidate {
            tool: "Claude",
            label: "Claude memory".into(),
            path: root.join("CLAUDE.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Claude",
            label: "Claude memory".into(),
            path: root.join("MEMORY.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Claude",
            label: "Claude memory".into(),
            path: root.join("memory.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Claude",
            label: "Claude config memory fields".into(),
            path: root.join(".claude.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
        Candidate {
            tool: "Claude",
            label: "Claude settings memory fields".into(),
            path: root.join("settings.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
    ]);
}

fn add_codex_candidates(candidates: &mut Vec<Candidate>, root: PathBuf) {
    candidates.extend([
        Candidate {
            tool: "Codex",
            label: "Codex instructions".into(),
            path: root.join("AGENTS.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex memory registry".into(),
            path: root.join("memories").join("MEMORY.md"),
            source_kind: "markdown",
            note: "Full Markdown memory registry.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex memory summary".into(),
            path: root.join("memories").join("memory_summary.md"),
            source_kind: "markdown",
            note: "Full Markdown memory summary.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex raw memories".into(),
            path: root.join("memories").join("raw_memories.md"),
            source_kind: "markdown",
            note: "Full Markdown raw memory file.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex memory".into(),
            path: root.join("MEMORY.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex memory".into(),
            path: root.join("memory.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex config memory fields".into(),
            path: root.join("config.toml"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex global state memory fields".into(),
            path: root.join("config.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
        Candidate {
            tool: "Codex",
            label: "Codex global state memory fields".into(),
            path: root.join(".codex-global-state.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
    ]);

    let automations = root.join("automations");
    if let Ok(entries) = fs::read_dir(automations) {
        for entry in entries.flatten() {
            let memory_path = entry.path().join("memory.md");
            let label = entry.file_name().to_string_lossy().replace(['_', '-'], " ");
            candidates.push(Candidate {
                tool: "Codex",
                label: format!("Codex automation memory: {label}").into(),
                path: memory_path,
                source_kind: "markdown",
                note: "Full Markdown automation memory.",
            });
        }
    }
}

fn add_cursor_candidates(candidates: &mut Vec<Candidate>) {
    for workspace in discover_cursor_workspaces() {
        candidates.push(Candidate {
            tool: "Cursor",
            label: "Cursor rules".into(),
            path: workspace.join(".cursorrules"),
            source_kind: "markdown",
            note: "Cursor repo rules file.",
        });

        let rules_dir = workspace.join(".cursor").join("rules");
        if let Ok(entries) = fs::read_dir(&rules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !matches!(ext.as_str(), "md" | "mdc" | "txt") {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("rules");
                candidates.push(Candidate {
                    tool: "Cursor",
                    label: format!("Cursor rule: {name}").into(),
                    path,
                    source_kind: "markdown",
                    note: "Cursor per-repo rule file.",
                });
            }
        }
    }
}

fn discover_cursor_workspaces() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home_dir() {
        push_unique_path(
            &mut roots,
            home.join("Desktop").join("fleet").join("CodeVetter"),
        );
    }

    let db_path = resolve_cursor_global_db();
    if !db_path.is_file() {
        return roots;
    }

    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(conn) => conn,
        Err(_) => return roots,
    };

    let mut stmt =
        match conn.prepare("SELECT value FROM cursorDiskKV WHERE key LIKE 'composerData:%'") {
            Ok(stmt) => stmt,
            Err(_) => return roots,
        };

    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(_) => return roots,
    };

    for raw in rows.flatten() {
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            collect_cursor_workspace_paths(&value, &mut roots);
        }
    }

    roots
}

fn collect_cursor_workspace_paths(value: &Value, roots: &mut Vec<PathBuf>) {
    let composer = value.get("composer").unwrap_or(value);

    if let Some(path) = composer
        .pointer("/workspaceIdentifier/uri/fsPath")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        push_unique_path(roots, PathBuf::from(path));
    }

    if let Some(repos) = composer
        .get("trackedGitRepos")
        .and_then(|value| value.as_array())
    {
        for repo in repos {
            if let Some(path) = repo
                .get("path")
                .or_else(|| repo.get("repoPath"))
                .or_else(|| repo.get("rootPath"))
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
            {
                push_unique_path(roots, PathBuf::from(path));
            }
        }
    }
}

fn resolve_cursor_global_db() -> PathBuf {
    if let Some(home) = home_dir() {
        if cfg!(target_os = "macos") {
            return home
                .join("Library")
                .join("Application Support")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb");
        }
        if cfg!(target_os = "linux") {
            return home
                .join(".config")
                .join("Cursor")
                .join("User")
                .join("globalStorage")
                .join("state.vscdb");
        }
    }

    if let Ok(appdata) = env::var("APPDATA") {
        return PathBuf::from(appdata)
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb");
    }

    PathBuf::from("state.vscdb")
}

fn add_grok_candidates(candidates: &mut Vec<Candidate>, root: PathBuf) {
    candidates.extend([
        Candidate {
            tool: "Grok",
            label: "Grok instructions".into(),
            path: root.join("GROK.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Grok",
            label: "Grok memory".into(),
            path: root.join("MEMORY.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Grok",
            label: "Grok memory".into(),
            path: root.join("memory.md"),
            source_kind: "markdown",
            note: "Full Markdown memory file.",
        },
        Candidate {
            tool: "Grok",
            label: "Grok config memory fields".into(),
            path: root.join("config.toml"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
        Candidate {
            tool: "Grok",
            label: "Grok config memory fields".into(),
            path: root.join("config.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
        Candidate {
            tool: "Grok",
            label: "Grok settings memory fields".into(),
            path: root.join("settings.json"),
            source_kind: "config",
            note: "Memory-like fields only; secret-looking lines are redacted.",
        },
    ]);
}

fn dedupe_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for candidate in candidates {
        let key = candidate.path.to_string_lossy().to_string();
        if seen.insert(key) {
            out.push(candidate);
        }
    }

    out
}

fn source_from_candidate(candidate: &Candidate) -> AgentMemorySource {
    let metadata = fs::metadata(&candidate.path).ok();
    let exists = metadata.as_ref().is_some_and(|m| m.is_file());
    let readable = exists && fs::File::open(&candidate.path).is_ok();
    let preview = if readable {
        read_preview(candidate).unwrap_or_default()
    } else {
        String::new()
    };

    AgentMemorySource {
        id: stable_id(&candidate.path),
        tool: candidate.tool.to_string(),
        label: candidate.label.to_string(),
        path: candidate.path.to_string_lossy().to_string(),
        exists,
        readable,
        file_size_bytes: metadata.as_ref().map(|m| m.len()),
        modified_at: metadata
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()),
        source_kind: candidate.source_kind.to_string(),
        preview,
        note: candidate.note.to_string(),
    }
}

fn read_preview(candidate: &Candidate) -> Result<String, String> {
    let mut file = fs::File::open(&candidate.path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(32 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let raw = String::from_utf8_lossy(&bytes);
    let (content, _) = extract_memory_content(&candidate.path, &raw);
    Ok(content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
        .chars()
        .take(180)
        .collect())
}

fn extract_memory_content(path: &Path, raw: &str) -> (String, String) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if matches!(ext.as_str(), "md" | "markdown" | "txt") {
        return (
            redact_content(raw),
            "Showing the full text with secret-looking lines redacted.".to_string(),
        );
    }

    if ext == "json" {
        if let Ok(value) = serde_json::from_str::<Value>(raw) {
            let mut lines = Vec::new();
            collect_json_memory_lines("$", &value, false, &mut lines);
            if lines.is_empty() {
                return (
                    "No memory-like fields found in this config.".to_string(),
                    "Parsed JSON and found no memory/instruction/context fields.".to_string(),
                );
            }
            return (
                lines.join("\n"),
                "Showing memory/instruction/context fields from JSON only.".to_string(),
            );
        }
    }

    let lines = extract_keyword_lines(raw);
    if lines.is_empty() {
        (
            "No memory-like lines found in this config.".to_string(),
            "Scanned text and found no memory/instruction/context lines.".to_string(),
        )
    } else {
        (
            lines.join("\n"),
            "Showing matching memory/instruction/context lines only.".to_string(),
        )
    }
}

fn collect_json_memory_lines(
    path: &str,
    value: &Value,
    in_memory_context: bool,
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if is_secret_key(key) {
                    continue;
                }
                let child_path = format!("{path}.{key}");
                let matched = in_memory_context || is_memory_key(key);
                collect_json_memory_lines(&child_path, child, matched, out);
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                collect_json_memory_lines(&format!("{path}[{idx}]"), item, in_memory_context, out);
            }
        }
        Value::String(text) if in_memory_context => {
            if !text.trim().is_empty() {
                out.push(format!("{path}: {}", redact_line(text.trim())));
            }
        }
        Value::Bool(_) | Value::Number(_) if in_memory_context => {
            out.push(format!("{path}: {value}"));
        }
        _ => {}
    }
}

fn extract_keyword_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            is_memory_text(&lower) && !is_secret_text(&lower)
        })
        .map(|line| redact_line(line.trim()))
        .filter(|line| !line.is_empty())
        .collect()
}

fn redact_content(raw: &str) -> String {
    raw.lines().map(redact_line).collect::<Vec<_>>().join("\n")
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if is_secret_text(&lower) {
        "[redacted secret-like line]".to_string()
    } else {
        line.to_string()
    }
}

fn is_memory_key(key: &str) -> bool {
    is_memory_text(&key.to_ascii_lowercase())
}

fn is_memory_text(lower: &str) -> bool {
    lower.contains("memory")
        || lower.contains("memories")
        || lower.contains("instruction")
        || lower.contains("instructions")
        || lower.contains("context")
        || lower.contains("guideline")
        || lower.contains("guidelines")
        || lower.contains("rules")
        || lower.contains("prompt")
}

fn is_secret_key(key: &str) -> bool {
    is_secret_text(&key.to_ascii_lowercase())
}

fn is_secret_text(lower: &str) -> bool {
    lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("auth_token")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("credential_secret")
        || lower.contains("authorization")
        || lower.contains("bearer ")
        || lower.contains("private_key")
}

fn find_allowed_candidate(requested: &Path) -> Option<Candidate> {
    let requested_canonical = fs::canonicalize(requested).ok()?;

    memory_candidates().into_iter().find(|candidate| {
        candidate.path.is_file()
            && fs::canonicalize(&candidate.path)
                .map(|path| path == requested_canonical)
                .unwrap_or(false)
    })
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

fn display_path(path: &Path) -> String {
    if let Some(home) = home_dir() {
        if let Ok(stripped) = path.strip_prefix(&home) {
            return format!("~/{}", stripped.to_string_lossy());
        }
    }
    path.to_string_lossy().to_string()
}

fn stable_id(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}
