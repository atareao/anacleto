//! The `format_document` tool: format a code file using its language server.
//!
//! This tool launches a language server process (e.g. `rust-analyzer`) over
//! stdio for the duration of the formatting request, so it requires the
//! `command.run` permission.

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::lsp::{LspClient, default_server_for_extension, path_to_uri};

/// Supported extensions for `format_document`.
const SUPPORTED_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "py", "go"];

/// Tool definition for the `format_document` tool.
pub fn format_document_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "format_document".to_string(),
        description:
            "Format a code file using its language server (LSP). Supports: .rs (rust-analyzer), \
             .ts/.tsx/.js/.jsx (typescript-language-server), .py (pyright-langserver), \
             .go (gopls)."
                .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to format"
                }
            },
            "required": ["path"]
        }),
    }
}

/// Execute a `format_document` tool call.
pub async fn execute_format_document_tool(tool_call: &ToolCall) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse format_document arguments: {e}"))?;

    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "format_document requires 'path'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::format",
        file_path = %file_path,
        "format_document tool"
    );

    // Detect extension and look up the LSP server.
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let server_command = default_server_for_extension(ext).ok_or_else(|| {
        format!(
            "No LSP server known for extension '.{ext}'. Supported extensions: {}",
            SUPPORTED_EXTENSIONS
                .iter()
                .map(|e| format!(".{e}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    if !std::path::Path::new(file_path).exists() {
        return Err(format!("File does not exist: {file_path}"));
    }

    let uri = path_to_uri(file_path);
    let root_uri = path_to_uri(
        std::path::Path::new(file_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("."),
    );

    let mut client = LspClient::new();
    client
        .start(server_command, &[], &root_uri)
        .await
        .map_err(|e| format!("Failed to start LSP server '{server_command}': {e}"))?;

    let content = tokio::fs::read_to_string(file_path)
        .await
        .map_err(|e| format!("Failed to read file '{file_path}': {e}"))?;

    let formatted = client
        .format_document(&uri, &content)
        .await
        .map_err(|e| format!("LSP formatting failed: {e}"))?;

    // Shut down the server regardless of write outcome.
    client.shutdown().await.ok();

    if formatted == content {
        return Ok(format!("File already formatted: {file_path}"));
    }

    tokio::fs::write(file_path, &formatted)
        .await
        .map_err(|e| format!("Failed to write formatted file '{file_path}': {e}"))?;

    Ok(format!("Formatted file: {file_path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn tool_call(args: &str) -> ToolCall {
        ToolCall {
            id: "call_format".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: "format_document".into(),
                arguments: args.into(),
            },
        }
    }

    #[test]
    fn test_format_document_definition() {
        let def = format_document_tool_definition();
        assert_eq!(def.name, "format_document");
        assert!(def.description.contains(".rs"));
        assert!(def.description.contains("rust-analyzer"));
        assert_eq!(def.input_schema["type"], "object");
        assert!(
            def.input_schema["required"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("path"))
        );
        assert_eq!(def.input_schema["properties"]["path"]["type"], "string");
    }

    #[tokio::test]
    async fn test_format_document_unknown_extension() {
        let result = execute_format_document_tool(&tool_call(r#"{"path":"/tmp/a.xyz"}"#)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("No LSP server known for extension '.xyz'"),
            "got: {err}"
        );
        for ext in SUPPORTED_EXTENSIONS {
            assert!(
                err.contains(&format!(".{ext}")),
                "missing {ext} in error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn test_format_document_missing_path_errors() {
        let result = execute_format_document_tool(&tool_call(r#"{}"#)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires 'path'"));
    }

    #[tokio::test]
    async fn test_format_document_file_not_found_errors() {
        let result =
            execute_format_document_tool(&tool_call(r#"{"path":"/tmp/does_not_exist_12345.rs"}"#))
                .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("File does not exist"));
    }
}
