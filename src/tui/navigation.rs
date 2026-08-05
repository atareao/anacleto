//! Key handling for the non-input panels (Chat, MCPs, Skills, Agents).
//!
//! Contains the `App` methods that route keys while one of the sidebar/chat
//! panels has focus, plus the shared list-navigation helper and the
//! double-`g` (gg) detection.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::{App, Focus};
use crate::tui::keymap::Action;

impl App {
    /// Handle a key while the Chat window (1) has focus.
    pub(crate) fn handle_chat_key(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        if self.keymap.matches(key_event, Action::ScrollUp) {
            self.chat_scroll = self.chat_scroll.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::ScrollDown) {
            self.chat_scroll = self.chat_scroll.saturating_add(1);
        } else if self.keymap.matches(key_event, Action::PageUp) {
            self.chat_scroll = self.chat_scroll.saturating_add(10);
        } else if self.keymap.matches(key_event, Action::PageDown) {
            self.chat_scroll = self.chat_scroll.saturating_sub(10);
        } else if self.keymap.matches(key_event, Action::ChatTop) {
            if key == KeyCode::Home || (key == KeyCode::Char('g') && self.is_double_g()) {
                // Home or gg: jump to the top of the chat.
                self.chat_scroll = u16::MAX;
            }
        } else if self.keymap.matches(key_event, Action::ChatBottom) {
            // End or G: jump to the bottom (auto-scroll).
            self.chat_scroll = 0;
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            self.focus = Focus::Input;
        }
    }

    /// Handle a key while the MCPs sidebar panel (2) has focus.
    pub(crate) fn handle_mcp_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        let len = self.unique_mcp_count();
        self.mcp_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.mcp_panel_index);
    }

    /// Handle a key while the Skills sidebar panel (3) has focus.
    pub(crate) fn handle_skill_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        let len = self.unique_skill_count();
        self.skill_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.skill_panel_index);
    }

    /// Handle a key while the Agents sidebar panel (4) has focus.
    pub(crate) fn handle_agent_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        let len = self.agent_panel_count();
        self.agent_panel_index =
            self.handle_list_nav_key(key, modifiers, key_event, len, self.agent_panel_index);
    }

    /// Shared Vim/arrow navigation for a list panel (MCPs, Skills, Agents).
    /// Returns the updated selection index.
    pub(crate) fn handle_list_nav_key(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
        key_event: KeyEvent,
        len: usize,
        mut index: usize,
    ) -> usize {
        if self.keymap.matches(key_event, Action::ListDown) {
            if len > 0 {
                index = (index + 1).min(len - 1);
            }
        } else if self.keymap.matches(key_event, Action::ListUp) {
            index = index.saturating_sub(1);
        } else if self.keymap.matches(key_event, Action::ListTop) {
            if key == KeyCode::Home || (key == KeyCode::Char('g') && self.is_double_g()) {
                index = 0;
            }
        } else if self.keymap.matches(key_event, Action::ListBottom) {
            if len > 0 {
                index = len - 1;
            }
        } else if self.keymap.matches(key_event, Action::CancelInput) {
            self.focus = Focus::Input;
        }
        index
    }

    /// Detect a double-'g' press (gg) within a short window.
    pub(crate) fn is_double_g(&mut self) -> bool {
        let now = Instant::now();
        let double = match self.last_g_press {
            Some(t) => now.duration_since(t) < std::time::Duration::from_millis(500),
            None => false,
        };
        self.last_g_press = Some(now);
        double
    }
}
