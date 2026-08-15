# Plan: Convert filesystem skill to built-in fs/search tools

## Problem
The `filesystem` skill causes ~30+ JSON parse errors per session because the LLM must construct nested JSON (JSON inside a `task` string field). The LLM consistently fails to escape special characters (newlines, quotes) in content strings.

## Solution
Replace the filesystem skill (loaded from SKILL.md) and several built-in tools with two new unified tools that have proper `input_schema` fields:

1. **`fs`** — replaces `read`, `insert_lines`, `replace_lines`, `delete_lines`, `apply_patch`, and the `filesystem` skill
2. **`search`** — replaces `grep` and `glob`

## Files to create
- `src/tools/fs.rs` — unified filesystem tool
- `src/tools/search.rs` — unified search tool

## Files to modify
- `src/tools/mod.rs` — export new tools, remove old exports
- `src/agent/lifecycle.rs` — register new tools, remove old registrations
- `src/agent/tools.rs` — remove filesystem skill handling
- `src/skill/executor.rs` — remove filesystem skill handling
- `src/lib.rs` — remove filesystem module if no longer needed
- `/home/lorenzo/.config/anacleto/config.yaml` — add fs/search tools, remove old ones
- `/home/lorenzo/.config/anacleto/agents/*.md` — replace filesystem skill with fs tool

## Files to delete
- `/home/lorenzo/.config/anacleto/skills/filesystem/SKILL.md`

## Tareas

### Tarea 1: Create `src/tools/fs.rs`

**Archivos:**
- Crear: `src/tools/fs.rs`

- [ ] **Paso 1:** Define `fs_tool_definition()` returning a `ToolDefinition` with `input_schema` containing:
      `op` (string enum: read, write, insert, replace, delete, list), `path` (string, required), `content` (string, optional), `old` (string, optional), `new` (string, optional), `after_line` (integer, optional), `start_line` (integer, optional), `end_line` (integer, optional), `offset` (integer, optional), `limit` (integer, optional)

- [ ] **Paso 2:** Define `execute_fs_tool()` that dispatches to internal handlers per `op`

- [ ] **Paso 3:** Include all functionality from `read.rs`, `edit.rs`, and `filesystem/mod.rs`

- [ ] **Paso 4:** Include comprehensive tests for all ops

### Tarea 2: Create `src/tools/search.rs`

**Archivos:**
- Crear: `src/tools/search.rs`

- [ ] **Paso 1:** Define `search_tool_definition()` with `input_schema` containing:
      `mode` (string enum: content, files), `pattern` (string, required), `path` (string, optional), `include` (string, optional), `max_results` (integer, optional), `context_lines` (integer, optional), `case_sensitive` (boolean, optional)

- [ ] **Paso 2:** Define `execute_search_tool()` that dispatches to grep or glob logic

- [ ] **Paso 3:** Include comprehensive tests

### Tarea 3: Update `src/tools/mod.rs`

**Archivos:**
- Modificar: `src/tools/mod.rs`

- [ ] **Paso 1:** Add `pub mod fs;` and `pub mod search;`

- [ ] **Paso 2:** Add re-exports for the new tool definitions and executors

- [ ] **Paso 3:** Remove re-exports for `read`, `edit`, `grep`, `glob` (they stay as modules but are no longer re-exported as tools)

### Tarea 4: Update `src/agent/lifecycle.rs`

**Archivos:**
- Modificar: `src/agent/lifecycle.rs`

- [ ] **Paso 1:** Add imports for `fs_tool_definition`, `execute_fs_tool`, `search_tool_definition`, `execute_search_tool`

- [ ] **Paso 2:** Add them to `builtin_tool_definitions()`

- [ ] **Paso 3:** Remove `read_tool_definition`, `grep_tool_definition`, `glob_tool_definition`, `insert_lines_tool_definition`, `replace_lines_tool_definition`, `delete_lines_tool_definition` from `builtin_tool_definitions()`

- [ ] **Paso 4:** Add tool execution dispatch for `fs` and `search` in the tool execution loop

- [ ] **Paso 5:** Remove dispatch for `read`, `grep`, `glob`, `insert_lines`, `replace_lines`, `delete_lines`

### Tarea 5: Update `src/agent/tools.rs`

**Archivos:**
- Modificar: `src/agent/tools.rs`

- [ ] **Paso 1:** Remove `FILESYSTEM_TASK_DOC` constant

- [ ] **Paso 2:** Remove filesystem-specific handling in `skill_to_tool_definition()`

- [ ] **Paso 3:** Remove `execute_filesystem_operation()` function

- [ ] **Paso 4:** Remove filesystem handling in `plan_mode_blocked()` and `classify_tool_operation()`

### Tarea 6: Update `src/skill/executor.rs`

**Archivos:**
- Modificar: `src/skill/executor.rs`

- [ ] **Paso 1:** Remove filesystem skill handling (the `else if skill_name_lower == "filesystem"` branch)

- [ ] **Paso 2:** Remove `execute_filesystem_operation()` function

### Tarea 7: Update config.yaml

**Archivos:**
- Modificar: `/home/lorenzo/.config/anacleto/config.yaml`

- [ ] **Paso 1:** Add `fs` and `search` tool entries with descriptions, colors, display templates

- [ ] **Paso 2:** Remove `read`, `grep`, `glob`, `insert_lines`, `replace_lines`, `delete_lines` entries

### Tarea 8: Update agent .md files

**Archivos:**
- Modificar: `/home/lorenzo/.config/anacleto/agents/*.md`

- [ ] **Paso 1:** In all agent files that reference `filesystem` skill, replace the skill path with the `fs` tool

- [ ] **Paso 2:** Add `fs` and `search` to the tools list where appropriate

### Tarea 9: Remove filesystem skill SKILL.md

**Archivos:**
- Eliminar: `/home/lorenzo/.config/anacleto/skills/filesystem/SKILL.md`

- [ ] **Paso 1:** Delete the file

### Tarea 10: Build and test

- [ ] **Paso 1:** Run `cargo build` and fix any compilation errors

- [ ] **Paso 2:** Run `cargo test` and fix any failing tests

- [ ] **Paso 3:** Run `cargo clippy` and fix any warnings