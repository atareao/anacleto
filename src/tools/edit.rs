//! Builtin line-editing tools: `insert_lines`, `replace_lines`, `delete_lines`.
//!
//! Each tool reads a file, performs an in-memory line-based operation (insert,
//! replace, or delete a range of lines), and writes the result back. Lines are
//! 1-based, consistent with the `read` tool.
//!
//! Paths are resolved relative to the workspace. Writing to a path that escapes
//! the workspace (absolute path or `..` traversal) additionally requires the
//! `fs.external` permission.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::permissions::checker::{check_fs_external, check_fs_write};
use crate::permissions::types::Permissions;

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

/// Tool definition for `insert_lines`:
/// insert content after a specific line number (1-based, 0 = beginning).
pub fn insert_lines_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "insert_lines".to_string(),
        description: "Insert content after a specific line number in a file. \
                      Line numbers are 1-based. Use after_line=0 to insert \
                      at the beginning."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string"
                },
                "after_line": {
                    "type": "integer",
                    "minimum": 0
                },
                "content": {
                    "type": "string"
                }
            },
            "required": ["path", "after_line", "content"]
        }),
    }
}

/// Tool definition for `replace_lines`:
/// replace a range of lines (inclusive) with new content.
pub fn replace_lines_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "replace_lines".to_string(),
        description: "Replace a range of lines (inclusive) in a file with new \
                      content. Line numbers are 1-based."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string"
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1
                },
                "content": {
                    "type": "string"
                }
            },
            "required": ["path", "start_line", "end_line", "content"]
        }),
    }
}

/// Tool definition for `delete_lines`:
/// delete a range of lines (inclusive) from a file.
pub fn delete_lines_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "delete_lines".to_string(),
        description: "Delete a range of lines (inclusive) from a file. \
                      Line numbers are 1-based."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string"
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1
                }
            },
            "required": ["path", "start_line", "end_line"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve an edit path, enforcing workspace containment unless the caller has
/// the `fs.external` permission.
fn resolve_edit_path(
    workspace: &Path,
    path: &str,
    external_granted: bool,
) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if p.is_absolute() {
        if external_granted {
            Ok(p.to_path_buf())
        } else {
            Err("Writing outside the workspace requires the 'fs.external' permission".to_string())
        }
    } else {
        match crate::engine::apply_patch::resolve_within_workspace(workspace, path) {
            Ok(full) => Ok(full),
            Err(_) if external_granted => Ok(workspace.join(p)),
            Err(e) => Err(e),
        }
    }
}

/// Read the full contents of a file into a `Vec<String>` of lines.
fn read_lines(full: &Path) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(full)
        .map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;
    if content.is_empty() {
        return Ok(Vec::new());
    }
    Ok(content.lines().map(String::from).collect())
}

/// Write a `Vec<String>` of lines back to a file, joining with `\n`.
fn write_lines(full: &Path, lines: &[String]) -> Result<(), String> {
    let content = lines.join("\n");
    std::fs::write(full, content.as_bytes())
        .map_err(|e| format!("Failed to write '{}': {e}", full.display()))
}

// ---------------------------------------------------------------------------
// Executors
// ---------------------------------------------------------------------------

/// Execute an `insert_lines` tool call.
pub async fn execute_insert_lines_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse insert_lines arguments: {e}"))?;

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "insert_lines requires 'path'".to_string())?;

    let after_line =
        args.get("after_line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "insert_lines requires 'after_line'".to_string())? as usize;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "insert_lines requires 'content'".to_string())?;

    // Base permission: writing files requires fs.write.
    check_fs_write(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // If the path is outside the workspace, additionally require fs.external.
    let external_granted = check_fs_external(permissions).is_ok();
    let full = resolve_edit_path(workspace, path, external_granted)?;

    let mut lines = read_lines(&full)?;
    let total = lines.len();

    // after_line is 0-based offset counting lines; line numbers are 1-based.
    // after_line=0 → insert before the first line.
    // after_line >= total → append to the end.
    let insert_at = after_line.min(total);

    let new_lines: Vec<&str> = if content.is_empty() {
        // Empty content is a no-op, but still a success.
        return Ok(format!("No changes: empty content provided for '{}'", path));
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
        path
    ))
}

