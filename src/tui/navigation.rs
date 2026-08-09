//! Key handling for the non-input panels (Chat, Info (Skills/MCPs), Agents,
//! Queue).
//!
//! Contains the `App` methods that route keys while one of the sidebar/chat
//! panels has focus, plus the shared list-navigation helper and the
//! double-`g` (gg) detection.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::App;
use super::types::Focus;
use crate::tui::keymap::Action;

impl App {
    /// Handle a key while the Chat window (2) has focus.
    pub(crate) fn handle_chat_key(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        if self.keymap.matches(key_event, Action::ScrollUp) {
            self.chat_scroll = self.chat_scroll.saturating_add(1);
        } else if self.keymap.matches(key_event, Action::ScrollDown) {
            self.chat_scroll = self.chat_scroll.saturating_sub(1);
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

    /// Handle a key while the Info panel (3) has focus — the unified
    /// Skills/MCPs tabbed panel.
    pub(crate) fn handle_info_panel_key(
        &mut self,
        key: KeyCode,
        modifiers: KeyModifiers,
        key_event: KeyEvent,
    ) {
        // Left/Right (and vim h/l) cycle through the Skills/MCPs/SubAgents tabs.
        match key {
            KeyCode::Right | KeyCode::Char('l') => {
                self.info_tab = (self.info_tab + 1) % 3;
                return;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.info_tab = self.info_tab.saturating_sub(1);
                return;
            }
            _ => {}
        }

        // Up/Down (and list navigation) move the selection within the active tab.
        let (len, index) = if self.info_tab == 0 {
            (self.unique_skill_count(), self.skill_panel_index)
        } else if self.info_tab == 1 {
            (self.unique_mcp_count(), self.mcp_panel_index)
        } else {
            (self.unique_subagent_count(), self.subagent_panel_index)
        };
        let new_index = self.handle_list_nav_key(key, modifiers, key_event, len, index);
        if self.info_tab == 0 {
            self.skill_panel_index = new_index;
        } else if self.info_tab == 1 {
            self.mcp_panel_index = new_index;
        } else {
            self.subagent_panel_index = new_index;
        }
    }

    /// Handle a key while the Queue panel (5) has focus — the visible,
    /// interactive prompt queue.
    pub(crate) fn handle_queue_panel_key(
        &mut self,
        key: KeyCode,
        _modifiers: KeyModifiers,
        _key_event: KeyEvent,
    ) {
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
                // Send the selected item: remove it, then hand it to the engine.
                if let Some(prompt) = self.prompt_queue.get(self.prompt_queue_index) {
                    let text = prompt.clone();
                    self.prompt_queue.remove(self.prompt_queue_index);
                    // Send through the shared helper so `sent_message` is marked
                    // (guards against double-send while the agent is busy).
                    if !self.send_prompt(text.clone()) {
                        // Send failed (channel full/closed): re-queue so it is
                        // not lost; it will be retried.
                        self.prompt_queue
                            .insert(self.prompt_queue_index.min(self.prompt_queue.len()), text);
                    }
                    if self.prompt_queue.is_empty() {
                        self.prompt_queue_index = 0;
                    } else {
                        self.prompt_queue_index =
                            self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                    }
                }
            }
            KeyCode::Char('d') => {
                // Delete the selected item.
                if !self.prompt_queue.is_empty() {
                    self.prompt_queue.remove(self.prompt_queue_index);
                    if self.prompt_queue.is_empty() {
                        self.prompt_queue_index = 0;
                    } else {
                        self.prompt_queue_index =
                            self.prompt_queue_index.min(self.prompt_queue.len() - 1);
                    }
                }
            }
            KeyCode::Char('e') => {
                // Edit: load the selected item into the input buffer, remove it
                // from the queue, and move focus to Input.
                if let Some(prompt) = self.prompt_queue.get(self.prompt_queue_index) {
                    self.input = prompt.clone();
                    self.input_cursor = self.input.chars().count();
                    self.prompt_queue.remove(self.prompt_queue_index);
                    self.focus = Focus::Input;
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
                self.focus = Focus::Input;
            }
            _ => {}
        }
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

        // Enter switches to the selected agent
        if key == KeyCode::Enter {
            let display_agents: Vec<&crate::tui::types::AgentInfo> = self
                .agents
                .iter()
                .filter(|a| a.status != crate::agent::types::AgentStatus::Completed)
                .collect();
            if let Some(agent) = display_agents.get(self.agent_panel_index) {
                if agent.name != self.active_agent {
                    let name = agent.name.clone();
                    let _ = self.cmd_tx.try_send(
                        crate::engine::orchestrator::EngineCommand::SwitchAgent(name),
                    );
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{AgentId, AgentRole, AgentStatus};
    use crate::config::Config;
    use crate::engine::orchestrator::EngineCommand;
    use crate::tui::types::AgentInfo;
    use tokio::sync::mpsc;

    fn test_app() -> App {
        let (cmd_tx, _cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        App::new(cmd_tx, event_rx, false, &Config::default())
    }

    fn agent_with_skills(n: usize) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: "agent".to_string(),
            role: AgentRole::Root,
            status: AgentStatus::Idle,
            skills: (0..n).map(|i| format!("skill{i}")).collect(),
            mcps: Vec::new(),
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: None,
            mode: None,
        }
    }

    fn agent_with_mcps(n: usize) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: "agent".to_string(),
            role: AgentRole::Root,
            status: AgentStatus::Idle,
            skills: Vec::new(),
            mcps: (0..n).map(|i| format!("mcp{i}")).collect(),
            model: String::new(),
            parent_id: None,
            subagent_count: 0,
            agent_type: None,
            mode: None,
        }
    }

    fn key(code: KeyCode) -> (KeyCode, KeyEvent, KeyModifiers) {
        (
            code,
            KeyEvent::new(code, KeyModifiers::NONE),
            KeyModifiers::NONE,
        )
    }

    #[test]
    fn info_tab_advances_on_right() {
        let mut app = test_app();
        assert_eq!(app.info_tab, 0);
        let (c, ev, m) = key(KeyCode::Right);
        app.handle_info_panel_key(c, m, ev);
        assert_eq!(app.info_tab, 1);
        app.handle_info_panel_key(c, m, ev);
        assert_eq!(app.info_tab, 2);
        // Third Right wraps back to 0 (3 tabs).
        app.handle_info_panel_key(c, m, ev);
        assert_eq!(app.info_tab, 0);
    }

    #[test]
    fn info_tab_shift_tab_goes_backward() {
        let mut app = test_app();
        app.info_tab = 1;
        // Shift+Tab is no longer handled in info panel (Tab cycles focus);
        // use Left for going backward.
        let ev = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        app.handle_info_panel_key(KeyCode::Left, KeyModifiers::NONE, ev);
        assert_eq!(app.info_tab, 0);
        // Already at the first tab: stays at 0 (saturating, no wrap).
        app.handle_info_panel_key(KeyCode::Left, KeyModifiers::NONE, ev);
        assert_eq!(app.info_tab, 0);
    }

    #[test]
    fn info_tab_left_right_changes_tab() {
        let mut app = test_app();
        let (r, ev, m) = key(KeyCode::Right);
        app.handle_info_panel_key(r, m, ev);
        assert_eq!(app.info_tab, 1);
        let (l, ev, m) = key(KeyCode::Left);
        app.handle_info_panel_key(l, m, ev);
        assert_eq!(app.info_tab, 0);
    }

    #[test]
    fn info_tab_skill_down_navigates_skill_list() {
        let mut app = test_app();
        app.info_tab = 0;
        app.active_agent = "agent".to_string();
        app.agents.push(agent_with_skills(3));
        app.skill_panel_index = 0;
        let (d, ev, m) = key(KeyCode::Down);
        app.handle_info_panel_key(d, m, ev);
        assert_eq!(app.skill_panel_index, 1);
        let (u, ev, m) = key(KeyCode::Up);
        app.handle_info_panel_key(u, m, ev);
        assert_eq!(app.skill_panel_index, 0);
    }

    #[test]
    fn info_tab_mcp_down_navigates_mcp_list() {
        let mut app = test_app();
        app.info_tab = 1;
        app.active_agent = "agent".to_string();
        app.agents.push(agent_with_mcps(3));
        app.mcp_panel_index = 0;
        let (d, ev, m) = key(KeyCode::Down);
        app.handle_info_panel_key(d, m, ev);
        assert_eq!(app.mcp_panel_index, 1);
    }

    #[test]
    fn info_tab_right_switch_does_not_mutate_panel_indices() {
        let mut app = test_app();
        app.agents.push(agent_with_skills(3));
        app.skill_panel_index = 2;
        let (t, ev, m) = key(KeyCode::Right);
        app.handle_info_panel_key(t, m, ev);
        assert_eq!(app.info_tab, 1);
        assert_eq!(
            app.skill_panel_index, 2,
            "tab switch must not move the list"
        );
    }

    #[test]
    fn queue_enter_sends_selected_item_and_removes() {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(16);
        let (_ev_tx, event_rx) = mpsc::channel(16);
        let mut app = App::new(cmd_tx, event_rx, false, &Config::default());
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];
        app.prompt_queue_index = 1;

        let (e, ev, m) = key(KeyCode::Enter);
        app.handle_queue_panel_key(e, m, ev);

        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        match cmd_rx.try_recv() {
            Ok(EngineCommand::UserInput(text)) => assert_eq!(text, "second"),
            other => panic!("expected UserInput, got {:?}", other),
        }
    }

    #[test]
    fn queue_d_deletes_selected_item() {
        let mut app = test_app();
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];
        app.prompt_queue_index = 0;

        let (d, ev, m) = key(KeyCode::Char('d'));
        app.handle_queue_panel_key(d, m, ev);

        assert_eq!(app.prompt_queue, vec!["second".to_string()]);
    }

