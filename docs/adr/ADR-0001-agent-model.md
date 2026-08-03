# ADR-0001: Agent Model and Hierarchy

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs an agent model that supports hierarchical orchestration: a root agent that delegates work to subagents, each with independent configuration.

## Decision

- **Agents and subagents are the same type** with the same schema. The only structural difference is that an agent has a `subagents: []` field referencing subagent names.
- **Agents are defined as self-contained Markdown files** with YAML frontmatter (see ADR-0005). The frontmatter holds the structural config and the Markdown body is the system prompt. The root agent is identified by an explicit `role: root` field.
- **Only agents are invocable directly** by the user. Subagents are invoked exclusively through their parent agent.
- **Subagents cannot have subagents** — the hierarchy is strictly two levels (agent → subagent).
- **Subagents are disposable**: created for a task, destroyed after completion.
- **Subagents are independent**: they do not inherit skills, MCPs, or permissions from the parent. The parent only passes messages.
- **Only the root agent** can create subagents.

## Consequences

- Simple, flat hierarchy that is easy to reason about.
- Clear separation of concerns: each agent/subagent is self-contained.
- No complex inheritance resolution needed.
- Subagent lifecycle is straightforward (create → work → reply → destroy).

## Alternatives Considered

- **N-level hierarchy**: Rejected for complexity. Two levels cover the primary use case (delegation).
- **Inheritance model**: Rejected. Independence is simpler and avoids subtle bugs.