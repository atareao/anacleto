# Slash Commands estilo OpenCode — Implementation Plan

## Objetivo

Añadir al motor de orquestación Anacleto un conjunto de slash commands estilo OpenCode (`/undo`, `/fork`, `/export`, `/share`, `/skills`, `/themes`, etc.) que se ejecutan desde la TUI, cubriendo tanto lógica de backend (engine + SQLite) como estado puramente de frontend (TUI).

## Alcance

### Comandos en alcance

| Comando | Alias | Descripción | Capa |
|---|---|---|---|
| `/undo` | — | Deshacer el último par de mensajes de la sesión activa | Backend |
| `/redo` | — | Rehacer el último par deshecho | Backend |
| `/fork` | — | Bifurcar la sesión activa en una nueva sesión | Backend |
| `/export` | — | Exportar la transcripción de la sesión a un archivo (markdown/JSON) | Backend |
| `/import` | — | Importar una transcripción desde un archivo | Backend |
| `/share` | — | Marcar la sesión como compartida y generar un enlace | Backend |
| `/unshare` | — | Quitar el estado compartido de la sesión | Backend |
| `/skills` | — | Listar skills disponibles del agente activo | Backend |
| `/mcps` | — | Listar y activar/desactivar servidores MCP | Backend |
| `/status` | — | Mostrar estado del motor (modelo, sesión, tokens, coste, debug, workspace) | Backend |
| `/init` | — | Setup guiado de AGENTS.md (prompts interactivos) | Backend |
| `/review` | — | Revisar cambios de git (diff sin commit por defecto; arg opcional commit/branch) | Backend |
| `/warp` | — | Establecer el directorio de trabajo | Backend |
| `/workspaces` | — | Gestionar/listar workspaces | Backend |
| `/timeline` | — | Mostrar línea temporal de mensajes de la sesión activa (saltar a un mensaje) | Backend |
| `/themes` | — | Cambiar el tema de color | TUI |
| `/timestamps` | — | Alternar marcas de tiempo en los mensajes | TUI |
| `/thinking` | — | Alternar la visualización de razonamiento/thinking | TUI |
| `/stash` | — | Guardar el prompt actual (con pop/list) | TUI |
| `/editor` | — | Abrir editor externo para la entrada | TUI |
| `/move` | — | Mover la sesión a otro workspace | Backend |

### Comandos fuera de alcance (solo cloud de OpenCode)

`/connect`, `/org`, `/variants`, `/upgrade`, `/doctor`, `/login`, `/logout`, `/config`, `/permissions`, `/hooks`, `/context`, `/cost`, `/usage`, `/templates`, `/prompt`.

### Comandos ya implementados (se conservan)

`/help`, `/sessions`, `/new`, `/resume`, `/delete`, `/rename`, `/agents`, `/subagents`, `/copy`, `/compact`, `/debug`, `/models`, `/exit`.

## Arquitectura / decisiones de diseño

### Flujo de un comando

Todo comando sigue el mismo pipeline de 4 etapas:

1. **`COMMANDS` const** (`src/tui/app.rs`): se registra el par `("cmd", "descripción")` para que aparezca en la paleta fuzzy y en el autocompletado con Tab.
2. **`process_input` → `handle_command`** (`src/tui/app.rs`): se añade un `match` sobre `parts[0]` que construye el `EngineCommand` correspondiente y lo envía por `command_tx`.
3. **`Engine::run()`** (`src/engine/orchestrator.rs`): el bucle despacha el nuevo `EngineCommand` a su handler. El handler ejecuta la lógica (DB, git, config) y emite un `EngineEvent` de vuelta a la TUI.
4. **`handle_event(EngineEvent)`** (`src/tui/app.rs`): la TUI reacciona actualizando su estado y re-renderizando.

Regla general: **toda mutación de estado persistente ocurre en el engine**; la TUI solo muestra resultados y mantiene estado efímero (temas, timestamps, stash).

### Nuevos `EngineCommand` (src/engine/orchestrator.rs, enum líneas 163-186)

Se añaden variantes: `Undo`, `Redo`, `Fork`, `Export { path: Option<PathBuf>, format: Option<ExportFormat> }`, `Import { path: PathBuf }`, `Share`, `Unshare`, `ListSkills`, `ListMcps`, `ToggleMcp { name: String, enabled: bool }`, `Status`, `Init { answers: InitAnswers }`, `Review { target: Option<String> }`, `Warp { dir: PathBuf }`, `ListWorkspaces`, `MoveSession { workspace: String }`, `Timeline`.

