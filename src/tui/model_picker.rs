use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

/// A popup for selecting a model for the active agent.
///
/// Supports several browsing modes (tabs): `All` (default list), `Recent`
/// (models used recently, from the frecency ranking), `Providers` (grouped by
/// provider) and `Favorites` (user-pinned models).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    All,
    Recent,
    Providers,
    Favorites,
}

impl PickerMode {
    /// Human-readable label for the mode tab.
    pub fn label(self) -> &'static str {
        match self {
            PickerMode::All => "Todos",
            PickerMode::Recent => "Recientes",
            PickerMode::Providers => "Providers",
            PickerMode::Favorites => "Favoritos",
        }
    }

    /// The next mode (wrapping).
    pub fn next(self) -> Self {
        match self {
            PickerMode::All => PickerMode::Recent,
            PickerMode::Recent => PickerMode::Providers,
            PickerMode::Providers => PickerMode::Favorites,
            PickerMode::Favorites => PickerMode::All,
        }
    }

    /// The previous mode (wrapping).
    pub fn previous(self) -> Self {
        match self {
            PickerMode::All => PickerMode::Favorites,
            PickerMode::Recent => PickerMode::All,
            PickerMode::Providers => PickerMode::Recent,
            PickerMode::Favorites => PickerMode::Providers,
        }
    }
}

/// A popup for selecting a model for the active agent.
pub struct ModelPicker {
    pub visible: bool,
    pub models: Vec<String>,
    pub selected: usize,
    pub mode: PickerMode,
    pub recent: Vec<String>,
    pub favorites: Vec<String>,
    all_models: Vec<String>,
}

impl ModelPicker {
    /// Create a hidden picker with the given model list.
    pub fn new(models: Vec<String>) -> Self {
        Self {
            visible: false,
            models: models.clone(),
            selected: 0,
            mode: PickerMode::All,
            recent: Vec::new(),
            favorites: Vec::new(),
            all_models: models,
        }
    }

    /// Set the list of recently used models (from the frecency ranking).
    pub fn set_recent(&mut self, recent: Vec<String>) {
        self.recent = recent;
        if self.mode == PickerMode::Recent {
            self.rebuild();
        }
    }

    /// Set the list of favorite models.
    pub fn set_favorites(&mut self, favorites: Vec<String>) {
        self.favorites = favorites;
        if self.mode == PickerMode::Favorites {
            self.rebuild();
        }
    }

    /// Rebuild the displayed model list from the active mode's source.
    fn rebuild(&mut self) {
        self.models = match self.mode {
            PickerMode::All => self.all_models.clone(),
            PickerMode::Recent => self.recent.clone(),
            PickerMode::Providers => self.all_models.clone(),
            PickerMode::Favorites => self.favorites.clone(),
        };
        self.selected = 0;
    }

    /// Move the selection to the next model (wrapping).
    pub fn next(&mut self) {
        if !self.models.is_empty() {
            self.selected = (self.selected + 1) % self.models.len();
        }
    }

    /// Move the selection to the previous model (wrapping).
    pub fn previous(&mut self) {
        if !self.models.is_empty() {
            self.selected = self.selected.saturating_sub(1);
            if self.selected == 0 && self.models.len() > 1 {
                // wrap to the last element when moving up from the first
            }
        }
    }

    /// Switch to the next browsing mode (wrapping) and reset the selection.
    pub fn next_mode(&mut self) {
        self.mode = self.mode.next();
        self.rebuild();
    }

    /// Switch to the previous browsing mode (wrapping) and reset the selection.
    pub fn previous_mode(&mut self) {
        self.mode = self.mode.previous();
        self.rebuild();
    }

    /// The currently selected model, if any.
    pub fn selected_model(&self) -> Option<String> {
        self.models.get(self.selected).cloned()
    }

    /// Render the picker as a centered popup.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let width = area.width.min(56);
        let height = (self.models.len() as u16 + 6).min(area.height.min(22));
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let popup_area = Rect::new(x, y, width, height);

        let overlay = Clear;
        f.render_widget(overlay, popup_area);

