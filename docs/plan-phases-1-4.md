# Anacleto — Phases 1–4 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take Anacleto from feature-complete prototype to production-ready — fix all lint/format issues, add documentation, professionalize the project (CI/CD, Docker, versioning), and add advanced testing.

**Architecture:** Four independent phases executed sequentially. Phase 1 touches only existing source files (fixing warnings, adding type aliases, implementing Default). Phase 2 creates documentation files only (README, user guide, example, rustdoc). Phase 3 adds CI/CD, Docker, changelog, and toolchain pinning. Phase 4 adds test infrastructure (mock MCP server, property tests, coverage, concurrency tests).

**Tech Stack:** Rust 2024 edition, tokio, ratatui, sqlx, serde, reqwest, tower, clap, proptest (new), cargo-tarpaulin (new)

## Global Constraints

- Rust edition 2024, rustc ≥ 1.85 (current: 1.97.0)
- No new dependencies beyond those listed in Cargo.toml unless explicitly noted in a task
- All clippy warnings must be fixed before any feature work
- `cargo fmt --check` must pass before commits
- All existing tests must continue to pass after each change
- Documentation files go in `docs/` (README.md at project root)
- CI/CD uses GitHub Actions only
- Docker image must be < 100MB

---

## Phase 1: Limpieza — Fix all clippy warnings + fmt

**Goal:** Zero clippy warnings, zero formatting issues. All 14 warnings fixed, `cargo fmt --all` applied.

**Files modified:**
- `src/agent/retry.rs` — 2 fixes (empty_line_after_doc_comments, let_unit_value)
- `src/agent/types.rs` — 1 fix (new_without_default for AgentId)
- `src/agent/lifecycle.rs` — 6 fixes (type_complexity x2, too_many_arguments x2, needless_borrow x2, redundant_closure)
- `src/llm/provider.rs` — 3 fixes (dead_code x2, new_without_default for LlmProviderRegistry)
- `src/mcp/client.rs` — 1 fix (new_without_default for McpRegistry)

### Task 1.1: Fix `empty_line_after_doc_comments` in retry.rs

**Files:**
- Modify: `src/agent/retry.rs:1-6`

- [ ] **Step 1: Remove the empty line after the module-level doc comment**

  The module doc comment on lines 1-5 is followed by a blank line (line 6). Remove it so the doc comment is immediately followed by `use rand::...`.

  ```rust
  /// Retry helper with exponential backoff + jitter.
  ///
  /// Generic over any async operation returning `Result<T, E>` where `E: std::fmt::Display`.
  /// Retries on any error, up to `config.max_retries` times, with exponential
  /// backoff (base_delay_ms × 2^attempt) + 25% random jitter, capped at max_delay_ms.
  use rand::rngs::OsRng;
  ```

  Change to:

  ```rust
  /// Retry helper with exponential backoff + jitter.
  ///
  /// Generic over any async operation returning `Result<T, E>` where `E: std::fmt::Display`.
  /// Retries on any error, up to `config.max_retries` times, with exponential
  /// backoff (base_delay_ms × 2^attempt) + 25% random jitter, capped at max_delay_ms.
  use rand::rngs::OsRng;
  ```

  (Just delete the blank line between the `///` block and `use rand::...`)

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep empty_line_after_doc_comments`
  Expected: no output (warning gone)

- [ ] **Step 3: Commit**

  ```bash
  git add src/agent/retry.rs
  git commit -m "fix: remove empty line after doc comment in retry.rs"
  ```

### Task 1.2: Fix `let_unit_value` in retry.rs

**Files:**
- Modify: `src/agent/retry.rs:43`

- [ ] **Step 1: Remove `let _ =` from the tracing::warn! call**

  Change line 43 from:
  ```rust
  let _ = tracing::warn!(
      "{} attempt {}/{}, retrying in {}ms",
      operation_name,
      attempt,
      config.max_retries,
      delay_ms,
  );
  ```

  To:
  ```rust
  tracing::warn!(
      "{} attempt {}/{}, retrying in {}ms",
      operation_name,
      attempt,
      config.max_retries,
      delay_ms,
  );
  ```

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep let_unit_value`
  Expected: no output

- [ ] **Step 3: Run tests**

  Run: `cargo test test_retry`
  Expected: all retry tests pass

- [ ] **Step 4: Commit**

  ```bash
  git add src/agent/retry.rs
  git commit -m "fix: remove let _ = from tracing::warn in retry.rs"
  ```

### Task 1.3: Fix `dead_code` for `tool_calls` in provider.rs

**Files:**
- Modify: `src/llm/provider.rs:137,239`

- [ ] **Step 1: Rename `tool_calls` to `_tool_calls` in OpenAiStreamDelta**

  Change line 137:
  ```rust
  tool_calls: Option<Vec<OpenAiToolCall>>,
  ```
  To:
  ```rust
  _tool_calls: Option<Vec<OpenAiToolCall>>,
  ```

- [ ] **Step 2: Rename `tool_calls` to `_tool_calls` in OllamaResponseMessage**

  Change line 239:
  ```rust
  tool_calls: Option<Vec<serde_json::Value>>,
  ```
  To:
  ```rust
  _tool_calls: Option<Vec<serde_json::Value>>,
  ```

- [ ] **Step 3: Verify the fixes**

  Run: `cargo clippy 2>&1 | grep "tool_calls.*never.read"`
  Expected: no output

- [ ] **Step 4: Commit**

  ```bash
  git add src/llm/provider.rs
  git commit -m "fix: prefix unused tool_calls fields with underscore"
  ```

### Task 1.4: Fix `type_complexity` — add type alias for PendingApprovals

**Files:**
- Modify: `src/agent/lifecycle.rs:1-20` (add type alias after imports)

- [ ] **Step 1: Add the type alias after the imports**

  After line 21 (`use crate::skill::types::Skill;`), add:
  ```rust
  /// Shared map of pending approval requests, keyed by request ID.
  type PendingApprovals = Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>>;
  ```

- [ ] **Step 2: Replace the inline type in `spawn_agent` signature**

  Find line 63 (the `pending_approvals` parameter in `spawn_agent`):
  ```rust
  pending_approvals: Option<Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>>>,
  ```
  Replace with:
  ```rust
  pending_approvals: Option<PendingApprovals>,
  ```

- [ ] **Step 3: Replace the inline type in `spawn_subagent_and_delegate` signature**

  Find the second occurrence (around line 442, the `pending_approvals` parameter in `spawn_subagent_and_delegate`):
  ```rust
  pending_approvals: Option<Arc<tokio::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<bool>>>>>,
  ```
  Replace with:
  ```rust
  pending_approvals: Option<PendingApprovals>,
  ```

- [ ] **Step 4: Verify the fix**

  Run: `cargo clippy 2>&1 | grep type_complexity`
  Expected: no output

- [ ] **Step 5: Run tests**

  Run: `cargo test`
  Expected: all 15 tests pass

- [ ] **Step 6: Commit**

  ```bash
  git add src/agent/lifecycle.rs
  git commit -m "fix: add PendingApprovals type alias for complex type"
  ```

