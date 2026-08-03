# Anacleto — AGENTS.md

## Project identity

Anacleto is an agent orchestration engine in Rust, inspired by OpenCode. It manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a TUI built with ratatui + crossterm.

## Toolchain

- **Rust edition 2024** — requires rustc ≥ 1.85. Current: 1.97.0 (stable).
- Edition 2024 changes: `impl Trait` in arg position is `use<..>`-aware, `unsafe` blocks are required on `static mut` accesses, `gen` is a reserved keyword, and `cargo fix --edition` handles migration. Do not target older editions.
- No `rust-toolchain.toml` yet — pin one once CI is set up.

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
  main.rs              # entrypoint, CLI arg parsing
  tui/                 # ratatui + crossterm (sole interface)
  engine/              # orchestration loop (spawn, route, collect)
  agent/               # agent/subagent types, lifecycle, communication
  skill/               # skill loading (Anthropic Markdown format), execution
  mcp/                 # MCP client (JSON-RPC 2.0 over stdio/TCP)
  llm/                 # LLM providers (Anthropic, OpenAI, Ollama)
  config/              # YAML config parsing, global + project merge
  permissions/         # permission rules per agent/subagent
  db/                  # SQLite persistence via sqlx
  error.rs             # global error types
```

### Key design decisions (see docs/adr/ for full ADRs)

| Decision | Choice |
|---|---|
| **Agent model** | Agents and subagents are the same type. Agents have `subagents: []`. Only agents are user-invocable. Subagents cannot nest. |
| **Subagent lifecycle** | Disposable: create → work → reply → destroy. No inheritance from parent. |
| **Skills** | Markdown + YAML frontmatter (Anthropic format). Per-agent, no inheritance. |
| **MCPs** | Consumer only (no lifecycle management). Per-agent, no inheritance. |
| **TUI** | ratatui + crossterm, same process as engine (separate Tokio tasks). |
| **Config** | YAML. Global (`~/.config/anacleto/`) + project (`.anacleto/`) merged. |
| **Persistence** | SQLite via sqlx. Sessions are resumable. Context limit: 50% of model window. |
| **Permissions** | Allow by default, deny explicitly. Human approval for sensitive ops. |
| **Streaming** | Always on by default. Intermediate steps (skill/MCP execution) visible in TUI. |
| **Error handling** | Retries configurable for LLM, MCP, subagent timeouts. Session is recoverable. |

### Agent/subagent config schema

```yaml
agents:
  - name: root
    description: ~/.config/anacleto/agents/root.md   # Markdown with frontmatter
    model: claude-sonnet-4
    skills:
      - ~/.config/anacleto/skills/shell/
    mcps: [filesystem]                                 # references global MCPs
    permissions:
      deny: []
    subagents: [reviewer, writer]                      # references by name

  - name: reviewer
    description: ~/.config/anacleto/agents/reviewer.md
    model: claude-sonnet-4
    skills:
      - ~/.config/anacleto/skills/code-review/
    mcps: [filesystem-tmp]
    permissions:
      deny: [command.run, net.http]
```

## Dependencies

| Crate | Purpose |
|---|---|
| `tokio` | Async runtime |
| `ratatui` + `crossterm` | TUI |
| `serde` + `serde_yaml` | Serialization |
| `sqlx` | SQLite (async) |
| `reqwest` | HTTP client for LLM APIs |
| `tower` | Middleware (retries, rate limiting) |
| `anyhow` | Error handling |

## Testing

- Unit tests live next to the code (`#[cfg(test)] mod tests` in each file).
- Integration tests go in `tests/` at the crate root.
- MCP integration tests require a mock MCP server binary in `tests/mocks/`.
- Skill tests must not access the network unless the test is marked `#[ignore]`.

## Anti-patterns

- Do not add dependencies lightly. Prefer the standard library or the crates listed above. Avoid `async-std`, `actix`, or niche agent frameworks.
- Do not conflate agent identity with OS processes. Agents are in-process async tasks, not subprocesses.
- Do not embed skill logic in the engine. Skills are loaded dynamically and communicate via a trait interface.
- Do not hardcode MCP server paths. They come from config.
- Do not make subagents inherit anything from their parent. They are fully independent.
- Do not add a web UI or batch mode. TUI is the sole interface.