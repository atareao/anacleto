# Workspace obligatorio para subagentes — Implementation Plan

## Objetivo

Propagar el `workspace` del agente padre al subagente haciendo que sea un campo requerido en el `task` tool, de modo que el subagente sepa en qué directorio trabajar.

## Arquitectura

Se añade `workspace: PathBuf` como campo requerido en `TaskToolArgs`, `SpawnSubagentConfig`, y como parámetro de `execute_task_tool`. En `spawn_subagent_and_delegate` se usa para renderizar el system prompt del subagente (inyectando `{workspace}` via `render_template`), replicando lo que ya hace `spawn_agent` en `lifecycle.rs`.

## Tareas

### Tarea 1: Añadir `workspace` a `TaskToolArgs` y su parseo

**Archivos:**
- Modificar: `src/agent/tools.rs:1135-1195`

- [ ] **Paso 1.1:** Añadir `use std::path::PathBuf;` si no existe ya al inicio del archivo.

- [ ] **Paso 1.2:** Añadir el campo `workspace: PathBuf` a la struct `TaskToolArgs`:

```rust
struct TaskToolArgs {
    task_id: String,
    description: String,
    mode: TaskMode,
    model: Option<String>,
    tools: Vec<String>,
    /// Optional name of a configured subagent type (e.g. "reviewer") used as
    /// the template for this subagent. When `None`, a dynamic subagent is
    /// created from the task description.
    agent: Option<String>,
    /// The workspace directory where the subagent will operate.
    workspace: PathBuf,
}
```

- [ ] **Paso 1.3:** En el método `parse`, añadir el parseo de `workspace` entre el parseo de `tools` y `agent`:

```rust
        let workspace = args
            .get("workspace")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "task requires 'workspace'".to_string())?
            .to_string();
```

Y añadirlo al `Ok(Self { ... })`:

```rust
        Ok(Self {
            task_id,
            description,
            mode,
            model,
            tools,
            workspace: PathBuf::from(workspace),
            agent,
        })
```

### Tarea 2: Actualizar `task_tool_definition()` schema

**Archivos:**
- Modificar: `src/agent/tools.rs:1200-1260`

- [ ] **Paso 2.1:** Añadir la propiedad `"workspace"` al objeto `"properties"` en `input_schema`:

```rust
                "workspace": {
                    "type": "string",
                    "description": "The workspace directory where the subagent will operate."
                },
```

- [ ] **Paso 2.2:** Añadir `"workspace"` al array `"required"`:

```rust
            "required": ["task_id", "description", "workspace"]
```

### Tarea 3: Añadir `workspace` a `SpawnSubagentConfig`

**Archivos:**
- Modificar: `src/agent/tools.rs:1517-1550`

- [ ] **Paso 3.1:** Añadir el campo `pub workspace: PathBuf` a la struct `SpawnSubagentConfig`:

```rust
pub(crate) struct SpawnSubagentConfig {
    pub(crate) parent_id: AgentId,
    pub(crate) parent_name: String,
    pub(crate) task_id: String,
    pub(crate) description: String,
    pub(crate) mode: TaskMode,
    pub(crate) model: Option<String>,
    pub(crate) tools: Vec<String>,
    pub(crate) workspace: PathBuf,
    pub(crate) permissions: Permissions,
    pub(crate) event_tx: mpsc::Sender<EngineEvent>,
    pub(crate) usage_tx: Option<mpsc::Sender<UsageEvent>>,
    pub(crate) db: Option<Database>,
    pub(crate) session_id: Option<Uuid>,
    pub(crate) history_limit_percent: f64,
    pub(crate) retry_config: RetryConfig,
    pub(crate) debug: Arc<AtomicBool>,
    pub(crate) depth: u32,
    pub(crate) subagent_depth: u32,
    pub(crate) job_registry: Option<Arc<tokio::sync::Mutex<JobRegistry>>>,
    pub(crate) agent: Option<AgentConfig>,
    pub(crate) llm_registry: LlmProviderRegistry,
    pub(crate) skill_registry: crate::skill::registry::SharedSkillRegistry,
    pub(crate) skill_names: Vec<String>,
}
```

