//! MCP resource tools: `mcp_list_resources`, `mcp_read_resource` and
//! `mcp_list_resource_templates`.
//!
//! These let the agent inspect and read resources exposed by a connected MCP
//! server. All three require the `mcp.use` permission.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::mcp::client::McpRegistry;

/// Tool definition for `mcp_list_resources`: list the resources of an MCP server.
pub fn mcp_list_resources_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "mcp_list_resources".to_string(),
        description: "List resources exposed by a connected MCP server.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string"
                }
            },
            "required": ["server"]
        }),
    }
}

/// Tool definition for `mcp_read_resource`: read a resource by URI.
pub fn mcp_read_resource_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "mcp_read_resource".to_string(),
        description: "Read a resource from a connected MCP server by URI.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string"
                },
                "uri": {
                    "type": "string"
                }
            },
            "required": ["server", "uri"]
        }),
    }
}

/// Tool definition for `mcp_list_resource_templates`: list resource templates.
pub fn mcp_list_resource_templates_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "mcp_list_resource_templates".to_string(),
        description: "List resource templates from a connected MCP server.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "server": {
                    "type": "string"
                }
            },
            "required": ["server"]
        }),
    }
}

/// Execute a `mcp_list_resources` tool call.
pub async fn execute_mcp_list_resources_tool(
    mcp_registry: &Arc<Mutex<McpRegistry>>,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse mcp_list_resources arguments: {e}"))?;
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp_list_resources requires 'server'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::mcp",
        server = %server,
        "mcp_list_resources tool"
    );

    let registry = mcp_registry.lock().await;
    let resources = registry
        .list_resources(server)
        .await
        .map_err(|e| format!("Failed to list resources from '{server}': {e}"))?;

    if resources.is_empty() {
        return Ok(format!("MCP server '{server}' exposes no resources."));
    }

    let mut out = format!("Resources from MCP server '{server}':\n");
    for r in &resources {
        let desc = r.description.as_deref().unwrap_or("-");
        let mime = r.mime_type.as_deref().unwrap_or("-");
        out.push_str(&format!("- {} ({}) [{}] — {}\n", r.name, r.uri, mime, desc));
    }
    Ok(out)
}

/// Execute a `mcp_read_resource` tool call.
pub async fn execute_mcp_read_resource_tool(
    mcp_registry: &Arc<Mutex<McpRegistry>>,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse mcp_read_resource arguments: {e}"))?;
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp_read_resource requires 'server'".to_string())?;
    let uri = args
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp_read_resource requires 'uri'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::mcp",
        server = %server,
        uri = %uri,
        "mcp_read_resource tool"
    );

    let registry = mcp_registry.lock().await;
    let content = registry
        .read_resource(server, uri)
        .await
        .map_err(|e| format!("Failed to read resource '{uri}' from '{server}': {e}"))?;

    Ok(format!("Resource '{uri}' from '{server}':\n\n{content}"))
}

/// Execute a `mcp_list_resource_templates` tool call.
pub async fn execute_mcp_list_resource_templates_tool(
    mcp_registry: &Arc<Mutex<McpRegistry>>,
    tool_call: &ToolCall,
) -> Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse mcp_list_resource_templates arguments: {e}"))?;
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp_list_resource_templates requires 'server'".to_string())?;

    tracing::debug!(
        target: "anacleto::tools::mcp",
        server = %server,
        "mcp_list_resource_templates tool"
    );

    let registry = mcp_registry.lock().await;
    let templates = registry
        .list_resource_templates(server)
        .await
        .map_err(|e| format!("Failed to list resource templates from '{server}': {e}"))?;

    if templates.is_empty() {
        return Ok(format!(
            "MCP server '{server}' exposes no resource templates."
        ));
    }

    let mut out = format!("Resource templates from MCP server '{server}':\n");
    for t in &templates {
        let desc = t.description.as_deref().unwrap_or("-");
        let mime = t.mime_type.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "- {} ({}) [{}] — {}\n",
            t.name, t.uri_template, mime, desc
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ToolFunction;

    fn tool_call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "call_mcp".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    #[tokio::test]
    async fn mcp_list_resources_unknown_server_errors() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_mcp_list_resources_tool(
            &registry,
            &tool_call("mcp_list_resources", r#"{"server":"nope"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn mcp_read_resource_missing_uri_errors() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_mcp_read_resource_tool(
            &registry,
            &tool_call("mcp_read_resource", r#"{"server":"filesystem"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires 'uri'"));
    }
}
