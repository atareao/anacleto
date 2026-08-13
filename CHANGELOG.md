# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.27.0] - 2026-08-13

### Fixed

- **Word-wrap at word boundaries** — `prewrap_line` now splits long lines at spaces instead of span boundaries, preventing orphaned punctuation (e.g. `,` at line start). Added `split_at_word_boundary` and `push_span_across_lines` helpers.
- **Table column width clamping** — `render_table_block` now accepts `content_width` and shrinks columns proportionally when the table exceeds the available width, with ellipsis (`…`) for truncated cells.

## [0.26.0] - 2026-08-12

### Added

- **Subagent `when_to_use` field** — agents can now specify `when_to_use` descriptions for better subagent selection.
- **Tool execution events** — tool calls and results are now emitted as structured events in the TUI.

### Changed

- **TUI performance** — optimized rendering pipeline for large conversations.
- **Context improvements** — better context window management for long agent sessions.

## [0.25.0] - 2026-08-12

### Fixed

- **Table rendering order** — `render_sectioned_block` now flushes `normal_buffer` before entering table mode, so headings (`## Tabla de Subagentes`) and content before a table render in the correct order instead of appearing after the table.

### Changed

- **`flush_section` refactored**: removed shared-border logic between consecutive same-type sections; each section now always gets its own top border. Leading/trailing blank lines in section buffers are automatically trimmed.
- **Agent panel**: aligned agent names with emoji-based status indicators (`🧠`/`🔧`, `⬆️`/`⬇️`) and prefix-width alignment.

## [0.24.0] - 2026-08-12

### Added

