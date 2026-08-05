# Anacleto User Guide

Anacleto is an agent orchestration engine in Rust. It manages a tree of agents and subagents with clean separation of skills, MCP servers, and permissions. The sole interface is a TUI built with [ratatui](https://github.com/ratatui-org/ratatui) + [crossterm](https://github.com/crossterm-rs/crossterm).

---

## Table of Contents

- [Configuration](#configuration)
- [Skills](#skills)
- [MCPs (Model Context Protocol)](#mcps-model-context-protocol)
- [Agents and Subagents](#agents-and-subagents)
- [Sessions](#sessions)
- [Permissions](#permissions)
- [TUI Commands](#tui-commands)

---

## Configuration

Anacleto uses a two-layer YAML configuration system: a **global** config for machine-wide defaults and a **project** config for per-project overrides. The project config merges on top of the global config, so you can keep API keys and shared MCP servers in the global file while defining project-specific agents and skills locally.

### Config locations

| Level | Path |
|---|---|
| Global | `~/.config/anacleto/config.yaml` |
| Project | `.anacleto/config.yaml` (project root) |

### YAML schema

```yaml
# ── LLM model definitions ──────────────────────────────────────────
models:
  anthropic:
    api_key: "${ANTHROPIC_API_KEY}"    # Resolved from env var at runtime
    model: "claude-sonnet-4-20250514"
    context_window: 200000

  openai:
    api_key: "${OPENAI_API_KEY}"
    model: "gpt-4o"
    context_window: 128000

  openrouter:
    api_key: "${OPENROUTER_API_KEY}"
    model: "openai/gpt-4o"
    context_window: 128000
    base_url: "https://openrouter.ai/api/v1"

  ollama:
    base_url: "http://localhost:11434"
    model: "llama3.2"
    context_window: 8192

# ── MCP server definitions ─────────────────────────────────────────
mcps:
  filesystem:
    transport: stdio
    command: "/usr/local/bin/mcp-filesystem"
    args:
      - "--allowed-dirs"
      - "/home/user/projects"

  postgres:
    transport: tcp
    host: "localhost"
    port: 5432

# ── Session configuration ──────────────────────────────────────────
session:
  history_limit_percent: 50
  database_path: "~/.local/share/anacleto/sessions.db"
  retry:
    max_retries: 3
    base_delay_ms: 1000
    max_delay_ms: 30000

# ── Agent definitions ──────────────────────────────────────────────
agents:
  - name: root
    description: ".anacleto/agents/root.md"
    model: "claude-sonnet-4"
    skills:
      - ".anacleto/skills/shell/"
    mcps:
      - filesystem
    permissions:
      deny:
        - "command.run.sudo"
    subagents:
      - reviewer
```

See the [example global config](example-global-config.yaml) for a complete reference.

### Provider resolution

An agent's `model` field determines which provider serves it. The engine uses prefix matching:

| Model name pattern | Provider |
|---|---|
| Starts with `claude` | Anthropic |
| Starts with `gpt`/`o1`/`o3` | OpenAI |
| Contains `/` | OpenRouter (OpenAI-compatible) |
| Anything else | Ollama |

### Environment variable resolution

Any value in the config can reference an environment variable using the `${VAR_NAME}` syntax. Variables are resolved at runtime when the config is loaded. This is the recommended way to handle API keys:

```yaml
models:
  anthropic:
    api_key: "${ANTHROPIC_API_KEY}"
```

### Config merge rules

1. Load global config from `~/.config/anacleto/config.yaml`.
2. Load project config from `.anacleto/config.yaml` (if it exists).
3. Merge project config on top of global config (project values override globals).
4. Resolve `${VAR}` environment variable references.
5. CLI flags (`--config`, `--database`) override both levels.

### CLI flags

```
Usage: anacleto [OPTIONS]

Options:
  -c, --config <PATH>      Path to a project config file (overrides auto-detection)
  -d, --database <PATH>    Database path (overrides config)
  -v, --verbose            Enable verbose logging
  -h, --help               Print help
  -V, --version            Print version
```

---

## Skills

Skills are specialized capabilities that agents can invoke as tools. They are defined as Markdown files with YAML frontmatter, following the [Anthropic Skill Format](https://docs.anthropic.com/en/docs/agents-and-tools/skills).

### Format

```markdown
---
name: shell
description: Execute shell commands in the workspace environment
metadata:
  version: "1.0"
  category: system
  risk: high
---

# Shell skill

Execute shell commands and scripts within the current workspace environment.

## Usage

Provide a `task` describing exactly what shell commands to run.

### Example

```yaml
task: "Run the test suite: cargo test"
```

## Output

The raw stdout and stderr from the command execution.
```

### Loading

- Skills are loaded from directories specified in an agent's `skills` field.
- Each directory is scanned for `*.md` files.
- The frontmatter `name` and `description` fields are used for discovery.
- The Markdown body provides instructions and examples loaded on invocation.

### Per-agent, no inheritance

Skills are configured **per agent** and are **not inherited** by subagents. Each agent (and subagent) specifies its own list of skill directories. This means:

- The root agent might have `shell` and `web-research` skills.
- A `reviewer` subagent would only have `code-review` skills.
- A `writer` subagent would only have `web-research` skills.

### Skill files available

| File | Description |
|---|---|
| `.anacleto/skills/shell/skill.md` | Execute shell commands |
| `.anacleto/skills/web-research/skill.md` | Fetch and analyze web content |
| `.anacleto/skills/code-review/skill.md` | Review code for quality and correctness |

---

## MCPs (Model Context Protocol)

Anacleto integrates with external tools and services through the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/), a JSON-RPC 2.0 protocol.

### Consumer only

Anacleto is an MCP **consumer** only — it connects to MCP servers but does not manage their lifecycle. Starting, stopping, and updating MCP servers is handled externally (e.g., via systemd, Docker, or the command line).

### Transports

Two transport methods are supported:

**stdio** — The MCP server is spawned as a child process and communicates over stdin/stdout.

```yaml
mcps:
  filesystem:
    transport: stdio
    command: "/usr/local/bin/mcp-filesystem"
    args:
      - "--allowed-dirs"
      - "/home/user/projects"
```

**tcp** — The MCP server runs as a network service accessible at a host:port.

```yaml
mcps:
  postgres:
    transport: tcp
    host: "localhost"
    port: 5432
```

### Per-agent, no inheritance

Like skills, MCP servers are configured per agent and are not inherited by subagents. Each agent specifies its own list of MCP references.

### Tool naming

When an LLM calls an MCP tool, the tool name is prefixed with the server name and an underscore:

```
{server_name}_{tool_name}
```

For example, a `read_file` tool exposed by the `filesystem` MCP server would be invoked as `filesystem_read_file`. This prevents naming conflicts when multiple MCP servers expose tools with the same name.

---

## Plugins

Plugins extend the engine with hooks and transforms. They are loaded
**declaratively** from the global plugins directory `~/.config/anacleto/plugins/`.
Each plugin is a subdirectory containing a `plugin.yaml` manifest:

```yaml
# ~/.config/anacleto/plugins/myplugin/plugin.yaml
name: myplugin
description: Example plugin
version: 1.0.0
```

Plugins can hook into the engine lifecycle:

- `on_agent_spawn` — transform an agent's system prompt before it is sent.
- `on_tool_call` — intercept a tool call and return a replacement result.
- `on_command` — handle a custom slash command.
- `on_event` — react to engine events.
- `register_tool` — register a custom tool and its handler, available to all
  spawned agents at runtime.

Plugins are trusted code from your own configuration. They are loaded once at
engine startup and shared (read-only) across all agents.

---

## Agents and Subagents

Agents and subagents are the **same type** under the hood. The only difference is how they are invoked:

- **Agents** are listed in the config's `agents` array and are user-invocable. The first agent in the list is the **root** agent — the one that receives user input directly from the TUI.
- **Subagents** are referenced by an agent's `subagents` field. They cannot be invoked directly by the user.

### Lifecycle

**Agent lifecycle** — Persistent. Created at startup, lives for the duration of the session.

**Subagent lifecycle** — Disposable. The pattern is:

1. **Create** — Parent agent decides to delegate work and spawns a subagent.
2. **Work** — Subagent receives the task, works independently (with its own skills, MCPs, and model).
3. **Reply** — Subagent returns results to the parent agent.
4. **Destroy** — Subagent is cleaned up.

### Rules

| Rule | Detail |
|---|---|
| **No nesting** | Subagents cannot have subagents (`subagents: []`). |
| **No inheritance** | Subagents are fully independent — they get no skills, MCPs, or permissions from their parent. |
| **Disposable** | Each subagent invocation creates a fresh instance. |

### Agent information in the TUI

- Press `/agents` (`/a`) to see all agents and subagents with their status, model, skills, and MCPs.
- Press `/subagents` (`/sa`) to see the agent hierarchy tree.

Each agent shows a status badge:

| Badge | Meaning |
|---|---|
| `IDLE` | Waiting for work |
| `BUSY` | Currently working |
| `WAIT` | Waiting for a subagent to complete |
| `DONE` | Completed |
| `ERR` | Encountered an error |

---

## Sessions

Sessions represent complete conversations with Anacleto. They are persisted to SQLite and support resumability.

### Persistence

- Sessions are stored in a SQLite database.
- Default path: `~/.local/share/anacleto/sessions.db` (Linux) or `~/Library/Application Support/anacleto/sessions.db` (macOS).
- The path can be overridden in config or via the `--database` CLI flag.
- Each session contains per-agent message history.

### Context limit

To prevent exceeding the LLM's context window, Anacleto limits the conversation history to a configurable percentage of the model's context window. The default is **50%**. The remaining space is reserved for system prompts, tool results, and new generation.

Configure in the `session` section:

```yaml
session:
  history_limit_percent: 50
```

### Session commands

See [TUI Commands](#tui-commands) for the full list of session-related slash commands.

---

## Permissions

Anacleto uses a simple permission model: **allow by default, deny explicitly**. Permissions control what operations an agent or subagent can perform.

### Permission types

| Permission | Description |
|---|---|
| `command.run` | Execute arbitrary shell commands |
| `command.run.sudo` | Execute commands with sudo |
| `fs.read` | Read files and directories |
| `fs.write` | Write, create, or delete files |
| `net.http` | Make HTTP requests |
| `env.read` | Read environment variables |
| `skill.use` | Invoke a skill tool |

### Configuration

```yaml
agents:
  - name: reviewer
    permissions:
      deny:
        - "command.run"    # Cannot run shell commands
        - "net.http"       # Cannot make network requests
```

The `allow` list can be used to restrict to specific operations:

```yaml
agents:
  - name: writer
    permissions:
      allow:
        - "skill.use"      # Can only use skills
      deny:
        - "command.run"
        - "filesystem.write"
```

### Human-in-the-loop approval

For sensitive operations, Anacleto can request human approval before proceeding. When an approval dialog appears in the TUI:

- Press **Y** to approve the operation.
- Press **N** to deny the operation.

The approval dialog shows the operation being requested so you can make an informed decision.

---

## TUI Commands

All commands are entered by typing in the input panel and pressing Enter.

### Slash commands

| Command | Alias | Description |
|---|---|---|
| `/help` | `/h` | Show available commands |
| `/resume <id>` | `/r` | Resume a session by ID |
| `/delete <id>` | `/d` | Delete a session by ID |
| `/rename <id> <name>` | — | Rename a session |
| `/new <name>` | — | Create a new session |
| `/sessions` | `/s` | List all sessions |
| `/agents` | `/a` | Show agent list overlay |
| `/subagents` | `/sa` | Show subagent tree overlay |

### Custom slash commands

Custom slash commands are defined in config under the `commands` key. Each
command has a `name` (including the leading `/`), an optional `description`
shown in the command palette, and a `template` that is expanded and sent to the
engine as user input.

```yaml
commands:
  - name: /deploy
    description: Deploy the current branch
    template: "Deploy branch {env:BRANCH} to production"
  - name: /summarize
    description: Summarize the README
    template: "Summarize this file: {file:README.md}"
```

Templates support two placeholders:

- `{env:VAR}` — replaced by the value of the environment variable `VAR`. If the
  variable is unset, the placeholder is left literal.
- `{file:path}` — replaced by the contents of the file at `path` (relative to
  the current working directory). If the file cannot be read, the placeholder
  is left literal.

Arguments typed after the command are appended to the expanded template. Custom
commands are dispatched before built-ins, so a custom command shadows a built-in
with the same name.

### Other controls

| Key | Action |
|---|---|
| `Y` / `y` | Approve a pending permission request |
| `N` / `n` | Deny a pending permission request |
| `Esc` | Close overlay panels or exit |
| `Ctrl+C` | Shutdown Anacleto |

### Status bar

The top status bar displays:

```
 Anacleto | session-name:id/active-count | agents: N | subagents: N    skills: N | mcps: N
```

- **session-name** — Current session name.
- **id** — Truncated session ID.
- **active-count** — Number of agents currently working.
- **agents** — Total root agents.
- **subagents** — Total subagents.
- **skills** — Total skills loaded across root agents.
- **mcps** — Total MCP servers connected.
