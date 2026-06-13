use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Hard-coded directories to always skip regardless of .gitignore.
const ALWAYS_SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".next",
    "dist",
    "build",
];

/// Walk a directory tree respecting .gitignore patterns and return a flat list
/// of entries.  Directories are listed first, then files, both alphabetical.
#[tauri::command]
pub async fn list_directory_tree(
    repo_path: String,
    max_depth: Option<u32>,
) -> Result<Value, String> {
    let root = PathBuf::from(&repo_path);
    if !root.is_dir() {
        return Err(format!("Not a directory: {repo_path}"));
    }

    let depth_limit = max_depth.unwrap_or(4);

    // Parse .gitignore from repo root (best-effort).
    let ignore_patterns = parse_gitignore(&root);

    let mut entries: Vec<Value> = Vec::new();
    walk_dir(&root, &root, 0, depth_limit, &ignore_patterns, &mut entries);

    Ok(json!({ "entries": entries }))
}

/// Read the first N lines of a file and detect language from extension.
#[tauri::command]
pub async fn read_file_preview(file_path: String, max_lines: Option<u32>) -> Result<Value, String> {
    let path = Path::new(&file_path);
    if !path.is_file() {
        return Err(format!("Not a file: {file_path}"));
    }

    let limit = max_lines.unwrap_or(100) as usize;

    let file = fs::File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
    let reader = BufReader::new(file);

    let mut lines_collected: Vec<String> = Vec::new();
    let mut total_lines: u32 = 0;

    for line in reader.lines() {
        total_lines += 1;
        match line {
            Ok(l) => {
                if lines_collected.len() < limit {
                    lines_collected.push(l);
                }
            }
            Err(_) => break, // binary or encoding issue — stop
        }
    }

    let content = lines_collected.join("\n");
    let language = detect_language(path);

    Ok(json!({
        "content": content,
        "total_lines": total_lines,
        "language": language,
    }))
}

/// Read lines around a specific line number in a file.
/// Returns context_before lines before and context_after lines after the target line.
#[tauri::command]
pub async fn read_file_around_line(
    file_path: String,
    line: u32,
    context_before: Option<u32>,
    context_after: Option<u32>,
) -> Result<Value, String> {
    let path = Path::new(&file_path);
    if !path.is_file() {
        return Err(format!("Not a file: {file_path}"));
    }

    let before = context_before.unwrap_or(10) as usize;
    let after = context_after.unwrap_or(10) as usize;
    let target = line as usize;
    let start = if target > before { target - before } else { 1 };
    let end = target + after;

    let file = fs::File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
    let reader = BufReader::new(file);

    let mut lines: Vec<Value> = Vec::new();
    for (i, result) in reader.lines().enumerate() {
        let line_num = i + 1;
        if line_num > end {
            break;
        }
        if line_num >= start {
            match result {
                Ok(text) => lines.push(json!({
                    "line": line_num,
                    "text": text,
                    "highlight": line_num == target,
                })),
                Err(_) => break,
            }
        }
    }

    let language = detect_language(path);

    Ok(json!({
        "lines": lines,
        "language": language,
        "target_line": target,
        "file_path": file_path,
    }))
}

/// Open a path in an external application (Cursor, VS Code, Finder, Terminal).
#[tauri::command]
pub async fn open_in_app(app_name: String, path: String) -> Result<Value, String> {
    let result = match app_name.as_str() {
        "cursor" => std::process::Command::new("open")
            .args(["-a", "Cursor", &path])
            .output(),
        "vscode" => std::process::Command::new("open")
            .args(["-a", "Visual Studio Code", &path])
            .output(),
        "finder" => std::process::Command::new("open").arg(&path).output(),
        "terminal" => std::process::Command::new("open")
            .args(["-a", "Terminal", &path])
            .output(),
        _ => return Err(format!("Unknown app: {app_name}")),
    };

    match result {
        Ok(output) if output.status.success() => Ok(json!({ "success": true })),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Failed to open {app_name}: {stderr}"))
        }
        Err(e) => Err(format!("Failed to launch: {e}")),
    }
}

// ─── Internal helpers ───────────────────────────────────────────────────────

