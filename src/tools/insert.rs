//! The `insert` tool: insert content after a specific line number in a file.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Tool definition for the `insert` tool.
pub fn insert_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "insert".to_string(),
        description: "Insert content after a specific line number in a file. \
             Use after_line=0 to insert at the beginning of the file."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Target file path"
                },
                "after_line": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Insert content after this line number (0 = beginning)"
                },
                "content": {
                    "type": "string",
                    "description": "Content to insert"
                }
            },
            "required": ["path", "after_line", "content"]
        }),
    }
}

fn resolve_insert_path(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if p.is_absolute() {
        // Allow absolute paths that are within the workspace
        let workspace_canon = workspace
            .canonicalize()
            .map_err(|e| format!("Invalid workspace '{}': {e}", workspace.display()))?;

        match p.canonicalize() {
            Ok(canon) if canon.starts_with(&workspace_canon) => Ok(p.to_path_buf()),
            Ok(_) => Err("Writing outside the workspace is not allowed".to_string()),
            Err(e) => Err(format!("Path does not exist: {e}")),
        }
    } else {
        crate::engine::apply_patch::resolve_within_workspace(workspace, path)
    }
}

fn read_lines(full: &Path) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(full)
        .map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;
    if content.is_empty() {
        return Ok(Vec::new());
    }
    Ok(content.lines().map(String::from).collect())
}

fn write_lines(full: &Path, lines: &[String]) -> Result<(), String> {
    let content = lines.join("\n");
    std::fs::write(full, content.as_bytes())
        .map_err(|e| format!("Failed to write '{}': {e}", full.display()))
}

/// Execute an `insert` tool call.
pub async fn execute_insert_tool(workspace: &Path, tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse insert arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "insert requires 'path'".to_string())?;
    let after_line = args
        .get("after_line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "insert requires 'after_line'".to_string())? as usize;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "insert requires 'content'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::insert",
        path = %path,
        after_line = %after_line,
        content_len = %content.len(),
        "insert tool"
    );

    let full = resolve_insert_path(workspace, path)?;

    let mut lines = read_lines(&full)?;
    let total = lines.len();
    let insert_at = after_line.min(total);

    let new_lines: Vec<&str> = if content.is_empty() {
        return Ok(format!(
            "No changes: empty content provided for '{}'",
            full.display()
        ));
    } else {
        content.lines().collect()
    };

    for (i, line) in new_lines.iter().enumerate() {
        lines.insert(insert_at + i, line.to_string());
    }

    write_lines(&full, &lines)?;
    Ok(format!(
        "Inserted {} line(s) after line {} in '{}'",
        new_lines.len(),
        after_line,
        full.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anacleto_insert_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_insert".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "insert".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn test_insert_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("i.txt"), "one\ntwo\nfour\n").unwrap();
        let result = execute_insert_tool(
            &ws,
            &tool_call(r#"{"path":"i.txt","after_line":2,"content":"three"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 1 line(s) after line 2"));
        let content = std::fs::read_to_string(ws.join("i.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_at_beginning() {
        let ws = temp_workspace();
        std::fs::write(ws.join("j.txt"), "two\nthree\n").unwrap();
        let result = execute_insert_tool(
            &ws,
            &tool_call(r#"{"path":"j.txt","after_line":0,"content":"one"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 1 line(s) after line 0"));
        let content = std::fs::read_to_string(ws.join("j.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_multiple_lines() {
        let ws = temp_workspace();
        std::fs::write(ws.join("k.txt"), "one\nfour\n").unwrap();
        let result = execute_insert_tool(
            &ws,
            &tool_call(r#"{"path":"k.txt","after_line":1,"content":"two\nthree"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 2 line(s) after line 1"));
        let content = std::fs::read_to_string(ws.join("k.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_insert_tool(
            &ws,
            &tool_call(r#"{"path":"../secret.txt","after_line":0,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