### Task 1.5: Fix `too_many_arguments` — create SpawnAgentConfig struct

**Files:**
- Modify: `src/agent/lifecycle.rs:50-69` (spawn_agent signature)

- [ ] **Step 1: Add the SpawnAgentConfig struct before spawn_agent**

  Before the `spawn_agent` function (around line 50), add:
  ```rust
  /// Configuration for spawning an agent task.
  #[allow(dead_code)]
  pub struct SpawnAgentConfig {
      pub agent: Agent,
      pub provider: Arc<dyn LlmProvider>,
      pub skills: Vec<Skill>,
      pub subagent_configs: Vec<AgentConfig>,
      pub llm_registry: LlmProviderRegistry,
      pub mcp_registry: Option<Arc<tokio::sync::Mutex<McpRegistry>>>,
      pub pending_approvals: Option<PendingApprovals>,
      pub db: Option<Database>,
      pub session_id: Option<Uuid>,
      pub event_tx: mpsc::Sender<EngineEvent>,
      pub history_limit_percent: f64,
      pub retry_config: RetryConfig,
  }
  ```

- [ ] **Step 2: Refactor `spawn_agent` to accept the struct**

  Change the `spawn_agent` function signature from 12 individual parameters to:
  ```rust
  pub fn spawn_agent(config: SpawnAgentConfig) -> AgentHandle {
  ```

  Inside the function body, replace all uses of the old parameter names with `config.agent`, `config.provider`, etc.

  The function body starts with:
  ```rust
      let (tx, mut rx) = mpsc::channel::<AgentMessage>(256);
      let handle = AgentHandle::new(tx);

      let agent_id = config.agent.id.clone();
      let agent_name = config.agent.name.clone();
      let model = config.agent.model.clone();
      let agent_permissions = config.agent.permissions.clone();

      // Load agent description as system prompt
      let system_prompt = load_agent_description(&config.agent.description_path);

      // Build tool list: skills + subagents
      let mut tools: Vec<ToolDefinition> = config.skills.iter().map(skill_to_tool_definition).collect();
      for sc in &config.subagent_configs {
          tools.push(subagent_config_to_tool_definition(sc));
      }

      // Clone what the task needs
      let agent_mcp_names = config.agent.mcps.clone();
  ```

  Continue replacing every reference: `provider` → `config.provider`, `skills` → `config.skills`, `subagent_configs` → `config.subagent_configs`, `llm_registry` → `config.llm_registry`, `mcp_registry` → `config.mcp_registry`, `pending_approvals` → `config.pending_approvals`, `db` → `config.db`, `session_id` → `config.session_id`, `event_tx` → `config.event_tx`, `history_limit_percent` → `config.history_limit_percent`, `retry_config` → `config.retry_config`.

- [ ] **Step 3: Update all callers of `spawn_agent`**

  Search for `spawn_agent(` in the file and in `src/engine/orchestrator.rs`. Replace each call from:
  ```rust
  spawn_agent(agent, provider, skills, subagent_configs, llm_registry, mcp_registry, pending_approvals, db, session_id, event_tx, history_limit_percent, retry_config)
  ```
  To:
  ```rust
  spawn_agent(SpawnAgentConfig {
      agent,
      provider,
      skills,
      subagent_configs,
      llm_registry,
      mcp_registry,
      pending_approvals,
      db,
      session_id,
      event_tx,
      history_limit_percent,
      retry_config,
  })
  ```

- [ ] **Step 4: Verify the fix**

  Run: `cargo clippy 2>&1 | grep "too_many_arguments.*spawn_agent"`
  Expected: no output

- [ ] **Step 5: Run tests**

  Run: `cargo test`
  Expected: all tests pass

- [ ] **Step 6: Commit**

  ```bash
  git add src/agent/lifecycle.rs src/engine/orchestrator.rs
  git commit -m "fix: refactor spawn_agent to use SpawnAgentConfig struct"
  ```

### Task 1.6: Fix `too_many_arguments` — create SpawnSubagentConfig struct

**Files:**
- Modify: `src/agent/lifecycle.rs` (spawn_subagent_and_delegate signature)

- [ ] **Step 1: Add the SpawnSubagentConfig struct**

  Before the `spawn_subagent_and_delegate` function (around line 430), add:
  ```rust
  /// Configuration for spawning a subagent and delegating a task to it.
  #[allow(dead_code)]
  pub struct SpawnSubagentConfig {
      pub parent_id: AgentId,
      pub parent_name: String,
      pub subagent_config: AgentConfig,
      pub task: String,
      pub context: Vec<MessageEntry>,
      pub provider: Arc<dyn LlmProvider>,
      pub llm_registry: LlmProviderRegistry,
      pub mcp_registry: Option<Arc<tokio::sync::Mutex<McpRegistry>>>,
      pub pending_approvals: Option<PendingApprovals>,
      pub db: Option<Database>,
      pub session_id: Option<Uuid>,
      pub event_tx: mpsc::Sender<EngineEvent>,
      pub history_limit_percent: f64,
      pub retry_config: RetryConfig,
  }
  ```

- [ ] **Step 2: Refactor `spawn_subagent_and_delegate` to accept the struct**

  Change the function signature from individual parameters to:
  ```rust
  pub fn spawn_subagent_and_delegate(config: SpawnSubagentConfig) -> AgentHandle {
  ```

  Replace all parameter references inside the function body with `config.parent_id`, `config.parent_name`, etc.

- [ ] **Step 3: Update all callers of `spawn_subagent_and_delegate`**

  Search for `spawn_subagent_and_delegate(` in the file and in `src/engine/orchestrator.rs`. Replace each call with the struct literal form.

- [ ] **Step 4: Verify the fix**

  Run: `cargo clippy 2>&1 | grep "too_many_arguments"`
  Expected: no output

- [ ] **Step 5: Run tests**

  Run: `cargo test`
  Expected: all tests pass

- [ ] **Step 6: Commit**

  ```bash
  git add src/agent/lifecycle.rs src/engine/orchestrator.rs
  git commit -m "fix: refactor spawn_subagent_and_delegate to use SpawnSubagentConfig struct"
  ```

### Task 1.7: Fix `needless_borrow` (ref db) in lifecycle.rs

**Files:**
- Modify: `src/agent/lifecycle.rs:144,257`

- [ ] **Step 1: Fix first occurrence (around line 144)**

  Find:
  ```rust
  if let Some(ref db) = db {
  ```
  Change to:
  ```rust
  if let Some(db) = db {
  ```

- [ ] **Step 2: Fix second occurrence (around line 257)**

  Find:
  ```rust
  if let Some(ref db) = db {
  ```
  Change to:
  ```rust
  if let Some(db) = db {
  ```

- [ ] **Step 3: Verify the fix**

  Run: `cargo clippy 2>&1 | grep needless_borrow`
  Expected: no output

- [ ] **Step 4: Commit**

  ```bash
  git add src/agent/lifecycle.rs
  git commit -m "fix: remove needless ref borrow in lifecycle.rs"
  ```

### Task 1.8: Fix `redundant_closure` in lifecycle.rs

