# Lanzamiento de múltiples subagentes del mismo tipo en paralelo + tipo/modo en TUI — Implementation Plan

## Objetivo

Permitir lanzar varios subagentes del **mismo tipo configurado** en paralelo (p. ej. 3 subagentes `reviewer` a la vez para revisar 3 archivos distintos), añadiendo un parámetro `agent` al tool `task` y ejecutando las múltiples llamadas `task` de un mismo turno de forma concurrente. Además, mostrar en la TUI el **tipo de subagente** (o `generic` si es dinámico) y el **modo de ejecución** (`fg`/`bg`).

## Estado actual (verificado)

- Rama: `development`. Cambios sin commitear (gitflow pendiente).
- `cargo fmt --all` ✅ limpio
- `cargo clippy` ✅ sin warnings
- `cargo test` ✅ **385 passed, 0 failed, 1 ignored** (356 lib + 4 + 19 + 5 + 1 doctest)
- Archivos modificados: `src/agent/tools.rs`, `src/agent/lifecycle.rs`, `src/engine/events.rs`, `src/tui/types.rs`, `src/tui/events.rs`, `src/tui/render.rs`, `src/tui/app.rs`, `src/tui/navigation.rs`, `PLAN.md`.

## Contexto / Estado actual (código)

- `src/agent/tools.rs`:
  - `TaskToolArgs` (~línea 986): campos `task_id`, `description`, `mode` (`TaskMode::Foreground/Background`), `model` (`Option<String>`), `tools` (`Vec<String>`), **`agent` (`Option<String>`)**. Método `parse(arguments: &str)`.
  - `task_tool_definition()` (~línea 1039): esquema JSON del tool `task` (properties: `task_id`, `description`, `mode`, `model`, `tools`, **`agent`**; required: `task_id`, `description`).
  - `execute_task_tool(...)` (~línea 1126): recibe `subagent_configs: &[AgentConfig]`; resuelve el tipo por nombre cuando `args.agent` está presente.
  - `spawn_subagent_and_delegate(cfg: SpawnSubagentConfig)` (~línea 1325): crea el subagente desde `AgentConfig`, resuelve provider, carga skills, ejecuta el bucle de tools del subagente y devuelve `String`. `SpawnSubagentConfig` tiene: `config`, `parent_id`, `llm_registry`, `task`, `db`, `session_id`, `event_tx`, `usage_tx`, `history_limit_percent`, `retry_config`, `debug`, `max_steps`, `depth`, `skills_override`, `permissions_override`, **`agent_type` (`Option<String>`)**, **`mode` (`TaskMode`)**.
  - `subagent_config_to_tool_definition(config: &AgentConfig)` (~línea 953): tool para subagentes configurados, esquema solo con campo `task`.
  - `resolve_provider_for_model(model, registry)`: `'/'`→openrouter, `'claude'`→anthropic, `'gpt'/'o1'/'o3'`→openai, else ollama.
  - `child_permissions = parent_permissions.intersection(&Permissions::default())`.
  - Límite de profundidad: `if depth >= subagent_depth { return Err(...) }`.

- `src/agent/lifecycle.rs`:
  - Bucle de ejecución de tools (~línea 545): `for tc in &tool_calls { ... }`. Dentro: `check_tool_permission`, `plan_mode_blocked`, dispatch por nombre (`task`, `todo`, `question`, `apply_patch`, `read`, `grep`, `glob`, `webfetch`, `websearch`, `mcp_*`, `skills`, `subagent_configs` por nombre, `plugins`). Al final inserta en `tool_store` y hace push a `conversation` con `LlmMessage` role `Tool`.
  - **Ejecución paralela** (~línea 832): las llamadas `task` de un mismo batch se ejecutan con `futures::future::join_all`, recogiendo resultados **en orden** (preservando el índice original). 0–1 llamadas `task` → secuencial.
  - Los subagentes configurados se invocan por nombre (~línea 722): `subagent_configs.iter().find(|c| c.name == tc.function.name)`, extrae `task` de args, llama `spawn_subagent_and_delegate` con `config.clone()`, `agent_type: Some(config.name.clone())`, `mode: TaskMode::Foreground` (~756).
  - `execute_task_tool` se llama en línea ~624 dentro del bucle, pasando `subagent_configs`.
  - `subagent_configs: Vec<AgentConfig>` está disponible en el scope del bucle (parte de `SpawnAgentConfig`).

- `TaskMode` enum en `src/agent/types.rs:186` (`Foreground`, `Background`).
- `AgentConfig` fields: `name`, `description`, `role`, `model`, `skills`, `mcps`, `permissions`, `subagents`, `system_prompt`, `max_steps`, `subagent_depth`.

