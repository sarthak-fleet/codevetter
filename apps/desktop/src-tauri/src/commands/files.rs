use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

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
        "reveal" => std::process::Command::new("open")
            .args(["-R", &path])
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