**Files:**
- Modify: `src/agent/lifecycle.rs:747`

- [ ] **Step 1: Simplify the closure**

  Find (around line 747):
  ```rust
  .map_err(|e| Error::Provider(e))?
  ```
  Change to:
  ```rust
  .map_err(Error::Provider)?
  ```

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep redundant_closure`
  Expected: no output

- [ ] **Step 3: Commit**

  ```bash
  git add src/agent/lifecycle.rs
  git commit -m "fix: simplify redundant closure in lifecycle.rs"
  ```

### Task 1.9: Fix `new_without_default` for AgentId

**Files:**
- Modify: `src/agent/types.rs:12-16`

- [ ] **Step 1: Add `impl Default for AgentId`**

  After the existing `impl AgentId` block (line 16), add:
  ```rust
  impl Default for AgentId {
      fn default() -> Self {
          Self::new()
      }
  }
  ```

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep "new_without_default.*AgentId"`
  Expected: no output

- [ ] **Step 3: Commit**

  ```bash
  git add src/agent/types.rs
  git commit -m "fix: add Default impl for AgentId"
  ```

### Task 1.10: Fix `new_without_default` for LlmProviderRegistry

**Files:**
- Modify: `src/llm/provider.rs:1105-1110`

- [ ] **Step 1: Add `impl Default for LlmProviderRegistry`**

  After the `impl LlmProviderRegistry` block (around line 1119), add:
  ```rust
  impl Default for LlmProviderRegistry {
      fn default() -> Self {
          Self::new()
      }
  }
  ```

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep "new_without_default.*LlmProviderRegistry"`
  Expected: no output

- [ ] **Step 3: Commit**

  ```bash
  git add src/llm/provider.rs
  git commit -m "fix: add Default impl for LlmProviderRegistry"
  ```

### Task 1.11: Fix `new_without_default` for McpRegistry

**Files:**
- Modify: `src/mcp/client.rs:288-293`

- [ ] **Step 1: Add `impl Default for McpRegistry`**

  After the `impl McpRegistry` block (around line 310), add:
  ```rust
  impl Default for McpRegistry {
      fn default() -> Self {
          Self::new()
      }
  }
  ```

- [ ] **Step 2: Verify the fix**

  Run: `cargo clippy 2>&1 | grep "new_without_default.*McpRegistry"`
  Expected: no output

- [ ] **Step 3: Commit**

  ```bash
  git add src/mcp/client.rs
  git commit -m "fix: add Default impl for McpRegistry"
  ```

### Task 1.12: Run cargo fmt --all

**Files:**
- Modify: all source files

- [ ] **Step 1: Format all files**

  Run: `cargo fmt --all`
  Expected: no output (success)

- [ ] **Step 2: Verify no remaining clippy warnings**

  Run: `cargo clippy 2>&1`
  Expected: no warnings, no errors. Output should end with "warning: 0 generated" or similar.

- [ ] **Step 3: Verify all tests still pass**

  Run: `cargo test`
  Expected: all 15 tests pass

- [ ] **Step 4: Commit**

  ```bash
  git add -A
  git commit -m "style: cargo fmt --all"
  ```

---

## Phase 2: Documentación

**Goal:** Comprehensive documentation — README, user guide, end-to-end example, and rustdoc on all public API.

**Files created:**
- `README.md` (project root)
- `docs/user-guide.md`
- `docs/example.md`

**Files modified:**
- `src/agent/types.rs` — rustdoc on Agent, AgentId, AgentMessage, etc.
- `src/agent/lifecycle.rs` — rustdoc on AgentHandle, spawn_agent, spawn_subagent_and_delegate
- `src/config/types.rs` — rustdoc on Config, AgentConfig
- `src/engine/orchestrator.rs` — rustdoc on Engine, EngineCommand, EngineEvent
- `src/llm/provider.rs` — rustdoc on LlmProvider trait
- `src/mcp/client.rs` — rustdoc on McpRegistry
- `src/permissions/checker.rs` — rustdoc on public functions
- `src/skill/loader.rs` — rustdoc on public functions

### Task 2.1: Write README.md

**Files:**
- Create: `README.md`

- [ ] **Step 1: Write the README**

  ```markdown
  # Anacleto

  Agent orchestration engine in Rust — agents, subagents, skills, and MCPs.

  Anacleto manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a TUI built with ratatui + crossterm.

  ## Quickstart

  ### Prerequisites

  - Rust 1.85+ (edition 2024)
  - An LLM provider: Ollama (local), Anthropic, or OpenAI

  ### Install

  ```bash
  git clone https://github.com/your-org/anacleto.git
  cd anacleto
  cargo build --release
  ```

  ### Configure

  Create `~/.config/anacleto/config.yaml`:

  ```yaml
  llm:
    provider: ollama
    model: llama3.2
    url: http://localhost:11434

  agents:
    - name: root
      description: ~/.config/anacleto/agents/root.md
      model: llama3.2
      skills:
        - ~/.config/anacleto/skills/shell/
      mcps: [filesystem]
      permissions:
        deny: []
      subagents: [reviewer]

    - name: reviewer
      description: ~/.config/anacleto/agents/reviewer.md
      model: llama3.2
      skills:
        - ~/.config/anacleto/skills/code-review/
      permissions:
        deny: [command.run, net.http]
  ```

  ### Run

  ```bash
  cargo run
  ```

  ## Architecture

  ```
  src/
    main.rs              # Entrypoint, CLI arg parsing
    tui/                 # ratatui + crossterm (sole interface)
    engine/              # Orchestration loop (spawn, route, collect)
    agent/               # Agent/subagent types, lifecycle, communication
    skill/               # Skill loading (Anthropic Markdown format), execution
    mcp/                 # MCP client (JSON-RPC 2.0 over stdio/TCP)
    llm/                 # LLM providers (Anthropic, OpenAI, Ollama)
    config/              # YAML config parsing, global + project merge
    permissions/         # Permission rules per agent/subagent
    db/                  # SQLite persistence via sqlx
    error.rs             # Global error types
  ```

  ### Key Concepts

  - **Agents and subagents** are the same type. Agents have `subagents: []`. Only agents are user-invocable. Subagents cannot nest.
  - **Skills** are Markdown + YAML frontmatter (Anthropic format), loaded per-agent.
  - **MCPs** are consumed via JSON-RPC 2.0 over stdio or TCP. Per-agent, no inheritance.
  - **Permissions** are deny-by-default for sensitive operations (command.run, fs.write, net.http).
  - **Sessions** are persisted to SQLite and are resumable.

  ## Commands

  | Command | Description |
  |---|---|
  | `/help` | Show available TUI commands |
  | `/resume <session_id>` | Resume a previous session |
  | `/delete <session_id>` | Delete a session |
  | `/rename <session_id> <name>` | Rename a session |
  | `Ctrl+C` | Quit |

  ## Examples

  See [docs/example.md](docs/example.md) for a complete end-to-end walkthrough.

  ## Project Status

  Feature-complete against the original ADRs. See [TODO.md](TODO.md) for the production-readiness roadmap.

  ## Architecture Decision Records

  Key decisions are documented in [docs/adr/](docs/adr/):

  | ADR | Decision |
  |---|---|
  | 001 | Agent model: agents and subagents are the same type |
  | 002 | Subagent lifecycle: disposable, no inheritance |
  | 003 | Skills: Markdown + YAML frontmatter |
  | 004 | MCPs: consumer only, no lifecycle management |
  | 005 | TUI: ratatui + crossterm, same process |
  | 006 | Config: YAML, global + project merged |
  | 007 | Persistence: SQLite via sqlx |
  | 008 | Permissions: allow by default, deny explicitly |

  ## License

  MIT
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add README.md
  git commit -m "docs: add README.md"
  ```

### Task 2.2: Write user guide

**Files:**
- Create: `docs/user-guide.md`

- [ ] **Step 1: Write the user guide**

  ```markdown
  # Anacleto User Guide

  ## Configuration

  Anacleto uses YAML configuration merged from two locations:

  1. **Global:** `~/.config/anacleto/config.yaml` — shared across all projects
  2. **Project:** `.agents/config.yaml` — per-project overrides

  Project config takes precedence over global config (deep merge).

  ### Global Config Structure

  ```yaml
  # ~/.config/anacleto/config.yaml

  llm:
    provider: ollama          # ollama | anthropic | openai
    model: llama3.2
    url: http://localhost:11434
    api_key: null             # set via ANTHROPIC_API_KEY or OPENAI_API_KEY env vars

  agents:
    - name: root
      description: ~/.config/anacleto/agents/root.md
      model: llama3.2
      skills:
        - ~/.config/anacleto/skills/shell/
      mcps: [filesystem]
      permissions:
        deny: []
      subagents: [reviewer]

    - name: reviewer
      description: ~/.config/anacleto/agents/reviewer.md
      model: llama3.2
      skills:
        - ~/.config/anacleto/skills/code-review/
      permissions:
        deny: [command.run, net.http]

  mcps:
    filesystem:
      command: npx
      args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
      transport: stdio

  sessions:
    history_limit_percent: 50   # % of context window for history
    retry:
      max_retries: 3
      base_delay_ms: 1000
      max_delay_ms: 30000
  ```

  ### Project Config Example

  ```yaml
  # .agents/config.yaml
  agents:
    - name: root
      description: .agents/agents/root.md
      model: llama3.2
      skills:
        - .agents/skills/shell/
      mcps: [filesystem]
      permissions:
        deny: []
      subagents: [reviewer]
  ```

  ## Skills

  Skills are Markdown files with YAML frontmatter (Anthropic format):

  ```markdown
  ---
  name: shell
  description: Execute shell commands
  tools:
    - name: command_run
      description: Run a shell command and return output
      parameters:
        type: object
        properties:
          command:
            type: string
            description: The command to run
        required: [command]
  ---

  You are a shell command expert. Execute commands safely and return their output.
  ```

  Skills are loaded from the paths specified in the agent config. Each agent has its own set of skills (no inheritance from parent).

  ## MCPs (Model Context Protocol)

  MCP servers are defined globally in the config and referenced by name in agent configs:

  ```yaml
  mcps:
    filesystem:
      command: npx
      args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
      transport: stdio

    web-search:
      command: python
      args: ["mcp-server-web-search.py"]
      transport: stdio
      env:
        API_KEY: "${SEARCH_API_KEY}"
  ```

  Supported transports:
  - `stdio` — spawn a subprocess and communicate over stdin/stdout
  - `tcp` — connect to a TCP server (future)

  ## Agents and Subagents

  - **Agents** are top-level, user-invocable entities defined in config.
  - **Subagents** are created by agents at runtime for task delegation.
  - Subagents are **disposable**: create → work → reply → destroy.
  - Subagents do **not** inherit skills, MCPs, or permissions from their parent.

  ## Sessions

  Sessions are persisted to SQLite (`~/.local/share/anacleto/sessions.db`). Each session stores:
  - Conversation history
  - Agent state
  - Session metadata (name, created_at, updated_at)

  Sessions are resumable via the `/resume` TUI command.

  ## Permissions

  Permissions control what operations an agent can perform:

  | Permission | Description |
  |---|---|
  | `command.run` | Execute shell commands |
  | `fs.write` | Write to the filesystem |
  | `net.http` | Make HTTP requests |
  | `skill.use` | Use loaded skills |

  Default behavior: **allow** (operations are permitted unless explicitly denied).

  ```yaml
  permissions:
    deny: [command.run, net.http]
  ```

  When a denied operation is attempted, the user is prompted for approval in the TUI.

  ## TUI Commands

  | Command | Description |
  |---|---|
  | `/help` | Show available commands |
  | `/resume <session_id>` | Resume a previous session by ID |
  | `/delete <session_id>` | Delete a session |
  | `/rename <session_id> <name>` | Rename a session |
  | `Ctrl+C` | Quit the application |

  The TUI shows:
  - Conversation history (scrollable)
  - Agent status indicators
  - Intermediate skill/MCP execution steps (streaming)
  - Permission approval prompts
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add docs/user-guide.md
  git commit -m "docs: add user guide"
  ```

### Task 2.3: Write end-to-end example

**Files:**
- Create: `docs/example.md`

- [ ] **Step 1: Write the example**

  ```markdown
  # End-to-End Example: Code Review with Anacleto

  This walkthrough shows how to set up Anacleto with a local Ollama instance, create a project with a code review skill, and run a review session with subagents.

  ## Prerequisites

  - Docker and Docker Compose
  - Rust 1.85+

  ## Step 1: Project Setup

  Create a new project directory:

  ```bash
  mkdir my-anacleto-project
  cd my-anacleto-project
  ```

  Create the project config directory:

  ```bash
  mkdir -p .agents/agents .agents/skills
  ```

  ## Step 2: Create Agent Descriptions

  Create `.agents/agents/root.md`:

  ```markdown
  You are a senior software engineer. You help users with coding tasks,
  code review, and architecture decisions. You can delegate specialized
  tasks to your subagent reviewers.
  ```

  Create `.agents/agents/reviewer.md`:

  ```markdown
  You are a meticulous code reviewer. Focus on:
  - Security vulnerabilities
  - Performance issues
  - Code style and maintainability
  - Error handling
  - Test coverage

  Provide actionable feedback with specific line references.
  ```

  ## Step 3: Create a Skill

  Create `.agents/skills/shell/SKILL.md`:

  ```markdown
  ---
  name: shell
  description: Execute shell commands in the project directory
  tools:
    - name: command_run
      description: Run a shell command and return its stdout/stderr
      parameters:
        type: object
        properties:
          command:
            type: string
            description: The shell command to execute
        required: [command]
  ---

  You can execute shell commands to read files, run tests, and inspect code.
  Always prefer reading files with `cat` or `head` before making changes.
  ```

  ## Step 4: Create Project Config

  Create `.agents/config.yaml`:

  ```yaml
  llm:
    provider: ollama
    model: llama3.2
    url: http://localhost:11434

  agents:
    - name: root
      description: .agents/agents/root.md
      model: llama3.2
      skills:
        - .agents/skills/shell/
      permissions:
        deny: []
      subagents: [reviewer]

    - name: reviewer
      description: .agents/agents/reviewer.md
      model: llama3.2
      permissions:
        deny: [command.run, net.http]

  sessions:
    history_limit_percent: 50
    retry:
      max_retries: 3
      base_delay_ms: 1000
      max_delay_ms: 30000
  ```

  ## Step 5: Docker Compose for Ollama

  Create `docker-compose.yml`:

  ```yaml
  version: "3.8"

  services:
    ollama:
      image: ollama/ollama:latest
      ports:
        - "11434:11434"
      volumes:
        - ollama_data:/root/.ollama
      restart: unless-stopped

  volumes:
    ollama_data:
  ```

  Start Ollama:

  ```bash
  docker compose up -d
  ```

  Pull a model:

  ```bash
  docker compose exec ollama ollama pull llama3.2
  ```

  ## Step 6: Build and Run Anacleto

  ```bash
  # From the anacleto project directory
  cargo build --release

  # Run from your project directory
  cargo run --release
  ```

  ## Step 7: Interact

  Once the TUI starts, type a message like:

  ```
  Review the code in src/main.rs for potential issues
  ```

  Anacleto will:
  1. Load the `root` agent with the shell skill
  2. Read `src/main.rs` using the shell skill
  3. Delegate the review to the `reviewer` subagent
  4. The reviewer analyzes the code and returns feedback
  5. The root agent presents the results

  ## Step 8: Using Subagents

  When the root agent decides a task needs specialized review, it creates a subagent dynamically. The subagent:
  - Gets its own description (system prompt)
  - Has restricted permissions (no command.run or net.http)
  - Processes the delegated task
  - Returns its response and is destroyed

  You'll see subagent activity in the TUI as intermediate steps.

  ## TUI Commands During Session

  ```
  /help          # Show available commands
  /resume <id>   # Resume a previous session
  /delete <id>   # Delete a session
  /rename <id> n # Rename a session
  ```

  ## Cleanup

  ```bash
  docker compose down
  ```
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add docs/example.md
  git commit -m "docs: add end-to-end example"
  ```

### Task 2.4: Add rustdoc to public API

**Files:**
- Modify: `src/agent/types.rs`, `src/agent/lifecycle.rs`, `src/config/types.rs`, `src/engine/orchestrator.rs`, `src/llm/provider.rs`, `src/mcp/client.rs`, `src/permissions/checker.rs`, `src/skill/loader.rs`

- [ ] **Step 1: Add rustdoc to `src/agent/types.rs`**

  Add `///` doc comments to:
  - `AgentId` — "Unique identifier for an agent or subagent, backed by a UUIDv4."
  - `AgentRole` — "The role of an agent in the hierarchy: Root (top-level, user-invocable) or SubAgent (disposable, created by parent)."
  - `Agent` — "An agent or subagent instance with its configuration, permissions, and metadata."
  - `Agent::from_config` — "Create a new agent from an `AgentConfig` and role. Generates a new UUID."
  - `Agent::create_subagent` — "Create a subagent from explicit parameters with a parent reference."
  - `Agent::is_root` — "Whether this agent is a root-level (user-invocable) agent."
  - `Agent::is_subagent` — "Whether this agent is a subagent (created by a parent)."
  - `AgentMessage` — "Message sent between agents or between the engine and an agent."
  - `MessageEntry` — "A single entry in a message history with role, content, and timestamp."
  - `MessageRole` — "Role of a message sender: User, Assistant, System, or Tool."
  - `AgentStatus` — "Status of an agent's lifecycle: Idle, Working, WaitingForSubAgent, Completed, or Error."

