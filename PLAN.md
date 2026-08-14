# Configurable Tools System Implementation Plan

> **For agentic workers:** Tasks are implemented sequentially. Each task produces independently testable changes.

**Goal:** Move built-in tools from hardcoded-in-Rust to per-agent-declared-in-config, with defaults in config.yaml.

**Architecture:** Add `ToolDefaults` to `Config` (config.yaml), change `spawn_agent()` to only include tools the agent declares, merge defaults from config with overrides from agent frontmatter.

**Tech Stack:** Rust (serde YAML), YAML frontmatter in agent Markdown files.

## Global Constraints

- Everything in English (code, comments, docs, config)
- No backwards compatibility — breaking change
- If a tool is not in the agent's `tools:` list, the agent doesn't have it
- No tool is core — even `task`, `question`, `todo` must be declared
- JSON Schema stays in Rust — only display properties go in YAML
- `cargo fmt --check && cargo clippy && cargo test` must pass

---

### Task 1: Add `ToolDefaults` to `Config` and `builtin_tool_definitions()` registry

**Files:**
- Modify: `src/config/types.rs` — add `ToolDefaults` struct and `tools` field to `Config`
- Modify: `src/agent/lifecycle.rs` — add `builtin_tool_definitions()` function

**Interfaces:**
- Produces: `ToolDefaults` struct with `description`, `show`, `display`, `color` fields
- Produces: `Config.tools: HashMap<String, ToolDefaults>` field
- Produces: `builtin_tool_definitions() -> HashMap<String, ToolDefinition>` function

- [ ] **Step 1: Add `ToolDefaults` to `src/config/types.rs`**

Add after the `ToolSettings` struct:

```rust
/// Default values for a built-in tool's display properties, defined in config.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefaults {
    /// Description sent to the LLM (overrides the hardcoded one).
    #[serde(default)]
    pub description: String,
    /// Whether executions are shown in the chat (default: true).
    #[serde(default = "default_tool_show")]
    pub show: bool,
    /// Custom display template with `{param}` placeholders (optional).
    pub display: Option<String>,
    /// Custom color for the tool execution line in the TUI (optional).
    pub color: Option<String>,
}
```

- [ ] **Step 2: Add `tools` field to `Config` struct**

Add to `Config` in `src/config/types.rs`:

```rust
    /// Default tool definitions and display properties.
    /// Each key is a built-in tool name, value is its default display config.
    #[serde(default)]
    pub tools: HashMap<String, ToolDefaults>,
```

- [ ] **Step 3: Add `builtin_tool_definitions()` to `src/agent/lifecycle.rs`**

Add a function that returns all built-in tool definitions keyed by name:

```rust
/// Returns all built-in tool definitions keyed by tool name.
pub fn builtin_tool_definitions() -> HashMap<String, ToolDefinition> {
    let mut map = HashMap::new();
    for def in [
        todo_tool_definition(),
        question_tool_definition(),
        apply_patch_tool_definition(),
        read_tool_definition(),
        grep_tool_definition(),
        glob_tool_definition(),
        webfetch_tool_definition(),
        websearch_tool_definition(),
        mcp_list_resources_tool_definition(),
        mcp_read_resource_tool_definition(),
        mcp_list_resource_templates_tool_definition(),
        lsp_query_tool_definition(),
        task_tool_definition(),
    ] {
        map.insert(def.name.clone(), def);
    }
    map
}
```

- [ ] **Step 4: Build to verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Compiles successfully (new types are unused but valid).

- [ ] **Step 5: Commit**

```bash
git add src/config/types.rs src/agent/lifecycle.rs
git commit -m "feat: add ToolDefaults config and builtin_tool_definitions registry"
```

---

### Task 2: Change `spawn_agent()` to use per-agent tool declarations

**Files:**
- Modify: `src/agent/lifecycle.rs` — change tool assembly logic in `spawn_agent()`

**Interfaces:**
- Consumes: `agent.tool_settings: HashMap<String, ToolSettings>` (keys = enabled tools)
- Consumes: `config.tools: HashMap<String, ToolDefaults>` (defaults from config.yaml)
- Consumes: `builtin_tool_definitions()` (tool schemas from Rust)

- [ ] **Step 1: Replace hardcoded tool list with filtered + merged logic**

In `spawn_agent()`, replace lines 193-205 (the 13 `tools.push(...)` calls) with:

```rust
    // Add built-in tools based on agent's tool declarations.
    // Only tools listed in the agent's `tools:` frontmatter are included.
    let builtin_tools = builtin_tool_definitions();
    for (tool_name, agent_settings) in &agent.tool_settings {
        if let Some(mut def) = builtin_tools.get(tool_name).cloned() {
            // Merge defaults from config.yaml
            if let Some(defaults) = config.tools.get(tool_name) {
                if !defaults.description.is_empty() {
                    def.description = defaults.description.clone();
                }
            }
            // Apply agent-level overrides (ToolSettings has enabled/show/display/color)
            // These are display-only; the schema stays from Rust
            tools.push(def);
        }
    }
```

- [ ] **Step 2: Build to verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add src/agent/lifecycle.rs
git commit -m "feat: spawn_agent filters tools by agent declaration"
```

---

### Task 3: Update `~/.config/anacleto/config.yaml` with tool defaults

**Files:**
- Modify: `~/.config/anacleto/config.yaml`

- [ ] **Step 1: Add `tools:` section with all built-in tool defaults**

```yaml
# ---------------------------------------------------------------------------
# Built-in tool defaults
# ---------------------------------------------------------------------------
# Each agent declares which tools it uses in its frontmatter `tools:` field.
# This section defines default display properties for all built-in tools.
# Agents can override these in their frontmatter.

tools:
  read:
    description: "Read a file from the filesystem. Use for viewing file contents, logs, or any text file."
    show: true
    color: cyan
  grep:
    description: "Search file contents using regular expressions. Use for finding patterns in code or text files."
    show: true
    color: blue
  glob:
    description: "Find files by glob pattern. Use for locating files by name pattern."
    show: true
    color: blue
  bash:
    description: "Execute shell commands in the workspace environment."
    show: true
    color: green
    display: "$ {command}"
  webfetch:
    description: "Fetch content from a URL and return it as markdown or text."
    show: true
    color: green
    display: "🌐 {url}"
  websearch:
    description: "Search the web using SearXNG meta-search engine."
    show: true
    color: green
    display: "🔍 {query}"
  todo:
    description: "Create and maintain a structured task list for the current session."
    show: true
    color: magenta
    display: "📝 {action}"
  question:
    description: "Ask the user a question and wait for their response."
    show: true
    color: yellow
  compress:
    description: "Compress conversation history into a detailed technical summary."
    show: true
    color: yellow
  task:
    description: "Launch a subagent to handle a complex multi-step task autonomously."
    show: true
    color: magenta
    display: "⚡ {description}"
  skill:
    description: "Load and execute a specialized skill for a specific task."
    show: true
    color: cyan
    display: "🎯 {name}"
  apply_patch:
    description: "Apply a patch to modify files in the workspace."
    show: true
    color: green
  mcp_list_resources:
    description: "List available resources from connected MCP servers."
    show: true
    color: cyan
  mcp_read_resource:
    description: "Read a specific resource from an MCP server."
    show: true
    color: cyan
  mcp_list_resource_templates:
    description: "List resource templates from connected MCP servers."
    show: true
    color: cyan
  lsp_query:
    description: "Query the LSP server for code intelligence (completions, diagnostics, etc.)."
    show: true
    color: cyan
