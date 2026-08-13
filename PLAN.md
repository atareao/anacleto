# Plan: Subagentes auto-descriptivos (self-describing subagents)

> **Histórico:** Este plan reemplaza al anterior "Mejoras en manejo de capacidades MCP + tool descriptions"
> (ya implementado y completado: verificación de capacidades MCP en client.rs, tool descriptions
> mejoradas en tools/mcp.rs, y actualización comportamental de root.md y dev-manager.md).

## Objetivo

Cuando un usuario declara `subagents: [documenter]` en el frontmatter YAML de un agente, el agente
padre debe **saber automáticamente** qué hace cada subagente y **cuándo invocarlo**, sin necesidad de
editar el system prompt (cuerpo Markdown) del agente padre.

Hoy, `subagent_config_to_tool_definition` (src/agent/tools.rs:1123) genera una descripción genérica
("Delegate a task to the 'X' subagent for specialized work") que no incluye ni la `description` real
del subagente ni ninguna directriz de uso. El resultado: el agente padre no sabe cuándo usar sus
subagentes salvo que se le instruya manualmente en su Markdown.

### Decisiones de diseño confirmadas por el usuario

1. `when_to_use` es **texto libre** interpretado por el LLM del padre (no triggers estructurados del motor).
2. **NO** se inyecta el cuerpo Markdown completo del subagente — solo `description` + `when_to_use`.
3. `when_to_use` es **opcional** (`#[serde(default)]`), cero ruptura con configs existentes.

---

## Tarea 1: Añadir campo `when_to_use` a `AgentConfig`

### Archivo: `src/config/types.rs`

Añadir al struct `AgentConfig` (después de `description`):

```rust
/// Directrices de cuándo el agente padre debe invocar este subagente
/// (texto libre, inyectado automáticamente en el system prompt del padre).
#[serde(default)]
pub when_to_use: String,
```

## Tarea 2: Parsear `when_to_use` del frontmatter

### Archivo: `src/agent/loader.rs`

Añadir al struct `Frontmatter` (struct de deserialización local en `parse_agent`):

```rust
#[serde(default)]
when_to_use: String,
```

La construcción de `AgentConfig` ya asigna todos los campos del frontmatter; añadir la asignación.

## Tarea 3: Enriquecer la tool definition del subagente

### Archivo: `src/agent/tools.rs` — función `subagent_config_to_tool_definition`

Cambiar la descripción genérica por una que incluya `description` y, si no está vacío, `when_to_use`:

```rust
description: {
    let mut desc = format!(
        "Delegate a task to the '{}' subagent. What it does: {}",
        config.name, config.description
    );
    if !config.when_to_use.is_empty() {
        desc.push_str(&format!(" When to use: {}", config.when_to_use));
    }
    desc
}
```

El input_schema (campo `task`) se mantiene igual.

## Tarea 4: Auto-inyección en el system prompt del padre

### Archivo: `src/agent/lifecycle.rs` — función `spawn_agent`

Después de renderizar el system prompt (`render_template(&agent.description, &vars)`, ~línea 225),
si `subagent_configs` no está vacío, concatenar un bloque auto-generado:

```rust
// Auto-inyectar el bloque de subagentes: el padre descubre qué hacen y
// cuándo usarlos sin editar su Markdown.
if !subagent_configs.is_empty() {
    system_prompt.push_str("\n\n--- Subagents disponibles ---\n");
    for sc in &subagent_configs {
        system_prompt.push_str(&format!("• **{}** — {}\n", sc.name, sc.description));
        if !sc.when_to_use.is_empty() {
            system_prompt.push_str(&format!("  *Cómo usarlo*: {}\n", sc.when_to_use));
        }
    }
}
```

Además, añadir una variable de template `subagents` al HashMap `vars` (con el mismo bloque, sin
encabezado) para que un agente pueda posicionarlo explícitamente con `{subagents}` si lo desea.
NOTA: `render_template` deja las variables desconocidas como literal, así que los agentes que no
usen `{subagents}` no se ven afectados.

## Tarea 5: Tests

- `src/agent/loader.rs`: test de parseo de `when_to_use` en frontmatter (presente y ausente → vacío).
- `src/agent/tools.rs`: test de `subagent_config_to_tool_definition` con y sin `when_to_use`.
- `src/agent/lifecycle.rs` o donde mejor encaje: test del bloque auto-generado de subagentes.
- Actualizar los constructores literales de `AgentConfig` en tests existentes (loader.rs, etc.) que
  ahora requerirán el campo `when_to_use` (usar `String::new()` o `when_to_use: "".into()`).

## Archivos a modificar (resumen)

1. `src/config/types.rs` — campo `when_to_use` en `AgentConfig`
2. `src/agent/loader.rs` — frontmatter + test
3. `src/agent/tools.rs` — tool description enriquecida + test
4. `src/agent/lifecycle.rs` — bloque auto-generado + variable `{subagents}` + test

## Ejemplo de uso

`.agents/agents/documenter.md`:
```yaml
---
name: documenter
description: Documenta todas las acciones del agente
when_to_use: >
  Tras CADA ejecución de herramienta (tool call), delega al documenter
  un resumen de la acción realizada, con qué tool, qué resultado obtuvo
  y por qué la hizo.
---
Documenta cada acción que realiza el agente principal...
```

Al declarar `subagents: [documenter]` en el padre, este automáticamente ve el tool `documenter`
con descripción rica Y recibe la instrucción de delegarle tras cada tool call. Sin tocar su Markdown.

## Orden de ejecución

1. Tarea 1 (types.rs)
2. Tarea 2 (loader.rs)
3. Tarea 3 (tools.rs)
4. Tarea 4 (lifecycle.rs)
5. Tarea 5 (tests)

## Verificación final

- `cargo fmt --check && cargo clippy && cargo test`