- [ ] **Step 2: Add rustdoc to `src/agent/lifecycle.rs`**

  Add `///` doc comments to:
  - `AgentHandle` — "Handle for communicating with a running agent task. Provides a channel to send messages."
  - `AgentHandle::new` — "Create a new AgentHandle with the given sender channel."
  - `AgentHandle::send` — "Send a message to the agent. Returns `Err` if the channel is closed."
  - `SpawnAgentConfig` — "Configuration for spawning an agent task. Use with `spawn_agent()`."
  - `spawn_agent` — "Spawn a new agent task and return a handle. The agent runs in a tokio task, processing messages from its channel."
  - `SpawnSubagentConfig` — "Configuration for spawning a subagent and delegating a task to it."
  - `spawn_subagent_and_delegate` — "Spawn a subagent, delegate a task with context, and return a handle."

- [ ] **Step 3: Add rustdoc to `src/config/types.rs`**

  Read the file first to see existing types, then add `///` doc comments to:
  - `Config` — "Top-level configuration, merged from global (`~/.config/anacleto/`) and project (`.agents/`) sources."
  - `AgentConfig` — "Configuration for a single agent or subagent, as defined in YAML."

- [ ] **Step 4: Add rustdoc to `src/engine/orchestrator.rs`**

  Read the file first, then add `///` doc comments to:
  - `Engine` — "The main orchestration engine. Manages agent lifecycle, message routing, and session persistence."
  - `EngineCommand` — "Commands that can be sent to the engine via its command channel."
  - `EngineEvent` — "Events emitted by the engine to the TUI for display."

