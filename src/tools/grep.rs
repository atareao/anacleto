//! The `grep` tool: search files for a regex pattern.
//!
//! Uses ripgrep (`rg`) when it is available on the system, falling back to a
//! `std`-only implementation that walks the target directory and matches each
//! line with the built-in regex engine. Results are returned as
//! `path:line:content`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::permissions::checker::check_fs_read;
use crate::permissions::types::Permissions;

/// Maximum number of matches returned per `grep` call.
const MAX_MATCHES: usize = 500;

/// Tool definition for the `grep` tool.
pub fn grep_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "grep".to_string(),
        description: "Search files for a regular expression pattern. Returns \
                       matches as 'path:line:content'. `path` optionally limits \
                       the search to a file or directory (relative to the \
                       workspace); `include` optionally filters files by glob \
                       (e.g. '*.rs')."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression to search for."
                },
                "path": {
                    "type": "string",
                    "description": "Optional file or directory to search (relative to the workspace)."
                },
                "include": {
                    "type": "string",
                    "description": "Optional glob to filter files, e.g. '*.rs'."
                }
            },
            "required": ["pattern"]
        }),
    }
}

/// Return `true` if ripgrep is available on the system.
fn rg_available() -> bool {
    std::process::Command::new("rg")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Execute a `grep` tool call.
pub async fn execute_grep_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse grep arguments: {e}"))?;
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "grep requires 'pattern'".to_string())?;
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let include = args.get("include").and_then(|v| v.as_str());

    check_fs_read(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // Resolve the search target within the workspace.
    let target = if path.is_empty() {
        workspace.to_path_buf()
    } else {
        crate::engine::apply_patch::resolve_within_workspace(workspace, path)?
    };

    if !target.exists() {
        return Err(format!("Path does not exist: {}", target.display()));
    }

    if rg_available() {
        grep_with_rg(&target, pattern, include).await
    } else {
        grep_fallback(&target, pattern, include)
    }
}

/// Run ripgrep over the target and format the output.
async fn grep_with_rg(
    target: &Path,
    pattern: &str,
    include: Option<&str>,
) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .arg("-e")
        .arg(pattern);
    if let Some(inc) = include {
        cmd.arg("--glob").arg(inc);
    }
    cmd.arg(target);

    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "grep timed out after 60 seconds".to_string())?
        .map_err(|e| format!("Failed to run ripgrep: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if stdout.trim().is_empty() {
            Ok("No matches found.".to_string())
        } else {
            Ok(truncate_matches(&stdout))
        }
    } else if output.status.code() == Some(1) {
        // Exit code 1 means no matches.
        Ok("No matches found.".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        Err(format!("ripgrep failed: {}", stderr.trim()))
    }
}

/// Recursively collect files under `dir`.
fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_files(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
}

/// Fallback grep implementation using `std::fs` and the built-in regex engine.
fn grep_fallback(target: &Path, pattern: &str, include: Option<&str>) -> Result<String, String> {
    // Validate the pattern up-front so a bad regex is reported clearly.
    let _ = crate::tools::pattern::regex_is_match(pattern, "")?;

    let mut files: Vec<PathBuf> = Vec::new();
    if target.is_file() {
        files.push(target.to_path_buf());
    } else {
        walk_files(target, &mut files);
    }

    let mut out = String::new();
    let mut count = 0usize;
    for file in files {
        if let Some(inc) = include {
            let rel = file
                .strip_prefix(target)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            if !crate::tools::pattern::glob_match(inc, &rel) {
                continue;
            }
        }
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let content = String::from_utf8_lossy(&bytes).into_owned();
        for (idx, line) in content.lines().enumerate() {
            if count >= MAX_MATCHES {
                out.push_str(&format!(
                    "... (truncated at {MAX_MATCHES} matches; refine your pattern)\n"
                ));
                return Ok(out);
            }
            if crate::tools::pattern::regex_is_match(pattern, line)? {
                out.push_str(&format!("{}:{}:{}\n", file.display(), idx + 1, line));
                count += 1;
            }
        }
    }

    if out.is_empty() {
        Ok("No matches found.".to_string())
    } else {
        Ok(out)
    }
}

/// Cap the number of returned matches to `MAX_MATCHES` lines.
fn truncate_matches(output: &str) -> String {
    let mut out = String::new();
    for (count, line) in output.lines().enumerate() {
        if count >= MAX_MATCHES {
            out.push_str(&format!(
                "... (truncated at {MAX_MATCHES} matches; refine your pattern)\n"
            ));
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;
    use crate::permissions::types::Permissions;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anacleto_grep_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_grep".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "grep".into(),
                arguments: args.into(),
            },
        }
    }

    fn allow_all() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec![],
        })
    }

    #[tokio::test]
    async fn grep_finds_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.rs"), "fn main() {}\nlet x = 1;\n").unwrap();
        std::fs::write(ws.join("b.txt"), "no match here\n").unwrap();
        let result = execute_grep_tool(&ws, &allow_all(), &tool_call(r#"{"pattern":"fn main"}"#))
            .await
            .unwrap();
        assert!(result.contains("a.rs:1:fn main() {}"));
        assert!(!result.contains("b.txt"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn grep_no_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "hello\n").unwrap();
        let result = execute_grep_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"pattern":"zzz_nonexistent"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("No matches"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn grep_with_include_glob() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.rs"), "needle\n").unwrap();
        std::fs::write(ws.join("b.txt"), "needle\n").unwrap();
        let result = execute_grep_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"pattern":"needle","include":"*.rs"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("a.rs"));
        assert!(!result.contains("b.txt"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn grep_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_grep_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"pattern":"x","path":"../etc"}"#),
        )
        .await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
