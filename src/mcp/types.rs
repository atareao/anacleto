use serde::{Deserialize, Serialize};

use crate::config::McpDefinition;

/// Unique name for an MCP server instance.
pub type McpName = String;

/// Transport type for MCP communication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpTransport {
    /// Communicate over stdin/stdout of a child process.
    Stdio { command: String, args: Vec<String> },
    /// Communicate over TCP.
    Tcp { host: String, port: u16 },
}

impl From<&McpDefinition> for McpTransport {
    fn from(def: &McpDefinition) -> Self {
        match def.transport.as_str() {
            "tcp" => McpTransport::Tcp {
                host: def.host.clone().unwrap_or_else(|| "localhost".into()),
                port: def.port.unwrap_or(8080),
            },
            _ => McpTransport::Stdio {
                command: def.command.clone().unwrap_or_default(),
                args: def.args.clone(),
            },
        }
    }
}

/// A tool exposed by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for tool parameters.
    #[serde(default)]
    pub input_schema: serde_json::Value,
}

/// A resource exposed by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    /// Resource URI.
    pub uri: String,
    /// Resource name.
    pub name: String,
    /// Resource description.
    pub description: Option<String>,
    /// MIME type.
    pub mime_type: Option<String>,
}

/// Capabilities advertised by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpCapabilities {
    /// Whether the server supports tools.
    #[serde(default)]
    pub tools: bool,
    /// Whether the server supports resources.
    #[serde(default)]
    pub resources: bool,
    /// Whether the server supports prompts.
    #[serde(default)]
    pub prompts: bool,
}

/// Information about a connected MCP server.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    /// Server name.
    pub name: McpName,
    /// Server version.
    pub version: String,
    /// Server capabilities.
    pub capabilities: McpCapabilities,
    /// Available tools.
    pub tools: Vec<McpTool>,
    /// Available resources.
    pub resources: Vec<McpResource>,
}

/// Request to call an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCall {
    /// Tool name.
    pub name: String,
    /// Arguments as a JSON object.
    pub arguments: serde_json::Value,
}

/// Result of an MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Whether the call succeeded.
    pub success: bool,
    /// Result content.
    pub content: serde_json::Value,
    /// Error message if failed.
    pub error: Option<String>,
}

/// JSON-RPC message types for MCP protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request {
        jsonrpc: String,
        id: u64,
        method: String,
        #[serde(default)]
        params: serde_json::Value,
    },
    Response {
        jsonrpc: String,
        id: u64,
        #[serde(default)]
        result: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<JsonRpcError>,
    },
    Notification {
        jsonrpc: String,
        method: String,
        #[serde(default)]
        params: serde_json::Value,
    },
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}
