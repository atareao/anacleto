# Anacleto — Implementación de Features de OpenCode (v0.3.0)

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

- [ ] **Paso 1:** Añadir variante `Task(TaskCall)` al enum `ToolCall` con campos `{ task_id, description, mode: Foreground|Background, model, tools }`.
- [ ] **Paso 2:** Extender `SpawnAgentConfig` con `task_id: Option<String>`, `depth: u32`, y `permissions: Permissions` derivadas.
- [ ] **Paso 3:** En el handler del engine, interceptar `ToolCall::Task`; si `mode == Foreground`, spawn + esperar resultado y devolver `ToolResult`; si `Background`, registrar job y devolver `task_id`.
- [ ] **Paso 4:** Implementar derivación de permisos: `child.permissions = parent.permissions ∩ child.permissions` (deny del padre se propaga).
- [ ] **Paso 5:** Implementar límite `subagent_depth`: si `depth > config.subagent_depth`, devolver error al modelo.
- [ ] **Paso 6:** Implementar reanudación por `task_id`: cargar historial de la sesión del subagente vía `LoadHistory`.

**Criterio de aceptación:** El modelo puede invocar `task` en foreground y background; el subagente hereda permisos restringidos del padre; `task_id` reanuda sesión existente; `subagent_depth` bloquea anidación excesiva.

#### Tarea 1.2: Subagentes en background + notificaciones de finalización

**Archivos:**
- Crear: `src/engine/jobs.rs` (JobRegistry)
- Modificar: `src/engine/orchestrator.rs` (registro de jobs, canal de resultados en `tokio::select!`)
- Modificar: `src/engine/mod.rs` (exponer `jobs`)
- Modificar: `src/tui/app.rs` (toast de finalización)

- [ ] **Paso 1:** Crear `JobRegistry` con `HashMap<task_id, JoinHandle>` y canal `mpsc` de resultados.
- [ ] **Paso 2:** Añadir el `rx` de resultados al `tokio::select!` del loop principal.
- [ ] **Paso 3:** Al completar un job, emitir `EngineEvent::SubagentFinished(task_id, summary)`.
- [ ] **Paso 4:** En la TUI, mostrar indicador de job activo y toast al completar.

**Criterio de aceptación:** Los subagentes background no bloquean el loop; la TUI muestra el estado del job y un toast al finalizar.

#### Tarea 1.3: Plan Mode → Build handoff

**Archivos:**
- Modificar: `src/engine/orchestrator.rs` (comando `/build`, transición plan→build)
- Modificar: `src/agent/types.rs` (modo plan/build en estado del agente)
- Modificar: `src/tui/app.rs` (comando `/build`)

- [ ] **Paso 1:** Definir estado `plan`/`build` en el agente; en plan mode, todas las herramientas de escritura devuelven error.
- [ ] **Paso 2:** Implementar `/build`: leer archivo markdown de plan, crear agente build con permisos de escritura.
- [ ] **Paso 3:** Inyectar el contenido del plan como mensaje sintético de ejecución (`UserInput`/`System`).

**Criterio de aceptación:** Un agente plan solo-lectura genera un plan markdown; al aprobarse, un agente build lo ejecuta con el plan como contexto.

#### Tarea 1.4: Árbol de sesiones / navegación padre-hijo

**Archivos:**
- Modificar: `src/db/models.rs` (Session → `parent_id`)
- Modificar: `src/db/mod.rs` (`ensure_column` para `parent_id`, método `set_parent`)
- Modificar: `src/engine/orchestrator.rs` (fork con parentID, comandos `/parent`, `/children`)
- Modificar: `src/tui/app.rs` (comandos + navegación)

- [ ] **Paso 1:** Añadir columna `parent_id` vía `ensure_column` y campo en `Session`.
- [ ] **Paso 2:** En `/fork`, registrar `parent_id = sesión actual`.
- [ ] **Paso 3:** Implementar `/parent` y `/children` para navegar la jerarquía.
- [ ] **Paso 4:** Mostrar jerarquía en el sidebar de sesiones.

**Criterio de aceptación:** `/fork` crea sesión hija con parentID; se navega padre↔hijo; la jerarquía se persiste y se muestra en la TUI.

**Dependencia:** FASE 1 depende de la infraestructura de `spawn_agent` y `ToolCall` existentes (ya presentes). FASE 4 (TUI) consume los `EngineEvent` de FASE 1.

### FASE 2 — Contexto y memoria

#### Tarea 2.1: Compaction anclada con plantilla estructurada

