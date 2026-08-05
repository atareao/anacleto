# Anacleto — Implementación de Features de OpenCode (v0.6.0)

## Objetivo

Portar el conjunto completo de features de OpenCode al motor de orquestación de agentes Anacleto (Rust, TUI ratatui+crossterm, persistencia SQLite vía sqlx), organizado en 7 fases incrementales con tareas atómicas, cambios a nivel de archivo y criterios de aceptación verificables.

## Alcance

### Features por fase

| # | Feature | Capa | Prioridad |
|---|---|---|---|
| **FASE 1 — Agente y orquestación** | | | |
| 1 | `task` tool — delegación dinámica de subagentes (foreground/background, `task_id`, `subagent_depth`, permisos derivados) | Backend | Alta |
| 2 | Subagentes en background + notificaciones de finalización (toast) | Backend + TUI | Alta |
| 3 | Plan Mode → Build handoff (agente solo-lectura → archivo markdown → agente build) | Backend | Media |
| 4 | Árbol de sesiones / navegación padre-hijo (fork con parentID) | Backend + DB + TUI | Media |
| **FASE 2 — Contexto y memoria** | | | |
| 5 | Compaction anclada con plantilla estructurada (`## Objective / Important Details / Work State / Next Move / Relevant Files`) | Backend + LLM | Alta |
| 6 | Truncado de salida de herramientas + tool-output store | Backend + TUI | Alta |
| 7 | Revert/fork basado en snapshots (git-tree content-addressed por turno) | Backend + Shell | Media |
| 8 | Contexto de sistema como fuentes tipadas refrescables (Source<A>, baseline/update delta) | Backend | Media |
| 9 | Archivos de instrucción (AGENTS.md, CLAUDE.md, CONTEXT.md) auto-descubiertos | Backend | Media |
| **FASE 3 — Herramientas y MCP** | | | |
| 10 | `todo` tool + lista de tareas persistida por sesión | Backend + DB + TUI | Alta |
| 11 | `question` tool (Q&A inline estructurado) | Backend + TUI | Media |
| 12 | `apply_patch` tool (batch add/update/delete, aprobación en lote, BOM/CRLF-aware) | Backend + Permissions | Alta |
| 13 | Herramientas estructuradas read/grep/glob/webfetch/websearch (schemas estrictos, paginación, ripgrep, permisos web) | Backend | Alta |
| 14 | Autorización de directorio externo (permiso separado fuera del workspace) | Backend + Permissions | Media |
| 15 | MCP resource tools (list/read resources & templates, mime binario) | Backend + MCP | Media |
| 16 | Integración LSP (language servers, diagnósticos al loop + TUI) | Backend + TUI | Baja |
| **FASE 4 — TUI/UX** | | | |
| 17 | Diff viewer completo (árbol de archivos, hunks, split/unified, git/branch/last-turn) | TUI | Media |
| 18 | Sistema which-key / leader-key (prefijo ctrl+x, key chording, ~100 keybindings) | TUI | Alta |
| 19 | Keymap totalmente rebindable en config | TUI + Config | Alta |
| 20 | Diálogos de modelo (favoritos ctrl+f, recientes f2, providers ctrl+a, ranking frecency) | TUI | Media |
| 21 | Sidebar de sesiones con pinning + quick slots (`<leader>1..9`) | TUI | Media |
| 22 | Gestor de prompts en cola (`<leader>q`) | TUI | Baja |
| 23 | Sistema de toasts/notificaciones | TUI | Media |
| 24 | Round-trip de editor externo (`<leader>e`) | TUI + Shell | Baja |
| **FASE 5 — Sesión y workflow** | | | |
| 25 | Mover sesión entre proyectos/directorios (re-homing) | Backend + DB | Media |
| 26 | Copiar transcripción al portapapeles / exportar a editor | TUI + Shell | Baja |
| 27 | Gestión de git worktrees | Backend + Shell | Baja |
| 28 | Review con diff vs branch / VCS diffs | Backend + TUI | Media |
| **FASE 5.5 — Cambio de agente activo** | | | |
| 29 | Cambio de agente activo (múltiples agentes root, selector `/agent`, enrutado al activo) | Backend + TUI | Alta |
| **FASE 6 — LLM y providers** | | | |
| 30 | Política de prompt-caching / cache-control breakpoints (cache:auto, buckets TTL, provider-aware) | LLM | Alta |
| 31 | Anthropic extended thinking (budget tokens) | LLM | Media |
| 32 | Plantillas de system-prompt por modelo/agente | LLM + Config | Media |
| 33 | Catálogo ampliado de providers (Bedrock, Azure, Google, etc.) | LLM | Baja |
| **FASE 7 — Extensibilidad** | | | |
| 34 | Sistema de plugins con hooks/transforms (agents, tools, commands) | Backend | Media |
| 35 | Comandos slash personalizados con templating de variables (`{env:VAR}`, `{file:path}`) | Backend + TUI | Media |
| 36 | Tools y providers personalizados en runtime | Backend | Baja |

### Fuera de alcance (explícitamente NO se implementa)

- **ACP (Agent Client Protocol)** — protocolo de cliente/servidor para agentes.
- **Servidor HTTP / API REST** — Anacleto es TUI-only por decisión de diseño (ADR).
- **Cloud console / dashboard web**.
- **Integración IDE** (VS Code, JetBrains, etc.).
- **Observabilidad / telemetría / tracing remoto**.
- **Auto-update / auto-instalación de binarios**.
- **Web UI o batch mode** (prohibido por AGENTS.md).

## Arquitectura / decisiones de diseño

Todas las features se integran en el modelo de flujo existente:

```
EngineCommand ──► Engine::run() (tokio::select! en command_rx + usage_rx) ──► handler ──► EngineEvent ──► TUI (app.rs)
```

### 1. `task` tool — nuevo tipo de ToolCall

El `task` tool se modela como un **nuevo variante del enum `ToolCall`** (o un `ToolCall::Task(TaskCall)`), no como un comando slash. El handler del engine intercepta el ToolCall `task` y, en lugar de ejecutar una herramienta local, invoca `spawn_agent` con un `SpawnAgentConfig` derivado del padre:

- **Foreground**: el subagente corre en el mismo turno; el resultado se devuelve como `ToolResult` al modelo.
- **Background**: se lanza un job async (tokio task) y se devuelve un `task_id`; al completar se emite `EngineEvent::SubagentFinished(task_id, summary)` que la TUI convierte en toast.
- **`task_id`**: si el modelo lo provee, se reanuda una sesión de subagente existente (cargando su historial vía `LoadHistory`).
- **`subagent_depth`**: contador en el contexto del agente; si excede el límite configurado, el `task` tool devuelve un error al modelo.
- **Permisos derivados**: el `AgentConfig` del hijo se construye intersectando los permisos del padre (deny del padre se propaga al hijo).

### 2. Background jobs

Se introduce un `JobRegistry` (HashMap<task_id, JoinHandle> + canal de resultados) en el engine. Los jobs de subagente en background no bloquean el loop principal; el resultado llega por un canal dedicado que se añade al `tokio::select!`. La TUI muestra un indicador de job activo y un toast al completar.

### 3. Plan Mode → Build handoff

Un agente con `permissions.deny` que bloquea todas las herramientas de escritura opera en "plan mode". Al aprobarse el plan (comando `/build` o confirmación), el engine:
1. Lee el archivo markdown de plan generado.
2. Crea/transiciona a un agente build con permisos de escritura.
3. Inyecta un mensaje sintético de ejecución (el contenido del plan) como `UserInput`/`System`.

### 4. Árbol de sesiones

Se añade columna `parent_id` a la tabla `sessions` (vía `ensure_column`). `/fork` crea una sesión hija con `parent_id = sesión actual`. La TUI navega padre↔hijo con un comando `/parent` y `/children`, mostrando la jerarquía en el sidebar.

### 5. Compaction anclada

La plantilla fija se define como constante Markdown. El resumen previo se **fusiona/actualiza** (no se regenera): se parsea el resumen existente por secciones y se reemplazan las secciones `Work State`, `Next Move`, `Relevant Files`; `Objective` e `Important Details` se conservan salvo cambio explícito. Config `session.compaction = { mode: auto|manual, buffer: tokens, keep: tokens }`. Dispara cuando `context_used > window − buffer`.

### 6. Truncado de salida de herramientas

En el handler de `ToolResult`, si la salida supera ~2000 chars, se trunca antes de enviarla al modelo y el contenido completo se guarda en un `ToolOutputStore` (mapa `tool_call_id → contenido`). La TUI colapsa la salida larga con un toggle para expandir.

### 7. Snapshots

Por cada turno de asistente se crea un snapshot content-addressed del árbol de archivos (hash del contenido → git-tree). Se guarda en `db` (tabla `snapshots`) con referencia al turno. `/revert` restaura archivos desde un snapshot previo; `/stage`, `/clear`, `/commit` operan sobre el snapshot actual.

### 8. System-context sources

Trait `Source<A>` con `baseline()` y `delta()`. El engine mantiene un registro de fuentes y su estado; solo las fuentes cuyo estado cambió se reinyectan en el siguiente turno (baseline se envía una vez, deltas después).

### 9. Archivos de instrucción

Descubrimiento automático: se buscan `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md` en el workspace y en el directorio global de config. Se inyectan como contexto de sistema por turnos (con cache-control en FASE 6).

### 10. `todo` tool

Nuevo ToolCall `todo` con operaciones `add/update/delete/list`. La lista se persiste por sesión en `db` (tabla `todos` con `session_id`). La TUI muestra un sidebar en vivo que se refresca con `EngineEvent::TodosUpdated`.

### 11. `question` tool

Nuevo ToolCall `question` que pausa el turno y emite `EngineEvent::Question(Question)` a la TUI. La TUI muestra un diálogo estructurado (opción múltiple, default recomendado, custom). La respuesta del usuario vuelve al engine como `EngineEvent::QuestionAnswer` y se inyecta como `ToolResult`.

### 12. `apply_patch` tool

Nuevo ToolCall `apply_patch` con formato de patch batch (add/update/delete). El engine agrupa los archivos y solicita **una sola aprobación de permisos en lote** antes de leer; luego aplica secuencialmente con manejo BOM/CRLF-aware (detectar y preservar encoding por archivo).

### 13. Herramientas estructuradas

Se definen schemas JSON estrictos (serde) para `read`, `grep`, `glob`, `webfetch`, `websearch`. Paginación: read devuelve máx 2000 líneas/50KB con offset. `grep` usa ripgrep. `webfetch`/`websearch` requieren permiso `net.http` (ya existe en el modelo de permisos).

### 14. Autorización de directorio externo

Nuevo permiso `fs.external` (o `dir.external`) separado del `fs.write` normal. Cualquier edición fuera del workspace requiere este permiso explícito.

### 15. MCP resource tools

Se exponen `mcp_list_resources` y `mcp_read_resource` como tools del modelo, delegando en `McpRegistry`. Manejo de mime binario: si el resource es binario, se devuelve base64 con metadatos de mime.

### 16. LSP

Integración opcional: se lanza un language server por lenguaje (config), se recogen diagnósticos y se emiten como `EngineEvent::Diagnostics` al loop del agente y a la TUI. Prioridad baja.

### 17-24. TUI/UX

- **Diff viewer**: nuevo componente `DiffViewer` con árbol de archivos, navegación de hunks, modos split/unified y fuentes git/branch/last-turn.
- **Which-key / leader-key**: prefijo `ctrl+x`; un `Keymap` central (HashMap<Vec<Key>, Action>) con chording descubrible; overlay which-key que muestra bindings disponibles.
- **Keymap rebindable**: `Keymap` se serializa en config YAML; `Config` gana campo `keymap`.
- **Diálogos de modelo**: popup con favoritos (ctrl+f), ciclo de recientes (f2), lista de providers (ctrl+a); ranking frecency (frecuencia × recencia) persistido en db.
- **Sidebar de sesiones**: pinning + quick slots `<leader>1..9`.
- **Gestor de prompts en cola**: cola de prompts pendientes con `<leader>q`.
- **Toasts**: cola de notificaciones en la TUI (jobs, subagentes, errores).
- **Editor round-trip**: `<leader>e` abre el editor externo (config `editor`), captura el buffer y lo envía como input.

### 25-28. Sesión y workflow

- **Re-homing**: `set_session_workspace` ya existe; se añade comando `/move` mejorado que re-homea la sesión a otro directorio y re-resuelve rutas.
- **Portapapeles/editor**: comando `/copy` (ya existe) ampliado + `/export-editor`.
- **Worktrees**: comandos `/worktree add|list|remove` delegando en git.
- **Review vs branch**: `/review` ampliado para diff vs branch/VCS.

