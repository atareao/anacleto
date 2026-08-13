//! Global key handling for the TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::engine::orchestrator::EngineCommand;
use crate::tui::app::App;
use crate::tui::keymap::Action;
use crate::tui::render::shift_char;
use crate::tui::toast::ToastKind;
use crate::tui::types::Focus;
#[cfg(test)]
use ratatui_textarea::TextArea;

impl App {
    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let key_event = KeyEvent::new(key, modifiers);

        // If the search overlay is open, all keys drive the search box.
        if self.search.visible {
            match key {
                KeyCode::Esc => {
                    self.search.visible = false;
                    self.search.query.clear();
                    self.search.matches.clear();
                    self.search.selected = 0;
                }
                KeyCode::Enter => {
                    // Jump to the selected match in the chat.
                    if let Some(idx) = self.search.matches.get(self.search.selected) {
                        self.chat_scroll = self.chat_height_at(*idx);
                    }
                    self.search.visible = false;
                    self.search.query.clear();
                    self.search.matches.clear();
                    self.search.selected = 0;
                }
                KeyCode::Up => {
                    if !self.search.matches.is_empty() {
                        self.search.selected = self.search.selected.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if self.search.selected + 1 < self.search.matches.len() {
                        self.search.selected += 1;
                    }
                }
                KeyCode::Backspace => {
                    self.search.query.pop();
                    self.update_search_matches();
                }
                KeyCode::Char(c) => {
                    self.search.query.push(c);
                    self.update_search_matches();
                }
                _ => {}
            }
            return;
        }

        // If the which-key popup is open, any key press closes it.
        if self.which_key.visible {
            self.which_key.visible = false;
            return;
        }

        // If approval dialog is active, Y/N are handled specially
        if self.pending_approval.is_some() {
            match key {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::ApprovalResponse { id, approved: true });
                        self.push_msg(format!("[Aprobación] ✓ Aprobado: {}", approval.operation));
                        self.toasts.push("Aprobado", ToastKind::Success);
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::ApprovalResponse {
                            id,
                            approved: false,
                        });
                        self.push_msg(format!("[Aprobación] ✗ Denegado: {}", approval.operation));
                        self.toasts.push("Denegado", ToastKind::Info);
                    }
                }
                _ if self.keymap.matches(key_event, Action::Approve) => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::ApprovalResponse { id, approved: true });
                        self.push_msg(format!("[Aprobación] ✓ Aprobado: {}", approval.operation));
                        self.toasts.push("Aprobado", ToastKind::Success);
                    }
                }
                _ if self.keymap.matches(key_event, Action::Deny) => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::ApprovalResponse {
                            id,
                            approved: false,
                        });
                        self.push_msg(format!("[Aprobación] ✗ Denegado: {}", approval.operation));
                        self.toasts.push("Denegado", ToastKind::Info);
                    }
                }
                _ => {}
            }
            return;
        }

        // Inline question dialog (`/question` tool): capture answer.
        if self.pending_question.is_some() {
            match key {
                KeyCode::Enter => {
                    if let Some(q) = self.pending_question.take() {
                        let answer = if !q.options.is_empty() {
                            q.options.get(q.selected).cloned().unwrap_or_default()
                        } else {
                            q.answer_input.trim().to_string()
                        };
                        let id = q.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::QuestionAnswer {
                            id,
                            answer: answer.clone(),
                        });
                        self.push_msg(format!("[Respuesta] {}", answer));
                    }
                }
                KeyCode::Esc => {
                    if let Some(q) = self.pending_question.take() {
                        let id = q.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::QuestionAnswer {
                            id,
                            answer: String::new(),
                        });
                        self.push_msg("[Respuesta] (cancelada)".to_string());
                    }
                }
                KeyCode::Up => {
                    if let Some(q) = self.pending_question.as_mut()
                        && !q.options.is_empty()
                    {
                        q.selected = q.selected.saturating_sub(1);
                    }
                }
                KeyCode::Down => {
                    if let Some(q) = self.pending_question.as_mut()
                        && !q.options.is_empty()
                    {
                        q.selected = (q.selected + 1) % q.options.len();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(q) = self.pending_question.as_mut()
                        && q.options.is_empty()
                    {
                        q.answer_input.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(q) = self.pending_question.as_mut()
                        && q.options.is_empty()
                    {
                        q.answer_input.pop();
                    }
                }
                _ => {}
            }
            return;
        }

        // Interactive `/init` flow: capture answers.
        if self.init_flow.is_some() {
            match key {
                KeyCode::Enter => {
                    self.collect_init_answer();
                }
                KeyCode::Esc => {
                    self.init_flow = None;
                    self.reset_textarea();
                }
                KeyCode::Char(c) => {
                    self.textarea.insert_char(c);
                }
                KeyCode::Backspace => {
                    self.textarea.delete_char();
                }
                _ => {}
            }
            return;
        }

        // Timeline navigation.
        if self.show_timeline {
            match key {
                KeyCode::Up => {
                    self.timeline_index = self.timeline_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.timeline.is_empty() {
                        self.timeline_index = (self.timeline_index + 1) % self.timeline.len();
                    }
                }
                KeyCode::Enter => {
                    self.jump_to_timeline_entry();
                }
                KeyCode::Esc => {
                    self.show_timeline = false;
                }
                _ => {}
            }
            return;
        }

        // MCP list navigation.
        if self.show_mcps {
            match key {
                KeyCode::Up => {
                    self.mcps_index = self.mcps_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if !self.mcps_list.is_empty() {
                        self.mcps_index = (self.mcps_index + 1) % self.mcps_list.len();
                    }
                }
                KeyCode::Enter => {
                    self.toggle_selected_mcp();
                }
                KeyCode::Esc => {
                    self.show_mcps = false;
                }
                _ => {}
            }
            return;
        }

        // ── Model picker navigation ──────────────────────────────────
        if self.model_picker.visible {
            match key {
                KeyCode::Up => self.model_picker.previous(),
                KeyCode::Down => self.model_picker.next(),
                KeyCode::Tab | KeyCode::Right => self.model_picker.next_mode(),
                KeyCode::Left => self.model_picker.previous_mode(),
                KeyCode::Enter => {
                    if let Some(model) = self.model_picker.selected_model() {
                        let _ = self.cmd_tx.try_send(EngineCommand::SetModel(model.clone()));
                        let _ = self.cmd_tx.try_send(EngineCommand::RecordModelUsage(model));
                        self.toasts.push("Cambiando modelo…", ToastKind::Info);
                    }
                    self.model_picker.visible = false;
                }
                KeyCode::Esc => {
                    self.model_picker.visible = false;
                }
                _ => {}
            }
            return;
        }

        // ── Edit agent/subagent dialog navigation ──────────────────
        if self.edit_dialog.visible {
            match key {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.edit_dialog.index > 0 {
                        self.edit_dialog.index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let len = self.edit_dialog.section_len();
                    if len > 0 && self.edit_dialog.index + 1 < len {
                        self.edit_dialog.index += 1;
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if self.edit_dialog.section > 0 {
                        self.edit_dialog.section -= 1;
                        self.edit_dialog.index = 0;
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => {
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
                    let skills: Vec<String> = self
                        .edit_dialog
                        .all_skills
                        .iter()
                        .zip(self.edit_dialog.skills_enabled.iter())
                        .filter(|&(_, &enabled)| enabled)
                        .map(|(s, _)| s.clone())
                        .collect();
                    let mcps: Vec<String> = self
                        .edit_dialog
                        .all_mcps
                        .iter()
                        .zip(self.edit_dialog.mcps_enabled.iter())
                        .filter(|&(_, &enabled)| enabled)
                        .map(|(m, _)| m.clone())
                        .collect();
                    let subagents: Option<Vec<String>> = if self.edit_dialog.is_root {
                        Some(
                            self.edit_dialog
                                .all_subagents
                                .iter()
                                .zip(self.edit_dialog.subagents_enabled.iter())
                                .filter(|&(_, &enabled)| enabled)
                                .map(|(s, _)| s.clone())
                                .collect(),
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
                    self.toasts.push(
                        "Configuración actualizada",
                        crate::tui::toast::ToastKind::Success,
                    );
                }
                KeyCode::Esc => {
                    self.edit_dialog.visible = false;
                }
                _ => {}
            }
            return;
        }

        // ── Diff viewer navigation ───────────────────────────────────
        if self.diff_viewer.visible {
            match key {
                KeyCode::Up => self.diff_viewer.scroll_up(1),
                KeyCode::Down => self.diff_viewer.scroll_down(1),
                KeyCode::PageUp => self.diff_viewer.scroll_up(10),
                KeyCode::PageDown => self.diff_viewer.scroll_down(10),
                KeyCode::Esc => {
                    self.diff_viewer.visible = false;
                }
                _ => {}
            }
            return;
        }

        // ── Prompt queue popup navigation ───────────────────────────
        if self.show_prompt_queue {
            match key {
                KeyCode::Up => {
                    self.prompt_queue_index = self.prompt_queue_index.saturating_sub(1);
                }
                KeyCode::Down => {
                    if self.prompt_queue_index + 1 < self.prompt_queue.len() {
                        self.prompt_queue_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(prompt) = self.prompt_queue.get(self.prompt_queue_index) {
                        let text = prompt.clone();
                        self.prompt_queue.remove(self.prompt_queue_index);
                        if self.prompt_queue.is_empty() {
                            self.show_prompt_queue = false;
                        } else {
                            self.prompt_queue_index =
                                self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                        }
                        let _ = self.cmd_tx.try_send(EngineCommand::UserInput(text));
                    }
                }
                KeyCode::Char('d') => {
                    if !self.prompt_queue.is_empty() {
                        self.prompt_queue.remove(self.prompt_queue_index);
                        if self.prompt_queue.is_empty() {
                            self.show_prompt_queue = false;
                        } else {
                            self.prompt_queue_index =
                                self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                        }
                    }
                }
                KeyCode::Char('e') => {
                    // Edit: load the selected item into the input buffer,
                    // remove it from the queue and close the popup.
                    if let Some(prompt) = self.prompt_queue.get(self.prompt_queue_index) {
                        let prompt = prompt.clone();
                        self.set_textarea_text(prompt.as_str());
                        self.prompt_queue.remove(self.prompt_queue_index);
                        self.show_prompt_queue = false;
                    }
                }
                KeyCode::Char('[') => {
                    // Move the selected item up in the queue.
                    if self.prompt_queue_index > 0 {
                        self.prompt_queue
                            .swap(self.prompt_queue_index, self.prompt_queue_index - 1);
                        self.prompt_queue_index -= 1;
                    }
                }
                KeyCode::Char(']') => {
                    // Move the selected item down in the queue.
                    if self.prompt_queue_index + 1 < self.prompt_queue.len() {
                        self.prompt_queue
                            .swap(self.prompt_queue_index, self.prompt_queue_index + 1);
                        self.prompt_queue_index += 1;
                    }
                }
                KeyCode::Esc => {
                    self.show_prompt_queue = false;
                }
                _ => {}
            }
            return;
        }

        // ── Focus switching (Alt+1..Alt+5) + keymap-driven global actions ──
        // Only dispatch when the key is a special/modified key, or when the
        // input is empty (so plain characters can still be typed normally).
        // Alt+1..Alt+5 are modified keys, so they always apply; the legacy
        // letter bindings ('c'/'i') only switch focus when the input is empty.
        //
        // Ctrl+E is intercepted here, BEFORE the keymap dispatch, so that in
        // the Agents panel or the SubAgents tab it opens the edit dialog
        // instead of being swallowed by Action::OpenEditor (which launches the
        // external text editor).
        if key == KeyCode::Char('e')
            && modifiers == KeyModifiers::CONTROL
            && self.open_edit_dialog_for_focus()
        {
            return;
        }

        // / key from any panel switches focus to Input and inserts the slash
        // so the user can immediately start typing a command.
        //
        // Without Kitty protocol: the terminal sends the actual character '/'
        // (KeyModifiers::NONE on US layout, KeyModifiers::SHIFT on some
        // terminals with Spanish layout where / is Shift+7).
        //
        // With Kitty protocol (kb_supported): the terminal sends the raw key
        // (e.g. '7' on Spanish layout) and the SHIFT modifier; the application
        // resolves the actual character via shift_char(). We use shift_char
        // here too so the detection works regardless of keyboard layout.
        if self.focus != Focus::Input {
            let produces_slash = match key {
                KeyCode::Char(c)
                    if c == '/'
                        && (modifiers == KeyModifiers::NONE
                            || modifiers == KeyModifiers::SHIFT) =>
                {
                    true
                }
                KeyCode::Char(c)
                    if self.kb_supported
                        && modifiers == KeyModifiers::SHIFT
                        && shift_char(c, &self.lang) == '/' =>
                {
                    true
                }
                _ => false,
            };
            if produces_slash {
                self.focus = Focus::Input;
                self.set_textarea_text("/");
                return;
            }
        }

        if self.keymap_applies(key_event) {
            if self.keymap.matches(key_event, Action::FocusChat) {
                self.focus = Focus::Chat;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusInfo) {
                self.focus = Focus::Info;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusQueue) {
                self.focus = Focus::Queue;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusMcps) {
                // Config-compat: legacy MCPs focus maps to the Info panel's
                // MCPs tab.
                self.focus = Focus::Info;
                self.info_tab = 1;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusSkills) {
                // Config-compat: legacy Skills focus maps to the Info panel's
                // Skills tab.
                self.focus = Focus::Info;
                self.info_tab = 0;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusAgents) {
                self.focus = Focus::Agents;
                return;
            }
            if self.keymap.matches(key_event, Action::FocusInput) {
                self.focus = Focus::Input;
                return;
            }

            // Tab / Shift+Tab cycle focus through panels.
            // Order: Input(1) → Chat(2) → Info(3) → Agents(4) → Queue(5) → Input(1)
            if self.keymap.matches(key_event, Action::FocusNext) {
                self.focus = match self.focus {
                    Focus::Input => Focus::Chat,
                    Focus::Chat => Focus::Info,
                    Focus::Info => Focus::Agents,
                    Focus::Agents => Focus::Queue,
                    Focus::Queue => Focus::Input,
                };
                return;
            }
            if self.keymap.matches(key_event, Action::FocusPrev) {
                self.focus = match self.focus {
                    Focus::Input => Focus::Queue,
                    Focus::Queue => Focus::Agents,
                    Focus::Agents => Focus::Info,
                    Focus::Info => Focus::Chat,
                    Focus::Chat => Focus::Input,
                };
                return;
            }

            if self.keymap.matches(key_event, Action::Quit) {
                self.should_exit = true;
                return;
            }
            if self.keymap.matches(key_event, Action::OpenWhichKey) {
                self.which_key.visible = true;
                return;
            }
            if self.keymap.matches(key_event, Action::ToggleSidebar) {
                self.show_sidebar = !self.show_sidebar;
                return;
            }
            if self.keymap.matches(key_event, Action::ToggleDiffViewer) {
                self.diff_viewer.visible = !self.diff_viewer.visible;
                return;
            }
            if self.keymap.matches(key_event, Action::OpenModelPicker) {
                self.model_picker.visible = true;
                let _ = self.cmd_tx.try_send(EngineCommand::ListModelFrecency);
                return;
            }
            if self.keymap.matches(key_event, Action::OpenEditor) {
                self.open_editor();
                return;
            }
            if self.keymap.matches(key_event, Action::OpenPromptQueue) {
                self.show_prompt_queue = true;
                self.prompt_queue_index = 0;
                return;
            }
            if self.keymap.matches(key_event, Action::ToggleSearch) {
                self.search.visible = !self.search.visible;
                if self.search.visible {
                    self.search.query.clear();
                    self.update_search_matches();
                }
                return;
            }
            if self.keymap.matches(key_event, Action::EmergencyStop) {
                // Send stop command to engine
                let _ = self.cmd_tx.try_send(EngineCommand::StopAgent);
                // Push a visual confirmation message
                self.push_msg("⏹ Stopped");
                // Clear any in-progress streaming response
                self.current_stream = None;
                self.current_thinking = None;
                // Show a toast notification
                self.toasts.push("⏹ Stopped", ToastKind::Info);
                return;
            }
            // Quick slots 1..9 resume the pinned session at that index.
            let quick_slots = [
                Action::QuickSlot1,
                Action::QuickSlot2,
                Action::QuickSlot3,
                Action::QuickSlot4,
                Action::QuickSlot5,
                Action::QuickSlot6,
                Action::QuickSlot7,
                Action::QuickSlot8,
                Action::QuickSlot9,
            ];
            for (idx, action) in quick_slots.iter().enumerate() {
                if self.keymap.matches(key_event, *action) {
                    self.resume_quick_slot(idx);
                    return;
                }
            }
        }

        // ── Route the remaining keys by the focused window ───────────
        match self.focus {
            Focus::Input => self.handle_input_key(key, modifiers, key_event),
            Focus::Chat => self.handle_chat_key(key, modifiers, key_event),
            Focus::Info => self.handle_info_panel_key(key, modifiers, key_event),
            Focus::Queue => self.handle_queue_panel_key(key, modifiers, key_event),
            Focus::Agents => self.handle_agent_panel_key(key, modifiers, key_event),
        }
    }

    /// Handle a mouse click or scroll — set focus to the panel under the cursor,
    /// or scroll the chat with the mouse wheel.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::MouseButton;
        use crossterm::event::MouseEventKind;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                // Scroll up (show older content)
                self.chat_scroll = self.chat_scroll.saturating_add(3);
                return;
            }
            MouseEventKind::ScrollDown => {
                // Scroll down (show newer content)
                self.chat_scroll = self.chat_scroll.saturating_sub(3);
                return;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Left-click: set focus to the panel under the cursor
            }
            _ => return,
        }

        let Ok((term_width, term_height)) = crossterm::terminal::size() else {
            return;
        };
        let area = Rect::new(0, 0, term_width, term_height);

        // Replicate the vertical layout from render.rs
        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // status bar
                Constraint::Min(1),    // main content
                Constraint::Length(5), // input
                Constraint::Length(1), // working directory
            ])
            .split(area);

        let y = mouse.row;

        // ── Status bar (row 0) — no focus change ────────────────
        if y < vert[0].height {
            return;
        }

        // ── Input area (row 3, height 4) ────────────────────────
        if y >= vert[2].y && y < vert[2].y + vert[2].height {
            self.focus = Focus::Input;
            return;
        }

        // ── Working directory bar (last row) — no focus change ──
        if y >= vert[3].y {
            return;
        }

        // ── Main content area ───────────────────────────────────
        let main_area = vert[1];

        if self.show_sidebar {
            let horiz = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(main_area);

            let x = mouse.column;

            // Left panel (chat / overlays)
            if x < horiz[0].x + horiz[0].width {
                self.focus = Focus::Chat;
                // Check for a click on a code block's `[copy]` line.
                // Pass the LEFT panel area (horiz[0]), NOT main_area, so
                // content_x/y and content_width match the render.
                self.handle_code_block_click(x, y, horiz[0]);
                // Check for a click on a collapsed section summary line.
                self.handle_section_click(x, y, horiz[0]);
                return;
            }

            // Right panel — determine which sub-panel
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6),   // Status
                    Constraint::Ratio(1, 3), // Info (Skills/MCPs)
                    Constraint::Ratio(1, 3), // Agents
                    Constraint::Ratio(1, 3), // Queue
                ])
                .split(horiz[1]);

            for (i, chunk) in right.iter().enumerate() {
                if y >= chunk.y && y < chunk.y + chunk.height {
                    match i {
                        1 => self.focus = Focus::Info,
                        2 => self.focus = Focus::Agents,
                        3 => self.focus = Focus::Queue,
                        _ => {} // Status panel (index 0): no focus
                    }
                    return;
                }
            }
        } else {
            // Sidebar hidden: left panel takes full width.
            self.focus = Focus::Chat;
            self.handle_code_block_click(mouse.column, y, main_area);
            self.handle_section_click(mouse.column, y, main_area);
        }
    }

    /// Toggle collapse/expand for a section clicked in the chat area.
    ///
    /// Clicking on any line of a section collapses it (if not already collapsed)
    /// or expands it back (if currently collapsed).
    fn handle_section_click(&mut self, x: u16, y: u16, main_area: Rect) {
        // The chat content is inset by a 1-cell border on each side.
        let content_x = main_area.x + 1;
        let content_y = main_area.y + 1;
        if x < content_x || y < content_y {
            return;
        }
        let row = (y - content_y) as usize;

        // Recompute the visible start index the same way render does.
        let content_width = (main_area.width.saturating_sub(2)).max(1) as usize;
        let visible = (main_area.height.max(2) as usize) - 2;
        let vs = crate::tui::markdown::select_visible_start(
            &self.rendered_chat_lines,
            visible,
            content_width,
            self.chat_scroll,
        );

        let abs_line = vs.start_idx as usize + row;

        // Find which section (if any) this line belongs to.
        let Some(section_id) = self.section_line_map.get(abs_line).and_then(|v| v.as_ref()) else {
            return;
        };

        // Look up section info for the toast.
        let Some(section) = self.section_info.iter().find(|s| s.id == *section_id) else {
            return;
        };

        if self.collapsed_sections.contains(section_id) {
            // ── Expand ──
            self.collapsed_sections.remove(section_id);
            self.toasts.push(
                format!(
                    "Expanded {} section ({} lines)",
                    section.section_type, section.line_count
                ),
                crate::tui::toast::ToastKind::Info,
            );
        } else {
            // ── Collapse (only if the section has meaningful content) ──
            if section.line_count < 2 {
                return;
            }
            self.collapsed_sections.insert(section_id.clone());
            self.toasts.push(
                format!(
                    "Collapsed {} section ({} lines)",
                    section.section_type, section.line_count
                ),
                crate::tui::toast::ToastKind::Info,
            );
        }
    }

    /// If the click lands on a code block's `[copy]` line, copy that block.
    /// `main_area` is the chat panel area (before borders).
    fn handle_code_block_click(&mut self, x: u16, y: u16, main_area: Rect) {
        // The chat content is inset by a 1-cell border on each side.
        let content_x = main_area.x + 1;
        let content_y = main_area.y + 1;
        if x < content_x || y < content_y {
            return;
        }
        let row = (y - content_y) as usize;

        // Recompute the visible start index the same way render does.
        let content_width = (main_area.width.saturating_sub(2)).max(1) as usize;
        let visible = (main_area.height.max(2) as usize) - 2;
        let vs = crate::tui::markdown::select_visible_start(
            &self.rendered_chat_lines,
            visible,
            content_width,
            self.chat_scroll,
        );

        // Check code block copy (match on logical line index — copy buttons
        // are short, never wrap, so this is reliable even with wrapping elsewhere).
        for block in &self.code_block_positions {
            if block.copy_line == vs.start_idx as usize + row {
                match crate::tui::render::copy_to_clipboard(&block.code) {
                    Ok(()) => self.toasts.push(
                        format!(
                            "Código '{}' copiado al portapapeles ({} líneas)",
                            block.lang,
                            block.code.lines().count()
                        ),
                        crate::tui::toast::ToastKind::Success,
                    ),
                    Err(e) => self.toasts.push(
                        format!("Error al copiar: {}", e),
                        crate::tui::toast::ToastKind::Error,
                    ),
                }
                return;
            }
        }
    }

    /// Whether a key event should be dispatched through the keymap.
    ///
    /// Special keys (Enter, Esc, PageUp, ...) and modified keys (Ctrl+...) are
    /// always dispatched. Plain character keys are only dispatched when the
    /// input buffer is empty, so that typing normally is never intercepted.
    fn keymap_applies(&self, key_event: KeyEvent) -> bool {
        match key_event.code {
            KeyCode::Char(_) => {
                // In the Input window, plain characters are always typed and never
                // trigger global actions, even with an empty buffer.
                if self.focus == Focus::Input && key_event.modifiers == KeyModifiers::NONE {
                    return false;
                }
                key_event.modifiers != KeyModifiers::NONE
                    || self.textarea.lines().join("\n").is_empty()
            }
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::sync::mpsc;

    fn test_app() -> App {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        App::new(cmd_tx, event_rx, false, &Config::default())
    }

    #[test]
    fn queue_popup_e_edits_selected_item() {
        let mut app = test_app();
        app.show_prompt_queue = true;
        app.prompt_queue_index = 1;
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        app.handle_key(KeyCode::Char('e'), KeyModifiers::NONE);

        // Item loaded into input, removed from queue, popup closed.
        assert_eq!(app.textarea.lines().join("\n"), "second");
        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        assert!(!app.show_prompt_queue);
    }

    #[test]
    fn queue_popup_bracket_moves_item_up() {
        let mut app = test_app();
        app.show_prompt_queue = true;
        app.prompt_queue_index = 1;
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        app.handle_key(KeyCode::Char('['), KeyModifiers::NONE);

        assert_eq!(
            app.prompt_queue,
            vec!["second".to_string(), "first".to_string()]
        );
        assert_eq!(app.prompt_queue_index, 0);
    }

    #[test]
    fn queue_popup_bracket_moves_item_down() {
        let mut app = test_app();
        app.show_prompt_queue = true;
        app.prompt_queue_index = 0;
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];

        app.handle_key(KeyCode::Char(']'), KeyModifiers::NONE);

        assert_eq!(
            app.prompt_queue,
            vec!["second".to_string(), "first".to_string()]
        );
        assert_eq!(app.prompt_queue_index, 1);
    }

    #[test]
    fn ctrl_e_in_agents_panel_opens_edit_dialog() {
        use crate::agent::types::{AgentId, AgentRole, AgentStatus};
        use crate::tui::types::AgentInfo;

        let mut app = test_app();
        app.focus = Focus::Agents;
        app.agents.push(AgentInfo {
            id: AgentId::new(),
            name: "root".to_string(),
            role: AgentRole::Root,
            status: AgentStatus::Idle,
            skills: vec!["skill1".to_string()],
            mcps: vec!["mcp1".to_string()],
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: None,
            mode: None,
        });
        app.agent_panel_index = 0;

        app.handle_key(KeyCode::Char('e'), KeyModifiers::CONTROL);

        assert!(app.edit_dialog.visible, "dialog should open on Ctrl+E");
        assert_eq!(app.edit_dialog.target_name, "root");
        assert!(app.edit_dialog.is_root);
    }

    #[test]
    fn ctrl_e_in_subagent_tab_opens_edit_dialog() {
        use crate::agent::types::{AgentId, AgentRole, AgentStatus};
        use crate::tui::types::AgentInfo;

        let mut app = test_app();
        app.focus = Focus::Info;
        app.info_tab = 2;
        app.configured_subagents
            .insert("root".to_string(), vec!["reviewer".to_string()]);
        app.subagent_panel_index = 0;
        // Provide an agent instance of that subagent type so skills/MCPs resolve.
        app.agents.push(AgentInfo {
            id: AgentId::new(),
            name: "reviewer".to_string(),
            role: AgentRole::SubAgent,
            status: AgentStatus::Idle,
            skills: vec!["code-review".to_string()],
            mcps: vec!["filesystem".to_string()],
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: Some("reviewer".to_string()),
            mode: None,
        });

        app.handle_key(KeyCode::Char('e'), KeyModifiers::CONTROL);

        assert!(app.edit_dialog.visible, "dialog should open on Ctrl+E");
        assert_eq!(app.edit_dialog.target_name, "reviewer");
        assert!(!app.edit_dialog.is_root);
    }

    #[test]
    fn slash_from_chat_focuses_input_and_inserts_slash() {
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_from_info_focuses_input_and_inserts_slash() {
        let mut app = test_app();
        app.focus = Focus::Info;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_from_agents_focuses_input_and_inserts_slash() {
        let mut app = test_app();
        app.focus = Focus::Agents;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_from_queue_focuses_input_and_inserts_slash() {
        let mut app = test_app();
        app.focus = Focus::Queue;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_from_input_does_not_duplicate() {
        // When already in Input, pressing / should just type the character
        // (handled by handle_input_key), not trigger the focus switch.
        let mut app = test_app();
        app.focus = Focus::Input;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_with_alt_modifier_does_not_trigger_focus_switch() {
        // Alt+/ should not switch focus (it might be used for something else).
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::ALT);

        // Focus should remain on Chat since Alt modifier is used
        assert_eq!(app.focus, Focus::Chat);
    }

    #[test]
    fn slash_with_shift_modifier_triggers_focus_switch() {
        // Shift+/ (Spanish keyboard layout) should switch focus to Input.
        let mut app = test_app();
        app.focus = Focus::Chat;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('/'), KeyModifiers::SHIFT);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_with_kitty_protocol_spanish_layout() {
        // With Kitty protocol enabled and Spanish keyboard,
        // Shift+7 arrives as Char('7') + SHIFT.
        let mut app = test_app();
        app.kb_supported = true;
        app.lang = "es_ES.UTF-8".to_string();
        app.focus = Focus::Chat;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('7'), KeyModifiers::SHIFT);

        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.textarea.lines().join("\n"), "/");
    }

    #[test]
    fn slash_kitty_protocol_no_false_positive() {
        // With Kitty protocol, plain '7' without shift should NOT trigger.
        let mut app = test_app();
        app.kb_supported = true;
        app.lang = "es_ES.UTF-8".to_string();
        app.focus = Focus::Chat;
        app.textarea = TextArea::default();

        app.handle_key(KeyCode::Char('7'), KeyModifiers::NONE);

        assert_eq!(app.focus, Focus::Chat);
        assert!(app.textarea.lines().join("\n").is_empty());
    }
}