- [ ] **Step 5: Add rustdoc to `src/llm/provider.rs`**

  Add `///` doc comments to:
  - `LlmProvider` trait — "Trait for LLM providers (Anthropic, OpenAI, Ollama). Implementations handle authentication, request formatting, and response parsing."
  - `LlmProviderRegistry` — "Registry of named LLM providers. Providers are registered by name and retrieved for use by agents."

- [ ] **Step 6: Add rustdoc to `src/mcp/client.rs`**

  Add `///` doc comments to:
  - `McpRegistry` — "Registry of connected MCP clients. Manages the lifecycle of MCP connections and provides tool discovery."

- [ ] **Step 7: Add rustdoc to `src/permissions/checker.rs`**

  Read the file first, then add `///` doc comments to:
  - `check_command_run` — "Check whether the agent is permitted to run a shell command."
  - `check_fs_write` — "Check whether the agent is permitted to write to the filesystem."
  - `check_net_http` — "Check whether the agent is permitted to make HTTP requests."
  - `check_skill_use` — "Check whether the agent is permitted to use a specific skill."

- [ ] **Step 8: Add rustdoc to `src/skill/loader.rs`**

  Read the file first, then add `///` doc comments to:
  - `load_agent_skills` — "Load all skills for an agent from the configured skill paths."

- [ ] **Step 9: Verify rustdoc builds**

  Run: `cargo doc --no-deps 2>&1`
  Expected: no warnings about missing documentation

- [ ] **Step 10: Commit**

  ```bash
  git add src/agent/types.rs src/agent/lifecycle.rs src/config/types.rs src/engine/orchestrator.rs src/llm/provider.rs src/mcp/client.rs src/permissions/checker.rs src/skill/loader.rs
  git commit -m "docs: add rustdoc to public API"
  ```

---

## Phase 3: Profesionalización

