//! Unified filesystem tool (`fs`) — replaces `read`, `insert_lines`,
//! `replace_lines`, `delete_lines`, and the old `filesystem` skill.
//!
//! Single tool with an `op` field selecting the operation and flat parameters
//! for each operation. No nested JSON — the LLM provides top-level fields
//! directly in the tool call arguments.
//!
//! ## Operations
//!
//! | op | Required | Optional | Description |
//! |----|----------|----------|-------------|
//! | `read` | path | offset, limit | Read file with line numbers |
//! | `write` | path, content | — | Write content, creating parent dirs |
//! | `insert` | path, after_line, content | — | Insert content after line number |
//! | `replace` | path | old+new OR start_line+end_line+content | Replace text or line range |
//! | `delete` | path | — | Delete file |
//! | `list` | path | — | List directory entries |
//!
//! Paths are resolved relative to the workspace. Writing outside the workspace
//! is not allowed by default.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of lines returned per `read` operation.
pub const MAX_LINES: usize = 2000;
/// Maximum number of bytes returned per `read` operation.
pub const MAX_BYTES: usize = 50 * 1024;

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

/// Tool definition for the unified `fs` tool.
pub fn fs_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "fs".to_string(),
        description:
            "Unified filesystem tool. Supports read, write, insert, replace, delete, and list \
             operations. Use `op` to select the operation and provide the relevant parameters. \
             For `replace`, provide either `old`+`new` (text-based replace all) or \
             `start_line`+`end_line`+`content` (line-range replace)."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["read", "write", "insert", "replace", "delete", "list"],
                    "description": "Operation to perform"
                },
                "path": {
                    "type": "string",
                    "description": "Target file or directory path"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write (for write/insert/replace)"
                },
                "old": {
                    "type": "string",
                    "description": "Text to find and replace (for text-based replace)"
                },
                "new": {
                    "type": "string",
                    "description": "Replacement text (for text-based replace)"
                },
                "after_line": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Insert content after this line number (0 = beginning, for insert)"
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Start line for replace/delete (1-based)"
                },
                "end_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "End line for replace/delete (1-based, inclusive)"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Line offset for read (1-based)"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 2000,
                    "description": "Max lines to read"
                }
            },
            "required": ["op", "path"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Path resolution
// ---------------------------------------------------------------------------

/// Resolve a path, enforcing workspace containment.
fn resolve_fs_path(
    workspace: &Path,
    path: &str,
    external_granted: bool,
) -> Result<PathBuf, String> {
    if path.is_empty() || path == "." {
        return Ok(workspace.to_path_buf());
    }
    let p = Path::new(path);
    if p.is_absolute() {
        if external_granted {
            Ok(p.to_path_buf())
        } else {
            Err(
                "Accessing paths outside the workspace requires the 'fs.external' permission"
                    .to_string(),
            )
        }
    } else {
        match crate::engine::apply_patch::resolve_within_workspace(workspace, path) {
            Ok(full) => Ok(full),
            Err(_) if external_granted => Ok(workspace.join(p)),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Line helpers
// ---------------------------------------------------------------------------

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
// Operation executors
// ---------------------------------------------------------------------------

/// Execute a `read` operation: read file with optional offset/limit and line numbers.
fn execute_read_op(full: &Path, offset: usize, limit: usize) -> Result<String, String> {
    let bytes =
        std::fs::read(full).map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();

    let start = offset.saturating_sub(1);
    let end = (start + limit).min(total);
    let has_more = end < total;

    let mut out = String::new();
    let mut byte_count = 0usize;
    for (i, line) in lines[start..end].iter().enumerate() {
        let lineno = start + i + 1;
        let entry = format!("{lineno:>6} | {line}\n");
        if byte_count + entry.len() > MAX_BYTES {
            out.push_str("... (output truncated at 50KB)\n");
            break;
        }
        byte_count += entry.len();
        out.push_str(&entry);
    }

    if has_more {
        out.push_str(&format!(
            "... (showing lines {}-{} of {}; pass offset={} to read more)\n",
            start + 1,
            end,
            total,
            end + 1
        ));
    }

    Ok(out)
}

/// Execute a `write` operation: write content, creating parent directories.
fn execute_write_op(full: &Path, content: &str) -> Result<String, String> {
    let len = content.len();
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create parent dirs for '{}': {e}", full.display()))?;
    }
    std::fs::write(full, content)
        .map_err(|e| format!("Failed to write '{}': {e}", full.display()))?;
    Ok(format!("Wrote {len} bytes to '{}'", full.display()))
}

/// Execute an `insert` operation: insert content after a specific line number.
fn execute_insert_op(full: &Path, after_line: usize, content: &str) -> Result<String, String> {
    let mut lines = read_lines(full)?;
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

    write_lines(full, &lines)?;
    Ok(format!(
        "Inserted {} line(s) after line {} in '{}'",
        new_lines.len(),
        after_line,
        full.display()
    ))
}

/// Execute a `replace` operation: text-based (old+new) or line-range
/// (start_line+end_line+content).
fn execute_replace_op(
    full: &Path,
    old: Option<&str>,
    new: Option<&str>,
    start_line: Option<usize>,
    end_line: Option<usize>,
    content: Option<&str>,
) -> Result<String, String> {
    // Text-based replace (old+new)
    if let (Some(old_text), Some(new_text)) = (old, new) {
        let file_content = std::fs::read_to_string(full)
            .map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;

        if !file_content.contains(old_text) {
            return Err(format!(
                "The text to replace was not found in '{}'",
                full.display()
            ));
        }

        let count = file_content.matches(old_text).count();
        let updated = file_content.replace(old_text, new_text);

        std::fs::write(full, updated)
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

    let mut lines = read_lines(full)?;
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
    let end_idx = end; // exclusive for drain
    let removed = end - start + 1;

    let content_str = content.unwrap_or("");
    if content_str.is_empty() {
        lines.drain(start_idx..end_idx);
        write_lines(full, &lines)?;
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
    write_lines(full, &lines)?;

    Ok(format!(
        "Replaced {removed} line(s) (lines {start}-{end}) with {} line(s) in '{}'",
        new_lines.len(),
        full.display()
    ))
}

/// Execute a `delete` operation: delete a file or directory.
fn execute_delete_op(full: &Path) -> Result<String, String> {
    if full.is_dir() {
        std::fs::remove_dir_all(full)
            .map_err(|e| format!("Failed to delete directory '{}': {e}", full.display()))?;
        Ok(format!("Deleted directory '{}'", full.display()))
    } else {
        std::fs::remove_file(full)
            .map_err(|e| format!("Failed to delete '{}': {e}", full.display()))?;
        Ok(format!("Deleted '{}'", full.display()))
    }
}

/// Execute a `list` operation: list directory entries.
fn execute_list_op(full: &Path) -> Result<String, String> {
    let entries =
        std::fs::read_dir(full).map_err(|e| format!("Failed to list '{}': {e}", full.display()))?;

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

// ---------------------------------------------------------------------------
// Main executor
// ---------------------------------------------------------------------------

/// Execute an `fs` tool call.
pub async fn execute_fs_tool(workspace: &Path, tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse fs arguments: {e}"))?;

    let op = args
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs requires 'op'".to_string())?;

    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs requires 'path'".to_string())?;

    let external_granted = false;
    let full = resolve_fs_path(workspace, path, external_granted)?;

    match op {
        "read" => {
            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(1);
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(MAX_LINES)
                .clamp(1, MAX_LINES);
            execute_read_op(&full, offset, limit)
        }
        "write" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "write requires 'content'".to_string())?;
            execute_write_op(&full, content)
        }
        "insert" => {
            let after_line = args
                .get("after_line")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "insert requires 'after_line'".to_string())?
                as usize;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "insert requires 'content'".to_string())?;
            execute_insert_op(&full, after_line, content)
        }
        "replace" => {
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
            execute_replace_op(&full, old, new, start_line, end_line, content)
        }
        "delete" => execute_delete_op(&full),
        "list" => execute_list_op(&full),
        other => Err(format!(
            "Unknown fs operation: '{other}'. Valid ops: read, write, insert, replace, delete, list"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anacleto_fs_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_fs".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "fs".into(),
                arguments: args.into(),
            },
        }
    }

    // -----------------------------------------------------------------------
    // read tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_read_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"read","path":"a.txt"}"#))
            .await
            .unwrap();
        assert!(result.contains("1 | one"));
        assert!(result.contains("2 | two"));
        assert!(result.contains("3 | three"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_read_with_offset_and_limit() {
        let ws = temp_workspace();
        let content: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        std::fs::write(ws.join("b.txt"), content).unwrap();
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"read","path":"b.txt","offset":3,"limit":2}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("3 | line3"));
        assert!(result.contains("4 | line4"));
        assert!(!result.contains("5 | line5"));
        assert!(result.contains("read more"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_read_missing_file_errors() {
        let ws = temp_workspace();
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"read","path":"nope.txt"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // write tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let ws = temp_workspace();
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"write","path":"nested/deep/file.txt","content":"hello"}"#),
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
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"write","path":"x.txt"}"#)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("content"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // insert tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_insert_basic() {
        let ws = temp_workspace();
        std::fs::write(ws.join("i.txt"), "one\ntwo\nfour\n").unwrap();
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"insert","path":"i.txt","after_line":2,"content":"three"}"#),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"insert","path":"j.txt","after_line":0,"content":"one"}"#),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"insert","path":"k.txt","after_line":1,"content":"two\nthree"}"#),
        )
        .await
        .unwrap();
        assert!(result.contains("Inserted 2 line(s) after line 1"));
        let content = std::fs::read_to_string(ws.join("k.txt")).unwrap();
        assert_eq!(content, "one\ntwo\nthree\nfour");
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // replace tests (text-based)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_replace_text_based() {
        let ws = temp_workspace();
        std::fs::write(ws.join("r.txt"), "foo foo bar foo").unwrap();
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"replace","path":"r.txt","old":"foo","new":"baz"}"#),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(r#"{"op":"replace","path":"s.txt","old":"zzz","new":"yyy"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // replace tests (line-range)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_replace_line_range() {
        let ws = temp_workspace();
        std::fs::write(ws.join("t.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let result = execute_fs_tool(
            &ws,
            &tool_call(
                r#"{"op":"replace","path":"t.txt","start_line":2,"end_line":3,"content":"hello\nworld"}"#,
            ),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(
                r#"{"op":"replace","path":"u.txt","start_line":2,"end_line":2,"content":"dos"}"#,
            ),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(
                r#"{"op":"replace","path":"v.txt","start_line":2,"end_line":2,"content":""}"#,
            ),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(
                r#"{"op":"replace","path":"w.txt","start_line":3,"end_line":2,"content":"x"}"#,
            ),
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
        let result = execute_fs_tool(
            &ws,
            &tool_call(
                r#"{"op":"replace","path":"x.txt","start_line":10,"end_line":10,"content":"x"}"#,
            ),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds file length"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // delete tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_file() {
        let ws = temp_workspace();
        let file = ws.join("del.txt");
        std::fs::write(&file, "x").unwrap();
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"delete","path":"del.txt"}"#))
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
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"delete","path":"subdir"}"#))
            .await
            .unwrap();
        assert!(result.contains("Deleted directory"));
        assert!(!dir.exists());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let ws = temp_workspace();
        let result =
            execute_fs_tool(&ws, &tool_call(r#"{"op":"delete","path":"missing.txt"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // list tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_directory() {
        let ws = temp_workspace();
        std::fs::create_dir(ws.join("subdir")).unwrap();
        std::fs::write(ws.join("b.txt"), "").unwrap();
        std::fs::write(ws.join("a.txt"), "").unwrap();
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"list","path":"."}"#))
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
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"list","path":"."}"#))
            .await
            .unwrap();
        assert!(result.contains("empty"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    // -----------------------------------------------------------------------
    // permission tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_rejects_path_traversal() {
        let ws = temp_workspace();
        let result =
            execute_fs_tool(&ws, &tool_call(r#"{"op":"read","path":"../secret.txt"}"#)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("escapes workspace") || err.contains("fs.external"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_external_requires_fs_external() {
        let ws = temp_workspace();
        let outside =
            std::env::temp_dir().join(format!("anacleto_fs_outside_{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "external data").unwrap();

        // An absolute external path is denied.
        let result = execute_fs_tool(
            &ws,
            &tool_call(&format!(
                r#"{{"op":"read","path":"{}"}}"#,
                outside.display()
            )),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fs.external"));

        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[tokio::test]
    async fn test_unknown_op() {
        let ws = temp_workspace();
        let result = execute_fs_tool(&ws, &tool_call(r#"{"op":"unknown","path":"x.txt"}"#)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown fs operation"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn test_write_external_requires_fs_external() {
        let ws = temp_workspace();
        let outside = std::env::temp_dir().join(format!(
            "anacleto_fs_write_outside_{}",
            uuid::Uuid::new_v4()
        ));

        // An absolute external path is denied for writes.
        let result = execute_fs_tool(
            &ws,
            &tool_call(&format!(
                r#"{{"op":"write","path":"{}","content":"hello"}}"#,
                outside.display()
            )),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fs.external"));

        std::fs::remove_dir_all(&ws).unwrap();
    }
}
