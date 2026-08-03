# ADR-0003: MCP Integration Model

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs to integrate with external tools and services via the Model Context Protocol (MCP).

## Decision

- **Anacleto is a consumer of MCP servers**, not a manager. It connects to existing MCP servers but does not start, stop, or restart them.
- **Transport**: stdio or TCP (as defined by the MCP spec).
- **Protocol**: JSON-RPC 2.0 over the transport.
- **MCP servers are configured globally** in the config file, then referenced by name in each agent/subagent.
- **Each agent/subagent has its own independent set of MCPs** — no inheritance.
- **If an MCP server is unavailable or crashes**, Anacleto does not attempt recovery. The error is surfaced to the user in the TUI.
- **Retries are configurable** for transient failures.

## Consequences

- Simple, stateless integration model.
- No process management overhead in Anacleto.
- Clear failure semantics: MCP errors are surfaced but not hidden.
- Users manage MCP servers externally (systemd, Docker, etc.).

## Alternatives Considered

- **Anacleto manages MCP servers as child processes**: Rejected. Adds complexity and couples Anacleto to process lifecycle.
- **Built-in tools instead of MCP**: Rejected. MCP provides a standard protocol for tool integration.