### Tarea 4: Añadir parámetro `workspace` a `execute_task_tool`

**Archivos:**
- Modificar: `src/agent/tools.rs:1285-1515`

- [ ] **Paso 4.1:** Añadir `workspace: &Path` como nuevo parámetro en la firma de `execute_task_tool`:

```rust
pub(crate) async fn execute_task_tool(
    tool_call: &ToolCall,
    parent_permissions: &Permissions,
    llm_registry: &LlmProviderRegistry,
    parent_skill_registry: &crate::skill::registry::SharedSkillRegistry,
    parent_skill_names: &[String],
    event_tx: &mpsc::Sender<EngineEvent>,
    usage_tx: &Option<mpsc::Sender<UsageEvent>>,
    db: &Option<Database>,
    session_id: Option<Uuid>,
    history_limit_percent: f64,
    retry_config: &RetryConfig,
    debug: &Arc<AtomicBool>,
    depth: u32,
    subagent_depth: u32,
    parent_name: &str,
    parent_id: &AgentId,
    parent_model: &str,
    job_registry: &Option<Arc<tokio::sync::Mutex<JobRegistry>>>,
    subagent_configs: &[AgentConfig],
    workspace: &Path,
) -> std::result::Result<String, String> {
```

### Tarea 5: Pasar `workspace` en ambas ramas de `execute_task_tool`

**Archivos:**
- Modificar: `src/agent/tools.rs` (dentro de `execute_task_tool`, ~líneas 1310-1400)

- [ ] **Paso 5.1:** En la rama donde se construye `SpawnSubagentConfig` para un agente configurado (`if let Some(agent_name) = &args.agent`), añadir `workspace: workspace.to_path_buf()`:

```rust
            let sub_cfg = SpawnSubagentConfig {
                parent_id: parent_id.clone(),
                parent_name: parent_name.to_string(),
                task_id: args.task_id.clone(),
                description: args.description.clone(),
                mode: args.mode,
                model: args.model.clone(),
                tools: args.tools.clone(),
                workspace: workspace.to_path_buf(),
                permissions: config.permissions.clone(),
                event_tx: event_tx.clone(),
                usage_tx: usage_tx.clone(),
                db: db.clone(),
                session_id,
                history_limit_percent,
                retry_config: retry_config.clone(),
                debug: debug.clone(),
                depth: depth + 1,
                subagent_depth,
                job_registry: job_registry.clone(),
                agent: Some(config),
                llm_registry: llm_registry.clone(),
                skill_registry: skill_registry.clone(),
                skill_names: skill_names.to_vec(),
            };
```

- [ ] **Paso 5.2:** En la rama del agente dinámico (`else`), hacer lo mismo:

```rust
            let sub_cfg = SpawnSubagentConfig {
                parent_id: parent_id.clone(),
                parent_name: parent_name.to_string(),
                task_id: args.task_id.clone(),
                description: args.description.clone(),
                mode: args.mode,
                model: args.model.clone(),
                tools: args.tools.clone(),
                workspace: workspace.to_path_buf(),
                permissions: parent_permissions.clone(),
                event_tx: event_tx.clone(),
                usage_tx: usage_tx.clone(),
                db: db.clone(),
                session_id,
                history_limit_percent,
                retry_config: retry_config.clone(),
                debug: debug.clone(),
                depth: depth + 1,
                subagent_depth,
                job_registry: job_registry.clone(),
                agent: None,
                llm_registry: llm_registry.clone(),
                skill_registry: skill_registry.clone(),
                skill_names: parent_skill_names.to_vec(),
            };
```

### Tarea 6: Usar `workspace` en `spawn_subagent_and_delegate` para renderizar system prompt

