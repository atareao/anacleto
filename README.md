# Anacleto

**Agent orchestration engine in Rust** — manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a terminal UI built with [ratatui] + [crossterm].

[ratatui]: https://github.com/ratatui-org/ratatui
[crossterm]: https://github.com/crossterm-rs/crossterm

> **Status:** Early development. Not yet recommended for production use.

---

## Features

- **Agent orchestration** — root agents delegate tasks to disposable subagents. Subagents are created per-task, reply, and are destroyed.
- **Skills** — Markdown + YAML frontmatter skills in [Anthropic format], loaded dynamically from the filesystem.
- **MCP integration** — Model Context Protocol clients over stdio or TCP transports.
- **Multi-LLM** — Anthropic (Claude), OpenAI (GPT), OpenRouter, and Ollama (local models).
- **Permissions** — Allow-by-default, deny-explicitly model with configurable rules per agent.
- **TUI** — ratatui-based terminal interface with persistent event streaming.
- **Session persistence** — SQLite via sqlx; sessions are resumable across restarts.
- **Resilience** — Exponential backoff with jitter for LLM, MCP, and subagent retries.
- **Configurable context window** — configurable per agent with a 50% default history limit.

[Anthropic format]: https://docs.anthropic.com/en/docs/build-with-claude/tool-use

---

## Quickstart

### Install

```sh
cargo install --path .
```

Requires Rust ≥ 1.85 (edition 2024).

### Configure

```sh
mkdir -p ~/.config/anacleto/
cp docs/example-global-config.yaml ~/.config/anacleto/config.yaml
```

Edit the config to add your API keys. Keys are resolved from environment variables at runtime using `${VAR}` syntax:

```sh
export ANTHROPIC_API_KEY="sk-..."
export OPENAI_API_KEY="sk-..."
```

Optionally add a project-level config at `.anacleto/config.yaml` that merges on top of the global config.

### Run

```sh
anacleto
```

CLI flags:

| Flag | Description |
|---|---|
| `-c`, `--config <PATH>` | Path to a project config file (overrides auto-detection) |
| `-d`, `--database <PATH>` | Database path (overrides config) |
| `-v`, `--verbose` | Enable debug-level logging |

---

## Architecture

```
src/
├── main.rs           # Entrypoint, CLI arg parsing (clap)
├── lib.rs            # Module declarations
├── agent/            # Agent/subagent types, lifecycle, communication
├── config/           # YAML config parsing, global + project merge
├── db/               # SQLite persistence via sqlx
├── engine/           # Orchestration loop (spawn, route, collect)
├── error.rs          # Global error types (thiserror)
├── llm/              # LLM providers (Anthropic, OpenAI, OpenRouter, Ollama)
├── mcp/              # MCP client (JSON-RPC 2.0 over stdio/TCP)
├── permissions/      # Permission rules per agent/subagent
├── skill/            # Skill loading (Anthropic Markdown format), execution
└── tui/              # ratatui + crossterm terminal interface
```

### Design decisions

> 8 Architecture Decision Records are available at [`docs/adr/`](docs/adr/).

| ADR | Decision |
|---|---|
| [ADR-0001](docs/adr/ADR-0001-agent-model.md) | Agents and subagents are the same type. Subagents are disposable and cannot nest. |
| [ADR-0002](docs/adr/ADR-0002-skill-system.md) | Skills are Markdown + YAML frontmatter (Anthropic format), loaded dynamically. |
| [ADR-0003](docs/adr/ADR-0003-mcp-integration.md) | MCP is consumed via JSON-RPC 2.0 over stdio or TCP. |
| [ADR-0004](docs/adr/ADR-0004-tui-architecture.md) | ratatui + crossterm, same process as engine (separate Tokio tasks). |
| [ADR-0005](docs/adr/ADR-0005-configuration-system.md) | YAML. Global (`~/.config/anacleto/`) + project (`.anacleto/`) merged. |
| [ADR-0006](docs/adr/ADR-0006-persistence.md) | SQLite via sqlx. Sessions are resumable. Context limit: 50% of model window. |
| [ADR-0007](docs/adr/ADR-0007-permissions-model.md) | Allow by default, deny explicitly. Human approval for sensitive ops. |
| [ADR-0008](docs/adr/ADR-0008-technology-stack.md) | Rust edition 2024, Tokio, ratatui, sqlx, reqwest, serde. |

---

## Configuration

### Global config

`~/.config/anacleto/config.yaml` defines machine-wide defaults for all projects.

### Project config

`.anacleto/config.yaml` in the project root merges on top of the global config. Agents with the same name override their global counterparts.

### LLM provider resolution

Model names are matched to providers via prefix rules in the engine:

| Pattern | Provider |
|---|---|
| Starts with `claude` | Anthropic |
| Starts with `gpt`, `o1`, or `o3` | OpenAI |
| Contains `/` | OpenRouter (OpenAI-compatible) |
| Anything else | Ollama |

### Agent schema

```yaml
agents:
  - name: root
    description: ".anacleto/agents/root.md"
    model: "claude-sonnet-4"
    skills:
      - ".anacleto/skills/shell/"
    mcps:
      - filesystem
    permissions:
      deny:
        - "command.run.sudo"
    subagents:
      - reviewer
      - writer
```

Subagents are fully independent — they do not inherit skills, MCPs, or permissions from their parent.

### Retry policy

Retries use exponential backoff with jitter:

```
delay = min(base_delay × 2^attempt × random(0.75, 1.25), max_delay)
```

---

## Development

```sh
cargo build
cargo test
cargo clippy
cargo fmt
```

Make sure to run `cargo fmt --check && cargo clippy && cargo test` before committing.

### Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime |
| `ratatui` + `crossterm` | TUI |
| `serde` + `serde_yaml` | Serialization |
| `sqlx` | SQLite (async) |
| `reqwest` | HTTP client for LLM APIs |
| `tower` | Middleware (retries, rate limiting) |
| `clap` | CLI argument parsing |
| `anyhow` + `thiserror` | Error handling |
| `tracing` | Structured logging |
| `uuid` | Session/agent ID generation |
| `chrono` | Date/time |

---

## Project status

Active development. Core architecture is stable; APIs and configuration schema may change.

### Roadmap

- [x] Agent/subagent lifecycle
- [x] TUI with ratatui
- [x] Multi-LLM provider support
- [x] MCP stdio/TCP clients
- [x] Session persistence (SQLite)
- [x] Permission system
- [ ] Window management and layout persistence in TUI
- [ ] MCP server lifecycle management
- [ ] Skill marketplace