Los comandos puramente TUI (`/themes`, `/timestamps`, `/thinking`, `/stash`, `/editor`) **no** generan `EngineCommand`; se manejan íntegramente en `handle_command`/`handle_event` de `app.rs`.

### Nuevos `EngineEvent`

Se añaden variantes para que la TUI reaccione: `UndoApplied`, `RedoApplied`, `Forked { new_session_id: Uuid }`, `Exported { path: PathBuf }`, `Imported { session_id: Uuid }`, `ShareUpdated { shared: bool, link: Option<String> }`, `SkillsListed(Vec<SkillInfo>)`, `McpsListed(Vec<McpStatus>)`, `StatusReport(StatusInfo)`, `InitDone`, `ReviewResult(String)`, `WorkspaceChanged(PathBuf)`, `WorkspacesListed(Vec<String>)`, `Timeline(Vec<TimelineEntry>)`, `SessionMoved { session_id: Uuid, workspace: String }`.

### Diseño del stack undo/redo

- El engine mantiene `undo_stack: Vec<Vec<StoredMessage>>` y `redo_stack: Vec<Vec<StoredMessage>>` (campos nuevos en `Engine`).
- `/undo`: toma el último par de mensajes de la sesión activa (el `UserInput` + su `Response`), los elimina de la DB (`delete_messages`), los empuja a `undo_stack` y a `redo_stack`, y emite `UndoApplied`.
- `/redo`: hace `pop` de `redo_stack`, reinserta los mensajes en la DB (`restore_messages`) y emite `RedoApplied`.
- El stack se limpia al cambiar de sesión (`/new`, `/resume`, `/fork`).
- La DB es la fuente de verdad; el stack solo guarda los mensajes eliminados para poder restaurarlos (la eliminación es destructiva).

### Modelo de datos para fork/share/export

- **Fork**: `create_session` con un nuevo id, luego `copy_messages(from_session, to_session)` que inserta copias de los mensajes de la sesión activa en la nueva. Se emite `Forked { new_session_id }` y se activa la nueva sesión.
- **Share**: se añade una columna `shared INTEGER NOT NULL DEFAULT 0` y `metadata TEXT` (JSON) a la tabla `sessions`. `/share` pone `shared=1` y escribe en `metadata` un `share_link` generado (UUID). `/unshare` pone `shared=0` y limpia el enlace. Métodos `set_shared(session_id, shared, link)` y `get_session_metadata`.
- **Export/Import**: formato JSON recomendado para fidelidad de ida y vuelta (contiene `session_id`, `title`, `created_at`, `messages[]` con `role`, `content`, `timestamp`); markdown para lectura humana. `export_session(session_id, path, format)` y `import_session(path) -> Uuid` (crea una sesión nueva y reinserta los mensajes).

### Manejo de workspaces

- `/warp <dir>`: actualiza el directorio de trabajo del engine (campo `workspace: PathBuf` en `Engine`/`Config`) y emite `WorkspaceChanged`.
- `/workspaces`: lista los workspaces conocidos (directorios registrados en config, p. ej. `config.workspaces: Vec<PathBuf>`).
- `/move <workspace>`: mueve la sesión activa a otro workspace actualizando su `workspace` en la DB (columna nueva `workspace TEXT`) y emite `SessionMoved`.

### Comandos interactivos

- `/init`: la TUI muestra prompts secuenciales (nombre, descripción, stack tecnológico) recogiendo respuestas en `InitAnswers`; al completarse se envía `EngineCommand::Init { answers }` y el engine escribe `AGENTS.md` en el workspace. Emite `InitDone`.
- `/review`: el engine ejecuta `git diff` (o `git diff <target>`) vía `std::process::Command`, captura la salida y la envía al agente root con `send_to_root(AgentMessage::UserInput(...))` para su revisión. Emite `ReviewResult` con el resumen.

## Tareas de backend (backend-dev)

### Tarea B1: Ampliar el enum `EngineCommand`

**Archivos:**
- Modificar: `src/engine/orchestrator.rs:163-186`

