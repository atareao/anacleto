<p align="center">
  <img src="assets/anacleto-ai-agent-tool.png" alt="Anacleto TUI" width="100%" />
</p>

<h1 align="center">Anacleto</h1>

<p align="center">
  <strong>Agent orchestration engine in Rust</strong> — a terminal-first way to run trees of agents and subagents, with clean separation of skills, MCP servers, and permissions.
</p>

<p align="center">
  <a href="https://crates.io/crates/anacleto"><img alt="Crates.io" src="https://img.shields.io/crates/v/anacleto.svg"></a>
  <a href="https://docs.rs/anacleto"><img alt="docs.rs" src="https://img.shields.io/docsrs/anacleto"></a>
  <a href="https://github.com/atareao/anacleto/actions"><img alt="CI status" src="https://img.shields.io/github/actions/workflow/status/atareao/anacleto/ci.yml"></a>
  <a href="https://github.com/atareao/anacleto/blob/main/LICENSE"><img alt="License: MIT" src="https://img.shields.io/github/license/atareao/anacleto"></a>
  <a href="https://github.com/atareao/anacleto"><img alt="Rust version" src="https://img.shields.io/badge/rust-1.85+-blue"></a>
</p>

> [!WARNING]
> Anacleto is in **early development**. The core architecture is stable, but APIs and the configuration schema may change without notice. Do not rely on it in production yet.

---

## Table of contents

- [Features](#features)
- [Quickstart](#quickstart)
- [Architecture](#architecture)
- [Configuration](#configuration)
- [Development](#development)
- [Project status](#project-status)
- [License](#license)

---

## Features

- **Agent orchestration** — root agents delegate tasks to disposable subagents through a simple lifecycle: *create → work → reply → destroy*. Subagents cannot nest — they are fully independent of their parent.
- **Skills** — Markdown + YAML frontmatter skills in the [Anthropic format], loaded dynamically from the filesystem per agent.
- **MCP integration** — Model Context Protocol clients over [stdio or TCP] transports, referenced declaratively in config.
- **Multi-LLM** — Anthropic (Claude), OpenAI (GPT), OpenRouter, and Ollama (fully local models).
- **Permissions** — an allow-by-default / deny-explicitly model with rules configurable per agent.
- **TUI** — a ratatui-based terminal interface with persistent event streaming, running in the same process as the engine.
- **Session persistence** — SQLite via sqlx; sessions are resumable across restarts.
- **Resilience** — exponential backoff with jitter for LLM, MCP, and subagent retries.
- **Configurable context window** — a per-agent history limit, 50% of the model window by default.

[Anthropic format]: https://docs.anthropic.com/en/docs/build-with-claude/tool-use
[stdio or TCP]: https://github.com/modelcontextprotocol/modelcontextprotocol

---

## Quickstart

### 1. Install

```sh
cargo install --path .
```

> [!TIP]
> You need **Rust ≥ 1.85** (edition 2024). Check with `rustc --version`.

### 2. Configure

Create the global config for your machine:

```sh
mkdir -p ~/.config/anacleto/
cp docs/example-global-config.yaml ~/.config/anacleto/config.yaml
```

API keys are read from environment variables using `${VAR}` syntax. Export them before running:

```sh
export ANTHROPIC_API_KEY="sk-..."
export OPENAI_API_KEY="sk-..."
```

> [!WARNING]
> API keys are sensitive credentials. Use environment variables or a secrets manager — never commit keys to your repository or config files.

Optionally add a **project-level config** at `.anacleto/config.yaml` that merges on top of the global config.

### 3. Run

```sh
anacleto
```

CLI flags:

| Flag | Description |
|---|---|
| `-c`, `--config <PATH>` | Path to a project config file (overrides auto-detection) |
| `-d`, `--database <PATH>` | Database path (overrides config) |
| `-v`, `--verbose` | Enable debug-level logging |

See [`docs/user-guide.md`](docs/user-guide.md) for a full walkthrough.

---

## Architecture

```
src/
├── main.rs            # Entrypoint, CLI argument parsing (clap)
├── lib.rs             # Module declarations
├── agent/             # Agent/subagent types, lifecycle, communication, retries
├── config/            # YAML config parsing, global + project merge, paths
├── db/                # SQLite persistence via sqlx
├── engine/            # Orchestration loop (spawn, route, collect)
├── error.rs           # Global error types (thiserror)
├── filesystem/        # Filesystem access helpers
├── llm/               # LLM providers (Anthropic, OpenAI, OpenRouter, Ollama)
├── mcp/               # MCP client (JSON-RPC 2.0 over stdio/TCP)
├── permissions/       # Permission rules per agent/subagent
├── shell/             # Shell command execution
├── skill/             # Skill loading (Anthropic Markdown format), execution
└── tui/               # ratatui + crossterm terminal interface
```

### Design decisions

The project follows [Architecture Decision Records (ADRs)](docs/adr/); all eight are summarized here.

| ADR | Decision |
|---|---|
| [ADR-0001](docs/adr/ADR-0001-agent-model.md) | Agents and subagents are the same type; subagents are disposable and cannot nest. |
| [ADR-0002](docs/adr/ADR-0002-skill-system.md) | Skills are Markdown + YAML frontmatter (Anthropic format), loaded dynamically. |
| [ADR-0003](docs/adr/ADR-0003-mcp-integration.md) | MCP is consumed via JSON-RPC 2.0 over stdio or TCP; consumer-only. |
| [ADR-0004](docs/adr/ADR-0004-tui-architecture.md) | ratatui + crossterm in the same process as the engine (separate Tokio tasks). |
| [ADR-0005](docs/adr/ADR-0005-configuration-system.md) | YAML. Global (`~/.config/anacleto/`) + project (`.anacleto/`) merged. |
| [ADR-0006](docs/adr/ADR-0006-persistence.md) | SQLite via sqlx. Sessions resumable; context limit 50% of model window. |
| [ADR-0007](docs/adr/ADR-0007-permissions-model.md) | Allow by default, deny explicitly; human approval for sensitive ops. |
| [ADR-0008](docs/adr/ADR-0008-technology-stack.md) | Rust edition 2024, Tokio, ratatui, sqlx, reqwest, serde. |

---

## Configuration

Two layers of YAML configuration are merged at startup:

- **Global** — `~/.config/anacleto/config.yaml`, machine-wide defaults for all projects.
- **Project** — `.anacleto/config.yaml` in the project root, merged on top of the global config. Agents with the same name override their global counterparts.

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

> [!NOTE]
> Subagents are fully independent — they do not inherit skills, MCPs, or permissions from their parent.

### Retry policy

Retries use exponential backoff with jitter:

```
delay = min(base_delay × 2^attempt × random(0.75, 1.25), max_delay)
```

---

## Development

```sh
cargo build    # debug build
cargo test     # run all tests
cargo clippy   # lint
cargo fmt      # format
```

Run these before committing:

```sh
cargo fmt --check && cargo clippy && cargo test
```

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

Active development. The core architecture is stable; APIs and the configuration schema may change.

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

---

## License

Licensed under the [MIT License](LICENSE).