fn walk_dir(
    root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    ignore_patterns: &[GlobPattern],
    out: &mut Vec<Value>,
) {
    if depth > max_depth {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    let mut files: Vec<(String, PathBuf, u64)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/dirs (starting with .) at root level
        // and always-skip directories at any level.
        if ALWAYS_SKIP.contains(&name.as_str()) {
            continue;
        }

        let rel_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        // Check gitignore patterns
        if is_ignored(&rel_path, path.is_dir(), ignore_patterns) {
            continue;
        }

        if path.is_dir() {
            dirs.push((name, path));
        } else if path.is_file() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            files.push((name, path, size));
        }
    }

    // Sort alphabetically
    dirs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    // Directories first
    for (name, path) in &dirs {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        out.push(json!({
            "path": rel,
            "name": name,
            "is_dir": true,
            "depth": depth,
            "size_bytes": null,
        }));
        walk_dir(root, path, depth + 1, max_depth, ignore_patterns, out);
    }

    // Then files
    for (name, path, size) in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        out.push(json!({
            "path": rel,
            "name": name,
            "is_dir": false,
            "depth": depth,
            "size_bytes": size,
        }));
    }
}

// ─── Simple gitignore parser ───────────────────────────────────────────────

struct GlobPattern {
    pattern: String,
    negated: bool,
    dir_only: bool,
}

fn parse_gitignore(root: &Path) -> Vec<GlobPattern> {
    let gitignore_path = root.join(".gitignore");
    let content = match fs::read_to_string(gitignore_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }

            let mut pattern = line.to_string();
            let negated = pattern.starts_with('!');
            if negated {
                pattern = pattern[1..].to_string();
            }

            let dir_only = pattern.ends_with('/');
            if dir_only {
                pattern = pattern.trim_end_matches('/').to_string();
            }

            Some(GlobPattern {
                pattern,
                negated,
                dir_only,
            })
        })
        .collect()
}

fn is_ignored(rel_path: &str, is_dir: bool, patterns: &[GlobPattern]) -> bool {
    let mut ignored = false;
    let name = Path::new(rel_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    for pat in patterns {
        if pat.dir_only && !is_dir {
            continue;
        }

        let matches = simple_glob_match(&pat.pattern, rel_path, &name);
        if matches {
            ignored = !pat.negated;
        }
    }

    ignored
}

/// Simple glob matching — handles `*`, `**`, and exact names.
fn simple_glob_match(pattern: &str, rel_path: &str, name: &str) -> bool {
    // If pattern contains '/', match against full relative path
    if pattern.contains('/') {
        let pattern = pattern.trim_start_matches('/');
        return glob_path_match(pattern, rel_path);
    }

    // Otherwise, match against the file/dir name only
    glob_path_match(pattern, name)
}

fn glob_path_match(pattern: &str, text: &str) -> bool {
    // Handle ** (matches everything)
    if pattern == "**" {
        return true;
    }

    // Simple wildcard: *.ext
    if let Some(ext) = pattern.strip_prefix("*.") {
        return text.ends_with(&format!(".{ext}"));
    }
    if pattern.starts_with('*') && !pattern.contains('/') {
        let suffix = &pattern[1..];
        return text.ends_with(suffix);
    }

    // Exact match
    if pattern == text {
        return true;
    }

    // Pattern is a directory name — match if rel_path starts with it
    if text.starts_with(pattern) && text[pattern.len()..].starts_with('/') {
        return true;
    }

    false
}

/// Detect programming language from file extension.
fn detect_language(path: &Path) -> String {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    match ext.as_str() {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "rs" => "rust",
        "py" => "python",
        "rb" => "ruby",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "css" => "css",
        "scss" | "sass" => "scss",
        "html" | "htm" => "html",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "mdx" => "markdown",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "shell",
        "dockerfile" => "dockerfile",
        "xml" => "xml",
        "vue" => "vue",
        "svelte" => "svelte",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "lua" => "lua",
        "r" => "r",
        "php" => "php",
        "graphql" | "gql" => "graphql",
        "proto" => "protobuf",
        _ => "plaintext",
    }
    .to_string()
}
