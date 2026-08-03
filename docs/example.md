# End-to-End Example: Using Anacleto

This example walks through setting up a new project with Anacleto, adding skills, running it, and working with subagents and sessions.

---

## 1. Setup

Create a project directory with a `.anacleto/config.yaml`:

```bash
mkdir -p my-project/.anacleto/agents
mkdir -p my-project/.anacleto/skills
cd my-project
```

Create `.anacleto/config.yaml`:

```yaml
# .anacleto/config.yaml — Project-level configuration
# Merges on top of ~/.config/anacleto/config.yaml

models:
  anthropic:
    api_key: "${ANTHROPIC_API_KEY}"
    model: "claude-sonnet-4-20250514"
    context_window: 200000

mcps:
  filesystem:
    transport: stdio
    command: "/usr/local/bin/mcp-filesystem"
    args:
      - "--allowed-dirs"
      - "/home/user/my-project"

  fetch:
    transport: stdio
    command: "/usr/local/bin/mcp-fetch"
    args: []

session:
  history_limit_percent: 50

agents:
  - name: root
    description: ".anacleto/agents/root.md"
    model: "claude-sonnet-4"
    skills:
      - ".anacleto/skills/shell/"
      - ".anacleto/skills/web-research/"
    mcps:
      - filesystem
      - fetch
    permissions:
      deny:
        - "command.run.sudo"
    subagents:
      - reviewer
      - writer

  - name: reviewer
    description: ".anacleto/agents/reviewer.md"
    model: "claude-sonnet-4"
    skills:
      - ".anacleto/skills/code-review/"
    mcps:
      - filesystem
    permissions:
      deny:
        - "command.run"
        - "net.http"
    subagents: []

  - name: writer
    description: ".anacleto/agents/writer.md"
    model: "claude-sonnet-4"
    skills:
      - ".anacleto/skills/web-research/"
    mcps: []
    permissions:
      deny:
        - "command.run"
        - "filesystem.write"
    subagents: []
```

Create the root agent description at `.anacleto/agents/root.md`:

```markdown
You are **Anacleto**, a senior engineering agent specialized in software
architecture, code generation, and system design.

## Capabilities

You have access to the following skills:

1. **shell** — Execute shell commands and scripts in the workspace.
2. **web-research** — Search the web and fetch documentation.

You can delegate tasks to your subagents:

- **reviewer** — Code review specialist.
- **writer** — Technical writing specialist.

## Workflow

1. **Understand** — Clarify requirements if needed.
2. **Plan** — Break the task into steps.
3. **Execute** — Use skills or delegate to subagents.
4. **Review** — Verify the output meets requirements.
5. **Report** — Summarize what was done.
```

---

## 2. Add a skill

Create `.anacleto/skills/web-research/search.md`:

```markdown
---
name: web-search
description: Search the web using a search engine and return results
metadata:
  version: "1.0"
  category: research
  risk: low
---

# Web Search skill

Perform web searches and return relevant results. Use this for:

- Finding documentation, tutorials, and guides
- Researching technical solutions
- Looking up current information

## Usage

Provide a `query` describing what to search for and the `num_results`
(number of results to return, default 5).

### Example

```yaml
query: "Rust async/await best practices 2026"
num_results: 5
```

## Output

A list of search results with titles, URLs, and brief excerpts.
```

---

## 3. Run Anacleto

Start Anacleto from the project directory:

```bash
anacleto
```

The TUI starts. You will see:

```
┌─────────────────────────────────────────────────────────────┐
│ Anacleto | default:-/0 | agents: 1 | subagents: 2           │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Anacleto started.                                          │
│  Agent 'root' created.                                      │
│                                                             │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ Input                                                       │
│ > _                                                         │
└─────────────────────────────────────────────────────────────┘
```

The status bar shows:

- **Session**: `default` (no active session ID yet)
- **Agents**: 1 root agent loaded
- **Subagents**: 2 (reviewer, writer) configured and ready

Press `/agents` to see the agent details:

```
┌─────────────────────────────────────────────────────────────┐
│ Agents (Esc to close)                                       │
│ ─── Root Agents ───                                         │
│  IDLE  root [claude-sonnet-4]  skills: shell, web-research  │
│        mcps: filesystem, fetch  children: 2                 │
│ ─── SubAgents ───                                           │
│  IDLE  reviewer [claude-sonnet-4]  skills: code-review     │
│        mcps: filesystem                                     │
│  IDLE  writer [claude-sonnet-4]     skills: web-research   │
│        mcps: none                                           │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Interact

Type a message and press Enter:

```
Research the latest Rust edition features and document them
```

Anacleto processes the request. You will see live streaming output:

```
> Research the latest Rust edition features and document them
▌I'll research this using my web-research skill and then...
```

The TUI shows intermediate steps — the agent using skills and MCP tools:

```
> Research the latest Rust edition features and document them
[Agent 'root' using skill 'web-research']
  Fetching: https://blog.rust-lang.org/category/releases
