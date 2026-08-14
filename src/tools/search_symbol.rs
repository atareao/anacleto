//! Builtin tool `search_symbol`: search for symbol definitions in the codebase
//! using the CodeGraph MCP server.
//!
//! Returns symbol name, kind, file location, and signature. Supports filtering
//! by symbol kind. Requires the `mcp.use` permission.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::mcp::client::McpRegistry;
use crate::permissions::checker::check_mcp_use;
use crate::permissions::types::Permissions;

/// Valid kinds for the `kind` filter parameter.
const VALID_KINDS: &[&str] = &[
    "function",
    "method",
    "struct",
    "enum",
    "trait",
    "type",
    "variable",
    "interface",
    "component",
    "route",
];

// ---------------------------------------------------------------------------
// Tool definition
// ---------------------------------------------------------------------------

/// Tool definition for `search_symbol`: search for symbol definitions via
/// CodeGraph.
pub fn search_symbol_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_symbol".to_string(),
        description: "Search for symbol definitions in the codebase using CodeGraph. \
             Returns symbol name, kind, file location, and signature. \
             Supports filtering by symbol kind."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string"
                },
                "kind": {
                    "type": "string",
                    "enum": VALID_KINDS
                },
                "path": {
                    "type": "string"
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["query"]
        }),
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Execute a `search_symbol` tool call.
pub async fn execute_search_symbol_tool(
    mcp_registry: &Arc<Mutex<McpRegistry>>,
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    check_mcp_use(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    // Parse arguments -------------------------------------------------------
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse search_symbol arguments: {e}"))?;

    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "search_symbol requires a non-empty 'query' string.".to_string())?;

    // Validate optional kind ------------------------------------------------
    let kind = args.get("kind").and_then(|v| v.as_str());
    if let Some(k) = kind
        && !VALID_KINDS.contains(&k)
    {
        return Err(format!(
            "Invalid kind '{k}'. Valid kinds: {}.",
            VALID_KINDS.join(", ")
        ));
    }

    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_i64())
        .map(|n| n.clamp(1, 50))
        .unwrap_or(10);

    // Build the arguments for CodeGraph's codegraph_search tool -------------
    let mut cg_args = serde_json::json!({
        "query": query,
        "limit": max_results,
    });
    if let Some(k) = kind {
        cg_args["kind"] = serde_json::json!(k);
    }
    if let Some(p) = args
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        cg_args["path"] = serde_json::json!(p);
    }

    // Call CodeGraph --------------------------------------------------------
    let registry = mcp_registry.lock().await;
    let raw = registry
        .call_tool("codegraph", "codegraph_search", cg_args)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("'codegraph'") {
                "CodeGraph MCP server is not available. Use 'grep' for text-based search."
                    .to_string()
            } else {
                format!("CodeGraph search failed: {msg}")
            }
        })?;

    // Parse the result ------------------------------------------------------
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);

    let symbols = match &parsed {
        // CodeGraph may return an array directly
        Value::Array(items) => items.clone(),
        // Or a single object
        Value::Object(_) if parsed.get("name").is_some() => vec![parsed.clone()],
        // Unknown shape — return raw text
        _ => return Ok(raw),
    };

    if symbols.is_empty() {
        return Ok(format!("No symbols found matching \"{query}\"."));
    }

    Ok(format_search_results(query, &symbols, max_results))
}

// ---------------------------------------------------------------------------
// Formatting helper (also tested independently)
// ---------------------------------------------------------------------------

/// Format a list of symbol results into a structured text block suitable for
/// the LLM to read.
fn format_search_results(query: &str, symbols: &[serde_json::Value], max_results: i64) -> String {
    let count = symbols.len();
    let header = format!(
        "Found {count} symbol(s) matching \"{query}\":\n\n",
        count = if count >= max_results as usize {
            format!("{}+", max_results)
        } else {
            count.to_string()
        },
    );

    let mut out = String::with_capacity(header.len() + count * 160);
    out.push_str(&header);

    for (i, sym) in symbols.iter().enumerate() {
        let name = sym
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("<unnamed>");
        let kind = sym.get("kind").and_then(|v| v.as_str()).unwrap_or("symbol");
        let file = sym.get("file").and_then(|v| v.as_str()).unwrap_or("?");
        let line = sym.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
        let signature = sym.get("signature").and_then(|v| v.as_str()).unwrap_or("");

        out.push_str(&format!(
            "{}. {} ({}) — {}:{}\n",
            i + 1,
            name,
            kind,
            file,
            line
        ));
        if !signature.is_empty() {
            out.push_str(&format!("   {signature}\n"));
        }
    }

    out
}

