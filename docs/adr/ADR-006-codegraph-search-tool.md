# ADR-006: CodeGraph-based symbol search tool

## Status

Accepted

## Context

When the LLM needs to find where a symbol is defined or used, the current options are:

1. **`grep`** — Text-based search. Returns every occurrence, including comments and strings. No semantic understanding. Slow on large codebases.
2. **`lsp_query` with `definition`/`references`** — Requires spawning an LSP server and knowing the exact file + line + character of the symbol. Circular dependency: to find where a symbol is, you need to know where it is.
3. **`glob`** — Only finds files by name pattern, not symbol definitions.

The project already has CodeGraph, a tree-sitter-parsed knowledge graph of every symbol, edge, and file in the workspace. CodeGraph runs as an MCP server and provides structured queries: search by name, find callers/callees, get symbol signatures, etc.

However, the LLM currently has no direct builtin tool to query CodeGraph. It must use the generic `mcp_read_resource` or `mcp_list_resources` tools, which are low-level and require the LLM to know the exact resource URI format.

## Decision

Create a new builtin tool `search_symbol` that queries the CodeGraph MCP server for symbol information.

### `search_symbol`

- **Input schema:**
  - `query` (string, required) — Symbol name or partial name to search for
  - `kind` (string, optional) — Filter by symbol kind: `function`, `method`, `struct`, `enum`, `trait`, `type`, `variable`, `interface`, `component`, `route`
  - `path` (string, optional) — Scope search to a specific file or directory
  - `max_results` (integer, optional, default: 10, max: 50) — Maximum results to return
- **Behavior:**
  1. Connect to the CodeGraph MCP server (via the existing MCP client infrastructure)
  2. Call the `codegraph_search` tool with the provided parameters
  3. Format the results as structured text: symbol name, kind, file location, and signature
  4. Return the formatted results
- **Permission:** Requires `mcp.use` (to call the CodeGraph MCP server).
- **Edge cases:**
  - No results → "No symbols found matching '{query}'"
  - CodeGraph not available → clear error suggesting fallback to `grep`
  - Invalid kind filter → error listing valid kinds

### Result format

```
Found 3 symbols matching "auth":

1. AuthService (struct) — src/auth/service.rs:12
   fn new(config: Config) -> Self
   fn authenticate(&self, credentials: Credentials) -> Result<User>

2. authenticate (method) — src/auth/service.rs:45
   pub async fn authenticate(&self, credentials: Credentials) -> Result<User>

3. AuthMiddleware (struct) — src/auth/middleware.rs:8
   fn from_fn(f: impl FnOnce) -> Self
```

## Consequences

### Positive

- **Semantic search:** Finds symbol definitions, not text matches. No false positives from comments or strings.
- **Structured results:** Returns signatures, docstrings, and file locations in a parseable format.
- **Fast:** CodeGraph queries are sub-millisecond on the indexed graph.
- **Complements grep:** `search_symbol` for definitions, `grep` for text occurrences.

### Negative

- **Requires CodeGraph:** The tool only works if CodeGraph is running as an MCP server. Mitigation: clear error message with fallback to `grep`.
- **Index lag:** CodeGraph's index lags file writes by ~1 second. Newly created symbols may not appear immediately. Mitigation: document this limitation.
- **MCP dependency:** Adds a dependency on the MCP infrastructure. Mitigation: the existing MCP client is already robust with timeout and error handling.

## Implementation

- `src/tools/search_symbol.rs` — Module with `search_symbol_tool_definition()` and `execute_search_symbol_tool()`.
- Registration in `builtin_tool_definitions()` in `src/agent/lifecycle.rs`.
- Dispatch branch in the tool execution loop.
- Tests: unit tests for result formatting, integration tests with a mock MCP server.