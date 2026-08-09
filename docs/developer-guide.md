# Anacleto — Guía para Desarrolladores

**Versión:** 0.17.1  
**Repositorio:** https://github.com/atareao/anacleto  
**Licencia:** MIT  
**Rust Edition:** 2024 (rustc ≥ 1.85)

---

## Índice

1. [Arquitectura general](#1-arquitectura-general)
2. [Mapa de módulos](#2-mapa-de-módulos)
3. [Modelo de agente](#3-modelo-de-agente)
4. [Sistema de skills](#4-sistema-de-skills)
5. [Integración MCP](#5-integración-mcp)
6. [Sistema de permisos](#6-sistema-de-permisos)
7. [Motor de orquestación (Engine)](#7-motor-de-orquestación-engine)
8. [Proveedores LLM](#8-proveedores-llm)
9. [TUI](#9-tui)
10. [Sistema de hooks](#10-sistema-de-hooks)
11. [Plugins](#11-plugins)
12. [Persistencia (SQLite)](#12-persistencia-sqlite)
13. [ADRs (Architecture Decision Records)](#13-adrs)
14. [Flujo de trabajo típico](#14-flujo-de-trabajo-típico)
15. [Cómo contribuir](#15-cómo-contribuir)

---

## 1. Arquitectura general

Anacleto es un **motor de orquestación de agentes** construido en Rust. Gestiona un árbol de agentes y subagentes con separación limpia de skills, servidores MCP y permisos. La única interfaz es una TUI construida con ratatui + crossterm.

### Principios de diseño

| Principio | Descripción |
|---|---|
| **Misma interfaz** | TUI es la única interfaz. Sin web UI, sin batch mode. |
| **Agente = subagente** | El mismo tipo `Agent` con diferentes roles (`Root`/`SubAgent`). |
| **Sin herencia** | Los subagentes NO heredan skills, MCPs ni permisos del padre. |
| **Jerarquía plana** | Solo 2 niveles: agente → subagente. Subagentes no anidan. |
| **Subagentes desechables** | Crear → trabajar → responder → destruir. |
| **Streaming siempre activo** | Pasos intermedios visibles en TUI. |
| **Allow by default** | Permisos: todo lo no denegado explícitamente está permitido. |

### Diagrama de alto nivel

```
┌─────────────────────────────────────────────────────────────┐
│                        main.rs                              │
│  ┌──────────────┐              ┌──────────────────────────┐ │
│  │    Engine     │ ←─ mpsc ──→ │         TUI (App)        │ │
│  │ (orquestador) │   events/   │  ratatui + crossterm     │ │
│  │              │   commands  │                          │ │
│  │  ┌─────────┐ │              │  ┌────────────────────┐ │ │
│  │  │ Agentes │ │              │  │ Paneles: chat,     │ │ │
│  │  │ Skills  │ │              │  │ skills, mcps,      │ │ │
│  │  │ MCPs    │ │              │  │ subagentes, input  │ │ │
│  │  │ LLM     │ │              │  │                    │ │ │
│  │  └─────────┘ │              │  └────────────────────┘ │ │
│  └──────────────┘              └──────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Tecnologías clave

| Crate | Propósito |
|---|---|
| `tokio` | Runtime asíncrono |
| `ratatui` + `crossterm` | TUI |
| `serde` + `serde_yaml` | Serialización (config, frontmatter) |
| `sqlx` (SQLite) | Persistencia de sesiones |
| `reqwest` | Cliente HTTP para APIs LLM |
| `tower` | Middleware (retries, rate limiting) |
| `anyhow` + `thiserror` | Manejo de errores |
| `clap` | Parseo de CLI |
| `uuid` | Identificadores de agentes/sesiones |
| `tracing` | Logging estructurado |

---

## 2. Mapa de módulos

```
src/
├── main.rs              ← Entrypoint, CLI, init logging, bootstrap TUI
├── lib.rs               ← Declaración de módulos públicos
├── error.rs             ← Error enum global (Config, Agent, Skill, Mcp, Llm, Permission...)
│
├── config/              ← Carga y fusión de configuración YAML
│   ├── types.rs         ← Tipos: Config, ModelsConfig, McpDefinition, SessionConfig, etc.
│   ├── loader.rs        ← load_config(), merge_configs(), expand_env_vars()
│   └── paths.rs         ← project_root(), project_config_path(), global_skills_dir()
│
├── agent/               ← Modelo de agente/subagente
│   ├── types.rs         ← Agent, AgentId, AgentRole (Root|SubAgent), AgentStatus
│   ├── loader.rs        ← parse_agent() (Markdown + YAML frontmatter), load_agents()
│   ├── lifecycle.rs     ← spawn_agent(), AgentHandle, run_agent_loop()
│   ├── tools.rs         ← skill_to_tool_definition(), check_tool_permission()
│   ├── tool_store.rs    ← ToolOutputStore (caché FIFO de outputs completos)
│   ├── context.rs       ← Compaction (summarize_conversation), estimación de tokens
│   ├── source.rs        ← Source trait, FileSource, workspace instructions
│   └── retry.rs         ← retry_with_backoff() (jitter exponencial)
│
├── skill/               ← Sistema de skills
│   ├── types.rs         ← Skill, SkillResult, SkillExecutor trait
│   ├── loader.rs        ← parse_skill(), load_skills_from_dir()
│   ├── executor.rs      ← DefaultSkillExecutor (shell, web, filesystem, genérico)
│   ├── registry.rs      ← SkillRegistry (carga, lookup, hot-reload)
│   └── discovery.rs     ← discover_skills() (escanea directorios)
│
├── mcp/                 ← Cliente MCP (JSON-RPC 2.0)
│   ├── types.rs         ← McpTransport (Stdio|Tcp), McpTool, McpResource
│   ├── client.rs        ← McpClient (connect, initialize, list_tools, call_tool)
│   ├── registry.rs      ← McpRegistry (registro, conexión, colecta de herramientas)
│   └── parse.rs         ← Parseo de respuestas MCP
│
├── permissions/         ← Sistema de permisos
│   ├── types.rs         ← Permission enum, Permissions struct
│   └── checker.rs       ← check_permission(), check_fs_read/write, etc.
│
├── engine/              ← Motor de orquestación
│   ├── orchestrator.rs  ← Engine (run loop, procesa comandos, gestiona agentes)
│   ├── events.rs        ← EngineEvent, EngineCommand (eventos TUI ↔ engine)
│   ├── commands.rs      ← Handlers: /skills, /mcps, /status, /init, /review...
│   ├── sessions.rs      ← Handlers: /new, /resume, /undo, /redo, /fork, /export...
│   ├── jobs.rs          ← JobRegistry (tareas background)
│   ├── template.rs      ← Expansión de {env:VAR} y {file:path}
│   └── apply_patch.rs   ← Operación batch para cambios múltiples
│
├── llm/                 ← Proveedores LLM
│   ├── provider.rs      ← LlmProvider trait, create_provider(), LlmProviderRegistry
│   ├── types.rs         ← LlmMessage, LlmRequest, LlmResponse, ToolCall, ToolDefinition
│   ├── anthropic.rs     ← Implementación Anthropic API
│   ├── openai.rs        ← Implementación OpenAI / OpenRouter
│   ├── ollama.rs        ← Implementación Ollama
│   ├── azure.rs         ← Implementación Azure OpenAI
│   ├── bedrock.rs       ← Implementación AWS Bedrock
│   ├── google.rs        ← Implementación Google Gemini
│   ├── template.rs      ← Renderizado de templates de system prompt
│   └── models.rs        ← Respuestas de API para info de modelos
│
├── tui/                 ← Interfaz de usuario TUI
│   ├── app.rs           ← App struct, run_tui(), main event loop
│   ├── types.rs         ← Focus, AgentInfo, BUILTIN_COMMANDS, ApprovalRequest
│   ├── render.rs        ← render(), render_status_bar, render_main_content, etc.
│   ├── state.rs         ← State helpers, fuzzy_score()
│   ├── keymap.rs        ← Keymap (atajos configurables)
│   ├── keys.rs / keyparse.rs / input.rs / events.rs / markdown.rs
│   ├── diff_viewer.rs / model_picker.rs / navigation.rs / palette.rs
│   ├── theme.rs / which_key.rs / toast.rs
│
├── tools/               ← Herramientas estructuradas built-in
│   ├── read.rs / grep.rs / glob.rs / web.rs / lsp.rs / mcp.rs / pattern.rs
│
├── shell/               ← Detección de shell e inventario de herramientas modernas
│   └── mod.rs           ← ShellInfo, ToolInfo, default_tools(), init()
│
├── filesystem/          ← Operaciones atómicas del skill filesystem
│   └── mod.rs           ← FsOp, FsRequest, execute_operation()
│
├── hook/                ← Sistema de hooks
│   ├── mod.rs           ← HookPoint, HookAction, HookRegistry, HookContext
│   └── autoconfig.rs    ← Auto-detección de hooks para herramientas conocidas
│
├── plugin/              ← Sistema de plugins
│   └── mod.rs           ← Plugin trait, PluginRegistry, PluginManifest
│
├── db/                  ← Persistencia SQLite (sqlx)
│   ├── session.rs       ← Database (open, migrations, CRUD sesiones/mensajes)
│   ├── models.rs        ← Session, StoredMessage, Snapshot, SessionSummary
│   ├── messages.rs / snapshots.rs / todos.rs / export.rs / usage.rs
│
└── lsp/                 ← Cliente LSP
    ├── mod.rs           ← LspClient, query()
    └── format.rs        ← Formateo de respuestas LSP
```

---

## 3. Modelo de agente

### Definición del tipo `Agent` (`agent/types.rs`)

```rust
pub struct Agent {
    pub id: AgentId,              // UUID v4 único
    pub name: String,              // Nombre del agente
    pub role: AgentRole,           // Root | SubAgent
    pub description: String,       // System prompt (cuerpo del Markdown)
    pub model: String,             // Modelo LLM (ej: "deepseek/deepseek-v4-flash")
    pub skills: Vec<PathBuf>,      // Paths a skills
    pub mcps: Vec<String>,         // Referencias a MCPs globales
    pub permissions: Permissions,  // Permisos del agente
    pub subagent_names: Vec<String>, // Solo Root — lista de subagentes
    pub parent_id: Option<AgentId>,  // Solo SubAgent — ID del padre
    pub max_steps: u32,           // Pasos máximos por tarea (default: 100)
    pub subagent_depth: u32,      // Profundidad máxima de delegación dinámica
}
```

### Role: Root vs SubAgent

| Característica | Root | SubAgent |
|---|---|---|
| Invocable por usuario | ✅ | ❌ |
| Puede tener subagentes | ✅ | ❌ |
| Tiene `parent_id` | ❌ (None) | ✅ |
| Ciclo de vida | Persistente | Desechable (crear → trabajar → destruir) |

### Definición de agente (archivo Markdown)

Los agentes se definen como archivos Markdown con frontmatter YAML:

```markdown
---
name: root
role: root
description: Senior engineering agent
model: deepseek/deepseek-v4-flash
max_steps: 90
skills:
  - .agents/skills/shell/
  - .agents/skills/filesystem/
mcps: [codegraph]
permissions:
  deny:
    - command.run.sudo
    - net.http.delete
subagents:
  - reviewer
  - writer
  - rust-dev
---

You are **Anacleto**, a senior engineering agent...
```

El **frontmatter YAML** contiene la configuración estructural. El **cuerpo Markdown** es el system prompt.

### Ubicaciones de los archivos de agente

| Ámbito | Ruta |
|---|---|
| Global | `~/.config/anacleto/agents/<name>.md` |
| Proyecto | `.agents/agents/<name>.md` (sobrescribe global) |

### Carga de agentes (`agent/loader.rs`)

`parse_agent()` parsea el Markdown:
1. Busca delimitadores `---`
2. Extrae el frontmatter YAML
3. El resto es el system prompt
4. Retorna `AgentConfig`

`load_agents()`:
1. Escanea `~/.config/anacleto/agents/` (global)
2. Escanea `.agents/agents/` (proyecto)
3. Fusiona por nombre: proyecto sobrescribe global

### Ciclo de vida de un agente (`agent/lifecycle.rs`)

La función principal es `run_agent_loop()`. Secuencia:

```
1. Recibir input (usuario o engine)
2. Construir system prompt:
   - description (del Markdown)
   - workspace instructions (AGENTS.md, CLAUDE.md, CONTEXT.md)
   - instrucciones de source si existen
3. Cargar skills del agente en SkillRegistry
4. Cargar MCPs y colectar herramientas
5. Convertir skills a ToolDefinitions para el LLM
6. Añadir built-in tools:
   - read, grep, glob, webfetch, websearch
   - lsp_query, MCP resource tools
   - todo, question, task (spawn subagent)
   - apply_patch
7. Bucle LLM + tools:
   a. Enviar request al LLM (con streaming)
   b. Recibir respuesta (texto + tool_calls)
   c. Para cada tool_call:
      - Verificar permisos
      - Solicitar aprobación humana si es sensible
      - Ejecutar tool
      - Devolver resultado al LLM
   d. Repetir hasta finish_reason = "stop" o max_steps
8. Emitir eventos EngineEvent para la TUI
9. Al completar o fallar, notificar al engine
```

### Tool Store (`agent/tool_store.rs`)

Almacena los outputs COMPLETOS de las tools (el LLM solo recibe una versión truncada a 4000 caracteres).

- FIFO con capacidad máxima (default: 100 entradas)
- Se usa durante la compactación para generar resúmenes con información completa
- Permite consultar outputs previos sin re-ejecutar

### Compaction (`agent/context.rs`)

Cuando la conversación excede el 80% del context window, se dispara compactación automática:

1. El LLM genera un resumen estructurado con:
   - Objetivo
   - Decisiones tomadas
   - Hechos y contexto clave
   - Código/patrones relevantes
   - Tareas pendientes
   - Riesgos o bloqueos
2. El resumen se inyecta como mensaje System
3. El historial antiguo se descarta

También hay comando manual `/compact`.

---

## 4. Sistema de skills

### Formato

Las skills son archivos Markdown con frontmatter YAML, siguiendo el formato Anthropic:

```markdown
---
name: code-review
description: Code review specialist — reviews code for quality, correctness, and adherence
to project standards
metadata:
  category: development
hooks:
  after_apply:
    - type: shell
      command: "echo review done"
      timeout_secs: 30
---

When reviewing code, evaluate against these dimensions:
1. Correctness
2. Safety & robustness
3. ...
```

### Ubicaciones

| Ámbito | Ruta |
|---|---|
| Proyecto | `.agents/skills/<nombre>/SKILL.md` |
| Global | `$HOME/.agents/skills/<nombre>/SKILL.md` |
| Absoluta | Cualquier path absoluto |

### Registro (`skill/registry.rs`)

`SkillRegistry`:

```rust
pub struct SkillRegistry {
    skills: HashMap<String, Skill>,     // key: nombre en lowercase
    sources: HashMap<String, PathBuf>,   // source path para hot-reload
}
```

- `load_from_paths(paths)` — carga skills desde paths (archivos o directorios)
- `get(name)` — lookup case-insensitive
- `insert(skill)` — insertar dinámicamente
- `reload()` — recargar desde los paths originales (hot-reload)
- `list()` — listar todas las skills cargadas

### Ejecución (`skill/executor.rs`)

`DefaultSkillExecutor` despacha según el nombre de la skill:

| Nombre de skill | Acción |
|---|---|
| `shell` | Ejecuta comando shell (`sh -c "..."`) |
| Contiene "web" o "research" | `webfetch()` |
| `filesystem` | Operaciones atómicas (read/write/edit/list/delete) |
| Otros | Devuelve las instrucciones al LLM como contexto |

### Skill trait

```rust
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    async fn execute(&self, skill: &Skill, input: &str) -> SkillResult;
}
```

### Descubrimiento (`skill/discovery.rs`)

`discover_skills()` escanea `.agents/skills/` y `$HOME/.agents/skills/` buscando subdirectorios que contengan un archivo `SKILL.md`. Retorna `Vec<DiscoveredSkill>` con nombre y directorio.

### Skills incorporados en el proyecto

shell, filesystem, web-research, code-review, rust-dev, find-skills, skill-creator, agent-creator, planning, version-control, tool-discovery, weather, python-best-practices

---

## 5. Integración MCP

### Protocolo

MCP (Model Context Protocol) es un protocolo JSON-RPC 2.0 para integrar herramientas y servicios externos con agentes LLM.

### Transportes (`mcp/types.rs`)

```rust
pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },  // proceso hijo
    Tcp { host: String, port: u16 },                // socket TCP
}
```

### Definición en config.yaml

```yaml
mcps:
  filesystem:
    transport: stdio
    command: "/usr/local/bin/mcp-filesystem"
    args: ["--allowed-dirs", "/home/user/projects"]

  postgres:
    transport: tcp
    host: "localhost"
    port: 5432
```

### Cliente (`mcp/client.rs`)

`McpClient`:

1. `connect()` — spawn proceso o conecta TCP
2. `perform_initialize()` — handshake JSON-RPC 2.0
3. `list_tools()` — descubre herramientas del servidor
4. `call_tool(name, args)` — ejecuta herramienta
5. `list_resources()` / `read_resource(uri)` / `list_resource_templates()`

### Registro (`mcp/registry.rs`)

`McpRegistry` mantiene los clientes conectados. Método clave:

```rust
// Convierte tools MCP a ToolDefinitions para el LLM
// Los nombres se prefijan con el servidor: "codegraph_analyze"
pub async fn collect_tools(&self, server_names: &[String])
    -> Vec<(String, String, ToolDefinition)>;
```

### Integración con agentes

- Los MCPs se asignan por nombre en `agent.mcps: ["filesystem"]`
- Cada agente tiene su propia lista (sin herencia)
- Se pueden activar/desactivar en caliente via `/mcps <name> on|off`
- Las tools MCP se presentan al LLM con prefijo del servidor

---

## 6. Sistema de permisos

### Tipos de permiso (`permissions/types.rs`)

```rust
pub enum Permission {
    FsRead,       // "fs.read"
    FsWrite,      // "fs.write"
    FsExternal,   // "fs.external" — fuera del workspace
    NetHttp,      // "net.http"
    CommandRun,   // "command.run"
    McpUse,       // "mcp.use"
    EnvRead,      // "env.read"
    SkillUse,     // "skill.use"
}
```

### Modelo

| Regla | Descripción |
|---|---|
| **Allow by default** | Todo lo no denegado explícitamente está permitido |
| **Deny explícito** | `permissions.deny: ["command.run"]` |
| **Allow explícito** | Si hay `allow`, se usa deny-by-default para esos permisos |
| **FsExternal es opt-in** | Nunca se concede por defecto, debe estar en `allow` explícito |
| **Aprobación humana** | Operaciones sensibles requieren confirmación en TUI |

### Operaciones sensibles (`agent/tools.rs`)

Requieren aprobación humana:
- `sudo` en comandos
- `rm -rf`
- `chmod`
- Operaciones en `/boot/`
- Escritura de archivos fuera del workspace

### Configuración típica

```yaml
permissions:
  allow:
    - fs.external     # permitir acceso fuera del workspace
  deny:
    - command.run     # denegar ejecución de comandos
    - net.http        # denegar peticiones HTTP
```

### Checker (`permissions/checker.rs`)

Funciones helper:

```rust
pub fn check_permission(permissions: &Permissions, action: &Permission) -> Result<()>;
pub fn check_fs_read(permissions: &Permissions) -> Result<()>;
pub fn check_fs_write(permissions: &Permissions) -> Result<()>;
pub fn check_command_run(permissions: &Permissions) -> Result<()>;
pub fn check_net_http(permissions: &Permissions) -> Result<()>;
pub fn check_mcp_use(permissions: &Permissions) -> Result<()>;
pub fn check_skill_use(permissions: &Permissions) -> Result<()>;
pub fn check_env_read(permissions: &Permissions) -> Result<()>;
```

---

## 7. Motor de orquestación (Engine)

### Estructura (`engine/orchestrator.rs`)

```rust
pub struct Engine {
    config: Config,
    agents: HashMap<String, AgentId>,
    handles: HashMap<AgentId, AgentHandle>,
    llm_registry: LlmProviderRegistry,
    mcp_registry: Arc<Mutex<McpRegistry>>,
    skill_registry: SharedSkillRegistry,
    database: Option<Database>,
    active_session_id: Option<Uuid>,
    event_tx: mpsc::Sender<EngineEvent>,
    command_rx: mpsc::Receiver<EngineCommand>,
    usage_tx: mpsc::Sender<UsageEvent>,
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    pending_questions: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    debug: Arc<AtomicBool>,
    current_model: String,
    active_agent: String,
    undo_stack: Vec<Vec<StoredMessage>>,
    redo_stack: Vec<Vec<StoredMessage>>,
    mcp_enabled: Arc<Mutex<HashMap<String, bool>>>,
    // ...
}
```

### Bucle principal (`Engine::run()`)

1. Inicializa providers LLM, skills, MCPs
2. Dispara hooks `OnStartup`
3. Crea sesión inicial o reanuda
4. Bucle de eventos:
   - Recibe `EngineCommand` del canal
   - Procesa según el tipo:
     - `UserInput` → envía al agente activo
     - `SlashCommand` → dispatch a handler
     - `ApprovalResponse` → resuelve pending approval
     - `QuestionResponse` → resuelve pending question
     - `SwitchAgent` → cambia agente activo
     - `Shutdown` → termina
   - Emite `EngineEvent` para la TUI

### EngineEvent (`engine/events.rs`)

Cubre TODO el ciclo de vida. Eventos principales:

| Evento | Disparo |
|---|---|
| `Started` | Engine arrancado |
| `AgentCreated` | Agente creado |
| `AgentMessage` | Mensaje recibido por agente |
| `AgentStreamChunk` | Chunk streaming LLM |
| `AgentThinkingChunk` | Chunk de razonamiento |
| `AgentStatusChanged` | Cambio de estado |
| `SubagentCreated` | Subagente creado |
| `SubagentCompleted` | Subagente completado |
| `ApprovalRequired` | Operación sensible pendiente |
| `TokenUsage` | Uso de tokens reportado |
| `SkillsListed` / `McpsListed` | Listados solicitados |
| `SessionSwitched` / `SessionDeleted` | Gestión de sesiones |

---

## 8. Proveedores LLM

### Provider trait (`llm/provider.rs`)

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>;
    async fn complete_stream(&self, request: LlmRequest)
        -> Result<mpsc::Receiver<Result<LlmStreamChunk>>>;
    fn context_window(&self) -> usize;
    async fn fetch_context_window(&self) -> Result<usize>;
    fn set_context_window(&self, value: usize);
    fn input_price_per_million(&self) -> f64;
    fn output_price_per_million(&self) -> f64;
}
```

### Resolución de proveedor por modelo

| Patrón de modelo | Provider |
|---|---|
| `claude*` | Anthropic |
| `gpt*`, `o1*`, `o3*` | OpenAI |
| Contiene `/` | OpenRouter (OpenAI-compatible) |
| Cualquier otro | Ollama |
| Config explícita | Azure, Bedrock, Google |

### Fábrica de providers

```rust
pub fn create_provider(config: &LlmProviderConfig) -> Box<dyn LlmProvider> {
    match config.provider_type {
        LlmProviderType::Anthropic => Box::new(AnthropicProvider::new(config)),
        LlmProviderType::OpenAI => Box::new(OpenAIProvider::new(config)),
        LlmProviderType::OpenRouter => Box::new(OpenRouterProvider::new(config)),
        LlmProviderType::Ollama => Box::new(OllamaProvider::new(config)),
        LlmProviderType::Bedrock => Box::new(BedrockProvider::new(config)),
        LlmProviderType::Azure => Box::new(AzureProvider::new(config)),
        LlmProviderType::Google => Box::new(GoogleProvider::new(config)),
    }
}
```

### LlmProviderRegistry

Registry de providers por nombre de modelo, compartido via `Arc`. Los agentes obtienen el provider según el modelo que tienen configurado.

---

## 9. TUI

### Arquitectura

- ratatui + crossterm en el mismo proceso que el engine
- Tareas Tokio separadas: engine task + TUI task
- Comunicación via canales `mpsc`:
  - `event_tx → event_rx`: Engine → TUI (EngineEvent)
  - `cmd_tx → cmd_rx`: TUI → Engine (EngineCommand)

### Layout de pantalla

```
┌─────────────────────────────────────────────┐
│ Status Bar (agente activo | sesión | debug) │
├─────────────────────────────────────────────┤
│                                             │
│  Chat Panel (mensajes, streaming)           │
│                                             │
│  Sidebar panels (skills, mcps,              │
│  subagentes, agentes)                        │
│                                             │
├─────────────────────────────────────────────┤
│ > Input line                                │
├─────────────────────────────────────────────┤
│ Working directory                           │
└─────────────────────────────────────────────┘
```

### Paneles y diálogos

| Componente | Descripción |
|---|---|
| Status Bar | Agente activo, sesión, modo debug |
| Chat Panel | Mensajes de la conversación |
| Input Line | Entrada de texto del usuario |
| Skills Panel | Skills del agente activo |
| MCPs Panel | Servidores MCP (on/off) |
| SubAgents Panel | Subagentes activos |
| Agents Panel | Agentes raíz disponibles |
| Approval Dialog | Confirmación de operaciones sensibles |
| Question Dialog | Preguntas inline del LLM |
| Which-key Popup | Atajos de teclado |
| Command Palette | Búsqueda difusa de comandos |
| Agent Palette | Cambio de agente activo |
| Model Palette | Cambio de modelo |
| Edit Dialog | Editar agente/subagente |
| Diff Viewer | Visualización de cambios |
| Model Picker | Selector de modelos |
| Search Overlay | Búsqueda en la conversación |
| Toast Queue | Notificaciones temporales |

### Temas

Sistema de temas de color configurables via `/themes`. Ver `tui/theme.rs`.

### Keymap

Atajos de teclado configurables en `config.yaml`:

```yaml
keymap:
  keys:
    focus_input: "escape i"
    focus_chat: "escape c"
    toggle_sidebar: "escape s"
    command_palette: "ctrl+p"
    submit: "enter"
    search: "ctrl+f"
```

---

## 10. Sistema de hooks

### Hook Points (`hook/mod.rs`)

| Hook Point | Cuándo se dispara |
|---|---|
| `BeforeTool` | Antes de ejecutar cualquier tool |
| `AfterTool` | Después de cualquier tool (éxito) |
| `BeforeApply` | Antes de `apply_patch` |
| `AfterApply` | Después de `apply_patch` (éxito) |
| `BeforeShell` | Antes de comando shell |
| `AfterShell` | Después de comando shell (éxito) |
| `BeforeFsWrite` | Antes de write/edit/delete |
| `AfterFsWrite` | Después de write/edit/delete (éxito) |
| `OnStartup` | Arranque del engine |
| `OnShutdown` | Parada del engine |

### Acciones

```rust
pub enum HookAction {
    Shell { command: String },
}
```

### Configuración

```yaml
hooks:
  after_apply:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
  on_startup:
    - type: shell
      command: "echo 'engine started'"
```

### Auto-configuración (`hook/autoconfig.rs`)

Detecta herramientas conocidas en PATH (como `codegraph`) y registra hooks automáticamente.

---

## 11. Plugins

### Plugin trait (`plugin/mod.rs`)

```rust
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn register_hooks(&self) -> Vec<(HookPoint, HookActionConfig)> { vec![] }
    fn on_agent_spawn(&self, _name: &str, _prompt: &str) -> HookResult<String> { None }
    fn on_tool_call(&self, _call: &ToolCall) -> HookResult<String> { None }
    fn on_command(&self, _cmd: &str, _args: &str) -> HookResult<String> { None }
    fn on_event(&self, _event: &str) {}
    fn register_tool(&self) -> Vec<ToolDefinition> { vec![] }
}
```

### PluginRegistry

```rust
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
    custom_tools: Vec<ToolDefinition>,
    custom_tool_handlers: HashMap<String, ToolHandler>,
}
```

Los plugins se cargan desde `~/.config/anacleto/plugins/`.

---

## 12. Persistencia (SQLite)

### Esquema de base de datos

```sql
-- Sesiones
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    metadata TEXT,
    shared INTEGER NOT NULL DEFAULT 0,
    workspace TEXT,
    pinned INTEGER NOT NULL DEFAULT 0,
    parent_id TEXT
);

-- Mensajes
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_name TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    token_count INTEGER,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- TODOs
CREATE TABLE todos (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Snapshots
CREATE TABLE snapshots (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    serialized_state TEXT NOT NULL
);

-- Token usage
CREATE TABLE usage (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    model TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    timestamp TEXT NOT NULL
);
```

### Database API

```rust
pub struct Database { pool: SqlitePool }

impl Database {
    pub async fn open(path: &Path) -> Result<Self>;
    pub async fn create_session(&self, name: &str) -> Result<Session>;
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>>;
    pub async fn get_session_messages(&self, id: Uuid) -> Result<Vec<StoredMessage>>;
    pub async fn save_message(&self, msg: &StoredMessage) -> Result<()>;
    pub async fn delete_session(&self, id: Uuid) -> Result<()>;
    pub async fn rename_session(&self, id: Uuid, name: &str) -> Result<()>;
    pub async fn fork_session(&self, id: Uuid) -> Result<Session>;
    pub async fn export_session(&self, id: Uuid, format: ExportFormat) -> Result<String>;
    pub async fn import_session(&self, data: &str) -> Result<Session>;
    // ... snapshots, todos, usage
}
```

---

## 13. ADRs

| ADR | Título | Decisión clave |
|---|---|---|
| 0001 | Agent Model | Agentes = subagentes, jerarquía 2 niveles, sin herencia |
| 0002 | Skill System | Markdown + YAML frontmatter, lazy loading |
| 0003 | MCP Integration | JSON-RPC 2.0, stdio/TCP, tools con prefijo de server |
| 0004 | TUI Architecture | ratatui + crossterm, mismo proceso, canales mpsc |
| 0005 | Configuration System | YAML 2 capas, agentes en Markdown |
| 0006 | Persistence | SQLite via sqlx, sesiones reanudables |
| 0007 | Permissions Model | Allow by default, deny explícito, aprobación humana |
| 0008 | Technology Stack | Rust, Tokio, ratatui, sqlx, reqwest |

---

## 14. Flujo de trabajo típico

### Para desarrollar una nueva funcionalidad

1. Leer `AGENTS.md` y ADRs relevantes
2. Entender la arquitectura del módulo afectado
3. Implementar siguiendo las convenciones:
   - Rust edition 2024
   - `cargo fmt` antes de commit
   - `cargo clippy` sin warnings
   - Tests unitarios en `#[cfg(test)]`
4. Probar:
   ```sh
   cargo build
   cargo test
   cargo clippy
   cargo fmt --check
   ```
5. Commit con Conventional Commits

### Para crear un nuevo agente

1. Crear `.agents/agents/<name>.md` con frontmatter YAML
2. Asignar skills, MCPs y permisos
3. Si tiene subagentes, crearlos también
4. Configurar en `.agents/config.yaml` si es necesario
5. `/reload` en la TUI para recargar

### Para crear una nueva skill

1. Crear `.agents/skills/<name>/SKILL.md` con frontmatter YAML
2. El cuerpo Markdown son las instrucciones
3. Opcional: `scripts/`, `references/`, `assets/`
4. Asignar la skill al agente correspondiente
5. `/reload` para recargar

---

## 15. Cómo contribuir

1. Haz fork del repositorio
2. Crea una rama con nombre descriptivo: `feat/nueva-funcionalidad`
3. Implementa siguiendo las guías de `AGENTS.md`
4. Asegura que pasa:
   ```sh
   cargo fmt --check && cargo clippy && cargo test
   ```
5. Crea un Pull Request con descripción clara
6. Usa [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat: add X`
   - `fix: correct Y`
   - `docs: update Z`
   - `refactor: restructure W`

---

*Documentación generada a partir del estudio exhaustivo del código fuente. Última actualización: agosto 2026.*
