# Edit Agent/SubAgent Dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Ctrl+E dialog in the TUI to edit agent/subagent skills, MCPs and subagents.

**Architecture:** Add `EditDialogState` type, an `EngineCommand::UpdateAgentConfig` variant, and integrate into the existing TUI key handling and rendering patterns (ratatui popup overlay).

**Tech Stack:** Rust, ratatui, crossterm, tokio

## Global Constraints

- Follow existing TUI patterns (see `pending_question`, `QuestionState`, `render_question_dialog`)
- Ctrl+E is currently bound to `Action::CursorEnd` (input editing) and `Action::OpenEditor` — those keymap-based actions are only triggered via `keymap_applies()`. The new Ctrl+E handling is done **before** the keymap dispatch, captured in the panel-specific handlers when the edit dialog is not already visible.
- No new dependencies

---

### Task 1: Add `EditDialogState` type and `EngineCommand` variant

**Files:**
- Modify: `src/tui/types.rs:111-130` (after `QuestionState`)
- Modify: `src/engine/events.rs:395` (append to `EngineCommand`)

**Interfaces:**
- Consumes: `AgentInfo` (from `types.rs`)
- Produces: `EditDialogState` struct, `EngineCommand::UpdateAgentConfig` variant

- [x] **Step 1: Add `EditDialogState` to `tui/types.rs`**

Add after `QuestionState`:

```rust
/// State for the Ctrl+E edit-agent/subagent dialog.
pub(crate) struct EditDialogState {
    /// Whether the dialog is visible.
    pub visible: bool,
    /// Name of the agent or subagent being edited.
    pub target_name: String,
    /// Whether this is a root agent (shows subagents section).
    pub is_root: bool,
    /// All available skill names (union across agents).
    pub all_skills: Vec<String>,
    /// Which skills are currently enabled for the target.
    pub skills_enabled: Vec<bool>,
    /// All available MCP names (union across agents).
    pub all_mcps: Vec<String>,
    /// Which MCPs are currently enabled for the target.
    pub mcps_enabled: Vec<bool>,
    /// All available subagent names for root agents.
    pub all_subagents: Vec<String>,
    /// Which subagents are currently enabled for the target.
    pub subagents_enabled: Vec<bool>,
    /// Currently focused section (0 = Skills, 1 = MCPs, 2 = SubAgents — only for root).
    pub section: usize,
    /// Currently selected index within the section.
    pub index: usize,
}
```

- [x] **Step 2: Add `UpdateAgentConfig` variant to `EngineCommand`**

In `src/engine/events.rs`, add after `ReloadConfig`:

```rust
/// Update the configuration of an agent/subagent (skills, mcps, subagents).
UpdateAgentConfig {
    name: String,
    skills: Vec<String>,
    mcps: Vec<String>,
    /// Only for root agents: the list of configured subagents.
    subagents: Option<Vec<String>>,
},
```

- [x] **Step 3: Build check**

Run: `cargo build 2>&1 | head -20`
Expected: Build succeeds (new types are defined but not yet used, so no errors).

- [x] **Step 4: Commit**

```bash
git add src/tui/types.rs src/engine/events.rs
git commit -m "feat: add EditDialogState and UpdateAgentConfig command"
```

---

### Task 2: Wire up edit dialog in App state

**Files:**
- Modify: `src/tui/app.rs` — add field, init in `new()`, add open/close helpers

**Interfaces:**
- Consumes: `EditDialogState`, `AgentInfo`, `EngineCommand`
- Produces: `App::open_edit_dialog(target_info, is_root, configured_subagents)` method

- [ ] **Step 1: Add `edit_dialog` field to `App` struct**

After line `pub(crate) search: SearchState,`:

```rust
/// Ctrl+E edit-agent/subagent dialog state.
pub(crate) edit_dialog: EditDialogState,
```

- [ ] **Step 2: Initialize in `App::new()`**

After `search: SearchState::default(),`:

```rust
edit_dialog: EditDialogState::new(),
```

- [ ] **Step 3: Add helper methods to `App`**

Before the `#[cfg(test)]` module:

```rust
impl App {
    /// Open the edit dialog for a given agent/subagent.
    pub(crate) fn open_edit_dialog(
        &mut self,
        target_name: String,
        is_root: bool,
        skills: &[String],
        mcps: &[String],
        subagents: Option<&[String]>,
    ) {
        // Collect all unique skills across all agents
        let all_skills: Vec<String> = {
            let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for agent in &self.agents {
                for s in &agent.skills {
                    set.insert(s);
                }
            }
            set.into_iter().map(String::from).collect()
        };

        let skills_enabled: Vec<bool> = all_skills.iter().map(|s| skills.contains(s)).collect();

        // Collect all unique MCPs across all agents
        let all_mcps: Vec<String> = {
            let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for agent in &self.agents {
                for m in &agent.mcps {
                    set.insert(m);
                }
            }
            set.into_iter().map(String::from).collect()
        };

        let mcps_enabled: Vec<bool> = all_mcps.iter().map(|m| mcps.contains(m)).collect();

        // Collect all unique subagent names from configured_subagents
        let all_subagents: Vec<String> = {
            let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for v in self.configured_subagents.values() {
                for s in v {
                    set.insert(s);
                }
            }
            set.into_iter().map(String::from).collect()
        };

        let subagents_enabled: Vec<bool> = if let Some(sa) = subagents {
            all_subagents.iter().map(|s| sa.contains(s)).collect()
        } else {
            vec![false; all_subagents.len()]
        };

        self.edit_dialog = EditDialogState::new_with(
            target_name,
            is_root,
            all_skills,
            skills_enabled,
            all_mcps,
            mcps_enabled,
            all_subagents,
            subagents_enabled,
        );
    }
}
```

- [ ] **Step 4: Build check**

Run: `cargo build 2>&1 | head -20`
Expected: Build succeeds.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat: wire edit_dialog field and open helpers in App"
```

---

### Task 3: Add `EditDialogState` methods + key handling

**Files:**
- Modify: `src/tui/types.rs` — add `new()` and `new_with()` constructors
- Modify: `src/tui/keys.rs` — add dialog key handling in main `handle_key`
- Modify: `src/tui/navigation.rs` — add Ctrl+E handling in info_panel and agent_panel

**Interfaces:**
- Consumes: `EditDialogState`, `KeyCode`, `KeyModifiers`
- Produces: Dialog navigation (toggle items, confirm/cancel)

- [ ] **Step 1: Add constructors and helpers to `EditDialogState`**

In `src/tui/types.rs`, after the `EditDialogState` struct definition:

```rust
impl EditDialogState {
    pub(crate) fn new() -> Self {
        Self {
            visible: false,
            target_name: String::new(),
            is_root: false,
            all_skills: Vec::new(),
            skills_enabled: Vec::new(),
            all_mcps: Vec::new(),
            mcps_enabled: Vec::new(),
            all_subagents: Vec::new(),
            subagents_enabled: Vec::new(),
            section: 0,
            index: 0,
        }
    }

    pub(crate) fn new_with(
        target_name: String,
        is_root: bool,
        all_skills: Vec<String>,
        skills_enabled: Vec<bool>,
        all_mcps: Vec<String>,
        mcps_enabled: Vec<bool>,
        all_subagents: Vec<String>,
        subagents_enabled: Vec<bool>,
    ) -> Self {
        Self {
            visible: true,
            target_name,
            is_root,
            all_skills,
            skills_enabled,
            all_mcps,
            mcps_enabled,
            all_subagents,
            subagents_enabled,
            section: 0,
            index: 0,
        }
    }

    /// The number of sections in this dialog (2 for subagents, 3 for root agents).
    pub(crate) fn section_count(&self) -> usize {
        if self.is_root { 3 } else { 2 }
    }

    /// The number of items in the current section.
    pub(crate) fn section_len(&self) -> usize {
        match self.section {
            0 => self.all_skills.len(),
            1 => self.all_mcps.len(),
            _ => self.all_subagents.len(),
        }
    }

