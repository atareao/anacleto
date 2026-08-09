//! Input-box key handling and cursor editing helpers.
//!
//! Contains the `App` methods that handle keys while the Input window has
//! focus, plus the shell-style cursor/word editing helpers they rely on.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::App;
use super::render::shift_char;
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
            if !self.input.starts_with('/') {
                return;
            }
            let prefix = self.input.to_lowercase();
            if self.tab_index == 0 || self.tab_matches.is_empty() {
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
            self.input = self.tab_matches[idx].clone();
            self.input_cursor = self.input.chars().count();
            self.tab_index += 1;
        } else if self.keymap.matches(key_event, Action::InsertNewline) {
            self.reset_tab_state();
            self.input_insert_char('\n');
        } else if self.keymap.matches(key_event, Action::ClearInput) {
            self.reset_tab_state();
            self.input.clear();
            self.input_cursor = 0;
        } else if self.keymap.matches(key_event, Action::DeleteToStart) {
            self.reset_tab_state();
            self.input_delete_to_start();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteWordBefore) {
            self.reset_tab_state();
            self.input_delete_word_before();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteToEnd) {
            self.reset_tab_state();
            self.input_delete_to_end();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::CursorHome) {
            self.reset_tab_state();
            self.input_cursor = 0;
        } else if self.keymap.matches(key_event, Action::CursorEnd) {
            self.reset_tab_state();
            self.input_cursor = self.input.chars().count();
        } else if self.keymap.matches(key_event, Action::CursorWordLeft) {
            self.reset_tab_state();
            self.input_move_word_left();
        } else if self.keymap.matches(key_event, Action::CursorWordRight) {
            self.reset_tab_state();
            self.input_move_word_right();
        } else if self.keymap.matches(key_event, Action::CursorLeft) {
            self.input_cursor = self.input_cursor.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::CursorRight) {
            let len = self.input.chars().count();
            self.input_cursor = (self.input_cursor + 1).min(len);
        } else if self.keymap.matches(key_event, Action::DeleteChar) {
            self.input_delete_at();
            self.update_command_palette();
        } else if self.keymap.matches(key_event, Action::DeleteCharBefore) {
            self.tab_matches.clear();
            self.tab_index = 0;
            self.input_delete_before();
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
                self.input = self.input_history[next].clone();
                self.input_cursor = self.input.chars().count();
                self.tab_matches.clear();
                self.tab_index = 0;
            }
        } else if self.keymap.matches(key_event, Action::HistoryDown) {
            if self.show_model_palette && !self.model_matches.is_empty() {
                self.model_index = (self.model_index + 1) % self.model_matches.len();
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                self.agent_index = (self.agent_index + 1) % self.agent_matches.len();
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                self.palette_index = (self.palette_index + 1) % self.palette_matches.len();
            } else if self.history_index.is_some() {
                // Navigate forwards through input history; past the newest returns to empty.
                match self.history_index {
                    Some(i) if i + 1 < self.input_history.len() => {
                        self.history_index = Some(i + 1);
                        self.input = self.input_history[i + 1].clone();
                    }
                    _ => {
                        self.history_index = None;
                        self.input.clear();
                    }
                }
                self.input_cursor = self.input.chars().count();
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
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(format!("/models {}", name));
            } else if self.show_agent_palette && !self.agent_matches.is_empty() {
                // Execute `/agent <selected>` from the agent combo.
                let name = self.agent_matches[self.agent_index].clone();
                self.show_agent_palette = false;
                self.agent_matches.clear();
                self.agent_index = 0;
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(format!("/agent {}", name));
            } else if self.show_command_palette && !self.palette_matches.is_empty() {
                // Execute the highlighted command from the palette.
                let idx = self.palette_matches[self.palette_index];
                let cmd = self.commands[idx].0.clone();
                self.show_command_palette = false;
                self.palette_matches.clear();
                self.palette_index = 0;
                self.input.clear();
                self.input_cursor = 0;
                self.handle_command(cmd);
            } else {
                let input = std::mem::take(&mut self.input);
                self.input_cursor = 0;
                if !input.is_empty() {
                    // Record in input history (dedupe consecutive repeats).
                    if self.input_history.last() != Some(&input) {
                        self.input_history.push(input.clone());
                    }
                    self.history_index = None;
                    // Auto-scroll al final al enviar: cuando el usuario escribe
                    // y envía un mensaje (incluso si había hecho scroll arriba),
                    // el chat debe saltar al final para mostrar el mensaje
                    // y seguir el streaming de la respuesta.
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
                self.input.clear();
                self.input_cursor = 0;
            }
        } else if let KeyCode::Char(c) = key {
            // Any non-Tab key resets autocomplete state
            self.tab_matches.clear();
            self.tab_index = 0;
            if self.kb_supported && modifiers.contains(KeyModifiers::SHIFT) {
                // Kitty protocol: shift is reported as a modifier;
                // apply keyboard-appropriate shift mapping
                self.input_insert_char(shift_char(c, &self.lang));
            } else {
                self.input_insert_char(c);
            }
            self.update_command_palette();
        }
    }

    /// Reset the Tab-completion autocomplete state.
    ///
    /// Any non-Tab key that edits the input should clear the cached matches so
    /// the next Tab press recomputes them from the current input.
    pub(crate) fn reset_tab_state(&mut self) {
        self.tab_matches.clear();
        self.tab_index = 0;
    }

    /// Convert a character index into a byte index within `input`.
    pub(crate) fn input_char_to_byte(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    /// Insert a character at the cursor position and advance the cursor.
    pub(crate) fn input_insert_char(&mut self, c: char) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.insert(byte_idx, c);
        self.input_cursor += 1;
    }

    /// Delete the character before the cursor (Backspace).
    pub(crate) fn input_delete_before(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let prev_len = self.input[..byte_idx]
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.input.replace_range(byte_idx - prev_len..byte_idx, "");
        self.input_cursor -= 1;
    }

    /// Delete the character at the cursor (Delete).
    pub(crate) fn input_delete_at(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        if byte_idx >= self.input.len() {
            return;
        }
        let next_len = self.input[byte_idx..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.input.replace_range(byte_idx..byte_idx + next_len, "");
    }

    /// Move the cursor to the start of the previous word.
    pub(crate) fn input_move_word_left(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let before: Vec<char> = self.input[..byte_idx].chars().collect();
        let mut i = before.len();
        // Skip trailing whitespace.
        while i > 0 && before[i - 1].is_whitespace() {
            i -= 1;
        }
        // Skip the word.
        while i > 0 && !before[i - 1].is_whitespace() {
            i -= 1;
        }
        self.input_cursor = i;
    }

    /// Move the cursor to the start of the next word.
    pub(crate) fn input_move_word_right(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        let after: Vec<char> = self.input[byte_idx..].chars().collect();
        let mut i = 0;
        // Skip the current word.
        while i < after.len() && !after[i].is_whitespace() {
            i += 1;
        }
        // Skip whitespace.
        while i < after.len() && after[i].is_whitespace() {
            i += 1;
        }
        self.input_cursor = (self.input_cursor + i).min(self.input.chars().count());
    }

    /// Delete the word before the cursor (Ctrl+W).
    pub(crate) fn input_delete_word_before(&mut self) {
        let old_cursor = self.input_cursor;
        self.input_move_word_left();
        let new_cursor = self.input_cursor;
        let start_byte = self.input_char_to_byte(new_cursor);
        let end_byte = self.input_char_to_byte(old_cursor);
        self.input.replace_range(start_byte..end_byte, "");
        self.input_cursor = new_cursor;
    }

    /// Delete from the start of the line to the cursor (Ctrl+U).
    pub(crate) fn input_delete_to_start(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.replace_range(0..byte_idx, "");
        self.input_cursor = 0;
    }

    /// Delete from the cursor to the end of the line (Ctrl+K).
    pub(crate) fn input_delete_to_end(&mut self) {
        let byte_idx = self.input_char_to_byte(self.input_cursor);
        self.input.truncate(byte_idx);
    }
}
