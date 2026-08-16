# Simplificación del modelo de agentes: tools como array + writable_paths

> **Para workers automáticos:** Implementar tarea por tarea en orden. Cada tarea produce código compilable y testeable.

**Objetivo:** Reemplazar el sistema actual de permisos (PermissionConfig/Permissions), ToolSettings con overrides, y task tool dinámico por un modelo simple donde los agentes declaran solo un array de tools y writable_paths.

**Arquitectura:**
- `tools: [string]` — strict allow list. Si no está en la lista, el agente no tiene ese tool.
- `writable_paths: [string]` — rutas adicionales donde se permite escritura (workspace siempre permitido).
- Sin herencia de writable_paths entre padre e hijo. Cada agente declara los suyos.
- Sin sistema de permisos (PermissionConfig, Permissions, Permission enum).
- Sin `task` tool. Los padres solo llaman a subagentes declarados en `subagents:`, que aparecen como tools.
- Sin `ToolSettings` (show/color/display/enabled por agente). Solo `ToolDefaults` global en config.yaml.
- Sin `JobRegistry`, `TaskMode`, `PendingApprovals`, `subagent_depth`.

**Tech Stack:** Rust, serde, serde_yaml

---

## Archivos a modificar/eliminar/crear

| Archivo | Acción |
|---|---|
| `src/config/types.rs` | Modificar: eliminar PermissionConfig, ToolSettings. Cambiar AgentConfig.tools a Vec<String>. Añadir writable_paths. |
| `src/agent/types.rs` | Modificar: eliminar permissions, tool_settings, subagent_depth. Añadir writable_paths. |
| `src/agent/loader.rs` | Modificar: actualizar frontmatter parsing. |
| `src/agent/tools.rs` | Modificar: eliminar task tool, permisos, approvals. Añadir is_write_allowed. |
| `src/agent/lifecycle.rs` | Modificar: eliminar task tool, job registry, approvals, subagent_depth. |
| `src/permissions/` | **Eliminar** módulo completo. |
| `src/engine/orchestrator.rs` | Modificar: eliminar job_registry, pending_approvals, subagent_depth. |
| `src/engine/events.rs` | Modificar: eliminar ApprovalRequired, SubagentFinished, ToolSettingsUpdated. |
| `src/engine/commands.rs` | Modificar: eliminar comando /jobs. |
| `src/engine/jobs.rs` | **Eliminar** archivo. |
| `src/error.rs` | Modificar: eliminar PermissionDenied. |
| `src/tui/app.rs` | Modificar: eliminar UI de approvals, jobs, tool settings. |
| `src/tui/events.rs` | Modificar: eliminar manejo de eventos eliminados. |
| `src/tui/state.rs` | Modificar: eliminar estado de approvals, jobs. |
| `src/lib.rs` | Modificar: eliminar módulo permissions. |
| `src/agent/mod.rs` | Modificar: eliminar re-export si es necesario. |

---

### Task 1: Actualizar tipos base (config/types.rs + agent/types.rs)

**Files:**
- Modify: `src/config/types.rs`
- Modify: `src/agent/types.rs`

**Interfaces:**
- Consumes: tipos actuales AgentConfig, Agent, PermissionConfig, ToolSettings, ToolDefaults
- Produces: AgentConfig con `tools: Vec<String>`, `writable_paths: Vec<PathBuf>`, sin `permissions` ni `tools: HashMap<String, ToolSettings>`

- [ ] **Step 1.1: Modificar AgentConfig en config/types.rs**

Cambiar:
```rust
pub struct AgentConfig {
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub role: AgentRole,
    pub model: String,
    pub skills: Vec<PathBuf>,
    pub mcps: Vec<String>,
    pub permissions: PermissionConfig,         // ← ELIMINAR
    pub subagents: Vec<String>,
    pub system_prompt: String,
    pub max_steps: u32,
    pub subagent_depth: u32,                   // ← ELIMINAR
    pub tools: HashMap<String, ToolSettings>,  // ← CAMBIAR
}
```

A:
```rust
pub struct AgentConfig {
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub role: AgentRole,
    pub model: String,
    pub skills: Vec<PathBuf>,
    pub mcps: Vec<String>,
    pub subagents: Vec<String>,
    pub system_prompt: String,
    pub max_steps: u32,
    pub tools: Vec<String>,
    pub writable_paths: Vec<PathBuf>,
}
```

- [ ] **Step 1.2: Eliminar PermissionConfig, ToolSettings, ToolDefaults**

Eliminar los structs `PermissionConfig`, `ToolSettings`, `ToolDefaults` y sus funciones auxiliares (`default_tool_enabled`, `default_tool_show`).

Mantener `ToolDefaults` solo si se usa desde config.yaml para display global. Si no se usa en ningún sitio, eliminarlo también.