### 29-32. LLM

- **Prompt-caching**: `cache:auto` inyecta `cache_control` en el último tool/system/user; buckets con TTL; provider-aware (Anthropic `cache_control`, OpenAI, etc.).
- **Extended thinking**: campo `thinking: { type: enabled, budget_tokens }` en la request Anthropic; se parsea el bloque thinking de la respuesta.
- **Plantillas de system-prompt**: `AgentConfig.system_prompt` puede ser una plantilla con variables (`{model}`, `{workspace}`, `{tools}`).
- **Catálogo de providers**: nuevos constructores en `LlmProviderRegistry` para Bedrock, Azure, Google.

### 33-35. Extensibilidad

- **Plugins**: trait `Plugin` con hooks (`on_agent_spawn`, `on_tool_call`, `on_command`, `on_event`) y transforms. Se cargan desde `~/.config/anacleto/plugins/`.
- **Comandos slash personalizados**: se mueve la lógica de `COMMANDS` en `src/tui/app.rs` a un registro dinámico; los comandos personalizados se definen en config con templating de variables (`{env:VAR}`, `{file:path}`).
- **Tools/providers personalizados**: registro en runtime vía plugins.

## Tareas por fase

> Convención: cada tarea es atómica y termina con `cargo fmt --check && cargo clippy && cargo test` en verde. Las dependencias entre fases se marcan explícitamente.

### FASE 1 — Agente y orquestación

#### Tarea 1.1: `task` tool — delegación dinámica de subagentes

**Archivos:**
- Modificar: `src/agent/types.rs` (enum `ToolCall` → añadir variante `Task`)
- Modificar: `src/agent/lifecycle.rs` (`spawn_agent`, `SpawnAgentConfig` → añadir `task_id`, `depth`, `permissions_derived`)
- Modificar: `src/engine/orchestrator.rs` (handler de ToolCall::Task)
- Modificar: `src/config/types.rs` (`AgentConfig` → añadir `subagent_depth`)

- [x] **Paso 1:** Añadir variante `Task(TaskCall)` al enum `ToolCall` con campos `{ task_id, description, mode: Foreground|Background, model, tools }`.
- [x] **Paso 2:** Extender `SpawnAgentConfig` con `task_id: Option<String>`, `depth: u32`, y `permissions: Permissions` derivadas.
- [x] **Paso 3:** En el handler del engine, interceptar `ToolCall::Task`; si `mode == Foreground`, spawn + esperar resultado y devolver `ToolResult`; si `Background`, registrar job y devolver `task_id`.
- [x] **Paso 4:** Implementar derivación de permisos: `child.permissions = parent.permissions ∩ child.permissions` (deny del padre se propaga).
- [x] **Paso 5:** Implementar límite `subagent_depth`: si `depth > config.subagent_depth`, devolver error al modelo.
- [x] **Paso 6:** Implementar reanudación por `task_id`: cargar historial de la sesión del subagente vía `LoadHistory`.

**Criterio de aceptación:** El modelo puede invocar `task` en foreground y background; el subagente hereda permisos restringidos del padre; `task_id` reanuda sesión existente; `subagent_depth` bloquea anidación excesiva.

#### Tarea 1.2: Subagentes en background + notificaciones de finalización

**Archivos:**
- Crear: `src/engine/jobs.rs` (JobRegistry)
- Modificar: `src/engine/orchestrator.rs` (registro de jobs, canal de resultados en `tokio::select!`)
- Modificar: `src/engine/mod.rs` (exponer `jobs`)
- Modificar: `src/tui/app.rs` (toast de finalización)

- [x] **Paso 1:** Crear `JobRegistry` con `HashMap<task_id, JoinHandle>` y canal `mpsc` de resultados.
- [x] **Paso 2:** Añadir el `rx` de resultados al `tokio::select!` del loop principal.
- [x] **Paso 3:** Al completar un job, emitir `EngineEvent::SubagentFinished(task_id, summary)`.
- [x] **Paso 4:** En la TUI, mostrar indicador de job activo y toast al completar.

**Criterio de aceptación:** Los subagentes background no bloquean el loop; la TUI muestra el estado del job y un toast al finalizar.

#### Tarea 1.3: Plan Mode → Build handoff

**Archivos:**
- Modificar: `src/engine/orchestrator.rs` (comando `/build`, transición plan→build)
- Modificar: `src/agent/types.rs` (modo plan/build en estado del agente)
- Modificar: `src/tui/app.rs` (comando `/build`)

- [x] **Paso 1:** Definir estado `plan`/`build` en el agente; en plan mode, todas las herramientas de escritura devuelven error.
- [x] **Paso 2:** Implementar `/build`: leer archivo markdown de plan, crear agente build con permisos de escritura.
- [x] **Paso 3:** Inyectar el contenido del plan como mensaje sintético de ejecución (`UserInput`/`System`).

**Criterio de aceptación:** Un agente plan solo-lectura genera un plan markdown; al aprobarse, un agente build lo ejecuta con el plan como contexto.

#### Tarea 1.4: Árbol de sesiones / navegación padre-hijo

**Archivos:**
- Modificar: `src/db/models.rs` (Session → `parent_id`)
- Modificar: `src/db/mod.rs` (`ensure_column` para `parent_id`, método `set_parent`)
- Modificar: `src/engine/orchestrator.rs` (fork con parentID, comandos `/parent`, `/children`)
- Modificar: `src/tui/app.rs` (comandos + navegación)

- [x] **Paso 1:** Añadir columna `parent_id` vía `ensure_column` y campo en `Session`.
- [x] **Paso 2:** En `/fork`, registrar `parent_id = sesión actual`.
- [x] **Paso 3:** Implementar `/parent` y `/children` para navegar la jerarquía.
- [x] **Paso 4:** Mostrar jerarquía en el sidebar de sesiones.

**Criterio de aceptación:** `/fork` crea sesión hija con parentID; se navega padre↔hijo; la jerarquía se persiste y se muestra en la TUI.

**Dependencia:** FASE 1 depende de la infraestructura de `spawn_agent` y `ToolCall` existentes (ya presentes). FASE 4 (TUI) consume los `EngineEvent` de FASE 1.

### FASE 2 — Contexto y memoria

#### Tarea 2.1: Compaction anclada con plantilla estructurada

**Archivos:**
- Crear: `src/engine/compaction.rs` (plantilla + fusión de resumen)
- Modificar: `src/config/types.rs` (`Config.session.compaction`)
- Modificar: `src/engine/orchestrator.rs` (disparo por umbral de contexto)
- Modificar: `src/llm/provider.rs` (exponer `window` del modelo)

- [x] **Paso 1:** Definir plantilla Markdown fija (`## Objective / Important Details / Work State / Next Move / Relevant Files`).
- [x] **Paso 2:** Implementar fusión: parsear resumen previo por secciones; actualizar `Work State`, `Next Move`, `Relevant Files`; conservar `Objective`/`Important Details` salvo cambio.
- [x] **Paso 3:** Config `compaction = { mode: auto|manual, buffer, keep }`.
- [x] **Paso 4:** Disparar cuando `context_used > window − buffer`; compactar manteniendo `keep` tokens.

**Criterio de aceptación:** La compactación actualiza (no regenera) el resumen anclado; respeta `buffer`/`keep`; se dispara automáticamente en modo `auto`.

#### Tarea 2.2: Truncado de salida de herramientas + tool-output store

**Archivos:**
- Crear: `src/engine/tool_output.rs` (ToolOutputStore)
- Modificar: `src/engine/orchestrator.rs` (truncado en handler de ToolResult)
- Modificar: `src/tui/app.rs` (colapso/expansión de salida larga)

- [x] **Paso 1:** Crear `ToolOutputStore` (mapa `tool_call_id → contenido completo`).
- [x] **Paso 2:** En el handler de `ToolResult`, truncar a ~2000 chars antes del modelo; guardar el completo en el store.
- [x] **Paso 3:** En la TUI, colapsar salidas largas con toggle para expandir (lee del store).

**Criterio de aceptación:** El modelo recibe salidas truncadas; el contenido completo está disponible en la TUI vía toggle.

#### Tarea 2.3: Revert/fork basado en snapshots

**Archivos:**
- Crear: `src/engine/snapshot.rs` (git-tree content-addressed)
- Modificar: `src/db/mod.rs` (tabla `snapshots`)
- Modificar: `src/engine/orchestrator.rs` (comandos `/revert`, `/stage`, `/clear`, `/commit`)
- Modificar: `src/tui/app.rs` (comandos)

- [x] **Paso 1:** Implementar snapshot content-addressed del árbol de archivos por turno de asistente.
- [x] **Paso 2:** Persistir snapshots en `db` con referencia al turno.
- [x] **Paso 3:** Implementar `/revert` (restaurar desde snapshot previo), `/stage`, `/clear`, `/commit`.

**Criterio de aceptación:** Cada turno de asistente genera un snapshot; `/revert` restaura archivos desde un snapshot previo; stage/clear/commit operan sobre el snapshot.

#### Tarea 2.4: Contexto de sistema como fuentes tipadas refrescables

**Archivos:**
- Crear: `src/engine/source.rs` (trait `Source<A>`)
- Modificar: `src/engine/orchestrator.rs` (registro de fuentes, baseline/delta)

- [x] **Paso 1:** Definir trait `Source<A>` con `baseline()` y `delta()`.
- [x] **Paso 2:** Mantener registro de fuentes y su estado; reinyectar solo las cambiadas.
- [x] **Paso 3:** Enviar baseline una vez; deltas en turnos siguientes.

**Criterio de aceptación:** Solo las fuentes cuyo estado cambió se reinyectan; el baseline se envía una vez.

#### Tarea 2.5: Archivos de instrucción (AGENTS.md, CLAUDE.md, CONTEXT.md)

**Archivos:**
- Crear: `src/engine/instructions.rs` (descubrimiento)
- Modificar: `src/engine/orchestrator.rs` (inyección por turnos)
- Modificar: `src/config/paths.rs` (rutas global + proyecto)

- [x] **Paso 1:** Descubrir `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md` en workspace y config global.
- [x] **Paso 2:** Inyectar como contexto de sistema por turnos (con cache-control en FASE 6).

**Criterio de aceptación:** Los archivos de instrucción se descubren automáticamente y se inyectan en el contexto.

**Dependencia:** FASE 2 depende de FASE 1 (orquestación estable). La Tarea 2.1 se integra con FASE 6 (cache-control).

### FASE 3 — Herramientas y MCP

#### Tarea 3.1: `todo` tool + lista de tareas persistida

**Archivos:**
- Modificar: `src/agent/types.rs` (variante `Todo` en `ToolCall`)
- Crear: `src/db/todos.rs` (tabla `todos`)
- Modificar: `src/engine/orchestrator.rs` (handler + `EngineEvent::TodosUpdated`)
- Modificar: `src/tui/app.rs` (sidebar en vivo)

- [x] **Paso 1:** Añadir variante `Todo` con operaciones `add/update/delete/list`.
- [x] **Paso 2:** Persistir lista por sesión en `db` (tabla `todos`).
- [x] **Paso 3:** Emitir `EngineEvent::TodosUpdated`; la TUI muestra sidebar en vivo.

**Criterio de aceptación:** El modelo gestiona una lista de tareas persistida por sesión; la TUI la muestra en vivo.

#### Tarea 3.2: `question` tool (Q&A inline)

**Archivos:**
- Modificar: `src/agent/types.rs` (variante `Question` en `ToolCall`)
- Modificar: `src/engine/orchestrator.rs` (pausa de turno, `EngineEvent::Question`, `EngineEvent::QuestionAnswer`)
- Modificar: `src/tui/app.rs` (diálogo estructurado)

- [x] **Paso 1:** Añadir variante `Question` con opciones múltiples, default recomendado, custom.
- [x] **Paso 2:** Pausar el turno y emitir `EngineEvent::Question` a la TUI.
- [x] **Paso 3:** La respuesta vuelve como `EngineEvent::QuestionAnswer` y se inyecta como `ToolResult`.

**Criterio de aceptación:** El agente puede preguntar al usuario a mitad de turno; la respuesta se inyecta al modelo.

#### Tarea 3.3: `apply_patch` tool

**Archivos:**
- Modificar: `src/agent/types.rs` (variante `ApplyPatch` en `ToolCall`)
- Crear: `src/engine/apply_patch.rs` (parser + aplicación BOM/CRLF-aware)
- Modificar: `src/permissions/checker.rs` (aprobación en lote)
- Modificar: `src/engine/orchestrator.rs` (handler)

