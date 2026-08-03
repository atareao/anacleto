# Anacleto — TODO

> Proyecto feature-complete contra ADRs originales. Esto es lo que queda para llevarlo a producción.

## 1. 🧹 Limpieza (clippy warnings)

| # | Warning | Archivo | Fix |
|---|---|---|---|
| 1 | `tool_calls` never read (OpenAiStreamDelta) | `src/llm/provider.rs:137` | `#[allow(dead_code)]` o prefix `_` |
| 2 | `tool_calls` never read (OllamaResponseMessage) | `src/llm/provider.rs:239` | `#[allow(dead_code)]` o prefix `_` |
| 3 | `too_many_arguments` spawn_agent (11 args) | `src/agent/lifecycle.rs:54` | Builder pattern o struct de config |
| 4 | `too_many_arguments` spawn_subagent_and_delegate (9 args) | `src/agent/lifecycle.rs:556` | Builder pattern o struct de config |
| 5 | `needless_borrow` (ref db) x2 | `src/agent/lifecycle.rs:140,253` | Quitar `ref` |
| 6 | `redundant_closure` (Error::Provider) | `src/agent/lifecycle.rs:584` | Usar `Error::Provider` directo |
| 7 | `let_unit_value` (tracing::warn) | `src/agent/retry.rs:43` | Quitar `let _ =` |
| 8 | `new_without_default` AgentId | `src/agent/types.rs:13` | Añadir `impl Default` |
| 9 | `new_without_default` LlmProviderRegistry | `src/llm/provider.rs:1106` | Añadir `impl Default` |
| 10 | `new_without_default` McpRegistry | `src/mcp/client.rs:289` | Añadir `impl Default` |
| 11 | `empty_line_after_doc_comments` | `src/agent/retry.rs:5` | Quitar línea vacía |
| 12 | `empty_line_after_doc_comments` (otras) | varios | Revisar doc comments |

## 2. 📝 Documentación

- [ ] README.md — intro, quickstart, arquitectura, comandos, ejemplos
- [ ] Guía de usuario — config, skills, MCPs, agentes, sesiones
- [ ] Ejemplo funcional end-to-end con docker-compose
- [ ] Comentarios de documentación en API pública (rustdoc)

## 3. 🚀 Profesionalización

- [ ] CI/CD: GitHub Actions (build, test, clippy, fmt en PRs y push)
- [ ] Versionado: release 0.1.0 con changelog
- [ ] Dockerfile multi-stage + docker-compose para desarrollo
- [ ] rust-toolchain.toml para pin de versión

## 4. 🔬 Testing avanzado

- [ ] Mock MCP server para tests de integración reales
- [ ] Medir cobertura (tarpaulin)
- [ ] Tests de propiedades (proptest)
- [ ] Tests de estrés/concurrencia

## 5. ✨ Features post-ADR

- [ ] Streaming en subagentes (ahora usan `complete()` no `complete_stream()`)
- [ ] Modo headless (sin TUI, para scripts/automación)
- [ ] Config hot-reload (SIGHUP)
- [ ] Logs a archivo (tracing subscriber con fichero + stdout)
- [ ] Comando `/status` en TUI con info del sistema
- [ ] History search en TUI (Ctrl+R style)