    /// Toggle the currently selected item.
    pub(crate) fn toggle_current(&mut self) {
        let toggled = match self.section {
            0 => self.skills_enabled.get_mut(self.index),
            1 => self.mcps_enabled.get_mut(self.index),
            _ => self.subagents_enabled.get_mut(self.index),
        };
        if let Some(val) = toggled {
            *val = !*val;
        }
    }
}
```

- [ ] **Step 2: Add dialog key handling in `handle_key`**

In `src/tui/keys.rs`, add after the model_picker block (line ~258) and before the diff viewer block:

```rust
        // ── Edit agent/subagent dialog navigation ──────────────────
        if self.edit_dialog.visible {
            match key {
                KeyCode::Up => {
                    if self.edit_dialog.index > 0 {
                        self.edit_dialog.index -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = self.edit_dialog.section_len();
                    if len > 0 && self.edit_dialog.index + 1 < len {
                        self.edit_dialog.index += 1;
                    }
                }
                KeyCode::Left => {
                    if self.edit_dialog.section > 0 {
                        self.edit_dialog.section -= 1;
                        self.edit_dialog.index = 0;
                    }
                }
                KeyCode::Right => {
                    if self.edit_dialog.section + 1 < self.edit_dialog.section_count() {
                        self.edit_dialog.section += 1;
                        self.edit_dialog.index = 0;
                    }
                }
                KeyCode::Char(' ') => {
                    self.edit_dialog.toggle_current();
                }
                KeyCode::Enter => {
                    // Confirm: send changes to engine
                    let target_name = self.edit_dialog.target_name.clone();
                    let skills: Vec<String> = self.edit_dialog
                        .all_skills
                        .iter()
                        .zip(self.edit_dialog.skills_enabled.iter())
                        .filter(|(_, &enabled)| enabled)
                        .map(|(s, _)| s.clone())
                        .collect();
                    let mcps: Vec<String> = self.edit_dialog
                        .all_mcps
                        .iter()
                        .zip(self.edit_dialog.mcps_enabled.iter())
                        .filter(|(_, &enabled)| enabled)
                        .map(|(m, _)| m.clone())
                        .collect();
                    let subagents: Option<Vec<String>> = if self.edit_dialog.is_root {
                        Some(
                            self.edit_dialog
                                .all_subagents
                                .iter()
                                .zip(self.edit_dialog.subagents_enabled.iter())
                                .filter(|(_, &enabled)| enabled)
                                .map(|(s, _)| s.clone())
                                .collect()
                        )
                    } else {
                        None
                    };
                    let _ = self.cmd_tx.try_send(EngineCommand::UpdateAgentConfig {
                        name: target_name,
                        skills,
                        mcps,
                        subagents,
                    });
                    self.edit_dialog.visible = false;
                    self.toasts.push("Configuración actualizada", ToastKind::Success);
                }
                KeyCode::Esc => {
                    self.edit_dialog.visible = false;
                }
                _ => {}
            }
            return;
        }
```

- [ ] **Step 3: Add Ctrl+E handling in info_panel key handler**

In `src/tui/navigation.rs`, in `handle_info_panel_key`, add before the Left/Right check:

```rust
    // Ctrl+E opens the edit dialog for the selected subagent
    if key == KeyCode::Char('e') && modifiers == KeyModifiers::CONTROL {
        // Find the selected subagent in the unique list (info_tab == 2)
        if self.info_tab == 2 {
            let unique_subagents: Vec<&str> = {
                let set: std::collections::BTreeSet<&str> = self
                    .configured_subagents
                    .values()
                    .flat_map(|v| v.iter().map(|s| s.as_str()))
                    .collect();
                set.into_iter().collect()
            };
            if let Some(&name) = unique_subagents.get(self.subagent_panel_index) {
                // Find the first agent that has this subagent type to get its skills/MCPs
                let (skills, mcps) = self.agents
                    .iter()
                    .find(|a| a.agent_type.as_deref() == Some(name))
                    .map(|a| (a.skills.clone(), a.mcps.clone()))
                    .unwrap_or_default();
                self.open_edit_dialog(
                    name.to_string(),
                    false,  // is_root = false for subagents
                    &skills,
                    &mcps,
                    None,
                );
            }
        }
        return;
    }
```

- [ ] **Step 4: Add Ctrl+E handling in agent_panel key handler**

In `src/tui/navigation.rs`, in `handle_agent_panel_key`, add at the beginning:

```rust
    // Ctrl+E opens the edit dialog for the selected agent
    if key == KeyCode::Char('e') && modifiers == KeyModifiers::CONTROL {
        let display_agents: Vec<&AgentInfo> = self
            .agents
            .iter()
            .filter(|a| a.status != AgentStatus::Completed)
            .collect();
        if let Some(agent) = display_agents.get(self.agent_panel_index) {
            let subagents = if agent.role == AgentRole::Root {
                Some(
                    self.configured_subagents
                        .get(&agent.name)
                        .cloned()
                        .unwrap_or_default()
                )
            } else {
                None
            };
            self.open_edit_dialog(
                agent.name.clone(),
                agent.role == AgentRole::Root,
                &agent.skills,
                &agent.mcps,
                subagents.as_deref(),
            );
        }
        return;
    }
```

Note: Need to add `use crate::agent::types::AgentStatus;` import (may already be imported).

- [ ] **Step 5: Build check**

Run: `cargo build 2>&1 | head -30`
Expected: Build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/tui/types.rs src/tui/keys.rs src/tui/navigation.rs
git commit -m "feat: add edit dialog key handling with Ctrl+E trigger"
```

---

### Task 4: Render the edit dialog

**Files:**
- Modify: `src/tui/render.rs` — add `render_edit_dialog` function and call it from `render()`

**Interfaces:**
- Consumes: `App::edit_dialog`, `App::theme`
- Produces: ratatui popup with sections for skills, MCPs, subagents

- [ ] **Step 1: Add `render_edit_dialog` function**

In `src/tui/render.rs`, after `render_question_dialog` (line ~1900):

```rust
/// Render the Ctrl+E edit-agent/subagent dialog.
fn render_edit_dialog(f: &mut Frame, area: Rect, app: &App) {
    if !app.edit_dialog.visible {
        return;
    }

    let ed = &app.edit_dialog;

    // Dialog dimensions
    let dialog_width = area.width.min(70);
    let dialog_height = 18;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    // Clear area behind dialog
    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    let role_label = if ed.is_root { "Agente" } else { "Subagente" };
    lines.push(Line::from(Span::styled(
        format!(" ✏️  Editando {}: {} ", role_label, ed.target_name),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw("")));

    // Section tabs header
    let section_labels: Vec<&str> = if ed.is_root {
        vec![" Skills ", " MCPs ", " SubAgentes "]
    } else {
        vec![" Skills ", " MCPs "]
    };
    let mut tab_spans: Vec<Span> = Vec::new();
    for (i, label) in section_labels.iter().enumerate() {
        let style = if i == ed.section {
            Style::default()
                .fg(Color::Black)
                .bg(app.theme.accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        tab_spans.push(Span::styled(*label, style));
        tab_spans.push(Span::raw(" │ "));
    }
    if !tab_spans.is_empty() {
        tab_spans.pop(); // remove trailing separator
    }
    lines.push(Line::from(tab_spans));
    lines.push(Line::from(Span::raw("")));

    // Current section items
    let items: &[String] = match ed.section {
        0 => &ed.all_skills,
        1 => &ed.all_mcps,
        _ => &ed.all_subagents,
    };
    let enabled: &[bool] = match ed.section {
        0 => &ed.skills_enabled,
        1 => &ed.mcps_enabled,
        _ => &ed.subagents_enabled,
    };

    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no hay elementos)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let start = ed.index.saturating_sub(8);
        let end = std::cmp::min(start + 10, items.len());
        for i in start..end {
            let checkbox = if enabled[i] { "[✓]" } else { "[ ]" };
            let marker = if i == ed.index { "▸" } else { " " };
            let style = if i == ed.index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!(" {} {} {} ", marker, checkbox, items[i]),
                style,
            )));
        }
    }

    // Footer
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  ←/→: sección  │  ↑/↓: navegar  │  Espacio: toggle  │  Enter: confirmar  │  Esc: cancelar ",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
    )));

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Rgb(0, 30, 40))),
        );

    f.render_widget(dialog, dialog_area);
}
```

- [ ] **Step 2: Call `render_edit_dialog` from `render()`**

In the `render()` function, after the search overlay render and before the toasts render:

```rust
    // Render the edit-agent/subagent dialog if visible.
    if app.edit_dialog.visible {
        render_edit_dialog(f, f.area(), app);
    }
```

- [ ] **Step 3: Build check**

Run: `cargo build 2>&1 | head -30`
Expected: Build succeeds.

- [ ] **Step 4: Commit**

```bash
git add src/tui/render.rs
git commit -m "feat: render edit agent/subagent dialog overlay"
```

---

### Task 5: Handle `UpdateAgentConfig` in the engine

**Files:**
- Modify: `src/engine/orchestrator.rs` — add match arm and handler method

**Interfaces:**
- Consumes: `EngineCommand::UpdateAgentConfig`
- Produces: Updates agent runtime state, notifies agent

- [ ] **Step 1: Add match arm to command handler**

In `src/engine/orchestrator.rs` command match block, add after the `ReloadConfig` arm:

```rust
                            EngineCommand::UpdateAgentConfig {
                                name,
                                skills,
                                mcps,
                                subagents,
                            } => {
                                self.handle_update_agent_config(name, skills, mcps, subagents)
                                    .await?;
                            }
```

- [ ] **Step 2: Add `handle_update_agent_config` method**

In the orchestrator `impl` block, add:

```rust
    async fn handle_update_agent_config(
        &mut self,
        name: String,
        skills: Vec<String>,
        mcps: Vec<String>,
        subagents: Option<Vec<String>>,
    ) -> anyhow::Result<()> {
        // Update the agent in the agent manager
        self.agent_manager
            .update_agent_config(&name, &skills, &mcps, subagents.as_deref())
            .await?;

        // Notify the TUI with an updated agent info
        // by re-emitting the agent list
        self.emit_agent_list().await?;

        Ok(())
    }
```

- [ ] **Step 3: Add `update_agent_config` method to `AgentManager`**

Search for where `self.agent_manager` is used and find the `AgentManager` type location:

Use grep: `rg "struct AgentManager" src/`

In the agent manager module, add:

```rust
    /// Update the skills, MCPs and subagents of an agent by name.
    pub async fn update_agent_config(
        &mut self,
        name: &str,
        skills: &[String],
        mcps: &[String],
        subagents: Option<&[String]>,
    ) -> anyhow::Result<()> {
        if let Some(agent) = self.agents.values_mut().find(|a| a.name == name) {
            agent.skills = skills.iter().map(|s| PathBuf::from(s)).collect();
            agent.mcps = mcps.to_vec();
            if let Some(sa) = subagents {
                agent.subagents = sa.to_vec();
            }
            // Persist the change to the agent config if it's a root agent
            // (subagent config is managed by their parent).
            self.persist_agent_config(name)?;
        }
        Ok(())
    }
```

Note: the actual implementation depends on the agent manager's structure. We need to find the actual type first.

- [ ] **Step 4: Build check**

Run: `cargo build 2>&1 | head -30`
Expected: Build succeeds.

- [ ] **Step 5: Commit**

```bash
git add src/engine/orchestrator.rs src/agent/manager.rs  # or wherever agent_manager lives
git commit -m "feat: handle UpdateAgentConfig in engine"
```

---

### Task 6: Integration and verification

**Files:**
- All modified files from Tasks 1-5

- [ ] **Step 1: Build release**

Run: `cargo build 2>&1`
Expected: Clean build, no warnings.

- [ ] **Step 2: Run tests**

Run: `cargo test 2>&1`
Expected: All tests pass.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy 2>&1`
Expected: No warnings or errors.

- [ ] **Step 4: Final commit**

```bash
git commit -m "chore: final touches on edit agent dialog"
```