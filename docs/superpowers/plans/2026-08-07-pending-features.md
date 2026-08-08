# Pending Features — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete all remaining features for Anacleto v1.0: CI/CD, streaming subagents, headless mode, config hot-reload, file logging, and TUI history search.

**Architecture:** Five independent phases, each producing a testable deliverable. Phase 1 is infrastructure (CI/CD). Phases 2–5 are feature work. They can be executed in any order.

**Tech Stack:** Rust 2024 edition, tokio, ratatui, sqlx, serde, reqwest, tower, clap, tracing, tracing-appender (new), futures, signal-hook (new)

## Global Constraints

- Rust edition 2024, rustc ≥ 1.85 (current: 1.97.0)
- No new dependencies unless explicitly listed in a task
- `cargo fmt --check` must pass before commits
- `cargo clippy` must pass before commits (currently 0 warnings ✅)
- All existing tests (385+) must continue to pass after each change
- CI/CD uses GitHub Actions only
- TUI is the default interface; headless mode is a CLI flag

---

## Phase 1: Profesionalización (CI/CD)

**Goal:** CI/CD pipeline with GitHub Actions.

**Files created:**
- `.github/workflows/ci.yml`

### Task 1.1: Create GitHub Actions CI/CD pipeline

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Create the directory**

  ```bash
  mkdir -p .github/workflows
  ```

- [ ] **Step 2: Write `.github/workflows/ci.yml`**

  ```yaml
  name: CI

  on:
    push:
      branches: [main, development]
    pull_request:
      branches: [main, development]

  env:
    CARGO_TERM_COLOR: always

  jobs:
    fmt:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - run: cargo fmt --check

    clippy:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo clippy -- -D warnings

    build:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo build --verbose

    test:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
        - uses: Swatinem/rust-cache@v2
        - run: cargo test --verbose
  ```

- [ ] **Step 3: Verify the workflow file is valid**

  ```bash
  # At minimum, check YAML syntax
  python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"
  ```
  Expected: no error

- [ ] **Step 4: Commit**

  ```bash
  git add .github/workflows/ci.yml
  git commit -m "ci: add GitHub Actions CI pipeline"
  ```

---

## Phase 2: Streaming en subagentes

**Goal:** Subagents currently use `prov.complete()` (non-streaming) at `src/agent/tools.rs:1536`. Change to `prov.complete_stream()` so subagent LLM responses are streamed to the TUI in real time, matching the parent agent behavior.

**Files modified:**
- `src/agent/tools.rs:1506-1559` (subagent LLM call loop)

**Key insight:** The subagent's `stream: false` in `LlmRequest` (line 1512) must become `stream: true`, and the `complete()` call (line 1536) must become `complete_stream()` with a chunk-receiving loop that emits `EngineEvent::AgentStreamChunk` events.

### Task 2.1: Change subagent LLM call from `complete()` to `complete_stream()`

**Files:**
- Modify: `src/agent/tools.rs:1506-1559`

- [ ] **Step 1: Change `stream: false` to `stream: true` in the subagent request**

  Find line 1512:
  ```rust
  stream: false, // non-streaming for subagents
  ```
  Change to:
  ```rust
  stream: true,
  ```

