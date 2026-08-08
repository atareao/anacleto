# Hook System — Implementation Plan

## Objetivo

Add a configurable shell-command hook system to Anacleto that fires before/after tool execution, apply_patch batches, shell commands, filesystem writes, and engine lifecycle events.

## Arquitectura

```
src/hook/mod.rs          ← new module: HookPoint, HookAction, HookRegistry, HookConfig
src/config/types.rs      ← add hooks: HashMap<String, Vec<HookActionConfig>> to Config
src/engine/orchestrator.rs  ← create HookRegistry at startup, call OnStartup/OnShutdown
src/agent/lifecycle.rs   ← pass HookRegistry via SpawnAgentConfig, wrap tool execution
src/agent/tools.rs       ← add hooks in execute_shell_command, execute_apply_patch_tool
src/filesystem/mod.rs    ← add hooks in execute() for write ops
src/lib.rs               ← register pub mod hook
```

**Data flow:**

1. YAML config declares `hooks.on_tool_call: [{ command: "codegraph sync" }]`
2. `Engine::initialize()` parses config into `HookRegistry` (one per engine)
3. `HookRegistry` is cloned into each `SpawnAgentConfig` → passed to agent tasks
4. At each hook point, the agent calls `registry.run(HookPoint::BeforeTool, ctx)` which spawns the configured shell command
5. The shell command's stdout/stderr are logged via `EngineEvent` but do not block execution (fire-and-forget with optional timeout)

## Tareas

### Tarea 1: Create `src/hook/mod.rs` — core types and registry

**Archivos:**
- Crear: `src/hook/mod.rs`

- [ ] **Paso 1:** Define `HookPoint` enum with all hook points
      ```rust
      #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
      #[serde(rename_all = "snake_case")]
      pub enum HookPoint {
          BeforeTool,
          AfterTool,
          BeforeApply,
          AfterApply,
          BeforeShell,
          AfterShell,
          BeforeFsWrite,
          AfterFsWrite,
          OnStartup,
          OnShutdown,
      }
      ```

- [ ] **Paso 2:** Define `HookAction` enum (currently only Shell, extensible to Event)
      ```rust
      #[derive(Debug, Clone, Serialize, Deserialize)]
      #[serde(tag = "type", rename_all = "snake_case")]
      pub enum HookAction {
          Shell { command: String },
      }
      ```

- [ ] **Paso 3:** Define `HookActionConfig` for YAML deserialization
      ```rust
      #[derive(Debug, Clone, Serialize, Deserialize)]
      pub struct HookActionConfig {
          #[serde(flatten)]
          pub action: HookAction,
          /// Optional timeout in seconds (default: 30).
          #[serde(default = "default_hook_timeout")]
          pub timeout_secs: u64,
      }
      fn default_hook_timeout() -> u64 { 30 }
      ```

- [ ] **Paso 4:** Define `HookRegistry` struct
      ```rust
      #[derive(Default, Clone)]
      pub struct HookRegistry {
          hooks: Arc<HashMap<HookPoint, Vec<HookActionConfig>>>,
      }
      impl HookRegistry {
          pub fn new(config: HashMap<HookPoint, Vec<HookActionConfig>>) -> Self;
          /// Run all hooks for a given point. Returns Vec of (command, stdout_truncated) results.
          pub async fn run(&self, point: HookPoint, ctx: &HookContext) -> Vec<HookResult>;
      }
      ```

- [ ] **Paso 5:** Define `HookContext` — carries per-hook metadata (tool name, file path, command, etc.)
      ```rust
      #[derive(Debug, Default)]
      pub struct HookContext {
          pub tool_name: Option<String>,
          pub file_path: Option<String>,
          pub shell_command: Option<String>,
          pub agent_name: Option<String>,
      }
      ```

- [ ] **Paso 6:** Define `HookResult` — captures stdout/stderr/exit code
      ```rust
      #[derive(Debug)]
      pub struct HookResult {
          pub command: String,
          pub stdout: String,
          pub stderr: String,
          pub exit_code: Option<i32>,
      }
      ```

