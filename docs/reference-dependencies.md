# Referencia — Dependencies

Texto literal extraído de `AGENTS.md` (sección *Dependencies*).

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
