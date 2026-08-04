//! The `lsp_query` tool: query a Language Server Protocol (LSP) server for
//! hover, definition, references or diagnostics.
//!
//! This tool launches a language server process (e.g. `rust-analyzer`) over
//! stdio, so it requires the `command.run` permission.

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::lsp::{LspClient, LspPosition, LspQueryType, default_server_for_extension, path_to_uri};
use crate::permissions::checker::check_command_run;
use crate::permissions::types::Permissions;

/// Tool definition for the `lsp_query` tool.
pub fn lsp_query_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "lsp_query".to_string(),
        description: "Query a Language Server Protocol (LSP) server for code \
                       intelligence. Provide `server_command` (e.g. 'rust-analyzer' \
                       or 'typescript-language-server'), the `file_path` to analyze, \
                       the 0-based `line` and `character` position, and a `query_type` \
                       of 'hover', 'definition', 'references' or 'diagnostic'. \
                       If `server_command` is omitted, a default is chosen from the \
                       file extension. Launches the server process, so it requires \
                       the command.run permission."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server_command": {
                    "type": "string",
                    "description": "The LSP server executable (e.g. 'rust-analyzer'). Optional; inferred from the file extension if omitted."
                },
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to analyze."
                },
                "line": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "0-based line number of the position (ignored for 'diagnostic')."
                },
                "character": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "0-based character offset of the position (ignored for 'diagnostic')."
                },
                "query_type": {
                    "type": "string",
                    "enum": ["hover", "definition", "references", "diagnostic"],
                    "description": "The kind of LSP query to perform."
                }
            },
            "required": ["file_path", "query_type"]
        }),
    }
}

/// Execute an `lsp_query` tool call.
pub async fn execute_lsp_query_tool(
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    check_command_run(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse lsp_query arguments: {e}"))?;

    let file_path = args
        .get("file_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "lsp_query requires 'file_path'".to_string())?;
    let query_type = args
        .get("query_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "lsp_query requires 'query_type'".to_string())?;
    let query_type = LspQueryType::parse(query_type)
        .ok_or_else(|| format!("Unknown query_type '{query_type}'"))?;

    let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    // Resolve the server command: explicit, or inferred from the extension.
    let server_command = args
        .get("server_command")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            let ext = std::path::Path::new(file_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            default_server_for_extension(ext).map(|s| s.to_string())
        })
        .ok_or_else(|| {
            "No server_command provided and no default LSP server is known for this file extension"
                .to_string()
        })?;

    let uri = path_to_uri(file_path);
    let root_uri = path_to_uri(
        std::path::Path::new(file_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("."),
    );

    let position = LspPosition { line, character };
    let result = LspClient::query_once(&server_command, &[], &root_uri, &uri, position, query_type)
        .await
        .map_err(|e| format!("LSP query failed: {e}"))?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;
    use crate::permissions::types::Permissions;

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_lsp".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "lsp_query".into(),
                arguments: args.into(),
            },
        }
    }

    fn deny_command() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec!["command.run".into()],
            allow: vec![],
        })
    }

    fn allow_all() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec![],
        })
    }

    #[tokio::test]
    async fn lsp_query_denied_without_command_run() {
        let result = execute_lsp_query_tool(
            &deny_command(),
            &tool_call(r#"{"file_path":"/tmp/a.rs","query_type":"hover"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));
    }

    #[tokio::test]
    async fn lsp_query_missing_file_path_errors() {
        let result =
            execute_lsp_query_tool(&allow_all(), &tool_call(r#"{"query_type":"hover"}"#)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires 'file_path'"));
    }

    #[tokio::test]
    async fn lsp_query_unknown_query_type_errors() {
        let result = execute_lsp_query_tool(
            &allow_all(),
            &tool_call(r#"{"file_path":"/tmp/a.rs","query_type":"bogus"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown query_type"));
    }

    #[tokio::test]
    async fn lsp_query_unknown_extension_and_no_server_errors() {
        // No server_command and an unknown extension -> clear error, no process spawned.
        let result = execute_lsp_query_tool(
            &allow_all(),
            &tool_call(r#"{"file_path":"/tmp/a.xyz","query_type":"hover"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No server_command provided"));
    }
}
