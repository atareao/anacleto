use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::config::McpDefinition;
use crate::error::{Error, Result};

use super::types::*;

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

        // List tools if supported
        let tools = if capabilities
            .get("tools")
            .and_then(|t| t.as_bool())
            .unwrap_or(false)
        {
            self.list_tools_inner().await.unwrap_or_default()
        } else {
            vec![]
        };

        Ok(McpServerInfo {
            name: server_name,
            version: server_version,
            capabilities: McpCapabilities {
                tools: capabilities
                    .get("tools")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false),
                resources: capabilities
                    .get("resources")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false),
                prompts: capabilities
                    .get("prompts")
                    .and_then(|t| t.as_bool())
                    .unwrap_or(false),
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

/// Parse a `resources/list` JSON-RPC response into a list of resources.
fn parse_resources_response(response: &serde_json::Value) -> Result<Vec<McpResource>> {
    if let Some(error) = response.get("error") {
        return Err(Error::Mcp(
            error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string(),
        ));
    }

    let result = response
        .get("result")
        .ok_or_else(|| Error::Mcp("resources/list response missing 'result'".into()))?;

    let resources = result
        .get("resources")
        .and_then(|r| serde_json::from_value::<Vec<McpResource>>(r.clone()).ok())
        .unwrap_or_default();

    Ok(resources)
}

/// Parse a `resources/templates/list` JSON-RPC response into a list of templates.
fn parse_resource_templates_response(
    response: &serde_json::Value,
) -> Result<Vec<McpResourceTemplate>> {
    if let Some(error) = response.get("error") {
        return Err(Error::Mcp(
            error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string(),
        ));
    }

    let result = response
        .get("result")
        .ok_or_else(|| Error::Mcp("resources/templates/list response missing 'result'".into()))?;

    let templates = result
        .get("resourceTemplates")
        .and_then(|t| serde_json::from_value::<Vec<McpResourceTemplate>>(t.clone()).ok())
        .unwrap_or_default();

    Ok(templates)
}

/// Parse a `resources/read` JSON-RPC response into a string.
///
/// Text contents are concatenated; binary `blob` contents are exposed as
/// `data:<mime>;base64,<payload>` data URIs.
fn parse_read_resource_response(response: &serde_json::Value) -> Result<String> {
    if let Some(error) = response.get("error") {
        return Err(Error::Mcp(
            error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string(),
        ));
    }

    let result = response
        .get("result")
        .ok_or_else(|| Error::Mcp("resources/read response missing 'result'".into()))?;

    let contents = result
        .get("contents")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    if contents.is_empty() {
        return Ok(String::new());
    }

    let mut out = String::new();
    for item in contents {
        let mime = item
            .get("mimeType")
            .and_then(|m| m.as_str())
            .unwrap_or("text/plain");
        if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
            out.push_str(text);
        } else if let Some(blob) = item.get("blob").and_then(|b| b.as_str()) {
            // Binary content: expose as a data URI with mime metadata.
            out.push_str(&format!("data:{mime};base64,{blob}"));
        }
        out.push('\n');
    }

    Ok(out)
}

/// Registry of connected MCP clients.
pub struct McpRegistry {
    clients: HashMap<McpName, Mutex<McpClient>>,
}

impl McpRegistry {
    /// Creates an empty MCP registry.
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    /// Register and connect an MCP server.
    pub async fn register(&mut self, name: McpName, definition: &McpDefinition) -> Result<()> {
        let mut client = McpClient::new(name.clone(), definition);
        client.connect().await?;
        self.clients.insert(name, Mutex::new(client));
        Ok(())
    }

    /// Get a client by name.
    pub fn get(&self, name: &str) -> Option<&Mutex<McpClient>> {
        self.clients.get(name)
    }

    /// List the names of all registered MCP servers.
    pub fn names(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// Collect tools from specific MCP servers and convert to ToolDefinitions.
    /// Returns a list of (server_name, original_tool_name, ToolDefinition) tuples.
    pub async fn collect_tools(
        &self,
        server_names: &[String],
    ) -> Vec<(String, String, crate::llm::types::ToolDefinition)> {
        let mut tools = Vec::new();
        for name in server_names {
            if let Some(client_lock) = self.clients.get(name) {
                let mut client = client_lock.lock().await;
                if let Ok(mcp_tools) = client.list_tools().await {
                    for t in mcp_tools {
                        let original_name = t.name.clone();
                        let tool_def = crate::llm::types::ToolDefinition {
                            name: format!("{}_{}", name, t.name),
                            description: format!("[MCP {}] {}", name, t.description),
                            input_schema: t.input_schema,
                        };
                        tools.push((name.clone(), original_name, tool_def));
                    }
                }
            }
        }
        tools
    }

    /// Execute an MCP tool call.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String> {
        let client_lock = self
            .clients
            .get(server_name)
            .ok_or_else(|| Error::Mcp(format!("MCP server '{}' not found", server_name)))?;

        let mut client = client_lock.lock().await;
        let result = client
            .call_tool(McpToolCall {
                name: tool_name.to_string(),
                arguments,
            })
            .await?;

        if result.success {
            Ok(serde_json::to_string_pretty(&result.content).unwrap_or_else(|_| "{}".to_string()))
        } else {
            Err(Error::Mcp(
                result.error.unwrap_or_else(|| "Unknown error".into()),
            ))
        }
    }

