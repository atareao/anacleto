# ADR-005: LSP-based document formatting tool

## Status

Accepted

## Context

When the LLM writes or edits code, the result is often unformatted (missing trailing newlines, inconsistent indentation, etc.). Currently, the LLM must:

1. Finish editing
2. Remember to run a formatter via `shell` (e.g., `cargo fmt`, `prettier`, `ruff`)
3. Parse the output to verify success

This is an easily forgotten step that leads to code that fails CI formatting checks. The project already has a working LSP client in `src/lsp/` that can spawn language servers and communicate via JSON-RPC 2.0. The LSP protocol includes `textDocument/formatting` which returns formatted text.

## Decision

Create a new builtin tool `format_document` that formats a file using the appropriate LSP server.

### `format_document`

- **Input schema:**
  - `path` (string, required) — File to format
- **Behavior:**
  1. Detect the language server from the file extension (reuses `default_server_for_extension()` from `src/lsp/format.rs`)
  2. Spawn the LSP server
  3. Read the current file content
  4. Send `textDocument/formatting` with `{ tabSize: 4, insertSpaces: true }`
  5. Apply the returned `TextEdit[]` to the file
  6. Shut down the LSP server
  7. Return success message with summary of changes
- **Permission:** Requires `command.run` (to spawn the LSP server) and `fs.write` (to modify the file).
- **Edge cases:**
  - Unknown file extension → clear error message suggesting manual formatting
  - LSP server not installed → clear error with installation instructions
  - File already formatted → no-op, success
  - LSP returns no edits → file is already formatted

### Server detection

Reuses the existing mapping from `src/lsp/format.rs`:

| Extension | Server |
|---|---|
| `.rs` | `rust-analyzer` |
| `.ts`, `.tsx`, `.js`, `.jsx` | `typescript-language-server` |
| `.py` | `pyright-langserver` |
| `.go` | `gopls` |

If no server is known for the extension, the tool returns a clear error suggesting the user configure a server explicitly.

## Consequences

### Positive

- **Automatic formatting:** The LLM can format code immediately after writing it, in the same turn.
- **Consistent results:** Uses the project's language server, respecting `.rustfmt.toml`, `tsconfig.json`, etc.
- **Low complexity:** Reuses existing LSP infrastructure. The `textDocument/formatting` request is standard LSP.
- **No shell dependency:** Doesn't require `cargo fmt`, `prettier`, etc. to be in PATH separately from the LSP server.

### Negative

- **LSP server startup cost:** Each call spawns a new LSP server process (~200-500ms for rust-analyzer). Mitigation: acceptable for an AI coding assistant; formatting is not a hot-path operation.
- **Limited to supported extensions:** Files with unknown extensions cannot be formatted. Mitigation: the LLM can fall back to `shell` with the appropriate formatter.

## Implementation

- `src/tools/format.rs` — Module with `format_document_tool_definition()` and `execute_format_document_tool()`.
- Registration in `builtin_tool_definitions()` in `src/agent/lifecycle.rs`.
- Dispatch branch in the tool execution loop.
- Tests: mock LSP server for integration tests, unit tests for extension detection.