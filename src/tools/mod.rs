//! Structured tools for the agent: `fs`, `search`, `webfetch`,
//! `websearch`, MCP resource tools and the `lsp_query` tool.
//!
//! Each tool exposes a strict JSON schema via a `xxx_tool_definition()`
//! function and an executor `execute_xxx_tool(...)` that returns
//! `Result<String, String>`.

pub mod format;
pub mod fs;
pub mod lsp;
pub mod mcp;
pub mod pattern;
pub mod search;
pub mod search_symbol;
pub mod web;

pub use format::{execute_format_document_tool, format_document_tool_definition};
pub use fs::{execute_fs_tool, fs_tool_definition};
pub use lsp::{execute_lsp_query_tool, lsp_query_tool_definition};
pub use mcp::{
    execute_mcp_list_resource_templates_tool, execute_mcp_list_resources_tool,
    execute_mcp_read_resource_tool, mcp_list_resource_templates_tool_definition,
    mcp_list_resources_tool_definition, mcp_read_resource_tool_definition,
};
pub use search::{execute_search_tool, search_tool_definition};
pub use search_symbol::{execute_search_symbol_tool, search_symbol_tool_definition};
pub use web::{
    execute_webfetch_tool, execute_websearch_tool, webfetch_tool_definition,
    websearch_tool_definition,
};
