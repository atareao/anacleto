//! Minimal Language Server Protocol (LSP) client over stdio.
//!
//! This module lets the agent query a language server (e.g. `rust-analyzer`,
//! `typescript-language-server`) for hover, definition, references and
//! diagnostics. It speaks JSON-RPC 2.0 with the standard LSP
//! `Content-Length` framing over the server's stdin/stdout.
//!
//! The client is deliberately minimal and fault-tolerant: if the server
//! cannot be spawned or does not respond, a clear error is returned rather
//! than panicking.

mod format;

pub use format::{default_server_for_extension, path_to_uri};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::error::{Error, Result};

/// A minimal LSP client bound to a single language server process.
pub struct LspClient {
    /// Child process handle.
    child: Option<Child>,
    /// Stdin for sending requests.
    stdin: Option<ChildStdin>,
    /// Buffered stdout for reading responses.
    stdout: Option<tokio::io::BufReader<ChildStdout>>,
    /// Next request ID.
    next_id: u64,
    /// Whether the server has been initialized.
    initialized: bool,
}

/// A position within a text document (0-based line and character).
#[derive(Debug, Clone, Copy)]
pub struct LspPosition {
    /// 0-based line number.
    pub line: u32,
    /// 0-based character offset.
    pub character: u32,
}

/// The kind of LSP query to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LspQueryType {
    /// `textDocument/hover`
    Hover,
    /// `textDocument/definition`
    Definition,
    /// `textDocument/references`
    References,
    /// `textDocument/diagnostic`
    Diagnostic,
}

impl LspQueryType {
    /// Parse a query type from its string name.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hover" => Some(LspQueryType::Hover),
            "definition" | "def" | "goto_definition" => Some(LspQueryType::Definition),
            "references" | "refs" => Some(LspQueryType::References),
            "diagnostic" | "diagnostics" => Some(LspQueryType::Diagnostic),
            _ => None,
        }
    }
}

impl LspClient {
    /// Create a new LSP client (does not spawn the process yet).
    pub fn new() -> Self {
        Self {
            child: None,
            stdin: None,
            stdout: None,
            next_id: 1,
            initialized: false,
        }
    }

    /// Spawn the language server process and perform the `initialize` handshake.
    ///
    /// `server_command` is the executable (e.g. `rust-analyzer`) and `args`
    /// any additional arguments. Returns a clear error if the process cannot
    /// be spawned or the handshake fails.
    pub async fn start(
        &mut self,
        server_command: &str,
        args: &[String],
        root_uri: &str,
    ) -> Result<()> {
        let mut child = Command::new(server_command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                Error::Lsp(format!(
                    "Failed to spawn LSP server '{}': {}. Is it installed?",
                    server_command, e
                ))
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Lsp("Failed to capture LSP stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Lsp("Failed to capture LSP stdout".into()))?;

        self.stdin = Some(stdin);
        self.stdout = Some(tokio::io::BufReader::new(stdout));
        self.child = Some(child);

        self.initialize(root_uri).await?;
        Ok(())
    }

    /// Send the `initialize` request and mark the client as initialized.
    async fn initialize(&mut self, root_uri: &str) -> Result<()> {
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {},
        });
        let _result = self.request("initialize", params).await?;

        // Notify the server that initialization is complete.
        self.notify("initialized", serde_json::json!({})).await?;
        self.initialized = true;
        Ok(())
    }

