//! Web tools: `webfetch` (HTTP GET a URL) and `websearch` (stub).
//!
//! Both require the `net.http` permission.

use crate::llm::types::{ToolCall, ToolDefinition};
use crate::permissions::checker::check_net_http;
use crate::permissions::types::Permissions;

/// Maximum number of bytes of body text returned by `webfetch`.
const MAX_BODY_BYTES: usize = 10_000;

/// Tool definition for the `webfetch` tool.
pub fn webfetch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "webfetch".to_string(),
        description: "Fetch the content of a URL over HTTP(S) and return it as \
                       text. The response body is truncated to 10000 characters."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must start with http:// or https://)."
                }
            },
            "required": ["url"]
        }),
    }
}

/// Tool definition for the `websearch` tool (stub).
pub fn websearch_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "websearch".to_string(),
        description: "Search the web for a query. NOTE: no search backend is \
                       configured, so this tool currently returns a 'not \
                       configured' message."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                }
            },
            "required": ["query"]
        }),
    }
}

/// Execute a `webfetch` tool call.
pub async fn execute_webfetch_tool(
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    check_net_http(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse webfetch arguments: {e}"))?;
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "webfetch requires 'url'".to_string())?;

    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(format!(
            "Invalid URL '{url}': must start with http:// or https://"
        ));
    }

    let response = tokio::time::timeout(std::time::Duration::from_secs(30), reqwest::get(url))
        .await
        .map_err(|_| format!("Request timed out after 30 seconds fetching: {url}"))?
        .map_err(|e| format!("HTTP request failed for {url}: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} fetching {url}", status.as_u16()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let mut result = format!("Content from {url}:\n\n");
    if body.len() > MAX_BODY_BYTES {
        result.push_str(&body[..MAX_BODY_BYTES]);
        result.push_str(&format!(
            "\n\n... (truncated at {MAX_BODY_BYTES} characters)"
        ));
    } else {
        result.push_str(&body);
    }

    Ok(result)
}

/// Execute a `websearch` tool call (stub).
pub async fn execute_websearch_tool(
    permissions: &Permissions,
    tool_call: &ToolCall,
) -> Result<String, String> {
    check_net_http(permissions).map_err(|e| format!("Permission denied: {e}"))?;

    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse websearch arguments: {e}"))?;
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

    Ok(format!(
        "Web search is not configured. No search backend is available, so the \
         query '{query}' could not be executed. Use `webfetch` to fetch a \
         specific URL instead."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionConfig;
    use crate::llm::types::ToolFunction;
    use crate::permissions::types::Permissions;

    fn tool_call(name: &str, args: &str) -> ToolCall {
        ToolCall {
            id: "call_web".into(),
            call_type: "function".into(),
            function: ToolFunction {
                name: name.into(),
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

    fn deny_http() -> Permissions {
        Permissions::from_config(&PermissionConfig {
            deny: vec!["net.http".into()],
            allow: vec![],
        })
    }

    #[tokio::test]
    async fn websearch_denied_without_net_http() {
        let result =
            execute_websearch_tool(&deny_http(), &tool_call("websearch", r#"{"query":"rust"}"#))
                .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));
    }

    #[tokio::test]
    async fn websearch_stub_returns_not_configured() {
        let result =
            execute_websearch_tool(&allow_all(), &tool_call("websearch", r#"{"query":"rust"}"#))
                .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("not configured"));
    }

    #[tokio::test]
    async fn webfetch_denied_without_net_http() {
        let result = execute_webfetch_tool(
            &deny_http(),
            &tool_call("webfetch", r#"{"url":"https://example.com"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));
    }

    #[tokio::test]
    async fn webfetch_rejects_invalid_url() {
        let result = execute_webfetch_tool(
            &allow_all(),
            &tool_call("webfetch", r#"{"url":"not-a-url"}"#),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with http"));
    }
}
