//! The `list` tool: list directory entries.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Tool definition for the `list` tool.
pub fn list_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "list".to_string(),
        description:
            "List entries in a directory. Returns file names and directory names (with trailing /). \
             Entries are sorted alphabetically."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path (use '.' for workspace root)"
                }
            },
            "required": ["path"]
        }),
    }
}

fn resolve_list_path(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path == "." {
        return Ok(workspace.to_path_buf());
    }
    let p = Path::new(path);
    if p.is_absolute() {
        // Allow absolute paths that are within the workspace
        let workspace_canon = workspace
            .canonicalize()
            .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

        match p.canonicalize() {
            Ok(canon) if canon.starts_with(&workspace_canon) => Ok(p.to_path_buf()),
            Ok(_) => Err("Listing outside the workspace is not allowed".to_string()),
            Err(e) => Err(format!("Path does not exist: {e}")),
        }
    } else {
        crate::engine::apply_patch::resolve_within_workspace(workspace, path)
    }
}

/// Execute a `list` tool call.
pub async fn execute_list_tool(workspace: &Path, tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse list arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "list requires 'path'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::list",
        path = %path,
        "list tool"
    );

    let full = resolve_list_path(workspace, path)?;

    let entries = std::fs::read_dir(&full)
        .map_err(|e| format!("Failed to list '{}': {e}", full.display()))?;

    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().ok()?.is_dir();
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();

    names.sort();
    if names.is_empty() {
        Ok(format!("Directory '{}' is empty", full.display()))
    } else {
        Ok(names.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anacleto_list_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_list".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "list".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn test_list_directory() {
        let ws = temp_workspace();
        std::fs::create_dir(ws.join("subdir")).unwrap();
        std::fs::write(ws.join("b.txt"), "").unwrap();
        std::fs::write(ws.join("a.txt"), "").unwrap();
        let result = execute_list_tool(&ws, &tool_call(r#"{"path":"."}"#))
            .await
            .unwrap();
        let lines: Vec<&str> = result.lines().collect();
        assert!(lines.contains(&"a.txt"));
        assert!(lines.contains(&"b.txt"));
        assert!(lines.contains(&"subdir/"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_list_empty_directory() {
        let ws = temp_workspace();
        let result = execute_list_tool(&ws, &tool_call(r#"{"path":"."}"#))
            .await
            .unwrap();
        assert!(result.contains("empty"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_list_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_list_tool(&ws, &tool_call(r#"{"path":"../secret"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