```

- [ ] **Step 2: Commit**

```bash
git add ~/.config/anacleto/config.yaml
git commit -m "feat: add built-in tool defaults to global config"
```

---

### Task 4: Update all agent files with explicit tool lists

**Files:**
- Modify: All 23 files in `~/.config/anacleto/agents/`

Each agent needs a `tools:` section listing only the tools it should have access to. Below are the tool lists per agent based on their role:

**root.md** — Full engineering agent: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `webfetch`, `todo`, `question`, `compress`, `task`, `skill`

**chat.md** — Conversational agent: `todo`, `question`, `read` (show:false), `grep` (show:false), `glob` (show:false), `webfetch`

**reviewer.md** — Code review: `codegraph_*`, `read`, `grep`, `glob`, `question`, `compress`

**writer.md** — Technical writing: `read`, `webfetch`, `question`, `compress`

**chronicler.md** — Project logger: `read`, `bash`, `question`, `compress`

**rust-dev.md** — Rust development: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `question`, `compress`

**tech-writer.md** — Article writing: `read`, `webfetch`, `question`, `compress`

**python-dev.md** — Python development: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `question`, `compress`

**dev-manager.md** — Development manager: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `webfetch`, `todo`, `question`, `compress`, `task`, `skill`

**agent-manager.md** — Agent/skill manager: `read`, `grep`, `glob`, `bash`, `question`, `task`, `skill`

**planner.md** — Planning specialist: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `question`, `compress`

**podcast-manager.md** — Podcast production: `read`, `bash`, `question`, `compress`, `task`

**executor.md** — Simple executor: `read`, `bash`, `question`, `compress`

**frontend-dev.md** — Frontend development: `codegraph_*`, `read`, `grep`, `glob`, `bash`, `question`, `compress`

**git-controller.md** — Git operations: `read`, `bash`, `question`, `compress`

**researcher.md** — Research: `webfetch`, `question`, `compress`

**script-writer.md** — Script writing: `read`, `webfetch`, `question`, `compress`

**script-verifier.md** — Script verification: `read`, `webfetch`, `question`, `compress`

**tech-researcher.md** — Technical research: `codegraph_*`, `read`, `webfetch`, `question`, `compress`

**article-writer.md** — Article writing: `read`, `webfetch`, `question`, `compress`

**verifier.md** — Article verification: `read`, `webfetch`, `question`, `compress`

**writer-manager.md** — Writing coordination: `read`, `bash`, `question`, `compress`, `task`

- [ ] **Step 1: Update root.md**

Replace the `tools:` section with explicit tool list including all codegraph tools and built-in tools.

- [ ] **Step 2: Update chat.md**

Replace `tools:` with: `todo`, `question`, `read` (show:false), `grep` (show:false), `glob` (show:false), `webfetch`

- [ ] **Step 3: Update reviewer.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `question`, `compress`

- [ ] **Step 4: Update writer.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 5: Update chronicler.md**

Replace `tools:` with `read`, `bash`, `question`, `compress`

- [ ] **Step 6: Update rust-dev.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `bash`, `question`, `compress`

- [ ] **Step 7: Update tech-writer.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 8: Update python-dev.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `bash`, `question`, `compress`

- [ ] **Step 9: Update dev-manager.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `bash`, `webfetch`, `todo`, `question`, `compress`, `task`, `skill`

- [ ] **Step 10: Update agent-manager.md**

Replace `tools:` with `read`, `grep`, `glob`, `bash`, `question`, `task`, `skill`

- [ ] **Step 11: Update planner.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `bash`, `question`, `compress`

- [ ] **Step 12: Update podcast-manager.md**

Replace `tools:` with `read`, `bash`, `question`, `compress`, `task`

- [ ] **Step 13: Update executor.md**

Replace `tools:` with `read`, `bash`, `question`, `compress`

- [ ] **Step 14: Update frontend-dev.md**

Replace `tools:` with codegraph tools + `read`, `grep`, `glob`, `bash`, `question`, `compress`

- [ ] **Step 15: Update git-controller.md**

Replace `tools:` with `read`, `bash`, `question`, `compress`

- [ ] **Step 16: Update researcher.md**

Replace `tools:` with `webfetch`, `question`, `compress`

- [ ] **Step 17: Update script-writer.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 18: Update script-verifier.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 19: Update tech-researcher.md**

Replace `tools:` with codegraph tools + `read`, `webfetch`, `question`, `compress`

- [ ] **Step 20: Update article-writer.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 21: Update verifier.md**

Replace `tools:` with `read`, `webfetch`, `question`, `compress`

- [ ] **Step 22: Update writer-manager.md**

Replace `tools:` with `read`, `bash`, `question`, `compress`, `task`

- [ ] **Step 23: Commit**

```bash
git add ~/.config/anacleto/agents/
git commit -m "feat: add explicit tool declarations to all agents"
```

---

### Task 5: Build and verify

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: Compiles with no errors.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy 2>&1`
Expected: No warnings or errors.

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 4: Run fmt check**

Run: `cargo fmt --check 2>&1`
Expected: No formatting issues.

---

### Task 6: Document the new system

**Files:**
- Modify: `AGENTS.md` or create `docs/tools-configuration.md`

- [ ] **Step 1: Add documentation explaining the new tools system**

Document:
1. How tools are declared in agent frontmatter
2. How defaults work in config.yaml
3. How overrides work
4. The list of all built-in tools
5. Migration guide (what changed)