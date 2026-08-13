//! The `read` tool: read a file with optional line offset/limit and pagination.
//!
//! Paths are resolved relative to the workspace. Reading a path that escapes
//! the workspace (absolute path or `..` traversal) additionally requires the
//! `fs.external` permission.

use std::path::{Path, PathBuf};

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::permissions::checker::{check_fs_external, check_fs_read};
use crate::permissions::types::Permissions;

/// Maximum number of lines returned per `read` call.
pub const MAX_LINES: usize = 2000;
/// Maximum number of bytes returned per `read` call.
pub const MAX_BYTES: usize = 50 * 1024;

/// Tool definition for the `read` tool.
pub fn read_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read".to_string(),
        description:
            "Read a file with line numbers. Supports offset (1-based) and limit (max 2000)."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 2000
                }
            },
            "required": ["path"]
        }),
    }
}

/// Resolve a read path, enforcing workspace containment unless the caller has
/// the `fs.external` permission.
fn resolve_read_path(
    workspace: &Path,
    path: &str,
    external_granted: bool,
) -> Result<PathBuf, String> {
    let p = Path::new(path);
    if p.is_absolute() {
        if external_granted {
            Ok(p.to_path_buf())
        } else {
            Err("Reading outside the workspace requires the 'fs.external' permission".to_string())
        }
    } else {
        match crate::engine::apply_patch::resolve_within_workspace(workspace, path) {
            Ok(full) => Ok(full),
            Err(_) if external_granted => Ok(workspace.join(p)),
            Err(e) => Err(e),
        }
    }
}

/// Execute a `read` tool call.
pub async fn execute_read_tool(
    workspace: &Path,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse read arguments: {e}"))?;
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "read requires 'path'".to_string())?;
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

    // Base permission: reading files requires fs.read.
    check_fs_read(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // If the path is outside the workspace, additionally require fs.external.
    let external_granted = check_fs_external(permissions).is_ok();
    let full = resolve_read_path(workspace, path, external_granted)?;

    let bytes =
        std::fs::read(&full).map_err(|e| format!("Failed to read '{}': {e}", full.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;
    use crate::permissions::types::Permissions;

    fn temp_workspace() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anacleto_read_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_read".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "read".into(),
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
    async fn read_basic_with_line_numbers() {
        let ws = temp_workspace();
        std::fs::write(ws.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        let result = execute_read_tool(&ws, &allow_all(), &tool_call(r#"{"path":"a.txt"}"#))
            .await
            .unwrap();
        assert!(result.contains("1 | one"));
        assert!(result.contains("2 | two"));
        assert!(result.contains("3 | three"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn read_with_offset_and_limit() {
        let ws = temp_workspace();
        let content: String = (1..=10).map(|i| format!("line{i}\n")).collect();
        std::fs::write(ws.join("b.txt"), content).unwrap();
        let result = execute_read_tool(
            &ws,
            &allow_all(),
            &tool_call(r#"{"path":"b.txt","offset":3,"limit":2}"#),
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
    async fn read_rejects_path_traversal() {
        let ws = temp_workspace();
        let result =
            execute_read_tool(&ws, &allow_all(), &tool_call(r#"{"path":"../secret.txt"}"#)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("escapes workspace") || err.contains("fs.external"));
        std::fs::remove_dir_all(&ws).unwrap();
    }

    #[tokio::test]
    async fn read_external_requires_fs_external() {
        let ws = temp_workspace();
        let outside =
            std::env::temp_dir().join(format!("anacleto_outside_{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, "external data").unwrap();

        // Without fs.external, an absolute external path is denied.
        let result = execute_read_tool(
            &ws,
            &allow_all(),
            &tool_call(&format!(r#"{{"path":"{}"}}"#, outside.display())),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("fs.external"));

        // With fs.external granted, it is allowed.
        let perms = Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec!["fs.read".into(), "fs.external".into()],
        });
        let result = execute_read_tool(
            &ws,
            &perms,
            &tool_call(&format!(r#"{{"path":"{}"}}"#, outside.display())),
        )
        .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("external data"));

        std::fs::remove_dir_all(&ws).unwrap();
        std::fs::remove_file(&outside).unwrap();
    }

    #[tokio::test]
    async fn read_missing_file_errors() {
        let ws = temp_workspace();
        let result =
            execute_read_tool(&ws, &allow_all(), &tool_call(r#"{"path":"nope.txt"}"#)).await;
        assert!(result.is_err());
        std::fs::remove_dir_all(&ws).unwrap();
    }
}