[Agent 'root' using MCP 'filesystem_read']
  Reading: /home/user/my-project/README.md
▌Based on my research, the latest Rust edition 2024 includes...
```

When the agent is done, the full response appears:

```
> Research the latest Rust edition features and document them
Based on my research, here are the key Rust 2024 edition features:

- `impl Trait` in argument position is now `use<..>`-aware
- `unsafe` blocks are required on `static mut` accesses
- `gen` is a reserved keyword
- `cargo fix --edition` handles migration automatically

Would you like me to write this to a file or delegate to the writer subagent?
```

---

## 5. Use subagents

When the root agent needs specialized work (e.g., code review or technical writing), it delegates to a subagent. Let's ask it to write documentation:

```
Write a README.md for this project based on what you found
```

The root agent delegates to the **writer** subagent:

```
> Write a README.md for this project based on what you found
Subagent 'writer' created.
Writer is working on the README...
Subagent 'writer' completed.
```

During delegation, the status bar updates to show the subagent is busy:

```
 Anacleto | default:-/2 | agents: 1 | subagents: 2
```

Press `/subagents` to see the tree view:

```
┌─ Subagent Tree (Esc to close) ─────────────────────────────┐
│ ┌─  IDLE  root [claude-sonnet-4]                            │
│ │  └──  DONE  writer                                        │
│ └─                                                          │
└─────────────────────────────────────────────────────────────┘
```

The writer subagent returned its result to the root agent, which can then present it to you or request a review:

```
> Write a README.md for this project based on what you found
Subagent 'writer' created.
Writer is working on the README...
Subagent 'writer' completed.

Here's the README I'd like to write. Let me have the reviewer check it first.

Subagent 'reviewer' created.
Reviewer is reviewing the draft...
Subagent 'reviewer' completed.

Both the writer and reviewer have completed their work.
Here's the final README.md content:

# My Project

...
```

---

## 6. Sessions

Anacleto automatically saves your conversation to the current session. Use session commands to manage them.

### List sessions

```
/sessions
```

```
> /sessions
  abc12345  msgs:12  default  2026-08-02 14:30  ◀
  def67890  msgs:5   research  2026-08-01 10:15
```

The `◀` marker indicates the active session.

### Resume a session

```
/resume def67890
```

```
> /resume def67890
Switched to session: research
```

The status bar updates to show the resumed session name and ID.

### Rename a session

```
/rename abc12345 my-feature
```

```
> /rename abc12345 my-feature
Session renamed to: my-feature
```

### Delete a session

```
/delete def67890
```

```
> /delete def67890
Session def67890 deleted.
```

---

## Docker + Ollama example

Run Anacleto with a local Ollama instance for fully offline operation:

```yaml
version: '3'
services:
  ollama:
    image: ollama/ollama
    ports:
      - "11434:11434"
    volumes:
      - ollama_data:/root/.ollama

  anacleto:
    build: .
    depends_on:
      - ollama
    environment:
      - OLLAMA_BASE_URL=http://ollama:11434
      # Optional: set other API keys if you want hybrid cloud/local
      # - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
      # - OPENAI_API_KEY=${OPENAI_API_KEY}

volumes:
  ollama_data:
```

With an `.anacleto/config.yaml` pointing to Ollama:

```yaml
models:
  ollama:
    base_url: "http://ollama:11434"
    model: "llama3.2"
    context_window: 8192

# No cloud API keys needed!
```

Start it:

```bash
export ANTHROPIC_API_KEY=sk-...  # Optional, only if using Claude
docker compose up
```

---

## Summary

This example showed:

1. **Setup** — Created a project with `.anacleto/config.yaml` and agent descriptions.
2. **Skills** — Added a custom `web-search` skill in the Anthropic Markdown format.
3. **Running** — Started the TUI, saw agent initialization and status indicators.
4. **Interacting** — Sent a message, watched the agent use skills and MCP tools with live streaming output.
5. **Subagents** — Saw the root agent delegate work to reviewer and writer subagents.
6. **Sessions** — Listed, resumed, renamed, and deleted sessions.