- [ ] **Paso 7:** Implement `HookRegistry::run()` — spawn shell command via `tokio::process::Command`, apply timeout, capture output, log via `tracing::info!`

- [ ] **Paso 8:** Implement `From<&Config>` for `HookRegistry` — parse the `hooks` HashMap from config, mapping string keys to `HookPoint` variants

- [ ] **Paso 9:** Add `pub mod hook;` to `src/lib.rs`

### Tarea 2: Add hooks field to Config

**Archivos:**
- Modificar: `src/config/types.rs`

- [ ] **Paso 1:** Add `hooks` field to `Config` struct
      ```rust
      /// Hook system configuration: hook_point_name -> list of actions.
      #[serde(default)]
      pub hooks: HashMap<String, Vec<HookActionConfig>>,
      ```
      (Import `HookActionConfig` from `crate::hook` — or define a local alias to avoid circular deps; prefer a local `HookActionConfig` re-export in config types.)

- [ ] **Paso 2:** Add `use crate::hook::HookActionConfig;` import (or define a parallel `HookActionConfig` in config types that maps 1:1)

### Tarea 3: Integrate HookRegistry into Engine lifecycle

**Archivos:**
- Modificar: `src/engine/orchestrator.rs`

- [ ] **Paso 1:** Add `pub(crate) hook_registry: HookRegistry` field to `Engine` struct

- [ ] **Paso 2:** Initialize in `Engine::new()`:
      ```rust
      hook_registry: HookRegistry::from(&config),
      ```

- [ ] **Paso 3:** Call `OnStartup` hooks at the end of `Engine::initialize()`:
      ```rust
      self.hook_registry.run(HookPoint::OnStartup, &HookContext::default()).await;
      ```

- [ ] **Paso 4:** Call `OnShutdown` hooks at the beginning of `Engine::shutdown()`:
      ```rust
      self.hook_registry.run(HookPoint::OnShutdown, &HookContext::default()).await;
      ```

- [ ] **Paso 5:** Pass `hook_registry` to `SpawnAgentConfig`:
      ```rust
      hook_registry: self.hook_registry.clone(),
      ```

### Tarea 4: Pass HookRegistry through SpawnAgentConfig

**Archivos:**
- Modificar: `src/agent/lifecycle.rs`

- [ ] **Paso 1:** Add `pub hook_registry: HookRegistry` field to `SpawnAgentConfig`

- [ ] **Paso 2:** Destructure the new field in `spawn_agent()` and clone it into the async task

- [ ] **Paso 3:** Before executing each tool call in the `execute_one` closure, call:
      ```rust
      let ctx = HookContext { tool_name: Some(tc.function.name.clone()), ..Default::default() };
      hook_registry.run(HookPoint::BeforeTool, &ctx).await;
      ```
      After tool execution:
      ```rust
      hook_registry.run(HookPoint::AfterTool, &ctx).await;
      ```

- [ ] **Paso 4:** Wrap the `apply_patch` branch with `BeforeApply`/`AfterApply` hooks

### Tarea 5: Add hooks in agent/tools.rs for shell and apply_patch

**Archivos:**
- Modificar: `src/agent/tools.rs`

- [ ] **Paso 1:** Add `hook_registry: &HookRegistry` parameter to `execute_shell_command()`

- [ ] **Paso 2:** Wrap the shell execution with `BeforeShell`/`AfterShell` hooks:
      ```rust
      let ctx = HookContext { shell_command: Some(command.clone()), ..Default::default() };
      hook_registry.run(HookPoint::BeforeShell, &ctx).await;
      // ... existing shell execution ...
      hook_registry.run(HookPoint::AfterShell, &ctx).await;
      ```

- [ ] **Paso 3:** Add `hook_registry: &HookRegistry` parameter to `execute_apply_patch_tool()`

