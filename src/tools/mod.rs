//! Structured tools for the agent: `read`, `write`, `insert`, `replace`,
//! `delete`, `list`, `grep`, `glob`, `webfetch`, `websearch`, MCP resource
//! tools, `lsp_query`, `search_symbol`, `execute`, and `format_document`.
//!
//! Each tool exposes a strict JSON schema via a `xxx_tool_definition()`
//! function and an executor `execute_xxx_tool(...)` that returns
//! `Result<String, String>`.

pub mod execute;
pub mod format;
pub mod lsp;
pub mod mcp;
pub mod pattern;
pub mod search_symbol;
pub mod web;

pub mod delete;
pub mod glob;
pub mod grep;
pub mod insert;
pub mod list;
pub mod read;
pub mod replace;
pub mod write;

pub use execute::{execute_execute_tool, execute_tool_definition};
pub use format::{execute_format_document_tool, format_document_tool_definition};
pub use lsp::{execute_lsp_query_tool, lsp_query_tool_definition};
pub use mcp::{
    execute_mcp_list_resource_templates_tool, execute_mcp_list_resources_tool,
    execute_mcp_read_resource_tool, mcp_list_resource_templates_tool_definition,
    mcp_list_resources_tool_definition, mcp_read_resource_tool_definition,
};
pub use search_symbol::{execute_search_symbol_tool, search_symbol_tool_definition};
pub use web::{
    execute_webfetch_tool, execute_websearch_tool, webfetch_tool_definition,
    websearch_tool_definition,
};

pub use delete::{delete_tool_definition, execute_delete_tool};
pub use glob::{execute_glob_tool, glob_tool_definition};
pub use grep::{execute_grep_tool, grep_tool_definition};
pub use insert::{execute_insert_tool, insert_tool_definition};
pub use list::{execute_list_tool, list_tool_definition};
pub use read::{execute_read_tool, read_tool_definition};
pub use replace::{execute_replace_tool, replace_tool_definition};
pub use write::{execute_write_tool, write_tool_definition};