- [ ] **Paso 1:** Añadir las variantes nuevas al enum `EngineCommand` (Undo, Redo, Fork, Export, Import, Share, Unshare, ListSkills, ListMcps, ToggleMcp, Status, Init, Review, Warp, ListWorkspaces, MoveSession, Timeline) con sus tipos de datos asociados.
- [ ] **Paso 2:** Definir los tipos auxiliares `ExportFormat` (Markdown | Json), `InitAnswers { name, description, stack }`, `SkillInfo`, `McpStatus`, `StatusInfo`, `TimelineEntry` en el módulo de tipos del engine.
- [ ] **Criterio:** `cargo build` compila con las nuevas variantes (aunque aún sin handlers).

### Tarea B2: Añadir campos de estado al `Engine`

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Añadir `undo_stack: Vec<Vec<StoredMessage>>`, `redo_stack: Vec<Vec<StoredMessage>>` y `workspace: PathBuf` como campos de `Engine`.
- [ ] **Paso 2:** Inicializarlos en el constructor de `Engine` (workspace desde config o directorio actual).
- [ ] **Criterio:** `cargo build` compila; los stacks se limpian al cambiar de sesión.

### Tarea B3: Métodos de DB para undo/redo

**Archivos:**
- Modificar: `src/db/session.rs`, `src/db/models.rs`

- [ ] **Paso 1:** Añadir `delete_messages(session_id, limit) -> Vec<StoredMessage>` que borra los últimos N mensajes y devuelve los borrados.
- [ ] **Paso 2:** Añadir `restore_messages(session_id, &[StoredMessage])` que reinserta mensajes (con sus ids originales).
- [ ] **Criterio:** tests unitarios de `delete_messages`/`restore_messages` en `src/db/session.rs` pasan.

### Tarea B4: Métodos de DB para fork

**Archivos:**
- Modificar: `src/db/session.rs`

- [ ] **Paso 1:** Añadir `copy_messages(from_session, to_session)` que copia todos los mensajes de una sesión a otra.
- [ ] **Criterio:** test unitario que verifica que tras `fork` la nueva sesión tiene los mismos mensajes.

### Tarea B5: Métodos de DB para share

**Archivos:**
- Modificar: `src/db/session.rs`, `src/db/models.rs`, migraciones SQL

- [ ] **Paso 1:** Añadir columnas `shared INTEGER NOT NULL DEFAULT 0` y `metadata TEXT` a la tabla `sessions` (migración nueva en `run_migrations`).
- [ ] **Paso 2:** Añadir `set_shared(session_id, shared, link)` y `get_session_metadata(session_id)`.
- [ ] **Criterio:** migración aplica sin romper sesiones existentes; test de `set_shared`/`get_session_metadata` pasa.

### Tarea B6: Métodos de DB para export/import

**Archivos:**
- Modificar: `src/db/session.rs`

- [ ] **Paso 1:** Añadir `export_session(session_id, path, format)` que serializa la transcripción a JSON o markdown y la escribe a disco.
- [ ] **Paso 2:** Añadir `import_session(path) -> Uuid` que lee el archivo, crea una sesión nueva y reinserta los mensajes.
- [ ] **Criterio:** round-trip export→import preserva el contenido de los mensajes (test unitario).

### Tarea B7: Método de DB para move

**Archivos:**
- Modificar: `src/db/session.rs`, `src/db/models.rs`

- [ ] **Paso 1:** Añadir columna `workspace TEXT` a `sessions` y método `set_session_workspace(session_id, workspace)`.
- [ ] **Criterio:** test unitario de `set_session_workspace` pasa.

### Tarea B8: Handlers de undo/redo/fork en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar el handler de `Undo`: obtener la sesión activa, `delete_messages` del último par, empujar a `undo_stack`/`redo_stack`, emitir `UndoApplied`.
- [ ] **Paso 2:** Implementar el handler de `Redo`: `pop` de `redo_stack`, `restore_messages`, emitir `RedoApplied`.
- [ ] **Paso 3:** Implementar el handler de `Fork`: `create_session` + `copy_messages`, activar la nueva sesión, emitir `Forked`.
- [ ] **Criterio:** `cargo test` pasa; los stacks se limpian al cambiar de sesión.

### Tarea B9: Handlers de share/unshare en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `Share`: generar un `share_link` (UUID), `set_shared(true, link)`, emitir `ShareUpdated`.
- [ ] **Paso 2:** Implementar `Unshare`: `set_shared(false, None)`, emitir `ShareUpdated`.
- [ ] **Criterio:** `cargo test` pasa; el enlace se persiste en `metadata`.

