# ADR-0008: Technology Stack

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs a coherent technology stack. Key dependencies must be chosen early to establish the architecture.

## Decision

| Concern | Choice | Rationale |
|---|---|---|
| **Language** | Rust edition 2024 | System-level performance, safety, async ecosystem |
| **Async runtime** | Tokio | Industry standard, mature, excellent ecosystem |
| **TUI framework** | ratatui + crossterm | Most mature Rust TUI stack |
| **Serialization** | serde + serde_yaml | Standard Rust serialization |
| **Database** | sqlx (SQLite) | Async-native, compile-time query checking |
| **HTTP client** | reqwest | De facto standard, streaming support |
| **Middleware** | tower | Rate limiting, retries, tracing |
| **Error handling** | anyhow | Simple error handling for application code |
| **MCP protocol** | Custom JSON-RPC 2.0 | MCP spec is well-defined; no mature Rust crate exists |

## Consequences

- Coherent, well-tested ecosystem.
- All dependencies are async-native and Tokio-compatible.
- Minimal dependency count — each crate serves a clear purpose.
- Easy onboarding for Rust developers familiar with the ecosystem.

## Alternatives Considered

- **actix-web**: Rejected. Not needed; no HTTP server component.
- **async-std**: Rejected. Tokio has broader ecosystem support.
- **rusqlite**: Rejected in favor of sqlx for async-native SQLite access.