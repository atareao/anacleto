# ADR-0007: Permissions Model

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto agents can execute commands, read/write files, make network requests, and use MCP tools. A permissions system is needed to control what each agent can do.

## Decision

- **Permission types**:
  - `fs.read` — read files (with path allow/deny lists)
  - `fs.write` — write files (with path allow/deny lists)
  - `net.http` — make HTTP requests
  - `command.run` — execute system commands
  - `mcp.use` — use configured MCP servers
  - `env.read` — read environment variables
  - `skill.use` — invoke skills
- **Model**: Allow by default, deny explicitly.
- **Configuration**: Permissions are defined in the agent/subagent config under `permissions.deny`.
- **Human approval**: Certain sensitive operations (e.g., `sudo`, destructive commands) require explicit human approval via the TUI.
- **Subagents can have specific prohibitions** that differ from their parent agent.
- **No inheritance**: Each agent/subagent has its own independent permission set.

## Consequences

- Simple, permissive default that doesn't get in the way during development.
- Explicit deny for dangerous operations.
- Human-in-the-loop for critical operations.
- Clear audit trail of what was denied.

## Alternatives Considered

- **Deny by default**: Rejected. Too restrictive for a developer tool; would require extensive allow-listing.
- **Capability-based security**: Rejected. Overly complex for the use case.