- [ ] **Step 2: Replace the `complete()` call with a streaming loop**

  Find lines 1529-1559:
  ```rust
  // Wrap subagent LLM call with retries
  let sub_retry_cfg = subagent_retry_config.clone();
  let sub_agent_name = agent_name.clone();
  let complete_result = retry_with_backoff(
      |_attempt| {
          let req = request.clone();
          let prov = provider.clone();
          async move { prov.complete(req).await }
      },
      &sub_retry_cfg,
      &format!("Subagent LLM call for '{}'", sub_agent_name),
  )
  .await;

  match complete_result {
      Ok(response) => {
          // Emit token usage if available
          if let Some(ref usage) = response.usage {
              let cost = (usage.prompt_tokens as f64
                  * provider.input_price_per_million()
                  + usage.completion_tokens as f64 * provider.output_price_per_million())
                  / 1_000_000.0;
              let _ = event_tx
                  .send(EngineEvent::TokenUsage {
                      agent_id: agent_id.clone(),
                      agent_name: agent_name.clone(),
                      total_tokens: usage.total_tokens,
                      context_window: provider.context_window() as u32,
                      cost,
                  })
                  .await;
          }

          let response_text = response.content;
          // ... (rest of match arm)
      }
      Err(e) => { ... }
  }
  ```

  Change to:
  ```rust
  // Wrap subagent LLM call with retries (streaming)
  let sub_retry_cfg = subagent_retry_config.clone();
  let sub_agent_name = agent_name.clone();
  let sub_agent_id = agent_id.clone();
  let sub_event_tx = event_tx.clone();
  let complete_result = retry_with_backoff(
      |_attempt| {
          let req = request.clone();
          let prov = provider.clone();
          async move { prov.complete_stream(req).await }
      },
      &sub_retry_cfg,
      &format!("Subagent LLM stream call for '{}'", sub_agent_name),
  )
  .await;

  match complete_result {
      Ok(mut stream_rx) => {
          let mut full_response = String::new();
          let mut tool_calls: Vec<ToolCall> = Vec::new();
          let mut stream_error: Option<String> = None;

          // Collect all chunks from the stream
          while let Some(chunk) = stream_rx.recv().await {
              match chunk {
                  Ok(LlmStreamChunk::Content(text)) => {
                      full_response.push_str(&text);
                      let _ = sub_event_tx
                          .send(EngineEvent::AgentStreamChunk {
                              agent_id: sub_agent_id.clone(),
                              agent_name: sub_agent_name.clone(),
                              content: text,
                          })
                          .await;
                  }
                  Ok(LlmStreamChunk::ToolCall(tc)) => {
                      tool_calls.push(tc);
                  }
                  Ok(LlmStreamChunk::Done(usage)) => {
                      // Emit token usage
                      if let Some(usage) = usage {
                          let cost = (usage.prompt_tokens as f64
                              * provider.input_price_per_million()
                              + usage.completion_tokens as f64 * provider.output_price_per_million())
                              / 1_000_000.0;
                          let _ = sub_event_tx
                              .send(EngineEvent::TokenUsage {
                                  agent_id: sub_agent_id.clone(),
                                  agent_name: sub_agent_name.clone(),
                                  total_tokens: usage.total_tokens,
                                  context_window: provider.context_window() as u32,
                                  cost,
                              })
                              .await;
                      }
                  }
                  Err(e) => {
                      stream_error = Some(e.to_string());
                  }
              }
          }

          if let Some(err) = stream_error {
              // Handle stream error
              let _ = response_tx.send(format!("[Error en subagente] {}", err));
              return;
          }

          let response_text = full_response;
          // ... (rest of the original code that uses response_text, unchanged)
      }
      Err(e) => { ... }
  }
  ```

  > **Note:** The `use` imports for `LlmStreamChunk` must be checked. If not already imported in `tools.rs`, add:
  > ```rust
  > use crate::llm::types::LlmStreamChunk;
  > ```

- [ ] **Step 3: Verify compiles and clippy passes**

  ```bash
  cargo clippy 2>&1
  ```
  Expected: no warnings

- [ ] **Step 4: Run tests**

  ```bash
  cargo test 2>&1 | tail -5
  ```
  Expected: all tests pass (385+ passed)

- [ ] **Step 5: Commit**

  ```bash
  git add src/agent/tools.rs
  git commit -m "feat: stream subagent LLM responses to TUI"
  ```

---

## Phase 3: Modo headless

**Goal:** Add a `--headless` CLI flag. When set, skip the TUI initialization and run the engine in a non-interactive mode, outputting agent responses to stdout.

**Files modified:**
- `src/main.rs` (CLI parsing + conditional TUI)
- `src/engine/orchestrator.rs` (maybe — headless output mode)