// Used inside parse_json_value destructuring.
use serde_json::Value;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;

    // -- helpers ------------------------------------------------------------

    fn tool_call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "call_search_sym".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: name.into(),
                arguments: args.into(),
            },
        }
    }

    fn deny_mcp() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec!["mcp.use".into()],
            allow: vec![],
        })
    }

    fn allow_all() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec![],
            allow: vec![],
        })
    }

    // -- definition tests ---------------------------------------------------

    #[test]
    fn test_search_symbol_definition() {
        let def = search_symbol_tool_definition();
        assert_eq!(def.name, "search_symbol");
        assert!(!def.description.is_empty());

        let schema = def.input_schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["query"].is_object());
        assert_eq!(schema["properties"]["query"]["type"], "string");
        assert!(schema["properties"]["kind"].is_object());
        assert!(schema["properties"]["kind"]["enum"].is_array());
        assert!(schema["properties"]["max_results"].is_object());
        assert!(schema["properties"]["path"].is_object());

        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    // -- permission tests ---------------------------------------------------

    #[tokio::test]
    async fn test_search_symbol_denied_without_mcp_use() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_search_symbol_tool(
            &registry,
            &deny_mcp(),
            &tool_call("search_symbol", r#"{"query":"foo"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));
    }

    // -- argument validation tests ------------------------------------------

    #[tokio::test]
    async fn test_search_symbol_missing_query() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_search_symbol_tool(
            &registry,
            &allow_all(),
            &tool_call("search_symbol", r#"{}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-empty 'query'"));
    }

    #[tokio::test]
    async fn test_search_symbol_empty_query() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_search_symbol_tool(
            &registry,
            &allow_all(),
            &tool_call("search_symbol", r#"{"query":""}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-empty 'query'"));
    }

    #[tokio::test]
    async fn test_search_symbol_invalid_kind() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_search_symbol_tool(
            &registry,
            &allow_all(),
            &tool_call("search_symbol", r#"{"query":"foo","kind":"invalid_kind"}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid kind"));
        assert!(err.contains("function"));
        assert!(err.contains("method"));
    }

    #[tokio::test]
    async fn test_search_symbol_codegraph_not_available() {
        let registry = Arc::new(Mutex::new(McpRegistry::new()));
        let result = execute_search_symbol_tool(
            &registry,
            &allow_all(),
            &tool_call("search_symbol", r#"{"query":"foo"}"#),
        )
        .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("CodeGraph MCP server is not available"));
        assert!(err.contains("grep"));
    }

    // -- formatting tests ---------------------------------------------------

    #[test]
    fn test_search_symbol_result_formatting() {
        let symbols = serde_json::from_str::<Vec<Value>>(
            r#"[
                {"name":"read_file","kind":"function","file":"src/fs.rs","line":42,"signature":"pub fn read_file(path: &str) -> Result<String>"},
                {"name":"FileReader","kind":"struct","file":"src/fs.rs","line":100,"signature":"pub struct FileReader"},
                {"name":"write_file","kind":"function","file":"src/fs.rs","line":200}
            ]"#,
        )
        .unwrap();

        let formatted = format_search_results("read", &symbols, 10);
        assert!(formatted.contains("Found 3 symbol(s) matching \"read\""));
        assert!(formatted.contains("1. read_file (function) — src/fs.rs:42"));
        assert!(formatted.contains("pub fn read_file(path: &str) -> Result<String>"));
        assert!(formatted.contains("2. FileReader (struct) — src/fs.rs:100"));
        assert!(formatted.contains("pub struct FileReader"));
        assert!(formatted.contains("3. write_file (function) — src/fs.rs:200"));
    }

    #[test]
    fn test_search_symbol_no_results() {
        let symbols: Vec<Value> = vec![];
        let result = format_search_results("xyz", &symbols, 10);
        // When there are no symbols the executor returns a different message;
        // this function produces a header with "0":
        assert!(result.contains("Found 0 symbol(s) matching \"xyz\""));
    }

    #[test]
    fn test_search_symbol_result_truncation_header() {
        let symbols = vec![serde_json::json!({"name":"a","kind":"fn","file":"a.rs","line":1}); 20];
        let result = format_search_results("trunc", &symbols, 10);
        // When count ≥ max_results, header shows "10+"
        assert!(result.contains("Found 10+ symbol(s) matching \"trunc\""));
    }
}
