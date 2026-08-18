//! The `write` tool: write content to a file, creating parent directories.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Tool definition for the `write` tool.
pub fn write_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "write".to_string(),
        description: "Write content to a file, creating parent directories if they don't exist. \
             Overwrites the file if it already exists."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Target file path"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write"
                }
            },
            "required": ["path", "content"]
        }),
    }
}

/// Resolve a write path, enforcing workspace containment.
fn resolve_write_path(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path == "." {
        return Ok(workspace.to_path_buf());
    }
    let p = Path::new(path);
    if p.is_absolute() {
        // Allow absolute paths that are within the workspace
        let workspace_canon = workspace
            .canonicalize()
            .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

        // For write, the file may not exist yet, so check the parent directory
        let probe = p.parent().unwrap_or(p);
        match probe.canonicalize() {
            Ok(canon) if canon.starts_with(&workspace_canon) => Ok(p.to_path_buf()),
            Ok(_) => Err("Writing outside the workspace is not allowed".to_string()),
            Err(e) => Err(format!("Cannot resolve path: {e}")),
        }
    } else {
        crate::engine::apply_patch::resolve_within_workspace(workspace, path)
    }
}

/// Execute a `write` tool call.
pub async fn execute_write_tool(workspace: &Path, tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse write arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "write requires 'path'".to_string())?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "write requires 'content'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::write",
        path = %path,
        content_len = %content.len(),
        "write tool"
    );

    let full = resolve_write_path(workspace, path)?;

    let len = content.len();
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create parent dirs for '{}': {e}", full.display()))?;
    }
    std::fs::write(&full, content)
        .map_err(|e| format!("Failed to write '{}': {e}", full.display()))?;
    Ok(format!("Wrote {len} bytes to '{}'", full.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anacleto_write_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_write".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "write".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let ws = temp_workspace();
        let result = execute_write_tool(
            &ws,
            &tool_call(r#"{"path":"nested/deep/file.txt","content":"hello"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Wrote"));
        assert_eq!(
            std::fs::read_to_string(ws.join("nested/deep/file.txt")).unwrap(),
            "hello"
        );
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_write_missing_content() {
        let ws = temp_workspace();
        let result = execute_write_tool(&ws, &tool_call(r#"{"path":"x.txt"}"#)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("content"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_write_rejects_path_traversal() {
        let ws = temp_workspace();
        let result =
            execute_write_tool(&ws, &tool_call(r#"{"path":"../secret.txt","content":"x"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_write_external_is_denied() {
        let ws = temp_workspace();
        let outside =
            std::env::temp_dir().join(format!("anacleto_write_outside_{}", uuid::Uuid::new_v4()));
        let result = execute_write_tool(
            &ws,
            &tool_call(&format!(
                r#"{{"path":"{}","content":"hello"}}"#,
                outside.display()
            )),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("is not allowed"));
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