- [x] **Paso 1:** Definir formato de patch batch (add/update/delete).
- [x] **Paso 2:** Agrupar archivos y solicitar una sola aprobación de permisos en lote antes de leer.
- [x] **Paso 3:** Aplicar secuencialmente con detección/preservación de BOM y CRLF.

**Criterio de aceptación:** `apply_patch` aplica cambios en lote con una sola aprobación; preserva BOM/CRLF por archivo.

#### Tarea 3.4: Herramientas estructuradas read/grep/glob/webfetch/websearch

**Archivos:**
- Crear: `src/tools/mod.rs` (schemas estrictos serde)
- Crear: `src/tools/read.rs`, `src/tools/grep.rs`, `src/tools/glob.rs`, `src/tools/web.rs`
- Modificar: `src/engine/orchestrator.rs` (registro de tools)
- Modificar: `src/permissions/checker.rs` (permiso `net.http` para web)

- [x] **Paso 1:** Definir schemas JSON estrictos para cada tool.
- [x] **Paso 2:** Implementar paginación en `read` (2000 líneas/50KB con offset).
- [x] **Paso 3:** Implementar `grep` con ripgrep.
- [x] **Paso 4:** Implementar `webfetch`/`websearch` con permiso `net.http`.

**Criterio de aceptación:** Las tools estructuradas validan schemas, pagan resultados y respetan permisos web.

#### Tarea 3.5: Autorización de directorio externo

**Archivos:**
- Modificar: `src/permissions/checker.rs` (permiso `fs.external`)
- Modificar: `src/config/types.rs` (permiso en schema)
- Modificar: `src/engine/orchestrator.rs` (chequeo en ediciones fuera del workspace)

- [x] **Paso 1:** Añadir permiso `fs.external` separado de `fs.write`.
- [x] **Paso 2:** Requerir este permiso para cualquier edición fuera del workspace.

**Criterio de aceptación:** Editar fuera del workspace requiere el permiso `fs.external` explícito.

#### Tarea 3.6: MCP resource tools

**Archivos:**
- Modificar: `src/mcp/client.rs` (exponer list/read resources)
- Modificar: `src/agent/types.rs` (variantes `McpListResources`, `McpReadResource`)
- Modificar: `src/engine/orchestrator.rs` (handler)

- [x] **Paso 1:** Exponer `mcp_list_resources` y `mcp_read_resource` como tools.
- [x] **Paso 2:** Manejar mime binario (base64 + metadatos).

**Criterio de aceptación:** El modelo puede listar y leer resources/templates MCP; los binarios se devuelven con mime.

#### Tarea 3.7: Integración LSP

**Archivos:**
- Crear: `src/lsp/mod.rs` (client LSP)
- Modificar: `src/engine/orchestrator.rs` (diagnósticos → `EngineEvent::Diagnostics`)
- Modificar: `src/tui/app.rs` (mostrar diagnósticos)

- [x] **Paso 1:** Lanzar language server por lenguaje (config).
- [x] **Paso 2:** Recoger diagnósticos y emitirlos al loop del agente y a la TUI.

**Criterio de aceptación:** Los diagnósticos LSP llegan al agente y se muestran en la TUI (opcional, prioridad baja).

**Dependencia:** FASE 3 depende de FASE 1 (orquestación) y de la infraestructura MCP existente. La Tarea 3.4 (web) requiere el permiso `net.http` ya presente.

### FASE 4 — TUI/UX

#### Tarea 4.1: Diff viewer completo

**Archivos:**
- Crear: `src/tui/diff_viewer.rs`
- Modificar: `src/tui/app.rs` (comando `/diff`, integración)

- [x] **Paso 1:** Implementar árbol de archivos y navegación de hunks.
- [x] **Paso 2:** Modos split/unified y fuentes git/branch/last-turn.

**Criterio de aceptación:** El diff viewer muestra árbol de archivos, hunks, split/unified y modos git/branch/last-turn.

#### Tarea 4.2: Sistema which-key / leader-key

**Archivos:**
- Crear: `src/tui/keymap.rs` (Keymap central)
- Crear: `src/tui/which_key.rs` (overlay)
- Modificar: `src/tui/app.rs` (prefijo `ctrl+x`, chording)

- [x] **Paso 1:** Definir `Keymap` (HashMap<Vec<Key>, Action>) con chording.
- [x] **Paso 2:** Implementar prefijo `ctrl+x` y overlay which-key descubrible.
- [x] **Paso 3:** Registrar ~100 keybindings.

**Criterio de aceptación:** El prefijo `ctrl+x` habilita key chording; el overlay which-key muestra bindings disponibles.

#### Tarea 4.3: Keymap totalmente rebindable en config

**Archivos:**
- Modificar: `src/config/types.rs` (campo `keymap`)
- Modificar: `src/config/loader.rs` (parseo)
- Modificar: `src/tui/keymap.rs` (carga desde config)

- [x] **Paso 1:** Serializar `Keymap` en config YAML.
- [x] **Paso 2:** Cargar y aplicar el keymap desde config al arrancar la TUI.

**Criterio de aceptación:** Todos los keybindings son rebindables vía config YAML.

#### Tarea 4.4: Diálogos de modelo

**Archivos:**
- Crear: `src/tui/model_picker.rs`
- Modificar: `src/db/mod.rs` (persistir frecency)
- Modificar: `src/tui/app.rs` (ctrl+f, f2, ctrl+a)

- [x] **Paso 1:** Implementar popup con favoritos (ctrl+f), recientes (f2), providers (ctrl+a).
- [x] **Paso 2:** Implementar ranking frecency (frecuencia × recencia) persistido.

**Criterio de aceptación:** Los diálogos de modelo muestran favoritos/recientes/providers con ranking frecency.

#### Tarea 4.5: Sidebar de sesiones con pinning + quick slots

**Archivos:**
- Modificar: `src/tui/app.rs` (sidebar)
- Modificar: `src/db/mod.rs` (persistir pin)

- [x] **Paso 1:** Implementar pinning de sesiones.
- [x] **Paso 2:** Implementar quick slots `<leader>1..9`.

**Criterio de aceptación:** Las sesiones se pueden fijar y acceder con `<leader>1..9`.

#### Tarea 4.6: Gestor de prompts en cola

**Archivos:**
- Modificar: `src/tui/app.rs` (cola de prompts, `<leader>q`)

- [x] **Paso 1:** Implementar cola de prompts pendientes y su gestión.

**Criterio de aceptación:** Los prompts se pueden encolar y gestionar con `<leader>q`.

#### Tarea 4.7: Sistema de toasts/notificaciones

**Archivos:**
- Crear: `src/tui/toast.rs`
- Modificar: `src/tui/app.rs` (cola de toasts)

- [x] **Paso 1:** Implementar cola de toasts (jobs, subagentes, errores).

**Criterio de aceptación:** Las notificaciones (jobs, subagentes, errores) se muestran como toasts.

#### Tarea 4.8: Round-trip de editor externo

**Archivos:**
- Modificar: `src/config/types.rs` (campo `editor`)
- Modificar: `src/tui/app.rs` (comando `<leader>e`)
- Modificar: `src/shell/mod.rs` (lanzar editor)

- [x] **Paso 1:** Configurar editor externo.
- [x] **Paso 2:** Implementar `<leader>e`: abrir editor, capturar buffer, enviar como input.

**Criterio de aceptación:** `<leader>e` abre el editor externo y el contenido editado se envía como input.

**Dependencia:** FASE 4 depende de FASE 1 (EngineEvent) y de la infraestructura TUI existente. La Tarea 4.2/4.3 (keymap) es prerrequisito de 4.5/4.6/4.8 (que usan `<leader>`).

### FASE 5 — Sesión y workflow

#### Tarea 5.1: Mover sesión entre proyectos (re-homing)

**Archivos:**
- Modificar: `src/db/mod.rs` (`set_session_workspace` ampliado)
- Modificar: `src/engine/orchestrator.rs` (comando `/move` mejorado)
- Modificar: `src/tui/app.rs` (comando)

- [x] **Paso 1:** Ampliar re-homing: re-resolver rutas al mover la sesión a otro directorio.

**Criterio de aceptación:** Una sesión se mueve a otro directorio y sus rutas se re-resuelven.

#### Tarea 5.2: Copiar transcripción / exportar a editor

**Archivos:**
- Modificar: `src/tui/app.rs` (comando `/copy` ampliado, `/export-editor`)
- Modificar: `src/shell/mod.rs` (portapapeles)

- [x] **Paso 1:** Ampliar `/copy` para copiar la transcripción al portapapeles.
- [x] **Paso 2:** Implementar `/export-editor` para exportar a editor externo.

**Criterio de aceptación:** La transcripción se copia al portapapeles o se exporta al editor.

#### Tarea 5.3: Gestión de git worktrees

**Archivos:**
- Modificar: `src/shell/mod.rs` (comandos git)
- Modificar: `src/engine/orchestrator.rs` (comandos `/worktree add|list|remove`)
- Modificar: `src/tui/app.rs` (comandos)

- [x] **Paso 1:** Implementar `/worktree add|list|remove` delegando en git.

**Criterio de aceptación:** Los worktrees se gestionan desde la TUI.

#### Tarea 5.4: Review con diff vs branch / VCS diffs

**Archivos:**
- Modificar: `src/engine/orchestrator.rs` (comando `/review` ampliado)
- Modificar: `src/tui/diff_viewer.rs` (fuente branch/VCS)

- [x] **Paso 1:** Ampliar `/review` para diff vs branch/VCS.
- [x] **Paso 2:** Integrar con el diff viewer (fuente branch).

**Criterio de aceptación:** `/review` muestra diffs vs branch/VCS en el diff viewer.

**Dependencia:** FASE 5 depende de FASE 4 (diff viewer) y de la infraestructura de sesiones existente.

### FASE 5.5 — Cambio de agente activo

**Objetivo:** Permitir seleccionar cuál de los agentes root (ya spawneados por `initialize()`) recibe el input del usuario, enrutando input, modelo y respawn al agente activo en lugar de fijarse siempre en el primer root.

#### Tarea 5.5.1: Enrutado al agente activo

**Archivos:**
- Modificar: `src/engine/orchestrator.rs` (campo `active_agent`, enrutado de input/modelo, renombrado de helpers)

- [x] **Paso 1:** Añadir campo `active_agent: String` a `Engine` y inicializarlo en `Engine::new()` con el nombre del primer agente `role == AgentRole::Root` (misma lógica que `current_model`).
- [x] **Paso 2:** En `handle_user_input` (~línea 768), enrutar a `self.active_agent` en lugar de `root_agent_config()?.name`.
- [x] **Paso 3:** En `handle_set_model` (~línea 790), buscar el config del agente activo por `a.name == self.active_agent`, actualizar su modelo y llamar al respawn del agente activo.
- [x] **Paso 4:** Renombrar `respawn_root_agent` → `respawn_active_agent` y `send_to_root` → `send_to_active`, usando `self.active_agent` en lugar de `root_agent_config()?.name`. Actualizar todos los callers (líneas 678 Compact, 913 ClearHistory, 959 LoadHistory, 1074 LoadHistory en `reload_history_to_root`, 1410 UserInput en `handle_review`).
- [x] **Paso 5:** En `reload_history_to_root` (~línea 1071), cambiar el guard `if self.root_agent_config().is_err()` por `if !self.agents.contains_key(&self.active_agent)`.

**Criterio de aceptación:** El input del usuario, el cambio de modelo y el respawn se dirigen al agente activo, no siempre al primer root. `cargo build` compila sin referencias a `respawn_root_agent`/`send_to_root`/`root_agent_config` en los callers migrados.

#### Tarea 5.5.2: Comando de cambio de agente en la TUI

**Archivos:**
- Modificar: `src/engine/orchestrator.rs` (`EngineCommand::SwitchAgent`, `EngineEvent::AgentSwitched`, handler, dispatch en `run()`)
- Modificar: `src/tui/app.rs` (campo `active_agent`, manejo de evento, comando `/agent`, `COMMANDS`, barra de estado)

- [x] **Paso 1:** Añadir variante `EngineCommand::SwitchAgent(String)` al enum `EngineCommand` (línea 310) y `EngineEvent::AgentSwitched { name: String }` al enum `EngineEvent` (inicio del archivo).
- [x] **Paso 2:** Implementar `handle_switch_agent(name)`: validar que el agente existe en `self.agents` y es root; si no, emitir error; si sí, actualizar `self.active_agent` y `self.current_model` con el modelo del agente, y emitir `AgentSwitched { name }` + `ModelChanged`.
- [x] **Paso 3:** Añadir el dispatch de `EngineCommand::SwitchAgent` en el `match` de `run()` (~línea 625) llamando a `handle_switch_agent`.
- [x] **Paso 4:** Añadir campo `active_agent: String` a `App` y manejarlo en `handle_event` (~línea 405) cuando llegue `EngineEvent::AgentSwitched { name }`.
- [x] **Paso 5:** Añadir el comando `/agent <nombre>` en `handle_command` (línea 1279) que envía `EngineCommand::SwitchAgent(nombre)`; añadir `/agent` a la const `COMMANDS` (línea 35) para autocompletado.
- [x] **Paso 6:** Mostrar el agente activo en `render_status_bar` (línea 1931) junto a `current_model`.