### Tarea B10: Handlers de export/import en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `Export`: resolver formato (por defecto JSON), llamar `export_session`, emitir `Exported`.
- [ ] **Paso 2:** Implementar `Import`: llamar `import_session`, activar la sesión importada, emitir `Imported`.
- [ ] **Criterio:** `cargo test` pasa; round-trip export→import funciona de extremo a extremo.

### Tarea B11: Handlers de skills/mcps/status en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `ListSkills`: recoger skills del agente activo desde `agents`/config y emitir `SkillsListed`.
- [ ] **Paso 2:** Implementar `ListMcps`/`ToggleMcp`: consultar `mcp_registry` (Arc<Mutex<McpRegistry>>) para listar y activar/desactivar servidores; emitir `McpsListed`.
- [ ] **Paso 3:** Implementar `Status`: componer `StatusInfo` (modelo, sesión activa, tokens/coste si se trackean, debug, workspace) y emitir `StatusReport`.
- [ ] **Criterio:** `cargo test` pasa; los datos reflejan el estado real del engine.

### Tarea B12: Handler de init en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `Init`: generar `AGENTS.md` en el workspace a partir de `InitAnswers` y emitir `InitDone`.
- [ ] **Criterio:** `cargo test` pasa; el archivo `AGENTS.md` se crea con el contenido esperado.

### Tarea B13: Handler de review en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `Review`: ejecutar `git diff` (o `git diff <target>`) con `std::process::Command`, capturar stdout, enviarlo al agente root vía `send_to_root(AgentMessage::UserInput(...))` y emitir `ReviewResult`.
- [ ] **Criterio:** `cargo test` pasa; el diff se envía al agente root correctamente.

### Tarea B14: Handlers de warp/workspaces/move/timeline en el engine

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Implementar `Warp`: actualizar `workspace` del engine y emitir `WorkspaceChanged`.
- [ ] **Paso 2:** Implementar `ListWorkspaces`: listar desde config y emitir `WorkspacesListed`.
- [ ] **Paso 3:** Implementar `MoveSession`: `set_session_workspace` y emitir `SessionMoved`.
- [ ] **Paso 4:** Implementar `Timeline`: recuperar mensajes de la sesión activa y emitir `Timeline`.
- [ ] **Criterio:** `cargo test` pasa; cada handler emite su evento correspondiente.

## Tareas de frontend/TUI (frontend-dev)

### Tarea T1: Registrar comandos en `COMMANDS` y `handle_command`

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Añadir a `COMMANDS: &[(&str,&str)]` las entradas de todos los comandos nuevos (backend y TUI) con su descripción.
- [ ] **Paso 2:** En `handle_command`, añadir los `match` arms que construyen y envían los `EngineCommand` de backend por `command_tx`.
- [ ] **Criterio:** los comandos aparecen en la paleta fuzzy y en el autocompletado con Tab.

### Tarea T2: Manejar eventos de backend en `handle_event`

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Añadir arms en `handle_event(EngineEvent)` para `UndoApplied`, `RedoApplied`, `Forked`, `Exported`, `Imported`, `ShareUpdated`, `SkillsListed`, `McpsListed`, `StatusReport`, `InitDone`, `ReviewResult`, `WorkspaceChanged`, `WorkspacesListed`, `Timeline`, `SessionMoved`.
- [ ] **Paso 2:** Cada arm actualiza el estado de la TUI (mensajes, sesión activa, paneles de listado) y fuerza re-render.
- [ ] **Criterio:** `cargo build` compila; los eventos actualizan la UI correctamente.

### Tarea T3: Comandos TUI puros — themes/timestamps/thinking

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Añadir campos de estado `theme: Theme`, `show_timestamps: bool`, `show_thinking: bool` al `App`.
- [ ] **Paso 2:** Implementar `/themes` (ciclar/escoger tema), `/timestamps` (toggle) y `/thinking` (toggle) íntegramente en `handle_command`, sin `EngineCommand`.
- [ ] **Paso 3:** Aplicar `show_timestamps`/`show_thinking` en el renderizado de mensajes.
- [ ] **Criterio:** los toggles cambian el render en vivo sin tocar el engine.

