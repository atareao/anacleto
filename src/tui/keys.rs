//! Global key handling for the TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::engine::orchestrator::EngineCommand;
use crate::tui::app::App;
use crate::tui::keymap::Action;
use crate::tui::toast::ToastKind;
use crate::tui::types::Focus;

impl App {
    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        let key_event = KeyEvent::new(key, modifiers);

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
                        self.toasts.push("Denegado", ToastKind::Info);
                    }
                }
                _ if self.keymap.matches(key_event, Action::Approve) => {
                    if let Some(ref approval) = self.pending_approval.take() {
                        let id = approval.id.clone();
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::ApprovalResponse { id, approved: true });
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
                        let _ = self
                            .cmd_tx
                            .try_send(EngineCommand::QuestionAnswer { id, answer });
                    }
                }
                KeyCode::Esc => {
                    if let Some(q) = self.pending_question.take() {
                        let id = q.id.clone();
                        let _ = self.cmd_tx.try_send(EngineCommand::QuestionAnswer {
                            id,
                            answer: String::new(),
                        });
                    }
                }
                KeyCode::Up => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if !q.options.is_empty() {
                            q.selected = q.selected.saturating_sub(1);
                        }
                    }
                }
                KeyCode::Down => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if !q.options.is_empty() {
                            q.selected = (q.selected + 1) % q.options.len();
                        }
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if q.options.is_empty() {
                            q.answer_input.push(c);
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(q) = self.pending_question.as_mut() {
                        if q.options.is_empty() {
                            q.answer_input.pop();
                        }
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
                    self.input.clear();
                    self.input_cursor = 0;
                }
                KeyCode::Char(c) => {
                    self.input.push(c);
                    self.input_cursor = self.input.chars().count();
                }
                KeyCode::Backspace => {
                    self.input.pop();
                    self.input_cursor = self.input.chars().count();
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
                        self.input = prompt.clone();
                        self.input_cursor = self.input.chars().count();
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
                key_event.modifiers != KeyModifiers::NONE || self.input.is_empty()
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
        assert_eq!(app.input, "second");
        assert_eq!(app.input_cursor, 6);
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
}
