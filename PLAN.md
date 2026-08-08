# LLM Thinking/Reasoning Support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add real-time display of LLM reasoning/thinking (Anthropic extended thinking, OpenRouter reasoning tokens) in the TUI, togglable via `/thinking`.

**Architecture:** Extend `LlmStreamChunk` with a `Thinking` variant that carries reasoning text. Each provider parses its native reasoning field and emits `Thinking` chunks. The engine forwards these as `EngineEvent::AgentThinkingChunk`. The TUI accumulates them in a separate `current_thinking` field and renders with a distinct yellow/amber style. The existing `/thinking` command (which toggles `show_thinking`) controls visibility.

**Tech Stack:** Rust, Tokio, ratatui, serde, reqwest

## Global Constraints

- Edition 2024, Rust ≥ 1.85
- `cargo fmt --check && cargo clippy && cargo test` must pass before commits
- No new dependencies
- All providers must compile (even if they don't support reasoning)

---

### Task 1: LLM types — LlmStreamChunk::Thinking + OpenAI reasoning fields

**Files:**
- Modify: `src/llm/types.rs:100-105`
- Modify: `src/llm/openai.rs:84-88`
- Modify: `src/llm/openai.rs:112-117`

**Interfaces:**
- Consumes: `LlmStreamChunk` enum, `OpenAiResponseMessage`, `OpenAiStreamDelta`
- Produces: `LlmStreamChunk::Thinking(String)` variant, `OpenAiResponseMessage.reasoning: Option<String>`, `OpenAiStreamDelta.reasoning: Option<String>`

- [ ] **Step 1: Add `Thinking` variant to `LlmStreamChunk`**

In `src/llm/types.rs`, add after `ToolCall(ToolCall),`:
```rust
    /// Intermediate reasoning/thinking text (e.g. Anthropic extended thinking,
    /// OpenRouter reasoning tokens). Emitted before Content when available.
    Thinking(String),
```

- [ ] **Step 2: Add `reasoning` to `OpenAiResponseMessage`**

In `src/llm/openai.rs`, add after `tool_calls`:
```rust
    /// OpenRouter/OpenAI reasoning tokens (non-streaming).
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
```

- [ ] **Step 3: Add `reasoning` to `OpenAiStreamDelta`**

In `src/llm/openai.rs`, add after `tool_calls`:
```rust
    /// OpenRouter/OpenAI reasoning tokens (streaming).
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation or only pre-existing warnings.

---

### Task 2: OpenRouter provider — parse reasoning in complete() and complete_stream()

**Files:**
- Modify: `src/llm/provider.rs:425-445` (complete)
- Modify: `src/llm/provider.rs:455-540` (complete_stream)

**Interfaces:**
- Consumes: `OpenAiResponseMessage.reasoning`, `OpenAiStreamDelta.reasoning`, `LlmStreamChunk::Thinking`
- Produces: `LlmResponse.thinking` populated with reasoning text from non-streaming calls; `LlmStreamChunk::Thinking` emitted for streaming reasoning

- [ ] **Step 1: Parse reasoning in `OpenRouterProvider::complete()`**

Find the section in `src/llm/provider.rs` around line 440 where `Ok(LlmResponse { ... })` is constructed. Change:
```rust
            thinking: None,
```
to:
```rust
            thinking: choice.message.reasoning,
```

- [ ] **Step 2: Emit Thinking chunks in `OpenRouterProvider::complete_stream()`**

In `src/llm/provider.rs`, in the streaming loop where `choice.delta.content` is checked, add after content emission (around line 464):
```rust
                                    // Emit reasoning tokens if present
                                    if let Some(ref reasoning) = choice.delta.reasoning
                                        && !reasoning.is_empty()
                                    {
                                        let _ = tx.send(Ok(LlmStreamChunk::Thinking(reasoning.clone()))).await;
                                    }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 3: OpenAI provider — parse reasoning (forward-compatible)

**Files:**
- Modify: `src/llm/openai.rs:370-395` (complete)
- Modify: `src/llm/openai.rs:400-490` (complete_stream)

**Interfaces:**
- Same changes as Task 2 but in `src/llm/openai.rs`

- [ ] **Step 1: Parse reasoning in `OpenAIProvider::complete()`**

In `src/llm/openai.rs`, find the `Ok(LlmResponse { ... })` construction (around line 380). Change:
```rust
            thinking: None,
```
to:
```rust
            thinking: choice.message.reasoning,
```

- [ ] **Step 2: Emit Thinking chunks in `OpenAIProvider::complete_stream()`**

In `src/llm/openai.rs`, where `choice.delta.content` is checked, add after content emission (around line 395):
```rust
                                    // Emit reasoning tokens if present (OpenAI-compatible)
                                    if let Some(ref reasoning) = choice.delta.reasoning
                                        && !reasoning.is_empty()
                                    {
                                        let _ = tx.send(Ok(LlmStreamChunk::Thinking(reasoning.clone()))).await;
                                    }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 4: Anthropic provider — emit Thinking in complete_stream()

**Files:**
- Modify: `src/llm/anthropic.rs:295-305`

**Interfaces:**
- Consumes: `LlmResponse.thinking` (already populated by `complete()`)
- Produces: `LlmStreamChunk::Thinking(response.thinking)` before content

- [ ] **Step 1: Emit Thinking chunk before Content in complete_stream()**

In `src/llm/anthropic.rs`, around line 295, change:
```rust
        if !response.content.is_empty() {
            let _ = tx.send(Ok(LlmStreamChunk::Content(response.content))).await;
        }
```
to:
```rust
        // Emit thinking first (if present), then content
        if let Some(thinking) = response.thinking.filter(|t| !t.is_empty()) {
            let _ = tx.send(Ok(LlmStreamChunk::Thinking(thinking))).await;
        }
        if !response.content.is_empty() {
            let _ = tx.send(Ok(LlmStreamChunk::Content(response.content))).await;
        }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 5: Engine — handle Thinking chunks + new EngineEvent

**Files:**
- Modify: `src/agent/lifecycle.rs:430-482`
- Modify: `src/engine/events.rs:42-47`

**Interfaces:**
- Consumes: `LlmStreamChunk::Thinking(String)` — the engine receives these from the stream
- Produces: `EngineEvent::AgentThinkingChunk { agent_id, agent_name, content }` — new event sent to TUI

- [ ] **Step 1: Add `AgentThinkingChunk` variant to `EngineEvent`**

In `src/engine/events.rs`, add after `AgentStreamChunk` (around line 47):
```rust
    /// Thinking/reasoning chunk from an agent's LLM response.
    AgentThinkingChunk {
        agent_id: AgentId,
        agent_name: String,
        content: String,
    },
```

- [ ] **Step 2: Handle `LlmStreamChunk::Thinking` in engine stream loop**

In `src/agent/lifecycle.rs`, in the stream processing loop (around line 434), add a new match arm before `Ok(LlmStreamChunk::Content(text))`:
```rust
                                        Ok(LlmStreamChunk::Thinking(text)) => {
                                            let _ = event_tx
                                                .send(EngineEvent::AgentThinkingChunk {
                                                    agent_id: agent_id.clone(),
                                                    agent_name: agent_name.clone(),
                                                    content: text,
                                                })
                                                .await;
                                        }
```

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 6: App state — current_thinking field + Theme thinking colors

**Files:**
- Modify: `src/tui/app.rs:54-60` (add field)
- Modify: `src/tui/app.rs:269-275` (initialize)
- Modify: `src/tui/theme.rs` (add thinking colors)

**Interfaces:**
- Consumes: None (new state)
- Produces: `App.current_thinking: Option<String>`, `Theme::thinking()`, `Theme::thinking_dim()` used by renderer

- [ ] **Step 1: Add `current_thinking` field to `App`**

In `src/tui/app.rs`, add after `current_stream` (around line 55):
```rust
    /// Current streaming thinking/reasoning being accumulated.
    pub current_thinking: Option<String>,
```

- [ ] **Step 2: Initialize in constructor**

In `src/tui/app.rs`, add near `current_stream: None` (around line 269):
```rust
            current_thinking: None,
```

- [ ] **Step 3: Add thinking colors to Theme**

In `src/tui/theme.rs`, add after `tool_err_dim`:
```rust
    /// Color for thinking/reasoning text.
    pub(crate) fn thinking(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(255, 200, 100),
            Theme::Nord => Color::Rgb(235, 203, 139),
            Theme::Dracula => Color::Rgb(241, 250, 140),
            Theme::Solarized => Color::Rgb(181, 137, 0),
        }
    }

    /// Dimmed variant of `thinking` for finalized messages.
    pub(crate) fn thinking_dim(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(160, 120, 50),
            Theme::Nord => Color::Rgb(140, 120, 70),
            Theme::Dracula => Color::Rgb(150, 150, 70),
            Theme::Solarized => Color::Rgb(110, 80, 0),
        }
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 7: TUI events — handle AgentThinkingChunk with show_thinking

**Files:**
- Modify: `src/tui/events.rs:56-71`
- Modify: `src/tui/events.rs:72-90`
- Modify: `src/tui/commands.rs:430-440`

**Interfaces:**
- Consumes: `EngineEvent::AgentThinkingChunk`, `App.current_thinking`, `App.show_thinking`
- Produces: Updated `current_thinking` field

- [ ] **Step 1: Handle `AgentThinkingChunk` in event loop**

In `src/tui/events.rs`, add a new match arm after `AgentStreamChunk` (around line 71):
```rust
            EngineEvent::AgentThinkingChunk { content, .. } => {
                if self.show_thinking {
                    let thinking = self.current_thinking.get_or_insert_with(String::new);
                    thinking.push_str(&content);
                }
            }
```

- [ ] **Step 2: Clear `current_thinking` on `AgentOutput`**

In `src/tui/events.rs`, in the `AgentOutput` handler (around line 77), add at the beginning:
```rust
                self.current_thinking = None;
                // ... existing code
                let final_content = self.current_stream.take().unwrap_or(content);
```

- [ ] **Step 3: Clear `current_thinking` on Escape key**

In `src/tui/keys.rs`, find where `self.current_stream = None` is set on Escape (around line 451), and add:
```rust
                self.current_thinking = None;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 8: TUI rendering — show thinking with distinct style

**Files:**
- Modify: `src/tui/render.rs:1064-1098`

**Interfaces:**
- Consumes: `App.current_thinking`, `App.show_thinking`, `Theme::thinking()`, `Theme::thinking_dim()`
- Produces: Rendered thinking block in chat with yellow/amber styling

- [ ] **Step 1: Render thinking block before stream**

In `src/tui/render.rs`, in the `render_chat` function, after the message loop (around line 1064) and before the stream block, add:
```rust
    // Add thinking/reasoning block if active (only if show_thinking is true)
    if let Some(thinking) = &app.current_thinking
        && !thinking.is_empty()
        && app.show_thinking
    {
        let thinking_color = app.theme.thinking();
        let mut first = true;
        for line_text in thinking.split('\n') {
            let prefix = if first {
                "\u{258c} "
            } else {
                "  "
            };
            first = false;
            let span = Span::styled(
                format!("{}{}", prefix, line_text),
                Style::default()
                    .fg(thinking_color)
                    .add_modifier(Modifier::DIM),
            );
            lines.push(Line::from(vec![
                Span::styled("▐ ", Style::default().fg(app.theme.thinking_dim())),
                span,
            ]));
        }
        // Add a blank separator line after thinking block
        lines.push(Line::from(Span::styled(
            "▐",
            Style::default().fg(app.theme.thinking_dim()),
        )));
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Clean compilation.

---

### Task 9: Verify — fmt, clippy, test

**Files:** None (verification only)

- [ ] **Step 1: Format check**

Run: `cargo fmt --check`
Expected: No formatting changes needed.

- [ ] **Step 2: Clippy**

Run: `cargo clippy 2>&1`
Expected: No new warnings.

- [ ] **Step 3: Tests**

Run: `cargo test 2>&1 | tail -5`
Expected: All 428+ tests pass.