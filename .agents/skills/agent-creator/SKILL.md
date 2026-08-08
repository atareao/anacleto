---
name: agent-creator
description: |
  Create, modify, and manage Anacleto agents and subagents.
  Use when users want to create a new agent from scratch, edit an existing agent,
  add or remove skills and MCPs from an agent, or restructure subagent teams.
  Covers the full agent lifecycle: creation, configuration, review, and deletion.
metadata:
  version: "1.0"
  category: development
  risk: low
---

# Agent Creator

A skill for creating and managing **Anacleto agents and subagents** — the full
lifecycle from capture of intent to final configuration.

At a high level, the process of creating an agent goes like this:

- **Understand** what the user wants the agent to do
- **Design** the agent: role, personality, skills, MCPs, permissions, subagents
- **Write** the agent definition file (Markdown + YAML frontmatter)
- **Register** the agent so the engine discovers it
- **Test** the agent with real prompts
- **Iterate** based on results

Your job when using this skill is to figure out where the user is in this process
and then jump in and help them progress through these stages.

---

## Agent Architecture (must-know)

Before designing any agent, you MUST understand these rules from the Anacleto
architecture (see `docs/adr/ADR-0001-agent-model.md`):

| Rule | Detail |
|---|---|
| **Agents and subagents are the same type** | Same schema. Only difference: agents have `subagents: []` referencing other agents. |
| **Only agents are user-invocable** | Subagents are invoked exclusively through their parent agent. |
| **Subagents cannot nest** | Hierarchy is strictly two levels: agent → subagent. |
| **Subagents are disposable** | Created for a task, destroyed after completion. |
| **No inheritance** | Subagents do NOT inherit skills, MCPs, or permissions from their parent. |
| **Only `role: root` can create subagents** | At least one agent must declare `role: root`. |

### Agent types

| Type | `role` field | `subagents` field | Invocable by user |
|---|---|---|---|
| **Root agent** | `root` | Contains subagent names | ✅ Yes |
| **Agent** (non-root) | (omitted) or custom | Must be `[]` | ✅ Yes |
| **Subagent** | `subagent` | Must be `[]` | ❌ No (parent-only) |

> ⚠️ **Only agents with `role: root` may have subagents.** Non-root agents (no `role` or
> a custom role) cannot orchestrate subagent teams. If you need an agent that delegates
> work to others, it must declare `role: root`.

---

## Agent definition format

Every agent is a **self-contained Markdown file** with YAML frontmatter.
The frontmatter holds the structural config; the Markdown body is the system prompt.

### Location

```
# Project-level agents (override globals)
.agents/agents/<name>.md

# Global agents (machine-wide defaults)
~/.config/anacleto/agents/<name>.md
```

### Frontmatter schema

```yaml
---
name: <agent-name>                # Required. Unique identifier (kebab-case).
description: <brief description>  # Required. One-line summary of the agent's purpose.
role: root | subagent             # Optional for non-root agents. "root" for root agents, "subagent" for subagents.
model: <provider/model>           # Required. e.g. "claude-sonnet-4", "deepseek/deepseek-v4-flash"
max_steps: <integer>              # Optional. Max LLM+tool iterations per task. Default from config (90).
skills:                           # Optional. List of paths to skill directories.
  - .agents/skills/<name>/
mcps: [<mcp-name>]                # Optional. List of MCP server names (from config).
permissions:                      # Optional. Allow/deny rules.
  allow: []                       #   Explicit allow list (rarely needed — deny by default).
  deny:                           #   Explicit deny list.
    - command.run.sudo
    - net.http.delete
subagents:                        # Optional. Subagent names (only for agents, not subagents).
  - reviewer
  - writer
---
```

### System prompt (Markdown body)

Everything after the frontmatter `---` is the agent's system prompt. It defines:

- The agent's **identity** and **persona**
- **Capabilities** the agent has
- **Workflow** instructions
- **Constraints** and **rules**
- **Output format** expectations

---

## Communicating with the user

Adapt your language to the user's familiarity with the Anacleto system.

- If the user is new, explain the concepts briefly (agent vs subagent, skills, MCPs,
  permissions, the frontmatter format).
- If the user is experienced, get straight to the point.

Always use structure: present options, confirm decisions, then execute.

---

## Workflow: Creating a new agent

### Phase 1 — Capture intent

Start by understanding what the user wants. If the conversation already contains
the requirements (e.g., "I want an agent that writes commit messages"), extract
them and confirm. Otherwise, ask clarifying questions:

1. **Purpose**: What should this agent enable the user to do?
2. **Role**: Is this a root agent, a regular agent, or a subagent?
3. **Personality**: What tone/style should the agent have? (formal, creative,
   technical, concise)
4. **Skills**: What tools/capabilities does it need?
   - Shell access? Filesystem read/write? Web research? Code review?
5. **MCPs**: What external services does it need?
   - Database? Filesystem server? Custom API?
6. **Permissions**: Are there operations it should NEVER do?
   - Run sudo? Delete files? Make network calls?
7. **Subagents**: Should it delegate to subagents? Which ones?
8. **Model**: What LLM should power it? (claude, gpt, local via ollama)
9. **Max steps**: How many iterations before it must stop?

### Phase 2 — Resolve configuration details

Before writing the file, resolve these concrete values:

**Model selection** (based on provider conventions):

| Pattern | Provider |
|---|---|
| Starts with `claude` | Anthropic |
| Starts with `gpt`/`o1`/`o3` | OpenAI |
| Contains `/` | OpenRouter |
| Everything else | Ollama |

**Path resolution for skills**:

| Scope | Path pattern |
|---|---|
| Project | `.agents/skills/<name>/` |
| Global | `~/.config/anacleto/skills/<name>/` |

**MCP references**: Must match a name in `config.yaml` → `mcps:` section.

### Phase 3 — Write the agent file

Create the file at `.agents/agents/<name>.md`.

**Structure the system prompt** with these sections:

```markdown
---
<frontmatter>
---

# Identity (one paragraph)

You are **<Name>**, a <role> specialized in <purpose>.

## Core identity

- <trait 1>
- <trait 2>
- <trait 3>

## Capabilities

Describe what tools/skills the agent has access to.

## Workflow

Step-by-step process the agent follows when given a task.

## Constraints

Rules the agent must NEVER violate.

## Output format

How the agent formats its responses.
```

### Phase 4 — Register the agent

Ensure the agent is discoverable by the engine:

1. The file is at `.agents/agents/<name>.md` (project) or `~/.config/anacleto/agents/<name>.md` (global).
2. If it's a root agent, `role: root` is set.
3. If it's a subagent, its parent agent lists it in the `subagents:` field.
4. The engine discovers agents by scanning `agents/` directories.

### Phase 5 — Test the agent

Suggest 2-3 test prompts the user can run in the TUI to verify the agent works.

For subagents, the test involves invoking the parent agent with a task that
triggers delegation to the new subagent.

---

## Workflow: Editing an existing agent

### 1. Read the current agent file

```bash
cat .agents/agents/<name>.md
```

### 2. Identify what to change

Confirm with the user:

- **Frontmatter changes**: name, description, role, model, max_steps
- **Skill changes**: add or remove skills
- **MCP changes**: add or remove MCP server references
- **Permission changes**: add or remove deny rules
- **Subagent changes**: add or remove subagent references
- **Prompt changes**: rewrite part of the system prompt

### 3. Apply changes

Edit the agent file. Validate the YAML frontmatter is well-formed.

---

## Workflow: Adding skills to an agent

### Detect available skills

List available skills from the project:

```bash
ls .agents/skills/
```

List global skills:

```bash
ls ~/.config/anacleto/skills/ 2>/dev/null
```

Inspect a skill's frontmatter to see its description:

```bash
head -5 .agents/skills/<name>/SKILL.md
```

### Add a skill to an agent

1. Edit the agent's frontmatter `skills:` list.
2. Add the path: `.agents/skills/<name>/` or `~/.config/anacleto/skills/<name>/`.
3. Ensure the skill exists at that path.
4. If the agent's system prompt lists capabilities, update it to mention the new skill.

### Remove a skill from an agent

1. Edit the agent's frontmatter `skills:` list — remove the entry.
2. If the system prompt mentions the skill, remove or update that mention.
3. No other cleanup needed (skills are loaded dynamically).

---

## Workflow: Adding MCPs to an agent

### Detect available MCPs

Read the config file to see defined MCP servers:

```bash
# From project config
cat .agents/config.yaml

# From global config
cat ~/.config/anacleto/config.yaml 2>/dev/null
```

List configured MCP server names:

```bash
# From project config
yq '.mcps | keys | .[]' .agents/config.yaml 2>/dev/null

# From global config
yq '.mcps | keys | .[]' ~/.config/anacleto/config.yaml 2>/dev/null
```

### Add an MCP to an agent

1. Verify the MCP server is defined in `config.yaml` → `mcps:`.
2. Edit the agent's frontmatter `mcps:` list — add the MCP name.
3. Confirm the MCP server is running (Anacleto is a consumer, not a manager).

