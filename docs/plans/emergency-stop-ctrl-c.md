# Emergency Stop (Ctrl+C) — Implementation Plan

> **For agentic workers:** Read this entire plan before touching any file. Each task is bite-sized (2–5 min per step). Build, test, and commit after each task. Do not skip steps.

**Goal:** Pressing Ctrl+C stops all in-flight agent activity (LLM generation, tool execution) without exiting the app. The user sees a "⏹ Stopped" message in the chat. `ClearInput` moves to Ctrl+Z.

**Architecture:** A two-phase cancellation: (1) the TUI sends `EngineCommand::StopAgent` to the engine, (2) the engine sends `AgentMessage::Cancel` to the root agent, which sets an `Arc<AtomicBool>` flag checked before each LLM call and tool execution. The agent loop breaks out of the tool loop and returns to idle.

**Tech Stack:** Rust 2024, tokio, ratatui, crossterm, `Arc<AtomicBool>` for cancellation signaling.

## Global Constraints

- `cargo fmt --check && cargo clippy && cargo test` must pass after each task.
- All new public types must have doc comments.
- The app must NOT exit on Ctrl+C — only cancel the current operation.
- `ClearInput` (clear input buffer) moves from Ctrl+C to Ctrl+Z.

---

### Task 1: Add `Action::EmergencyStop` + rebind keys

**Files:**
- Modify: `src/tui/keymap.rs:10-120` (Action enum)
- Modify: `src/tui/keymap.rs:272` (ClearInput binding)
- Modify: `src/tui/keymap.rs:414` (add EmergencyStop binding)

**Interfaces:**
- Consumes: `Action` enum
- Produces: `Action::EmergencyStop` variant, new key bindings

- [ ] **Step 1: Add `EmergencyStop` variant to `Action` enum**
  Insert after `ClearInput` (line 56):
  ```rust
      /// Emergency stop: cancel all in-flight agent activity (Ctrl+C).
      EmergencyStop,
  ```

- [ ] **Step 2: Move `ClearInput` from Ctrl+C to Ctrl+Z**
  Change line 272 from:
  ```rust
  km.bind(Action::ClearInput, vec![key_event('c', true)]);
  ```
  to:
  ```rust
  km.bind(Action::ClearInput, vec![key_event('z', true)]);
  ```

- [ ] **Step 3: Bind `EmergencyStop` to Ctrl+C**
  Insert after the `ClearInput` binding:
  ```rust
  km.bind(Action::EmergencyStop, vec![key_event('c', true)]);
  ```

- [ ] **Step 4: Build and test**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

---

### Task 2: Add `EngineCommand::StopAgent` + `AgentMessage::Cancel`

**Files:**
- Modify: `src/engine/events.rs:289-392` (EngineCommand enum)
- Modify: `src/agent/types.rs:144-182` (AgentMessage enum)

**Interfaces:**
- Consumes: `EngineCommand` enum, `AgentMessage` enum
- Produces: `EngineCommand::StopAgent`, `AgentMessage::Cancel`

- [ ] **Step 1: Add `StopAgent` to `EngineCommand`**
  Insert before `Shutdown` (around line 390):
  ```rust
      /// Emergency stop: cancel all in-flight agent activity.
      StopAgent,
  ```

- [ ] **Step 2: Add `Cancel` to `AgentMessage`**
  Insert before `Shutdown` (around line 180):
  ```rust
      /// Emergency stop signal — cancel current operation and return to idle.
      Cancel,
  ```

- [ ] **Step 3: Build and test**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

---

### Task 3: Implement agent cancellation (cancel flag + check points)

**Files:**
- Modify: `src/agent/lifecycle.rs:137-1052` (spawn_agent function)

**Interfaces:**
- Consumes: `AgentMessage::Cancel`, `Arc<AtomicBool>`
- Produces: Cancellation check points in the tool loop

- [ ] **Step 1: Add cancel flag to the agent task**
  After `let debug_mode = debug;` (line 234), add:
  ```rust
      let cancel_flag = Arc::new(AtomicBool::new(false));
  ```

