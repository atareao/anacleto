# Anacleto — TODO

> Estado actual: **feature-complete para v0.10.0**. Todas las features post-ADR implementadas.

## ✅ Completado

### CI/CD y toolchain
- [x] GitHub Actions CI (fmt, clippy, build, test en push/PR)
- [x] rust-toolchain.toml (Rust 2024 edition, 1.85+)
- [x] Clippy: 0 warnings

### Documentación
- [x] README.md (578 líneas — intro, quickstart, arquitectura, comandos)
- [x] docs/user-guide.md (469 líneas — config, skills, MCPs, agentes)
- [x] docs/example.md (419 líneas — ejemplo end-to-end)
- [x] CHANGELOG.md (hasta v0.10.0)

### Testing
- [x] Mock MCP server (tests/mocks/mcp_server.py)
- [x] Tests de integración (tests/integration_test.rs, 442 líneas)
- [x] Tests de concurrencia (tests/concurrency_test.rs, 178 líneas)
- [x] Tests de integración MCP (tests/mcp_integration_test.rs, 141 líneas)
- [x] proptest en Cargo.toml
- [x] tarpaulin en Cargo.toml (cobertura)
- [x] Tests específicos para headless, streaming, SIGHUP y search

### Features post-ADR
- [x] Streaming en subagentes (complete() → complete_stream())
- [x] Modo headless (--headless + --task)
- [x] Config hot-reload (SIGHUP)
- [x] Logs a archivo (tracing-appender, daily rotation)
- [x] Comando /status en TUI
- [x] History search en TUI (Ctrl+R)

### Profesionalización
- [x] README, guía de usuario, ejemplo funcional
- [x] CI/CD pipeline
- [x] rust-toolchain.toml
- [x] CHANGELOG con versionado semántico

## 📋 Pendiente para v1.0.0

- [ ] Release automation (script o workflow GitHub)
- [ ] Tag v1.0.0 + GitHub Release
- [ ] Auditoría de seguridad de dependencias (cargo audit)
- [ ] Benchmarks de rendimiento
- [ ] Integración continua con más toolchains (beta, nightly)