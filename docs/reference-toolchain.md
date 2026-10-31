# Referencia — Toolchain

Texto literal extraído de `AGENTS.md` (sección *Toolchain*).

## Toolchain

- **Rust edition 2024** — requires rustc ≥ 1.85. Current: 1.97.0 (stable).
- Edition 2024 changes: `impl Trait` in arg position is `use<..>`-aware, `unsafe` blocks are required on `static mut` accesses, `gen` is a reserved keyword, and `cargo fix --edition` handles migration. Do not target older editions.
- **Minimum rust-version**: 1.97 (as specified in `Cargo.toml`).