- [ ] **Paso 4:** Wrap `apply_patch_batch()` call with `BeforeApply`/`AfterApply` hooks

- [ ] **Paso 5:** Thread `hook_registry` through `execute_skill_tool()` → `execute_shell_command()` and `execute_apply_patch_tool()` calls

### Tarea 6: Add hooks in filesystem/mod.rs for write operations

**Archivos:**
- Modificar: `src/filesystem/mod.rs`

- [ ] **Paso 1:** Add `hook_registry: &HookRegistry` parameter to `execute()` function

- [ ] **Paso 2:** Before write ops (Write, Edit, Delete), call `BeforeFsWrite` hook:
      ```rust
      if is_write_op(&req.op) {
          let ctx = HookContext { file_path: Some(req.path.to_string_lossy().to_string()), ..Default::default() };
          hook_registry.run(HookPoint::BeforeFsWrite, &ctx).await;
      }
      ```

- [ ] **Paso 3:** After write ops, call `AfterFsWrite` hook

- [ ] **Paso 4:** Thread `hook_registry` through `execute_filesystem_operation()` in `tools.rs`

## Integration Points

| Hook Point | Where it fires | Context fields |
|---|---|---|
| `BeforeTool` | `agent/lifecycle.rs` — before `execute_one` | `tool_name`, `agent_name` |
| `AfterTool` | `agent/lifecycle.rs` — after `execute_one` | `tool_name`, `agent_name` |
| `BeforeApply` | `agent/tools.rs` — before `apply_patch_batch` | `tool_name: "apply_patch"`, `agent_name` |
| `AfterApply` | `agent/tools.rs` — after `apply_patch_batch` | `tool_name: "apply_patch"`, `agent_name` |
| `BeforeShell` | `agent/tools.rs` — before `tokio::process::Command` | `shell_command`, `agent_name` |
| `AfterShell` | `agent/tools.rs` — after shell completes | `shell_command`, `agent_name` |
| `BeforeFsWrite` | `filesystem/mod.rs` — before write/edit/delete | `file_path` |
| `AfterFsWrite` | `filesystem/mod.rs` — after write/edit/delete | `file_path` |
| `OnStartup` | `engine/orchestrator.rs` — end of `initialize()` | (empty) |
| `OnShutdown` | `engine/orchestrator.rs` — start of `shutdown()` | (empty) |

## Config Schema

```yaml
# ~/.config/anacleto/config.yaml
hooks:
  on_startup:
    - type: shell
      command: "codegraph sync"
      timeout_secs: 60
  on_shutdown:
    - type: shell
      command: "echo 'engine stopped' >> /tmp/anacleto.log"
  before_tool:
    - type: shell
      command: "echo 'tool: {{tool_name}}' >> /tmp/hooks.log"
  after_apply:
    - type: shell
      command: "codegraph sync"
  before_shell:
    - type: shell
      command: "echo 'running: {{shell_command}}'"
```

Template variables `{{tool_name}}`, `{{file_path}}`, `{{shell_command}}`, `{{agent_name}}` are substituted from `HookContext` before execution.

## Testing

- **Unit tests in `src/hook/mod.rs`:**
  - `HookRegistry::run` with a valid shell command returns `Ok` with captured stdout
  - `HookRegistry::run` with a failing command returns `Ok` with non-zero exit code (does not propagate error)
  - `HookRegistry::run` with a timeout returns timeout error gracefully
  - `HookRegistry::run` with no hooks configured is a no-op
  - `HookPoint` serialization round-trip via serde

- **Integration test in `tests/hook_integration.rs`:**
  - Start engine with hook config pointing to `echo "hello"`
  - Send user input, verify hook fires via event channel
  - Shutdown engine, verify OnShutdown hook fires

- **Config parsing test in `src/config/types.rs`:**
  - Parse YAML with `hooks` section, verify `HookActionConfig` deserialization

- **No regression:** existing tests pass without hooks configured (empty `hooks: {}` is the default)