**Criterio de aceptación:** `/agent <nombre>` cambia el agente root activo, el indicador de la barra de estado se actualiza, y `/agents` sigue listando los disponibles. `cargo fmt --check && cargo clippy && cargo test` pasan.

### FASE 6 — LLM y providers

> **Nota de rutas:** Todo el código de providers vive en `src/llm/provider.rs` (1758 líneas). **NO existe `src/llm/anthropic.rs`** — los tipos Anthropic (`AnthropicRequest`, `AnthropicMessage`, `AnthropicResponse`, `AnthropicContentBlock`, `AnthropicUsage`) están definidos en `provider.rs` (líneas 185-239). El trait `LlmProvider` (línea ~20) y la factory `create_provider(config)` (línea ~44) también residen allí. `LlmProviderConfig` y `LlmProviderType` están en `src/llm/types.rs` (líneas 88 y 97).

#### Tarea 6.1: Política de prompt-caching / cache-control breakpoints

**Archivos:**
- Modificar: `src/config/types.rs` (config `cache: auto|off` en `ModelsConfig`, línea 55)
- Modificar: `src/llm/types.rs` (campo `cache_control` en `LlmRequest`, línea 51)
- Modificar: `src/llm/provider.rs` (inyección provider-aware en `AnthropicProvider::complete`, línea 1234)
- Modificar: `src/engine/orchestrator.rs` (propagación de `CacheControl` en `provider_config_to_llm`, línea 1938, y `ollama_config_to_llm`, línea 1951)

- [x] **Paso 1:** Añadir `cache: auto|off` a `ModelsConfig` en `src/config/types.rs` (junto a `anthropic`/`openai`/`openrouter`/`ollama`, línea 55) y parsearlo en `src/config/loader.rs` (`load_config`, línea 11).
- [x] **Paso 2:** Añadir campo `cache_control: Option<CacheControl>` a `LlmRequest` en `src/llm/types.rs` (línea 51).
- [x] **Paso 3:** En `src/llm/provider.rs`, implementar inyección provider-aware: para Anthropic, añadir `cache_control: { type: "ephemeral" }` a nivel top del request (automatic caching) en `AnthropicProvider::complete` (línea 1234); para OpenAI, delegar en el caching automático del proveedor (sin campo explícito).
- [x] **Paso 4:** En `src/engine/orchestrator.rs`, propagar el `CacheControl` derivado de `models.cache.mode` en `provider_config_to_llm` (línea 1938) y `ollama_config_to_llm` (línea 1951) al construir `LlmProviderConfig`.

**Criterio de aceptación:** `cache:auto` inyecta `cache_control` a nivel top del request Anthropic (automatic caching); es provider-aware (Anthropic explícito, OpenAI automático). Nota: la inyección a nivel de mensaje se descartó porque `cache_control` como campo hermano de `content: String` es un formato inválido para la API de Anthropic; el automatic caching top-level cubre el caso de uso.

#### Tarea 6.2: Anthropic extended thinking

**Archivos:**
- Modificar: `src/llm/provider.rs` (campo `thinking` en `AnthropicRequest`, línea ~185; parseo del bloque `thinking` en `AnthropicContentBlock`, línea ~239, y en `AnthropicProvider::complete`, línea 1234)

- [x] **Paso 1:** Añadir campo `thinking: Option<AnthropicThinking>` a `AnthropicRequest` (línea ~185) con `{ type: "enabled", budget_tokens: u32 }`.
- [x] **Paso 2:** Añadir variante `thinking` a `AnthropicContentBlock` (línea ~239) y parsear el bloque `thinking` de la respuesta en `AnthropicProvider::complete` (línea 1234), exponiéndolo en `LlmResponse`/`LlmStreamChunk` de `src/llm/types.rs`.

**Criterio de aceptación:** El modelo Anthropic recibe `budget_tokens` en la request y el bloque `thinking` de la respuesta se parsea correctamente y se expone al consumidor.

#### Tarea 6.3: Plantillas de system-prompt por modelo/agente

**Archivos:**
- Crear: `src/llm/template.rs` (renderizado de variables `{model}`, `{workspace}`, `{tools}`)
- Modificar: `src/config/types.rs` (`AgentConfig.system_prompt` como plantilla, línea 279)
- Modificar: `src/agent/lifecycle.rs` (renderizado al construir el contexto, líneas 160 y 218-238)

- [x] **Paso 1:** Crear `src/llm/template.rs` con una función de renderizado que sustituya `{model}`, `{workspace}` y `{tools}` en una plantilla.
- [x] **Paso 2:** En `src/config/types.rs`, documentar que `AgentConfig.system_prompt` (línea 279) puede ser una plantilla con esas variables.
- [x] **Paso 3:** En `src/agent/lifecycle.rs`, renderizar la plantilla al construir el system prompt (línea 160) y al inyectar los archivos de instrucción del workspace como `MessageRole::System` (líneas 227-238), antes de construir `LlmRequest` (líneas 322, 2207, 2553).

**Criterio de aceptación:** El system-prompt se renderiza con variables por modelo/agente y se inyecta como `MessageRole::System` en el contexto.

#### Tarea 6.4: Catálogo ampliado de providers

**Archivos:**
- Crear: `src/llm/bedrock.rs`, `src/llm/azure.rs`, `src/llm/google.rs`
- Modificar: `src/llm/types.rs` (variantes `Bedrock`/`Azure`/`Google` en `LlmProviderType`, línea 88)
- Modificar: `src/llm/provider.rs` (constructores en `create_provider`, línea ~44, y registro en `LlmProviderRegistry`, línea ~1106)
- Modificar: `src/config/types.rs` (campos `bedrock`/`azure`/`google` en `ModelsConfig`, línea 55)
- Modificar: `src/config/loader.rs` (parseo de los nuevos campos en `load_config`, línea 11)

- [x] **Paso 1:** Añadir variantes `Bedrock`, `Azure`, `Google` a `LlmProviderType` en `src/llm/types.rs` (línea 88).
- [x] **Paso 2:** Crear `src/llm/bedrock.rs`, `src/llm/azure.rs`, `src/llm/google.rs` con sus respectivos providers implementando el trait `LlmProvider`.
- [x] **Paso 3:** Añadir los constructores al `match` de `create_provider` en `src/llm/provider.rs` (línea ~44) y registrarlos en `LlmProviderRegistry` (línea ~1106).
- [x] **Paso 4:** Añadir campos `bedrock`/`azure`/`google` a `ModelsConfig` en `src/config/types.rs` (línea 55) y parsearlos en `src/config/loader.rs` (`load_config`, línea 11).

**Criterio de aceptación:** Los nuevos providers (Bedrock, Azure, Google) se registran en `create_provider`/`LlmProviderRegistry`, se seleccionan desde config y se construyen vía `provider_config_to_llm` en `src/engine/orchestrator.rs`.

**Dependencia:** FASE 6 depende de FASE 2 (compaction/contexto) para los breakpoints de cache.

### FASE 7 — Extensibilidad

> **Nota de rutas:** Los comandos slash se definen en la const `COMMANDS` en `src/tui/app.rs` (línea 36) y se despachan en `handle_command` (línea 1523) con un `match` sobre el primer token; la paleta fuzzy usa `COMMANDS` (líneas 401, 1330, 1415, 3537). El engine despacha `EngineCommand` en `Engine::run()` (línea 712) y `initialize()` (línea 497) registra providers y spawnea agentes. Las tools se construyen en `src/agent/lifecycle.rs` (línea 163) y se despachan por nombre en el handler de tool calls (líneas 554-743). El directorio global de config es `~/.config/anacleto/` (ver `src/agent/loader.rs:183` y `src/config/loader.rs:51`).

#### Tarea 7.1: Sistema de plugins con hooks/transforms

**Archivos:**
- Crear: `src/plugin/mod.rs` (trait `Plugin` + `PluginRegistry`)
- Modificar: `src/engine/orchestrator.rs` (invocación de hooks en `initialize()` línea 497, `run()` línea 712, y en el handler de tool calls)
- Modificar: `src/config/paths.rs` (directorio de plugins global `~/.config/anacleto/plugins/`)

- [x] **Paso 1:** Definir trait `Plugin` con hooks (`on_agent_spawn`, `on_tool_call`, `on_command`, `on_event`) y transforms.
- [x] **Paso 2:** Cargar plugins desde `~/.config/anacleto/plugins/` (cada plugin como un módulo Rust compilado o un archivo de definición).
- [x] **Paso 3:** Invocar hooks en los puntos del engine (`initialize()`, `run()`, handler de tool calls).

**Criterio de aceptación:** Los plugins se cargan y sus hooks/transforms se invocan en los puntos definidos.

#### Tarea 7.2: Comandos slash personalizados con templating

**Archivos:**
- Modificar: `src/tui/app.rs` (mover `COMMANDS` a registro dinámico, líneas 36, 401, 1330, 1415, 3537)
- Modificar: `src/config/types.rs` (campo `commands` en `Config`, línea 9)
- Modificar: `src/config/loader.rs` (parseo en `load_config`, línea 11)
- Crear: `src/engine/template.rs` (variables `{env:VAR}`, `{file:path}`)

- [x] **Paso 1:** Mover la lógica de `COMMANDS` a un registro dinámico (built-ins + personalizados).
- [x] **Paso 2:** Definir comandos personalizados en config con templating de variables (`{env:VAR}`, `{file:path}`).

**Criterio de aceptación:** Los comandos slash personalizados se definen en config y expanden `{env:VAR}`/`{file:path}`.

#### Tarea 7.3: Tools y providers personalizados en runtime

**Archivos:**
- Modificar: `src/plugin/mod.rs` (registro de tools/providers)
- Modificar: `src/engine/orchestrator.rs` (registro en runtime en `initialize()` línea 497)
- Modificar: `src/agent/lifecycle.rs` (despacho de tools personalizados en el handler, líneas 554-743)

- [x] **Paso 1:** Permitir que los plugins registren tools y providers en runtime.

**Criterio de aceptación:** Los plugins pueden registrar tools y providers personalizados en runtime.

**Dependencia:** FASE 7 depende de FASE 1 (orquestación) y de FASE 3 (tools). La Tarea 7.2 depende de la infraestructura de comandos existente.

## Orden de implementación y dependencias (ruta crítica)

```
FASE 1 (orquestación) ──► FASE 2 (contexto/memoria) ──► FASE 3 (tools/MCP) ──► FASE 6 (LLM)
        │                        │                            │
        └──────────► FASE 4 (TUI/UX) ◄────────────────────────┘
                          │
                          └──► FASE 5 (sesión/workflow)
                                     │
                                     └──► FASE 7 (extensibilidad)
```

**Ruta crítica recomendada:**
1. **FASE 1** primero (base de orquestación: `task` tool, background jobs, plan/build, árbol de sesiones). Sin esto, nada downstream funciona.
2. **FASE 2** (contexto/memoria) en paralelo conceptual con FASE 1, pero implementar tras estabilizar FASE 1.
3. **FASE 3** (tools/MCP) — depende de FASE 1.
4. **FASE 6** (LLM) — depende de FASE 2 para cache breakpoints.
5. **FASE 4** (TUI/UX) — consume los `EngineEvent` de FASE 1/2/3; el keymap (4.2/4.3) es prerrequisito de 4.5/4.6/4.8.
6. **FASE 5** (sesión/workflow) — depende de FASE 4 (diff viewer).
7. **FASE 7** (extensibilidad) — al final, sobre infraestructura estable.

**Prioridad de negocio (si hay que recortar):** FASE 1 (task tool, background) > FASE 3 (apply_patch, tools estructuradas) > FASE 2 (compaction, truncado) > FASE 6 (cache) > FASE 4 (keymap, toasts) > FASE 5 > FASE 7.

## Criterios de aceptación finales

### Verificación de calidad (obligatoria en cada commit)