### Remove an MCP from an agent

1. Edit the agent's frontmatter `mcps:` list — remove the entry.
2. No other cleanup needed.

---

## Workflow: Creating a subagent for an existing agent

### Step 1 — Design the subagent

Subagents are **fully independent** (no inheritance). They need their own:

- `name`, `description`, `model`
- `skills` (must be explicitly listed)
- `mcps` (must be explicitly listed)
- `permissions` (must be explicitly set)
- `role: subagent` and `subagents: []`

### Step 2 — Create the subagent file

Write the file at `.agents/agents/<subagent-name>.md`.

Example minimal subagent:
```markdown
---
name: my-subagent
description: Handles a specific task
role: subagent
model: deepseek/deepseek-v4-flash
skills:
  - .agents/skills/shell/
mcps: []
permissions:
  deny:
    - command.run.sudo
subagents: []
---

You are a <role> specialized in <purpose>.

## Workflow

1. <step 1>
2. <step 2>

## Constraints

- <constraint 1>
```

### Step 3 — Register in the parent

Edit the parent agent's frontmatter, adding the subagent name to `subagents:`:

```yaml
subagents:
  - reviewer
  - writer
  - my-subagent    # ← added
```

### Step 4 — Update the parent's system prompt

Add a mention of the new subagent in the parent's capabilities section, so the
parent knows it can delegate to it:

```markdown
You can also delegate tasks to your subagents:

- **my-subagent** — <what it does>
```

---

## Workflow: Removing a subagent from an agent

1. Remove the subagent file (or archive it):
   ```bash
   mv .agents/agents/<subagent>.md .agents/agents/<subagent>.md.bak
   ```
2. Edit the parent agent's frontmatter — remove the subagent name from `subagents:`.
3. Update the parent agent's system prompt — remove mention of the subagent.

---

## Permission configuration guide

### Available permission types

| Permission | Description | `deny` value |
|---|---|---|
| Filesystem read | Read files from disk | `fs.read` |
| Filesystem write | Write files to disk | `fs.write` |
| Network HTTP | Make HTTP/S requests | `net.http` |
| Command execution | Run shell commands | `command.run` |
| MCP usage | Use MCP tools | `mcp.use` |
| Environment read | Read environment variables | `env.read` |
| Skill usage | Invoke skills | `skill.use` |

### Scoped deny (with sub-permissions)

```yaml
permissions:
  deny:
    - command.run.sudo          # Deny only sudo commands
    - net.http.delete           # Deny only DELETE requests
    - fs.write./etc/            # Deny writes to /etc/
```

### Common patterns

| Agent type | Recommended deny list |
|---|---|
| **Root agent** | `command.run.sudo`, `net.http.delete` |
| **Reviewer subagent** | `command.run`, `fs.write` (read-only) |
| **Writer subagent** | `command.run`, `net.http` |
| **Research subagent** | `command.run`, `fs.write` |

---

## Testing and validation

After creating or modifying an agent, recommend the user run a quick test:

### For root agents
```
Just run `anacleto` in the project directory. The agent should appear in the
agent selector. Select it and try a simple prompt.
```

### For subagents
```
Use the parent agent and give it a task that should trigger delegation to the
new subagent. For example:

User: "Review the code in src/ for correctness"
→ Parent should delegate to the reviewer subagent.
```

### Validation checklist

Before declaring an agent done, verify:

- [ ] Frontmatter YAML is syntactically valid
- [ ] `name` is kebab-case and unique
- [ ] `description` is a single line
- [ ] `role` is correct:
  - `root` for root agents (those that orchestrate subagents)
  - `subagent` for subagents (those invoked by parent agents)
  - omitted for standalone non-root agents without subagents
- [ ] Agents with `role: root` have at least one subagent listed (or intentionally none)
- [ ] Agents with `role: subagent` have `subagents: []`
- [ ] `model` refers to a configured provider
- [ ] All skill paths exist on disk
- [ ] All MCP names are defined in `config.yaml` → `mcps:`
- [ ] `subagents: []` for subagents
- [ ] Subagents referenced in parent's `subagents:` actually exist
- [ ] System prompt clearly defines identity, capabilities, workflow, constraints

---

## Examples

### Example 1: Creating a simple subagent

**User says:** "I need a subagent that checks Rust code formatting"

**You respond:** Let's clarify...

1. Purpose: Runs `cargo fmt --check` and reports issues.
2. Skills: Needs `shell` to run the command.
3. Permissions: Read-only, no network, no sudo.
4. Model: Same as parent.