- `src/engine/events.rs`: `EngineEvent::SubagentCreated` gana `agent_type: Option<String>` y `mode: TaskMode`.
- `src/tui/types.rs`: `AgentInfo` gana `agent_type: Option<String>` y `mode: Option<TaskMode>`.
- `src/tui/events.rs`: `handle_event` rellena los nuevos campos de `AgentInfo`; import `TaskMode` eliminado (no se usaba).
- `src/tui/render.rs`: `render_agent_panel` (~633), `build_agent_list_item` (~1133) y `render_subagent_tree` (~1206) muestran `[tipo]`/`[generic]` en cian y `(fg)`/`(bg)` en gris oscuro.

## Decisiones de diseño (confirmadas)

1. El tool `task` aceptará un nuevo parámetro `agent` (string opcional) que referencia un subagente **configurado** por nombre (ej. `"reviewer"`, `"writer"`). Cuando se especifica, el subagente lanzado se construye a partir de la config de ese tipo (hereda TODAS sus instrucciones: description/system_prompt, skills, mcps, modelo, permisos). Si no se especifica, mantiene el comportamiento dinámico actual.
2. Las MÚLTIPLES llamadas al tool `task` en un mismo turno deben ejecutarse **en paralelo** (con `futures::future::join_all`), no secuencialmente.
3. Foreground por defecto: el agente padre ESPERA todos los resultados y los recibe juntos para consolidarlos. Se mantiene `mode: background` como opción (lanzar y no esperar).
4. La TUI muestra el tipo de subagente (`[nombre]` o `[generic]` para dinámicos) y el modo de ejecución (`(fg)`/`(bg)`).

## Tareas

### Tarea 1: Añadir el campo `agent` a `TaskToolArgs` y su parseo

**Archivos:**
- Modificado: `src/agent/tools.rs:986-1038` (struct `TaskToolArgs` y método `parse`)

- [x] **Paso 1:** Añadir el campo `agent: Option<String>` a la struct `TaskToolArgs`.
- [x] **Paso 2:** En `parse(arguments: &str)`, parsear el campo `agent` como `Option<String>` (ausente → `None`).
- [x] **Paso 3:** Añadir un test unitario que verifique el parseo con `agent` presente y con `agent` ausente (líneas ~1926, ~1936).

### Tarea 2: Añadir `agent` al esquema JSON del tool `task`

**Archivos:**
- Modificado: `src/agent/tools.rs:1039-1125` (`task_tool_definition`)

- [x] **Paso 1:** Añadir la property `agent` al esquema JSON de `task_tool_definition()` como string opcional (no incluida en `required`).
- [x] **Paso 2:** Escribir una descripción clara: "Nombre del subagente configurado a lanzar (ej. 'reviewer'). Si se omite, se usa el comportamiento dinámico actual."

### Tarea 3: Añadir `subagent_configs` a `execute_task_tool` y resolver el tipo por nombre

**Archivos:**
- Modificado: `src/agent/tools.rs:1126-1324` (`execute_task_tool`)

- [x] **Paso 1:** Añadir el parámetro `subagent_configs: &[AgentConfig]` a la firma de `execute_task_tool`.
- [x] **Paso 2:** Cuando `args.agent` esté presente:
      - Buscar el config por nombre: `subagent_configs.iter().find(|c| c.name == agent_name)`.
      - Si no existe, devolver un error claro: `Err(format!("subagente configurado '{}' no encontrado", agent_name))` (con lista de disponibles).
      - Si existe, construir el `SpawnSubagentConfig` a partir de ESE config:
        - `config`: el `AgentConfig` encontrado (clonado).
        - `description`/`system_prompt`: usar el `system_prompt`/`description` del config.
        - `skills_override`: `Some(config.skills)` (skills del config).
        - `permissions_override`: `Some(parent_permissions.intersection(&config.permissions))` (permisos del config ∩ parent).
        - `model`: el modelo del config (resuelto vía `resolve_provider_for_model`).
        - `agent_type`: `Some(args.agent.clone())`.
        - Resto de campos: igual que el flujo dinámico actual (parent_id, llm_registry, task, db, session_id, event_tx, usage_tx, history_limit_percent, retry_config, debug, max_steps, depth).
- [x] **Paso 3:** Cuando `args.agent` sea `None`, mantener el comportamiento dinámico actual sin cambios (`agent_type: None`).
- [x] **Paso 4:** Añadir un test unitario para el caso de error cuando el tipo no existe en `subagent_configs`.

### Tarea 4: Ejecución paralela de múltiples llamadas `task` en el bucle de lifecycle

**Archivos:**
- Modificado: `src/agent/lifecycle.rs:545-720` (bucle de ejecución de tools)

