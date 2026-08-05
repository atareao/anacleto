use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::config::McpDefinition;
use crate::error::{Error, Result};
use crate::llm::types::ToolDefinition;

use super::client::McpClient;
use super::types::{McpName, McpResource, McpResourceTemplate, McpToolCall};

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
    ) -> Vec<(String, String, ToolDefinition)> {
        let mut tools = Vec::new();
        for name in server_names {
            if let Some(client_lock) = self.clients.get(name) {
                let mut client = client_lock.lock().await;
                if let Ok(mcp_tools) = client.list_tools().await {
                    for t in mcp_tools {
                        let original_name = t.name.clone();
                        let tool_def = ToolDefinition {
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
