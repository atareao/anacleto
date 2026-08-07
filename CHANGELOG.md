# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- **YAML configuration** — Global (`~/.config/anacleto/`) and project (`.anacleto/`) config files merged on startup. Supports agent definitions, MCP servers, and permission rules.
- **SQLite persistence** — Async SQLite via sqlx. Sessions are resumable. Context window limited to 50% of the model's maximum.
- **Permission system** — Allow by default, deny explicitly. Human approval required for sensitive operations (e.g., shell commands, network access).
- **LLM providers** — Support for Anthropic, OpenAI, OpenRouter, and Ollama APIs with a common provider interface.
- **Streaming** — Always-on streaming of LLM responses. Intermediate steps (skill/MCP execution) are visible in the TUI in real time.
- **Configurable retries** — Exponential backoff with jitter for LLM, MCP, and subagent timeouts. Retry counts and backoff parameters configurable per-operation.
- **CLI argument parsing** — Command-line interface via clap for runtime configuration overrides.

[0.1.0]: https://github.com/atareao/anacleto/releases/tag/v0.1.0
[0.9.0]: https://github.com/atareao/anacleto/releases/tag/v0.9.0