**Goal:** CI/CD pipeline, changelog, Docker image, and Rust toolchain pinning.

**Files created:**
- `.github/workflows/ci.yml`
- `CHANGELOG.md`
- `Dockerfile`
- `.dockerignore`
- `rust-toolchain.toml`

### Task 3.1: Create CI/CD pipeline

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the CI workflow**

  ```yaml
  name: CI

  on:
    push:
      branches: [main]
    pull_request:
      branches: [main]

  env:
    CARGO_TERM_COLOR: always

  jobs:
    build:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo build --verbose

    test:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo test --verbose

    clippy:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo clippy -- -D warnings

    fmt:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - run: cargo fmt --check
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/ci.yml
  git commit -m "ci: add GitHub Actions CI pipeline"
  ```

### Task 3.2: Create CHANGELOG.md

**Files:**
- Create: `CHANGELOG.md`

- [ ] **Step 1: Write the changelog**

  ```markdown
  # Changelog

  All notable changes to this project will be documented in this file.

  The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
  and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

  ## [0.1.0] - 2026-08-02

  ### Added

  - Initial release of Anacleto agent orchestration engine
  - Agent and subagent lifecycle management
  - Skill loading (Anthropic Markdown format)
  - MCP client (JSON-RPC 2.0 over stdio)
  - LLM providers: Anthropic, OpenAI, Ollama
  - TUI interface (ratatui + crossterm)
  - YAML configuration (global + project merge)
  - SQLite session persistence
  - Permission system with user approval
  - Retry with exponential backoff and jitter
  - Streaming support for LLM responses
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add CHANGELOG.md
  git commit -m "docs: add CHANGELOG.md for v0.1.0"
  ```

### Task 3.3: Create Dockerfile

**Files:**
- Create: `Dockerfile`
- Create: `.dockerignore`

- [ ] **Step 1: Create .dockerignore**

  ```
  target/
  .git/
  .gitignore
  .github/
  docs/
  tests/
  *.md
  !README.md
  ```

- [ ] **Step 2: Create multi-stage Dockerfile**

  ```dockerfile
  # Stage 1: Builder
  FROM rust:1.85-slim-bookworm AS builder

  RUN apt-get update && apt-get install -y --no-install-recommends \
      pkg-config libssl-dev && \
      rm -rf /var/lib/apt/lists/*

  WORKDIR /app
  COPY Cargo.toml Cargo.lock ./
  RUN mkdir src && echo "fn main() {}" > src/main.rs
  RUN cargo build --release 2>/dev/null || true
  RUN rm -rf src

  COPY . .
  RUN cargo build --release

  # Stage 2: Runtime
  FROM debian:bookworm-slim

  RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates && \
      rm -rf /var/lib/apt/lists/*

  RUN groupadd -r anacleto && useradd -r -g anacleto -m -d /home/anacleto anacleto

  COPY --from=builder /app/target/release/anacleto /usr/local/bin/anacleto

  USER anacleto
  WORKDIR /home/anacleto

  ENTRYPOINT ["anacleto"]
  ```

- [ ] **Step 3: Build and verify image size**

  Run: `docker build -t anacleto:latest . 2>&1 | tail -5`
  Expected: successful build

  Run: `docker images anacleto:latest --format "{{.Size}}"`
  Expected: < 100MB

- [ ] **Step 4: Commit**

  ```bash
  git add Dockerfile .dockerignore
  git commit -m "build: add multi-stage Dockerfile"
  ```

### Task 3.4: Create rust-toolchain.toml

**Files:**
- Create: `rust-toolchain.toml`

- [ ] **Step 1: Write the toolchain file**

  ```toml
  [toolchain]
  channel = "stable"
  edition = "2024"
  ```

- [ ] **Step 2: Verify it's picked up**

  Run: `rustup show`
  Expected: shows stable channel, edition 2024

- [ ] **Step 3: Commit**

  ```bash
  git add rust-toolchain.toml
  git commit -m "chore: pin Rust toolchain with rust-toolchain.toml"
  ```

---

## Phase 4: Testing avanzado

**Goal:** Mock MCP server for integration tests, property-based tests, coverage configuration, and concurrency stress tests.

**Files created:**
- `tests/mocks/mcp_server.py`
- `tests/mcp_integration_test.rs`

**Files modified:**
- `Cargo.toml` — add `proptest` dev dependency
- `.github/workflows/ci.yml` — add coverage job

### Task 4.1: Create mock MCP server

**Files:**
- Create: `tests/mocks/mcp_server.py`

- [ ] **Step 1: Create the mock MCP server**

  ```python
  #!/usr/bin/env python3
  """Mock MCP server for integration testing.

  Implements a minimal stdio-based MCP server that supports:
  - initialize (capabilities negotiation)
  - tools/list (returns echo, add, reverse tools)
  - tools/call (executes the requested tool)

  Communicates via JSON-RPC 2.0 over stdin/stdout.
  """

  import json
  import sys


  TOOLS = [
      {
          "name": "echo",
          "description": "Echo back the input",
          "inputSchema": {
              "type": "object",
              "properties": {
                  "message": {"type": "string", "description": "Message to echo"}
              },
              "required": ["message"],
          },
      },
      {
          "name": "add",
          "description": "Add two numbers",
          "inputSchema": {
              "type": "object",
              "properties": {
                  "a": {"type": "number", "description": "First number"},
                  "b": {"type": "number", "description": "Second number"},
              },
              "required": ["a", "b"],
          },
      },
      {
          "name": "reverse",
          "description": "Reverse a string",
          "inputSchema": {
              "type": "object",
              "properties": {
                  "text": {"type": "string", "description": "Text to reverse"},
              },
              "required": ["text"],
          },
      },
  ]


  def handle_request(request):
      req_id = request.get("id")
      method = request.get("method")
      params = request.get("params", {})

      if method == "initialize":
          return {
              "jsonrpc": "2.0",
              "id": req_id,
              "result": {
                  "protocolVersion": "2024-11-05",
                  "capabilities": {"tools": {}},
                  "serverInfo": {"name": "mock-mcp-server", "version": "0.1.0"},
              },
          }
      elif method == "tools/list":
          return {
              "jsonrpc": "2.0",
              "id": req_id,
              "result": {"tools": TOOLS},
          }
      elif method == "tools/call":
          tool_name = params.get("name")
          arguments = params.get("arguments", {})

          if tool_name == "echo":
              message = arguments.get("message", "")
              return {
                  "jsonrpc": "2.0",
                  "id": req_id,
                  "result": {
                      "content": [
                          {"type": "text", "text": f"Echo: {message}"}
                      ]
                  },
              }
          elif tool_name == "add":
              a = arguments.get("a", 0)
              b = arguments.get("b", 0)
              result = a + b
              return {
                  "jsonrpc": "2.0",
                  "id": req_id,
                  "result": {
                      "content": [
                          {"type": "text", "text": str(result)}
                      ]
                  },
              }
          elif tool_name == "reverse":
              text = arguments.get("text", "")
              return {
                  "jsonrpc": "2.0",
                  "id": req_id,
                  "result": {
                      "content": [
                          {"type": "text", "text": text[::-1]}
                      ]
                  },
              }
          else:
              return {
                  "jsonrpc": "2.0",
                  "id": req_id,
                  "error": {"code": -32601, "message": f"Tool not found: {tool_name}"},
              }
      else:
          return {
              "jsonrpc": "2.0",
              "id": req_id,
              "error": {"code": -32601, "message": f"Method not found: {method}"},
          }


  def main():
      for line in sys.stdin:
          line = line.strip()
          if not line:
              continue
          try:
              request = json.loads(line)
              response = handle_request(request)
              sys.stdout.write(json.dumps(response) + "\n")
              sys.stdout.flush()
          except json.JSONDecodeError as e:
              error_response = {
                  "jsonrpc": "2.0",
                  "id": None,
                  "error": {"code": -32700, "message": f"Parse error: {e}"},
              }
              sys.stdout.write(json.dumps(error_response) + "\n")
              sys.stdout.flush()


  if __name__ == "__main__":
      main()
  ```

