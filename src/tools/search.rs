//! Unified search tool (`search`) — replaces `grep` and `glob`.
//!
//! Single tool with a `mode` field: `"content"` (regex search inside files,
//! replacing grep) or `"files"` (glob pattern matching on file names,
//! replacing glob).
//!
//! ## Content search (mode: "content")
//!
//! Searches file contents for a regex pattern. Uses ripgrep (`rg`) when
//! available, falling back to a `std`-only implementation. Returns results
//! as `path:line:content`.
//!
//! ## File search (mode: "files")
//!
//! Lists files matching a glob pattern within the workspace.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::permissions::checker::check_fs_read;
use crate::permissions::types::Permissions;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of matches returned per content search.
const MAX_MATCHES: usize = 500;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

/// Tool definition for the unified `search` tool.
pub fn search_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "search".to_string(),
        description:
            "Unified search tool. Use mode=\"content\" to search file contents with a regex \
             pattern (replaces grep), or mode=\"files\" to list files matching a glob pattern \
             (replaces glob)."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["content", "files"],
                    "description": "Search mode: 'content' for regex inside files, 'files' for glob pattern on names"
                },
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern (content mode) or glob pattern (files mode)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search (defaults to workspace root)"
                },
                "include": {
                    "type": "string",
                    "description": "Glob filter for file names (content mode only, e.g. '*.rs')"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Maximum results to return"
                },
                "context_lines": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 10,
                    "description": "Lines of context before/after each match (content mode only)"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether the search is case-sensitive (content mode only)"
                }
            },
            "required": ["mode", "pattern"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Cap the number of returned lines to `max` lines.
fn truncate_lines(output: &str, max: usize) -> String {
    let mut out = String::new();
    for (count, line) in output.lines().enumerate() {
        if count >= max {
            out.push_str(&format!(
                "... (truncated at {max} results; refine your pattern)\n"
            ));
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Content search (replaces grep)
// ---------------------------------------------------------------------------

/// Execute a content search using ripgrep.
fn search_content_with_rg(
    target: &Path,
    pattern: &str,
    include: Option<&str>,
    context_lines: usize,
    case_sensitive: bool,
    max_results: usize,
) -> Result<String, String> {
    let mut cmd = std::process::Command::new("rg");
    cmd.arg("--line-number")
        .arg("--no-heading")
        .arg("--color")
        .arg("never")
        .arg("-e")
        .arg(pattern);

    if let Some(inc) = include {
        cmd.arg("--glob").arg(inc);
    }

    if context_lines > 0 {
        cmd.arg("-C").arg(context_lines.to_string());
    }

    if !case_sensitive {
        cmd.arg("-i");
    }

    cmd.arg(target);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run ripgrep: {e}"))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if stdout.trim().is_empty() {
            Ok("No matches found.".to_string())
        } else {
            Ok(truncate_lines(&stdout, max_results))
        }
    } else if output.status.code() == Some(1) {
        Ok("No matches found.".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        Err(format!("ripgrep failed: {}", stderr.trim()))
    }
}

/// Fallback content search using `std::fs` and the built-in regex engine.
fn search_content_fallback(
    target: &Path,
    pattern: &str,
    include: Option<&str>,
    max_results: usize,
) -> Result<String, String> {
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
            if count >= max_results {
                out.push_str(&format!(
                    "... (truncated at {max_results} matches; refine your pattern)\n"
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

// ---------------------------------------------------------------------------
// File search (replaces glob)
// ---------------------------------------------------------------------------

/// Execute a file search (glob pattern matching).
fn search_files(workspace: &Path, pattern: &str, max_results: usize) -> Result<String, String> {
    let workspace_canon = workspace
        .canonicalize()
        .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

    let mut files: Vec<PathBuf> = Vec::new();
    walk_files(&workspace_canon, &mut files);

    let mut matches: Vec<String> = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&workspace_canon)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        if crate::tools::pattern::glob_match(pattern, &rel) {
            matches.push(rel);
        }
    }

    matches.sort();
    if matches.is_empty() {
        return Ok(format!("No files match pattern: {pattern}"));
    }

    let mut out = String::new();
    for (i, m) in matches.iter().enumerate() {
        if i >= max_results {
            out.push_str(&format!(
                "... (truncated at {max_results} paths; refine your pattern)\n"
            ));
            break;
        }
        out.push_str(m);
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Main executor
// ---------------------------------------------------------------------------

/// Execute a `search` tool call.
pub async fn execute_search_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse search arguments: {e}"))?;

    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "search requires 'mode'".to_string())?;

    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "search requires 'pattern'".to_string())?;

    check_fs_read(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(MAX_MATCHES)
        .clamp(1, 1000);

    match mode {
        "content" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let include = args.get("include").and_then(|v| v.as_str());
            let context_lines = args
                .get("context_lines")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(0)
                .clamp(0, 10);
            let case_sensitive = args
                .get("case_sensitive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

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
                search_content_with_rg(
                    &target,
                    pattern,
                    include,
                    context_lines,
                    case_sensitive,
                    max_results,
                )
            } else {
                search_content_fallback(&target, pattern, include, max_results)
            }
        }
        "files" => search_files(workspace, pattern, max_results),
        other => Err(format!(
            "Unknown search mode: '{other}'. Valid modes: content, files"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;
    use crate::permissions::types::Permissions;

    fn temp_workspace() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anacleto_search_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_search".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "search".into(),
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

    // -----------------------------------------------------------------------
    // Content search tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_search_content_finds_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.rs"), "fn main() {}\nlet x = 1;\n").unwrap();
        std::fs::write(ws.join("b.txt"), "no match here\n").unwrap();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"content","pattern":"fn main"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("a.rs:1:fn main() {}"));
        assert!(!result.contains("b.txt"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_search_content_no_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "hello\n").unwrap();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"content","pattern":"zzz_nonexistent"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("No matches"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_search_content_with_include() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.rs"), "needle\n").unwrap();
        std::fs::write(ws.join("b.txt"), "needle\n").unwrap();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"content","pattern":"needle","include":"*.rs"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("a.rs"));
        assert!(!result.contains("b.txt"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_search_content_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"content","pattern":"x","path":"../etc"}"#),
        )
        .await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // File search tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_search_files_matching() {
        let ws = temp_workspace();
        std::fs::create_dir_all(ws.join("src/sub")).unwrap();
        std::fs::write(ws.join("src/main.rs"), "").unwrap();
        std::fs::write(ws.join("src/sub/lib.rs"), "").unwrap();
        std::fs::write(ws.join("README.md"), "").unwrap();

        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"files","pattern":"**/*.rs"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("src/sub/lib.rs"));
        assert!(!result.contains("README.md"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_search_files_no_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "").unwrap();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"files","pattern":"*.py"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("No files match"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_search_unknown_mode() {
        let ws = temp_workspace();
        let result = execute_search_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"mode":"unknown","pattern":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown search mode"));
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
