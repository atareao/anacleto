use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::McpDefinition;
use crate::error::{Error, Result};

use super::parse::{
    parse_read_resource_response, parse_resource_templates_response, parse_resources_response,
};
use super::types::*;

pub use super::registry::McpRegistry;

/// A connected MCP client.
pub struct McpClient {
    /// Server name.
    name: McpName,
    /// Transport configuration.
    transport: McpTransport,
    /// Child process (for stdio transport).
    child: Option<Mutex<Child>>,
    /// Server info (populated after initialize).
    info: Option<McpServerInfo>,
    /// Next request ID.
    next_id: u64,
    /// Stdin handle for sending requests (stdio transport).
    stdin: Option<tokio::process::ChildStdin>,
    /// Stdout handle for reading responses (stdio transport).
    stdout: Option<tokio::io::BufReader<tokio::process::ChildStdout>>,
    /// TCP stream (tcp transport).
    tcp_stream: Option<TcpStream>,
}

impl McpClient {
    /// Create a new MCP client from a definition.
    pub fn new(name: McpName, definition: &McpDefinition) -> Self {
        Self {
            name,
            transport: McpTransport::from(definition),
            child: None,
            info: None,
            next_id: 1,
            stdin: None,
            stdout: None,
            tcp_stream: None,
        }
    }

    /// Connect to the MCP server and perform handshake.
    pub async fn connect(&mut self) -> Result<McpServerInfo> {
        let transport = self.transport.clone();
        match &transport {
            McpTransport::Stdio { command, args } => self.connect_stdio(command, args).await,
            McpTransport::Tcp { host, port } => self.connect_tcp(host, *port).await,
        }
    }

    async fn connect_stdio(&mut self, command: &str, args: &[String]) -> Result<McpServerInfo> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| Error::Mcp(format!("Failed to spawn MCP server: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Mcp("Failed to capture stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Mcp("Failed to capture stdout".into()))?;
        let reader = tokio::io::BufReader::new(stdout);

        self.stdin = Some(stdin);
        self.stdout = Some(reader);

        // Perform initialize handshake
        let info = self.perform_initialize().await?;

        self.child = Some(Mutex::new(child));
        self.info = Some(info.clone());
        Ok(info)
    }

    async fn connect_tcp(&mut self, host: &str, port: u16) -> Result<McpServerInfo> {
        let addr = format!("{}:{}", host, port);
        let stream = TcpStream::connect(&addr).await.map_err(|e| {
            Error::Mcp(format!(
                "Failed to connect to TCP MCP server at {}: {}",
                addr, e
            ))
        })?;
        self.tcp_stream = Some(stream);
        let info = self.perform_initialize().await?;
        self.info = Some(info.clone());
        Ok(info)
    }

