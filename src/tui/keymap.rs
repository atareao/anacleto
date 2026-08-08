use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::types::KeymapConfig;
use crate::tui::keyparse::{key_event, parse_action, parse_key};

/// A user-facing action that can be triggered by one or more key bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Submit the current input / send a message.
    Send,
    /// Cancel the current input or close an overlay.
    CancelInput,
    /// Quit the application.
    Quit,
    /// Toggle the right-hand sidebar panels.
    ToggleSidebar,
    /// Open/close the diff viewer.
    ToggleDiffViewer,
    /// Show the which-key popup.
    OpenWhichKey,
    /// Open the model picker.
    OpenModelPicker,
    /// Open the external editor.
    OpenEditor,
    /// Scroll the chat up by one line.
    ScrollUp,
    /// Scroll the chat down by one line.
    ScrollDown,
    /// Scroll the chat up by a page.
    PageUp,
    /// Scroll the chat down by a page.
    PageDown,
    /// Approve a pending human-in-the-loop request.
    Approve,
    /// Deny a pending human-in-the-loop request.
    Deny,
    /// Focus the input area.
    FocusInput,
    /// Focus the sidebar.
    FocusSidebar,
    /// Focus the chat area.
    FocusChat,
    /// Focus the Info panel (Skills/MCPs tabs).
    FocusInfo,
    /// Focus the Queue panel.
    FocusQueue,
    /// Focus the MCPs sidebar panel (config-compat; maps to Info tab MCPs).
    FocusMcps,
    /// Focus the Skills sidebar panel (config-compat; maps to Info tab Skills).
    FocusSkills,
    /// Focus the Agents sidebar panel.
    FocusAgents,
    /// Clear the current input buffer.
    ClearInput,
    /// Open the prompt queue popup.
    OpenPromptQueue,
    /// Resume the pinned session in quick slot 1..9.
    QuickSlot1,
    QuickSlot2,
    QuickSlot3,
    QuickSlot4,
    QuickSlot5,
    QuickSlot6,
    QuickSlot7,
    QuickSlot8,
    QuickSlot9,
    // ── Input editing ──────────────────────────────────────────────
    /// Move the cursor one character to the left.
    CursorLeft,
    /// Move the cursor one character to the right.
    CursorRight,
    /// Move the cursor to the start of the previous word.
    CursorWordLeft,
    /// Move the cursor to the start of the next word.
    CursorWordRight,
    /// Move the cursor to the start of the line.
    CursorHome,
    /// Move the cursor to the end of the line.
    CursorEnd,
    /// Delete the character at the cursor.
    DeleteChar,
    /// Delete the character before the cursor.
    DeleteCharBefore,
    /// Delete the word before the cursor.
    DeleteWordBefore,
    /// Delete from the start of the line to the cursor.
    DeleteToStart,
    /// Delete from the cursor to the end of the line.
    DeleteToEnd,
    /// Navigate backwards through input history.
    HistoryUp,
    /// Navigate forwards through input history.
    HistoryDown,
    /// Complete the current command from the palette.
    TabComplete,
    /// Insert a newline into the input.
    InsertNewline,
    /// Open the conversation history search overlay (Ctrl+R style).
    ToggleSearch,
    // ── Chat navigation ────────────────────────────────────────────
    /// Jump to the top of the chat.
    ChatTop,
    /// Jump to the bottom of the chat.
    ChatBottom,
    // ── List navigation (MCPs, Skills, Agents) ─────────────────────
    /// Move the selection up in a list panel.
    ListUp,
    /// Move the selection down in a list panel.
    ListDown,
    /// Jump to the top of a list panel.
    ListTop,
    /// Jump to the bottom of a list panel.
    ListBottom,
}

/// Central mapping of actions to the key events that trigger them.
///
/// Multiple keys can be bound to a single action. The defaults are sensible
/// for a TUI; the map is intentionally data-driven so it can later be loaded
/// from configuration.
pub struct Keymap {
    bindings: HashMap<Action, Vec<KeyEvent>>,
}

