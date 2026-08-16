//! The `glob` tool: list files matching a glob pattern within the workspace.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Maximum number of paths returned per `glob` call.
const MAX_PATHS: usize = 1000;

/// Tool definition for the `glob` tool.
pub fn glob_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "glob".to_string(),
        description: "List files matching a glob pattern within the workspace.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string"
                }
            },
            "required": ["pattern"]
        }),
    }
}

/// Recursively collect all files under `dir`.
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

/// Execute a `glob` tool call.
pub async fn execute_glob_tool(
    workspace: &Path,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse glob arguments: {e}"))?;
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "glob requires 'pattern'".to_string())?;

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
        if i >= MAX_PATHS {
            out.push_str(&format!(
                "... (truncated at {MAX_PATHS} paths; refine your pattern)\n"
            ));
            break;
        }
        out.push_str(m);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anacleto_glob_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_glob".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "glob".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn glob_lists_matching_files() {
        let ws = temp_workspace();
        std::fs::create_dir_all(ws.join("src/sub")).unwrap();
        std::fs::write(ws.join("src/main.rs"), "").unwrap();
        std::fs::write(ws.join("src/sub/lib.rs"), "").unwrap();
        std::fs::write(ws.join("README.md"), "").unwrap();

        let result = execute_glob_tool(&ws, &tool_call(r#"{"pattern":"**/*.rs"}"#))
            .await
            .unwrap();
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("src/sub/lib.rs"));
        assert!(!result.contains("README.md"));

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn glob_no_matches() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "").unwrap();
        let result = execute_glob_tool(&ws, &tool_call(r#"{"pattern":"*.py"}"#))
            .await
            .unwrap();
        assert!(result.contains("No files match"));
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
