# ADR-0002: Skill System

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs a mechanism to provide agents with specialized capabilities (skills) that can be loaded dynamically.

## Decision

- **Skills are Markdown files with YAML frontmatter**, following the Anthropic skill format:

  ```markdown
  ---
  name: my-skill
  description: What it does and when to use it
  ---
  # Instructions
  ...
  ```

- The **frontmatter** (name + description) is what the LLM reads to decide which skill to invoke.
- The **body** is only loaded into context when the skill is invoked.
- Skills can bundle optional resources: `scripts/`, `references/`, `assets/`.
- Skills are loaded from:
  - A local project directory (`.agents/skills/`)
  - Absolute paths on the filesystem
- Each agent/subagent has its **own independent set of skills**.
- The **LLM decides which skill to invoke** based on the `description` field, unless explicitly routed.
- Skills are **not inherited** from parent to subagent.

## Consequences

- Simple, human-readable skill format.
- Compatible with the Anthropic ecosystem.
- Token-efficient: only skill metadata is loaded at startup; bodies are lazy-loaded.
- Clear separation: each agent owns its skills.

## Alternatives Considered

- **WASM plugins**: Rejected for complexity. Markdown skills are sufficient for LLM-driven agents.
- **TOML/JSON skills**: Rejected in favor of the Anthropic Markdown standard.