/// Execute a `replace_lines` tool call.
pub async fn execute_replace_lines_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse replace_lines arguments: {e}"))?;

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "replace_lines requires 'path'".to_string())?;

    let start_line =
        args.get("start_line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "replace_lines requires 'start_line'".to_string())? as usize;

    let end_line =
        args.get("end_line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "replace_lines requires 'end_line'".to_string())? as usize;

    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "replace_lines requires 'content'".to_string())?;

    if start_line > end_line {
        return Err(format!(
            "Invalid range: start_line ({start_line}) is greater than end_line ({end_line})"
        ));
    }

    // Base permission: writing files requires fs.write.
    check_fs_write(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // If the path is outside the workspace, additionally require fs.external.
    let external_granted = check_fs_external(permissions).is_ok();
    let full = resolve_edit_path(workspace, path, external_granted)?;

    let mut lines = read_lines(&full)?;
    let total = lines.len();

    if total == 0 {
        return Err(format!("File '{}' is empty; nothing to replace", path));
    }

    // Validate bounds (1-based).
    if start_line > total {
        return Err(format!(
            "start_line ({start_line}) exceeds file length ({total}) in '{}'",
            path
        ));
    }
    if end_line > total {
        return Err(format!(
            "end_line ({end_line}) exceeds file length ({total}) in '{}'",
            path
        ));
    }

    // Convert to 0-based indices.
    let start = start_line - 1;
    let end = end_line; // exclusive end for drain

    if content.is_empty() {
        // Empty content: just delete the range (still a valid replace).
        lines.drain(start..end);
        let removed = end_line - start_line + 1;
        write_lines(&full, &lines)?;
        return Ok(format!(
            "Replaced {} line(s) with empty content in '{}'",
            removed, path
        ));
    }

    let new_lines: Vec<&str> = content.lines().collect();
    let removed = end_line - start_line + 1;

    // Splice: remove the old range and insert new lines.
    lines.splice(start..end, new_lines.iter().cloned().map(String::from));

    write_lines(&full, &lines)?;

    Ok(format!(
        "Replaced {} line(s) (lines {}-{}) with {} line(s) in '{}'",
        removed,
        start_line,
        end_line,
        new_lines.len(),
        path
    ))
}

/// Execute a `delete_lines` tool call.
pub async fn execute_delete_lines_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse delete_lines arguments: {e}"))?;

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "delete_lines requires 'path'".to_string())?;

    let start_line =
        args.get("start_line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "delete_lines requires 'start_line'".to_string())? as usize;

    let end_line = args
        .get("end_line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "delete_lines requires 'end_line'".to_string())? as usize;

    if start_line > end_line {
        return Err(format!(
            "Invalid range: start_line ({start_line}) is greater than end_line ({end_line})"
        ));
    }

    // Base permission: writing files requires fs.write.
    check_fs_write(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // If the path is outside the workspace, additionally require fs.external.
    let external_granted = check_fs_external(permissions).is_ok();
    let full = resolve_edit_path(workspace, path, external_granted)?;

    let mut lines = read_lines(&full)?;
    let total = lines.len();

    if total == 0 {
        return Err(format!("File '{}' is empty; nothing to delete", path));
    }

    // Validate bounds (1-based).
    if start_line > total {
        return Err(format!(
            "start_line ({start_line}) exceeds file length ({total}) in '{}'",
            path
        ));
    }
    if end_line > total {
        return Err(format!(
            "end_line ({end_line}) exceeds file length ({total}) in '{}'",
            path
        ));
    }

    // Convert to 0-based indices (end is exclusive for drain).
    let start = start_line - 1;
    let end = end_line;
    let removed = end_line - start_line + 1;

    lines.drain(start..end);

    write_lines(&full, &lines)?;

    Ok(format!(
        "Deleted {} line(s) (lines {}-{}) from '{}'",
        removed, start_line, end_line, path
    ))
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
        let dir = std::env::temp_dir().join(format!("anacleto_edit_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn insert_tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_insert".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "insert_lines".into(),
                arguments: args.into(),
            },
        }
    }

    fn replace_tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_replace".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "replace_lines".into(),
                arguments: args.into(),
            },
        }
    }

    fn delete_tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_delete".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "delete_lines".into(),
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
    // insert_lines tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_insert_lines_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "one\ntwo\nfour\n").unwrap();

        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"a.txt","after_line":2,"content":"three"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 1 line(s) after line 2"));

        let content = std::fs::read_to_string(ws.join("a.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_lines_at_beginning() {
        let ws = temp_workspace();
        std::fs::write(ws.join("b.txt"), "two\nthree\n").unwrap();

        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"b.txt","after_line":0,"content":"one"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 1 line(s) after line 0"));

        let content = std::fs::read_to_string(ws.join("b.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_lines_at_end() {
        let ws = temp_workspace();
        std::fs::write(ws.join("c.txt"), "one\ntwo\n").unwrap();

        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"c.txt","after_line":999,"content":"three"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 1 line(s) after line 999"));

        let content = std::fs::read_to_string(ws.join("c.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_lines_empty_content_is_noop() {
        let ws = temp_workspace();
        std::fs::write(ws.join("d.txt"), "one\ntwo\n").unwrap();

        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"d.txt","after_line":1,"content":""}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("No changes"));

        // File content unchanged (no write occurs for empty content).
        let content = std::fs::read_to_string(ws.join("d.txt")).unwrap();
        assert_eq!(content, "one\ntwo\n");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_insert_lines_multiple_lines() {
        let ws = temp_workspace();
        std::fs::write(ws.join("e.txt"), "one\nfour\n").unwrap();

        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"e.txt","after_line":1,"content":"two\nthree"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 2 line(s) after line 1"));

        let content = std::fs::read_to_string(ws.join("e.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // replace_lines tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_replace_lines_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(
                r#"{"path":"f.txt","start_line":2,"end_line":3,"content":"hello\nworld"}"#,
            ),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 2 line(s) (lines 2-3) with 2 line(s)"));

        let content = std::fs::read_to_string(ws.join("f.txt")).unwrap();
        assert_eq!(content, "one\nhello\nworld\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_lines_single_line() {
        let ws = temp_workspace();
        std::fs::write(ws.join("g.txt"), "one\ntwo\nthree\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(r#"{"path":"g.txt","start_line":2,"end_line":2,"content":"dos"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 1 line(s) (lines 2-2) with 1 line(s)"));

        let content = std::fs::read_to_string(ws.join("g.txt")).unwrap();
        assert_eq!(content, "one\ndos\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_lines_with_empty_content() {
        let ws = temp_workspace();
        std::fs::write(ws.join("h.txt"), "one\ntwo\nthree\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(r#"{"path":"h.txt","start_line":2,"end_line":2,"content":""}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 1 line(s) with empty content"));

        let content = std::fs::read_to_string(ws.join("h.txt")).unwrap();
        assert_eq!(content, "one\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_lines_invalid_range() {
        let ws = temp_workspace();
        std::fs::write(ws.join("i.txt"), "one\ntwo\nthree\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(r#"{"path":"i.txt","start_line":3,"end_line":2,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("greater than"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_replace_lines_start_out_of_bounds() {
        let ws = temp_workspace();
        std::fs::write(ws.join("j.txt"), "one\ntwo\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(r#"{"path":"j.txt","start_line":10,"end_line":10,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("exceeds file length"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // delete_lines tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_lines_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("k.txt"), "one\ntwo\nthree\nfour\n").unwrap();

        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"k.txt","start_line":2,"end_line":3}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Deleted 2 line(s) (lines 2-3)"));

        let content = std::fs::read_to_string(ws.join("k.txt")).unwrap();
        assert_eq!(content, "one\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_lines_single_line() {
        let ws = temp_workspace();
        std::fs::write(ws.join("l.txt"), "one\ntwo\nthree\n").unwrap();

        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"l.txt","start_line":2,"end_line":2}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Deleted 1 line(s) (lines 2-2)"));

        let content = std::fs::read_to_string(ws.join("l.txt")).unwrap();
        assert_eq!(content, "one\nthree");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_lines_all() {
        let ws = temp_workspace();
        std::fs::write(ws.join("m.txt"), "one\ntwo\nthree\n").unwrap();

        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"m.txt","start_line":1,"end_line":3}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Deleted 3 line(s) (lines 1-3)"));

        let content = std::fs::read_to_string(ws.join("m.txt")).unwrap();
        assert_eq!(content, "");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // Shared rejection tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_edit_rejects_path_traversal() {
        let ws = temp_workspace();

        // insert_lines
        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"../secret.txt","after_line":0,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("escapes workspace") || err.contains("fs.external"));

        // replace_lines
        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(
                r#"{"path":"../secret.txt","start_line":1,"end_line":1,"content":"x"}"#,
            ),
        )
        .await;
        assert!(result.is_err());

        // delete_lines
        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"../secret.txt","start_line":1,"end_line":1}"#),
        )
        .await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_edit_missing_file_errors() {
        let ws = temp_workspace();

        // insert_lines
        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(r#"{"path":"nope.txt","after_line":0,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());

        // replace_lines
        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(r#"{"path":"nope.txt","start_line":1,"end_line":1,"content":"x"}"#),
        )
        .await;
        assert!(result.is_err());

        // delete_lines
        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"nope.txt","start_line":1,"end_line":1}"#),
        )
        .await;
        assert!(result.is_err());

        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_edit_external_requires_fs_external() {
        let ws = temp_workspace();
        let outside =
            std::env::temp_dir().join(format!("anacleto_edit_outside_{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "external data").unwrap();

        // Without fs.external, an absolute external path is denied.
        let result = execute_insert_lines_tool(
            &ws,
            &allow_all(),
            &insert_tool_call(&format!(
                r#"{{"path":"{}","after_line":0,"content":"x"}}"#,
                outside.display()
            )),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fs.external"));

        // With fs.external granted, it is allowed.
        let perms = Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec!["fs.write".into(), "fs.external".into()],
        });
        let result = execute_insert_lines_tool(
            &ws,
            &perms,
            &insert_tool_call(&format!(
                r#"{{"path":"{}","after_line":0,"content":"hello"}}"#,
                outside.display()
            )),
        )
        .await;
        assert!(result.is_ok());

        let content = std::fs::read_to_string(&outside).unwrap();
        assert_eq!(content, "hello\nexternal data");

        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[tokio::test]
    async fn test_edit_delete_lines_invalid_range() {
        let ws = temp_workspace();
        std::fs::write(ws.join("n.txt"), "one\ntwo\n").unwrap();

        let result = execute_delete_lines_tool(
            &ws,
            &allow_all(),
            &delete_tool_call(r#"{"path":"n.txt","start_line":2,"end_line":1}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("greater than"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_edit_replace_lines_entire_file() {
        let ws = temp_workspace();
        std::fs::write(ws.join("o.txt"), "old line 1\nold line 2\n").unwrap();

        let result = execute_replace_lines_tool(
            &ws,
            &allow_all(),
            &replace_tool_call(
                r#"{"path":"o.txt","start_line":1,"end_line":2,"content":"new line 1\nnew line 2\nnew line 3"}"#,
            ),
        )
        .await
        .unwrap();
        assert!(result.contains("Replaced 2 line(s) (lines 1-2) with 3 line(s)"));

        let content = std::fs::read_to_string(ws.join("o.txt")).unwrap();
        assert_eq!(content, "new line 1\nnew line 2\nnew line 3");
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
