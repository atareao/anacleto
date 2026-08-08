---
name: agent-creator
description: Create, modify, and manage Anacleto agents and subagents (Markdown files with YAML frontmatter in .anacleto/agents/). Use when the user asks to create a new agent or subagent, edit an existing agent definition, change an agent's role, model, skills, mcps, permissions, or subagents, add or remove a subagent to/from an agent, or refactor an agent's system prompt. This skill generates and edits the agent .md files only — it does not edit config.yaml or manage MCP server definitions.
metadata:
  domain: anacleto-orchestration
---

# Agent Creator

A skill for creating, editing, and managing **Anacleto** agents and subagents.

In Anacleto, agents and subagents are the **same type**, defined solely as Markdown files with YAML frontmatter living in `~/.config/anacleto/agents/` (global) and `<project_root>/.anacleto/agents/` (project). The Markdown body after the frontmatter becomes the agent's **system prompt**. There is no compiled logic — the files `src/agent/loader.rs` (`parse_agent`) and `src/config/types.rs` (`AgentConfig`, `PermissionConfig`) read them directly.

## Scope (important)

- ✅ **In scope**: create, edit, and review agent `.md` files; add/remove subagents from an agent's `subagents:` list.
- ❌ **Out of scope**: editing `config.yaml`, managing MCP server definitions, creating skills, or compiling code. (Point the user to `skill-creator` for skills.)
- This skill generates/edits the **agent file only**. Registering runtime wiring, availability checks, or other global config is handled elsewhere.

## The file format

Every agent file must start with YAML frontmatter delimited by `---`, followed by a Markdown body that is the system prompt. Example:

```markdown
---
name: reviewer
description: Code review specialist
role: subagent
model: deepseek/deepseek-v4-flash-0731
skills:
  - .anacleto/skills/code-review/
mcps: []
permissions:
  deny:
    - command.run
    - net.http
subagents: []
---

You are **Reviewer**, a code review specialist...
```

## Frontmatter fields (authoritative schema)

These come from `AgentConfig` / `parse_agent`. Use exactly these names.

| Field | Type | Required | Default | Meaning |
|---|---|---|---|---|
| `name` | string | ✅ | — | Unique agent name. Lowercase, kebab-case recommended. |
| `description` | string | ✅ | — | Short summary used for the agent picker/subagents. |
| `role` | `root` / `subagent` | — | `subagent` | `root` = user-invocable coordinator; `subagent` = disposable child. |
| `model` | string | — | `claude-sonnet-4-20250514` | LLM model name (see provider prefix rules below). |
| `skills` | list of paths | — | `[]` | Paths to skill dirs/files. **No inheritance** — each agent lists its own. |
| `mcps` | list of strings | — | `[]` | MCP server names (references global/project MCP definitions). Present but empty by default. |
| `permissions` | `{ allow: [], deny: [] }` | — | allow-all | Default is allow-by-default; list explicit `deny` (and optional `allow`). |
| `subagents` | list of names | — | `[]` | Subagent names **only for `role: root`** agents. Subagents can never nest. |
| `max_steps` | integer | — | from config (default 90) | Max LLM+tool turns per task before forced stop. |
| `subagent_depth` | integer | — | default from config | Max depth of dynamic subagent delegation. |

> **Root role requirement**: the merged agent set must contain at least one `role: root` agent, or Anacleto refuses to start. Multiple roots are allowed.

## Model → provider resolution (prefix matching)

The provider is chosen by the model name in `Engine::resolve_agent_provider`:

- starts with `claude` → **anthropic**
- starts with `gpt` / `o1` / `o3` → **openai**
- contains `/` → **openrouter** (OpenAI-compatible)
- anything else → **ollama**

## Invariants / validation rules

When creating or editing an agent, always enforce these (they mirror the architecture):

