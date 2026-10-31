# Anacleto — AGENTS.md

## Project identity

Anacleto is an agent orchestration engine in Rust, inspired by OpenCode. It manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a TUI built with ratatui + crossterm.

## Toolchain

- **Rust edition 2024** — requires rustc ≥ 1.85. Current: 1.97.0 (stable).
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

## Architecture (`docs/reference-architecture.md`)

- Agents and subagents are the same type. Subagents are disposable and cannot nest. Multiple root agents are supported.
- Subagent lifecycle: disposable; when it runs out of steps it MUST stop, return all results to the parent, and explicitly indicate it has run out of steps.
- Each agent is a self-contained Markdown file with YAML frontmatter, located in `agents/` (global: `~/.config/anacleto/agents/*.md`, project: `.agents/agents/*.md`). Project overrides global when same name.
- Exactly one agent must declare `role: root`.
- Subagents are fully independent — they do **not** inherit skills, MCPs, permissions, or any other config from their parent.

## Testing (`docs/reference-testing.md`)

- Unit tests live next to the code (`#[cfg(test)] mod tests` in each file); integration tests go in `tests/` at the crate root.
- Skill tests must not access the network unless the test is marked `#[ignore]`. Run full suite: `cargo test`

## Git Flow

Este proyecto sigue **Git Flow** estricto. Ver [GIT_FLOW.md](./GIT_FLOW.md):

- Estructura de ramas: `main`, `development`, `feature/*`, `hotfix/*`.
- Conventional commits con **gitmoji**.
- Cómo crear features, hotfixes y releases.

## Dónde está el resto

| Tema | Fichero |
|---|---|
| Cambios de edition 2024 y migración | `docs/reference-toolchain.md` |
| Module layout, decisiones de diseño, definición de agentes | `docs/reference-architecture.md` |
| Dependencies (crates y propósito) | `docs/reference-dependencies.md` |
| Testing (proptest, mock MCP, tempfile) | `docs/reference-testing.md` |
| Anti-patterns | `docs/reference-anti-patterns.md` |
| ADRs completos | `docs/adr/ADR-0001-agent-model.md` |
