# ADR-0006: Persistence and Sessions

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs to persist session history for resumability and audit.

## Decision

- **Database**: SQLite via `sqlx` (async-native).
- **Storage location**: `~/.local/share/anacleto/sessions.db`.
- **Data model**:
  - Sessions: each conversation is a session with metadata (created, updated, model, agent).
  - History: messages are stored per session and per agent, with role, content, and timestamp.
- **Sessions are resumable**: users can close and reopen Anacleto and continue a previous session.
- **Context window management**: configurable percentage of the model's context window (default: 50%). When the limit is reached, older messages are summarized or truncated.
- **History is per-session and per-agent**: each agent within a session has its own message history.

## Consequences

- Full session persistence and resumability.
- Async SQLite via sqlx integrates naturally with Tokio.
- Context window management prevents token overflow.
- Per-agent history enables independent subagent contexts.

## Alternatives Considered

- **JSON file storage**: Rejected. SQLite provides better querying, concurrency, and integrity.
- **In-memory only**: Rejected. Session resumability requires persistence.
- **PostgreSQL**: Rejected. Overkill for a local CLI tool.