- [ ] **Step 1.3: Modificar Agent en agent/types.rs**

Cambiar:
```rust
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub role: AgentRole,
    pub description: String,
    pub model: String,
    pub skills: Vec<PathBuf>,
    pub mcps: Vec<String>,
    pub permissions: Permissions,                    // ← ELIMINAR
    pub subagent_names: Vec<String>,
    pub parent_id: Option<AgentId>,
    pub max_steps: u32,
    pub subagent_depth: u32,                         // ← ELIMINAR
    pub tool_settings: HashMap<String, ToolSettings>, // ← ELIMINAR
}
```

A:
```rust
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub role: AgentRole,
    pub description: String,
    pub model: String,
    pub skills: Vec<PathBuf>,
    pub mcps: Vec<String>,
    pub subagent_names: Vec<String>,
    pub parent_id: Option<AgentId>,
    pub max_steps: u32,
    pub tools: Vec<String>,
    pub writable_paths: Vec<PathBuf>,
}
```

- [ ] **Step 1.4: Actualizar Agent::from_config() y Agent::create_subagent()**

```rust
impl Agent {
    pub fn from_config(config: &AgentConfig, role: AgentRole) -> Self {
        Self {
            id: AgentId::new(),
            name: config.name.clone(),
            role,
            description: config.system_prompt.clone(),
            model: config.model.clone(),
            skills: config.skills.clone(),
            mcps: config.mcps.clone(),
            subagent_names: config.subagents.clone(),
            parent_id: None,
            max_steps: config.max_steps,
            tools: config.tools.clone(),
            writable_paths: config.writable_paths.clone(),
        }
    }

    pub fn create_subagent(
        name: String,
        description: String,
        model: String,
        skills: Vec<PathBuf>,
        mcps: Vec<String>,
        max_steps: u32,
        parent_id: AgentId,
        tools: Vec<String>,
        writable_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            id: AgentId::new(),
            name,
            role: AgentRole::SubAgent,
            description,
            model,
            skills,
            mcps,
            subagent_names: Vec::new(),
            parent_id: Some(parent_id),
            max_steps,
            tools,
            writable_paths,
        }
    }
}
```

- [ ] **Step 1.5: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 2: Actualizar parser de frontmatter (loader.rs)

**Files:**
- Modify: `src/agent/loader.rs`

**Interfaces:**
- Consumes: AgentConfig actualizado (tools: Vec<String>, writable_paths: Vec<PathBuf>)
- Produces: parse_agent() que parsea el nuevo formato de frontmatter

- [ ] **Step 2.1: Actualizar struct Frontmatter en loader.rs**

```rust
#[derive(serde::Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default)]
    when_to_use: String,
    #[serde(default)]
    role: Option<AgentRole>,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    skills: Vec<PathBuf>,
    #[serde(default)]
    mcps: Vec<String>,
    #[serde(default)]
    subagents: Vec<String>,
    #[serde(default)]
    max_steps: Option<u32>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    writable_paths: Vec<PathBuf>,
}
```

- [ ] **Step 2.2: Actualizar parse_agent()**

```rust
Ok(AgentConfig {
    name: frontmatter.name,
    description: frontmatter.description,
    when_to_use: frontmatter.when_to_use,
    role: frontmatter.role.unwrap_or(AgentRole::SubAgent),
    model: frontmatter.model,
    skills: frontmatter.skills,
    mcps: frontmatter.mcps,
    subagents: frontmatter.subagents,
    system_prompt,
    max_steps: frontmatter.max_steps.unwrap_or(default_max_steps),
    tools: frontmatter.tools,
    writable_paths: frontmatter.writable_paths,
})
```

- [ ] **Step 2.3: Actualizar tests**

Actualizar `test_parse_agent_full_frontmatter`, `test_merge_agents_project_overrides_global`, etc. para usar el nuevo formato.

- [ ] **Step 2.4: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 3: Eliminar módulo de permisos

**Files:**
- Delete: `src/permissions/types.rs`
- Delete: `src/permissions/checker.rs`
- Delete: `src/permissions/mod.rs`
- Modify: `src/lib.rs` (eliminar `mod permissions`)

- [ ] **Step 3.1: Eliminar archivos del módulo permissions**

```bash
rm src/permissions/types.rs src/permissions/checker.rs src/permissions/mod.rs
```

- [ ] **Step 3.2: Eliminar referencia en lib.rs**

```bash
grep -n "mod permissions" src/lib.rs
# Eliminar esa línea
```

- [ ] **Step 3.3: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 4: Simplificar agent/tools.rs

**Files:**
- Modify: `src/agent/tools.rs`

**Interfaces:**
- Consumes: Agent sin permissions ni tool_settings
- Produces: tools.rs sin check_tool_permission, task tool, approvals. Con is_write_allowed().

