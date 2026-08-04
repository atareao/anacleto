use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph},
};

/// A popup for selecting a model for the active agent.
pub struct ModelPicker {
    pub visible: bool,
    pub models: Vec<String>,
    pub selected: usize,
}

impl ModelPicker {
    /// Create a hidden picker with the given model list.
    pub fn new(models: Vec<String>) -> Self {
        Self {
            visible: false,
            models,
            selected: 0,
        }
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

    /// The currently selected model, if any.
    pub fn selected_model(&self) -> Option<String> {
        self.models.get(self.selected).cloned()
    }

    /// Render the picker as a centered popup.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let width = area.width.min(50);
        let height = (self.models.len() as u16 + 4).min(area.height.min(20));
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

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Magenta))
                    .title(" Seleccionar modelo ")
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
        let footer = " ↑/↓ navegar  ·  Enter: seleccionar  ·  Esc: cancelar ";
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
}
