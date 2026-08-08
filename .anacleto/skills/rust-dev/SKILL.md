---
name: rust-dev
description: Escribir, compilar, probar y depurar código Rust de forma idiomática
metadata:
  version: "1.0"
  category: development
  risk: medium
---

# Rust development skill

Especializado en escribir y mantener código Rust idiomático, moderno y correcto.
Usa este skill cuando necesites implementar, refactorizar, compilar, probar o
depurar código Rust dentro del workspace.

## Convenciones de edición 2024

- El proyecto usa **Rust edition 2024** (rustc ≥ 1.85). No apuntes a ediciones antiguas.
- `unsafe` se requiere explícitamente para accesos a `static mut`.
- `gen` es una palabra reservada.
- Para migrar ediciones usa `cargo fix --edition`.

## Flujo de trabajo estándar

1. **Explora** el crate: lee `Cargo.toml`, `rust-toolchain.toml` y la estructura de `src/`.
2. **Compila** antes de escribir: `cargo build` para conocer el estado base.
3. **Implementa** el cambio con tipos, módulos y nombres consistentes con el proyecto.
4. **Verifica** la orden obligatoria antes de dar por terminado:

```sh
cargo fmt --check
cargo clippy
cargo test
```

5. **Documenta** cualquier decisión con doc comments `///` y ejemplos.

## Comandos útiles

```sh
cargo build              # build de debug
cargo build --release    # build de release
cargo run                # ejecuta el binario
cargo test <name>        # test individual por substring
cargo clippy             # linters (debe pasar antes de commits)
cargo fmt                # formateo (debe pasar)
cargo doc --no-deps      # documentación local
```

## Reglas y anti-patterns (según AGENTS.md del proyecto)

- **No añadas dependencias a la ligera.** Prefiere `std` o las crates ya listadas
  en `Cargo.toml`. Evita `async-std`, `actix` y frameworks de agentes nicho.
- **Usa `Result` para errores** y propágalos correctamente. Justifica cada
  `unwrap()`/`expect()` — o conviértelos en manejo de errores apropiado.
- **Evita clonar** cuando un borrow (`&T`, `&mut T`) sea suficiente. Prefiere
  rebanadas sobre `Vec` cuando no necesites propiedad.
- **Async no bloqueante:** usa I/O async real (Tokio); no llames I/O síncrono
  bloqueante dentro del runtime async.
- Los tests unitarios van junto al código (`#[cfg(test)] mod tests`) y los de
  integración en `tests/`.
- Los tests de skill no deben acceder a la red salvo que estén marcados `#[ignore]`.

## Modo de uso

Pasa el `task` describiendo qué construir o arreglar, rutas relevantes y el
criterio de aceptación. El skill ejecutará los comandos y devolverá stdout+stderr.

### Ejemplo
task: |
  Implementa un módulo `crc32` en src/crc32.rs con cobertura de tests,
  luego valida con cargo fmt --check && cargo clippy && cargo test