1. **Agents and subagents are the same type** — do not invent a different structure for subagents.
2. **Subagents cannot nest** — a `subagent` must have empty `subagents: []` and empty `subagent_depth`.
3. **No inheritance** — a subagent does NOT inherit skills, mcps, or permissions from its parent. List everything it needs explicitly.
4. **Only `root` agents have `subagents`** — if `role: subagent`, force `subagents: []`.
5. **Skill paths** are relative to the project root (e.g. `.anacleto/skills/<name>/`), which resolves correctly from any invocation directory (see `config/paths.rs`).
6. **Permissions default to allow** — only add `deny` entries that must be blocked. `allow` is optional and only used when you want to whitelist.
7. **`name` must be unique** across global + project agents (project overrides global by same name).
8. **At least one root** must exist when finished, or the session will not boot.

## Workflow

### 1. Create a new agent or subagent

1. Clarify with the user: role (`root` or `subagent`), purpose/persona, model, and which skills/mcps/permissions they need.
2. Ask which root agent will host a subagent (only relevant if `role: subagent`), so you can add it to that parent's `subagents:` list.
3. Generate the file with proper frontmatter + a clear Markdown system prompt (persona, responsibilities, workflow, constraints, tools).
4. Save to `~/.config/anacleto/agents/<name>.md` for global or `<project_root>/.anacleto/agents/<name>.md` for project scope (project is recommended for project-specific agents).
5. If it's a subagent, add `<name>` to the parent's `subagents:` list (see "Add/remove subagents" below).
6. Validate (see Validation).

### 2. Edit an existing agent

1. Read the current file (`cat ~/.config/anacleto/agents/<name>.md` or `<project_root>/.anacleto/agents/<name>.md`).
2. Apply only the requested changes, preserving everything else and the system prompt unless editing the prompt is explicitly requested.
3. Re-validate.

### 3. Add or remove a subagent from an agent

Use the `shell` skill to locate the parent agent file, then edit its `subagents:` list:

- **Add**: find the parent's file (e.g. a `root`), insert the new subagent name under `subagents:`, keeping the YAML list style consistent (either `- name` single-line items or `[a, b]` flow style). The subagent must exist as its own file; if not, create it first (Step 1).
- **Remove**: delete the name from the parent's `subagents:` list so the subagent is no longer spawned by that parent.
- Never add a subagent name to the list of another subagent (invariant 2/4).

Example — adding `writer` to a root's file:

```yaml
subagents:
  - reviewer
  - writer   # <- added
```

## Validation

Before declaring done, verify the file(s) against the schema:

1. File starts with `---` and has a closing `---`.
2. Frontmatter parses as valid YAML (run `python3 -c "import yaml,sys; yaml.safe_load(open(sys.argv[1])); print('OK')" <file>` if uncertain).
3. Required fields present: `name`, `description`.
4. `role` is one of `root`/`subagent`; subagents have `subagents: []`.
5. `subagents:` only on `root` agents.
6. No duplicate subagent names within a list.
7. Every `subagents:` entry resolves to an existing agent file (or is being created in the same operation).
8. `skills` paths point at existing skill dirs (`.anacleto/skills/<name>/`).
9. At least one root remains.
10. No frontmatter typo from the schema table above.

## Sample templates

### Root agent

```markdown
---
name: <name>
description: <short description>
role: root
model: claude-sonnet-4-20250514
skills:
  - .anacleto/skills/shell/
mcps: []
permissions:
  deny:
    - command.run.sudo
subagents: []
max_steps: 90
---

You are **<Name>**, <description>.

## Core identity
- ...

## Workflow
- ...

## Constraints
- ...
```

### Subagent

```markdown
---
name: <name>
description: <short description>
role: subagent
model: deepseek/deepseek-v4-flash-0731
skills:
  - .anacleto/skills/<skill>/
mcps: []
permissions:
  deny: []
subagents: []
---

You are **<Name>**, <description>.
...
```

## Common pitfalls

- Forgetting `role: root` on a coordinator → treated as a subagent, may break the "at least one root" guarantee.
- Copying a parent's `subagents` onto a subagent → subagent nesting, which is not allowed.
- Leaving `mcps` omitted when the schema expects `mcps: []` — always emit the field explicitly for consistency.
- Using a model name whose provider isn't configured → runtime resolution failure.
- Editing `config.yaml` when the user only asked for an agent file — **out of scope**.
