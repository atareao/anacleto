# Anacleto — AGENTS.md

## Project identity

Anacleto is an agent orchestration engine in Rust, inspired by OpenCode. It manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a TUI built with ratatui + crossterm.

## Toolchain

- **Rust edition 2024** — requires rustc ≥ 1.85. Current: 1.97.0 (stable).
- Edition 2024 changes: `impl Trait` in arg position is `use<..>`-aware, `unsafe` blocks are required on `static mut` accesses, `gen` is a reserved keyword, and `cargo fix --edition` handles migration. Do not target older editions.
- **Minimum rust-version**: 1.97 (as specified in `Cargo.toml`).

## Developer commands

```sh
cargo build              # debug build
cargo build --release    # release build
cargo run                # run the binary (starts TUI)
cargo test               # all tests
cargo test <name>        # single test by name substring
cargo clippy             # lint (must pass before commits)
cargo fmt                # format (must pass before commits)
cargo doc --no-deps      # local docs
```

**Required order before committing:** `cargo fmt --check && cargo clippy && cargo test`

## Architecture

### Module layout

```
src/
  lib.rs               # crate root — re-exports all modules
  main.rs              # entrypoint, CLI arg parsing via clap
  agent/               # agent model, lifecycle, session, tools, context, loader, retry
  config/              # YAML config parsing, global + project merge, path resolution
  db/                  # SQLite persistence via sqlx (session, messages, todos, snapshots, export, usage)
  engine/              # orchestration loop (orchestrator, sessions, commands, events, jobs, apply_patch, template)
  error.rs             # global error types (thiserror)
  hook/                # hook system — configurable shell commands at lifecycle points
  llm/                 # LLM providers (Anthropic, OpenAI, Ollama, Azure, Bedrock, Google, OpenRouter)
  lsp/                 # Language Server Protocol queries (LspClient, result formatting)
  mcp/                 # MCP client (JSON-RPC 2.0 over stdio/TCP, registry, types, parsing)
  plugin/              # plugin system with hooks and custom tool registration
  shell/               # shell command execution + modern CLI tool inventory
  skill/               # skill loading (Anthropic Markdown format), execution
  tools/               # structured agent tools (read, grep, glob, web, lsp, mcp, search_symbol)
  tui/                 # ratatui + crossterm interface (app, events, keys, input, navigation, keymap, render, theme, etc.)
```

### Key design decisions (see docs/adr/ for full ADRs)

| Decision | Choice |
|---|---|
| **Agent model** | Agents and subagents are the same type. Subagents are disposable and cannot nest. Multiple root agents are supported. |
| **Subagent lifecycle** | Disposable: create → work → reply → destroy. No inheritance from parent. When a subagent runs out of steps, it MUST stop, return all results to the parent agent, and explicitly indicate it has run out of steps. |
| **Agent definition** | Each agent is a Markdown file with YAML frontmatter (structural config in frontmatter, system prompt in body). Located in `agents/` directory (global: `~/.config/anacleto/agents/*.md`, project: `.agents/agents/*.md`). |
| **Skills** | Markdown + YAML frontmatter (Anthropic format). Per-agent, no inheritance. Loaded dynamically from the filesystem. |
| **MCPs** | Consumer only (no lifecycle management). Per-agent, no inheritance. JSON-RPC 2.0 over stdio or TCP. |
| **TUI** | ratatui + crossterm, same process as engine (separate Tokio tasks). Window navigation with `Alt+1`..`Alt+5`. Vim-style bindings in panels. |
| **Config** | YAML. Global (`~/.config/anacleto/`) + project (`.agents/`) merged. Two layers with project overrides. |
| **Persistence** | SQLite via sqlx. Sessions are resumable across restarts. Context limit: 50% of model window. Fork/import/export and snapshot/rollback. |
| **Permissions** | Allow by default, deny explicitly. Human approval for sensitive ops. Rules per agent: `fs.*`, `net.http`, `command.run`, `mcp.use`, `env.read`, `skill.use`. |
| **Streaming** | Always on by default. Intermediate steps (skill/MCP execution) visible in TUI. |
| **Error handling** | Retries configurable for LLM, MCP, subagent timeouts. Exponential backoff with jitter. Session is recoverable. |
| **Plugins** | Declarative plugin system with hooks (`on_agent_spawn`, `on_tool_call`, `on_command`, `on_event`) and custom tool registration. Loaded from `~/.config/anacleto/plugins/`. |
| **Hooks** | Configurable shell commands at lifecycle points (before/after tool, apply_patch, shell, fs_write, on_startup, on_shutdown) with template variable substitution. |

