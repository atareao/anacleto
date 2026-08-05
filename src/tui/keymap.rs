use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::config::types::KeymapConfig;

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
    /// Focus the MCPs sidebar panel.
    FocusMcps,
    /// Focus the Skills sidebar panel.
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
        km.bind(
            Action::FocusInput,
            vec![
                key_event('i', false),
                KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT),
            ],
        );
        km.bind(Action::FocusSidebar, vec![key_event('s', false)]);
        km.bind(
            Action::FocusChat,
            vec![
                key_event('c', false),
                KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            ],
        );
        km.bind(
            Action::FocusMcps,
            vec![KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusSkills,
            vec![KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT)],
        );
        km.bind(
            Action::FocusAgents,
            vec![KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT)],
        );
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
        (Action::FocusMcps, "Enfocar panel MCPs"),
        (Action::FocusSkills, "Enfocar panel Skills"),
        (Action::FocusAgents, "Enfocar panel Agents"),
        (Action::ClearInput, "Limpiar input"),
        (Action::OpenPromptQueue, "Cola de prompts"),
        (Action::QuickSlot1, "Quick slot 1"),
        (Action::QuickSlot2, "Quick slot 2"),
        (Action::QuickSlot3, "Quick slot 3"),
        (Action::QuickSlot4, "Quick slot 4"),
        (Action::QuickSlot5, "Quick slot 5"),
        (Action::QuickSlot6, "Quick slot 6"),
        (Action::QuickSlot7, "Quick slot 7"),
        (Action::QuickSlot8, "Quick slot 8"),
        (Action::QuickSlot9, "Quick slot 9"),
        (Action::CursorLeft, "Cursor izquierda"),
        (Action::CursorRight, "Cursor derecha"),
        (Action::CursorWordLeft, "Cursor palabra izquierda"),
        (Action::CursorWordRight, "Cursor palabra derecha"),
        (Action::CursorHome, "Cursor inicio de línea"),
        (Action::CursorEnd, "Cursor fin de línea"),
        (Action::DeleteChar, "Borrar carácter"),
        (Action::DeleteCharBefore, "Borrar carácter anterior"),
        (Action::DeleteWordBefore, "Borrar palabra anterior"),
        (Action::DeleteToStart, "Borrar hasta inicio"),
        (Action::DeleteToEnd, "Borrar hasta fin"),
        (Action::HistoryUp, "Historial atrás"),
        (Action::HistoryDown, "Historial adelante"),
        (Action::TabComplete, "Completar comando"),
        (Action::InsertNewline, "Insertar nueva línea"),
        (Action::ChatTop, "Ir al inicio del chat"),
        (Action::ChatBottom, "Ir al final del chat"),
        (Action::ListUp, "Lista arriba"),
        (Action::ListDown, "Lista abajo"),
        (Action::ListTop, "Ir al inicio de la lista"),
        (Action::ListBottom, "Ir al final de la lista"),
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
    if key.modifiers.contains(KeyModifiers::ALT) {
        s.push_str("Alt+");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        s.push_str("Shift+");
    }
    match key.code {
        KeyCode::Char(c) => s.push(c),
        KeyCode::Enter => s.push_str("Enter"),
        KeyCode::Esc => s.push_str("Esc"),
        KeyCode::Backspace => s.push_str("Backspace"),
        KeyCode::Tab => s.push_str("Tab"),
        KeyCode::Left => s.push_str("Left"),
        KeyCode::Right => s.push_str("Right"),
        KeyCode::Up => s.push_str("Up"),
        KeyCode::Down => s.push_str("Down"),
        KeyCode::Home => s.push_str("Home"),
        KeyCode::End => s.push_str("End"),
        KeyCode::Delete => s.push_str("Delete"),
        KeyCode::Insert => s.push_str("Insert"),
        KeyCode::PageUp => s.push_str("PageUp"),
        KeyCode::PageDown => s.push_str("PageDown"),
        KeyCode::F(n) => s.push_str(&format!("F{n}")),
        _ => s.push('?'),
    }
    s
}

/// Parse an action name (e.g. `ToggleSidebar`, `toggle_sidebar`) into an
/// [`Action`]. Returns `None` for unknown names.
fn parse_action(name: &str) -> Option<Action> {
    let normalized = name.replace(['-', ' '], "_");
    let lower = to_snake_case(&normalized);
    for action in [
        Action::Send,
        Action::CancelInput,
        Action::Quit,
        Action::ToggleSidebar,
        Action::ToggleDiffViewer,
        Action::OpenWhichKey,
        Action::OpenModelPicker,
        Action::OpenEditor,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::PageUp,
        Action::PageDown,
        Action::Approve,
        Action::Deny,
        Action::FocusInput,
        Action::FocusSidebar,
        Action::FocusChat,
        Action::FocusMcps,
        Action::FocusSkills,
        Action::FocusAgents,
        Action::ClearInput,
        Action::OpenPromptQueue,
        Action::QuickSlot1,
        Action::QuickSlot2,
        Action::QuickSlot3,
        Action::QuickSlot4,
        Action::QuickSlot5,
        Action::QuickSlot6,
        Action::QuickSlot7,
        Action::QuickSlot8,
        Action::QuickSlot9,
        Action::CursorLeft,
        Action::CursorRight,
        Action::CursorWordLeft,
        Action::CursorWordRight,
        Action::CursorHome,
        Action::CursorEnd,
        Action::DeleteChar,
        Action::DeleteCharBefore,
        Action::DeleteWordBefore,
        Action::DeleteToStart,
        Action::DeleteToEnd,
        Action::HistoryUp,
        Action::HistoryDown,
        Action::TabComplete,
        Action::InsertNewline,
        Action::ChatTop,
        Action::ChatBottom,
        Action::ListUp,
        Action::ListDown,
        Action::ListTop,
        Action::ListBottom,
    ] {
        if to_snake_case(&format!("{action:?}")) == lower {
            return Some(action);
        }
    }
    None
}

