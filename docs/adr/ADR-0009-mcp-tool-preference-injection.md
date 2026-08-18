# ADR-0009: Dynamic MCP Tool Preference Injection in Agent Prompts

**Status:** Proposed  
**Date:** 2026-08-18  
**Deciders:** Project Director, User  

## Context

Agents in Anacleto can connect to MCP servers that expose tools (e.g., CodeGraph's `codegraph_files`, `codegraph_search`, `codegraph_context`). These tools are injected as function tools in the LLM API call, making them technically available to the model.

However, the LLM has no way to know:

1. **Which MCP tools exist** — they are not documented in the system prompt.
2. **Which MCP tools are preferred** over built-in alternatives — e.g., `codegraph_files` is faster and more accurate than `find`/`ls`/`glob`, but the LLM doesn't know this.
3. **When to use each MCP tool** — the LLM falls back to familiar built-in tools (`read`, `grep`, `glob`, `execute`) even when better MCP alternatives exist.

This was observed in production: `code-analyzer` has `mcps: [codegraph]` but never called any CodeGraph tool across 4 invocations and 78+ tool calls. Instead it used `find`, `ls`, `glob`, and hallucinated file paths.

The current approach of hardcoding tool preferences in agent `.md` files (e.g., "Use codegraph as the first option") is:
- **Brittle**: breaks when MCP servers change
- **Non-scalable**: each agent needs manual documentation of every MCP's tools
- **Invisible to the LLM**: the preference lives in prose, not in the tool definition itself

## Decision

The engine will **dynamically inject MCP tool preferences into the agent's system prompt** at session start, based on the agent's configured MCP servers.

### Mechanism

When an agent session starts, the engine will:

1. Query each connected MCP server for its list of available tools (via `tools/list`).
2. For each MCP tool, check if a built-in tool with overlapping functionality exists.
3. Generate a **"Preferred Tools" section** appended to the system prompt:

```
## Preferred Tools (from MCP servers)

The following tools from connected MCP servers are MORE EFFICIENT than their built-in
alternatives. Use them FIRST:

| MCP Tool | Replaces | Why |
|---|---|---|
| `codegraph_files` | `find` / `ls` / `glob` | Sub-millisecond, AST-aware file listing |
| `codegraph_search` | `grep` for symbol names | Semantic symbol lookup, no false positives |
| `codegraph_context` | multiple `read` + `grep` | Single-call comprehensive context |
| `codegraph_explore` | `glob` + many `read` | Deep module exploration with relationship map |
| `codegraph_callers` | `grep` on function name | Structured caller graph |
| `codegraph_callees` | manual code reading | Structured callee graph |
| `codegraph_impact` | manual dependency traversal | Blast-radius analysis |
```

4. If an MCP server is unavailable, the section is omitted (no hard failure).

### Overlap detection

The engine maintains a registry of built-in tool → MCP tool overlaps:

| Built-in tool | Overlapping MCP tool pattern |
|---|---|
| `read` (multiple files) | tools named `*context*`, `*explore*` |
| `grep` (symbol search) | tools named `*search*`, `*find*` |
| `glob` / `find` / `ls` | tools named `*files*`, `*tree*`, `*list*` |
| `grep` (callers) | tools named `*callers*`, `*references*` |
| manual code reading | tools named `*callees*`, `*dependencies*` |

This registry is configurable (YAML) so users can extend it for custom MCP servers.

### Prompt injection point

The generated section is injected **after the agent's system prompt body** and **before the lifecycle instructions**, so it's visible but does not override the agent's core identity.

### Permission model

MCP tools already require `mcp.use` permission. This ADR does not change that. The preference injection is purely informational — it tells the LLM which tool is better, but the existing permission model still applies.

## Consequences

### Positive

- **Self-documenting**: agents automatically know what MCP tools are available and why they're better.
- **MCP-agnostic**: works with any MCP server, not just CodeGraph. A Postgres MCP would get its own preference table.
- **No manual prompt maintenance**: agent `.md` files don't need to document MCP tools.
- **Fixes the observed bug**: code-analyzer would see "codegraph_files replaces find/ls/glob — use FIRST" and stop hallucinating paths.

### Negative

- **Startup latency**: querying each MCP server for its tool list adds ~100-500ms to session start. Mitigation: cache the tool list per MCP server with a TTL (e.g., 60 seconds).
- **Prompt size increase**: each MCP adds ~5-15 lines to the system prompt. Mitigation: only include tools that have a built-in overlap; pure MCP tools with no built-in equivalent can be omitted from the preference table (they're still available as function tools).
- **Registry maintenance**: the overlap registry needs updating when new built-in tools are added. Mitigation: keep it in a single YAML file.

## Implementation sketch

### Files

- `src/mcp/preferences.rs` — Overlap registry, preference table generation
- `src/agent/session.rs` — Inject preferences into system prompt at session start
- `~/.config/anacleto/mcp-preferences.yaml` — User-configurable overlap registry

### `mcp-preferences.yaml`

```yaml
# Maps built-in tool patterns to MCP tool name patterns
overlaps:
  - builtin: ["glob", "execute"]    # tools that list files
    pattern: "*files*"
    reason: "Sub-millisecond, AST-aware file listing"
  - builtin: ["grep"]               # tools that search text
    pattern: "*search*"
    reason: "Semantic lookup, no false positives"
  - builtin: ["read", "grep"]       # tools for context gathering
    pattern: "*context*"
    reason: "Single-call comprehensive context"
  - builtin: ["read", "glob"]       # tools for deep exploration
    pattern: "*explore*"
    reason: "Deep module exploration with relationship map"
  - builtin: ["grep"]               # tools for finding callers
    pattern: "*callers*"
    reason: "Structured caller graph"
  - builtin: ["read"]               # tools for finding callees
    pattern: "*callees*"
    reason: "Structured callee graph"
  - builtin: ["grep", "read"]       # tools for impact analysis
    pattern: "*impact*"
    reason: "Blast-radius analysis"
```

### Session startup flow

```
1. Agent session starts
2. Engine reads agent's mcps: [codegraph, ...]
3. For each MCP, call tools/list → get tool names
4. Match tool names against overlap registry patterns
5. Generate "Preferred Tools" markdown table
6. Inject into system prompt
7. Proceed with normal session loop
```

## Alternatives Considered

### 1. Hardcode preferences in agent `.md` files

Rejected. Already proven brittle — code-analyzer's prompt says "Use codegraph as the first option" but the LLM ignores it because there's no structured mapping between MCP tools and built-in alternatives.

### 2. Remove overlapping built-in tools when MCP is available

Rejected. Too aggressive — built-in tools are fallbacks when MCP is down. The LLM should still have access to them.

### 3. Tag MCP tools with `preferred: true` in the function tool definition

Rejected. The OpenAI/Anthropic tool call schema has no standard field for "preference". Custom extensions would be non-standard and ignored by most LLM providers.

### 4. Re-rank tools in the function tools array (MCP tools first)

Rejected. Tool array order does not reliably influence LLM selection across different models and providers.