**Archivos:**
- Crear: `src/engine/compaction.rs` (plantilla + fusión de resumen)
- Modificar: `src/config/types.rs` (`Config.session.compaction`)
- Modificar: `src/engine/orchestrator.rs` (disparo por umbral de contexto)
- Modificar: `src/llm/provider.rs` (exponer `window` del modelo)

- [ ] **Paso 1:** Definir plantilla Markdown fija (`## Objective / Important Details / Work State / Next Move / Relevant Files`).
- [ ] **Paso 2:** Implementar fusión: parsear resumen previo por secciones; actualizar `Work State`, `Next Move`, `Relevant Files`; conservar `Objective`/`Important Details` salvo cambio.
- [ ] **Paso 3:** Config `compaction = { mode: auto|manual, buffer, keep }`.
- [ ] **Paso 4:** Disparar cuando `context_used > window − buffer`; compactar manteniendo `keep` tokens.

**Criterio de aceptación:** La compactación actualiza (no regenera) el resumen anclado; respeta `buffer`/`keep`; se dispara automáticamente en modo `auto`.

#### Tarea 2.2: Truncado de salida de herramientas + tool-output store

**Archivos:**
- Crear: `src/engine/tool_output.rs` (ToolOutputStore)
- Modificar: `src/engine/orchestrator.rs` (truncado en handler de ToolResult)
- Modificar: `src/tui/app.rs` (colapso/expansión de salida larga)

- [ ] **Paso 1:** Crear `ToolOutputStore` (mapa `tool_call_id → contenido completo`).
- [ ] **Paso 2:** En el handler de `ToolResult`, truncar a ~2000 chars antes del modelo; guardar el completo en el store.
- [ ] **Paso 3:** En la TUI, colapsar salidas largas con toggle para expandir (lee del store).

**Criterio de aceptación:** El modelo recibe salidas truncadas; el contenido completo está disponible en la TUI vía toggle.

#### Tarea 2.3: Revert/fork basado en snapshots

**Archivos:**
- Crear: `src/engine/snapshot.rs` (git-tree content-addressed)
- Modificar: `src/db/mod.rs` (tabla `snapshots`)
- Modificar: `src/engine/orchestrator.rs` (comandos `/revert`, `/stage`, `/clear`, `/commit`)
- Modificar: `src/tui/app.rs` (comandos)

- [ ] **Paso 1:** Implementar snapshot content-addressed del árbol de archivos por turno de asistente.
- [ ] **Paso 2:** Persistir snapshots en `db` con referencia al turno.
- [ ] **Paso 3:** Implementar `/revert` (restaurar desde snapshot previo), `/stage`, `/clear`, `/commit`.

**Criterio de aceptación:** Cada turno de asistente genera un snapshot; `/revert` restaura archivos desde un snapshot previo; stage/clear/commit operan sobre el snapshot.

#### Tarea 2.4: Contexto de sistema como fuentes tipadas refrescables

**Archivos:**
- Crear: `src/engine/source.rs` (trait `Source<A>`)
- Modificar: `src/engine/orchestrator.rs` (registro de fuentes, baseline/delta)

- [ ] **Paso 1:** Definir trait `Source<A>` con `baseline()` y `delta()`.
- [ ] **Paso 2:** Mantener registro de fuentes y su estado; reinyectar solo las cambiadas.
- [ ] **Paso 3:** Enviar baseline una vez; deltas en turnos siguientes.

**Criterio de aceptación:** Solo las fuentes cuyo estado cambió se reinyectan; el baseline se envía una vez.

#### Tarea 2.5: Archivos de instrucción (AGENTS.md, CLAUDE.md, CONTEXT.md)

**Archivos:**
- Crear: `src/engine/instructions.rs` (descubrimiento)
- Modificar: `src/engine/orchestrator.rs` (inyección por turnos)
- Modificar: `src/config/paths.rs` (rutas global + proyecto)

- [ ] **Paso 1:** Descubrir `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md` en workspace y config global.
- [ ] **Paso 2:** Inyectar como contexto de sistema por turnos (con cache-control en FASE 6).

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

#### Tarea 6.1: Política de prompt-caching / cache-control breakpoints

**Archivos:**
- Modificar: `src/llm/provider.rs` (inyección de `cache_control`)
- Modificar: `src/config/types.rs` (config `cache: auto|off`)
- Modificar: `src/engine/orchestrator.rs` (buckets TTL)

- [ ] **Paso 1:** Implementar `cache:auto`: inyectar `cache_control` en el último tool/system/user.
- [ ] **Paso 2:** Implementar buckets con TTL.
- [ ] **Paso 3:** Hacer provider-aware (Anthropic, OpenAI, etc.).