### Task 3.1: Add `--headless` CLI flag and conditional TUI startup

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add `headless` field to `Cli` struct**

  Find the `Cli` struct (around line 19):
  ```rust
  struct Cli {
      #[arg(short, long)]
      config: Option<String>,
      #[arg(short, long)]
      database: Option<String>,
      #[arg(short, long)]
      verbose: bool,
      #[arg(long)]
      debug: bool,
  }
  ```

  Add after `debug`:
  ```rust
  /// Run in headless mode (no TUI, output to stdout).
  #[arg(long)]
  headless: bool,
  ```

- [ ] **Step 2: Conditional TUI startup**

  Find the section after `engine.initialize().await?;` and before `Setup terminal` (around line 80):
  ```rust
  // Setup terminal
  let mut stdout = io::stdout();
  // ... (50+ lines of TUI setup)
  // Run engine and TUI concurrently
  ```

  Wrap the TUI setup and `run_tui` call in a conditional:
  ```rust
  if cli.headless {
      // Headless mode: run engine directly, output to stdout
      let _ = engine.run().await?;
  } else {
      // Setup terminal (existing code)
      let mut stdout = io::stdout();
      crossterm::terminal::enable_raw_mode()?;
      let kb_supported = crossterm::terminal::supports_keyboard_enhancement()?;
      let backend = CrosstermBackend::new(&mut stdout);
      let mut terminal = Terminal::new(backend)?;
      crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
      if kb_supported {
          crossterm::execute!(
              io::stdout(),
              PushKeyboardEnhancementFlags(
                  KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                      | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
              )
          )?;
      }

      // Run engine and TUI concurrently
      let engine_handle = tokio::spawn(async move {
          if let Err(e) = engine.run().await {
              eprintln!("Engine error: {}", e);
          }
      });

      let mut app = App::new(cmd_tx, event_rx, kb_supported, &config);
      let tui_result = run_tui(&mut terminal, &mut app).await;

      // Cleanup
      crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags)?;
      crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
      crossterm::terminal::disable_raw_mode()?;

      let _ = app.cmd_tx.try_send(EngineCommand::Shutdown);
      engine_handle.await.ok();

      tui_result.map_err(|e| anyhow::anyhow!("TUI error: {}", e))?;
  }
  ```

- [ ] **Step 3: Verify compiles**

  ```bash
  cargo build 2>&1
  ```
  Expected: successful build

- [ ] **Step 4: Test the headless mode starts**

  ```bash
  cargo run -- --headless 2>&1 &
  sleep 2
  kill %1 2>/dev/null
  ```
  Expected: should start and shut down without TUI errors

- [ ] **Step 5: Commit**

  ```bash
  git add src/main.rs
  git commit -m "feat: add headless mode (--headless CLI flag)"
  ```

---

## Phase 4: Config hot-reload (SIGHUP)

**Goal:** On SIGHUP, reload the YAML config from disk and update the engine's config in-memory without restarting. The engine must merge the new config gracefully.

**Files modified:**
- `Cargo.toml` — add `signal-hook` or use `tokio::signal::unix`
- `src/main.rs` — SIGHUP listener
- `src/engine/orchestrator.rs` — config reload method

### Task 4.1: Add SIGHUP signal handler and config reload

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Modify: `src/engine/orchestrator.rs`

- [ ] **Step 1: Check if `tokio::signal` already supports SIGHUP**

  ```bash
  grep -r "signal" Cargo.toml
  ```
  If `tokio` does not include the `signal` feature, add it to `Cargo.toml`:
  ```toml
  tokio = { version = "1", features = ["full", "signal"] }
  ```

- [ ] **Step 2: Add config reload method to Engine**

  In `src/engine/orchestrator.rs`, add a public method:
  ```rust
  /// Reload configuration from disk. Called on SIGHUP.
  pub fn reload_config(&mut self, config: Config) {
      self.config = config;
      // Propagate any config changes that affect running state
      // (e.g., session settings, MCP definitions)
      tracing::info!("Configuration reloaded from disk");
  }
  ```