### Agent definition (Markdown + YAML frontmatter)

Agents are **not** defined in `config.yaml`. Each agent is a self-contained Markdown file:

```markdown
---
name: root
description: Senior engineering agent
role: root
model: "claude-sonnet-4"
skills:
  - .agents/skills/shell/
  - .agents/skills/code-review/
mcps: [filesystem]
permissions:
  deny:
    - "command.run.sudo"
subagents:
  - reviewer
  - writer
max_steps: 90
---

You are **Anacleto**, a senior engineering agent...
```

Supported frontmatter fields:

| Field | Description |
|---|---|
| `name` | Unique agent name |
| `description` | Short human-readable summary |
| `role` | `root` or `subagent` (default `subagent`) |
| `model` | Model name resolved to a provider |
| `skills` | Skill paths (relative or absolute) |
| `mcps` | MCP server names (references global MCP config) |
| `permissions` | `allow`/`deny` list of permission rules |
| `subagents` | Subagent names (roots only, references by name) |
| `max_steps` | Maximum LLM+tool turns per task |

Resolution order:
1. Project agents: `.agents/agents/*.md`
2. Global agents: `~/.config/anacleto/agents/*.md`
3. Project overrides global when same name

Exactly one agent must declare `role: root`.

> **Important**: Subagents are fully independent — they do **not** inherit skills, MCPs, permissions, or any other config from their parent.

## Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime (full features) |
| `ratatui` + `crossterm` | TUI rendering and terminal input |
| `ratatui-textarea` | Text input widget with shell-style editing |
| `serde` + `serde_yaml` + `serde_json` | Serialization (YAML config, JSON-RPC) |
| `sqlx` | SQLite (async, runtime-tokio) |
| `reqwest` | HTTP client for LLM APIs (json, stream) |
| `clap` | CLI argument parsing (derive) |
| `anyhow` + `thiserror` | Error handling |
| `futures` | Async streams and combinators |
| `uuid` | Session/agent ID generation (v4, serde) |
| `tracing` + `tracing-appender` + `tracing-subscriber` | Structured logging (env-filter) |
| `syntect` | Syntax highlighting in TUI |
| `async-trait` | Async trait support |
| `dirs` | User config/data directories |
| `chrono` | Date/time (serde) |
| `rand` | Jitter for retry backoff |
| `unicode-width` | TUI text width calculation |

## Testing

- Unit tests live next to the code (`#[cfg(test)] mod tests` in each file).
- Integration tests go in `tests/` at the crate root.
- Property-based testing via `proptest` (dev-dependency) for randomized assertions.
- MCP integration tests require a mock MCP server binary in `tests/mocks/`.
- Skill tests must not access the network unless the test is marked `#[ignore]`.
- Session persistence tests use `tempfile` for isolated database files.
- Run full suite: `cargo test`

## Anti-patterns

- Do not add dependencies lightly. Prefer the standard library or the crates listed above. Avoid `async-std`, `actix`, or niche agent frameworks.
- Do not conflate agent identity with OS processes. Agents are **in-process async tasks**, not subprocesses.
- Do not embed skill logic in the engine. Skills are loaded dynamically and communicate via a trait interface.
- Do not hardcode MCP server paths. They come from config.
- Do not make subagents inherit anything from their parent. They are fully independent.
- Do not add a web UI or batch mode. TUI is the sole interface.
- Do not add heavy dependencies — think carefully about the binary size and compile time impact.
- Do not skip edge cases — this is agent tooling; failure modes matter.
- Do not add features without ADRs — non-obvious trade-offs belong in `docs/adr/`.