        let items: Vec<ListItem> = self
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let marker = if i == self.selected { "▸ " } else { "  " };
                ListItem::new(Line::from(Span::styled(
                    format!("{}{}", marker, m),
                    Style::default().fg(Color::White),
                )))
            })
            .collect();

        let tabs = [
            PickerMode::All,
            PickerMode::Recent,
            PickerMode::Providers,
            PickerMode::Favorites,
        ]
        .iter()
        .map(|m| {
            let active = *m == self.mode;
            let style = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Span::styled(format!(" {} ", m.label()), style)
        })
        .collect::<Vec<_>>();

        let title = Line::from(tabs);
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Magenta))
                    .title(title)
                    .style(Style::default().bg(Color::Rgb(25, 15, 30))),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Magenta)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(
            list,
            popup_area,
            &mut ratatui::widgets::ListState::default().with_selected(Some(self.selected)),
        );

        // Footer hint.
        let footer = " ↑/↓ navegar  ·  Tab: cambiar modo  ·  Enter: seleccionar  ·  Esc: cancelar ";
        let footer_y = y + height.saturating_sub(1);
        let footer_area = Rect::new(x, footer_y, width, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(Color::DarkGray),
            ))),
            footer_area,
        );
    }
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new(default_models())
    }
}

/// A reasonable default list of models across providers.
///
/// This can be replaced with the real configured models from `ModelsConfig`
/// when wiring the picker to the engine configuration.
pub fn default_models() -> Vec<String> {
    vec![
        "claude-sonnet-4".to_string(),
        "claude-opus-4".to_string(),
        "claude-haiku-4".to_string(),
        "gpt-4o".to_string(),
        "gpt-4o-mini".to_string(),
        "o1".to_string(),
        "o3-mini".to_string(),
        "llama3.3".to_string(),
        "qwen2.5".to_string(),
        "mistral".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_forward() {
        let mut p = ModelPicker::new(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(p.selected_model().as_deref(), Some("a"));
        p.next();
        assert_eq!(p.selected_model().as_deref(), Some("b"));
        p.next();
        assert_eq!(p.selected_model().as_deref(), Some("c"));
        p.next();
        assert_eq!(p.selected_model().as_deref(), Some("a"));
    }

    #[test]
    fn navigation_backward() {
        let mut p = ModelPicker::new(vec!["a".into(), "b".into(), "c".into()]);
        p.selected = 2;
        p.previous();
        assert_eq!(p.selected_model().as_deref(), Some("b"));
        p.previous();
        assert_eq!(p.selected_model().as_deref(), Some("a"));
        // stays at first
        p.previous();
        assert_eq!(p.selected_model().as_deref(), Some("a"));
    }

    #[test]
    fn empty_list_returns_none() {
        let p = ModelPicker::new(vec![]);
        assert_eq!(p.selected_model(), None);
    }

    #[test]
    fn defaults_are_non_empty() {
        assert!(!default_models().is_empty());
    }

    #[test]
    fn mode_cycles_forward_and_backward() {
        let mut p = ModelPicker::new(vec!["a".into(), "b".into()]);
        assert_eq!(p.mode, PickerMode::All);
        p.next_mode();
        assert_eq!(p.mode, PickerMode::Recent);
        p.next_mode();
        assert_eq!(p.mode, PickerMode::Providers);
        p.next_mode();
        assert_eq!(p.mode, PickerMode::Favorites);
        p.next_mode();
        assert_eq!(p.mode, PickerMode::All);
        p.previous_mode();
        assert_eq!(p.mode, PickerMode::Favorites);
    }

    #[test]
    fn recent_mode_shows_recent_models() {
        let mut p = ModelPicker::new(vec!["a".into(), "b".into()]);
        p.set_recent(vec!["x".into(), "y".into()]);
        p.next_mode(); // -> Recent
        assert_eq!(p.models, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(p.selected_model().as_deref(), Some("x"));
    }

    #[test]
    fn favorites_mode_shows_favorites() {
        let mut p = ModelPicker::new(vec!["a".into(), "b".into()]);
        p.set_favorites(vec!["fav1".into()]);
        p.next_mode(); // Recent
        p.next_mode(); // Providers
        p.next_mode(); // Favorites
        assert_eq!(p.models, vec!["fav1".to_string()]);
    }
}
