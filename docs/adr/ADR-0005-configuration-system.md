# ADR-0005: Configuration System

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs a configuration system that supports global defaults and per-project overrides.

## Decision

- **Format**: YAML for infrastructure; **Markdown + YAML frontmatter for agents**.
- **Locations**:
  - Global: `~/.config/anacleto/config.yaml`
  - Project: `.anacleto/config.yaml` (relative to project root)
- **Merge strategy**: Global and project configs are merged, with project config taking precedence.
- **Config sections** (in `config.yaml`):
  - `models`: LLM provider definitions (Anthropic, OpenAI, Ollama) with API keys, model IDs, and context windows.
  - `mcps`: Global MCP server definitions (name, transport, command, args).
  - `session`: Session settings (history limit percentage, database path).
- **Agents are NOT defined in `config.yaml`.** Each agent is a self-contained Markdown file with YAML frontmatter, located in the `agents/` directory:
  - Global: `~/.config/anacleto/agents/*.md`
  - Project: `.anacleto/agents/*.md`
  - The frontmatter holds the structural config (`name`, `description`, `role`, `model`, `max_steps`, `skills`, `mcps`, `permissions`, `subagents`) and the Markdown body is the agent's system prompt.
  - `max_steps` is the maximum number of turns (LLM + tool iterations) an agent may run per task before being forced to stop and mark the task as incomplete. The default comes from `config.yaml` → `session.max_steps` (default `90`) and can be overridden per agent in the frontmatter.
  - Agents are discovered by scanning the `agents/` directory (same pattern as skills).
  - Project agents override global agents with the same name.
  - At least one agent must declare `role: root`. Multiple root agents are allowed — each is a user-invocable coordinator with its own subagent team.

## Consequences

- Familiar YAML format for infrastructure, easy to read and write.
- Clean separation of global and project concerns.
- Agents and subagents share the same schema, reducing complexity.
- Agents are self-contained single-file artifacts (persona + structure co-located), consistent with the skill system.
- The `agents:` section was removed from `config.yaml`; agent definitions live entirely in Markdown.

## Alternatives Considered

- **JSON only**: Rejected. YAML is more readable and supports comments.
- **TOML**: Rejected in favor of YAML for familiarity.
- **Single file only**: Rejected. Global + project merge is more flexible.
- **Agents in `config.yaml`**: Rejected. Splits an agent's identity across two files and diverges from the skill pattern.