- [ ] `cargo fmt --check` pasa sin cambios.
- [ ] `cargo clippy` pasa sin warnings.
- [ ] `cargo test` pasa (unit + integration).
- [ ] No se añaden dependencias fuera de las permitidas (std, tokio, ratatui, crossterm, serde, serde_yaml, sqlx, reqwest, tower, anyhow).
- [ ] No se introduce web UI ni batch mode (TUI-only se mantiene).
- [ ] Los subagentes no heredan nada del padre salvo lo especificado (permisos derivados en task tool).
- [ ] Los MCP server paths provienen de config, no hardcodeados.

### Verificación E2E manual por feature

**FASE 1**
- [ ] El modelo invoca `task` en foreground y background; el subagente responde.
- [ ] Un subagente background muestra toast al completar sin bloquear el loop.
- [ ] Un agente plan solo-lectura genera un plan; `/build` lo ejecuta.
- [ ] `/fork` crea sesión hija; `/parent`/`/children` navegan la jerarquía.

**FASE 2**
- [ ] La compactación actualiza el resumen anclado sin regenerarlo; respeta buffer/keep.
- [ ] Las salidas largas de herramientas se truncan para el modelo y se expanden en la TUI.
- [ ] `/revert` restaura archivos desde un snapshot previo.
- [ ] Solo las fuentes de sistema cambiadas se reinyectan.
- [ ] AGENTS.md/CLAUDE.md/CONTEXT.md se descubren e inyectan.

**FASE 3**
- [x] El modelo gestiona la lista de tareas; el sidebar se actualiza en vivo.
- [x] El agente pregunta al usuario a mitad de turno y usa la respuesta.
- [x] `apply_patch` aplica en lote con una aprobación; preserva BOM/CRLF.
- [x] read/grep/glob/webfetch/websearch validan schemas y pagan resultados.
- [x] Editar fuera del workspace requiere `fs.external`.
- [x] El modelo lista/lee resources MCP; binarios con mime.
- [x] Los diagnósticos LSP llegan al agente y a la TUI.

**FASE 4**
- [x] El diff viewer muestra árbol, hunks, split/unified, git/branch/last-turn.
- [x] `ctrl+x` habilita key chording; el overlay which-key es descubrible.
- [x] Todos los keybindings son rebindables en config.
- [x] Los diálogos de modelo muestran favoritos/recientes/providers con frecency.
- [x] Las sesiones se fijan y se accede con `<leader>1..9`.
- [x] Los prompts se encolan con `<leader>q`.
- [x] Los toasts muestran jobs/subagentes/errores.
- [x] `<leader>e` hace round-trip con el editor externo.

**FASE 5**
- [x] Una sesión se mueve a otro directorio y re-resuelve rutas.
- [x] La transcripción se copia al portapapeles o se exporta al editor.
- [x] Los worktrees se gestionan desde la TUI.
- [x] `/review` muestra diffs vs branch/VCS.

**FASE 5.5**
- [x] El input del usuario llega al agente activo, no siempre al primer root.
- [x] `/agent <nombre>` cambia de agente root y el indicador se actualiza.

**FASE 6**
- [x] `cache:auto` inyecta cache_control a nivel top del request Anthropic (automatic caching).
- [x] Anthropic extended thinking recibe budget_tokens y se parsea.
- [x] El system-prompt se renderiza con variables por modelo/agente.
- [x] Bedrock/Azure/Google se registran y seleccionan desde config.

**FASE 7**
- [x] Los plugins se cargan y sus hooks/transforms se invocan.
- [x] Los comandos slash personalizados expanden `{env:VAR}`/`{file:path}`.
- [x] Los plugins registran tools/providers en runtime.

### Cierre de la release v0.7.0

- [x] Fases 1-7 (incl. 5.5) completadas y verificadas.
- [x] `cargo doc --no-deps` genera sin errores.
- [x] Documentación de nuevas config (keymap, compaction, plugins, providers) actualizada.
- [x] Rama `develop` con commits atómicos por tarea, cada uno pasando fmt/clippy/test.

### FASE 8 — Navegación por ventanas (focus)

**Objetivo:** Introducir un modelo de foco de 5 ventanas en la TUI: (1) Chat, (2) MCPs, (3) Skills, (4) Agents, (5) Input. Cambio de ventana con Alt+1..Alt+5. Cada ventana tiene su propia navegación: Input con atajos de shell para mover el cursor dentro de la caja de texto; Chat, MCPs, Skills y Agents con flechas de cursor y atajos de Vim.

#### Tarea 8.1: Enum Focus y campo de estado en App

**Archivos:**
- Modificar: `src/tui/app.rs` (enum `Focus`, campos de estado en `App` y `App::new`)

- [ ] **Paso 1:** Añadir `enum Focus { Chat, Mcps, Skills, Agents, Input }` en `src/tui/app.rs`.
- [ ] **Paso 2:** Añadir campo `focus: Focus` en `App`, inicializado a `Focus::Input` en `App::new`.
- [ ] **Paso 3:** Añadir campo `input_cursor: usize` (índice de carácter dentro de `input`) para edición de shell.
- [ ] **Paso 4:** Añadir índices de selección para los paneles del sidebar: `mcp_panel_index`, `skill_panel_index`, `agent_panel_index` (inicializados a 0).

**Criterio de aceptación:** `App` expone `focus`, `input_cursor` y los tres índices de panel, todos con valores iniciales correctos.

#### Tarea 8.2: Acciones de foco y bindings Alt+1..Alt+5 en keymap

**Archivos:**
- Modificar: `src/tui/keymap.rs` (variantes de `Action`, bindings en `Keymap::default()`, `format_keymap_table()`, `parse_action()`, tests)

- [ ] **Paso 1:** Añadir variantes `Action::FocusChat`, `Action::FocusMcps`, `Action::FocusSkills`, `Action::FocusAgents`, `Action::FocusInput` al enum `Action`.
- [ ] **Paso 2:** En `Keymap::default()`, enlazarlas a Alt+1, Alt+2, Alt+3, Alt+4, Alt+5 (KeyEvent con `KeyModifiers::ALT`).
- [ ] **Paso 3:** Añadirlas a `format_keymap_table()` (filas con descripción) y a la lista de `parse_action()`.
- [ ] **Paso 4:** Añadir tests unitarios para los nuevos bindings (p.ej. `km.matches(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT), Action::FocusChat)`).

**Criterio de aceptación:** Alt+1..Alt+5 resuelven a las acciones de foco; aparecen en la tabla which-key y en `parse_action`; los tests unitarios pasan.

#### Tarea 8.3: Reestructurar handle_key para enrutar por foco

**Archivos:**
- Modificar: `src/tui/app.rs` (`App::handle_key`, línea 921)

- [ ] **Paso 1:** Tras los checks de overlays (which_key, approval, question, init_flow, timeline, mcps list, model picker, diff viewer, prompt queue), añadir el cambio de foco con Alt+1..5 (siempre disponible).
- [ ] **Paso 2:** Mantener las acciones globales del keymap (Quit, OpenWhichKey, ToggleSidebar, ToggleDiffViewer, OpenModelPicker, OpenEditor, OpenPromptQueue, QuickSlots).
- [ ] **Paso 3:** Añadir enrutado por `self.focus` a los métodos `handle_input_key`, `handle_chat_key`, `handle_mcp_panel_key`, `handle_skill_panel_key`, `handle_agent_panel_key`.
- [ ] **Paso 4:** Mover el manejo de ScrollUp/ScrollDown/PageUp/PageDown y ClearInput fuera de la sección global (pasan a los handlers de foco).

**Criterio de aceptación:** `handle_key` cambia de foco con Alt+1..5, ejecuta acciones globales y delega el resto al handler de la ventana enfocada.

#### Tarea 8.4: Edición de shell en Input (5)

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_input_key`, helpers de cursor, `render_input` línea 3759)

- [ ] **Paso 1:** Implementar `handle_input_key` con cursor editable: Left/Right mueven el cursor un carácter; Home/End inicio/fin de línea; Ctrl+A / Ctrl+E inicio/fin; Ctrl+W borrar palabra hacia atrás; Ctrl+U borrar hasta el inicio; Ctrl+K borrar hasta el final; Alt+Left / Ctrl+Left y Alt+Right / Ctrl+Right mover por palabra; Backspace borrar carácter anterior al cursor; Delete borrar carácter en el cursor; Up/Down historial de entrada (comportamiento existente); Char insertar en la posición del cursor; Enter/Tab/Esc comportamiento existente.
- [ ] **Paso 2:** Añadir métodos helper de cursor: `input_char_to_byte`, `input_insert_char`, `input_delete_before`, `input_delete_at`, `input_move_word_left`, `input_move_word_right`, `input_delete_word_before`.
- [ ] **Paso 3:** Actualizar `render_input` (línea 3759) para colocar el cursor en `input_cursor` (no siempre al final), respetando el wrap de líneas.

**Criterio de aceptación:** En Input, los atajos de shell (Ctrl+A/E/W/U/K, Left/Right, Home/End, Alt+Left/Right, Backspace, Delete) funcionan y el cursor se mueve dentro de la caja.

#### Tarea 8.5: Navegación Vim en Chat (1)

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_chat_key`)

- [ ] **Paso 1:** Implementar `handle_chat_key`: j / Down scroll abajo (`chat_scroll += 1`); k / Up scroll arriba (`chat_scroll` saturating_sub 1); gg ir al inicio (`chat_scroll = valor máximo`); G ir al final (`chat_scroll = 0`); PageUp / Ctrl+U +10; PageDown / Ctrl+D -10; Home/End inicio/fin.

**Criterio de aceptación:** En Chat, las flechas y atajos Vim (j/k, gg/G) desplazan el scroll correctamente.

#### Tarea 8.6: Navegación Vim en MCPs (2), Skills (3), Agents (4)

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_mcp_panel_key`, `handle_skill_panel_key`, `handle_agent_panel_key`, `render_mcp_panel` línea 2661, `render_skill_panel` línea 2692, `render_agent_panel` línea 2726)

- [ ] **Paso 1:** Implementar `handle_mcp_panel_key`, `handle_skill_panel_key`, `handle_agent_panel_key`: j / Down índice +1 (clamp a len-1); k / Up índice -1 (saturating); gg índice 0; G índice len-1; Home/End inicio/fin.
- [ ] **Paso 2:** Actualizar `render_mcp_panel` (línea 2661), `render_skill_panel` (línea 2692) y `render_agent_panel` (línea 2726) para resaltar el elemento seleccionado según su índice.

**Criterio de aceptación:** En MCPs, Skills y Agents, las flechas y atajos Vim (j/k, gg/G) mueven la selección y el elemento activo se resalta.

#### Tarea 8.7: Números (1)-(5) en títulos de ventana e indicador de foco

**Archivos:**
- Modificar: `src/tui/app.rs` (títulos de ventana en `render_chat` línea 2853, `render_mcp_panel` línea 2661, `render_skill_panel` línea 2692, `render_agent_panel` línea 2726, `render_input` línea 3759)

- [ ] **Paso 1:** Añadir números a los títulos: Chat → " (1) Chat ", MCPs → " (2) MCPs ", Skills → " (3) Skills ", Agents → " (4) Agents ", Input → " (5) Input ".
- [ ] **Paso 2:** Resaltar visualmente la ventana enfocada (p.ej. borde con color accent) en Chat, MCPs, Skills, Agents e Input.

**Criterio de aceptación:** Los títulos de las 5 ventanas muestran (1)-(5) y la ventana enfocada se distingue visualmente.

**Criterios de aceptación de FASE 8:**
- [ ] Alt+1..Alt+5 cambian el foco entre Chat, MCPs, Skills, Agents e Input.
- [ ] En Input, los atajos de shell (Ctrl+A/E/W/U/K, Left/Right, Home/End, Alt+Left/Right, Backspace, Delete) funcionan y el cursor se mueve dentro de la caja.
- [ ] En Chat, MCPs, Skills y Agents, las flechas y atajos Vim (j/k, gg/G) funcionan.
- [ ] Los títulos de las 5 ventanas muestran (1)-(5).
- [ ] `cargo fmt --check && cargo clippy && cargo test` pasan.
- [ ] Tests unitarios nuevos en keymap.rs y app.rs.

**Dependencia:** FASE 8 depende de FASE 4 (keymap/which-key) y de la infraestructura TUI existente en `src/tui/app.rs`.

### FASE 9 — Atajos de teclado configurables desde config (commit 1)

**Objetivo:** Hacer que TODOS los atajos de teclado —incluidos los de edición de Input y la navegación Vim de Chat/paneles— sean configurables desde el archivo de configuración de anacleto, no solo las `Action` globales ya existentes.

**Contexto técnico:** `KeymapConfig` ya permite sobrescribir bindings de las `Action` existentes (Send, Quit, ToggleSidebar, Focus*, etc.) vía `config.keymap.bindings` (`src/config/types.rs`, `KeymapConfig { bindings: HashMap<String, Vec<String>> }` y `Keymap::apply_overrides` que usa `parse_action`/`parse_key`). PERO muchos atajos están HARDCODEADOS fuera del sistema de `Action`:
- `handle_input_key` (`src/tui/app.rs` línea 1304): Left/Right (con Ctrl/Alt = por palabra), Home/End, Delete, Up/Down (historial), Tab (completar), Ctrl+C (limpiar), Ctrl+J (newline), Ctrl+U (borrar a inicio), Ctrl+W (borrar palabra), Ctrl+K (borrar a fin), Ctrl+A/E (inicio/fin), Alt+b/f (palabra), Backspace.
- `handle_chat_key` (línea 1535): j/k, gg/G, PageUp/PageDown, Ctrl+U/D, Home/End.
- `handle_list_nav_key` (línea 1609): j/k, gg/G, Home/End, Esc.

#### Tarea 9.1: Ampliar el enum `Action` con variantes de edición y navegación

**Archivos:**
- Modificar: `src/tui/keymap.rs` (enum `Action`, `Keymap::default()`, `format_keymap_table()`, `parse_action()`)

- [x] **Paso 1:** Añadir variantes de edición de input al enum `Action`: `CursorLeft`, `CursorRight`, `CursorWordLeft`, `CursorWordRight`, `CursorHome`, `CursorEnd`, `DeleteChar`, `DeleteWordBefore`, `DeleteToStart`, `DeleteToEnd`, `HistoryUp`, `HistoryDown`, `TabComplete`, `InsertNewline` (nota: `ClearInput` ya existe).
- [x] **Paso 2:** Añadir variantes de navegación al enum `Action`: `ChatTop` (gg), `ChatBottom` (G), `ListTop` (gg), `ListBottom` (G).
- [x] **Paso 3:** En `Keymap::default()`, enlazar las nuevas variantes a sus atajos por defecto (Left/Right, Ctrl+Left/Alt+Left, Ctrl+Right/Alt+Right, Home/End, Delete, Ctrl+W, Ctrl+U, Ctrl+K, Ctrl+A/E, Up/Down, Tab, Ctrl+J, j/k, gg/G, PageUp/PageDown, Ctrl+U/D, Esc).
- [x] **Paso 4:** Añadir las nuevas variantes a `format_keymap_table()` (filas con descripción) y a la lista de `parse_action()`.

**Criterio de aceptación:** Todas las variantes nuevas existen en `Action`, tienen binding por defecto, aparecen en la tabla which-key y se parsean desde string en `parse_action`.

#### Tarea 9.2: Refactorizar los handlers para consultar el `Keymap`

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_input_key` línea 1304, `handle_chat_key` línea 1535, `handle_list_nav_key` línea 1609)

