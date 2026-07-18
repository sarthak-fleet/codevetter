use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

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

/// Open one repository-confined source coordinate in Cursor or VS Code.
///
/// The UI supplies the repository root separately from the persisted relative
/// path. Canonicalization rejects traversal and symlink escapes before any
/// external process receives a path.
#[tauri::command]
pub async fn open_repository_source_in_editor(
    app_name: String,
    repo_path: String,
    relative_path: String,
    line: u32,
    column: u32,
) -> Result<Value, String> {
    let app = match app_name.as_str() {
        "cursor" => "Cursor",
        "vscode" => "Visual Studio Code",
        _ => return Err("Unsupported source editor".to_string()),
    };
    let source = resolve_repository_source(&repo_path, &relative_path)?;
    let target = editor_goto_target(&source, line, column)?;
    let output = std::process::Command::new("open")
        .args(["-a", app, "--args", "--goto"])
        .arg(target)
        .output()
        .map_err(|_| "Failed to launch source editor".to_string())?;
    if !output.status.success() {
        return Err("Failed to open source coordinate in editor".to_string());
    }
    Ok(json!({ "success": true }))
}

fn resolve_repository_source(repo_path: &str, relative_path: &str) -> Result<PathBuf, String> {
    let root = Path::new(repo_path.trim())
        .canonicalize()
        .map_err(|_| "Repository source is unavailable".to_string())?;
    if !root.is_dir() {
        return Err("Repository source is unavailable".to_string());
    }
    let relative = Path::new(relative_path);
    if relative_path.is_empty()
        || relative_path.contains('\0')
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Repository source coordinate is invalid".to_string());
    }
    let source = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "Repository source is unavailable".to_string())?;
    if !source.starts_with(&root) || !source.is_file() {
        return Err("Repository source is unavailable".to_string());
    }
    Ok(source)
}

fn editor_goto_target(source: &Path, line: u32, column: u32) -> Result<String, String> {
    if line == 0 || column == 0 {
        return Err("Source coordinates must be one-based".to_string());
    }
    let source = source
        .to_str()
        .ok_or_else(|| "Repository source is unavailable".to_string())?;
    Ok(format!("{source}:{line}:{column}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn exact_source_target_is_repository_confined_and_one_based() {
        let repository = tempdir().expect("repository");
        let source = repository.path().join("legacy").join("PAYMENTS.cbl");
        fs::create_dir_all(source.parent().expect("source parent")).expect("source parent");
        fs::write(&source, "       IDENTIFICATION DIVISION.\n").expect("source");

        let resolved = resolve_repository_source(
            repository.path().to_str().expect("repository path"),
            "legacy/PAYMENTS.cbl",
        )
        .expect("confined source");
        assert_eq!(resolved, source.canonicalize().expect("canonical source"));
        assert_eq!(
            editor_goto_target(&resolved, 42, 8).expect("exact target"),
            format!("{}:42:8", resolved.to_string_lossy())
        );
        assert!(editor_goto_target(&resolved, 0, 8).is_err());
        assert!(resolve_repository_source(
            repository.path().to_str().expect("repository path"),
            "../outside.cbl"
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn exact_source_target_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let repository = tempdir().expect("repository");
        let outside = tempdir().expect("outside");
        let outside_source = outside.path().join("PAYMENTS.cbl");
        fs::write(&outside_source, "source\n").expect("outside source");
        symlink(&outside_source, repository.path().join("PAYMENTS.cbl")).expect("source link");

        assert!(resolve_repository_source(
            repository.path().to_str().expect("repository path"),
            "PAYMENTS.cbl"
        )
        .is_err());
    }
}
