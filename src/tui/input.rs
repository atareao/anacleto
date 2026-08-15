//! Input-box key handling using ratatui-textarea's TextArea widget.
//!
//! Contains the `App` methods that handle keys while the Input window has
//! focus, delegating text editing to `TextArea` and handling custom actions
//! (Tab completion, history, palettes) on top.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::CursorMove;

use super::app::App;
use super::render::shift_char;
use super::types::Focus;
use crate::engine::orchestrator::EngineCommand;
use crate::tui::keymap::Action;

impl App {
    /// Handle a key while the Input window (1) has focus.
    pub(crate) fn handle_input_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        if self.keymap.matches(key_event, Action::TabComplete) {
            // Reset matches if the input has changed since last Tab
            if !self
                .textarea
                .lines()
                .first()
                .is_none_or(|l| !l.starts_with('/'))
            {
                return;
            }
            let current_text = self.textarea.lines().join("\n");
            let prefix = current_text.to_lowercase();
            if self.tab_matches.is_empty() {
                self.tab_matches = self
                    .commands
                    .iter()
                    .filter(|(c, _)| c.starts_with(&prefix))
                    .map(|(c, _)| c.clone())
                    .collect();
            }
            if self.tab_matches.is_empty() {
                return;
            }
            let idx = self.tab_index % self.tab_matches.len();
            let completed = self.tab_matches[idx].clone();
            // Replace textarea content with the completed command
            self.set_textarea_text(completed.as_str());
            self.tab_index += 1;
        } else if self.keymap.matches(key_event, Action::InsertNewline) {
            self.reset_tab_state();
            self.textarea.insert_newline();
        } else if self.keymap.matches(key_event, Action::ClearInput) {
            self.reset_tab_state();
            self.reset_textarea();
        } else if self.keymap.matches(key_event, Action::DeleteToStart) {
            self.reset_tab_state();
            self.textarea.delete_line_by_head();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteWordBefore) {
            self.reset_tab_state();
            self.textarea.delete_word();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteToEnd) {
            self.reset_tab_state();
            self.textarea.delete_line_by_end();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::CursorHome) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::Head);
        } else if self.keymap.matches(key_event, Action::CursorEnd) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::End);
        } else if self.keymap.matches(key_event, Action::CursorWordLeft) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::WordBack);
        } else if self.keymap.matches(key_event, Action::CursorWordRight) {
            self.reset_tab_state();
            self.textarea.move_cursor(CursorMove::WordForward);
        } else if self.keymap.matches(key_event, Action::CursorLeft) {
            self.textarea.move_cursor(CursorMove::Back);
        } else if self.keymap.matches(key_event, Action::CursorRight) {
            self.textarea.move_cursor(CursorMove::Forward);
        } else if self.keymap.matches(key_event, Action::CursorUp) {
            // When a palette is open, ↑ navigates the palette instead of the cursor.
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = self
                    .model_index
                    .saturating_sub(1)
                    .min(self.model_matches.len() - 1);
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = self
                    .agent_index
                    .saturating_sub(1)
                    .min(self.agent_matches.len() - 1);
            } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                self.workspace_index = self
                    .workspace_index
                    .saturating_sub(1)
                    .min(self.workspace_matches.len() - 1);
            } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                self.skill_index = self
                    .skill_index
                    .saturating_sub(1)
                    .min(self.skill_matches.len() - 1);
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = self
                    .palette_index
                    .saturating_sub(1)
                    .min(self.palette_matches.len() - 1);
            } else {
                self.textarea.move_cursor(CursorMove::Up);
            }
        } else if self.keymap.matches(key_event, Action::CursorDown) {
            // When a palette is open, ↓ navigates the palette instead of the cursor.
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = (self.model_index + 1) % self.model_matches.len();
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
            } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                self.workspace_index = (self.workspace_index + 1) % self.workspace_matches.len();
            } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                self.skill_index = (self.skill_index + 1) % self.skill_matches.len();
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
            } else {
                self.textarea.move_cursor(CursorMove::Down);
            }
        } else if self.keymap.matches(key_event, Action::DeleteChar) {
            self.textarea.delete_next_char();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteCharBefore) {
            self.tab_matches.clear();
            self.tab_index = 0;
            self.textarea.delete_char();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::HistoryUp) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = self
                    .model_index
                    .saturating_sub(1)
                    .min(self.model_matches.len() - 1);
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = self
                    .agent_index
                    .saturating_sub(1)
                    .min(self.agent_matches.len() - 1);
            } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                self.workspace_index = self
                    .workspace_index
                    .saturating_sub(1)
                    .min(self.workspace_matches.len() - 1);
            } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                self.skill_index = self
                    .skill_index
                    .saturating_sub(1)
                    .min(self.skill_matches.len() - 1);
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = self
                    .palette_index
                    .saturating_sub(1)
                    .min(self.palette_matches.len() - 1);
            } else if !self.input_history.is_empty() {
                // Navigate backwards through input history.
                let next = match self.history_index {
                    Some(i) if i > 0 => i - 1,
                    Some(_) => 0,
                    None => self.input_history.len() - 1,
                };
                self.history_index = Some(next);
                let text = self.input_history[next].clone();
                self.set_textarea_text(text.as_str());
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::HistoryDown) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = (self.model_index + 1) % self.model_matches.len();
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
            } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                self.workspace_index = (self.workspace_index + 1) % self.workspace_matches.len();
            } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                self.skill_index = (self.skill_index + 1) % self.skill_matches.len();
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
            } else if self.history_index.is_some() {
                // Navigate forwards through input history; past the newest returns to empty.
                match self.history_index {
                    Some(i) if i + 1 < self.input_history.len() => {
                        self.history_index = Some(i + 1);
                        let text = self.input_history[i + 1].clone();
                        self.set_textarea_text(text.as_str());
                    }
                    _ => {
                        self.history_index = None;
                        self.reset_textarea();
                    }
                }
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::Send) {
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.show_model_palette && !self.model_matches.is_empty() {
                // Execute `/models <selected>` from the model combo.
                let name = self.model_matches[self.model_index].clone();
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
                self.reset_textarea();
                self.handle_command(format!("/models {}", name));
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                // Execute `/agent <selected>` from the agent combo.
                let name = self.agent_matches[self.agent_index].clone();
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
                self.reset_textarea();
                self.handle_command(format!("/agent {}", name));
            } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                let name = self.workspace_matches[self.workspace_index].clone();
                self.show_workspace_palette = false;
                self.workspace_matches.clear();
                self.workspace_index = 0;
                self.reset_textarea();
                self.push_msg(format!("> /workspace {}", name));
                let _ = self.cmd_tx.try_send(EngineCommand::ListWorkspaces);
            } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                let name = self.skill_matches[self.skill_index].clone();
                self.show_skill_palette = false;
                self.skill_matches.clear();
                self.skill_index = 0;
                self.reset_textarea();
                self.push_msg(format!("> /skill {}", name));
                self.push_msg(format!("Available skill: {}", name));
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                // Execute the highlighted command from the palette.
                let idx = self.palette_matches[self.palette_index];
                let cmd = self.commands[idx].0.clone();
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
                self.reset_textarea();
                // If the selected command is /agent or /models, open the
                // corresponding palette instead of inserting the command.
                if cmd == "/agents" || cmd == "/a" {
                    self.set_textarea_text("/agent ");
                    self.update_agent_palette();
                } else if cmd == "/models" || cmd == "/m" {
                    self.set_textarea_text("/models ");
                    self.update_model_palette();
                } else if cmd == "/workspace" || cmd == "/skills" {
                    self.set_textarea_text(format!("{} ", cmd).as_str());
                    if cmd == "/workspace" {
                        self.update_workspace_palette();
                    } else {
                        self.update_skill_palette();
                    }
                } else {
                    self.handle_command(cmd);
                }
            } else {
                let input = self.textarea.lines().join("\n");

                // If input is just "/workspace" or "/skills", open the
                // corresponding palette instead of sending.
                let trimmed = input.trim();
                if trimmed == "/workspace" {
                    self.update_workspace_palette();
                    return;
                }
                if trimmed == "/skills" {
                    self.update_skill_palette();
                    return;
                }

                self.reset_textarea();
                if !input.is_empty() {
                    // Record in input history (dedupe consecutive repeats).
                    if self.input_history.last() != Some(&input) {
                        self.input_history.push(input.clone());
                    }
                    self.history_index = None;
                    // Auto-scroll al final al enviar
                    self.chat_scroll = 0;
                    self.process_input(input);
                }
            }
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            // Close the command palette first, then other overlays
            if self.show_model_palette {
                self.show_model_palette = false;
                self.model_matches.clear();
                self.model_index = 0;
            } else if self.show_agent_palette {
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
            } else if self.show_workspace_palette {
                self.show_workspace_palette = false;
                self.workspace_matches.clear();
                self.workspace_index = 0;
            } else if self.show_skill_palette {
                self.show_skill_palette = false;
                self.skill_matches.clear();
                self.skill_index = 0;
            } else if self.show_command_palette {
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
            } else if self.show_session_list {
                self.show_session_list = false;
            } else if self.show_agents {
                self.show_agents = false;
            } else if self.show_subagents {
                self.show_subagents = false;
            } else {
                // No overlay open — clear input
                self.reset_textarea();
            }
        } else if let KeyCode::Char(c) = key {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            // When a palette is open, j/k navigate instead of inserting text
            let palette_open = self.show_command_palette
                || self.show_agent_palette
                || self.show_model_palette
                || self.show_workspace_palette
                || self.show_skill_palette;
            if palette_open && (c == 'j' || c == 'k') {
                if c == 'j' {
                    // Navigate down
                    if self.show_model_palette && !self.model_matches.is_empty() {
                        self.model_index = (self.model_index + 1) % self.model_matches.len();
                    } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                        self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
                    } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                        self.workspace_index =
                            (self.workspace_index + 1) % self.workspace_matches.len();
                    } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                        self.skill_index = (self.skill_index + 1) % self.skill_matches.len();
                    } else if self.show_command_palette && !self.palette_matches.is_empty() {
                        self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
                    }
                } else {
                    // Navigate up
                    if self.show_model_palette && !self.model_matches.is_empty() {
                        self.model_index = self
                            .model_index
                            .saturating_sub(1)
                            .min(self.model_matches.len() - 1);
                    } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                        self.agent_index = self
                            .agent_index
                            .saturating_sub(1)
                            .min(self.agent_matches.len() - 1);
                    } else if self.show_workspace_palette && !self.workspace_matches.is_empty() {
                        self.workspace_index = self
                            .workspace_index
                            .saturating_sub(1)
                            .min(self.workspace_matches.len() - 1);
                    } else if self.show_skill_palette && !self.skill_matches.is_empty() {
                        self.skill_index = self
                            .skill_index
                            .saturating_sub(1)
                            .min(self.skill_matches.len() - 1);
                    } else if self.show_command_palette && !self.palette_matches.is_empty() {
                        self.palette_index = self
                            .palette_index
                            .saturating_sub(1)
                            .min(self.palette_matches.len() - 1);
                    }
                }
                return;
            }
            if self.kb_supported && modifiers.contains(KeyModifiers::SHIFT) {
                // Kitty protocol: shift is reported as a modifier;
                // apply keyboard-appropriate shift mapping
                self.textarea.insert_char(shift_char(c, &self.lang));
            } else {
                self.textarea.insert_char(c);
            }
            self.update_command_palette();
        }
    }

    /// Reset the Tab-completion autocomplete state.
    pub(crate) fn reset_tab_state(&mut self) {
        self.tab_matches.clear();
        self.tab_index = 0;
    }

    /// Handle a paste event from bracketed paste mode.
    ///
    /// Inserts the pasted text at the cursor position in the textarea.
    /// Only processes the paste when the Input window has focus.
    pub(crate) fn handle_paste(&mut self, text: String) {
        if self.focus != Focus::Input {
            return;
        }
        self.reset_tab_state();
        self.textarea.insert_str(&text);
        self.update_command_palette();
    }
}
