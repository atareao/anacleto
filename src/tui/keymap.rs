use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    /// Clear the current input buffer.
    ClearInput,
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
        km.bind(
            Action::Quit,
            vec![key_event('q', false), key_event('q', true)],
        );
        km.bind(Action::ToggleSidebar, vec![key_event('b', true)]);
        km.bind(Action::ToggleDiffViewer, vec![key_event('g', true)]);
        km.bind(Action::OpenWhichKey, vec![key_event('?', false)]);
        km.bind(Action::OpenModelPicker, vec![key_event('m', true)]);
        km.bind(Action::OpenEditor, vec![key_event('e', true)]);
        km.bind(
            Action::ScrollUp,
            vec![
                KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
                key_event('u', true),
            ],
        );
        km.bind(
            Action::ScrollDown,
            vec![
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                key_event('d', true),
            ],
        );
        km.bind(
            Action::PageUp,
            vec![KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)],
        );
        km.bind(
            Action::PageDown,
            vec![KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)],
        );
        km.bind(
            Action::Approve,
            vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)],
        );
        km.bind(
            Action::Deny,
            vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)],
        );
        km.bind(Action::FocusInput, vec![key_event('i', false)]);
        km.bind(Action::FocusSidebar, vec![key_event('s', false)]);
        km.bind(Action::FocusChat, vec![key_event('c', false)]);
        km.bind(Action::ClearInput, vec![key_event('c', true)]);
        km
    }
}

/// Build a `KeyEvent` for a character, optionally with the Ctrl modifier.
pub fn key_event(ch: char, ctrl: bool) -> KeyEvent {
    let modifiers = if ctrl {
        KeyModifiers::CONTROL
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(ch), modifiers)
}

/// Render a human-readable table of every action and its bound keys.
/// Used by the which-key popup.
pub fn format_keymap_table() -> String {
    let km = Keymap::default();
    let rows: &[(Action, &str)] = &[
        (Action::Send, "Enviar mensaje"),
        (Action::CancelInput, "Cancelar / cerrar"),
        (Action::Quit, "Salir"),
        (Action::ToggleSidebar, "Mostrar/ocultar sidebar"),
        (Action::ToggleDiffViewer, "Abrir/cerrar diff viewer"),
        (Action::OpenWhichKey, "Mostrar atajos"),
        (Action::OpenModelPicker, "Cambiar modelo"),
        (Action::OpenEditor, "Abrir editor externo"),
        (Action::ScrollUp, "Scroll arriba"),
        (Action::ScrollDown, "Scroll abajo"),
        (Action::PageUp, "Página arriba"),
        (Action::PageDown, "Página abajo"),
        (Action::Approve, "Aprobar"),
        (Action::Deny, "Denegar"),
        (Action::FocusInput, "Enfocar input"),
        (Action::FocusSidebar, "Enfocar sidebar"),
        (Action::FocusChat, "Enfocar chat"),
        (Action::ClearInput, "Limpiar input"),
    ];

    let mut out = String::new();
    for (action, desc) in rows {
        let keys = km.resolve(*action);
        let key_str = keys.iter().map(format_key).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("  {:<18} {}\n", key_str, desc));
    }
    out
}

/// Format a single key event as a short human-readable string.
fn format_key(key: &KeyEvent) -> String {
    let mut s = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        s.push_str("Ctrl+");
    }
    match key.code {
        KeyCode::Char(c) => s.push(c),
        KeyCode::Enter => s.push_str("Enter"),
        KeyCode::Esc => s.push_str("Esc"),
        KeyCode::PageUp => s.push_str("PageUp"),
        KeyCode::PageDown => s.push_str("PageDown"),
        _ => s.push('?'),
    }
    s
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
    fn quit_bound_to_q_and_ctrl_q() {
        let km = Keymap::default();
        assert!(km.matches(key_event('q', false), Action::Quit));
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
        assert_eq!(keys, vec![key_event('?', false)]);
    }

    #[test]
    fn table_contains_all_actions() {
        let table = format_keymap_table();
        assert!(table.contains("Enviar mensaje"));
        assert!(table.contains("Salir"));
        assert!(table.contains("Ctrl+"));
    }
}