    /// Send a request and await its response.
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&request).await?;
        let response = self.read_message().await?;
        format::parse_lsp_response(&response)
    }

    /// Send a notification (no response expected).
    async fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&notification).await
    }

    /// Perform a hover query at the given position.
    pub async fn hover(&mut self, uri: &str, position: LspPosition) -> Result<String> {
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
        });
        let result = self.request("textDocument/hover", params).await?;
        Ok(format::format_lsp_result("hover", &result))
    }

    /// Perform a go-to-definition query at the given position.
    pub async fn definition(&mut self, uri: &str, position: LspPosition) -> Result<String> {
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
        });
        let result = self.request("textDocument/definition", params).await?;
        Ok(format::format_lsp_result("definition", &result))
    }

    /// Perform a find-references query at the given position.
    pub async fn references(&mut self, uri: &str, position: LspPosition) -> Result<String> {
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": position.line, "character": position.character },
            "context": { "includeDeclaration": true },
        });
        let result = self.request("textDocument/references", params).await?;
        Ok(format::format_lsp_result("references", &result))
    }

    /// Request diagnostics for a document (LSP 3.17 `textDocument/diagnostic`).
    pub async fn diagnostic(&mut self, uri: &str) -> Result<String> {
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
        });
        let result = self.request("textDocument/diagnostic", params).await?;
        Ok(format::format_lsp_result("diagnostic", &result))
    }

    /// Run a single query against a freshly spawned server and tear it down.
    ///
    /// This is a convenience for the `lsp_query` tool: it spawns the server,
    /// runs the requested query, and kills the process.
    pub async fn query_once(
        server_command: &str,
        args: &[String],
        root_uri: &str,
        uri: &str,
        position: LspPosition,
        query_type: LspQueryType,
    ) -> Result<String> {
        let mut client = LspClient::new();
        client.start(server_command, args, root_uri).await?;
        let result = match query_type {
            LspQueryType::Hover => client.hover(uri, position).await,
            LspQueryType::Definition => client.definition(uri, position).await,
            LspQueryType::References => client.references(uri, position).await,
            LspQueryType::Diagnostic => client.diagnostic(uri).await,
        };
        client.shutdown().await.ok();
        result
    }

    /// Write a JSON-RPC message using LSP `Content-Length` framing.
    async fn write_message(&mut self, message: &serde_json::Value) -> Result<()> {
        let body = serde_json::to_string(message)
            .map_err(|e| Error::Lsp(format!("Failed to serialize LSP message: {e}")))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Lsp("LSP stdin not available".into()))?;
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| Error::Lsp(format!("Failed to write LSP header: {e}")))?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| Error::Lsp(format!("Failed to write LSP body: {e}")))?;
        stdin
            .flush()
            .await
            .map_err(|e| Error::Lsp(format!("Failed to flush LSP stdin: {e}")))
    }

    /// Read a JSON-RPC message using LSP `Content-Length` framing.
    async fn read_message(&mut self) -> Result<serde_json::Value> {
        let reader = self
            .stdout
            .as_mut()
            .ok_or_else(|| Error::Lsp("LSP stdout not available".into()))?;

        // Read headers until an empty line.
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| Error::Lsp(format!("Failed to read LSP header: {e}")))?;
            if n == 0 {
                return Err(Error::Lsp(
                    "LSP server closed stdout before sending a response".into(),
                ));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(value) = trimmed.strip_prefix("Content-Length:") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }

        let length = content_length
            .ok_or_else(|| Error::Lsp("LSP response missing Content-Length header".into()))?;

        let mut body = vec![0u8; length];
        reader
            .read_exact(&mut body)
            .await
            .map_err(|e| Error::Lsp(format!("Failed to read LSP body: {e}")))?;

        serde_json::from_slice(&body)
            .map_err(|e| Error::Lsp(format!("Failed to parse LSP response: {e}")))
    }

    /// Shut down the language server process.
    pub async fn shutdown(&mut self) -> Result<()> {
        if self.initialized {
            let _ = self.request("shutdown", serde_json::json!({})).await;
            let _ = self.notify("exit", serde_json::json!({})).await;
        }
        if let Some(mut child) = self.child.take() {
            child.kill().await.ok();
            child.wait().await.ok();
        }
        self.stdin = None;
        self.stdout = None;
        self.initialized = false;
        Ok(())
    }
}

impl Default for LspClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_type_from_str() {
        assert_eq!(LspQueryType::parse("hover"), Some(LspQueryType::Hover));
        assert_eq!(
            LspQueryType::parse("definition"),
            Some(LspQueryType::Definition)
        );
        assert_eq!(
            LspQueryType::parse("references"),
            Some(LspQueryType::References)
        );
        assert_eq!(
            LspQueryType::parse("diagnostic"),
            Some(LspQueryType::Diagnostic)
        );
        assert_eq!(LspQueryType::parse("bogus"), None);
    }
}
