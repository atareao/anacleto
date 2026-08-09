---
name: tool-discovery
description: |
  Audits and recommends which skill, MCP, or subagent to use for a given task.
  Must be invoked before Execute to ensure you use the right tool for the job.
  Use when the user asks "how should I do X", "what skill do I need for Y",
  or before starting any non-trivial task to pick the right tool.
metadata:
  version: "1.0"
  category: "tool-discovery"
  risk: "low"
---

# Tool Discovery Skill

Audit a task description and recommend the best skill, MCP server, and/or subagent
to use. Run this **before** invoking Execute to avoid using a generic tool when a
specialized one exists.

## Workflow

When a task arrives, follow these steps before executing:

### Step 1: Analyze the task

Identify:

- **Domain** (web dev, devops, documentation, code review, data science, etc.)
- **Action type** (create, modify, review, search, research, plan, test, deploy)
- **Target** (code, config, documentation, infrastructure, data)
- **Complexity** (simple command, multi-step workflow, open-ended research)

### Step 2: Audit available skills

List all installed skills and match them against the task:

```bash
# List all project skills with names and descriptions
for f in .agents/skills/*/SKILL.md; do
  name=$(head -1 "$f" | sed -n 's/^name: *//p')
  desc=$(sed -n '/^description:/,/^[a-z]/p' "$f" | grep -v '^description:' | tr -d '\n' | sed 's/^ *//')
  echo "$name: $desc"
done
```

Map the task to skills using the following heuristics:

| If the task involves...                      | Use skill...         |
|----------------------------------------------|----------------------|
| Writing or modifying Rust code               | `rust-dev`           |
| Writing or modifying Python code             | `python-dev`         |
| Shell commands, scripts, or terminal ops     | `shell`              |
| Files, directories, reading/writing files    | `filesystem`         |
| Searching the web or fetching docs           | `web-research`       |
| Code quality, correctness, standards         | `code-review`        |
| Creating or editing a skill                  | `skill-creator`      |
| Creating or editing agents/subagents/MCPs    | `agent-creator`      |
| Finding which skill exists for a need        | `find-skills`        |
| Breaking down projects into steps            | `planning`           |
| Git, branching, merging, PRs                 | `version-control`    |
| Writing technical documentation (atareao)    | `tech-writer`        |
| Weather information                          | `weather`            |

### Step 3: Audit available MCPs

Check which MCP servers are configured for the current agent:

```bash
# List configured MCPs from agent config
grep -A5 "mcps:" .agents/agents/root.md | head -10

# Or from global config
grep -A5 "mcps:" ~/.config/anacleto/config.yaml 2>/dev/null
```

Common MCPs and their uses:

| MCP          | Use when...                                         |
|--------------|-----------------------------------------------------|
| `codegraph`  | You need structural code queries (symbols, callers) |
| `filesystem` | You need direct file read/write via MCP             |

### Step 4: Audit available subagents

Check which subagents are configured and match them to specialized work:

```bash
# List subagents from root agent
grep -A10 "subagents:" .agents/agents/root.md | head -12
```

Subagent delegation rules:

| Subagent    | Delegate when...                                        |
|-------------|----------------------------------------------------------|
| `reviewer`  | Code needs to be reviewed for quality/correctness        |
| `writer`    | Documentation, READMEs, or explanatory content needed    |
| `rust-dev`  | Rust implementation, compilation, testing, debugging     |
| `tech-writer` | Technical articles in atareao.es editorial style       |
| `python-dev` | Python implementation, testing, ruff/mypy/pytest        |

### Step 5: Cross-reference and recommend

For each candidate skill, verify:

1. **Is its domain a superset of the task's domain?** (e.g., `rust-dev` covers any Rust work)
2. **Can it perform the action type needed?** (e.g., `shell` can execute, not plan)
3. **Is there a more specific tool?** (prefer `rust-dev` over `shell` for Rust compilation)
4. **Does it need supporting skills?** (e.g., `planning` + `version-control` for a release plan)

## Output format

Return a structured recommendation:

```
Task: <brief description>

Recommended skill: <skill-name>
Reason: <why this skill fits best>

Supporting MCP: <mcp-name>
Reason: <what the MCP provides>

Delegate to subagent: <subagent-name>
Reason: <why this subagent should handle it>

Alternative: <fallback if primary unavailable>
```

## Examples

### Example 1: Implement a Rust function

```
Task: Implement a new async function in the engine module
Recommended skill: rust-dev
Reason: Rust implementation with async, compilation, and testing
Supporting MCP: codegraph
Reason: Need to understand caller/called symbol relationships before coding
Delegate to subagent: rust-dev
Reason: Rust development is a specialized subagent task
```

### Example 2: Write a blog post

```
Task: Write a technical article about agent orchestration
Recommended skill: filesystem
Reason: Read existing articles for style reference
Supporting MCP: (none)
Delegate to subagent: tech-writer
Reason: Specialized in atareao.es editorial style with two-phase workflow
```

### Example 3: Fix a bug in unknown codebase area

```
Task: Find and fix a crash in agent loading
Recommended skill: shell
Reason: Explore project structure, grep for relevant code
Supporting MCP: codegraph
Reason: Trace symbol dependencies to find root cause
Delegate to subagent: (none)
Alternative: rust-dev once the bug location is known
```

## Important notes

1. **Always run this skill before Execute.** It is a mandatory gate in the workflow.
2. **Prefer specialized skills over generic ones.** Do not use `shell` or `filesystem` when `rust-dev`, `python-dev`, or `code-review` covers the task.
3. **MCPs augment, not replace, skills.** Use both when the task benefits from structural code understanding (codegraph).
4. **Subagents are for delegation, not tools.** Subagents handle entire work items; skills are invoked by the agent directly.
5. **When in doubt, recommend `find-skills` first** to discover if a more specific skill exists than the ones you already know about.