impl Keymap {
    /// Create an empty keymap.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Bind one or more keys to an action (replacing any previous bindings).
    pub fn bind(&mut self, action: Action, keys: Vec<KeyEvent>) {
        self.bindings.insert(action, keys);
    }

    /// Return the keys currently bound to an action (empty if none).
    pub fn resolve(&self, action: Action) -> Vec<KeyEvent> {
        self.bindings.get(&action).cloned().unwrap_or_default()
    }

    /// Whether the given key event triggers the given action.
    pub fn matches(&self, key: KeyEvent, action: Action) -> bool {
        self.bindings
            .get(&action)
            .map(|keys| keys.contains(&key))
            .unwrap_or(false)
    }

    /// Apply user-provided keybinding overrides from configuration.
    ///
    /// Each entry maps an action name (e.g. `ToggleSidebar`) to a list of key
    /// strings (e.g. `ctrl+b`, `f2`, `enter`). Invalid action names or key
    /// strings are silently ignored; valid bindings replace the defaults.
    pub fn apply_overrides(&mut self, overrides: &KeymapConfig) {
        for (action_name, keys) in &overrides.bindings {
            let Some(action) = parse_action(action_name) else {
                continue;
            };
            let mut parsed = Vec::new();
            for k in keys {
                if let Some(ke) = parse_key(k) {
                    parsed.push(ke);
                }
            }
            if !parsed.is_empty() {
                self.bind(action, parsed);
            }
        }
    }
}