    #[test]
    fn queue_e_edits_selected_item_and_focuses_input() {
        let mut app = test_app();
        app.focus = Focus::Queue;
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];
        app.prompt_queue_index = 1;

        let (e, ev, m) = key(KeyCode::Char('e'));
        app.handle_queue_panel_key(e, m, ev);

        assert_eq!(app.input, "second");
        assert_eq!(app.input_cursor, 6);
        assert_eq!(app.prompt_queue, vec!["first".to_string()]);
        assert_eq!(app.focus, Focus::Input);
    }

    #[test]
    fn queue_bracket_moves_selected_item_up_and_down() {
        let mut app = test_app();
        app.prompt_queue = vec!["first".to_string(), "second".to_string()];
        app.prompt_queue_index = 1;

        let (lb, ev, m) = key(KeyCode::Char('['));
        app.handle_queue_panel_key(lb, m, ev);
        assert_eq!(
            app.prompt_queue,
            vec!["second".to_string(), "first".to_string()]
        );
        assert_eq!(app.prompt_queue_index, 0);

        let (rb, ev, m) = key(KeyCode::Char(']'));
        app.handle_queue_panel_key(rb, m, ev);
        assert_eq!(
            app.prompt_queue,
            vec!["first".to_string(), "second".to_string()]
        );
        assert_eq!(app.prompt_queue_index, 1);
    }

    #[test]
    fn queue_up_down_navigates() {
        let mut app = test_app();
        app.prompt_queue = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        app.prompt_queue_index = 1;

        let (u, ev, m) = key(KeyCode::Up);
        app.handle_queue_panel_key(u, m, ev);
        assert_eq!(app.prompt_queue_index, 0);

        let (d, ev, m) = key(KeyCode::Down);
        app.handle_queue_panel_key(d, m, ev);
        assert_eq!(app.prompt_queue_index, 1);
    }
}