- [x] **Paso 1:** En `handle_input_key` (línea 1304), sustituir las comparaciones hardcodeadas de `KeyCode`/`KeyModifiers` por consultas a `self.keymap.matches(key_event, Action::CursorLeft)` (y análogas para cada variante nueva).
- [x] **Paso 2:** En `handle_chat_key` (línea 1535), sustituir j/k, gg/G, PageUp/PageDown, Ctrl+U/D, Home/End por consultas a `self.keymap.matches`.
- [x] **Paso 3:** En `handle_list_nav_key` (línea 1609), sustituir j/k, gg/G, Home/End, Esc por consultas a `self.keymap.matches`.
- [x] **Paso 4:** Eliminar cualquier rama de `KeyCode::Char`/`KeyModifiers` que ahora quede cubierta por el keymap, manteniendo el comportamiento idéntico.

**Criterio de aceptación:** Los tres handlers consultan `self.keymap.matches` y no comparan `KeyCode`/`KeyModifiers` hardcodeados; el comportamiento por defecto es idéntico al previo.

#### Tarea 9.3: Cobertura de `apply_overrides` y documentación

**Archivos:**
- Modificar: `src/config/types.rs` (`Keymap::apply_overrides`)
- Modificar: `docs/example-global-config.yaml` (sección `keymap`)
- Modificar: `README.md` (documentación de bindings configurables)

- [x] **Paso 1:** Asegurar que `Keymap::apply_overrides` (en `src/config/types.rs`) cubre todas las variantes nuevas (que `parse_action` las reconoce y `parse_key` las enlaza).
- [x] **Paso 2:** Documentar en `docs/example-global-config.yaml` el formato de `keymap.bindings` con ejemplos de las nuevas acciones (p.ej. `CursorWordLeft: ["ctrl+left", "alt+left"]`).
- [x] **Paso 3:** Documentar en `README.md` que todos los atajos (edición de Input y navegación Vim) son rebindables desde config.

**Criterio de aceptación:** `apply_overrides` acepta overrides de todas las variantes nuevas; la documentación de config y README refleja el formato completo.

#### Tarea 9.4: Tests unitarios de bindings y overrides

**Archivos:**
- Modificar: `src/tui/keymap.rs` (módulo `#[cfg(test)] mod tests`)

- [x] **Paso 1:** Añadir tests que verifiquen que cada variante nueva resuelve a su binding por defecto (p.ej. `km.matches(KeyEvent::new(KeyCode::Left, NONE), Action::CursorLeft)`).
- [x] **Paso 2:** Añadir tests de `apply_overrides` que sobrescriban un binding por defecto y verifiquen el nuevo mapeo.
- [x] **Paso 3:** Añadir un test de `parse_action`/`parse_key` round-trip para las variantes nuevas.

**Criterio de aceptación:** Los tests unitarios nuevos pasan y cubren bindings por defecto, overrides y parseo round-trip.

**Criterios de aceptación de FASE 9:**
- [x] Todos los atajos de edición de Input y navegación Vim son configurables desde `config.keymap.bindings`.
- [x] `handle_input_key`, `handle_chat_key` y `handle_list_nav_key` consultan `self.keymap.matches` (sin `KeyCode`/`KeyModifiers` hardcodeados).
- [x] `apply_overrides` cubre todas las variantes nuevas.
- [x] `docs/example-global-config.yaml` y `README.md` documentan el formato.
- [x] Tests unitarios nuevos en keymap.rs pasan.
- [x] `cargo fmt --check && cargo clippy && cargo test` pasan.

**Dependencia:** FASE 9 depende de FASE 4 (keymap/which-key) y de FASE 8 (variantes `Focus*` y handlers de foco).

### FASE 10 — Input nunca interrumpe el flujo de escritura (commit 2)

**Objetivo:** En la caja de Input, los atajos NUNCA deben interrumpir el flujo de escritura. Cualquier tecla de carácter sin modificador debe escribirse siempre, incluso con el input vacío.

**Contexto técnico:** En `handle_key` (`src/tui/app.rs` línea 954), las acciones globales se despachan cuando `keymap_applies(key_event)` devuelve true. `keymap_applies` (línea 2514) devuelve true para `KeyCode::Char(_)` cuando `modifiers != NONE || input.is_empty()`. Esto significa que con el input VACÍO, pulsar letras sin modificador dispara acciones globales: `q` = Quit, `c` = FocusChat, `i` = FocusInput, `s` = FocusSidebar, `?` = OpenWhichKey. Resultado: el usuario NO puede empezar a escribir una frase que empiece por "q", "c", "i", "s", "?" etc. (p.ej. "quiero...", "N...").

#### Tarea 10.1: Eliminar bindings de letra sin modificador de las acciones globales

**Archivos:**
- Modificar: `src/tui/keymap.rs` (`Keymap::default()`, `format_keymap_table()`)

- [x] **Paso 1:** Eliminar el binding de `Quit` a `q` sin modificador; `Quit` pasa a ser solo `Ctrl+q` (o con confirmación).
- [x] **Paso 2:** Eliminar los bindings de letra sin modificador de `FocusChat` (`c`), `FocusInput` (`i`) y `FocusSidebar` (`s`); el cambio de foco ya se cubre con Alt+1..5.
- [x] **Paso 3:** Eliminar el binding de `OpenWhichKey` a `?` sin modificador (mantener el acceso vía `Ctrl+x` o similar).
- [x] **Paso 4:** Actualizar `format_keymap_table()` para reflejar los nuevos bindings.

**Criterio de aceptación:** Ninguna acción global queda enlazada a una letra sin modificador; `Quit` solo responde a `Ctrl+q`.

#### Tarea 10.2: Modificar `keymap_applies` / flujo de `handle_key` para Input

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_key` línea 954, `keymap_applies` línea 2514)

- [x] **Paso 1:** Modificar `keymap_applies` (línea 2514) o el flujo de `handle_key` (línea 954) para que, cuando `self.focus == Focus::Input`, las teclas de carácter sin modificador SIEMPRE vayan a `handle_input_key` y nunca a acciones globales.
- [x] **Paso 2:** Asegurar que las teclas de carácter con modificador (p.ej. `Ctrl+q`) sigan disparando acciones globales incluso con foco en Input.
- [x] **Paso 3:** Mantener el comportamiento de las demás ventanas (Chat, MCPs, Skills, Agents) sin cambios.

**Criterio de aceptación:** Con foco en Input, una letra sin modificador nunca dispara una acción global; con modificador sí.

#### Tarea 10.3: Asegurar que 'q' y 'N' se puedan teclear como primer carácter

**Archivos:**
- Modificar: `src/tui/app.rs` (`handle_input_key` línea 1304)

- [x] **Paso 1:** Verificar que `handle_input_key` inserta cualquier `KeyCode::Char` sin modificador en la posición del cursor, incluido como primer carácter con input vacío.
- [x] **Paso 2:** Confirmar que 'q' y 'N' (y cualquier letra) se insertan y no disparan `Quit` ni otras acciones.

**Criterio de aceptación:** Escribir 'q' o 'N' con input vacío inserta el carácter y no dispara ninguna acción global.

#### Tarea 10.4: Tests de regresión

**Archivos:**
- Modificar: `src/tui/app.rs` (módulo `#[cfg(test)] mod tests`)

- [x] **Paso 1:** Añadir un test de regresión: con `focus == Focus::Input` e input vacío, teclear 'q' inserta el carácter y no dispara `Quit`.
- [x] **Paso 2:** Añadir un test de regresión análogo para 'N' (y al menos una letra más).
- [x] **Paso 3:** Añadir un test que verifique que `Ctrl+q` con foco en Input sí dispara `Quit`.

**Criterio de aceptación:** Los tests de regresión pasan y cubren el caso de letra como primer carácter y el de `Ctrl+q`.

**Criterios de aceptación de FASE 10:**
- [x] Ninguna acción global está enlazada a una letra sin modificador.
- [x] Con foco en Input, cualquier tecla de carácter sin modificador se escribe siempre, incluso con input vacío.
- [x] 'q' y 'N' se pueden teclear como primer carácter sin disparar acciones.
- [x] `Ctrl+q` sigue disparando `Quit` con foco en Input.
- [x] Tests de regresión nuevos pasan.
- [x] `cargo fmt --check && cargo clippy && cargo test` pasan.

**Dependencia:** FASE 10 depende de FASE 9 (keymap configurable) y de FASE 8 (modelo de foco).

### FASE 11 — Refactor: mejores prácticas Rust y división de archivos grandes (commit 3)

**Objetivo:** Dividir `src/tui/app.rs` (4714 líneas) en módulos cohesivos y aplicar mejores prácticas de Rust (idiomaticidad, ownership, errores, tests), sin cambios de comportamiento.

**Contexto técnico:** `src/tui/app.rs` contiene: struct `App` + estado, `handle_event`, `handle_key`, `handle_input_key`, `handle_chat_key`, `handle_mcp/skill/agent_panel_key`, `handle_list_nav_key`, helpers de cursor, `update_command_palette`/`update_agent_palette`/`update_model_palette`, `process_input`, `handle_command` (~460 líneas, 1940-2398), `open_editor`, `resume_quick_slot`, `collect_init_answer`, y TODAS las funciones `render_*` (`render`, `render_chat`, `render_input`, `render_mcp_panel`, `render_skill_panel`, `render_agent_panel`, `render_command_palette`, `render_agent_palette`, `render_model_palette`, `render_approval_dialog`, `render_question_dialog`, `render_markdown_line`, `parse_inline`, etc.) y helpers (`shift_char`, `copy_to_clipboard`, `format_tokens`, `visual_line_count`, `select_visible_start`).

#### Tarea 11.1: Crear módulos cohesivos en src/tui/