- [ ] **Step 2: Make the mock server executable**

  Run: `chmod +x tests/mocks/mcp_server.py`

- [ ] **Step 3: Verify the mock server works**

  Run: `echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | python3 tests/mocks/mcp_server.py`
  Expected: JSON response with server capabilities

- [ ] **Step 4: Commit**

  ```bash
  git add tests/mocks/mcp_server.py
  git commit -m "test: add mock MCP server for integration tests"
  ```

### Task 4.2: Create MCP integration test

**Files:**
- Create: `tests/mcp_integration_test.rs`

- [ ] **Step 1: Write the integration test**

  ```rust
  use std::process::{Command, Stdio};
  use std::time::Duration;
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
  use tokio::process::{Command as TokioCommand};
  use tokio::time::timeout;

  /// Path to the mock MCP server script.
  /// Tests run from the project root, so this is relative.
  const MOCK_SERVER_PATH: &str = "tests/mocks/mcp_server.py";

  /// Test that the mock MCP server responds to initialize.
  #[tokio::test]
  async fn test_mock_server_initialize() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Send initialize request
      let request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(request.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      // Read response with timeout
      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout waiting for response")
          .expect("Failed to read line")
          .expect("Empty response");

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      assert_eq!(parsed["id"], 1);
      assert_eq!(parsed["result"]["serverInfo"]["name"], "mock-mcp-server");

      // Kill the child process
      child.kill().await.unwrap();
  }

  /// Test that the mock MCP server lists tools.
  #[tokio::test]
  async fn test_mock_server_list_tools() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Initialize first
      let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(init.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();
      timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      // List tools
      let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
      writer.write_all(list.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      let tools = parsed["result"]["tools"].as_array().unwrap();
      let tool_names: Vec<&str> = tools
          .iter()
          .map(|t| t["name"].as_str().unwrap())
          .collect();
      assert!(tool_names.contains(&"echo"));
      assert!(tool_names.contains(&"add"));
      assert!(tool_names.contains(&"reverse"));

      child.kill().await.unwrap();
  }

  /// Test calling the echo tool.
  #[tokio::test]
  async fn test_mock_server_call_echo() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Initialize
      let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(init.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();
      timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      // Call echo
      let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"message":"hello world"}}}"#;
      writer.write_all(call.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
      assert_eq!(text, "Echo: hello world");

      child.kill().await.unwrap();
  }

  /// Test calling the add tool.
  #[tokio::test]
  async fn test_mock_server_call_add() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Initialize
      let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(init.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();
      timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      // Call add
      let call = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"add","arguments":{"a":40,"b":2}}}"#;
      writer.write_all(call.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
      assert_eq!(text, "42");

      child.kill().await.unwrap();
  }

  /// Test calling the reverse tool.
  #[tokio::test]
  async fn test_mock_server_call_reverse() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Initialize
      let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(init.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();
      timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      // Call reverse
      let call = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"reverse","arguments":{"text":"anacleto"}}}"#;
      writer.write_all(call.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      let text = parsed["result"]["content"][0]["text"].as_str().unwrap();
      assert_eq!(text, "otcelcana");

      child.kill().await.unwrap();
  }

  /// Test that calling an unknown tool returns an error.
  #[tokio::test]
  async fn test_mock_server_unknown_tool() {
      let mut child = TokioCommand::new("python3")
          .arg(MOCK_SERVER_PATH)
          .stdin(Stdio::piped())
          .stdout(Stdio::piped())
          .stderr(Stdio::piped())
          .spawn()
          .expect("Failed to spawn mock MCP server");

      let stdin = child.stdin.take().expect("Failed to open stdin");
      let stdout = child.stdout.take().expect("Failed to open stdout");
      let mut reader = BufReader::new(stdout).lines();

      // Initialize
      let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
      let mut writer = stdin;
      writer.write_all(init.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();
      timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      // Call unknown tool
      let call = r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nonexistent","arguments":{}}}"#;
      writer.write_all(call.as_bytes()).await.unwrap();
      writer.write_all(b"\n").await.unwrap();
      writer.flush().await.unwrap();

      let response = timeout(Duration::from_secs(5), reader.next_line())
          .await
          .expect("Timeout")
          .unwrap()
          .unwrap();

      let parsed: serde_json::Value = serde_json::from_str(&response).unwrap();
      assert!(parsed["error"].is_object());
      assert_eq!(parsed["error"]["code"], -32601);

      child.kill().await.unwrap();
  }
  ```

- [ ] **Step 2: Run the integration tests**

  Run: `cargo test test_mock_server -- --test-threads=1`
  Expected: all 5 integration tests pass

- [ ] **Step 3: Commit**

  ```bash
  git add tests/mcp_integration_test.rs
  git commit -m "test: add MCP integration tests with mock server"
  ```

### Task 4.3: Add property-based tests with proptest

**Files:**
- Modify: `Cargo.toml` (add proptest dev-dependency)
- Create: `tests/property_tests.rs`

- [ ] **Step 1: Add proptest to Cargo.toml**

  Add to the `[dev-dependencies]` section:
  ```toml
  proptest = "1"
  ```

- [ ] **Step 2: Create property tests file**

  ```rust
  use proptest::prelude::*;
  use anacleto::config::types::{Config, RetryConfig};
  use anacleto::permissions::types::Permissions;

  /// Test that config parsing never panics on arbitrary YAML strings.
  /// It may return an error (invalid YAML is expected), but must not panic.
  proptest! {
      #![proptest_config = ProptestConfig::with_cases(100)]

      #[test]
      fn config_parsing_doesnt_panic(yaml_string: String) {
          // Limit input size to avoid pathological cases
          if yaml_string.len() > 2000 {
              return Ok(());
          }
          let _result: Result<Config, _> = serde_yaml::from_str(&yaml_string);
          // We don't care if it parses or not — just that it doesn't panic
      }

      #[test]
      fn retry_config_roundtrip(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) {
          let config = RetryConfig {
              max_retries,
              base_delay_ms,
              max_delay_ms,
          };

          // Serialize to YAML
          let yaml = serde_yaml::to_string(&config).unwrap();

          // Deserialize back
          let deserialized: RetryConfig = serde_yaml::from_str(&yaml).unwrap();

          // Verify roundtrip
          assert_eq!(config.max_retries, deserialized.max_retries);
          assert_eq!(config.base_delay_ms, deserialized.base_delay_ms);
          assert_eq!(config.max_delay_ms, deserialized.max_delay_ms);
      }

      #[test]
      fn permissions_parsing_doesnt_panic(permissions_yaml: String) {
          if permissions_yaml.len() > 1000 {
              return Ok(());
          }
          let _result: Result<Permissions, _> = serde_yaml::from_str(&permissions_yaml);
      }
  }
  ```