### Tarea T4: Comando TUI puro — stash

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Añadir `stash_stack: Vec<String>` al `App`.
- [ ] **Paso 2:** Implementar `/stash` (guardar buffer actual), `/stash pop` (recuperar último) y `/stash list` (mostrar stack).
- [ ] **Criterio:** el buffer se guarda/recupera correctamente entre comandos.

### Tarea T5: Comando TUI puro — editor

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Implementar `/editor`: leer `$EDITOR` (o fallback), escribir el buffer actual a un archivo temporal, lanzar el editor con `std::process::Command`, leer el archivo de vuelta al buffer.
- [ ] **Criterio:** al cerrar el editor, el contenido editado se carga en el buffer de entrada.

### Tarea T6: Comando interactivo — init

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Implementar el flujo de prompts de `/init` (nombre, descripción, stack) recogiendo respuestas en `InitAnswers`.
- [ ] **Paso 2:** Al completar, enviar `EngineCommand::Init { answers }` y manejar `InitDone`.
- [ ] **Criterio:** el flujo guiado recoge las respuestas y dispara la generación de `AGENTS.md`.

### Tarea T7: Comando interactivo — timeline

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Implementar `/timeline`: enviar `EngineCommand::Timeline`, mostrar la lista de mensajes y permitir saltar a un mensaje seleccionado.
- [ ] **Criterio:** el usuario puede navegar la línea temporal y saltar a un mensaje.

### Tarea T8: Comando interactivo — mcps

**Archivos:**
- Modificar: `src/tui/app.rs`

- [ ] **Paso 1:** Implementar `/mcps`: enviar `ListMcps`, mostrar la lista con estado on/off y permitir activar/desactivar (enviar `ToggleMcp`).
- [ ] **Criterio:** el usuario puede listar y alternar servidores MCP desde la TUI.

## Orden de implementación y dependencias

1. **B1** (enum) → base para todo el backend.
2. **B2** (estado del engine) → necesario para undo/redo y warp.
3. **B3–B7** (métodos de DB) → dependen de B1; pueden ir en paralelo entre sí.
4. **B8–B14** (handlers) → dependen de B2 y de los métodos de DB correspondientes.
5. **T1–T2** (registro + eventos) → dependen de B1 y de los handlers; son el esqueleto de la TUI.
6. **T3–T5** (comandos TUI puros) → independientes; pueden ir en paralelo con el backend.
7. **T6–T8** (comandos interactivos) → dependen de T1/T2 y de los handlers de init/timeline/mcps.

**Ruta crítica:** B1 → B2 → (B3–B7) → B8–B14 → T1 → T2 → T6–T8. Los comandos TUI puros (T3–T5) se pueden implementar en cualquier momento.

## Criterios de aceptación finales

### Verificación estática y de tests

- [ ] `cargo fmt --check` pasa sin cambios pendientes.
- [ ] `cargo clippy` pasa sin warnings.
- [ ] `cargo test` pasa (incluidos los nuevos tests de DB y engine).

### Verificación manual E2E por comando

- [ ] `/undo` elimina el último par de mensajes; `/redo` lo restaura.
- [ ] `/fork` crea una nueva sesión con los mensajes copiados y la activa.
- [ ] `/export` escribe un archivo JSON/markdown; `/import` lo lee y crea una sesión con el contenido.
- [ ] `/share` marca la sesión como compartida y muestra un enlace; `/unshare` lo revierte.
- [ ] `/skills` lista los skills del agente activo.
- [ ] `/mcps` lista los servidores MCP y permite activar/desactivar.
- [ ] `/status` muestra modelo, sesión, tokens, coste, debug y workspace correctos.
- [ ] `/init` guía la creación de `AGENTS.md` en el workspace.
- [ ] `/review` envía el diff de git al agente root para su revisión.
- [ ] `/warp` cambia el directorio de trabajo; `/workspaces` los lista; `/move` mueve la sesión.
- [ ] `/timeline` muestra la línea temporal y permite saltar a un mensaje.
- [ ] `/themes` cambia el tema; `/timestamps` y `/thinking` alternan su visualización en vivo.
- [ ] `/stash` guarda/recupera el prompt; `/editor` abre el editor y carga el contenido editado.
- [ ] Los comandos ya implementados (`/help`, `/sessions`, `/new`, etc.) siguen funcionando sin regresiones.
- [ ] Los comandos cloud de OpenCode (fuera de alcance) no se registran ni se muestran.