    /// List resources from a specific MCP server.
    pub async fn list_resources(&self, server_name: &str) -> Result<Vec<McpResource>> {
        let client_lock = self
            .clients
            .get(server_name)
            .ok_or_else(|| Error::Mcp(format!("MCP server '{}' not found", server_name)))?;
        let mut client = client_lock.lock().await;
        client.list_resources().await
    }

    /// List resource templates from a specific MCP server.
    pub async fn list_resource_templates(
        &self,
        server_name: &str,
    ) -> Result<Vec<McpResourceTemplate>> {
        let client_lock = self
            .clients
            .get(server_name)
            .ok_or_else(|| Error::Mcp(format!("MCP server '{}' not found", server_name)))?;
        let mut client = client_lock.lock().await;
        client.list_resource_templates().await
    }

    /// Read a resource by URI from a specific MCP server.
    pub async fn read_resource(&self, server_name: &str, uri: &str) -> Result<String> {
        let client_lock = self
            .clients
            .get(server_name)
            .ok_or_else(|| Error::Mcp(format!("MCP server '{}' not found", server_name)))?;
        let mut client = client_lock.lock().await;
        client.read_resource(uri).await
    }

    /// Disconnect all clients.
    pub async fn disconnect_all(&mut self) {
        for client in self.clients.values_mut() {
            client.get_mut().disconnect().await.ok();
        }
        self.clients.clear();
    }
}

impl Default for McpRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_transport_from_config_stdio() {
        let def = McpDefinition {
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec![
                "-y".into(),
                "@modelcontextprotocol/server-filesystem".into(),
            ],
            host: None,
            port: None,
        };
        let transport = McpTransport::from(&def);
        match transport {
            McpTransport::Stdio { command, args } => {
                assert_eq!(command, "npx");
                assert!(!args.is_empty());
            }
            _ => panic!("Expected Stdio transport"),
        }
    }

    #[test]
    fn test_mcp_transport_from_config_tcp() {
        let def = McpDefinition {
            transport: "tcp".into(),
            command: None,
            args: vec![],
            host: Some("localhost".into()),
            port: Some(5432),
        };
        let transport = McpTransport::from(&def);
        match transport {
            McpTransport::Tcp { host, port } => {
                assert_eq!(host, "localhost");
                assert_eq!(port, 5432);
            }
            _ => panic!("Expected Tcp transport"),
        }
    }

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

    #[test]
    fn test_parse_resources_response() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "resources": [
                    {
                        "uri": "file:///tmp/a.txt",
                        "name": "a.txt",
                        "description": "A file",
                        "mimeType": "text/plain"
                    },
                    {
                        "uri": "file:///tmp/b.png",
                        "name": "b.png"
                    }
                ]
            }
        });
        let resources = parse_resources_response(&response).unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].uri, "file:///tmp/a.txt");
        assert_eq!(resources[0].name, "a.txt");
        assert_eq!(resources[0].description.as_deref(), Some("A file"));
        assert_eq!(resources[0].mime_type.as_deref(), Some("text/plain"));
        // Optional fields default to None.
        assert_eq!(resources[1].description, None);
        assert_eq!(resources[1].mime_type, None);
    }

    #[test]
    fn test_parse_resources_response_error() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        });
        let err = parse_resources_response(&response).unwrap_err();
        assert!(err.to_string().contains("Method not found"));
    }

    #[test]
    fn test_parse_resource_templates_response() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "resourceTemplates": [
                    {
                        "uriTemplate": "file:///{path}",
                        "name": "Any file",
                        "description": "Read any file",
                        "mimeType": "text/plain"
                    }
                ]
            }
        });
        let templates = parse_resource_templates_response(&response).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].uri_template, "file:///{path}");
        assert_eq!(templates[0].name, "Any file");
        assert_eq!(templates[0].description.as_deref(), Some("Read any file"));
        assert_eq!(templates[0].mime_type.as_deref(), Some("text/plain"));
    }

    #[test]
    fn test_parse_read_resource_response_text() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "contents": [
                    { "uri": "file:///tmp/a.txt", "mimeType": "text/plain", "text": "hello world" }
                ]
            }
        });
        let content = parse_read_resource_response(&response).unwrap();
        assert!(content.contains("hello world"));
    }

    #[test]
    fn test_parse_read_resource_response_binary_blob() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "contents": [
                    { "uri": "file:///tmp/b.png", "mimeType": "image/png", "blob": "aGVsbG8=" }
                ]
            }
        });
        let content = parse_read_resource_response(&response).unwrap();
        assert!(content.contains("data:image/png;base64,aGVsbG8="));
    }

    #[test]
    fn test_parse_read_resource_response_empty() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "contents": [] }
        });
        let content = parse_read_resource_response(&response).unwrap();
        assert!(content.is_empty());
    }
}
