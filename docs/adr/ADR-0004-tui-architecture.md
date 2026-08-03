# ADR-0004: TUI Architecture

**Status:** Accepted  
**Date:** 2026-08-02  
**Deciders:** Project Director, User  

## Context

Anacleto needs a user interface. The decision was made to use a Terminal User Interface (TUI) as the sole interaction mode.

## Decision

- **Framework**: `ratatui` + `crossterm`.
- **Same process as the engine**: TUI and engine run as separate Tokio tasks within the same binary, communicating via Tokio channels (`mpsc`).
- **TUI views**:
  - **Chat panel**: displays conversation history with streaming tokens.
  - **Input panel**: text input area for the user.
  - **Agent list**: shows available agents and their status.
  - **Skill/MCP indicator**: shows which skills and MCPs are active.
  - **Subagent tree**: shows subagent hierarchy and status.
- **Streaming**: LLM tokens are streamed in real-time to the chat panel.
- **Intermediate steps**: skill execution and MCP calls are shown as visible intermediate steps with live output.
- **Errors**: displayed as visible error messages in the TUI with an option to view raw details.
- **No batch/scripting mode**: TUI is the only interaction mode.

## Consequences

- Single binary, simple deployment (`cargo run`).
- Responsive TUI even during long LLM calls (thanks to Tokio task separation).
- Rich user experience with real-time streaming and progress visibility.
- No need for IPC or serialization between components.

## Alternatives Considered

- **Separate TUI and engine processes**: Rejected. Adds IPC complexity without sufficient benefit for a single-user tool.
- **Web UI**: Rejected. TUI is simpler and more appropriate for a CLI developer tool.
- **REPL mode**: Rejected in favor of full TUI.