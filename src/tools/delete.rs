//! The `delete` tool: delete a file or directory.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Tool definition for the `delete` tool.
pub fn delete_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "delete".to_string(),
        description: "Delete a file or directory. Deletes directories recursively.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Target file or directory path"
                }
            },
            "required": ["path"]
        }),
    }
}

fn resolve_delete_path(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if p.is_absolute() {
        // Allow absolute paths that are within the workspace
        let workspace_canon = workspace
            .canonicalize()
            .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

        match p.canonicalize() {
            Ok(canon) if canon.starts_with(&workspace_canon) => Ok(p.to_path_buf()),
            Ok(_) => Err("Deleting outside the workspace is not allowed".to_string()),
            Err(e) => Err(format!("Path does not exist: {e}")),
        }
    } else {
        crate::engine::apply_patch::resolve_within_workspace(workspace, path)
    }
}

/// Execute a `delete` tool call.
pub async fn execute_delete_tool(workspace: &Path, tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse delete arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "delete requires 'path'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::delete",
        path = %path,
        "delete tool"
    );

    let full = resolve_delete_path(workspace, path)?;

    if full.is_dir() {
        std::fs::remove_dir_all(&full)
            .map_err(|e| format!("Failed to delete directory '{}': {e}", full.display()))?;
        Ok(format!("Deleted directory '{}'", full.display()))
    } else {
        std::fs::remove_file(&full)
            .map_err(|e| format!("Failed to delete '{}': {e}", full.display()))?;
        Ok(format!("Deleted '{}'", full.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anacleto_delete_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_delete".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "delete".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn test_delete_file() {
        let ws = temp_workspace();
        let file = ws.join("del.txt");
        std::fs::write(&file, "x").unwrap();
        let result = execute_delete_tool(&ws, &tool_call(r#"{"path":"del.txt"}"#))
            .await
            .unwrap();
        assert!(result.contains("Deleted"));
        assert!(!file.exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_directory() {
        let ws = temp_workspace();
        let dir = ws.join("subdir");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("f.txt"), "x").unwrap();
        let result = execute_delete_tool(&ws, &tool_call(r#"{"path":"subdir"}"#))
            .await
            .unwrap();
        assert!(result.contains("Deleted directory"));
        assert!(!dir.exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let ws = temp_workspace();
        let result = execute_delete_tool(&ws, &tool_call(r#"{"path":"missing.txt"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_delete_tool(&ws, &tool_call(r#"{"path":"../secret.txt"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