**Archivos:**
- Crear: `src/tui/input.rs` (`handle_input_key` + helpers de cursor)
- Crear: `src/tui/navigation.rs` (`handle_chat_key` + panel keys + `handle_list_nav_key` + `is_double_g`)
- Crear: `src/tui/commands.rs` (`handle_command` + `process_input`)
- Crear: `src/tui/palette.rs` (`update_*_palette` + `render_*_palette`)
- Crear: `src/tui/markdown.rs` (`render_markdown_line`, `parse_inline`, `visual_line_count`, `select_visible_start`)
- Crear: `src/tui/theme.rs` (`Theme`)
- Crear: `src/tui/render.rs` (o submodulo `render/`) para las funciones `render_*` restantes
- Modificar: `src/tui/app.rs` (eliminar el código movido)

- [x] **Paso 1:** Crear `src/tui/input.rs` y mover `handle_input_key` (línea 1304) y los helpers de cursor.
- [x] **Paso 2:** Crear `src/tui/navigation.rs` y mover `handle_chat_key` (línea 1535), `handle_mcp/skill/agent_panel_key`, `handle_list_nav_key` (línea 1609) e `is_double_g`.
- [x] **Paso 3:** Crear `src/tui/commands.rs` y mover `handle_command` (líneas 1940-2398) y `process_input`.
- [x] **Paso 4:** Crear `src/tui/palette.rs` y mover `update_command_palette`/`update_agent_palette`/`update_model_palette` y sus `render_*_palette`.
- [x] **Paso 5:** Crear `src/tui/markdown.rs` y mover `render_markdown_line`, `parse_inline`, `visual_line_count` y `select_visible_start`.
- [x] **Paso 6:** Crear `src/tui/theme.rs` y mover `Theme`.
- [x] **Paso 7:** Crear `src/tui/render.rs` (o submodulo `render/`) y mover las funciones `render_*` restantes (`render`, `render_chat`, `render_input`, `render_mcp_panel`, `render_skill_panel`, `render_agent_panel`, `render_approval_dialog`, `render_question_dialog`).

**Criterio de aceptación:** `app.rs` queda reducido a la struct `App`, el estado y el enrutado; cada grupo de funciones vive en su módulo cohesivo.

#### Tarea 11.2: Ajustar visibilidad y declaraciones `mod`

**Archivos:**
- Modificar: `src/tui/mod.rs` (declaraciones `mod`)
- Modificar: los nuevos módulos (`pub(crate)`/`pub` en funciones y tipos)

- [x] **Paso 1:** Añadir las declaraciones `mod input; mod navigation; mod commands; mod palette; mod markdown; mod theme; mod render;` en `src/tui/mod.rs`.
- [x] **Paso 2:** Ajustar la visibilidad de funciones y tipos movidos a `pub(crate)`/`pub` según lo que consuma `app.rs` y el resto del crate.
- [x] **Paso 3:** Ajustar los accesos a campos de `App` y tipos compartidos (p.ej. `Focus`, `Action`) para que los módulos nuevos compilen.

**Criterio de aceptación:** El crate compila con los módulos nuevos y la visibilidad correcta.

#### Tarea 11.3: Aplicar mejores prácticas de Rust

**Archivos:**
- Modificar: los módulos nuevos y `src/tui/app.rs`

- [x] **Paso 1:** Reducir clonaciones innecesarias (usar referencias/`Cow` donde aplique).
- [x] **Paso 2:** Usar tipos correctos (p.ej. `usize` para índices, `saturating_sub`/`clamp` para navegación).
- [x] **Paso 3:** Manejo de errores idiomático (propagar con `anyhow`/`Result` en lugar de `unwrap`/`expect` donde sea posible).
- [x] **Paso 4:** Eliminar código muerto y añadir doc-comments a los módulos y funciones públicas.

**Criterio de aceptación:** El código movido sigue las mejores prácticas de Rust sin cambios de comportamiento.

#### Tarea 11.4: Verificación de calidad tras la división

**Archivos:**
- Modificar: `src/tui/app.rs` (módulo `#[cfg(test)] mod tests` si es necesario)

- [x] **Paso 1:** Ejecutar `cargo fmt --check` y corregir el formato.
- [x] **Paso 2:** Ejecutar `cargo clippy` y corregir warnings.
- [x] **Paso 3:** Ejecutar `cargo test` y asegurar que todos los tests pasan (sin cambios de comportamiento).
- [x] **Paso 4:** Verificar que los tests existentes (incluidos los de FASE 8/9/10) siguen pasando tras la división.

**Criterio de aceptación:** `cargo fmt --check && cargo clippy && cargo test` pasan en verde tras la división, sin cambios de comportamiento.

**Criterios de aceptación de FASE 11:**
- [x] `src/tui/app.rs` queda dividido en módulos cohesivos (`input`, `navigation`, `commands`, `palette`, `markdown`, `theme`, `render`).
- [x] `src/tui/mod.rs` declara los módulos nuevos con visibilidad correcta.
- [x] Se aplican mejores prácticas de Rust (menos clonaciones, tipos correctos, errores idiomáticos, sin código muerto).
- [x] `cargo fmt --check && cargo clippy && cargo test` pasan sin cambios de comportamiento.
- [x] Los tests existentes (incluidos los de FASE 8/9/10) siguen pasando.

**Dependencia:** FASE 11 depende de FASE 8, FASE 9 y FASE 10 (refactoriza el código ya modificado por esas fases).

### FASE 12 — Organización de archivos grandes restantes

**Objetivo:** Dividir los archivos de `src/` que siguen siendo grandes (>500 líneas) en módulos cohesivos, aplicando la misma técnica de división de módulos de FASE 11, sin cambios de comportamiento. Cada tarea numerada corresponde a UN archivo grande a dividir.

**Contexto técnico:** Tras FASE 11, `src/tui/app.rs` ya se dividió en módulos. Quedan por organizar los siguientes archivos grandes: `src/agent/lifecycle.rs` (3307), `src/engine/orchestrator.rs` (2448), `src/llm/provider.rs` (1898), `src/tui/app.rs` (1676, sigue grande), `src/db/session.rs` (1565), `src/tui/keymap.rs` (1019), `src/mcp/client.rs` (771), `src/lsp/mod.rs` (631) y `src/engine/apply_patch.rs` (558, borderline). Los números de línea citados son los actuales del código.

#### Tarea 12.1: Dividir `src/agent/lifecycle.rs` (3307 líneas)

**Archivos:**
- Crear: `src/agent/tools.rs` (definición y ejecución de tools)
- Crear: `src/agent/context.rs` (gestión de contexto)
- Modificar: `src/agent/lifecycle.rs` (dejar `AgentHandle`, `SpawnAgentConfig`, `spawn_agent` y los tests correspondientes)
- Modificar: `src/agent/mod.rs` (declaraciones `mod tools; mod context;`)

- [x] **Paso 1:** En `src/agent/lifecycle.rs`, conservar `AgentHandle` (63), `SpawnAgentConfig` (88) y `spawn_agent` (129-860, el bucle principal del agente).
- [x] **Paso 2:** Crear `src/agent/tools.rs` y mover la definición y ejecución de tools (860-2405): `check_tool_permission` (860), `classify_tool_operation` (964), `is_sensitive_operation` (1002), `skill_to_tool_definition` (1042), `todo_tool_definition` (1098), `execute_todo_tool` (1138), `question_tool_definition` (1219), `execute_question_tool` (1250), `apply_patch_tool_definition` (1302), `request_batch_approval` (1346), `execute_apply_patch_tool` (1382), `execute_skill_tool` (1438), `execute_shell_command` (1516), `extract_shell_command` (1589), `looks_like_shell_command` (1616), `execute_web_fetch` (1731), `execute_filesystem_operation` (1772), `subagent_config_to_tool_definition` (1778), `TaskToolArgs` (1805), `task_tool_definition` (1864), `plan_mode_blocked` (1909), `execute_task_tool` (1947), `resolve_provider_for_model` (2098), `SpawnSubagentConfig` (2118) y `spawn_subagent_and_delegate` (2146).
- [x] **Paso 3:** Crear `src/agent/context.rs` y mover la gestión de contexto (2405-2658): `estimate_tokens` (2405), `should_compact` (2424), `build_summary_prompt` (2459), `trim_conversation` (2495) y `summarize_conversation` (2554).
- [x] **Paso 4:** Mover los tests (2658-3307) a su módulo correspondiente (`tools.rs`/`context.rs`/`lifecycle.rs` según lo que prueben).
- [x] **Paso 5:** Ajustar visibilidad (`pub(crate)`/`pub`) y declaraciones `mod` en `src/agent/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `lifecycle.rs` queda con el bucle principal del agente; las tools viven en `tools.rs` y la gestión de contexto en `context.rs`; los tests se mueven con su código; el crate compila y los tests pasan.

#### Tarea 12.2: Dividir `src/engine/orchestrator.rs` (2448 líneas)

**Archivos:**
- Crear: `src/engine/sessions.rs` (comandos de sesión)
- Crear: `src/engine/commands.rs` (resto de comandos `handle_*`)
- Crear: `src/engine/events.rs` (tipos de eventos y comandos)
- Modificar: `src/engine/orchestrator.rs` (dejar `Engine` struct + core)
- Modificar: `src/engine/mod.rs` (declaraciones `mod`)

- [x] **Paso 1:** Crear `src/engine/events.rs` y mover los tipos: `EngineEvent` (29), `InitAnswers` (209), `SkillInfo` (220), `McpStatus` (229), `StatusInfo` (238), `TimelineEntry` (259), `UsageEvent` (273) y `EngineCommand` (339).
- [x] **Paso 2:** En `src/engine/orchestrator.rs`, conservar el struct `Engine` (281) y el core: `Engine::new` (444), `initialize` (501), `resolve_agent_provider` (707), `run` (728), `handle_user_input` (904), `handle_set_model` (926), `handle_switch_agent` (949), `handle_record_model_usage` (985), `handle_list_model_frecency` (993) y `respawn_active_agent` (1006).
- [x] **Paso 3:** Crear `src/engine/sessions.rs` y mover los comandos de sesión (1087-1446): `handle_new_session` (1087), `handle_resume_session` (1109), `handle_list_sessions` (1164), `handle_set_session_pinned` (1176), `handle_delete_session` (1189), `handle_rename_session` (1206), `clear_undo_redo` (1224), `reload_history_to_root` (1230), `handle_undo` (1262), `handle_redo` (1288), `handle_fork` (1313), `handle_export` (1344), `handle_import` (1385), `handle_share` (1407) y `handle_unshare` (1427).
- [x] **Paso 4:** Crear `src/engine/commands.rs` y mover el resto de comandos (1446-2448): `handle_list_skills` (1446), `handle_list_mcps` (1469), `handle_toggle_mcp` (1491), `handle_status` (1502), `handle_init` (1546), `handle_review` (1566), `handle_warp` (1604), `handle_list_workspaces` (1614), `handle_move_session` (1629), `handle_worktree_add/list/remove` (1654/1665/1675), `handle_timeline` (1686), `handle_build` (1716), `handle_parent` (1747), `handle_children` (1761), `handle_list_jobs` (1777), `handle_snapshot` (1787), `handle_revert` (1813), `handle_list_snapshots` (1850), `handle_stage` (1868), `handle_clear` (1890), `handle_commit` (1903) y `handle_approval_response` (1932).
- [x] **Paso 5:** Ajustar visibilidad y declaraciones `mod` en `src/engine/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `orchestrator.rs` queda con el struct `Engine` y el core; los comandos de sesión viven en `sessions.rs`, el resto de comandos en `commands.rs` y los tipos en `events.rs`; el crate compila y los tests pasan.

#### Tarea 12.3: Dividir `src/llm/provider.rs` (1898 líneas)

**Archivos:**
- Crear: `src/llm/openai.rs` (tipos OpenAI + `OpenAIProvider`)
- Crear: `src/llm/anthropic.rs` (tipos Anthropic + `AnthropicProvider`)
- Crear: `src/llm/ollama.rs` (tipos Ollama + `OllamaProvider`)
- Crear: `src/llm/models.rs` (tipos de catálogo de modelos)
- Modificar: `src/llm/provider.rs` (dejar trait `LlmProvider` + `create_provider` + helpers compartidos)
- Modificar: `src/llm/mod.rs` (declaraciones `mod`)

