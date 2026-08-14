# ADR-004: Line-based file editing tools

## Status

Accepted

## Context

The current file editing options for the LLM are:

1. **`apply_patch`** — Requires the LLM to reproduce the **exact** text to replace (`old` → `new`). Fails on whitespace mismatches, tab/space differences, or hallucinated context lines.
2. **`filesystem` with `write`** — Rewrites the entire file. Inefficient, high token cost, and risks data loss if the LLM truncates content.
3. **`shell` with `sed`** — Fragile regex-based editing, error-prone, and shell-dependent.

None of these allow the LLM to say: *"In file X, after line 42, insert this content."* Line-number-based editing is deterministic, low-token, and eliminates the fragility of text-matching.

## Decision

Create three new builtin tools for line-based file editing:

### `insert_lines`

- **Input schema:**
  - `path` (string, required) — File to edit
  - `after_line` (integer, required) — Line number after which to insert (1-based)
  - `content` (string, required) — Content to insert
- **Behavior:** Reads the file, splits by lines, inserts `content` after line `after_line`, writes back.
- **Edge cases:**
  - `after_line = 0` → insert at beginning of file
  - `after_line >= total_lines` → append to end of file
  - Empty content → no-op (success)
  - File doesn't exist → error

### `replace_lines`

- **Input schema:**
  - `path` (string, required) — File to edit
  - `start_line` (integer, required) — First line to replace (1-based)
  - `end_line` (integer, required) — Last line to replace (inclusive, 1-based)
  - `content` (string, required) — Replacement content
- **Behavior:** Reads the file, replaces lines `start_line..=end_line` with `content`, writes back.
- **Edge cases:**
  - `start_line > end_line` → error
  - `start_line` or `end_line` out of range → error
  - Empty content → deletes the range (same as `delete_lines`)

### `delete_lines`

- **Input schema:**
  - `path` (string, required) — File to edit
  - `start_line` (integer, required) — First line to delete (1-based)
  - `end_line` (integer, required) — Last line to delete (inclusive, 1-based)
- **Behavior:** Reads the file, removes lines `start_line..=end_line`, writes back.
- **Edge cases:**
  - `start_line > end_line` → error
  - Range out of bounds → error
  - Deleting all lines → empty file (not an error)

### Common design

- All three tools require `fs.write` permission.
- All three validate paths against workspace traversal (same as `read` tool).
- All three use the same path resolution logic as `read`/`grep`/`glob`.
- Line numbers are **1-based** (consistent with the `read` tool's display).
- All three are registered in `builtin_tool_definitions()` and dispatched in the tool dispatch chain in `lifecycle.rs`.
- Display templates and colors are configurable via `config.yaml` `tools:` section.

## Consequences

### Positive

- **Deterministic editing:** Line numbers are unambiguous. No text-matching fragility.
- **Low token cost:** The LLM only sends the content to insert/replace, not the entire file.
- **Composability:** The LLM can read a file with `read`, identify line numbers, then edit with `insert_lines`/`replace_lines`/`delete_lines` in a single turn.
- **Backward compatible:** Existing tools (`apply_patch`, `filesystem`) remain unchanged.

### Negative

- **Line number drift:** If the file changes between `read` and edit, line numbers may be stale. Mitigation: the LLM should re-read before editing if the file may have changed.
- **No merge conflict detection:** Unlike `apply_patch` which fails on text mismatch, line-based editing silently applies even if the file has changed. Mitigation: this is acceptable for an AI coding assistant where the LLM is the sole editor.

## Implementation

Each tool follows the existing pattern in `src/tools/`:
- `src/tools/edit.rs` — Module with `insert_lines_tool_definition()`, `replace_lines_tool_definition()`, `delete_lines_tool_definition()`, and their executor functions.
- Registration in `builtin_tool_definitions()` in `src/agent/lifecycle.rs`.
- Dispatch branches in the tool execution loop in `src/agent/lifecycle.rs`.
- Tests for each tool covering: basic operation, edge cases, path traversal protection, permission checks.