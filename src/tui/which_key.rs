use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::tui::keyparse::format_keymap_table;

/// A centered popup that shows the full keybinding table ("which-key").
///
/// It is a simple, statically-scrolled listing: any key press closes it.
pub struct WhichKeyPopup {
    /// Whether the popup is currently visible.
    pub visible: bool,
}

impl WhichKeyPopup {
    /// Create a hidden which-key popup.
    pub fn new() -> Self {
        Self { visible: false }
    }

    /// Render the popup centered over `area` if it is visible.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let width = area.width.min(70);
        let height = area.height.min(26);
        let x = area.x + (area.width.saturating_sub(width)) / 2;
        let y = area.y + (area.height.saturating_sub(height)) / 2;
        let popup_area = Rect::new(x, y, width, height);

        let overlay = Clear;
        f.render_widget(overlay, popup_area);

        let table = format_keymap_table();
        let mut lines: Vec<Line> = table
            .lines()
            .map(|l| {
                Line::from(Span::styled(
                    l.to_string(),
                    Style::default().fg(Color::White),
                ))
            })
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Pulsa la tecla del atajo o Esc para cerrar ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )));

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" Atajos de teclado ")
                    .style(Style::default().bg(Color::Rgb(20, 20, 30))),
            )
            .scroll((0, 0));

        f.render_widget(paragraph, popup_area);
    }
}

impl Default for WhichKeyPopup {
    fn default() -> Self {
        Self::new()
    }
}
