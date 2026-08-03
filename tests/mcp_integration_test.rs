//! Integration tests for the MCP client module.
//!
//! These tests spawn a mock Python MCP server over stdio and verify
//! the full JSON-RPC 2.0 communication lifecycle: connect, list tools,
//! call tools, and disconnect.

use anacleto::config::McpDefinition;
use anacleto::mcp::client::{McpClient, McpRegistry};
use anacleto::mcp::types::{McpToolCall, McpTransport};
use std::path::PathBuf;
use tempfile::TempDir;

/// Path to the mock MCP server script relative to the project root.
fn mock_server_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/mocks/mcp_server.py")
}

/// Build an McpDefinition pointing at the mock Python server.
fn mock_stdio_definition() -> McpDefinition {
    McpDefinition {
        transport: "stdio".into(),
        command: Some("python3".into()),
        args: vec![mock_server_path().to_string_lossy().to_string()],
        host: None,
        port: None,
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Runtime::new().unwrap()
}

#[test]
fn test_mcp_client_connect_stdio() {
    let _dir = TempDir::new().unwrap();
    let def = mock_stdio_definition();
    let mut client = McpClient::new("mock".into(), &def);
    let rt = runtime();

    rt.block_on(async {
        let info = client.connect().await.unwrap();
        assert_eq!(info.name, "mock-server");
        assert_eq!(info.version, "1.0.0");
        assert!(info.capabilities.tools);
        assert!(info.capabilities.resources);
        assert!(!info.tools.is_empty());
        assert_eq!(info.tools[0].name, "echo");

        client.disconnect().await.unwrap();
    });
}

#[test]
fn test_mcp_client_call_tool() {
    let _dir = TempDir::new().unwrap();
    let def = mock_stdio_definition();
    let mut client = McpClient::new("mock".into(), &def);
    let rt = runtime();

    rt.block_on(async {
        let _info = client.connect().await.unwrap();

        let result = client
            .call_tool(McpToolCall {
                name: "echo".into(),
                arguments: serde_json::json!({"message": "hello"}),
            })
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.error.is_none());
        let text = result.content[0]["text"].as_str().unwrap();
        assert_eq!(text, "hello");

        client.disconnect().await.unwrap();
    });
}

#[test]
fn test_mcp_client_disconnect() {
    let _dir = TempDir::new().unwrap();
    let def = mock_stdio_definition();
    let mut client = McpClient::new("mock".into(), &def);
    let rt = runtime();

    rt.block_on(async {
        let _info = client.connect().await.unwrap();

        // First disconnect should succeed
        client.disconnect().await.unwrap();

        // Second disconnect is a no-op (child already taken) — should not error
        client.disconnect().await.unwrap();
    });
}

#[test]
fn test_mcp_registry_integration() {
    let _dir = TempDir::new().unwrap();
    let def = mock_stdio_definition();
    let rt = runtime();

    rt.block_on(async {
        let mut registry = McpRegistry::new();
        registry.register("mock".into(), &def).await.unwrap();

        // Collect tools from the registered server
        let tools = registry.collect_tools(&["mock".to_string()]).await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "mock");
        assert_eq!(tools[0].1, "echo");
        // The tool definition name is prefixed with server name
        assert!(tools[0].2.name.contains("mock"));
        assert!(tools[0].2.name.contains("echo"));

        // Call a tool via the registry
        let result = registry
            .call_tool("mock", "echo", serde_json::json!({"message": "world"}))
            .await
            .unwrap();
        assert!(result.contains("world"));

        // Clean disconnect of all clients
        registry.disconnect_all().await;
    });
}

#[test]
fn test_mcp_transport_from_config_stdio() {
    let _dir = TempDir::new().unwrap();
    let def = mock_stdio_definition();
    let transport = McpTransport::from(&def);
    match transport {
        McpTransport::Stdio { command, args } => {
            assert_eq!(command, "python3");
            assert!(args[0].ends_with("mcp_server.py"));
        }
        _ => panic!("Expected Stdio transport"),
    }
}
