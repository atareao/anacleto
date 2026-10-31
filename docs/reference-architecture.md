# Referencia — Architecture

Texto literal extraído de `AGENTS.md` (sección *Architecture*).

## Architecture

### Module layout

```
src/
  lib.rs               # crate root — re-exports all modules
  main.rs              # entrypoint, CLI arg parsing via clap
  agent/               # agent model, lifecycle, session, tools, context, loader, retry
  config/              # YAML config parsing, global + project merge, path resolution
  db/                  # SQLite persistence via sqlx (session, messages, todos, snapshots, export, usage)
  engine/              # orchestration loop (orchestrator, sessions, commands, events, jobs, apply_patch, template)
  error.rs             # global error types (thiserror)
  hook/                # hook system — configurable shell commands at lifecycle points
  llm/                 # LLM providers (Anthropic, OpenAI, Ollama, Azure, Bedrock, Google, OpenRouter)
  lsp/                 # Language Server Protocol queries (LspClient, result formatting)
  mcp/                 # MCP client (JSON-RPC 2.0 over stdio/TCP, registry, types, parsing)
  plugin/              # plugin system with hooks and custom tool registration
  shell/               # shell command execution + modern CLI tool inventory
  skill/               # skill loading (Anthropic Markdown format), execution
  tools/               # structured agent tools (read, grep, glob, web, lsp, mcp, search_symbol)
  tui/                 # ratatui + crossterm interface (app, events, keys, input, navigation, keymap, render, theme, etc.)
```

### Key design decisions (see docs/adr/ for full ADRs)

| Decision | Choice |
|---|---|
| **Agent model** | Agents and subagents are the same type. Subagents are disposable and cannot nest. Multiple root agents are supported. |
| **Subagent lifecycle** | Disposable: create → work → reply → destroy. No inheritance from parent. When a subagent runs out of steps, it MUST stop, return all results to the parent agent, and explicitly indicate it has run out of steps. |
| **Agent definition** | Each agent is a Markdown file with YAML frontmatter (structural config in frontmatter, system prompt in body). Located in `agents/` directory (global: `~/.config/anacleto/agents/*.md`, project: `.agents/agents/*.md`). |
| **Skills** | Markdown + YAML frontmatter (Anthropic format). Per-agent, no inheritance. Loaded dynamically from the filesystem. |
| **MCPs** | Consumer only (no lifecycle management). Per-agent, no inheritance. JSON-RPC 2.0 over stdio or TCP. |
| **TUI** | ratatui + crossterm, same process as engine (separate Tokio tasks). Window navigation with `Alt+1`..`Alt+5`. Vim-style bindings in panels. |
| **Config** | YAML. Global (`~/.config/anacleto/`) + project (`.agents/`) merged. Two layers with project overrides. |
| **Persistence** | SQLite via sqlx. Sessions are resumable across restarts. Context limit: 50% of model window. Fork/import/export and snapshot/rollback. |
| **Permissions** | Allow by default, deny explicitly. Human approval for sensitive ops. Rules per agent: `fs.*`, `net.http`, `command.run`, `mcp.use`, `env.read`, `skill.use`. |
| **Streaming** | Always on by default. Intermediate steps (skill/MCP execution) visible in TUI. |
| **Error handling** | Retries configurable for LLM, MCP, subagent timeouts. Exponential backoff with jitter. Session is recoverable. |
| **Plugins** | Declarative plugin system with hooks (`on_agent_spawn`, `on_tool_call`, `on_command`, `on_event`) and custom tool registration. Loaded from `~/.config/anacleto/plugins/`. |
| **Hooks** | Configurable shell commands at lifecycle points (before/after tool, apply_patch, shell, fs_write, on_startup, on_shutdown) with template variable substitution. |

### Agent definition (Markdown + YAML frontmatter)

Agents are **not** defined in `config.yaml`. Each agent is a self-contained Markdown file:

```markdown
---
name: root
description: Senior engineering agent
role: root
model: "claude-sonnet-4"
skills:
  - .agents/skills/shell/
  - .agents/skills/code-review/
mcps: [filesystem]
permissions:
  deny:
    - "command.run.sudo"
subagents:
  - reviewer
  - writer
max_steps: 90
---

You are **Anacleto**, a senior engineering agent...
```

Supported frontmatter fields:

| Field | Description |
|---|---|
| `name` | Unique agent name |
| `description` | Short human-readable summary |
| `role` | `root` or `subagent` (default `subagent`) |
| `model` | Model name resolved to a provider |
| `skills` | Skill paths (relative or absolute) |
| `mcps` | MCP server names (references global MCP config) |
| `permissions` | `allow`/`deny` list of permission rules |
| `subagents` | Subagent names (roots only, references by name) |
| `max_steps` | Maximum LLM+tool turns per task |

Resolution order:
1. Project agents: `.agents/agents/*.md`
2. Global agents: `~/.config/anacleto/agents/*.md`
3. Project overrides global when same name

Exactly one agent must declare `role: root`.

> **Important**: Subagents are fully independent — they do **not** inherit skills, MCPs, permissions, or any other config from their parent.
