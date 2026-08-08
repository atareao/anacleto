# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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