- [x] **Paso 1:** En el bucle `for tc in &tool_calls`, detectar las llamadas cuyo nombre sea `task` (o que disparen `execute_task_tool`).
- [x] **Paso 2:** Si hay **más de una** llamada `task` en el mismo batch de `tool_calls`, ejecutarlas **en paralelo** usando `futures::future::join_all` (~línea 832).
- [x] **Paso 3:** Recoger los resultados **en orden** (preservando el índice original de cada tool call en el batch).
- [x] **Paso 4:** Mantener el resto de tools (no `task`) ejecutándose secuencialmente como hasta ahora.
- [x] **Paso 5:** Tras cada tool call (paralela o secuencial), insertar en `tool_store` y hacer push a `conversation` con `LlmMessage` role `Tool`, **preservando el orden** de los mensajes según el orden original de `tool_calls`.
- [x] **Paso 6:** Pasar `subagent_configs` a `execute_task_tool` en la llamada de la línea ~624.

### Tarea 5: Mostrar tipo y modo de ejecución en la TUI

**Archivos:**
- Modificado: `src/engine/events.rs`, `src/tui/types.rs`, `src/tui/events.rs`, `src/tui/render.rs`

- [x] **Paso 1:** `EngineEvent::SubagentCreated` gana `agent_type: Option<String>` y `mode: TaskMode`.
- [x] **Paso 2:** `AgentInfo` gana `agent_type: Option<String>` y `mode: Option<TaskMode>`.
- [x] **Paso 3:** `handle_event` en `src/tui/events.rs` rellena los nuevos campos de `AgentInfo`.
- [x] **Paso 4:** `render_agent_panel` (~633): muestra `[tipo]`/`[generic]` en cian y `(fg)`/`(bg)` en gris oscuro.
- [x] **Paso 5:** `build_agent_list_item` (~1133): muestra tipo y modo (con `agent_type.clone().unwrap_or_else(|| "generic".to_string())` para lifetime `'static`).
- [x] **Paso 6:** `render_subagent_tree` (~1206): muestra tipo y modo en cada subagente.
- [x] **Paso 7:** Eliminar el import `TaskMode` sin usar en `src/tui/events.rs`.

### Tarea 6: Tests y verificación

**Archivos:**
- Modificado: `src/agent/tools.rs` (tests unitarios), `src/agent/lifecycle.rs` (tests si aplica)

- [x] **Paso 1:** Unit tests de parseo del campo `agent` (presente/ausente) — ver Tarea 1.
- [x] **Paso 2:** Unit test del caso de error cuando el tipo no existe — ver Tarea 3.
- [x] **Paso 3:** Verificar que los tests existentes siguen pasando.

## Criterios de aceptación / Verificación

- [x] `cargo fmt --check` pasa sin cambios.
- [x] `cargo clippy` pasa sin warnings.
- [x] `cargo test` pasa (385 passed, 0 failed, 1 ignored) — incluidos los nuevos tests de parseo y error.
- [x] El tool `task` acepta `agent` opcional en su esquema JSON y lo parsea correctamente.
- [x] Con `agent` especificado, el subagente se construye desde la config del tipo (skills, mcps, modelo, permisos ∩ parent).
- [x] Con `agent` ausente, el comportamiento dinámico actual se mantiene intacto.
- [x] Múltiples llamadas `task` en un mismo turno se ejecutan en paralelo y sus resultados se consolidan en orden.
- [x] La TUI muestra el tipo de subagente (`[nombre]`/`[generic]`) y el modo (`(fg)`/`(bg)`).

## Pendiente

- [ ] Commitear ambas features vía gitflow (feature branch → PR a `development`). Nada está commiteado todavía.

## Riesgos y consideraciones

- **Orden de resultados:** Al ejecutar en paralelo, hay que preservar el orden original de `tool_calls` al insertar en `tool_store` y `conversation`, para no romper la coherencia del historial del LLM. ✅ Implementado con `join_all` + índices.
- **Límite de profundidad:** Cada subagente lanzado respeta `if depth >= subagent_depth { return Err(...) }`. Con varios en paralelo, todos comparten el mismo `depth`; no debe incrementarse por el número de subagentes.
- **Permisos:** Los subagentes configurados usan `parent_permissions.intersection(&config.permissions)`. Asegurarse de que el `permissions_override` se construye correctamente para el caso `agent`. ✅
- **Concurrencia y recursos:** Varios subagentes en paralelo consumen más conexiones LLM y memoria. Considerar si hace falta un límite de concurrencia máximo (fuera de alcance de este plan, pero a tener en cuenta).
- **`mode: background`:** Se mantiene como opción; en paralelo, los `background` no bloquean al padre, mientras que los `foreground` se esperan y consolidan juntos.
- **Compatibilidad:** El nuevo campo `agent` es opcional y no rompe llamadas existentes al tool `task`.