**Archivos:**
- Modificar: `src/agent/tools.rs:1552-1700`

- [ ] **Paso 6.1:** Añadir los imports necesarios al inicio del archivo si no existen:

```rust
use std::collections::HashMap;
use crate::llm::template::render_template;
```

- [ ] **Paso 6.2:** En `spawn_subagent_and_delegate`, destructure `workspace` del config y renderizar el system prompt:

```rust
pub(crate) async fn spawn_subagent_and_delegate(
    cfg: SpawnSubagentConfig,
) -> Result<SubagentOutcome> {
    let SpawnSubagentConfig {
        parent_id,
        parent_name,
        task_id,
        description,
        mode,
        model,
        tools,
        workspace,
        permissions,
        event_tx,
        usage_tx,
        db,
        session_id,
        history_limit_percent,
        retry_config,
        debug,
        depth,
        subagent_depth,
        job_registry,
        agent,
        llm_registry,
        skill_registry,
        skill_names,
    } = cfg;
```

- [ ] **Paso 6.3:** Renderizar el system prompt usando `render_template` donde antes se usaba `agent.description.clone()`. Buscar el lugar donde se asigna el system prompt (aproximadamente línea 1690) y reemplazar:

```rust
                    // Render the system prompt with workspace variable
                    let mut vars = HashMap::new();
                    vars.insert("workspace".to_string(), workspace.to_string_lossy().to_string());
                    let system_prompt = render_template(&agent.description, &vars);
```

Luego usar `system_prompt` en el mensaje System del subagente en lugar de `agent.description.clone()`.

Ejemplo del contexto donde se usa (aproximadamente líneas 1680-1700):

```rust
                    messages.push(SystemMessage {
                        content: system_prompt,  // antes era: agent.description.clone()
                        ..Default::default()
                    });
```

### Tarea 7: Actualizar el caller en `lifecycle.rs`

**Archivos:**
- Modificar: `src/agent/lifecycle.rs:814-835`

- [ ] **Paso 7.1:** Localizar la llamada a `execute_task_tool` (~línea 814). El workspace ya está disponible en el `SpawnAgentConfig` que posee la función `spawn_agent`. Añadirlo como último argumento:

```rust
                                            let task_result = execute_task_tool(
                                                &tc,
                                                agent_permissions,
                                                llm_registry,
                                                skill_registry,
                                                skill_names,
                                                event_tx,
                                                usage_tx,
                                                db,
                                                session_id,
                                                history_limit_percent,
                                                retry_config,
                                                debug_mode,
                                                depth,
                                                subagent_depth,
                                                agent_name,
                                                agent_id,
                                                model_name,
                                                job_registry,
                                                subagent_configs,
                                                &workspace,   // <-- nuevo parámetro
                                            )
                                            .await;
```

### Tarea 8: Actualizar tests existentes

**Archivos:**
- Modificar: `src/agent/tools.rs:2218-2260` (tests)

- [ ] **Paso 8.1:** En `test_task_tool_args_parse_with_agent` (~línea 2218), actualizar el JSON para incluir `"workspace":"/tmp/test"` y añadir assert:

```rust
    #[test]
    fn test_task_tool_args_parse_with_agent() {
        let json = r#"{
            "task_id": "t1",
            "description": "do something",
            "agent": "reviewer",
            "workspace": "/tmp/test"
        }"#;
        let args = TaskToolArgs::parse(json).unwrap();
        assert_eq!(args.task_id, "t1");
        assert_eq!(args.description, "do something");
        assert_eq!(args.agent, Some("reviewer".to_string()));
        assert_eq!(args.workspace, PathBuf::from("/tmp/test"));
    }
```

- [ ] **Paso 8.2:** En `test_task_tool_args_parse_without_agent` (~línea 2228), actualizar el JSON para incluir `"workspace":"/tmp/test"` y añadir assert:

