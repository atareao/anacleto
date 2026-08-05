use crate::error::{Error, Result};

use super::types::{McpResource, McpResourceTemplate};

/// Parse a `resources/list` JSON-RPC response into a list of resources.
pub(crate) fn parse_resources_response(response: &serde_json::Value) -> Result<Vec<McpResource>> {
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
pub(crate) fn parse_resource_templates_response(
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
pub(crate) fn parse_read_resource_response(response: &serde_json::Value) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
