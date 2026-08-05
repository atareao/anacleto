//! Formatting helpers for LSP query results.
//!
//! These functions convert raw JSON-RPC responses from a language server into
//! human-readable strings (hover text, locations, diagnostics) and provide
//! small utilities for building URIs and choosing default servers.

use std::collections::HashMap;

use crate::error::Result;

/// Parse a JSON-RPC response, returning the `result` field or an error.
pub(crate) fn parse_lsp_response(response: &serde_json::Value) -> Result<serde_json::Value> {
    if let Some(error) = response.get("error") {
        return Err(crate::error::Error::Lsp(
            error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown LSP error")
                .to_string(),
        ));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| crate::error::Error::Lsp("LSP response missing 'result'".into()))
}

/// Format an LSP query result into a human-readable string.
pub(crate) fn format_lsp_result(kind: &str, result: &serde_json::Value) -> String {
    match kind {
        "hover" => format_hover(result),
        "definition" => format_locations(result),
        "references" => format_locations(result),
        "diagnostic" => format_diagnostics(result),
        _ => serde_json::to_string_pretty(result).unwrap_or_else(|_| "{}".to_string()),
    }
}

/// Format a hover result (may be `null` or `{ contents, range }`).
fn format_hover(result: &serde_json::Value) -> String {
    if result.is_null() {
        return "No hover information available.".to_string();
    }
    let contents = result.get("contents").unwrap_or(result);
    let text = match contents {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|i| match i {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Object(o) => o
                    .get("value")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(o) => o
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    if text.is_empty() {
        "No hover information available.".to_string()
    } else {
        text
    }
}

/// Format a location or list of locations (definition/references).
fn format_locations(result: &serde_json::Value) -> String {
    if result.is_null() {
        return "No locations found.".to_string();
    }
    let locations: Vec<&serde_json::Value> = match result {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(_) => vec![result],
        _ => return serde_json::to_string_pretty(result).unwrap_or_default(),
    };

    if locations.is_empty() {
        return "No locations found.".to_string();
    }

    let mut out = String::new();
    for loc in locations {
        let uri = loc.get("uri").and_then(|u| u.as_str()).unwrap_or("unknown");
        let range = loc.get("range");
        let (start_line, start_char) = range
            .and_then(|r| r.get("start"))
            .map(|s| {
                (
                    s.get("line").and_then(|l| l.as_u64()).unwrap_or(0),
                    s.get("character").and_then(|c| c.as_u64()).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        out.push_str(&format!("{uri}:{}:{}\n", start_line + 1, start_char + 1));
    }
    out
}

/// Format a `textDocument/diagnostic` result.
fn format_diagnostics(result: &serde_json::Value) -> String {
    // LSP 3.17 returns { kind, items: [...] }.
    let items = result
        .get("items")
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        return "No diagnostics.".to_string();
    }

    let mut out = String::new();
    for item in items {
        let severity = item
            .get("severity")
            .and_then(|s| s.as_u64())
            .map(severity_label)
            .unwrap_or("?");
        let message = item.get("message").and_then(|m| m.as_str()).unwrap_or("");
        let (line, character) = item
            .get("range")
            .and_then(|r| r.get("start"))
            .map(|s| {
                (
                    s.get("line").and_then(|l| l.as_u64()).unwrap_or(0),
                    s.get("character").and_then(|c| c.as_u64()).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        out.push_str(&format!(
            "[{severity}] {}:{}: {message}\n",
            line + 1,
            character + 1
        ));
    }
    out
}

/// Map an LSP severity code to a short label.
fn severity_label(code: u64) -> &'static str {
    match code {
        1 => "Error",
        2 => "Warning",
        3 => "Info",
        4 => "Hint",
        _ => "?",
    }
}

/// Convert a filesystem path to an LSP `file://` URI.
pub fn path_to_uri(path: &str) -> String {
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// Build a map of default LSP server commands by file extension.
///
/// Returns a map from extension (e.g. `rs`) to the server command.
pub fn default_server_for_extension(ext: &str) -> Option<&'static str> {
    let map: HashMap<&str, &str> = HashMap::from([
        ("rs", "rust-analyzer"),
        ("ts", "typescript-language-server"),
        ("tsx", "typescript-language-server"),
        ("js", "typescript-language-server"),
        ("jsx", "typescript-language-server"),
        ("py", "pyright-langserver"),
        ("go", "gopls"),
    ]);
    map.get(ext).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lsp_response_result() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "contents": "hello" }
        });
        let result = parse_lsp_response(&response).unwrap();
        assert_eq!(result["contents"], "hello");
    }

    #[test]
    fn test_parse_lsp_response_error() {
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found" }
        });
        let err = parse_lsp_response(&response).unwrap_err();
        assert!(err.to_string().contains("Method not found"));
    }

    #[test]
    fn test_parse_lsp_response_missing_result() {
        let response = serde_json::json!({ "jsonrpc": "2.0", "id": 1 });
        assert!(parse_lsp_response(&response).is_err());
    }

    #[test]
    fn test_format_hover_string() {
        let result = serde_json::json!({ "contents": "fn foo()" });
        assert_eq!(format_hover(&result), "fn foo()");
    }

    #[test]
    fn test_format_hover_null() {
        assert_eq!(
            format_hover(&serde_json::Value::Null),
            "No hover information available."
        );
    }

    #[test]
    fn test_format_hover_array() {
        let result = serde_json::json!({
            "contents": [
                { "language": "rust", "value": "fn foo()" },
                "Some docs"
            ]
        });
        let out = format_hover(&result);
        assert!(out.contains("fn foo()"));
        assert!(out.contains("Some docs"));
    }

    #[test]
    fn test_format_locations_single() {
        let result = serde_json::json!({
            "uri": "file:///tmp/a.rs",
            "range": { "start": { "line": 4, "character": 2 }, "end": { "line": 4, "character": 5 } }
        });
        let out = format_locations(&result);
        assert!(out.contains("file:///tmp/a.rs:5:3"));
    }

    #[test]
    fn test_format_locations_array() {
        let result = serde_json::json!([
            { "uri": "file:///tmp/a.rs", "range": { "start": { "line": 0, "character": 0 } } },
            { "uri": "file:///tmp/b.rs", "range": { "start": { "line": 9, "character": 1 } } }
        ]);
        let out = format_locations(&result);
        assert!(out.contains("file:///tmp/a.rs:1:1"));
        assert!(out.contains("file:///tmp/b.rs:10:2"));
    }

    #[test]
    fn test_format_locations_null() {
        assert_eq!(
            format_locations(&serde_json::Value::Null),
            "No locations found."
        );
    }

    #[test]
    fn test_format_diagnostics() {
        let result = serde_json::json!({
            "kind": "full",
            "items": [
                {
                    "range": { "start": { "line": 2, "character": 0 } },
                    "severity": 1,
                    "message": "unused variable"
                }
            ]
        });
        let out = format_diagnostics(&result);
        assert!(out.contains("[Error] 3:1: unused variable"));
    }

    #[test]
    fn test_format_diagnostics_empty() {
        let result = serde_json::json!({ "kind": "full", "items": [] });
        assert_eq!(format_diagnostics(&result), "No diagnostics.");
    }

    #[test]
    fn test_path_to_uri() {
        assert_eq!(path_to_uri("/tmp/a.rs"), "file:///tmp/a.rs");
    }

    #[test]
    fn test_default_server_for_extension() {
        assert_eq!(default_server_for_extension("rs"), Some("rust-analyzer"));
        assert_eq!(
            default_server_for_extension("ts"),
            Some("typescript-language-server")
        );
        assert_eq!(default_server_for_extension("xyz"), None);
    }
}