- [ ] **Step 3: Add SIGHUP listener in main.rs**

  In `src/main.rs`, after the engine is initialized and before the TUI setup, add:
  ```rust
  // SIGHUP handler for config hot-reload
  let mut sighup_stream = tokio::signal::unix::signal(
      tokio::signal::unix::SignalKind::hangup(),
  )?;
  let reload_cmd_tx = cmd_tx.clone();
  tokio::spawn(async move {
      loop {
          sighup_stream.recv().await;
          tracing::info!("Received SIGHUP, reloading config...");
          let _ = reload_cmd_tx
              .send(EngineCommand::ReloadConfig)
              .await;
      }
  });
  ```

- [ ] **Step 4: Add `ReloadConfig` variant to `EngineCommand`**

  In `src/engine/events.rs`, find the `EngineCommand` enum and add:
  ```rust
  /// Reload configuration from disk (triggered by SIGHUP).
  ReloadConfig,
  ```

- [ ] **Step 5: Handle `ReloadConfig` in the engine event loop**

  In `src/engine/orchestrator.rs`, find the command handler loop and add a match arm:
  ```rust
  EngineCommand::ReloadConfig => {
      match crate::config::loader::load_config(None) {
          Ok(new_config) => {
              self.reload_config(new_config);
              let _ = self.event_tx.send(EngineEvent::ConfigReloaded).await;
          }
          Err(e) => {
              tracing::error!("Failed to reload config: {}", e);
          }
      }
  }
  ```

- [ ] **Step 6: Add `ConfigReloaded` event**

  In `src/engine/events.rs`, find the `EngineEvent` enum and add:
  ```rust
  /// Configuration was reloaded from disk.
  ConfigReloaded,
  ```

- [ ] **Step 7: Verify compiles**

  ```bash
  cargo clippy 2>&1
  ```
  Expected: no warnings

- [ ] **Step 8: Commit**

  ```bash
  git add Cargo.toml src/main.rs src/engine/orchestrator.rs src/engine/events.rs
  git commit -m "feat: add config hot-reload on SIGHUP"
  ```

---

## Phase 5: Logs a archivo

**Goal:** Add a file-based tracing subscriber alongside the existing stdout subscriber. Logs are written to `~/.local/share/anacleto/logs/` with daily rotation.

**Files modified:**
- `Cargo.toml` — add `tracing-appender`
- `src/main.rs` — initialize file logging

