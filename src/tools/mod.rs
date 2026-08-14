//! Structured tools for the agent: `read`, `grep`, `glob`, `webfetch`,
//! `websearch`, MCP resource tools and the `lsp_query` tool.
//!
//! Each tool exposes a strict JSON schema via a `xxx_tool_definition()`
//! function and an executor `execute_xxx_tool(...)` that returns
//! `Result<String, String>`.

pub mod edit;
pub mod format;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp;
pub mod pattern;
pub mod read;
pub mod search_symbol;
pub mod web;

pub use edit::{
    delete_lines_tool_definition, execute_delete_lines_tool, execute_insert_lines_tool,
    execute_replace_lines_tool, insert_lines_tool_definition, replace_lines_tool_definition,
};
pub use format::{execute_format_document_tool, format_document_tool_definition};
pub use glob::{execute_glob_tool, glob_tool_definition};
pub use grep::{execute_grep_tool, grep_tool_definition};
pub use lsp::{execute_lsp_query_tool, lsp_query_tool_definition};
pub use mcp::{
    execute_mcp_list_resource_templates_tool, execute_mcp_list_resources_tool,
    execute_mcp_read_resource_tool, mcp_list_resource_templates_tool_definition,
    mcp_list_resources_tool_definition, mcp_read_resource_tool_definition,
};
pub use read::{execute_read_tool, read_tool_definition};
pub use search_symbol::{execute_search_symbol_tool, search_symbol_tool_definition};
pub use web::{
    execute_webfetch_tool, execute_websearch_tool, webfetch_tool_definition,
    websearch_tool_definition,
};