- **Collapsible sections** — Chat messages are organized into visual sections (`[thinking]`, `[tool]`, `[normal]`, `[user]`, `[command]`) with distinctive `▐` borders. Sections can be collapsed/expanded by clicking on any line inside them.
- **Syntax highlighting** — Fenced code blocks (```` ```lang ````) are rendered with syntax highlighting via `syntect`, supporting dark and light themes.
- **Click-to-copy code blocks** — Each code block appends a `[copy]` indicator; clicking it copies the block content to the clipboard with toast notification.
- **Section markers** — Messages are automatically wrapped in `[normal]`/`[/normal]`, `[user]`/`[/user]`, and `[command]`/`[/command]` markers for proper section rendering. Tool execution is batched into single `[tool]` blocks.
- **Pre-wrapped lines** — Soft-wrapped continuations keep the left border (`▐`) for visual continuity.

### Changed

- `src/tui/code_block.rs` (new): `CodeBlockHighlighter` with syntect, `CodeBlockPosition` for click tracking.
- `src/tui/render.rs`: Section-based rendering with `flush_section()`, `apply_collapsed()`, `generate_section_id()`, syntect integration.
- `src/tui/events.rs`: Tool execution display distinguishes executable tools (⚡) from passive skills (📖).
- `src/tui/keys.rs`: `handle_section_click()` and `handle_code_block_click()` for mouse interaction.
- `src/tui/keymap.rs`: New actions `ToggleCodeBlock` (Ctrl+E) and `CopyCodeBlock` (Ctrl+Shift+C).
- `src/tui/app.rs`: `collapsed_sections`, `section_line_map`, `section_info`, `pending_tool_lines`, `code_block_hl`.
- `src/tui/types.rs`: `CollapsedSection` struct.
- `src/tui/theme.rs`: New section styles (`user_border`, `user_text`, `command_border`, `command_text`).
- `src/tui/markdown.rs`: `visual_line_count()` helper, syntect-aware markdown rendering.
- `src/tui/navigation.rs`, `diff_viewer.rs`, `commands.rs`: Updated section rendering.

[0.24.0]: https://github.com/atareao/anacleto/releases/tag/v0.24.0
[0.25.0]: https://github.com/atareao/anacleto/releases/tag/v0.25.0
[0.26.0]: https://github.com/atareao/anacleto/releases/tag/v0.26.0
[0.27.0]: https://github.com/atareao/anacleto/releases/tag/v0.27.0

## [0.22.0] - 2026-08-11

### Added

- **Dependency updates** — crossterm 0.29, sqlx 0.9, reqwest 0.13, rand 0.10.
- **MSRV bump** — Minimum supported Rust version raised to 1.97.

### Changed

- Adapted to rand 0.10 and sqlx 0.9 API changes.

[0.22.0]: https://github.com/atareao/anacleto/releases/tag/v0.22.0

## [0.21.0] - 2026-08-11

### Added

- **TextArea input widget** — Replaced manual `Paragraph`-based input with `ratatui-textarea` (v0.9.2), providing consistent cursor handling, word wrap, and proper line editing.
- **Up/Down cursor navigation** — Arrow keys navigate cursor lines in the input.
- **Planner subagent** — Subagent for task decomposition and PLAN.md lifecycle management.
- **Agent-manager subagent** — Subagent for agent and skill lifecycle management.

### Changed

- `src/tui/input.rs`: Rewritten to use `TextArea` widget, removed manual `input`/`input_cursor` fields.
- `src/tui/keys.rs`: Updated cursor navigation methods.
- `src/tui/app.rs`: All `app.input` references updated to use `textarea`.

[0.21.0]: https://github.com/atareao/anacleto/releases/tag/v0.21.0

## [0.20.0] - 2026-08-10

### Added

- **Item counts in panel titles** — Agent, MCP, and Skill panel headers now show item counts.
- **Resilient skill loading** — Skill loading from workspace and global paths continues even if one path fails.

### Changed

- `src/tui/render.rs`: Panel titles include counts.
- `src/skill/discovery.rs`: Graceful handling of missing/inaccessible directories.

[0.20.0]: https://github.com/atareao/anacleto/releases/tag/v0.20.0

## [0.19.0] - 2026-08-10

### Added

- **Configurable retry policy** — Retry policy with error classification for LLM providers. Supports exponential backoff with jitter, per-operation retry counts, and error classification (retryable vs. fatal).

[0.19.0]: https://github.com/atareao/anacleto/releases/tag/v0.19.0

## [0.18.0] - 2026-08-10

### Added

- **Skill discovery** — Automatic discovery of skills from workspace `.agents/skills/` and global `~/.config/anacleto/skills/`.
- **Workspace persistence** — Session workspace paths are persisted and restored.
- **TUI overhaul** — Refactored TUI rendering with improved layout, agent status panel, and skill panel.

[0.18.0]: https://github.com/atareao/anacleto/releases/tag/v0.18.0

## [0.17.1] - 2026-08-09

### Added

- **Updated find-skills skill** — v1.1 with skills.sh registry integration.

### Fixed

- **Thinking message ordering** — Thinking blocks are now persisted in the correct order within committed messages.

[0.17.1]: https://github.com/atareao/anacleto/releases/tag/v0.17.1

## [0.17.0] - 2026-08-09

### Added

- **Edit-agent dialog** — Ctrl+E dialog to edit agent/subagent skills, MCPs, and subagents.
- **SubAgents tab** — New tab in the info panel showing subagent tree with type and mode.
- **Agent switching** — Enter key switches to a selected agent.
- **Config directory rename** — Project config directory renamed from `.anacleto/` to `.agents/`.

[0.17.0]: https://github.com/atareao/anacleto/releases/tag/v0.17.0

## [0.16.0] - 2026-08-09

### Added

- **Real-time thinking/reasoning display** — LLM providers' thinking/reasoning content is displayed in real time in the TUI, wrapped in `[thinking]`/`[/thinking]` markers.

[0.16.0]: https://github.com/atareao/anacleto/releases/tag/v0.16.0

## [0.15.1] - 2026-08-09

### Changed

- Refactored skill files from `skill.md` to `SKILL.md` naming convention.

[0.15.1]: https://github.com/atareao/anacleto/releases/tag/v0.15.1

## [0.15.0] - 2026-08-09

### Added

- **Git branch in TUI footer** — Current git branch name displayed in the status bar.
- **`/reload` command** — Respawns the active agent mid-session, reloading config and skills.
- **Emergency stop (Ctrl+C)** — Cancels in-flight agent activity via a direct cancel flag that bypasses the mpsc channel.
- **Mouse click-to-select panels** — Click on any panel (Chat, Info, MCPs, Skills, Agents) to focus it directly.
- **Focus cycling** — Tab/Shift+Tab to cycle focus between panels.
- **Keymap module** — Centralized key-to-action mapping.
- **Theme module** — Themed border colors for AI, tool, and status messages.

[0.15.0]: https://github.com/atareao/anacleto/releases/tag/v0.15.0

## [0.14.0] - 2026-08-08

### Added

- **Mouse click-to-select panels** — Click on any panel (Chat, Info, MCPs, Skills, Agents) to focus it directly.
- **Focus cycling** — Tab/Shift+Tab to cycle focus between panels.
- **Keymap module** (`src/tui/keymap.rs`) — Centralized key-to-action mapping with `Action::FocusNext` and `Action::FocusPrev`.
- **Keys module** (`src/tui/keys.rs`) — Mouse event handling (`handle_mouse()`) with ratatui layout hit-testing.
- **Theme module** (`src/tui/theme.rs`) — Themed border colors for AI, tool, and status messages.
- **Streaming improvements** — Tool execution markers (🔧) and results (✅/❌) appear inline within the AI's response stream.
- **Reversed scroll direction** — `j` scrolls forward (down), `k` scrolls backward (up), matching vim conventions.
- **Auto-scroll on send** — Chat scroll resets to bottom when sending a new message.

### Changed

- `src/tui/app.rs`: Added `Event::Mouse` handling in event loop, removed `mcp_scroll`/`skill_scroll` fields.
- `src/tui/events.rs`: Streaming tool markers inline, preserved `HookExecuted` handler.
- `src/tui/navigation.rs`: Reversed scroll direction, Tab→Right/Left for info panel cycling.
- `src/tui/render.rs`: Theme colors for tools, tool marker detection within AI responses, simplified MCP/Skill panels.
- `src/tui/markdown.rs`: Simplified `select_visible_start` return type.
- `src/main.rs`: Added `EnableMouseCapture` / `DisableMouseCapture`.

[0.14.0]: https://github.com/atareao/anacleto/releases/tag/v0.14.0

## [0.13.0] - 2026-08-08

### Added

- **Auto-Configuration Hooks** — Configurable hook system that fires shell commands at agent lifecycle points (`BeforeTool`, `AfterTool`, `BeforeApply`, `AfterApply`, `BeforeShell`, `AfterShell`, `BeforeFsWrite`, `AfterFsWrite`, `OnStartup`, `OnShutdown`).
- **Three-layer auto-registration** merged into the `HookRegistry` with precedence Config > Plugin > Skill > Auto-detect and deduplication:
  - PATH auto-detect (`src/hook/autoconfig.rs`): scans PATH for known tools (e.g. codegraph) and registers their sync hooks.
  - Skill frontmatter: skills can declare `hooks` in their YAML frontmatter.
  - Plugin trait: `Plugin::register_hooks()` lets plugins register hooks.
- Hooks wired into tool execution (shell, filesystem) and the engine orchestrator.
- `PluginRegistry::list()`.
- `temporal.txt` and the third-party `awesome-claude-skills/` repository excluded from the repo (added to `.gitignore`).

### Changed

- `src/hook/` (new): `HookRegistry` with three-layer merged auto-registration and deduplication.
- `src/hook/autoconfig.rs` (new): PATH auto-detection of known tools.
- Hooks connected to tool execution (shell, filesystem) and the engine orchestrator.
- `PluginRegistry::list()`.
- `.gitignore`: `temporal.txt` and `awesome-claude-skills/`.

[0.13.0]: https://github.com/atareao/anacleto/releases/tag/v0.13.0

## [0.11.0] - 2026-08-08

### Added

- **SkillRegistry centralizado** — Nuevo `SkillRegistry` con caché, hot-reload y lookup O(1) por nombre. Los skills se cargan una vez al inicio del engine en lugar de re-parsificar del disco por cada agente.
- **DefaultSkillExecutor** — Implementación concreta del trait `SkillExecutor` que despacha a handlers built-in (shell, web, filesystem) según el nombre del skill.
- **Límite de concurrencia configurable** — Nueva opción `max_concurrency` en `SessionConfig` (default: 4). Los subagentes paralelos se limitan mediante un `tokio::sync::Semaphore`.
- **Integración del registry en el engine** — `Engine` ahora tiene un campo `skill_registry: SharedSkillRegistry` inicializado en startup. `SpawnAgentConfig` usa `skill_names: Vec<String>` + referencia al registry en lugar de `Vec<Skill>`.

### Changed

- `src/skill/registry.rs` (nuevo): `SkillRegistry` con `load_from_paths()`, `get()`, `list()`, `reload()`, `contains()`.
- `src/skill/executor.rs` (nuevo): `DefaultSkillExecutor` con dispatch a shell, web y filesystem.
- `src/agent/lifecycle.rs`: `SpawnAgentConfig` reemplaza `skills: Vec<Skill>` por `skill_registry + skill_names`; añade `concurrency_semaphore`.
- `src/agent/tools.rs`: sistema de subagentes migrado al registry.
- `src/config/types.rs`: nuevo campo `max_concurrency` en `SessionConfig`.
- `src/engine/orchestrator.rs`: `Engine` con `skill_registry` cargado una vez al inicio.
- `src/engine/commands.rs`: comando `/skills` usando el registry.

[0.11.0]: https://github.com/atareao/anacleto/releases/tag/v0.11.0

## [0.10.0] - 2026-08-07

### Added

- **Streaming en subagentes** — Los subagentes ahora usan `complete_stream()` en lugar de `complete()`, transmitiendo fragmentos de respuesta en tiempo real a la TUI a través de `EngineEvent::AgentStreamChunk`.
- **Modo headless** — Nuevo flag `--headless` para ejecutar Anacleto sin TUI, con `--task` opcional para enviar un prompt inicial. Las respuestas se escriben a stdout.
- **Config hot-reload (SIGHUP)** — `kill -HUP <pid>` recarga la configuración YAML del disco sin reiniciar el proceso. Nuevos `EngineCommand::ReloadConfig` y `EngineEvent::ConfigReloaded`.
- **Logs a archivo** — Los logs se escriben simultáneamente a stdout y a `~/.local/share/anacleto/logs/anacleto.log` con rotación diaria vía `tracing-appender`.
- **History search en TUI** — Ctrl+R abre un overlay de búsqueda en el historial de la conversación con filtrado case-insensitive, navegación ↑↓, Enter para saltar al mensaje y Esc para cerrar.
- **CI/CD pipeline** — GitHub Actions con jobs separados para fmt, clippy, build y test en cada push/PR a main/development.

### Changed

- `src/agent/tools.rs`: subagentes cambian de `complete()` a `complete_stream()` con emisión de chunks.
- `src/main.rs`: nuevo flag `--headless`, inicialización dual de tracing (stdout + archivo), listener SIGHUP.
- `src/engine/events.rs`: nuevos variants `ReloadConfig` y `ConfigReloaded`.
- `src/engine/orchestrator.rs`: nuevo método `reload_config()`.
- `src/tui/keymap.rs`: nuevo action `ToggleSearch`.
- `src/tui/keys.rs`: manejo del overlay de búsqueda.
- `src/tui/render.rs`: renderizado del overlay de búsqueda.
- `src/tui/types.rs`: nuevo tipo `SearchState`.
- `src/tui/app.rs`: nuevo campo `search`, métodos `update_search_matches()` y `chat_height_at()`.

[0.10.0]: https://github.com/atareao/anacleto/releases/tag/v0.10.0

## [0.9.0] - 2026-08-07

### Added

- **Parallel subagents of the same type** — The `task` tool now accepts an optional `agent` parameter referencing a configured subagent type (e.g. `reviewer`, `writer`). When provided, the spawned subagent inherits all of that type's instructions, skills, MCPs, model and permissions. Multiple `task` calls in the same turn run concurrently, with results consolidated in the original order.
- **Subagent type and mode in the TUI** — The agent panel, agent list and subagent tree now show the configured subagent type (`[name]`) or `[generic]` for dynamic subagents, plus the execution mode (`(fg)`/`(bg)`).

## [0.1.0] - 2025-03-15

### Added

- **Agent/subagent model** — Agents and subagents share the same type; agents own a list of subagents by name. Only agents are user-invocable. Subagents are disposable: create, work, reply, destroy. No inheritance from parent.
- **Skill system** — Skills defined as Markdown files with YAML frontmatter (Anthropic format). Loaded per-agent, no inheritance. Communicated via a trait interface, decoupled from the engine.
- **MCP integration** — JSON-RPC 2.0 client over stdio and TCP. Consumer-only (no lifecycle management). Per-agent MCP server lists, sourced from config.
- **TUI** — Terminal interface built with ratatui + crossterm, running in the same process as the engine on separate Tokio tasks.
- **YAML configuration** — Global (`~/.config/anacleto/`) and project (`.agents/`) config files merged on startup. Supports agent definitions, MCP servers, and permission rules.
- **SQLite persistence** — Async SQLite via sqlx. Sessions are resumable. Context window limited to 50% of the model's maximum.
- **Permission system** — Allow by default, deny explicitly. Human approval required for sensitive operations (e.g., shell commands, network access).
- **LLM providers** — Support for Anthropic, OpenAI, OpenRouter, and Ollama APIs with a common provider interface.
- **Streaming** — Always-on streaming of LLM responses. Intermediate steps (skill/MCP execution) are visible in the TUI in real time.
- **Configurable retries** — Exponential backoff with jitter for LLM, MCP, and subagent timeouts. Retry counts and backoff parameters configurable per-operation.
- **CLI argument parsing** — Command-line interface via clap for runtime configuration overrides.

[0.1.0]: https://github.com/atareao/anacleto/releases/tag/v0.1.0
[0.9.0]: https://github.com/atareao/anacleto/releases/tag/v0.9.0