# Referencia — Anti-patterns

Texto literal extraído de `AGENTS.md` (sección *Anti-patterns*).

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