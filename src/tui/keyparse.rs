//! Key parsing and formatting helpers for the TUI.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keymap::{Action, Keymap};

/// Build a `KeyEvent` for a character, optionally with the Ctrl modifier.
pub(crate) fn key_event(ch: char, ctrl: bool) -> KeyEvent {
    let modifiers = if ctrl {
        KeyModifiers::CONTROL
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(ch), modifiers)
}

/// Render a human-readable table of every action and its bound keys.
/// Used by the which-key popup.
pub(crate) fn format_keymap_table() -> String {
    let km = Keymap::default();
    let rows: &[(Action, &str)] = &[
        (Action::Send, "Enviar mensaje"),
        (Action::CancelInput, "Cancelar / cerrar"),
        (Action::Quit, "Salir"),
        (Action::ToggleSidebar, "Mostrar/ocultar sidebar"),
        (Action::ToggleDiffViewer, "Abrir/cerrar diff viewer"),
        (Action::OpenWhichKey, "Mostrar atajos"),
        (Action::OpenModelPicker, "Cambiar modelo"),
        (Action::OpenAgentPicker, "Cambiar agente"),
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
        (Action::FocusInfo, "Enfocar panel Info"),
        (Action::FocusQueue, "Enfocar panel Queue"),
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
        (Action::FocusNext, "Siguiente panel"),
        (Action::FocusPrev, "Panel anterior"),
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
pub(crate) fn parse_action(name: &str) -> Option<Action> {
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
        Action::OpenAgentPicker,
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
        Action::FocusInfo,
        Action::FocusQueue,
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
        Action::FocusNext,
        Action::FocusPrev,
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
pub(crate) fn parse_key(s: &str) -> Option<KeyEvent> {
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
    fn table_contains_all_actions() {
        let table = format_keymap_table();
        assert!(table.contains("Enviar mensaje"));
        assert!(table.contains("Salir"));
        assert!(table.contains("Ctrl+"));
    }

    #[test]
    fn parse_action_accepts_focus_actions() {
        assert_eq!(parse_action("FocusMcps"), Some(Action::FocusMcps));
        assert_eq!(parse_action("focus_skills"), Some(Action::FocusSkills));
        assert_eq!(parse_action("FocusAgents"), Some(Action::FocusAgents));
        assert_eq!(parse_action("FocusInfo"), Some(Action::FocusInfo));
        assert_eq!(parse_action("focus_queue"), Some(Action::FocusQueue));
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