- [x] **Paso 1:** En `src/llm/provider.rs`, conservar el trait `LlmProvider` (18), `create_provider` (45) y los helpers compartidos (332-500): `http_client` (332), `into_openai_messages` (340), `into_anthropic_messages` (378), `into_ollama_messages` (399), `into_openai_tools` (416), `anthropic_tools` (431), `parse_sse_line` (444) y `strip_model_prefix` (465).
- [x] **Paso 2:** Crear `src/llm/openai.rs` y mover los tipos OpenAI (62-180): `OpenAiChatRequest`, `OpenAiMessage`, `OpenAiTool`, `OpenAiFunction`, `OpenAiToolCall`, `OpenAiFunctionCall`, `OpenAiChatResponse`, `OpenAiChoice`, `OpenAiResponseMessage`, `OpenAiUsage`, `OpenAiStreamChunk`, `OpenAiStreamChoice`, `OpenAiStreamDelta`, `OpenAiStreamToolCall`, `OpenAiStreamFunction`, y el `OpenAIProvider` (501) con su impl `LlmProvider` (533).
- [x] **Paso 3:** Crear `src/llm/anthropic.rs` y mover los tipos Anthropic (192-280): `AnthropicRequest`, `AnthropicThinking`, `AnthropicCacheControl`, `AnthropicMessage`, `AnthropicTool`, `AnthropicResponse`, `AnthropicContentBlock`, `AnthropicToolUse`, `AnthropicUsage`, y el `AnthropicProvider` con su impl `LlmProvider`.
- [x] **Paso 4:** Crear `src/llm/ollama.rs` y mover los tipos Ollama (289-330): `OllamaChatRequest`, `OllamaMessage`, `OllamaOptions`, `OllamaChatResponse`, `OllamaResponseMessage`, y el `OllamaProvider` con su impl `LlmProvider`.
- [x] **Paso 5:** Crear `src/llm/models.rs` y mover los tipos de catálogo (471-500): `OpenAiModelInfo` (471), `OpenRouterModelList` (478), `OpenRouterModelData` (483) y `OllamaShowResponse` (491).
- [x] **Paso 6:** Ajustar visibilidad y declaraciones `mod` en `src/llm/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `provider.rs` queda con el trait, la factory y los helpers compartidos; cada provider vive en su módulo (`openai.rs`/`anthropic.rs`/`ollama.rs`) y los tipos de catálogo en `models.rs`; el crate compila y los tests pasan.

#### Tarea 12.4: Dividir `src/tui/app.rs` (1676 líneas, sigue grande)

**Archivos:**
- Crear: `src/tui/events.rs` (`handle_event`)
- Crear: `src/tui/keys.rs` (`handle_key` + `keymap_applies`)
- Crear: `src/tui/types.rs` (tipos compartidos)
- Crear: `src/tui/state.rs` (helpers de estado)
- Modificar: `src/tui/app.rs` (dejar struct `App` + `new` + `run_tui` + `push_msg`/`commit_stream`)
- Modificar: `src/tui/mod.rs` (declaraciones `mod`)

- [x] **Paso 1:** En `src/tui/app.rs`, conservar el struct `App` + campos, `new` (386), `push_msg` (505), `commit_stream` (512) y `run_tui` (~2527).
- [x] **Paso 2:** Crear `src/tui/events.rs` y mover `handle_event` (522-954, ~430 líneas).
- [x] **Paso 3:** Crear `src/tui/keys.rs` y mover `handle_key` (954-1304, ~350 líneas) y `keymap_applies` (~2471).
- [x] **Paso 4:** Crear `src/tui/types.rs` y mover los tipos compartidos: `Focus`, `AgentInfo`, `ApprovalRequest`, `QuestionState`, `InitFlow` y la const `BUILTIN_COMMANDS`.
- [x] **Paso 5:** Crear `src/tui/state.rs` y mover los helpers de estado: `unique_mcp_count`, `unique_skill_count`, `agent_panel_count` y `fuzzy_score`.
- [x] **Paso 6:** Mover el módulo `tests` a su módulo correspondiente; ajustar visibilidad y declaraciones `mod` en `src/tui/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `app.rs` queda con el struct `App`, `new`, `run_tui` y `push_msg`/`commit_stream`; el manejo de eventos vive en `events.rs`, las teclas en `keys.rs`, los tipos en `types.rs` y los helpers de estado en `state.rs`; el crate compila y los tests pasan.

#### Tarea 12.5: Dividir `src/db/session.rs` (1565 líneas)

**Archivos:**
- Crear: `src/db/messages.rs` (mensajes)
- Crear: `src/db/todos.rs` (todos)
- Crear: `src/db/snapshots.rs` (snapshots)
- Crear: `src/db/export.rs` (export/import/render)
- Crear: `src/db/usage.rs` (uso de modelo)
- Modificar: `src/db/session.rs` (dejar `Database` struct + CRUD de sesiones)
- Modificar: `src/db/mod.rs` (declaraciones `mod`)

- [x] **Paso 1:** En `src/db/session.rs`, conservar el struct `Database` + `open` (20), `run_migrations` (46), `ensure_column` (170) y el CRUD de sesiones: `create_session` (190), `create_session_with_parent` (195), `list_sessions` (355), `set_session_pinned` (400), `list_pinned_sessions` (410), `delete_session` (456), `rename_session` (472), `set_shared` (611), `get_session_metadata` (646), `get_session_name` (668), `set_session_workspace` (742), `set_parent` (752), `get_parent` (762) y `get_children` (773).
- [x] **Paso 2:** Crear `src/db/messages.rs` y mover `store_message` (231), `store_message_full` (278), `get_session_messages` (317), `delete_messages` (485), `restore_messages` (547) y `copy_messages` (585).
- [x] **Paso 3:** Crear `src/db/todos.rs` y mover `add_todo` (820), `update_todo` (856), `delete_todo` (885) y `list_todos` (894).
- [x] **Paso 4:** Crear `src/db/snapshots.rs` y mover `create_snapshot` (967), `list_snapshots` (1003), `get_snapshot` (1037) y `delete_snapshot` (1068).
- [x] **Paso 5:** Crear `src/db/export.rs` y mover `export_session` (678), `import_session` (719) y `render_markdown` (1078).
- [x] **Paso 6:** Crear `src/db/usage.rs` y mover `record_model_usage` (924) y `list_model_frecency` (944).
- [x] **Paso 7:** Mover los tests (1105-1565) a su módulo correspondiente; ajustar visibilidad y declaraciones `mod` en `src/db/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `session.rs` queda con el struct `Database` y el CRUD de sesiones; mensajes, todos, snapshots, export/import y uso de modelo viven en sus módulos; el crate compila y los tests pasan.

#### Tarea 12.6: Dividir `src/tui/keymap.rs` (1019 líneas)

**Archivos:**
- Crear: `src/tui/keyparse.rs` (parseo y formateo de teclas)
- Modificar: `src/tui/keymap.rs` (dejar enum `Action` + struct `Keymap`)
- Modificar: `src/tui/mod.rs` (declaración `mod keyparse;`)

- [x] **Paso 1:** En `src/tui/keymap.rs`, conservar el enum `Action` (~40 variantes), el struct `Keymap` + `new`/`bind`/`resolve`/`matches`/`apply_overrides` y `Keymap::default` (todos los bindings).
- [x] **Paso 2:** Crear `src/tui/keyparse.rs` y mover `key_event`, `format_keymap_table`, `format_key`, `parse_action`, `to_snake_case` y `parse_key`.
- [x] **Paso 3:** Mover los tests a su módulo correspondiente; ajustar visibilidad y declaración `mod keyparse;` en `src/tui/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `keymap.rs` queda con el enum `Action` y el struct `Keymap`; el parseo y formateo de teclas vive en `keyparse.rs`; los tests se mueven con su código; el crate compila y los tests pasan.

#### Tarea 12.7: Dividir `src/mcp/client.rs` (771 líneas)

**Archivos:**
- Crear: `src/mcp/registry.rs` (`McpRegistry`)
- Crear: `src/mcp/parse.rs` (helpers de parseo de respuestas)
- Modificar: `src/mcp/client.rs` (dejar `McpClient` + impl transporte/JSON-RPC)
- Modificar: `src/mcp/mod.rs` (declaraciones `mod`)

- [x] **Paso 1:** En `src/mcp/client.rs`, conservar el struct `McpClient` + impl: `new` (34), `connect` (48), `connect_stdio` (56), `connect_tcp` (86), `perform_initialize` (100), `call_tool` (181), `list_tools` (224), `list_tools_inner` (229), `list_resources` (254), `list_resource_templates` (269), `read_resource` (288), `send_jsonrpc` (305), `read_jsonrpc` (325) y `disconnect` (360).
- [x] **Paso 2:** Crear `src/mcp/registry.rs` y mover el struct `McpRegistry` + impl (471-607): `new`/`register`/`get`/`names`/`collect_tools`/`call_tool`/`list_resources`/`list_resource_templates`/`read_resource`/`disconnect_all`.
- [x] **Paso 3:** Crear `src/mcp/parse.rs` y mover los helpers de parseo: `parse_resources_response` (374), `parse_resource_templates_response` (398) y `parse_read_resource_response` (427).
- [x] **Paso 4:** Mover los tests (609-771) a su módulo correspondiente; ajustar visibilidad y declaraciones `mod` en `src/mcp/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `client.rs` queda con el `McpClient` y su transporte/JSON-RPC; el `McpRegistry` vive en `registry.rs` y los helpers de parseo en `parse.rs`; el crate compila y los tests pasan.

#### Tarea 12.8: Dividir `src/lsp/mod.rs` (631 líneas)

**Archivos:**
- Crear: `src/lsp/format.rs` (formateo de resultados)
- Modificar: `src/lsp/mod.rs` (dejar `LspClient` + impl transporte/consultas)
- Modificar: `src/lsp/mod.rs` (declaración `mod format;`)

- [x] **Paso 1:** En `src/lsp/mod.rs`, conservar el struct `LspClient` + impl: `new` (70), `start` (85), `initialize` (122), `request` (137), `notify` (156), `hover` (166), `definition` (176), `references` (186), `diagnostic` (197), `query_once` (209), `write_message` (230), `read_message` (253) y `shutdown` (295).
- [x] **Paso 2:** Crear `src/lsp/format.rs` y mover el formateo (318-493): `parse_lsp_response` (318), `format_lsp_result` (335), `format_hover` (346), `format_locations` (380), `format_diagnostics` (413), `severity_label` (453), `path_to_uri` (464) y `default_server_for_extension` (476).
- [x] **Paso 3:** Mover los tests (494-631) a su módulo correspondiente; ajustar visibilidad y declaración `mod format;`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.

**Criterio de aceptación:** `mod.rs` queda con el `LspClient` y su transporte/consultas; el formateo de resultados vive en `format.rs`; el crate compila y los tests pasan.

#### Tarea 12.9: Mover los tests de `src/engine/apply_patch.rs` (558 líneas, borderline)

**Archivos:**
- Crear: `src/engine/apply_patch_tests.rs` (tests movidos)
- Modificar: `src/engine/apply_patch.rs` (dejar solo la lógica)
- Modificar: `src/engine/mod.rs` (declaración `mod apply_patch_tests;`)

- [x] **Paso 1:** En `src/engine/apply_patch.rs`, conservar la lógica: `PatchOpKind` (14), `PatchOp` (25), `PatchBatch` (37), `FileEncoding` (44), `parse_patch_batch` (55), `resolve_within_workspace` (87), `detect_encoding` (141), `encode_content` (152), `resolve_patch_path` (174), `apply_patch_batch` (197) y `batch_to_unified_diff` (259).
- [x] **Paso 2:** Mover los tests (287-558, ~270 líneas) a `src/engine/apply_patch_tests.rs` (o `src/engine/apply_patch/tests.rs`), dejando `apply_patch.rs` con solo la lógica (~286 líneas).
- [x] **Paso 3:** Ajustar visibilidad y declaración `mod` en `src/engine/mod.rs`; ejecutar `cargo fmt --check && cargo clippy && cargo test`.
- [x] **Paso 4:** (Opcional) Si se prefiere, dejar `apply_patch.rs` como está al ser borderline; en ese caso marcar esta tarea como no aplicable.

**Criterio de aceptación:** `apply_patch.rs` queda con solo la lógica y los tests viven en su propio módulo; el crate compila y los tests pasan (o la tarea se descarta por ser borderline).

**Criterios de aceptación de FASE 12:**
- [x] Ningún archivo de `src/` supera las 500 líneas (salvo los que se decida dejar, p.ej. `apply_patch.rs` si se descarta la Tarea 12.9).
- [x] Cada archivo grande se divide en módulos cohesivos con nombres y símbolos reales del código.
- [x] Los tests existentes se mueven con su código y siguen pasando.
- [x] `cargo fmt --check && cargo clippy && cargo test` pasan sin cambios de comportamiento.
- [x] No se añaden dependencias nuevas ni se cambia la interfaz pública consumida por otras fases.

**Dependencia:** FASE 12 depende de FASE 11 (misma técnica de división de módulos).