```rust
    #[test]
    fn test_task_tool_args_parse_without_agent() {
        let json = r#"{
            "task_id": "t2",
            "description": "do something else",
            "workspace": "/tmp/test"
        }"#;
        let args = TaskToolArgs::parse(json).unwrap();
        assert_eq!(args.task_id, "t2");
        assert_eq!(args.description, "do something else");
        assert_eq!(args.agent, None);
        assert_eq!(args.workspace, PathBuf::from("/tmp/test"));
    }
```

- [ ] **Paso 8.3:** En `test_task_tool_args_parse_background_with_model_and_tools` (~línea 1175), actualizar el JSON:

```rust
    #[test]
    fn test_task_tool_args_parse_background_with_model_and_tools() {
        let json = r#"{
            "task_id": "t3",
            "description": "bg task",
            "mode": "background",
            "model": "gpt-4",
            "tools": ["shell", "read"],
            "workspace": "/tmp/test"
        }"#;
        let args = TaskToolArgs::parse(json).unwrap();
        assert_eq!(args.mode, TaskMode::Background);
        assert_eq!(args.model, Some("gpt-4".to_string()));
        assert_eq!(args.tools, vec!["shell", "read"]);
        assert_eq!(args.workspace, PathBuf::from("/tmp/test"));
    }
```

- [ ] **Paso 8.4:** En `test_execute_task_tool_agent_not_found` (~línea 2237), actualizar el JSON y la llamada:

```rust
    #[tokio::test]
    async fn test_execute_task_tool_agent_not_found() {
        let json = r#"{
            "task_id": "t4",
            "description": "do x",
            "agent": "nonexistent",
            "workspace": "/tmp/test"
        }"#;
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            function: FunctionCall {
                name: "task".to_string(),
                arguments: json.to_string(),
            },
        };
        let result = execute_task_tool(
            &tool_call,
            &permissions,
            &llm_registry,
            &skill_registry,
            &skill_names,
            &event_tx,
            &None,
            &None,
            None,
            0.5,
            &retry_config,
            &Arc::new(AtomicBool::new(false)),
            0,
            5,
            "parent",
            &AgentId("parent-id".to_string()),
            "claude-3",
            &None,
            &[],
            &Path::new("/tmp/test"),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }
```

- [ ] **Paso 8.5:** En `test_task_tool_args_parse_defaults` (~línea 1185), actualizar el JSON:

```rust
    #[test]
    fn test_task_tool_args_parse_defaults() {
        let json = r#"{
            "task_id": "t5",
            "description": "defaults test",
            "workspace": "/tmp/test"
        }"#;
        let args = TaskToolArgs::parse(json).unwrap();
        assert_eq!(args.mode, TaskMode::Foreground);
        assert_eq!(args.model, None);
        assert!(args.tools.is_empty());
        assert_eq!(args.workspace, PathBuf::from("/tmp/test"));
    }
```

### Tarea 9: Añadir test nuevo para `workspace` faltante

**Archivos:**
- Modificar: `src/agent/tools.rs` (añadir test nuevo cerca de los demás tests de parseo)

- [ ] **Paso 9.1:** Añadir test `test_task_tool_args_parse_missing_workspace`:

```rust
    #[test]
    fn test_task_tool_args_parse_missing_workspace() {
        let json = r#"{
            "task_id": "t6",
            "description": "missing workspace"
        }"#;
        let result = TaskToolArgs::parse(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("workspace"));
    }
```

## Resumen de cambios

| Archivo | Cambio |
|---|---|
| `src/agent/tools.rs` | Añadir `workspace: PathBuf` a `TaskToolArgs`, parseo, schema, `SpawnSubagentConfig`, parámetro de `execute_task_tool`, renderizado en `spawn_subagent_and_delegate`, tests |
| `src/agent/lifecycle.rs` | Pasar `&workspace` como argumento adicional a `execute_task_tool` |