### Task 5.1: Add file logging with tracing-appender

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`

- [ ] **Step 1: Add `tracing-appender` dependency**

  ```toml
  tracing-appender = "0.2"
  ```

- [ ] **Step 2: Initialize file logging in main.rs**

  Find the existing `tracing_subscriber::fmt()` initialization (around line 55):
  ```rust
  // Initialize logging
  tracing_subscriber::fmt()
      .with_env_filter(if cli.verbose {
          "anacleto=debug"
      } else {
          "anacleto=info"
      })
      .init();
  ```

  Replace with:
  ```rust
  use tracing_subscriber::prelude::*;

  // Initialize logging
  let log_filter = if cli.verbose {
      "anacleto=debug"
  } else {
      "anacleto=info"
  };

  // File appender with daily rotation
  let log_dir = dirs::data_dir()
      .unwrap_or_else(|| std::path::PathBuf::from("."))
      .join("anacleto")
      .join("logs");
  std::fs::create_dir_all(&log_dir).ok();
  let file_appender = tracing_appender::rolling::daily(log_dir, "anacleto.log");
  let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

  tracing_subscriber::fmt()
      .with_env_filter(log_filter)
      .with_writer(non_blocking)
      .with_ansi(false) // file output should not have ANSI codes
      .init();
  ```

  > **Note:** If `dirs` is not a dependency, use `std::env::temp_dir()` or `~/.local/share/anacleto/logs` directly. Check if `dirs` crate is available.

- [ ] **Step 3: Verify compiles**

  ```bash
  cargo build 2>&1
  ```
  Expected: successful build

- [ ] **Step 4: Verify log file is created**

  ```bash
  cargo run -- --headless 2>&1 &
  sleep 2
  kill %1 2>/dev/null
  ls -la ~/.local/share/anacleto/logs/ 2>/dev/null || echo "Check log dir"
  ```
  Expected: log file exists with content

- [ ] **Step 5: Commit**

  ```bash
  git add Cargo.toml src/main.rs
  git commit -m "feat: add file logging with daily rotation"
  ```

---

## Phase 6: History search en TUI (Ctrl+R style)

**Goal:** Add a search overlay to the TUI triggered by Ctrl+R. The user types a query and the conversation history is filtered to show matching messages. Matches are highlighted.

**Files modified:**
- `src/tui/commands.rs` — Ctrl+R handler
- `src/tui/app.rs` — search state
- `src/tui/types.rs` — search-related types
- `src/tui/events.rs` — search key handling
- `src/tui/render.rs` — search overlay rendering

### Task 6.1: Add search state and types

**Files:**
- Modify: `src/tui/types.rs`

- [ ] **Step 1: Add `SearchState` struct and `SearchMode` enum**

  In `src/tui/types.rs`, add:
  ```rust
  /// State for the conversation history search overlay.
  #[derive(Debug, Clone, Default)]
  pub struct SearchState {
      /// Whether the search overlay is visible.
      pub visible: bool,
      /// The current search query.
      pub query: String,
      /// Cursor position within the query.
      pub cursor: usize,
      /// Indices of matching messages in the conversation.
      pub matches: Vec<usize>,
      /// Currently selected match index.
      pub selected: usize,
  }

  /// Mode of the search overlay.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum SearchMode {
      /// Searching through conversation history.
      History,
  }
  ```

- [ ] **Step 2: Add `search` field to `App` or relevant state struct**

  Find the main app state struct (likely `App` in `src/tui/app.rs` or `src/tui/state.rs`). Add:
  ```rust
  /// Search overlay state.
  pub search: SearchState,
  ```

  Initialize it as `SearchState::default()`.

### Task 6.2: Add Ctrl+R binding and search handler

**Files:**
- Modify: `src/tui/keymap.rs` — add Ctrl+R binding
- Modify: `src/tui/commands.rs` — handle search commands
- Modify: `src/tui/events.rs` — route Ctrl+R to search

- [ ] **Step 1: Add Ctrl+R binding**

  In `src/tui/keymap.rs`, find the keybindings definition and add:
  ```rust
  // Search history
  bindings.insert(
      KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
      AppAction::ToggleSearch,
  );
  ```

  If `AppAction` does not have `ToggleSearch`, add it:
  ```rust
  /// Toggle the conversation search overlay.
  ToggleSearch,
  ```

- [ ] **Step 2: Handle `ToggleSearch` action**

  In `src/tui/events.rs` or wherever actions are dispatched, add:
  ```rust
  AppAction::ToggleSearch => {
      app.search.visible = !app.search.visible;
      if app.search.visible {
          app.search.query.clear();
          app.search.matches.clear();
          app.search.selected = 0;
      }
  }
  ```

- [ ] **Step 3: Handle search input when overlay is active**

  In the key event handler, when `app.search.visible` is true, intercept typing:
  ```rust
  if app.search.visible {
      match key.code {
          KeyCode::Esc => {
              app.search.visible = false;
          }
          KeyCode::Enter => {
              // Jump to selected match
              if let Some(idx) = app.search.matches.get(app.search.selected) {
                  app.scroll_to_message(*idx);
              }
              app.search.visible = false;
          }
          KeyCode::Char(c) => {
              app.search.query.push(c);
              app.search.cursor = app.search.query.len();
              // Update matches
              app.search.matches = app.conversation.iter()
                  .enumerate()
                  .filter(|(_, msg)| msg.content.to_lowercase().contains(&app.search.query.to_lowercase()))
                  .map(|(i, _)| i)
                  .collect();
              app.search.selected = 0;
          }
          KeyCode::Backspace => {
              app.search.query.pop();
              app.search.cursor = app.search.query.len();
              // Re-filter
              app.search.matches = app.conversation.iter()
                  .enumerate()
                  .filter(|(_, msg)| msg.content.to_lowercase().contains(&app.search.query.to_lowercase()))
                  .map(|(i, _)| i)
                  .collect();
              app.search.selected = 0;
          }
          KeyCode::Up => {
              if !app.search.matches.is_empty() {
                  app.search.selected = app.search.selected.saturating_sub(1);
              }
          }
          KeyCode::Down => {
              if app.search.selected + 1 < app.search.matches.len() {
                  app.search.selected += 1;
              }
          }
          _ => {}
      }
      return;
  }
  ```

### Task 6.3: Render search overlay

**Files:**
- Modify: `src/tui/render.rs`

- [ ] **Step 1: Render search overlay when visible**

  In the render function, after rendering the main chat area but before flushing, add:
  ```rust
  // Render search overlay
  if app.search.visible {
      let area = centered_rect(60, 8, f.size());
      let overlay = Block::default()
          .title(" Search History (Ctrl+R) ")
          .borders(Borders::ALL)
          .border_style(Style::default().fg(Color::Cyan));
      let inner = overlay.inner(area);
      f.render_widget(overlay, area);

      // Search query input
      let query = Paragraph::new(format!("> {}", app.search.query))
          .style(Style::default().fg(Color::White))
          .block(Block::default().borders(Borders::ALL).title("Query"));
      f.render_widget(query, inner);

      // Match count
      let match_count = format!(
          "{} match(es)",
          app.search.matches.len()
      );
      f.render_widget(
          Paragraph::new(match_count).style(Style::default().fg(Color::DarkGray)),
          Rect::new(inner.x, inner.y + 3, inner.width, 1),
      );
  }
  ```

  Add a helper function if not already present:
  ```rust
  /// Create a centered rectangle within the given area.
  fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
      let popup_layout = Layout::vertical([
          Constraint::Length((r.height * (100 - percent_y)) / 200),
          Constraint::Length((r.height * percent_y) / 100),
          Constraint::Length((r.height * (100 - percent_y)) / 200),
      ])
      .split(r);
      Layout::horizontal([
          Constraint::Length((r.width * (100 - percent_x)) / 200),
          Constraint::Length((r.width * percent_x) / 100),
          Constraint::Length((r.width * (100 - percent_x)) / 200),
      ])
      .split(popup_layout[1])[1]
  }
  ```

- [ ] **Step 2: Verify compiles**

  ```bash
  cargo clippy 2>&1
  ```
  Expected: no warnings

- [ ] **Step 3: Commit**

  ```bash
  git add src/tui/keymap.rs src/tui/commands.rs src/tui/events.rs src/tui/types.rs src/tui/render.rs src/tui/app.rs
  git commit -m "feat: add history search overlay (Ctrl+R) in TUI"
  ```

---

## Criterios de aceptación

- [ ] `cargo fmt --check` pasa sin cambios
- [ ] `cargo clippy` pasa sin warnings (0 generados)
- [ ] `cargo test` pasa (385+ passed, 0 failed)
- [ ] CI pipeline ejecuta fmt, clippy, build, test en cada push/PR
- [ ] Subagentes transmiten respuesta en tiempo real a la TUI
- [ ] `--headless` arranca sin TUI y escribe respuesta a stdout
- [ ] `kill -HUP <pid>` recarga la configuración sin reiniciar
- [ ] Logs de anacleto se escriben a `~/.local/share/anacleto/logs/anacleto.log`
- [ ] Ctrl+R abre overlay de búsqueda, filtrado y navegación por resultados