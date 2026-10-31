# Referencia — Testing

Texto literal extraído de `AGENTS.md` (sección *Testing*).

## Testing

- Unit tests live next to the code (`#[cfg(test)] mod tests` in each file).
- Integration tests go in `tests/` at the crate root.
- Property-based testing via `proptest` (dev-dependency) for randomized assertions.
- MCP integration tests require a mock MCP server binary in `tests/mocks/`.
- Skill tests must not access the network unless the test is marked `#[ignore]`.
- Session persistence tests use `tempfile` for isolated database files.
- Run full suite: `cargo test`