- [ ] **Step 2: Handle `AgentMessage::Cancel` in the message loop**
  In the `while let Some(msg) = rx.recv().await` loop (line 291), add before the `AgentMessage::Shutdown` handler (line 1017):
  ```rust
                  AgentMessage::Cancel => {
                      cancel_flag.store(true, Ordering::Relaxed);
                      // Emit status: Idle so the TUI knows we stopped
                      let _ = event_tx
                          .send(EngineEvent::AgentStatusChanged {
                              agent_id: agent_id.clone(),
                              agent_name: agent_name.clone(),
                              status: AgentStatus::Idle,
                          })
                          .await;
                  }
  ```

- [ ] **Step 3: Check cancel flag before each LLM call**
  At the top of the `'tool_loop` (after line 340, before `step_count += 1`), add:
  ```rust
                          if cancel_flag.load(Ordering::Relaxed) {
                              cancel_flag.store(false, Ordering::Relaxed);
                              let _ = event_tx
                                  .send(EngineEvent::AgentStatusChanged {
                                      agent_id: agent_id.clone(),
                                      agent_name: agent_name.clone(),
                                      status: AgentStatus::Idle,
                                  })
                                  .await;
                              break 'tool_loop;
                          }
  ```

- [ ] **Step 4: Check cancel flag before each tool execution**
  At the top of the `execute_one` closure (after line 596), add:
  ```rust
                                      if cancel_flag.load(Ordering::Relaxed) {
                                          return (
                                              tc.id.clone(),
                                              "[Cancelled] Operation stopped by user".to_string(),
                                          );
                                      }
  ```

- [ ] **Step 5: Build and test**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

---

### Task 4: Wire `StopAgent` in engine orchestrator

**Files:**
- Modify: `src/engine/orchestrator.rs:465-657` (Engine::run loop)

**Interfaces:**
- Consumes: `EngineCommand::StopAgent`
- Produces: Sends `AgentMessage::Cancel` to root agent

- [ ] **Step 1: Add `StopAgent` handler in the `run()` loop**
  Insert before `EngineCommand::Shutdown` (around line 616):
  ```rust
                              EngineCommand::StopAgent => {
                                  self.send_to_active(AgentMessage::Cancel).await?;
                              }
  ```

- [ ] **Step 2: Build and test**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

---

### Task 5: Wire `EmergencyStop` in TUI keys.rs

**Files:**
- Modify: `src/tui/keys.rs:344-463` (keymap dispatch section)

**Interfaces:**
- Consumes: `Action::EmergencyStop`
- Produces: Sends `EngineCommand::StopAgent`, pushes "⏹ Stopped" message, clears stream, shows toast

- [ ] **Step 1: Add `EmergencyStop` handler in `handle_key()`**
  Insert after the `ToggleSearch` block (around line 444):
  ```rust
              if self.keymap.matches(key_event, Action::EmergencyStop) {
                  // Send stop command to engine
                  let _ = self.cmd_tx.try_send(EngineCommand::StopAgent);
                  // Push a visual confirmation message
                  self.push_msg("⏹ Stopped");
                  // Clear any in-progress streaming response
                  self.current_stream = None;
                  self.stream_committed_index = None;
                  // Show a toast notification
                  self.toasts.push("⏹ Stopped", ToastKind::Info);
                  return;
              }
  ```

- [ ] **Step 2: Build and test**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

---

### Task 6: Build, test, commit

- [ ] **Step 1: Full build and lint**
  ```sh
  cargo fmt --check && cargo clippy && cargo test
  ```

- [ ] **Step 2: Commit**
  ```sh
  git add -A && git commit -m "✨ feat: emergency stop (Ctrl+C) cancels in-flight agent activity

  - Add Action::EmergencyStop bound to Ctrl+C
  - Move ClearInput from Ctrl+C to Ctrl+Z
  - Add EngineCommand::StopAgent and AgentMessage::Cancel
  - Add Arc<AtomicBool> cancel flag checked before LLM calls and tool execution
  - Wire EmergencyStop in TUI to send stop command and show visual feedback"
  ```