/// Convert a CamelCase string to snake_case (used for action names).
fn to_snake_case(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Parse a key string (e.g. `ctrl+b`, `alt+e`, `f2`, `enter`, `escape`,
/// `ctrl+shift+p`) into a [`KeyEvent`]. Returns `None` for invalid strings.
fn parse_key(s: &str) -> Option<KeyEvent> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key = s;
    for part in s.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => key = part,
        }
    }
    let mut modifiers = KeyModifiers::NONE;
    if ctrl {
        modifiers.insert(KeyModifiers::CONTROL);
    }
    if alt {
        modifiers.insert(KeyModifiers::ALT);
    }
    if shift {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    let code = match key.trim().to_lowercase().as_str() {
        "enter" => KeyCode::Enter,
        "escape" | "esc" => KeyCode::Esc,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "f1" => KeyCode::F(1),
        "f2" => KeyCode::F(2),
        "f3" => KeyCode::F(3),
        "f4" => KeyCode::F(4),
        "f5" => KeyCode::F(5),
        "f6" => KeyCode::F(6),
        "f7" => KeyCode::F(7),
        "f8" => KeyCode::F(8),
        "f9" => KeyCode::F(9),
        "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11),
        "f12" => KeyCode::F(12),
        other => {
            let mut chars = other.chars();
            let c = chars.next()?;
            if chars.next().is_some() || c.len_utf8() != 1 {
                return None;
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent::new(code, modifiers))
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

    #[test]
    fn alt_1_to_5_switch_focus() {
        let km = Keymap::default();
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT),
            Action::FocusChat
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT),
            Action::FocusMcps
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT),
            Action::FocusSkills
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT),
            Action::FocusAgents
        ));
        assert!(km.matches(
            KeyEvent::new(KeyCode::Char('5'), KeyModifiers::ALT),
            Action::FocusInput
        ));
    }

    #[test]
    fn focus_actions_keep_legacy_letter_bindings() {
        let km = Keymap::default();
        assert!(km.matches(key_event('c', false), Action::FocusChat));
        assert!(km.matches(key_event('i', false), Action::FocusInput));
    }

    #[test]
    fn parse_action_accepts_focus_actions() {
        assert_eq!(parse_action("FocusMcps"), Some(Action::FocusMcps));
        assert_eq!(parse_action("focus_skills"), Some(Action::FocusSkills));
        assert_eq!(parse_action("FocusAgents"), Some(Action::FocusAgents));
    }

    #[test]
    fn parse_action_accepts_camel_and_snake() {
        assert_eq!(parse_action("ToggleSidebar"), Some(Action::ToggleSidebar));
        assert_eq!(parse_action("toggle_sidebar"), Some(Action::ToggleSidebar));
        assert_eq!(
            parse_action("OpenModelPicker"),
            Some(Action::OpenModelPicker)
        );
        assert_eq!(parse_action("bogus"), None);
    }

    #[test]
    fn parse_key_handles_modifier_combos() {
        let ctrl_b = parse_key("ctrl+b").unwrap();
        assert_eq!(ctrl_b, key_event('b', true));
        let f2 = parse_key("F2").unwrap();
        assert_eq!(f2, KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        let esc = parse_key("escape").unwrap();
        assert_eq!(esc, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let ctrl_shift_p = parse_key("ctrl+shift+p").unwrap();
        assert!(ctrl_shift_p.modifiers.contains(KeyModifiers::CONTROL));
        assert!(ctrl_shift_p.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("abc"), None);
    }

    #[test]
    fn parse_key_handles_navigation_and_edit_keys() {
        assert_eq!(
            parse_key("home").unwrap(),
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("end").unwrap(),
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("delete").unwrap(),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("del").unwrap(),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("insert").unwrap(),
            KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE)
        );
        assert_eq!(
            parse_key("ctrl+home").unwrap(),
            KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn format_key_shows_modifiers_and_special_keys() {
        assert_eq!(format_key(&key_event('b', true)), "Ctrl+b");
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT)),
            "Alt+b"
        );
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL)),
            "Ctrl+Left"
        );
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            "Home"
        );
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)),
            "F9"
        );
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

    #[test]
    fn parse_action_round_trip_new_variants() {
        for action in [
            Action::CursorLeft,
            Action::CursorRight,
            Action::CursorWordLeft,
            Action::CursorWordRight,
            Action::CursorHome,
            Action::CursorEnd,
            Action::DeleteChar,
            Action::DeleteCharBefore,
            Action::DeleteWordBefore,
            Action::DeleteToStart,
            Action::DeleteToEnd,
            Action::HistoryUp,
            Action::HistoryDown,
            Action::TabComplete,
            Action::InsertNewline,
            Action::ChatTop,
            Action::ChatBottom,
            Action::ListUp,
            Action::ListDown,
            Action::ListTop,
            Action::ListBottom,
        ] {
            let name = format!("{action:?}");
            assert_eq!(parse_action(&name), Some(action), "camel: {name}");
            assert_eq!(
                parse_action(&to_snake_case(&name)),
                Some(action),
                "snake: {}",
                to_snake_case(&name)
            );
        }
    }
}