- [ ] **Step 3: Run the property tests**

  Run: `cargo test property -- --test-threads=1`
  Expected: all property tests pass (100 cases each)

- [ ] **Step 4: Commit**

  ```bash
  git add Cargo.toml Cargo.lock tests/property_tests.rs
  git commit -m "test: add property-based tests with proptest"
  ```

### Task 4.4: Add coverage configuration and CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Add coverage job to CI**

  Add after the `fmt` job in `.github/workflows/ci.yml`:

  ```yaml
    coverage:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - name: Install tarpaulin
          run: cargo install cargo-tarpaulin
        - name: Run coverage
          run: cargo tarpaulin --ignore-tests --out lcov --output-dir ./coverage
        - name: Upload coverage
          uses: actions/upload-artifact@v4
          with:
            name: coverage-report
            path: ./coverage/
  ```

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/ci.yml
  git commit -m "ci: add coverage job with cargo-tarpaulin"
  ```

### Task 4.5: Add concurrency/stress tests

**Files:**
- Create: `tests/concurrency_tests.rs`

- [ ] **Step 1: Write concurrency tests**

  ```rust
  use std::sync::Arc;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use tokio::sync::oneshot;
  use tokio::time::{sleep, Duration};

  /// Test that multiple concurrent tasks can run without deadlocks.
  #[tokio::test]
  async fn test_concurrent_task_spawning() {
      let counter = Arc::new(AtomicUsize::new(0));
      let mut handles = Vec::new();

      for i in 0..50 {
          let counter = counter.clone();
          handles.push(tokio::spawn(async move {
              // Simulate some work
              sleep(Duration::from_millis(10)).await;
              counter.fetch_add(1, Ordering::SeqCst);
              i
          }));
      }

      // Wait for all tasks to complete
      for handle in handles {
          handle.await.unwrap();
      }

      assert_eq!(counter.load(Ordering::SeqCst), 50);
  }

  /// Test that multiple concurrent oneshot channels work correctly.
  #[tokio::test]
  async fn test_concurrent_oneshot_channels() {
      let mut senders = Vec::new();
      let mut receivers = Vec::new();

      for i in 0..100 {
          let (tx, rx) = oneshot::channel();
          senders.push((i, tx));
          receivers.push(rx);
      }

      // Send on all channels concurrently
      let send_handles: Vec<_> = senders
          .into_iter()
          .map(|(i, tx)| {
              tokio::spawn(async move {
                  tx.send(i).unwrap();
              })
          })
          .collect();

      // Receive on all channels concurrently
      let recv_handles: Vec<_> = receivers
          .into_iter()
          .map(|rx| {
              tokio::spawn(async move {
                  rx.await.unwrap()
              })
          })
          .collect();

      // Wait for all sends
      for h in send_handles {
          h.await.unwrap();
      }

      // Wait for all receives and collect results
      let mut results: Vec<i32> = Vec::new();
      for h in recv_handles {
          results.push(h.await.unwrap());
      }

      results.sort();
      let expected: Vec<i32> = (0..100).collect();
      assert_eq!(results, expected);
  }

  /// Test that tokio::spawn can handle many concurrent short-lived tasks.
  #[tokio::test]
  async fn test_many_concurrent_short_tasks() {
      let count = 500;
      let results = Arc::new(AtomicUsize::new(0));
      let mut handles = Vec::with_capacity(count);

      for _ in 0..count {
          let results = results.clone();
          handles.push(tokio::spawn(async move {
              results.fetch_add(1, Ordering::SeqCst);
          }));
      }

      for h in handles {
          h.await.unwrap();
      }

      assert_eq!(results.load(Ordering::SeqCst), count);
  }

  /// Test that tasks with varying sleep durations complete correctly.
  #[tokio::test]
  async fn test_varying_duration_tasks() {
      let mut handles = Vec::new();

      for i in 0..20 {
          handles.push(tokio::spawn(async move {
              let ms = (i as u64) * 5;
              sleep(Duration::from_millis(ms)).await;
              i
          }));
      }

      let mut results: Vec<usize> = Vec::new();
      for h in handles {
          results.push(h.await.unwrap());
      }

      results.sort();
      let expected: Vec<usize> = (0..20).collect();
      assert_eq!(results, expected);
  }
  ```

- [ ] **Step 2: Run the concurrency tests**

  Run: `cargo test test_concurrent -- --test-threads=1`
  Expected: all concurrency tests pass

- [ ] **Step 3: Commit**

  ```bash
  git add tests/concurrency_tests.rs
  git commit -m "test: add concurrency and stress tests"
  ```

---

## Self-Review Checklist

### Spec Coverage

| Requirement | Covered In |
|---|---|
| Phase 1: Fix all 14 clippy warnings | Tasks 1.1–1.11 |
| Phase 1: cargo fmt --all | Task 1.12 |
| Phase 2: README.md | Task 2.1 |
| Phase 2: User guide | Task 2.2 |
| Phase 2: End-to-end example | Task 2.3 |
| Phase 2: Rustdoc on public API | Task 2.4 |
| Phase 3: CI/CD pipeline | Task 3.1 |
| Phase 3: CHANGELOG.md | Task 3.2 |
| Phase 3: Dockerfile + .dockerignore | Task 3.3 |
| Phase 3: rust-toolchain.toml | Task 3.4 |
| Phase 4: Mock MCP server | Task 4.1 |
| Phase 4: MCP integration tests | Task 4.2 |
| Phase 4: Property-based tests (proptest) | Task 4.3 |
| Phase 4: Coverage in CI | Task 4.4 |
| Phase 4: Concurrency/stress tests | Task 4.5 |

### Placeholder Scan

No placeholders (TBD, TODO, "implement later", "fill in details") found. Every step contains complete code or exact commands.

### Type Consistency

- `SpawnAgentConfig` struct defined in Task 1.5, used consistently in Task 1.5 callers
- `SpawnSubagentConfig` struct defined in Task 1.6, used consistently in Task 1.6 callers
- `PendingApprovals` type alias defined in Task 1.4, used in Tasks 1.5 and 1.6
- Mock server tool names (`echo`, `add`, `reverse`) consistent between Task 4.1 and Task 4.2
- `proptest` dev-dependency added in Task 4.3, used in same task

---

## Execution Handoff

Plan complete and saved to `docs/plan-phases-1-4.md`. Two execution options:

1. **Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**