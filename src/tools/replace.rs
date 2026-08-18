//! The `replace` tool: replace text (old+new) or line ranges in a file.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

/// Tool definition for the `replace` tool.
pub fn replace_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "replace".to_string(),
        description:
            "Replace text in a file. Two modes:\n\
             1. Text-based: provide 'old' and 'new' to replace all occurrences of a string.\n\
             2. Line-range: provide 'start_line', 'end_line', and 'content' to replace a range of lines."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Target file path"
                },
                "old": {
                    "type": "string",
                    "description": "Text to find and replace (for text-based replace)"
                },
                "new": {
                    "type": "string",
                    "description": "Replacement text (for text-based replace)"
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Start line for line-range replace (1-based)"
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "End line for line-range replace (1-based, inclusive)"
                },
                "content": {
                    "type": "string",
                    "description": "Replacement content (for line-range replace)"
                }
            },
            "required": ["path"]
        }),
    }
}

fn resolve_replace_path(workspace: &Path, path: &str) -> Result<PathBuf, String> {
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

/// Execute a `replace` tool call.
pub async fn execute_replace_tool(
    workspace: &Path,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse replace arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "replace requires 'path'".to_string())?;

    let full = resolve_replace_path(workspace, path)?;

    tracing::debug!(
        target: "anacleto::tools::replace",
        path = %path,
        "replace tool"
    );

    let old = args.get("old").and_then(|v| v.as_str());
    let new = args.get("new").and_then(|v| v.as_str());
    let start_line = args
        .get("start_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let content = args.get("content").and_then(|v| v.as_str());

    // Text-based replace (old+new)
    if let (Some(old_text), Some(new_text)) = (old, new) {
        let file_content = std::fs::read_to_string(&full)
            .map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;

        if !file_content.contains(old_text) {
            return Err(format!(
                "The text to replace was not found in '{}'",
                full.display()
            ));
        }

        let count = file_content.matches(old_text).count();
        let updated = file_content.replace(old_text, new_text);

        std::fs::write(&full, updated)
            .map_err(|e| format!("Failed to write '{}': {e}", full.display()))?;

        return Ok(format!(
            "Replaced {count} occurrence(s) of the old text in '{}'",
            full.display()
        ));
    }

    // Line-range replace (start_line+end_line+content)
    let start = start_line.ok_or_else(|| {
        "replace requires either 'old'+'new' (text-based) or 'start_line'+'end_line'+'content' (line-range)"
            .to_string()
    })?;
    let end = end_line
        .ok_or_else(|| "replace requires 'end_line' when using line-range mode".to_string())?;

    if start > end {
        return Err(format!(
            "Invalid range: start_line ({start}) is greater than end_line ({end})"
        ));
    }

    let mut lines = read_lines(&full)?;
    let total = lines.len();

    if total == 0 {
        return Err(format!(
            "File '{}' is empty; nothing to replace",
            full.display()
        ));
    }

    if start > total {
        return Err(format!(
            "start_line ({start}) exceeds file length ({total}) in '{}'",
            full.display()
        ));
    }
    if end > total {
        return Err(format!(
            "end_line ({end}) exceeds file length ({total}) in '{}'",
            full.display()
        ));
    }

    let start_idx = start - 1;
    let end_idx = end;
    let removed = end - start + 1;

    let content_str = content.unwrap_or("");
    if content_str.is_empty() {
        lines.drain(start_idx..end_idx);
        write_lines(&full, &lines)?;
        return Ok(format!(
            "Replaced {removed} line(s) with empty content in '{}'",
            full.display()
        ));
    }

    let new_lines: Vec<&str> = content_str.lines().collect();
    lines.splice(
        start_idx..end_idx,
        new_lines.iter().cloned().map(String::from),
    );
    write_lines(&full, &lines)?;

    Ok(format!(
        "Replaced {removed} line(s) (lines {start}-{end}) with {} line(s) in '{}'",
        new_lines.len(),
        full.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("anacleto_replace_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_replace".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "replace".into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn test_replace_text_based() {
        let ws = temp_workspace();
        std::fs::write(ws.join("r.txt"), "foo foo bar foo").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"r.txt","old":"foo","new":"baz"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 3 occurrence(s)"));
        assert_eq!(
            std::fs::read_to_string(ws.join("r.txt")).unwrap(),
            "baz baz bar baz"
        );
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_text_not_found() {
        let ws = temp_workspace();
        std::fs::write(ws.join("s.txt"), "hello").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"s.txt","old":"zzz","new":"yyy"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_line_range() {
        let ws = temp_workspace();
        std::fs::write(ws.join("t.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"t.txt","start_line":2,"end_line":3,"content":"hello\nworld"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 2 line(s) (lines 2-3) with 2 line(s)"));
        let content = std::fs::read_to_string(ws.join("t.txt")).unwrap();
        assert_eq!(content, "one\nhello\nworld\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_line_range_single_line() {
        let ws = temp_workspace();
        std::fs::write(ws.join("u.txt"), "one\ntwo\nthree\n").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"u.txt","start_line":2,"end_line":2,"content":"dos"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 1 line(s) (lines 2-2) with 1 line(s)"));
        let content = std::fs::read_to_string(ws.join("u.txt")).unwrap();
        assert_eq!(content, "one\ndos\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_line_range_empty_content() {
        let ws = temp_workspace();
        std::fs::write(ws.join("v.txt"), "one\ntwo\nthree\n").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"v.txt","start_line":2,"end_line":2,"content":""}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 1 line(s) with empty content"));
        let content = std::fs::read_to_string(ws.join("v.txt")).unwrap();
        assert_eq!(content, "one\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_line_range_invalid() {
        let ws = temp_workspace();
        std::fs::write(ws.join("w.txt"), "one\ntwo\nthree\n").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"w.txt","start_line":3,"end_line":2,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("greater than"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_line_range_out_of_bounds() {
        let ws = temp_workspace();
        std::fs::write(ws.join("x.txt"), "one\ntwo\n").unwrap();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"x.txt","start_line":10,"end_line":10,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds file length"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_rejects_path_traversal() {
        let ws = temp_workspace();
        let result = execute_replace_tool(
            &ws,
            &tool_call(r#"{"path":"../secret.txt","old":"x","new":"y"}"#),
        )
        .await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