Then create `.agents/agents/rust-fmt-checker.md`:

```markdown
---
name: rust-fmt-checker
description: Checks Rust code formatting with cargo fmt --check
role: subagent
model: deepseek/deepseek-v4-flash
skills:
  - .agents/skills/shell/
mcps: []
permissions:
  deny:
    - command.run.sudo
    - fs.write
    - net.http
subagents: []
---

You are a **Rust formatting checker**, a subagent specialized in running
`cargo fmt --check` and reporting formatting issues.

## Task

When asked to check Rust formatting:

1. Run `cargo fmt --check` in the workspace.
2. If it succeeds, report "✅ All files are properly formatted."
3. If it fails, show the diff and list the unformatted files.

## Constraints

- Do NOT modify any files (you are read-only).
- Do NOT run any command other than `cargo fmt --check`.
```

Then edit the parent agent's frontmatter to add `rust-fmt-checker` to `subagents:`
and update the system prompt.

### Example 2: Adding a skill to the root agent

**User says:** "Give my root agent access to the filesystem skill"

**You do:**

1. Verify `filesystem` skill exists: `ls .agents/skills/filesystem/SKILL.md`
2. Edit `.agents/agents/root.md` frontmatter:
   ```yaml
   skills:
     - .agents/skills/shell/
     - .agents/skills/filesystem/   # ← added
   ```
3. Update the system prompt's capabilities section to mention the new skill.

### Example 3: Removing an MCP from an agent

**User says:** "Remove the database MCP from my agent"

**You do:**

1. Read the agent file: `cat .agents/agents/root.md`
2. Find the `mcps:` line and remove `postgres` from the list.
3. Save and confirm the change.

### Example 4: Creating a new root agent

**User says:** "I want a separate agent for writing documentation"

**You do:**

1. Capture intent: documentation specialist, web research, no shell access.
2. Create `.agents/agents/doc-writer.md`:

```markdown
---
name: doc-writer
description: Documentation specialist for creating and maintaining project docs
role: root
model: deepseek/deepseek-v4-flash
skills:
  - .agents/skills/web-research/
mcps: []
permissions:
  deny:
    - command.run
subagents:
  - writer
---

You are **Doc Writer**, a documentation specialist within the Anacleto engine.

## Core identity

- You write clear, well-structured documentation.
- You research topics thoroughly before writing.
- You follow the project's documentation conventions.

## Capabilities

You have access to web research for fact-checking and reference gathering.

You can delegate writing tasks to your writer subagent.

## Workflow

1. Understand what needs to be documented.
2. Research the topic if needed.
3. Outline the document.
4. Write or delegate writing.
5. Review and polish.

## Constraints

- You do not execute shell commands.
- You follow Markdown conventions.
```
```

---

## Error recovery

### "Agent file already exists"

If the agent name already exists, ask the user:
- Do you want to **overwrite** it? (dangerous — confirm twice)
- Do you want to **edit** the existing one?
- Do you want to **choose a different name**?

### "Skill directory does not exist"

If a skill path in the frontmatter does not exist on disk:
- Suggest creating the skill first (with `skill-creator`).
- Or suggest removing the reference.

### "MCP not defined in config"

If an MCP name is referenced but not defined in `config.yaml` → `mcps:`:
- Suggest adding the MCP to the config first.
- Or using a different MCP name.

### "Parent agent not found"

If a subagent references a parent that doesn't list it in `subagents:`:
- Edit the parent to add the subagent reference.

---

## Creating test prompts

After creating an agent, create test prompts to verify it works. Store them at
`.agents/skills/agent-creator/tests/<agent-name>-tests.md`.

Each test file should contain:

```markdown
# Tests for <agent-name>

## Test 1: Happy path

**Prompt:** <what to ask>
**Expected behavior:** <what should happen>

## Test 2: Edge case

**Prompt:** <edge case prompt>
**Expected behavior:** <how the agent should handle it>

## Test 3: Constraint boundary

**Prompt:** <something that should be denied>
**Expected behavior:** <agent should refuse or escalate>
```

---

## Prohibited operations

This skill creates and modifies agent configurations. The following are prohibited:

- Do NOT delete agent files without explicit user confirmation (ask twice).
- Do NOT modify agents outside the project or global agents directories.
- Do NOT change an agent's `role: root` to anything else unless the user explicitly
  confirms they understand the implications (loss of root status).
- Do NOT create skills — use `skill-creator` for that. This skill only manages
  skill **references** in agent configurations.
- Do NOT modify the engine's source code (`src/`). Agent configuration is
  separate from engine code.