- [ ] **Step 4.1: Eliminar imports de permissions**

Eliminar:
```rust
use crate::permissions::checker::{
    check_command_run, check_fs_read, check_fs_write, check_mcp_use, check_net_http,
    check_skill_use,
};
use crate::permissions::types::Permissions;
```

- [ ] **Step 4.2: Eliminar check_tool_permission(), classify_tool_operation(), is_sensitive_operation()**

Eliminar las tres funciones completas (~180 líneas).

- [ ] **Step 4.3: Eliminar task_tool_definition(), execute_task_tool(), TaskToolArgs, SpawnSubagentConfig, spawn_subagent_and_delegate()**

Eliminar todo el bloque del task tool (~300 líneas).

- [ ] **Step 4.4: Eliminar plan_mode_blocked()**

Eliminar función (~40 líneas).

- [ ] **Step 4.5: Añadir is_write_allowed()**

```rust
/// Check if a path is allowed for write operations.
/// The workspace is always writable. Additional paths can be declared
/// in the agent's `writable_paths`.
pub fn is_write_allowed(path: &Path, workspace: &Path, writable_paths: &[PathBuf]) -> bool {
    if path.starts_with(workspace) {
        return true;
    }
    writable_paths.iter().any(|p| path.starts_with(p))
}
```

- [ ] **Step 4.6: Simplificar execute_apply_patch_tool()**

Eliminar el parámetro `permissions: &Permissions` y `pending_approvals`. Añadir `workspace: &Path` y `writable_paths: &[PathBuf]`. Usar `is_write_allowed()` en lugar de `check_fs_write()`.

```rust
pub(crate) async fn execute_apply_patch_tool(
    workspace: &Path,
    writable_paths: &[PathBuf],
    event_tx: &mpsc::Sender<EngineEvent>,
    agent_name: &str,
    tool_call: &ToolCall,
) -> std::result::Result<String, String> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.function.arguments)
        .map_err(|e| format!("Failed to parse apply_patch arguments: {e}"))?;

    let json = args
        .get("operations")
        .map(|v| v.to_string())
        .unwrap_or_else(|| tool_call.function.arguments.clone());

    let batch = crate::engine::apply_patch::parse_patch_batch(&json)?;

    // Validate every path: must be within workspace or writable_paths
    for op in &batch.operations {
        let resolved = crate::engine::apply_patch::resolve_within_workspace(workspace, &op.path)
            .map_err(|e| format!("Path validation failed: {e}"))?;
        if !is_write_allowed(&resolved, workspace, writable_paths) {
            return Err(format!(
                "Write not allowed for path: {} (not in workspace or writable_paths)",
                op.path
            ));
        }
    }

    let results = crate::engine::apply_patch::apply_patch_batch(workspace, &batch, false)?;

    // Emit a unified diff for the TUI diff viewer.
    let diff_text = crate::engine::apply_patch::batch_to_unified_diff(&batch);
    let _ = event_tx
        .send(EngineEvent::DiffAvailable {
            text: diff_text,
            title: format!("apply_patch — {}", agent_name),
        })
        .await;

    Ok(results.join("\n"))
}
```

- [ ] **Step 4.7: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 5: Simplificar agent/lifecycle.rs

**Files:**
- Modify: `src/agent/lifecycle.rs`

**Interfaces:**
- Consumes: Agent sin permissions/tool_settings, tools como Vec<String>
- Produces: lifecycle sin task tool, job registry, approvals, subagent_depth

- [ ] **Step 5.1: Eliminar imports obsoletos**

Eliminar imports de:
- `execute_task_tool`, `spawn_subagent_and_delegate`, `task_tool_definition` de agent/tools
- `JobRegistry`
- `TaskMode`
- `PendingApprovals`

- [ ] **Step 5.2: Eliminar builtin_tool_definitions() el task tool**

```rust
pub fn builtin_tool_definitions() -> HashMap<String, ToolDefinition> {
    let mut map = HashMap::new();
    for def in [
        todo_tool_definition(),
        question_tool_definition(),
        apply_patch_tool_definition(),
        fs_tool_definition(),
        execute_tool_definition(),
        search_tool_definition(),
        webfetch_tool_definition(),
        websearch_tool_definition(),
        mcp_list_resources_tool_definition(),
        mcp_read_resource_tool_definition(),
        mcp_list_resource_templates_tool_definition(),
        lsp_query_tool_definition(),
        format_document_tool_definition(),
        search_symbol_tool_definition(),
        // task_tool_definition() ← ELIMINAR
    ] {
        map.insert(def.name.clone(), def);
    }
    map
}
```

- [ ] **Step 5.3: Simplificar SpawnAgentConfig**

Eliminar campos:
- `job_registry`
- `concurrency_semaphore`
- `pending_approvals`
- `tool_defaults` (opcional, mantener si se usa para display global)