    async fn perform_initialize(&mut self) -> Result<McpServerInfo> {
        // Send initialize request
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "anacleto",
                    "version": "0.1.0"
                }
            }
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;

        // Parse server info from response
        let result = response
            .get("result")
            .ok_or_else(|| Error::Mcp("Initialize response missing 'result'".into()))?;

        let server_name = result
            .get("serverInfo")
            .and_then(|s| s.get("name"))
            .and_then(|s| s.as_str())
            .unwrap_or(&self.name)
            .to_string();
        let server_version = result
            .get("serverInfo")
            .and_then(|s| s.get("version"))
            .and_then(|s| s.as_str())
            .unwrap_or("0.1.0")
            .to_string();
        let capabilities = result.get("capabilities").cloned().unwrap_or_default();

        // Send initialized notification
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        self.send_jsonrpc(&notification).await?;

        // Helper: check if a capability is supported.
        // The MCP spec allows capabilities to be either a boolean (`true`) or
        // an object (`{}` with optional sub-capabilities). Both mean supported.
        let capability_supported = |key: &str| -> bool {
            capabilities
                .get(key)
                .is_some_and(|v| v.is_boolean() && v.as_bool().unwrap_or(false) || v.is_object())
        };

        // List tools if supported
        let tools = if capability_supported("tools") {
            self.list_tools_inner().await.unwrap_or_default()
        } else {
            vec![]
        };

        Ok(McpServerInfo {
            name: server_name,
            version: server_version,
            capabilities: McpCapabilities {
                tools: capability_supported("tools"),
                resources: capability_supported("resources"),
                prompts: capability_supported("prompts"),
            },
            tools,
            resources: vec![],
        })
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(&mut self, call: McpToolCall) -> Result<McpToolResult> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "tools/call",
            "params": {
                "name": call.name,
                "arguments": call.arguments
            }
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;

        if let Some(error) = response.get("error") {
            return Ok(McpToolResult {
                success: false,
                content: serde_json::Value::Null,
                error: Some(
                    error
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error")
                        .to_string(),
                ),
            });
        }

        let result = response.get("result").cloned().unwrap_or_default();
        let content = result
            .get("content")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok(McpToolResult {
            success: true,
            content,
            error: None,
        })
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        self.list_tools_inner().await
    }

    /// Internal: send tools/list request and parse response.
    async fn list_tools_inner(&mut self) -> Result<Vec<McpTool>> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "tools/list",
            "params": {}
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;

        let result = response
            .get("result")
            .ok_or_else(|| Error::Mcp("tools/list response missing 'result'".into()))?;

        let tools = result
            .get("tools")
            .and_then(|t| serde_json::from_value::<Vec<McpTool>>(t.clone()).ok())
            .unwrap_or_default();

        Ok(tools)
    }

    /// List available resources from the MCP server.
    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "resources/list",
            "params": {}
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;
        parse_resources_response(&response)
    }

    /// List available resource templates from the MCP server.
    pub async fn list_resource_templates(&mut self) -> Result<Vec<McpResourceTemplate>> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "resources/templates/list",
            "params": {}
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;
        parse_resource_templates_response(&response)
    }

    /// Read a resource by URI from the MCP server.
    ///
    /// Text contents are concatenated into a single string. Binary contents
    /// (base64 `blob`) are returned as `data:<mime>;base64,<payload>` so the
    /// caller can distinguish them from plain text.
    pub async fn read_resource(&mut self, uri: &str) -> Result<String> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "resources/read",
            "params": {
                "uri": uri
            }
        });
        self.next_id += 1;

        self.send_jsonrpc(&request).await?;
        let response = self.read_jsonrpc().await?;
        parse_read_resource_response(&response)
    }

    /// Send a JSON-RPC message over the active transport (stdio or TCP).
    async fn send_jsonrpc(&mut self, message: &serde_json::Value) -> Result<()> {
        let data = serde_json::to_string(message)
            .map_err(|e| Error::Mcp(format!("Failed to serialize JSON-RPC message: {}", e)))?;
        let data = format!("{}\n", data);

        if let Some(stdin) = self.stdin.as_mut() {
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| Error::Mcp(format!("Failed to write to MCP stdin: {}", e)))?;
        } else if let Some(stream) = self.tcp_stream.as_mut() {
            stream
                .write_all(data.as_bytes())
                .await
                .map_err(|e| Error::Mcp(format!("Failed to write to MCP TCP stream: {}", e)))?;
        }
        Ok(())
    }

    /// Read a JSON-RPC response line from the active transport (stdio or TCP).
    async fn read_jsonrpc(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();

        if let Some(ref mut reader) = self.stdout {
            reader
                .read_line(&mut line)
                .await
                .map_err(|e| Error::Mcp(format!("Failed to read from MCP stdout: {}", e)))?;
        } else if let Some(ref mut stream) = self.tcp_stream {
            // Read byte by byte until newline
            loop {
                let mut byte = [0u8; 1];
                let n = stream.read(&mut byte).await.map_err(|e| {
                    Error::Mcp(format!("Failed to read from MCP TCP stream: {}", e))
                })?;
                if n == 0 {
                    break; // EOF
                }
                let c = byte[0] as char;
                line.push(c);
                if c == '\n' {
                    break;
                }
            }
        }

        if line.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }

        serde_json::from_str(&line)
            .map_err(|e| Error::Mcp(format!("Failed to parse JSON-RPC response: {}", e)))
    }

    /// Disconnect from the MCP server.
    pub async fn disconnect(&mut self) -> Result<()> {
        if let Some(child) = self.child.take() {
            let mut child = child.into_inner();
            child.kill().await.ok();
            child.wait().await.ok();
        }
        if let Some(stream) = self.tcp_stream.take() {
            drop(stream);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_client_new_defaults() {
        let def = McpDefinition {
            transport: "stdio".into(),
            command: Some("echo".into()),
            args: vec!["hello".into()],
            host: None,
            port: None,
        };
        let client = McpClient::new("test".into(), &def);
        assert_eq!(client.name, "test");
        assert!(client.stdin.is_none());
        assert!(client.stdout.is_none());
        assert!(client.tcp_stream.is_none());
    }
}
