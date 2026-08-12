# Plan: Mejoras en manejo de capacidades MCP + tool descriptions

> **Histórico:** Este plan reemplaza al anterior de "Política de reintentos inteligente para LLM providers"
> (cuyos cambios ya están implementados en `src/agent/retry.rs`).

## Objetivo

Resolver 3 problemas detectados en la interacción con MCP servers:

1. **MCP client no verifica capacidades antes de llamar a `resources/list`** — Si un servidor
   no soporta resources (como codegraph), responde con "Method not found" y la tool falla.
2. **Tool descriptions sin info de capacidades** — Las tools `mcp_list_resources`, etc.
   no informan al LLM de que el servidor debe soportar resources.
3. **Comportamiento del agente** — El dev-manager debe usar codegraph de forma nativa.

---

## Tarea 1: Verificar capacidades en MCP client methods

### Archivos a modificar

- `src/mcp/client.rs`
- `src/mcp/client.rs` (tests al final del archivo)

### Cambios

#### `McpClient::list_resources()` (línea 254)

Antes de enviar `resources/list`, verificar:
```rust
if let Some(ref info) = self.info {
    if !info.capabilities.resources {
        return Ok(vec![]);
    }
}
```

#### `McpClient::list_resource_templates()` (línea 269)

Mismo check de `resources` capability. Si false → `Ok(vec![])`.

#### `McpClient::read_resource()` (línea 288)

Mismo check. Si false → `Err(Error::Mcp(...))` con mensaje claro.

#### Tests

Añadir tests unitarios en `#[cfg(test)] mod tests` al final de `client.rs`:
- `test_list_resources_returns_empty_when_not_supported()`
- `test_list_resource_templates_returns_empty_when_not_supported()`
- `test_read_resource_errors_when_not_supported()`

### Verificación

- `cargo test` (todos los tests pasan)
- `cargo clippy` (sin warnings nuevos)

---

## Tarea 2: Mejorar tool descriptions

### Archivos a modificar

- `src/tools/mcp.rs`

### Cambios

Actualizar `description` en:
- `mcp_list_resources_tool_definition()` — añadir "NOTE: only works if the server supports the resources capability."
- `mcp_list_resource_templates_tool_definition()` — idem
- `mcp_read_resource_tool_definition()` — idem

### Verificación

- `cargo build`
- `cargo clippy`

---

## Tarea 3: Auto-mejora del agente (comportamental)

- Usar `codegraph_codegraph_explore` en vez de múltiples `read()` + `glob()` para
  entender módulos completos.
- Usar `codegraph_codegraph_search` para encontrar símbolos rápidamente.
- Usar `codegraph_codegraph_impact` para analizar impacto de cambios.
- Usar `codegraph_codegraph_callers`/`codegraph_codegraph_callees` para entender flujos.

---

## Orden de ejecución

1. Tarea 1 (cambio de código + tests)
2. Tarea 2 (tool descriptions)
3. Tarea 3 (comportamental, no requiere código)

## Verificación final

- `cargo fmt --check && cargo clippy && cargo test`