- [ ] **Step 5.4: Simplificar spawn_agent()**

La construcción de tools cambia de:
```rust
// ANTES: iterar sobre tool_settings HashMap
let builtin_tools = builtin_tool_definitions();
for (tool_name, agent_settings) in &tool_settings_clone {
    if !agent_settings.enabled { continue; }
    if let Some(mut def) = builtin_tools.get(tool_name).cloned() {
        tools.push(def);
    }
}
```

A:
```rust
// DESPUÉS: iterar sobre tools Vec<String>
let builtin_tools = builtin_tool_definitions();
for tool_name in &agent.tools {
    if let Some(def) = builtin_tools.get(tool_name).cloned() {
        tools.push(def);
    }
}
```

- [ ] **Step 5.5: Eliminar lógica de background jobs y approvals**

En el tool loop, eliminar:
- Manejo de `PendingApprovals`
- Spawning de background tasks
- Referencias a `job_registry`

- [ ] **Step 5.6: Simplificar should_emit_tool() y resolve_tool_preview()**

Estas funciones usaban `ToolSettings`. Simplificarlas o eliminarlas.

```rust
/// Check whether tool execution should be shown in the chat.
/// Now always returns true since per-agent show/hide is removed.
fn should_emit_tool(_tool_name: &str) -> bool {
    true
}
```

```rust
/// Resolve display template for a tool.
/// Falls back to extract_task_preview since per-agent templates are removed.
pub fn resolve_tool_preview(tool_name: &str, args: &str) -> String {
    extract_task_preview(tool_name, args)
}
```

- [ ] **Step 5.7: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 6: Actualizar engine

**Files:**
- Modify: `src/engine/orchestrator.rs`
- Modify: `src/engine/events.rs`
- Modify: `src/engine/commands.rs`
- Delete: `src/engine/jobs.rs`
- Modify: `src/engine/mod.rs`
- Modify: `src/error.rs`

- [ ] **Step 6.1: Eliminar JobRegistry**

```bash
rm src/engine/jobs.rs
```

Eliminar `pub mod jobs;` de `src/engine/mod.rs`.

- [ ] **Step 6.2: Eliminar eventos obsoletos de EngineEvent**

En `src/engine/events.rs`, eliminar variantes:
- `ApprovalRequired`
- `SubagentFinished`
- `ToolSettingsUpdated`

- [ ] **Step 6.3: Eliminar EngineCommand obsoletos**

En `src/engine/events.rs`, eliminar variantes relacionadas con jobs y approvals.

- [ ] **Step 6.4: Simplificar Engine en orchestrator.rs**

Eliminar campos:
- `job_registry: Arc<tokio::sync::Mutex<JobRegistry>>`
- `pending_approvals: Arc<tokio::sync::Mutex<HashMap<String, oneshot::Sender<bool>>>>`
- `concurrency_semaphore`

Eliminar lógica de:
- `/jobs` command handling
- Approval request/response handling
- Subagent depth enforcement

- [ ] **Step 6.5: Eliminar comando /jobs de commands.rs**

- [ ] **Step 6.6: Eliminar PermissionDenied de error.rs**

```rust
pub enum Error {
    // PermissionDenied(String), ← ELIMINAR
}
```

- [ ] **Step 6.7: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 7: Actualizar TUI

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/events.rs`
- Modify: `src/tui/state.rs`

- [ ] **Step 7.1: Eliminar UI de approvals**

En `src/tui/app.rs`, eliminar:
- Renderizado de diálogo de approval
- Manejo de eventos `ApprovalRequired`
- Estado de approvals pendientes

- [ ] **Step 7.2: Eliminar UI de jobs**

Eliminar:
- Vista de jobs en la TUI
- Manejo de `SubagentFinished`
- Comando `/jobs`

- [ ] **Step 7.3: Eliminar ToolSettingsUpdated handling**

Eliminar manejo de evento `ToolSettingsUpdated`.

- [ ] **Step 7.4: Compilar y verificar**

```bash
cargo check 2>&1 | head -50
```

---

### Task 8: Compilar, testear y corregir

- [ ] **Step 8.1: Compilar todo el proyecto**

```bash
cargo build 2>&1
```

- [ ] **Step 8.2: Ejecutar tests**

```bash
cargo test 2>&1
```

- [ ] **Step 8.3: Ejecutar clippy**

```bash
cargo clippy 2>&1
```

- [ ] **Step 8.4: Corregir errores de compilación, tests y clippy**

Iterar hasta que todo pase.

- [ ] **Step 8.5: Formatear código**

```bash
cargo fmt
```

- [ ] **Step 8.6: Commit final**

```bash
git add -A && git commit -m "refactor: simplify agent model - tools as array, writable_paths, remove permissions and task tool"
```