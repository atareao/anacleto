//! Fuzzy command/agent/model palettes.
//!
//! Contains the `App` methods that recompute palette matches as the user
//! types, plus the free functions that render each palette as a dropdown
//! above the input area.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem};

use super::app::App;
use super::state::fuzzy_score;

impl App {
    /// Recompute the fuzzy command palette matches based on the current input.
    /// The palette opens whenever the input starts with `/`.
    pub(crate) fn update_command_palette(&mut self) {
        if !self.input.starts_with('/') {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        // `/agent` uses its own agent-selection combo instead of the command list.
        if self.input.starts_with("/agent") {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        // `/models` uses its own model-selection combo instead of the command list.
        if self.input.starts_with("/models") {
            self.show_command_palette = false;
            self.palette_matches.clear();
            self.palette_index = 0;
            self.update_agent_palette();
            self.update_model_palette();
            return;
        }

        self.show_agent_palette = false;
        self.agent_matches.clear();
        self.agent_index = 0;
        self.show_model_palette = false;
        self.model_matches.clear();
        self.model_index = 0;

        let query = self.input.trim_start_matches('/');
        let mut scored: Vec<(u32, String, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, (cmd, _))| fuzzy_score(query, cmd).map(|s| (s, cmd.to_string(), i)))
            .collect();
        // Sort by score descending (best match first), then alphabetically by
        // command name so the combo is stable and predictable.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.palette_matches = scored.into_iter().map(|(_, _, i)| i).collect();
        self.show_command_palette = !self.palette_matches.is_empty();
        if self.palette_index >= self.palette_matches.len() {
            self.palette_index = 0;
        }
    }

    /// Fuzzy agent-selection combo for `/agent`. Only root agents are
    /// switchable, so only those are offered.
    pub(crate) fn update_agent_palette(&mut self) {
        if !self.input.starts_with("/agent") {
            self.show_agent_palette = false;
            self.agent_matches.clear();
            self.agent_index = 0;
            return;
        }

        // Query is the part after `/agent` (e.g. `/agent writ` → "writ").
        let query = self.input.trim_start_matches("/agent").trim_start();

        let mut scored: Vec<(u32, String)> = self
            .agents
            .iter()
            .filter(|a| a.role == crate::agent::types::AgentRole::Root)
            .map(|a| a.name.clone())
            .filter_map(|name| fuzzy_score(query, &name).map(|s| (s, name)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.agent_matches = scored.into_iter().map(|(_, n)| n).collect();
        self.show_agent_palette = !self.agent_matches.is_empty();
        if self.agent_index >= self.agent_matches.len() {
            self.agent_index = 0;
        }
    }

    /// Fuzzy model-selection combo for `/models`.
    pub(crate) fn update_model_palette(&mut self) {
        if !self.input.starts_with("/models") {
            self.show_model_palette = false;
            self.model_matches.clear();
            self.model_index = 0;
            return;
        }

        // Query is the part after `/models` (e.g. `/models gpt` → "gpt").
        let query = self.input.trim_start_matches("/models").trim_start();

        let mut scored: Vec<(u32, String)> = self
            .model_picker
            .all_models()
            .iter()
            .cloned()
            .filter_map(|name| fuzzy_score(query, &name).map(|s| (s, name)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

        self.model_matches = scored.into_iter().map(|(_, n)| n).collect();
        self.show_model_palette = !self.model_matches.is_empty();
        if self.model_index >= self.model_matches.len() {
            self.model_index = 0;
        }
    }
}

/// Render the fuzzy command palette as a dropdown above the input area.
pub(crate) fn render_command_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.palette_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .palette_matches
        .iter()
        .take(max_items)
        .map(|&i| {
            let (cmd, desc) = &app.commands[i];
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:<12}", cmd),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Commands "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.palette_index)),
    );
}

/// Render the agent-selection combo as a dropdown above the input area.
pub(crate) fn render_agent_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.agent_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .agent_matches
        .iter()
        .take(max_items)
        .map(|name| {
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:<16}", name),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("root", Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Agents "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(60, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.agent_index)),
    );
}

/// Render the model-selection combo as a dropdown above the input area.
pub(crate) fn render_model_palette(f: &mut Frame, input_area: Rect, app: &App) {
    let max_items = 8usize;
    let count = app.model_matches.len().min(max_items);
    let width = input_area.width.min(60);
    let height = (count as u16) + 2; // +2 for borders
    let x = input_area.x;
    // Place the dropdown directly above the input area.
    let y = input_area.y.saturating_sub(height);
    let area = Rect::new(x, y, width, height);

    let items: Vec<ListItem> = app
        .model_matches
        .iter()
        .take(max_items)
        .map(|name| {
            let line = Line::from(vec![Span::styled(
                format!(" {:<24}", name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Models "),
        )
        .highlight_style(Style::default().bg(Color::Rgb(40, 40, 60)))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(
        list,
        area,
        &mut ratatui::widgets::ListState::default().with_selected(Some(app.model_index)),
    );
}