**Criterio de aceptación:** `cache:auto` inyecta cache_control en los breakpoints correctos; los buckets respetan TTL; es provider-aware.

#### Tarea 6.2: Anthropic extended thinking

**Archivos:**
- Modificar: `src/llm/provider.rs` (campo `thinking` en request)
- Modificar: `src/llm/anthropic.rs` (parseo de bloque thinking)

- [ ] **Paso 1:** Añadir `thinking: { type: enabled, budget_tokens }` a la request.
- [ ] **Paso 2:** Parsear el bloque thinking de la respuesta.

**Criterio de aceptación:** El modelo Anthropic recibe budget_tokens y el bloque thinking se parsea correctamente.

#### Tarea 6.3: Plantillas de system-prompt por modelo/agente

**Archivos:**
- Modificar: `src/config/types.rs` (`system_prompt` como plantilla)
- Crear: `src/llm/template.rs` (renderizado de variables)

- [ ] **Paso 1:** Permitir `system_prompt` como plantilla con variables (`{model}`, `{workspace}`, `{tools}`).
- [ ] **Paso 2:** Renderizar la plantilla al construir el contexto.

**Criterio de aceptación:** El system-prompt se renderiza con variables por modelo/agente.

#### Tarea 6.4: Catálogo ampliado de providers

**Archivos:**
- Modificar: `src/llm/provider.rs` (`LlmProviderRegistry`)
- Crear: `src/llm/bedrock.rs`, `src/llm/azure.rs`, `src/llm/google.rs`

- [ ] **Paso 1:** Añadir constructores para Bedrock, Azure, Google.

**Criterio de aceptación:** Los nuevos providers se registran y seleccionan desde config.

**Dependencia:** FASE 6 depende de FASE 2 (compaction/contexto) para los breakpoints de cache.

### FASE 7 — Extensibilidad

#### Tarea 7.1: Sistema de plugins con hooks/transforms

**Archivos:**
- Crear: `src/plugin/mod.rs` (trait `Plugin`)
- Modificar: `src/engine/orchestrator.rs` (invocación de hooks)
- Modificar: `src/config/paths.rs` (directorio de plugins)

- [ ] **Paso 1:** Definir trait `Plugin` con hooks (`on_agent_spawn`, `on_tool_call`, `on_command`, `on_event`) y transforms.
- [ ] **Paso 2:** Cargar plugins desde `~/.config/anacleto/plugins/`.
- [ ] **Paso 3:** Invocar hooks en los puntos del engine.

**Criterio de aceptación:** Los plugins se cargan y sus hooks/transforms se invocan en los puntos definidos.

#### Tarea 7.2: Comandos slash personalizados con templating

**Archivos:**
- Modificar: `src/tui/app.rs` (mover `COMMANDS` a registro dinámico)
- Modificar: `src/config/types.rs` (comandos personalizados)
- Crear: `src/engine/template.rs` (variables `{env:VAR}`, `{file:path}`)

- [ ] **Paso 1:** Mover la lógica de `COMMANDS` a un registro dinámico.
- [ ] **Paso 2:** Definir comandos personalizados en config con templating de variables.

**Criterio de aceptación:** Los comandos slash personalizados se definen en config y expanden `{env:VAR}`/`{file:path}`.

#### Tarea 7.3: Tools y providers personalizados en runtime

**Archivos:**
- Modificar: `src/plugin/mod.rs` (registro de tools/providers)
- Modificar: `src/engine/orchestrator.rs` (registro en runtime)

- [ ] **Paso 1:** Permitir que los plugins registren tools y providers en runtime.

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
- [ ] `cache:auto` inyecta cache_control en los breakpoints correctos con TTL.
- [ ] Anthropic extended thinking recibe budget_tokens y se parsea.
- [ ] El system-prompt se renderiza con variables por modelo/agente.
- [ ] Bedrock/Azure/Google se registran y seleccionan desde config.

**FASE 7**
- [ ] Los plugins se cargan y sus hooks/transforms se invocan.
- [ ] Los comandos slash personalizados expanden `{env:VAR}`/`{file:path}`.
- [ ] Los plugins registran tools/providers en runtime.

### Cierre de la release v0.3.0

- [ ] Todas las fases 1-7 (incl. 5.5) completadas y verificadas.
- [ ] `cargo doc --no-deps` genera sin errores.
- [ ] Documentación de nuevas config (keymap, compaction, plugins, providers) actualizada.
- [ ] Rama `develop` con commits atómicos por tarea, cada uno pasando fmt/clippy/test.