impl Default for Keymap {
    /// Sensible default bindings for the whole application.
    fn default() -> Self {
        let mut km = Self::new();
        km.bind(
            Action::Send,
            vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
        );
        km.bind(
            Action::CancelInput,
            vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        );
        km.bind(Action::Quit, vec![key_event('q', true)]);
        km.bind(Action::ToggleSidebar, vec![key_event('b', true)]);
        km.bind(Action::ToggleDiffViewer, vec![key_event('g', true)]);
        km.bind(Action::OpenWhichKey, vec![key_event('x', true)]);
        km.bind(Action::OpenModelPicker, vec![key_event('m', true)]);
        km.bind(Action::OpenEditor, vec![key_event('e', true)]);
        km.bind(
            Action::ScrollUp,
            vec![
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                key_event('k', false),
            ],
        );
        km.bind(
            Action::ScrollDown,
            vec![
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                key_event('j', false),
            ],
        );
        km.bind(
            Action::PageUp,
            vec![
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
                key_event('u', true),
            ],
        );
        km.bind(
            Action::PageDown,
            vec![
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                key_event('d', true),
            ],
        );
        km.bind(
            Action::Approve,
            vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
        );
        km.bind(
            Action::Deny,
            vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        );
        km.bind(Action::FocusSidebar, vec![]);
        km.bind(
            Action::FocusChat,
            vec![KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusInfo,
            vec![KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusAgents,
            vec![KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusQueue,
            vec![KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusInput,
            vec![KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT)],
        );
        // FocusMcps / FocusSkills retain no default binding but remain
        // parseable for config compatibility.
        km.bind(Action::FocusMcps, vec![]);
        km.bind(Action::FocusSkills, vec![]);
        km.bind(Action::ClearInput, vec![key_event('c', true)]);
        km.bind(Action::OpenPromptQueue, vec![key_event('q', true)]);
        km.bind(Action::QuickSlot1, vec![key_event('1', true)]);
        km.bind(Action::QuickSlot2, vec![key_event('2', true)]);
        km.bind(Action::QuickSlot3, vec![key_event('3', true)]);
        km.bind(Action::QuickSlot4, vec![key_event('4', true)]);
        km.bind(Action::QuickSlot5, vec![key_event('5', true)]);
        km.bind(Action::QuickSlot6, vec![key_event('6', true)]);
        km.bind(Action::QuickSlot7, vec![key_event('7', true)]);
        km.bind(Action::QuickSlot8, vec![key_event('8', true)]);
        km.bind(Action::QuickSlot9, vec![key_event('9', true)]);

        // ── Input editing ──────────────────────────────────────────
        km.bind(
            Action::CursorLeft,
            vec![KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)],
        );
        km.bind(
            Action::CursorRight,
            vec![KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)],
        );
        km.bind(
            Action::CursorWordLeft,
            vec![
                KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Left, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
            ],
        );
        km.bind(
            Action::CursorWordRight,
            vec![
                KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Right, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
            ],
        );
        km.bind(
            Action::CursorHome,
            vec![
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                key_event('a', true),
            ],
        );
        km.bind(
            Action::CursorEnd,
            vec![
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
                key_event('e', true),
            ],
        );
        km.bind(
            Action::DeleteChar,
            vec![KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)],
        );
        km.bind(
            Action::DeleteCharBefore,
            vec![KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)],
        );
        km.bind(
            Action::DeleteWordBefore,
            vec![
                key_event('w', true),
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
            ],
        );
        km.bind(Action::DeleteToStart, vec![key_event('u', true)]);
        km.bind(Action::DeleteToEnd, vec![key_event('k', true)]);
        km.bind(
            Action::HistoryUp,
            vec![KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)],
        );
        km.bind(
            Action::HistoryDown,
            vec![KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)],
        );
        km.bind(
            Action::TabComplete,
            vec![KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)],
        );
        km.bind(
            Action::InsertNewline,
            vec![
                key_event('j', true),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            ],
        );

        // ── History search ───────────────────────────────────────────
        km.bind(
            Action::ToggleSearch,
            vec![KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)],
        );

        // ── Chat navigation ────────────────────────────────────────
        km.bind(
            Action::ChatTop,
            vec![
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                key_event('g', false),
            ],
        );
        km.bind(
            Action::ChatBottom,
            vec![
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
                key_event('G', false),
            ],
        );

        // ── List navigation (MCPs, Skills, Agents) ─────────────────
        km.bind(
            Action::ListUp,
            vec![
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                key_event('k', false),
            ],
        );
        km.bind(
            Action::ListDown,
            vec![
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                key_event('j', false),
            ],
        );
        km.bind(
            Action::ListTop,
            vec![
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
                key_event('g', false),
            ],
        );
        km.bind(
            Action::ListBottom,
            vec![
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
                key_event('G', false),
            ],
        );
        km
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_send_bound_to_enter() {
        let km = Keymap::default();
        assert!(km.matches(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Action::Send
        ));
        assert!(!km.matches(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            Action::Send
        ));
    }

    #[test]
    fn quit_bound_only_to_ctrl_q() {
        let km = Keymap::default();
        assert!(!km.matches(key_event('q', false), Action::Quit));
        assert!(km.matches(key_event('q', true), Action::Quit));
    }

    #[test]
    fn bind_replaces_previous() {
        let mut km = Keymap::new();
        km.bind(Action::Send, vec![key_event('a', false)]);
        km.bind(Action::Send, vec![key_event('b', false)]);
        assert!(km.matches(key_event('b', false), Action::Send));
        assert!(!km.matches(key_event('a', false), Action::Send));
    }

    #[test]
    fn resolve_returns_bound_keys() {
        let km = Keymap::default();
        let keys = km.resolve(Action::OpenWhichKey);
        assert_eq!(keys, vec![key_event('x', true)]);
    }

    #[test]
    fn alt_1_to_5_switch_focus() {
        let km = Keymap::default();
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            Action::FocusChat
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT),
            Action::FocusInfo
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT),
            Action::FocusAgents
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT),
            Action::FocusQueue
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT),
            Action::FocusInput
        ));
    }

    #[test]
    fn focus_actions_use_modified_keys_only() {
        let km = Keymap::default();
        // Plain letters must NOT trigger focus switches (so typing is never
        // intercepted in the Input window).
        assert!(!km.matches(key_event('c', false), Action::FocusChat));
        assert!(!km.matches(key_event('i', false), Action::FocusInput));
        assert!(!km.matches(key_event('s', false), Action::FocusSidebar));
        // Modified keys still switch focus.
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            Action::FocusChat
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT),
            Action::FocusInput
        ));
    }

    #[test]
    fn apply_overrides_replaces_bindings() {
        let mut km = Keymap::default();
        let mut bindings = std::collections::HashMap::new();
        bindings.insert("toggle_sidebar".to_string(), vec!["f9".to_string()]);
        bindings.insert("bogus".to_string(), vec!["f1".to_string()]);
        let cfg = KeymapConfig { bindings };
        km.apply_overrides(&cfg);
        assert!(km.matches(
            KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE),
            Action::ToggleSidebar
        ));
        assert!(!km.matches(key_event('b', true), Action::ToggleSidebar));
        // Unbound action still has its defaults.
        assert!(km.matches(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Action::Send
        ));
    }

    #[test]
    fn input_editing_actions_resolve_to_defaults() {
        let km = Keymap::default();
        assert!(km.matches(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Action::CursorLeft
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Action::CursorRight
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
            Action::CursorWordLeft
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
            Action::CursorWordLeft
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
            Action::CursorWordRight
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
            Action::CursorWordRight
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            Action::CursorHome
        ));
        assert!(km.matches(key_event('a', true), Action::CursorHome));
        assert!(km.matches(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            Action::CursorEnd
        ));
        assert!(km.matches(key_event('e', true), Action::CursorEnd));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
            Action::DeleteChar
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            Action::DeleteCharBefore
        ));
        assert!(km.matches(key_event('w', true), Action::DeleteWordBefore));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT),
            Action::DeleteWordBefore
        ));
        assert!(km.matches(key_event('u', true), Action::DeleteToStart));
        assert!(km.matches(key_event('k', true), Action::DeleteToEnd));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::HistoryUp
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::HistoryDown
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            Action::TabComplete
        ));
        assert!(km.matches(key_event('j', true), Action::InsertNewline));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            Action::InsertNewline
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            Action::InsertNewline
        ));
    }

    #[test]
    fn chat_and_list_nav_actions_resolve_to_defaults() {
        let km = Keymap::default();
        assert!(km.matches(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::ScrollUp
        ));
        assert!(km.matches(key_event('k', false), Action::ScrollUp));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::ScrollDown
        ));
        assert!(km.matches(key_event('j', false), Action::ScrollDown));
        assert!(km.matches(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            Action::PageUp
        ));
        assert!(km.matches(key_event('u', true), Action::PageUp));
        assert!(km.matches(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            Action::PageDown
        ));
        assert!(km.matches(key_event('d', true), Action::PageDown));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            Action::ChatTop
        ));
        assert!(km.matches(key_event('g', false), Action::ChatTop));
        assert!(km.matches(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            Action::ChatBottom
        ));
        assert!(km.matches(key_event('G', false), Action::ChatBottom));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::ListUp
        ));
        assert!(km.matches(key_event('k', false), Action::ListUp));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::ListDown
        ));
        assert!(km.matches(key_event('j', false), Action::ListDown));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            Action::ListTop
        ));
        assert!(km.matches(key_event('g', false), Action::ListTop));
        assert!(km.matches(
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            Action::ListBottom
        ));
        assert!(km.matches(key_event('G', false), Action::ListBottom));
    }

    #[test]
    fn apply_overrides_overrides_new_binding() {
        let mut km = Keymap::default();
        let mut bindings = std::collections::HashMap::new();
        bindings.insert("cursor_left".to_string(), vec!["f10".to_string()]);
        let cfg = KeymapConfig { bindings };
        km.apply_overrides(&cfg);
        assert!(km.matches(
            KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE),
            Action::CursorLeft
        ));
        assert!(!km.matches(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Action::CursorLeft
        ));
    }
}
