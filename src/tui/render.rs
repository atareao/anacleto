//! Rendering of the TUI.
//!
//! All the free `render_*` functions that draw the various panels, dialogs
//! and overlays, plus the small rendering helpers (`format_tokens`,
//! `SPINNER_FRAMES`) and the keyboard/clipboard utilities used by other
//! modules (`shift_char`, `copy_to_clipboard`).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Tabs, Wrap,
};
use unicode_width::UnicodeWidthStr;

use super::app::App;
use super::code_block::CodeBlockHighlighter;
use super::markdown::{
    render_markdown_line_with_syntect, render_table_block, select_visible_start, visual_line_count,
};
use super::palette::{render_agent_palette, render_command_palette, render_model_palette};
use super::types::{AgentInfo, CollapsedSection, Focus};
use crate::agent::types::{AgentRole, AgentStatus, TaskMode};
use std::collections::{HashMap, HashSet};

/// Render the TUI.
pub(crate) fn render(f: &mut Frame, app: &mut App) {
    // Hide welcome banner once there's actual content
    if !app.messages.is_empty() || app.current_stream.is_some() {
        app.show_welcome = false;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1), // status bar
                Constraint::Min(1),    // main content
                Constraint::Length(5), // input
                Constraint::Length(1), // working directory
            ]
            .as_ref(),
        )
        .split(f.area());

    render_status_bar(f, chunks[0], app);
    render_main_content(f, chunks[1], app);
    render_input(f, chunks[2], app);
    render_working_dir(f, chunks[3], app);

    // Render the fuzzy command palette above the input if open.
    if app.show_command_palette && !app.palette_matches.is_empty() {
        render_command_palette(f, chunks[2], app);
    }
    // Render the agent-selection combo above the input if open.
    if app.show_agent_palette && !app.agent_matches.is_empty() {
        render_agent_palette(f, chunks[2], app);
    }
    // Render the model-selection combo above the input if open.
    if app.show_model_palette && !app.model_matches.is_empty() {
        render_model_palette(f, chunks[2], app);
    }

    // Render approval dialog on top if pending
    if app.pending_approval.is_some() {
        render_approval_dialog(f, f.area(), app);
    }

    // Render inline question dialog on top if pending
    if app.pending_question.is_some() {
        render_question_dialog(f, f.area(), app);
    }

    // Render the which-key popup on top if visible.
    app.which_key.render(f, f.area());

    // Render the diff viewer and model picker overlays if visible.
    app.diff_viewer.render(f, f.area());
    app.model_picker.render(f, f.area());

    // Render the prompt queue popup if visible.
    render_prompt_queue(f, f.area(), app);

    // Render the search overlay if visible.
    if app.search.visible {
        render_search_overlay(f, f.area(), app);
    }

    // Render the edit-agent/subagent dialog if visible.
    if app.edit_dialog.visible {
        render_edit_dialog(f, f.area(), app);
    }

    // Render transient toasts in the bottom-right corner.
    app.toasts.render(f, f.area());
}

/// Render the prompt queue popup (FASE 4.6).
fn render_prompt_queue(f: &mut Frame, area: Rect, app: &App) {
    if !app.show_prompt_queue {
        return;
    }
    let items: Vec<ListItem> = app
        .prompt_queue
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.prompt_queue_index {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!("{:>2}. {}", i + 1, p),
                style,
            )))
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Cola de prompts "),
    );
    let popup = Rect {
        x: area.width.saturating_sub(60) / 2,
        y: area.height.saturating_sub(20) / 2,
        width: 60.min(area.width),
        height: 20.min(area.height),
    };
    f.render_widget(Clear, popup);
    f.render_widget(list, popup);
}

/// Render the conversation history search overlay (Ctrl+R).
fn render_search_overlay(f: &mut Frame, area: Rect, app: &App) {
    let dialog_width = area.width.min(60);
    let dialog_height = 10;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    f.render_widget(Clear, dialog_area);

    // Build content lines
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("> {}", app.search.query),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(Span::raw("")));

    // Show match count
    let match_text = if app.search.query.is_empty() {
        "Type to search conversation history...".to_string()
    } else {
        format!("{} match(es) found", app.search.matches.len())
    };
    lines.push(Line::from(Span::styled(
        match_text,
        Style::default().fg(Color::DarkGray),
    )));

    // Show current match preview if any
    if !app.search.matches.is_empty() {
        let idx = app.search.matches[app.search.selected];
        if let Some(msg) = app.messages.get(idx) {
            let preview = if msg.len() > 60 {
                format!("{}...", &msg[..60])
            } else {
                msg.clone()
            };
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!(
                    "[{}/{}] {}",
                    app.search.selected + 1,
                    app.search.matches.len(),
                    preview,
                ),
                Style::default().fg(Color::Cyan),
            )));
        }
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        " ↑↓ navigate  ↵ jump  Esc close ",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM),
    )));

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Search History (Ctrl+R) ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Rgb(20, 20, 40))),
        )
        .alignment(Alignment::Left);

    f.render_widget(dialog, dialog_area);
}

/// Render the top status bar with agent/session info.
fn render_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let root_count = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .count();
    let subagent_count = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::SubAgent)
        .count();
    let active_count = app
        .agents
        .iter()
        .filter(|a| a.status == AgentStatus::Working)
        .count();

    let skill_count: usize = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .map(|a| a.skills.len())
        .sum();
    let mcp_count: usize = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .map(|a| a.mcps.len())
        .sum();

    let session_label = app.session_id.as_deref().unwrap_or("-");
    let mut all_spans: Vec<Span<'static>> = Vec::with_capacity(16);
    all_spans.push(Span::styled(
        " ⬡ anacleto ",
        Style::default()
            .fg(app.theme.accent())
            .add_modifier(Modifier::BOLD),
    ));
    // Keyboard protocol indicator
    if app.kb_supported {
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
        all_spans.push(Span::styled(
            " ⌨ ",
            Style::default().fg(Color::Rgb(100, 200, 100)),
        ));
    }
    all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    all_spans.push(Span::styled(
        format!(" {}:{} ", app.session_name, session_label),
        Style::default().fg(Color::Cyan),
    ));
    all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));

    // Active agent indicator
    if !app.active_agent.is_empty() {
        all_spans.push(Span::styled(
            format!(" @{} ", app.active_agent),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }

    // Pending prompt queue indicator
    if !app.prompt_queue.is_empty() {
        all_spans.push(Span::styled(
            format!(" ({} en cola) ", app.prompt_queue.len()),
            Style::default()
                .fg(Color::Rgb(255, 180, 50))
                .add_modifier(Modifier::BOLD),
        ));
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }

    // Debug mode indicator
    if app.debug_mode {
        all_spans.push(Span::styled(
            " \u{1f41b} DEBUG ",
            Style::default()
                .fg(Color::Rgb(255, 180, 50))
                .add_modifier(Modifier::BOLD),
        ));
        all_spans.push(Span::styled("│", Style::default().fg(Color::DarkGray)));
    }

    all_spans.push(Span::styled(
        format!(" {}a ", root_count),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ));
    all_spans.push(Span::styled(
        format!("{}sa ", subagent_count),
        Style::default().fg(Color::Yellow),
    ));
    all_spans.push(Span::styled(
        format!("{}⚡ ", active_count),
        Style::default()
            .fg(Color::Rgb(255, 215, 0))
            .add_modifier(Modifier::BOLD),
    ));

    // Right-aligned segment: compute padding
    let left_width: u16 = all_spans.iter().map(|s| s.width() as u16).sum::<u16>() + 2; // leading + trailing spaces
    let right_items = vec![
        Span::styled(
            format!(" ⚙ {} ", skill_count),
            Style::default().fg(Color::Rgb(100, 200, 100)),
        ),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" 🔌 {} ", mcp_count),
            Style::default().fg(Color::Rgb(180, 130, 255)),
        ),
    ];
    let right_width: u16 = right_items.iter().map(|s| s.width() as u16).sum::<u16>();

    let pad = area.width.saturating_sub(left_width + right_width + 2);
    all_spans.push(Span::raw(" ".repeat(pad as usize)));
    all_spans.extend(right_items);
    all_spans.push(Span::styled(" ", Style::default()));

    let bar = Line::from(all_spans);

    let paragraph =
        Paragraph::new(bar).style(Style::default().bg(Color::Rgb(25, 25, 35)).fg(Color::White));
    f.render_widget(paragraph, area);
}

/// Render the main content area: left (chat/overlays) and right (status panels).
fn render_main_content(f: &mut Frame, area: Rect, app: &mut App) {
    if app.show_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
            .split(area);

        render_left_panel(f, chunks[0], app);
        render_right_panels(f, chunks[1], app);
    } else {
        // Sidebar hidden: left panel takes the full width.
        render_left_panel(f, area, app);
    }
}

/// Render the left panel: session list, agent list, subagent tree, or chat.
fn render_left_panel(f: &mut Frame, area: Rect, app: &mut App) {
    if app.show_timeline {
        render_timeline_panel(f, area, app);
    } else if app.show_mcps {
        render_mcp_list_panel(f, area, app);
    } else if app.show_session_list {
        render_session_list(f, area, app);
    } else if app.show_agents {
        render_agent_list(f, area, app);
    } else if app.show_subagents {
        render_subagent_tree(f, area, app);
    } else {
        render_chat(f, area, app);
    }
}

/// Render the session timeline panel (`/timeline`).
fn render_timeline_panel(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .timeline
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = format!(
                "{} {}: {}",
                e.created_at.format("%H:%M:%S"),
                e.role,
                e.content.chars().take(60).collect::<String>()
            );
            let style = if i == app.timeline_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.accent()))
                .title(" Timeline "),
        )
        .highlight_style(Style::default().bg(app.theme.accent()));
    f.render_widget(list, area);
}

/// Render the MCP server list panel (`/mcps`).
fn render_mcp_list_panel(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .mcps_list
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let state = if m.enabled { "● ON" } else { "○ OFF" };
            let label = format!("{} {}", state, m.name);
            let style = if i == app.mcps_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(app.theme.accent()))
                .title(" MCP Servers "),
        )
        .highlight_style(Style::default().bg(app.theme.accent()));
    f.render_widget(list, area);
}

/// Render the right panel: 4 stacked info panels (Status, Info-tabs, Agents, Queue).
fn render_right_panels(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(6),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ]
            .as_ref(),
        )
        .split(area);

    render_status_panel(f, chunks[0], app);
    render_info_panel(f, chunks[1], app);
    render_agent_panel(f, chunks[2], app);
    render_queue_panel(f, chunks[3], app);
}

/// Panel 1: Status — tokens, coste y contexto en tres líneas.
fn render_status_panel(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(1)].as_ref())
        .split(area);

    let text = format!(
        "Tokens: {}\nCost: ${:.2}\nContext: {:.1}% ({} / {})",
        format_tokens(app.total_tokens),
        app.total_cost,
        app.context_window_pct,
        format_tokens(app.total_tokens),
        format_tokens(app.context_window)
    );

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Status "),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, chunks[0]);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Rgb(20, 40, 60)))
        .percent((app.context_window_pct.min(100.0)) as u16)
        .label(format!("Context: {:.1}%", app.context_window_pct));
    f.render_widget(gauge, chunks[1]);
}

/// Format a token count as thousands (K) or millions (M).
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Panel 2: Info — unified Skills/MCPs/SubAgents panel with three tabs.
fn render_info_panel(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)].as_ref())
        .split(area);

    let titles: Vec<Line> = [" Skills ", " MCPs ", " SubAgents "]
        .iter()
        .map(|t| Line::from(Span::styled(*t, Style::default())))
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.info_tab)
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(tabs, chunks[0]);

    if app.info_tab == 0 {
        render_skill_panel(f, chunks[1], app);
    } else if app.info_tab == 1 {
        render_mcp_panel(f, chunks[1], app);
    } else {
        render_subagent_panel(f, chunks[1], app);
    }
}

/// Panel 2a: MCPs — MCP server names of the active agent (active when `info_tab = 1`).
fn render_mcp_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_mcps: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .filter(|a| a.name == app.active_agent)
            .flat_map(|a| a.mcps.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Info && app.info_tab == 1;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Magenta
    };

    let items: Vec<ListItem> = if unique_mcps.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        unique_mcps
            .iter()
            .enumerate()
            .map(|(i, mcp)| {
                let style = if focused && i == app.mcp_panel_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(*mcp, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" [2] MCPs ({}) ", unique_mcps.len())),
    );

    f.render_widget(list, area);
}

/// Panel 2c: SubAgents — subagent names of the active agent (active when `info_tab = 2`).
fn render_subagent_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_subagents: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .configured_subagents
            .get(&app.active_agent)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Info && app.info_tab == 2;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Cyan
    };

    let items: Vec<ListItem> = if unique_subagents.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        unique_subagents
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let style = if focused && i == app.subagent_panel_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(*name, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" [2] SubAgents ({}) ", unique_subagents.len())),
    );

    f.render_widget(list, area);
}

/// Panel 2b: Skills — skill names of the active agent (active when `info_tab = 0`).
fn render_skill_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_skills: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .filter(|a| a.name == app.active_agent)
            .flat_map(|a| a.skills.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Info && app.info_tab == 0;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Green
    };

    let items: Vec<ListItem> = if unique_skills.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        unique_skills
            .iter()
            .enumerate()
            .map(|(i, skill)| {
                let style = if focused && i == app.skill_panel_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(Span::styled(*skill, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" [3] Skills ({}) ", unique_skills.len())),
    );

    f.render_widget(list, area);
}

/// Spinner animation frames (Braille dots).
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Panel 4: Running agents — agents with Working status.
fn render_agent_panel(f: &mut Frame, area: Rect, app: &App) {
    let display_agents: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.status != AgentStatus::Completed)
        .collect();

    let focused = app.focus == Focus::Agents;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Yellow
    };

    let items: Vec<ListItem> = if display_agents.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(none)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        display_agents
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let selected = focused && i == app.agent_panel_index;
                let active = a.name == app.active_agent;
                let (dot, dot_color) = match &a.status {
                    AgentStatus::Working => ("🟢", Color::Green),
                    AgentStatus::Idle => ("⏸", Color::Yellow),
                    AgentStatus::WaitingForSubAgent => ("⏳", Color::Blue),
                    AgentStatus::Completed => ("✅", Color::DarkGray),
                    AgentStatus::Error(_) => ("❌", Color::Red),
                };
                let role = match a.role {
                    AgentRole::Root => "Root",
                    AgentRole::SubAgent => "SubAgent",
                };
                let status_str = match &a.status {
                    AgentStatus::Working => "working",
                    AgentStatus::Idle => "idle",
                    AgentStatus::WaitingForSubAgent => "waiting",
                    AgentStatus::Completed => "done",
                    AgentStatus::Error(_) => "error",
                };
                // Selected rows get the accent background (matching the Skills
                // panel), applied to every span so the whole row is highlighted.
                let sel = |s: Style| -> Style {
                    if selected {
                        s.bg(app.theme.accent()).fg(Color::Black)
                    } else {
                        s
                    }
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if active { "▶ " } else { "  " },
                        sel(Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD)),
                    ),
                    Span::styled(
                        format!(" {} ", dot),
                        sel(Style::default().fg(dot_color).add_modifier(Modifier::BOLD)),
                    ),
                    Span::styled(
                        &a.name,
                        sel(Style::default()
                            .fg(if active { Color::Magenta } else { Color::White })
                            .add_modifier(Modifier::BOLD)),
                    ),
                    if a.status == AgentStatus::Working {
                        Span::styled(
                            format!(
                                " {}",
                                SPINNER_FRAMES[(app.frame_count as usize) % SPINNER_FRAMES.len()]
                            ),
                            sel(Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)),
                        )
                    } else {
                        Span::raw("")
                    },
                    Span::styled(
                        format!(" [{}]", role),
                        sel(Style::default().fg(Color::DarkGray)),
                    ),
                    Span::styled(
                        format!(" [{}]", a.agent_type.as_deref().unwrap_or("generic")),
                        sel(Style::default().fg(Color::Cyan)),
                    ),
                    if let Some(mode) = &a.mode {
                        let label = match mode {
                            TaskMode::Foreground => "fg",
                            TaskMode::Background => "bg",
                        };
                        Span::styled(
                            format!(" ({label})"),
                            sel(Style::default().fg(Color::DarkGray)),
                        )
                    } else {
                        Span::raw("")
                    },
                    Span::styled(
                        format!(" ({})", status_str),
                        sel(Style::default().fg(dot_color)),
                    ),
                ]))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" [4] Agents ({}) ", display_agents.len())),
    );

    f.render_widget(list, area);
}

/// Panel 4: Queue — the visible, interactive prompt queue.
fn render_queue_panel(f: &mut Frame, area: Rect, app: &App) {
    let focused = app.focus == Focus::Queue;
    let border_color = if focused {
        app.theme.accent()
    } else {
        Color::Blue
    };

    let items: Vec<ListItem> = if app.prompt_queue.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(vacía)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.prompt_queue
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let style = if focused && i == app.prompt_queue_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{:>2}. {}", i + 1, p),
                    style,
                )))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(format!(" [5] Queue ({}) ", app.prompt_queue.len())),
    );

    f.render_widget(list, area);
}

/// Render the current working directory (left), git branch (center),
/// and active model (right). The branch is only shown when inside a git
/// repo with a valid named branch.
fn render_working_dir(f: &mut Frame, area: Rect, app: &App) {
    let dir_text = format!(" 📁 {}", app.working_dir);
    let model_text = format!("🤖 {}", app.current_model);

    let branch_span = app.git_branch.as_ref().map(|b| {
        Span::styled(
            format!(" ⎇ {} ", b),
            Style::default().fg(Color::Rgb(188, 143, 254)), // lavender
        )
    });

    let width = area.width as usize;

    // Calculate display widths
    let dir_width = dir_text.width();
    let model_width = model_text.width();
    let branch_width = branch_span.as_ref().map(|s| s.width()).unwrap_or(0);

    // Remaining space after reserving dir + branch + model (with 1-space gaps)
    let total_fixed = dir_width + branch_width + model_width;
    let gap_count = if branch_span.is_some() { 2 } else { 1 };
    let padding = width.saturating_sub(total_fixed + gap_count);

    let line = if let Some(branch) = branch_span {
        // Distribute padding: put branch roughly centered.
        // 1/3 of padding before branch, 2/3 after (between branch and model).
        let left_pad = padding / 3;
        let right_pad = padding - left_pad;
        Line::from(vec![
            Span::styled(dir_text, Style::default().fg(Color::DarkGray)),
            Span::raw(" ".repeat(left_pad)),
            branch,
            Span::raw(" ".repeat(right_pad)),
            Span::styled(model_text, Style::default().fg(Color::Cyan)),
        ])
    } else {
        // Original layout: dir left, model right (no branch)
        let max_dir_width = width.saturating_sub(model_width + 1);
        let truncated_dir = if dir_width > max_dir_width && max_dir_width > 3 {
            let keep = max_dir_width.saturating_sub(1);
            let mut s = String::new();
            let mut w = 0;
            for ch in dir_text.chars() {
                let cw = ch.to_string().width();
                if w + cw > keep {
                    break;
                }
                s.push(ch);
                w += cw;
            }
            s.push('…');
            s
        } else {
            dir_text
        };
        let p = width.saturating_sub(truncated_dir.width() + model_width);
        Line::from(vec![
            Span::styled(truncated_dir, Style::default().fg(Color::DarkGray)),
            Span::raw(" ".repeat(p)),
            Span::styled(model_text, Style::default().fg(Color::Cyan)),
        ])
    };

    f.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Shared sectioned-block renderer for committed messages and stream.
// ---------------------------------------------------------------------------

/// Styles for each section type.
struct SectionStyles {
    border: Style,
    thinking_border: Style,
    thinking_text: Style,
    tool_border: Style,
    tool_exec: Style,
    tool_output: Style,
    tool_success: Style,
    tool_error: Style,
    user_border: Style,
    user_text: Style,
    command_border: Style,
    command_text: Style,
}

/// Configures whether/how the normal (non-thinking, non-tool) section draws
/// its `▐` borders and which prefix to use on its lines.
struct SectionConfig {
    /// Prefix for the first normal line (e.g. "\u{258c}" for stream vs "▐ " for
    /// committed).
    first_normal_prefix: &'static str,
    /// Prefix for subsequent normal lines (e.g. " " for stream vs "▐ " for
    /// committed).
    subsequent_normal_prefix: &'static str,
}

/// Helper: flush accumulated table lines into styled output.
#[allow(clippy::too_many_arguments)]
fn flush_table(
    out: &mut Vec<Line>,
    table_buffer: &mut Vec<String>,
    first_normal: &mut bool,
    section_has_content: &mut bool,
    cumulative_visual: &mut usize,
    content_width: usize,
    prefix: &'static str,
    border_style: Style,
    cell_style: Style,
) -> bool {
    if table_buffer.is_empty() {
        return false;
    }
    let table_lines: Vec<&str> = table_buffer.iter().map(|s| s.as_str()).collect();
    let table_rows = render_table_block(&table_lines, prefix, border_style, cell_style);
    let count = table_rows.len();
    for row in &table_rows {
        *cumulative_visual += visual_line_count(row, content_width);
    }
    out.extend(table_rows);
    table_buffer.clear();
    *first_normal = false;
    *section_has_content = true;
    count > 0
}

/// Shared section-tracking state machine that renders `[thinking]`/`[/thinking]`
/// markers and tool-execution lines (🔧/✅/❌) into independently-bordered
/// visual sections.
///
/// `normal_line_render` is called for every non-thinking, non-tool, non-table
/// line. It receives `(full_line_text, prefix_string, border_style)` and must
/// return a complete `Line` (including the prefix span rendered as it wishes).
///
/// Consecutive lines starting with `|` are automatically detected and rendered
/// as a table block using box-drawing characters.
/// Generate a unique section ID from a type name and a per-type counter.
/// Returns strings like "thinking_1", "tool_2", etc.
pub(crate) fn generate_section_id(
    section_type: &str,
    counters: &mut HashMap<String, u32>,
) -> String {
    let entry = counters.entry(section_type.to_string()).or_insert(0);
    *entry += 1;
    format!("{}_{}", section_type, entry)
}

/// Flush a section (thinking/tool) into the output, always showing full content
/// with a top border, content lines (each with a ▐ prefix from the caller),
/// and a bottom border. No collapse/expand toggle.
///
/// `cumulative_visual` is updated in-place as lines are emitted.
/// When `section_line_map`, `section_info`, and `counters` are provided,
/// this records section ID information for the collapse/expand feature.
#[allow(clippy::too_many_arguments)]
fn flush_section(
    out: &mut Vec<Line<'static>>,
    buffer: &mut Vec<Line<'static>>,
    section_type: &'static str,
    styles: &SectionStyles,
    cumulative_visual: &mut usize,
    content_width: usize,
    last_flushed: &mut Option<&'static str>,
    mut section_line_map: Option<&mut Vec<Option<String>>>,
    mut section_info: Option<&mut Vec<CollapsedSection>>,
    mut counters: Option<&mut HashMap<String, u32>>,
) {
    if buffer.is_empty() {
        return;
    }
    let border_style = match section_type {
        "thinking" => styles.thinking_border,
        "tool" => styles.tool_border,
        "normal" => styles.border,
        "user" => styles.user_border,
        "command" => styles.command_border,
        _ => return,
    };

    let slm = &mut section_line_map;
    let si = &mut section_info;
    let cnt = &mut counters;
    let track = slm.is_some() && si.is_some() && cnt.is_some();

    let mut section_id: Option<String> = None;
    let start_line: usize;

    // Top border — skipped when the previous flushed section had the same
    // type, so consecutive blocks of the same kind read as one continuous
    // section (the previous block's bottom border acts as the separator).
    if *last_flushed != Some(section_type) {
        let top_line = Line::from(Span::styled("\u{2590}", border_style));
        *cumulative_visual += visual_line_count(&top_line, content_width);
        start_line = out.len();
        out.push(top_line);

        if track {
            let slm_vec = slm.as_mut().unwrap();
            let si_vec = si.as_mut().unwrap();
            let cnt_vec = cnt.as_mut().unwrap();
            let id = generate_section_id(section_type, cnt_vec);
            section_id = Some(id.clone());
            if slm_vec.len() <= start_line {
                slm_vec.resize(start_line + 1, None);
            }
            slm_vec[start_line] = Some(id.clone());
            si_vec.push(CollapsedSection {
                id,
                section_type: section_type.to_string(),
                start_line,
                line_count: 0,
            });
        }
    } else {
        // No new top border — section content follows from the previous
        // same-type section's bottom border. The start is the last line.
        start_line = out.len().saturating_sub(1);
    }

    // Emit content lines directly (each already has a ▐ prefix from the caller)
    let lines: Vec<Line<'static>> = std::mem::take(buffer);
    for l in &lines {
        *cumulative_visual += visual_line_count(l, content_width);
    }
    out.extend(lines);

    // Bottom border
    let bottom_line = Line::from(Span::styled("\u{2590}", border_style));
    *cumulative_visual += visual_line_count(&bottom_line, content_width);
    out.push(bottom_line);

    // Update section_line_map and section_info with correct line_count
    if track {
        let slm_vec = slm.as_mut().unwrap();
        let si_vec = si.as_mut().unwrap();
        let total_lines = out.len() - start_line;
        for i in start_line..out.len() {
            if slm_vec.len() <= i {
                slm_vec.resize(i + 1, None);
            }
            if slm_vec[i].is_none() {
                slm_vec[i] = section_id.clone();
            }
        }
        if let Some(ref sec_id) = section_id
            && let Some(entry) = si_vec.iter_mut().rev().find(|s| s.id == *sec_id)
        {
            entry.line_count = total_lines;
        }
    }

    *last_flushed = Some(section_type);
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::needless_option_as_deref)]
fn render_sectioned_block(
    content: &str,
    ts: &str,
    config: &SectionConfig,
    styles: &SectionStyles,
    table_cell_style: Style,
    normal_line_render: impl Fn(String, &'static str, Style) -> Line<'static>,
    code_block_hl: &CodeBlockHighlighter,
    code_block_positions: &mut Vec<crate::tui::code_block::CodeBlockPosition>,
    is_dark: bool,
    cumulative_visual: &mut usize,
    content_width: usize,
    mut section_line_map: Option<&mut Vec<Option<String>>>,
    mut section_info: Option<&mut Vec<CollapsedSection>>,
    mut counters: Option<&mut HashMap<String, u32>>,
) -> Vec<Line<'static>> {
    // section_line_map, section_info, counters: Option<&mut ...> params passed below
    let mut out: Vec<Line> = Vec::new();
    let mut section: &str = "normal";
    let mut section_has_content = false;
    let mut awaiting_ts = !ts.is_empty();
    let mut first_normal = true;
    let mut table_buffer: Vec<String> = Vec::new();
    let mut thinking_buffer: Vec<Line<'static>> = Vec::new();
    let mut tool_buffer: Vec<Line<'static>> = Vec::new();
    let mut normal_buffer: Vec<Line<'static>> = Vec::new();
    let mut user_buffer: Vec<Line<'static>> = Vec::new();
    let mut command_buffer: Vec<Line<'static>> = Vec::new();
    let mut last_flushed: Option<&'static str> = None;
    let mut tool_style = styles.tool_exec;

    // Helper to pick the right prefix
    let prefix = |first: bool| -> &'static str {
        if first {
            config.first_normal_prefix
        } else {
            config.subsequent_normal_prefix
        }
    };

    let mut lines_iter = content.split('\n').peekable();
    while let Some(line_text) = lines_iter.next() {
        let marker = line_text.trim();

        // ── Fenced code block detection ──
        if marker.starts_with("```") {
            let lang = marker.trim_start_matches("```").trim().to_string();

            // Collect code content until closing ``` or end
            let mut code_lines: Vec<String> = Vec::new();
            for code_line in lines_iter.by_ref() {
                if code_line.trim() == "```" {
                    break;
                }
                code_lines.push(code_line.to_string());
            }

            let code = code_lines.join("\n");

            let highlighted = code_block_hl.highlight_fenced_block(&lang, &code, is_dark);

            // Record the block position (for mouse-click copy).
            let block_idx = code_block_positions.len();
            code_block_positions.push(crate::tui::code_block::CodeBlockPosition {
                lang: lang.clone(),
                code: code.clone(),
                copy_line: 0,
            });

            // Emit code block content (no extra ▐ border — content lines
            // already have a prefix from the highlighted rendering).
            if first_normal {
                first_normal = false;
            }

            for mut hl_line in highlighted {
                let p = prefix(first_normal);
                if first_normal {
                    first_normal = false;
                }
                hl_line.spans.insert(0, Span::styled(p, styles.border));
                hl_line.spans.insert(1, Span::styled(" ", Style::default()));
                *cumulative_visual += visual_line_count(&hl_line, content_width);
                out.push(hl_line);
            }

            // Find the [copy] line index for mouse-click handling
            if let Some(block) = code_block_positions.get_mut(block_idx) {
                let out_len = out.len();
                for (offset, line) in out.iter().enumerate().rev().take(out_len) {
                    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    if full.contains("[copy]") && block.copy_line == 0 {
                        block.copy_line = offset;
                        break;
                    }
                }
            }

            section_has_content = true;
            continue;
        }

        // ── [thinking] / [/thinking] markers ──
        if marker == "[thinking]" {
            // Close previous section
            if section == "tool" && section_has_content {
                flush_section(
                    &mut out,
                    &mut tool_buffer,
                    "tool",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "user" && section_has_content {
                flush_section(
                    &mut out,
                    &mut user_buffer,
                    "user",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "command" && section_has_content {
                flush_section(
                    &mut out,
                    &mut command_buffer,
                    "command",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "normal" && section_has_content {
                flush_section(
                    &mut out,
                    &mut normal_buffer,
                    "normal",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            }
            section = "thinking";
            section_has_content = false;
            thinking_buffer.clear();
            continue;
        }
        if marker == "[/thinking]" {
            // Flush any pending table
            let p = prefix(first_normal);
            flush_table(
                &mut out,
                &mut table_buffer,
                &mut first_normal,
                &mut section_has_content,
                cumulative_visual,
                content_width,
                p,
                styles.border,
                table_cell_style,
            );
            flush_section(
                &mut out,
                &mut thinking_buffer,
                "thinking",
                styles,
                cumulative_visual,
                content_width,
                &mut last_flushed,
                section_line_map.as_deref_mut(),
                section_info.as_deref_mut(),
                counters.as_deref_mut(),
            );
            section = "normal";
            section_has_content = false;
            continue;
        }

        // ── [tool] / [/tool] markers ──
        if marker == "[tool]" {
            // Close previous section
            if section == "thinking" && section_has_content {
                flush_section(
                    &mut out,
                    &mut thinking_buffer,
                    "thinking",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "user" && section_has_content {
                flush_section(
                    &mut out,
                    &mut user_buffer,
                    "user",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "command" && section_has_content {
                flush_section(
                    &mut out,
                    &mut command_buffer,
                    "command",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "normal" && section_has_content {
                flush_section(
                    &mut out,
                    &mut normal_buffer,
                    "normal",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            }
            section = "tool";
            section_has_content = false;
            tool_buffer.clear();
            tool_style = styles.tool_exec;
            continue;
        }
        if marker == "[/tool]" {
            flush_section(
                &mut out,
                &mut tool_buffer,
                "tool",
                styles,
                cumulative_visual,
                content_width,
                &mut last_flushed,
                section_line_map.as_deref_mut(),
                section_info.as_deref_mut(),
                counters.as_deref_mut(),
            );
            section = "normal";
            section_has_content = false;
            continue;
        }
        // ── [tool-output] marker: switch to dimmed style for output content ──
        if marker == "[tool-output]" && section == "tool" {
            tool_style = styles.tool_output;
            continue;
        }

        // ── [tool-success] / [tool-error] markers ──
        if marker == "[tool-success]" && section == "tool" {
            tool_style = styles.tool_success;
            continue;
        }
        if marker == "[tool-error]" && section == "tool" {
            tool_style = styles.tool_error;
            continue;
        }

        // ── [normal] / [/normal] markers ──
        if marker == "[normal]" {
            // Close previous section if it was thinking or tool
            if section == "thinking" && section_has_content {
                flush_section(
                    &mut out,
                    &mut thinking_buffer,
                    "thinking",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "tool" && section_has_content {
                flush_section(
                    &mut out,
                    &mut tool_buffer,
                    "tool",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "user" && section_has_content {
                flush_section(
                    &mut out,
                    &mut user_buffer,
                    "user",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "command" && section_has_content {
                flush_section(
                    &mut out,
                    &mut command_buffer,
                    "command",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            }
            section = "normal";
            section_has_content = false;
            normal_buffer.clear();
            first_normal = true;
            continue;
        }
        if marker == "[/normal]" {
            flush_section(
                &mut out,
                &mut normal_buffer,
                "normal",
                styles,
                cumulative_visual,
                content_width,
                &mut last_flushed,
                section_line_map.as_deref_mut(),
                section_info.as_deref_mut(),
                counters.as_deref_mut(),
            );
            section = "normal";
            section_has_content = false;
            first_normal = true;
            continue;
        }

        // ── [user] / [/user] markers ──
        if marker == "[user]" {
            // Close previous section if it was thinking, tool, or normal
            if section == "thinking" && section_has_content {
                flush_section(
                    &mut out,
                    &mut thinking_buffer,
                    "thinking",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "tool" && section_has_content {
                flush_section(
                    &mut out,
                    &mut tool_buffer,
                    "tool",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "normal" && section_has_content {
                flush_section(
                    &mut out,
                    &mut normal_buffer,
                    "normal",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "command" && section_has_content {
                flush_section(
                    &mut out,
                    &mut command_buffer,
                    "command",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            }
            section = "user";
            section_has_content = false;
            user_buffer.clear();
            continue;
        }
        if marker == "[/user]" {
            flush_section(
                &mut out,
                &mut user_buffer,
                "user",
                styles,
                cumulative_visual,
                content_width,
                &mut last_flushed,
                section_line_map.as_deref_mut(),
                section_info.as_deref_mut(),
                counters.as_deref_mut(),
            );
            section = "normal";
            section_has_content = false;
            first_normal = true;
            continue;
        }

        // ── [command] / [/command] markers ──
        if marker == "[command]" {
            // Close previous section if it was thinking, tool, normal, or user
            if section == "thinking" && section_has_content {
                flush_section(
                    &mut out,
                    &mut thinking_buffer,
                    "thinking",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "tool" && section_has_content {
                flush_section(
                    &mut out,
                    &mut tool_buffer,
                    "tool",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "normal" && section_has_content {
                flush_section(
                    &mut out,
                    &mut normal_buffer,
                    "normal",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            } else if section == "user" && section_has_content {
                flush_section(
                    &mut out,
                    &mut user_buffer,
                    "user",
                    styles,
                    cumulative_visual,
                    content_width,
                    &mut last_flushed,
                    section_line_map.as_deref_mut(),
                    section_info.as_deref_mut(),
                    counters.as_deref_mut(),
                );
            }
            section = "command";
            section_has_content = false;
            command_buffer.clear();
            continue;
        }
        if marker == "[/command]" {
            flush_section(
                &mut out,
                &mut command_buffer,
                "command",
                styles,
                cumulative_visual,
                content_width,
                &mut last_flushed,
                section_line_map.as_deref_mut(),
                section_info.as_deref_mut(),
                counters.as_deref_mut(),
            );
            section = "normal";
            section_has_content = false;
            first_normal = true;
            continue;
        }

        // Build full line with optional timestamp prefix
        let ts_prefix = if awaiting_ts { ts } else { "" };
        awaiting_ts = false;
        let full_line = format!("{}{}", ts_prefix, line_text);

        // ── Render the line ──
        if section == "thinking" {
            thinking_buffer.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.thinking_border),
                Span::styled(full_line, styles.thinking_text),
            ]));
            section_has_content = true;
        } else if section == "user" {
            user_buffer.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.user_border),
                Span::styled(full_line, styles.user_text),
            ]));
            section_has_content = true;
        } else if section == "command" {
            command_buffer.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.command_border),
                Span::styled(full_line, styles.command_text),
            ]));
            section_has_content = true;
        } else if section == "tool" {
            tool_buffer.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.tool_border),
                Span::styled(full_line, tool_style),
            ]));
            section_has_content = true;
        } else if section == "normal" {
            // ── Table detection (consecutive | lines) ──
            if marker.starts_with('|') {
                // Entering table mode
                if table_buffer.is_empty() && first_normal {
                    first_normal = false;
                }
                table_buffer.push(line_text.to_string());
                section_has_content = true;
                continue;
            }

            // If we were accumulating a table and hit a non-table line, flush it
            let p = prefix(first_normal);
            flush_table(
                &mut out,
                &mut table_buffer,
                &mut first_normal,
                &mut section_has_content,
                cumulative_visual,
                content_width,
                p,
                styles.border,
                table_cell_style,
            );

            // Normal line — buffer for section rendering
            let p = if first_normal {
                first_normal = false;
                config.first_normal_prefix
            } else {
                config.subsequent_normal_prefix
            };
            let rendered = normal_line_render(full_line, p, styles.border);
            normal_buffer.push(rendered);
            section_has_content = true;
        } else {
            // Unknown section — render inline as fallback
            let p = if first_normal {
                first_normal = false;
                config.first_normal_prefix
            } else {
                config.subsequent_normal_prefix
            };
            let rendered = normal_line_render(full_line, p, styles.border);
            out.push(rendered);
            section_has_content = true;
        }
    }

    // Flush any remaining table at end of content
    let p = prefix(first_normal);
    flush_table(
        &mut out,
        &mut table_buffer,
        &mut first_normal,
        &mut section_has_content,
        cumulative_visual,
        content_width,
        p,
        styles.border,
        table_cell_style,
    );

    // Flush any remaining section buffers (for sections that were never
    // explicitly closed via [/thinking] or tool→normal transition).
    flush_section(
        &mut out,
        &mut thinking_buffer,
        "thinking",
        styles,
        cumulative_visual,
        content_width,
        &mut last_flushed,
        section_line_map.as_deref_mut(),
        section_info.as_deref_mut(),
        counters.as_deref_mut(),
    );
    flush_section(
        &mut out,
        &mut tool_buffer,
        "tool",
        styles,
        cumulative_visual,
        content_width,
        &mut last_flushed,
        section_line_map.as_deref_mut(),
        section_info.as_deref_mut(),
        counters.as_deref_mut(),
    );
    flush_section(
        &mut out,
        &mut normal_buffer,
        "normal",
        styles,
        cumulative_visual,
        content_width,
        &mut last_flushed,
        section_line_map.as_deref_mut(),
        section_info.as_deref_mut(),
        counters.as_deref_mut(),
    );
    flush_section(
        &mut out,
        &mut user_buffer,
        "user",
        styles,
        cumulative_visual,
        content_width,
        &mut last_flushed,
        section_line_map.as_deref_mut(),
        section_info.as_deref_mut(),
        counters.as_deref_mut(),
    );
    flush_section(
        &mut out,
        &mut command_buffer,
        "command",
        styles,
        cumulative_visual,
        content_width,
        &mut last_flushed,
        section_line_map.as_deref_mut(),
        section_info.as_deref_mut(),
        counters.as_deref_mut(),
    );

    out
}

/// Flush the accumulated AI message batch: join all messages and render them
/// as a single sectioned block so that thinking → tool → response transitions
/// produce only one `▐` separator between sections, not two.
#[allow(clippy::too_many_arguments)]
fn flush_ai_batch(
    lines: &mut Vec<Line<'static>>,
    batch: &mut Vec<(String, String)>,
    app: &mut App,
    cumulative_visual: &mut usize,
    content_width: usize,
    section_line_map: Option<&mut Vec<Option<String>>>,
    section_info: Option<&mut Vec<CollapsedSection>>,
    counters: Option<&mut HashMap<String, u32>>,
) {
    if batch.is_empty() {
        return;
    }
    let combined: String = batch
        .iter()
        .map(|(c, _)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let ts = batch[0].1.clone();
    batch.clear();

    let themes = &app.theme;
    let styles = SectionStyles {
        border: Style::default().fg(themes.ai_border()),
        thinking_border: Style::default().fg(themes.thinking_dim()),
        thinking_text: Style::default()
            .fg(themes.thinking())
            .add_modifier(Modifier::DIM),
        tool_border: Style::default().fg(themes.tool_border()),
        tool_exec: Style::default().fg(themes.tool_text()),
        tool_output: Style::default()
            .fg(themes.tool_text())
            .add_modifier(Modifier::DIM),
        tool_success: Style::default().fg(themes.tool_ok()),
        tool_error: Style::default().fg(themes.tool_err()),
        user_border: Style::default().fg(themes.user_border()),
        user_text: Style::default().fg(themes.user_text()),
        command_border: Style::default().fg(themes.command_border()),
        command_text: Style::default().fg(themes.command_text()),
    };
    let config = SectionConfig {
        first_normal_prefix: "▐ ",
        subsequent_normal_prefix: "▐ ",
    };
    let base = Style::default().fg(Color::Rgb(200, 220, 255));
    let is_dark = app.is_dark();
    lines.extend(render_sectioned_block(
        &combined,
        &ts,
        &config,
        &styles,
        base,
        |full_line, prefix, border| {
            let mut rendered =
                render_markdown_line_with_syntect(&full_line, base, &app.code_block_hl, is_dark);
            rendered.spans.insert(0, Span::styled(prefix, border));
            rendered
        },
        &app.code_block_hl,
        &mut app.code_block_positions,
        is_dark,
        cumulative_visual,
        content_width,
        section_line_map,
        section_info,
        counters,
    ));
}
/// Pre-wrap a single `Line` into multiple `Line`s, each at most `max_width`
/// visual columns wide. Each resulting line keeps the first span (the border
/// prefix, e.g. `"▐ "`) so that soft-wrapped continuation lines also show
/// the left border.
///
/// When a line fits within `max_width` it is returned as-is (zero allocation).
fn prewrap_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    if line.spans.is_empty() || line.width() <= max_width {
        return vec![line];
    }

    // The first span is the border prefix (e.g. "▐ " or "\u{258c}")
    let prefix = line.spans[0].clone();
    let prefix_w = prefix.width();
    let available = max_width.saturating_sub(prefix_w);

    if available == 0 {
        // Degenerate case: prefix alone fills the width.
        return vec![line];
    }

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = vec![prefix.clone()];
    let mut current_w: usize = 0;

    for span in &line.spans[1..] {
        let style = span.style;
        let text = span.content.as_ref();
        let span_w = span.width();

        if current_w + span_w <= available {
            // Entire span fits on the current line.
            current.push(span.clone());
            current_w += span_w;
        } else if current_w == 0 {
            // Prefix + this span exceeds available width immediately.
            // Split the span text across multiple lines.
            let mut remaining = text.to_string();
            while !remaining.is_empty() {
                let (chunk, rest) = split_str_at_width(&remaining, available);
                current.push(Span::styled(chunk, style));
                result.push(Line::from(std::mem::take(&mut current)));
                current.push(prefix.clone());
                remaining = rest;
            }
            current_w = 0;
        } else {
            // Flush current line, start a new one with this span.
            result.push(Line::from(std::mem::take(&mut current)));
            current.push(prefix.clone());

            if span_w <= available {
                current.push(span.clone());
                current_w = span_w;
            } else {
                // This span alone exceeds available width.
                let mut remaining = text.to_string();
                while !remaining.is_empty() {
                    let (chunk, rest) = split_str_at_width(&remaining, available);
                    current.push(Span::styled(chunk, style));
                    result.push(Line::from(std::mem::take(&mut current)));
                    current.push(prefix.clone());
                    remaining = rest;
                }
                current_w = 0;
            }
        }
    }

    if current.len() > 1 {
        result.push(Line::from(current));
    }

    result
}

/// Split a string at a given visual width (using `unicode-width`),
/// returning `(left_part, right_part)`.  The left part is guaranteed to
/// have a visual width ≤ `width`.
fn split_str_at_width(s: &str, width: usize) -> (String, String) {
    if s.is_empty() || width == 0 {
        return (String::new(), s.to_string());
    }
    let mut left = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = UnicodeWidthStr::width(c.to_string().as_str());
        if w + cw > width {
            break;
        }
        left.push(c);
        w += cw;
    }
    let right = s[left.len()..].to_string();
    (left, right)
}

/// Replace collapsed section lines with a single summary line each.
///
/// Sections whose `id` is in `collapsed` get their content lines removed and
/// replaced by a single dimmed line showing `▶ type (N)`. Both `section_info`
/// and `section_line_map` are updated to reflect the new indices.
fn apply_collapsed(
    lines: &mut Vec<Line<'static>>,
    section_info: &mut [CollapsedSection],
    section_line_map: &mut Vec<Option<String>>,
    collapsed: &HashSet<String>,
    styles: &SectionStyles,
) {
    // Pass 1: compute the cumulative line shift for each section.
    // The shift is the total number of lines removed by collapsing sections
    // that appear EARLIER in the vector (lower index).
    let mut shifts: Vec<usize> = vec![0; section_info.len()];
    let mut cum_shift: usize = 0;
    for (i, sec) in section_info.iter().enumerate() {
        shifts[i] = cum_shift;
        if collapsed.contains(&sec.id) {
            cum_shift += sec.line_count.saturating_sub(1);
        }
    }

    // Pass 2: splice lines and update metadata (reverse to keep splice
    // indices valid as we go).
    for i in (0..section_info.len()).rev() {
        let adj_start = section_info[i].start_line.saturating_sub(shifts[i]);

        if collapsed.contains(&section_info[i].id) {
            let end = adj_start + section_info[i].line_count;
            let sec_type = section_info[i].section_type.as_str();
            let n = section_info[i].line_count;

            // Pick the border colour based on section type
            let border_style = match sec_type {
                "thinking" => styles.thinking_border,
                "tool" => styles.tool_border,
                "user" => styles.user_border,
                "command" => styles.command_border,
                _ => styles.border,
            };

            let summary = Line::from(Span::styled(
                format!("\u{2590} \u{25b6} {} ({})  [click to expand]", sec_type, n),
                border_style.add_modifier(Modifier::DIM),
            ));

            // Replace section lines with a single summary line
            lines.splice(adj_start..end, std::iter::once(summary.clone()));

            // Keep section_line_map in sync: remove the old entries, insert
            // one entry for the summary line.
            let section_id = section_info[i].id.clone();
            section_line_map.splice(adj_start..end, std::iter::repeat_n(Some(section_id), 1));

            // Update section metadata — keep original line_count for
            // the click handler (toast) which reads it before next render.
            section_info[i].start_line = adj_start;
        } else {
            section_info[i].start_line = adj_start;
        }
    }
}

fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
    if app.show_welcome {
        render_welcome_banner(f, area, app);
        return;
    }

    // Reset per-frame code block positions so mouse clicks map to the
    // current frame's rendered blocks only.
    app.code_block_positions.clear();

    // Compute content width early — needed for visual row tracking during render.
    let content_width = (area.width.saturating_sub(2)).max(1) as usize;

    let mut lines: Vec<Line> = Vec::with_capacity(app.messages.len() + 4);
    let mut ai_batch: Vec<(String, String)> = Vec::new();
    let mut cumulative_visual: usize = 0;
    let mut section_line_map: Vec<Option<String>> = Vec::new();
    let mut section_info: Vec<CollapsedSection> = Vec::new();
    let mut counters: HashMap<String, u32> = HashMap::new();

    for idx in 0..app.messages.len() {
        let msg = app.messages[idx].clone();
        let ts = if app.show_timestamps {
            app.message_timestamps
                .get(idx)
                .map(|t| format!("[{}] ", t.format("%H:%M:%S")))
                .unwrap_or_default()
        } else {
            String::new()
        };

        // AI-batchable messages: thinking, tool activity, and sectioned
        // messages ([normal]/[command]/[user]/[tool]) accumulate so that
        // flush_ai_batch renders consecutive same-type sections as ONE block
        // (sharing borders) instead of one block per message.
        if msg.starts_with("[thinking]")
            || msg.starts_with("[normal]")
            || msg.starts_with("[command]")
            || msg.starts_with("[user]")
            || msg.starts_with("[tool]")
            || msg.starts_with("\u{1f527}")
            || msg.starts_with("\u{2705}")
            || msg.starts_with("\u{274c}")
        {
            ai_batch.push((msg.clone(), ts));
            continue;
        }

        // Every other message type flushes the AI batch first.
        flush_ai_batch(
            &mut lines,
            &mut ai_batch,
            app,
            &mut cumulative_visual,
            content_width,
            Some(&mut section_line_map),
            Some(&mut section_info),
            Some(&mut counters),
        );

        if msg.starts_with("> ") && !msg.starts_with("> /") {
            let content_style = Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD);

            // Top border extension
            let top_line = Line::from(Span::styled(
                "\u{2590}",
                Style::default().fg(Color::Rgb(60, 80, 60)),
            ));
            cumulative_visual += visual_line_count(&top_line, content_width);
            lines.push(top_line);

            for line_text in msg.split("\n") {
                let l = Line::from(vec![
                    Span::styled("\u{2590} ", Style::default().fg(Color::Rgb(60, 80, 60))),
                    Span::styled(
                        format!("{}{}", ts, line_text.trim_start_matches("> ")),
                        content_style,
                    ),
                ]);
                cumulative_visual += visual_line_count(&l, content_width);
                lines.push(l);
            }

            let bottom_line = Line::from(Span::styled(
                "\u{2590}",
                Style::default().fg(Color::Rgb(60, 80, 60)),
            ));
            cumulative_visual += visual_line_count(&bottom_line, content_width);
            lines.push(bottom_line);
        } else if msg.starts_with("> /") {
            let style = Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD);
            let l = Line::from(Span::styled(msg.clone(), style));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Error:") || msg.starts_with("Error :") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Subagent '") && msg.contains("created") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Yellow),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Subagent '") && msg.contains("completed") {
            let l = Line::from(Span::styled(msg.clone(), Style::default().fg(Color::Green)));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Switched to session:")
            || msg.starts_with("Session renamed to:")
            || (msg.starts_with("Session ")
                && (msg.contains("deleted") || msg.contains("deleted.")))
            || msg.starts_with("Anacleto shutting down")
            || msg.starts_with("Anacleto started")
        {
            let l = Line::from(Span::styled(msg.clone(), Style::default().fg(Color::Blue)));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Agent '") && msg.contains("created") {
            let l = Line::from(Span::styled(msg.clone(), Style::default().fg(Color::Cyan)));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Unknown command") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("Usage:") || msg.starts_with("Commands:") {
            let l = Line::from(Span::styled(msg.clone(), Style::default().fg(Color::Blue)));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("$ ") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("\u{2502} ") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default()
                    .fg(Color::Rgb(160, 160, 180))
                    .add_modifier(Modifier::DIM),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("\u{2514} ") {
            let l = Line::from(Span::styled(
                msg.clone(),
                Style::default()
                    .fg(Color::Rgb(220, 120, 120))
                    .add_modifier(Modifier::DIM),
            ));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("\u{1f50d}") {
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::BOLD);
            let l = Line::from(Span::styled(msg.clone(), style));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else if msg.starts_with("  ") && app.debug_mode {
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::DIM);
            let l = Line::from(Span::styled(msg.clone(), style));
            cumulative_visual += visual_line_count(&l, content_width);
            lines.push(l);
        } else {
            ai_batch.push((msg.clone(), ts));
        }
    }

    // Flush any remaining AI messages.
    flush_ai_batch(
        &mut lines,
        &mut ai_batch,
        app,
        &mut cumulative_visual,
        content_width,
        Some(&mut section_line_map),
        Some(&mut section_info),
        Some(&mut counters),
    );

    // Add streaming indicator if active
    if let Some(stream) = &app.current_stream {
        let stream_style = Style::default()
            .fg(Color::Rgb(100, 200, 255))
            .add_modifier(Modifier::DIM);
        let themes = &app.theme;
        let styles = SectionStyles {
            border: stream_style,
            thinking_border: Style::default().fg(themes.thinking_dim()),
            thinking_text: Style::default()
                .fg(themes.thinking())
                .add_modifier(Modifier::DIM),
            tool_border: Style::default().fg(themes.tool_border()),
            tool_exec: Style::default().fg(themes.tool_text()),
            tool_output: Style::default()
                .fg(themes.tool_text())
                .add_modifier(Modifier::DIM),
            tool_success: Style::default().fg(themes.tool_ok()),
            tool_error: Style::default().fg(themes.tool_err()),
            user_border: Style::default().fg(themes.user_border()),
            user_text: Style::default().fg(themes.user_text()),
            command_border: Style::default().fg(themes.command_border()),
            command_text: Style::default().fg(themes.command_text()),
        };
        let config = SectionConfig {
            first_normal_prefix: "\u{258c}",
            subsequent_normal_prefix: " ",
        };
        let is_dark = app.is_dark();
        lines.extend(render_sectioned_block(
            stream,
            "",
            &config,
            &styles,
            stream_style,
            |full_line, prefix, border| {
                let mut rendered = render_markdown_line_with_syntect(
                    &full_line,
                    stream_style,
                    &app.code_block_hl,
                    is_dark,
                );
                rendered.spans.insert(0, Span::styled(prefix, border));
                rendered
            },
            &app.code_block_hl,
            &mut app.code_block_positions,
            is_dark,
            &mut cumulative_visual,
            content_width,
            Some(&mut section_line_map),
            Some(&mut section_info),
            Some(&mut counters),
        ));
    }

    // Pre-wrap every line that exceeds the content width so that
    // ratatui never needs to soft-wrap — each visual line is its own
    // `Line` with the proper border prefix.  Without this, wrapped
    // continuation lines lose the "▐ " left border.
    let mut prewrapped: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    let mut line_orig_idx: Vec<usize> = Vec::new();
    for (orig_idx, line) in lines.iter().enumerate() {
        let wrapped = prewrap_line(line.clone(), content_width);
        line_orig_idx.extend(std::iter::repeat_n(orig_idx, wrapped.len()));
        prewrapped.extend(wrapped);
    }
    lines = prewrapped;

    // Expand section_line_map to match prewrapped line indices
    app.section_line_map = Vec::with_capacity(lines.len());
    for &orig_idx in &line_orig_idx {
        app.section_line_map
            .push(section_line_map.get(orig_idx).cloned().flatten());
    }
    // Update section_info start_line indices for prewrapped positions
    let mut updated_si: Vec<CollapsedSection> = Vec::new();
    for section in &section_info {
        let mut new_start = 0;
        let mut new_line_count = lines.len(); // default: remaining lines
        // Find first prewrapped line that belongs to this section
        'outer: for (pw_idx, &orig_idx) in line_orig_idx.iter().enumerate() {
            if orig_idx >= section.start_line {
                new_start = pw_idx;
                // Count consecutive prewrapped lines belonging to this section
                let end_orig = section.start_line + section.line_count;
                for (pw_idx2, &orig_idx2) in line_orig_idx.iter().enumerate().skip(pw_idx) {
                    if orig_idx2 >= end_orig {
                        new_line_count = pw_idx2 - pw_idx;
                        break 'outer;
                    }
                }
                new_line_count = lines.len() - pw_idx;
                break 'outer;
            }
        }
        updated_si.push(CollapsedSection {
            start_line: new_start,
            line_count: new_line_count,
            ..section.clone()
        });
    }
    app.section_info = updated_si;

    // ── Apply collapsed sections ──
    // If any sections are marked collapsed, replace their lines with a single
    // summary line so the user sees a compact view.
    if !app.collapsed_sections.is_empty() {
        let themes = &app.theme;
        let collapse_styles = SectionStyles {
            border: Style::default().fg(themes.ai_border()),
            thinking_border: Style::default().fg(themes.thinking_dim()),
            thinking_text: Style::default()
                .fg(themes.thinking())
                .add_modifier(Modifier::DIM),
            tool_border: Style::default().fg(themes.tool_border()),
            tool_exec: Style::default().fg(themes.tool_text()),
            tool_output: Style::default()
                .fg(themes.tool_text())
                .add_modifier(Modifier::DIM),
            tool_success: Style::default().fg(themes.tool_ok()),
            tool_error: Style::default().fg(themes.tool_err()),
            user_border: Style::default().fg(themes.user_border()),
            user_text: Style::default().fg(themes.user_text()),
            command_border: Style::default().fg(themes.command_border()),
            command_text: Style::default().fg(themes.command_text()),
        };
        apply_collapsed(
            &mut lines,
            &mut app.section_info,
            &mut app.section_line_map,
            &app.collapsed_sections,
            &collapse_styles,
        );
    }

    let title = format!(" [2] \u{1f4ac} Chat [{}] ", app.session_name);
    let chat_border = if app.focus == Focus::Chat {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    // Instead of using Paragraph::scroll() which can leave content hidden when
    // wrapping creates more visual lines than logical ones we pre-select the
    // subset of lines that fits the visible area then render with scroll(0,0).
    let visible = (area.height.max(2) as usize) - 2; // minus borders

    // Compute the absolute line index of each code block's [copy] line
    // so mouse clicks can be matched back.
    {
        let mut block_iter = app.code_block_positions.iter_mut();
        let mut current_block = block_iter.next();

        for (idx, line) in lines.iter().enumerate() {
            let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if full.contains("[copy]")
                && let Some(b) = current_block.as_mut()
            {
                b.copy_line = idx;
                current_block = block_iter.next();
            }
        }
        // Fallback if last block wasn't matched
        if let Some(b) = current_block.take() {
            b.copy_line = lines.len().saturating_sub(1);
        }
    }

    // Save the full rendered lines for mouse-click handling.
    app.rendered_chat_lines = lines.clone();

    // Select visible portion: walk backwards from the end accumulating visual rows
    // (accounting for wrapping) until we fill the visible rows.
    let vs = select_visible_start(&lines, visible, content_width, app.chat_scroll);
    let display_lines: Vec<Line> = lines.into_iter().skip(vs.start_idx as usize).collect();

    let paragraph = Paragraph::new(display_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(chat_border))
                .title(title),
        )
        .scroll((0, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, area);
}
fn render_welcome_banner(f: &mut Frame, area: Rect, app: &App) {
    let version = env!("CARGO_PKG_VERSION");
    let banner_lines = vec![
        Line::from(Span::styled(
            format!(" ⬡ anacleto v{} ", version),
            Style::default()
                .fg(Color::Rgb(255, 107, 107))
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Agent Orchestration Engine ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Type /help for commands ",
            Style::default()
                .fg(Color::Rgb(150, 150, 180))
                .add_modifier(Modifier::DIM),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            if app.kb_supported {
                " ⌨  Shift+Enter: newline "
            } else {
                " ⚠  Ctrl+J: newline (Shift+Enter unsupported) "
            },
            Style::default().fg(if app.kb_supported {
                Color::Rgb(100, 200, 100)
            } else {
                Color::Rgb(255, 180, 80)
            }),
        )),
    ];

    let banner = Paragraph::new(banner_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(80, 80, 120)))
                .style(Style::default().bg(Color::Rgb(20, 20, 30))),
        )
        .alignment(Alignment::Center);

    // Center the banner vertically by padding
    let banner_height = 7u16;
    let vert_pad = area.height.saturating_sub(banner_height) / 2;
    let banner_area = Rect {
        x: area.x + area.width.saturating_sub(46).min(area.width) / 2,
        y: area.y + vert_pad,
        width: 46.min(area.width),
        height: banner_height.min(area.height),
    };

    f.render_widget(banner, banner_area);
}

fn render_session_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .session_list
        .iter()
        .map(|s| {
            let active_marker = if Some(s.id.to_string()) == app.session_id {
                " ◀"
            } else {
                ""
            };
            let pinned_marker = if s.pinned { "📌" } else { "  " };
            let style = if Some(s.id.to_string()) == app.session_id {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "{} {}  msgs:{}  {}  {}{}",
                    pinned_marker,
                    &s.id.to_string()[..8],
                    s.message_count,
                    s.name,
                    s.updated_at.format("%Y-%m-%d %H:%M"),
                    active_marker,
                ),
                style,
            )))
        })
        .collect();

    let sessions_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Sessions (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(sessions_list, area);
}

/// Render the agent list overlay.
fn render_agent_list(f: &mut Frame, area: Rect, app: &App) {
    // Separate roots from subagents
    let roots: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .collect();
    let subagents: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::SubAgent)
        .collect();

    let mut items: Vec<ListItem> = Vec::new();

    // Root agents section
    if !roots.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "─── Root Agents ───",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))));
        for agent in &roots {
            items.push(build_agent_list_item(agent, agent.name == app.active_agent));
        }
    }

    // Subagents section
    if !subagents.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "─── SubAgents ───",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ))));
        for agent in &subagents {
            items.push(build_agent_list_item(agent, agent.name == app.active_agent));
        }
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No agents loaded.",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let agent_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Agents (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(agent_list, area);
}

fn build_agent_list_item(agent: &AgentInfo, active: bool) -> ListItem<'static> {
    // Status badge
    let (status_color, badge) = match &agent.status {
        AgentStatus::Idle => (Color::Green, " IDLE "),
        AgentStatus::Working => (Color::Yellow, " BUSY "),
        AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT "),
        AgentStatus::Completed => (Color::DarkGray, " DONE "),
        AgentStatus::Error(_) => (Color::Red, " ERR  "),
    };

    let badge_span = Span::styled(
        badge.to_string(),
        Style::default()
            .fg(status_color)
            .add_modifier(Modifier::REVERSED),
    );

    // Active agent marker: a ▶ prefix with a highlighted background on the name.
    let marker_span = if active {
        Span::styled(
            "▶ ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let name_span = Span::styled(
        agent.name.clone(),
        Style::default().add_modifier(Modifier::BOLD).bg(if active {
            Color::Magenta
        } else {
            Color::Reset
        }),
    );

    let mut spans = vec![
        marker_span,
        badge_span,
        Span::raw(" ".to_string()),
        name_span,
    ];

    // Model info
    if !agent.model.is_empty() {
        spans.push(Span::raw(" [".to_string()));
        spans.push(Span::styled(
            agent.model.clone(),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::raw("]".to_string()));
    }

    // Type (configured subagent type, or "generic" for dynamic subagents).
    if agent.role == AgentRole::SubAgent {
        spans.push(Span::raw(" [".to_string()));
        spans.push(Span::styled(
            agent
                .agent_type
                .clone()
                .unwrap_or_else(|| "generic".to_string()),
            Style::default().fg(Color::Cyan),
        ));
        spans.push(Span::raw("]".to_string()));
    }

    // Mode (only for subagents that carry one).
    if let Some(mode) = &agent.mode {
        let label = match mode {
            TaskMode::Foreground => "fg",
            TaskMode::Background => "bg",
        };
        spans.push(Span::raw(" (".to_string()));
        spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw(")".to_string()));
    }

    // Skills
    if !agent.skills.is_empty() {
        spans.push(Span::raw("  skills: ".to_string()));
        spans.push(Span::styled(
            agent.skills.join(", "),
            Style::default().fg(Color::Cyan),
        ));
    }

    // MCPs
    if !agent.mcps.is_empty() {
        spans.push(Span::raw("  mcps: ".to_string()));
        spans.push(Span::styled(
            agent.mcps.join(", "),
            Style::default().fg(Color::Magenta),
        ));
    }

    // Subagent count
    if agent.subagent_count > 0 {
        spans.push(Span::raw("  children: ".to_string()));
        spans.push(Span::styled(
            agent.subagent_count.to_string(),
            Style::default().fg(Color::Blue),
        ));
    }

    ListItem::new(Line::from(spans))
}

/// Render the subagent tree overlay showing the hierarchy.
fn render_subagent_tree(f: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    // Find root agents with subagents
    let roots: Vec<&AgentInfo> = app
        .agents
        .iter()
        .filter(|a| a.role == AgentRole::Root)
        .collect();

    if roots.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No agents loaded.",
            Style::default().fg(Color::DarkGray),
        ))));
    } else {
        for root in &roots {
            // Root agent line
            let (status_color, badge) = match &root.status {
                AgentStatus::Idle => (Color::Green, " IDLE "),
                AgentStatus::Working => (Color::Yellow, " BUSY "),
                AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT "),
                AgentStatus::Completed => (Color::DarkGray, " DONE "),
                AgentStatus::Error(_) => (Color::Red, " ERR  "),
            };

            let mut root_spans = vec![
                Span::styled("┌─ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    badge,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::REVERSED),
                ),
                Span::raw(" "),
                Span::styled(
                    &root.name,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if !root.model.is_empty() {
                root_spans.push(Span::raw(" ["));
                root_spans.push(Span::styled(
                    &root.model,
                    Style::default().fg(Color::DarkGray),
                ));
                root_spans.push(Span::raw("]"));
            }
            items.push(ListItem::new(Line::from(root_spans)));

            // Find children (subagents whose parent_id matches this root)
            let children: Vec<&AgentInfo> = app
                .agents
                .iter()
                .filter(|a| a.parent_id == Some(root.id.clone()))
                .collect();

            // Configured subagents for this root that haven't been spawned yet.
            let spawned_names: std::collections::HashSet<&str> =
                children.iter().map(|c| c.name.as_str()).collect();
            let pending: Vec<&String> = app
                .configured_subagents
                .get(&root.name)
                .map(|names| {
                    names
                        .iter()
                        .filter(|n| !spawned_names.contains(n.as_str()))
                        .collect()
                })
                .unwrap_or_default();

            let total = children.len() + pending.len();

            if total == 0 {
                items.push(ListItem::new(Line::from(Span::styled(
                    "  │  (no subagents)",
                    Style::default().fg(Color::DarkGray),
                ))));
            } else {
                for (i, child) in children.iter().enumerate() {
                    let is_last = i == total - 1;
                    let (child_status_color, child_badge) = match &child.status {
                        AgentStatus::Idle => (Color::Green, " IDLE  "),
                        AgentStatus::Working => (Color::Yellow, " BUSY  "),
                        AgentStatus::WaitingForSubAgent => (Color::Blue, " WAIT  "),
                        AgentStatus::Completed => (Color::DarkGray, " DONE  "),
                        AgentStatus::Error(_) => (Color::Red, " ERR   "),
                    };

                    let prefix = if is_last { "└── " } else { "├── " };
                    let child_spans = vec![
                        Span::styled(
                            format!("│ {}", prefix),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            child_badge,
                            Style::default()
                                .fg(child_status_color)
                                .add_modifier(Modifier::REVERSED),
                        ),
                        Span::raw(" "),
                        Span::styled(&child.name, Style::default().fg(Color::Magenta)),
                        Span::styled(
                            format!(" [{}]", child.agent_type.as_deref().unwrap_or("generic")),
                            Style::default().fg(Color::Cyan),
                        ),
                        if let Some(mode) = &child.mode {
                            let label = match mode {
                                TaskMode::Foreground => "fg",
                                TaskMode::Background => "bg",
                            };
                            Span::styled(
                                format!(" ({label})"),
                                Style::default().fg(Color::DarkGray),
                            )
                        } else {
                            Span::raw("")
                        },
                    ];
                    items.push(ListItem::new(Line::from(child_spans)));
                }

                // Configured but not yet spawned subagents.
                for (j, name) in pending.iter().enumerate() {
                    let idx = children.len() + j;
                    let is_last = idx == total - 1;
                    let prefix = if is_last { "└── " } else { "├── " };
                    let child_spans = vec![
                        Span::styled(
                            format!("│ {}", prefix),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            " PEND ",
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::REVERSED),
                        ),
                        Span::raw(" "),
                        Span::styled(name.as_str(), Style::default().fg(Color::DarkGray)),
                        Span::styled(" (not created)", Style::default().fg(Color::DarkGray)),
                    ];
                    items.push(ListItem::new(Line::from(child_spans)));
                }
            }

            // Blank separator between roots
            items.push(ListItem::new(Line::from(Span::raw(""))));
        }
    }

    let tree_list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title("Subagent Tree (Esc to close)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(tree_list, area);
}

fn render_input(f: &mut Frame, area: Rect, app: &App) {
    let title = if let Some(flow) = &app.init_flow {
        format!(" Init — {} ", flow.prompt())
    } else {
        " [1] Input ".to_string()
    };
    let input_border = if app.focus == Focus::Input {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(input_border))
        .title(title);

    let mut textarea = app.textarea.clone();
    textarea.set_block(block);
    f.render_widget(&textarea, area);
}

/// Render the human-in-the-loop approval dialog as a centered overlay.
fn render_approval_dialog(f: &mut Frame, area: Rect, app: &App) {
    let Some(ref approval) = app.pending_approval else {
        return;
    };

    // Dialog dimensions
    let dialog_width = area.width.min(60);
    let dialog_height = 7;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    // Clear area behind dialog with a semi-transparent effect
    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    // Build dialog content
    let lines = vec![
        Line::from(Span::styled(
            " ⚠  Approval Required ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            &approval.operation,
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            " Press Y to approve  |  Press N to deny ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
        )),
    ];

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow))
                .style(Style::default().bg(Color::Rgb(40, 30, 0))),
        )
        .alignment(ratatui::layout::Alignment::Center);

    f.render_widget(dialog, dialog_area);
}

/// Render the inline question dialog (`/question` tool).
fn render_question_dialog(f: &mut Frame, area: Rect, app: &App) {
    let Some(ref q) = app.pending_question else {
        return;
    };

    let dialog_width = area.width.min(70);
    let dialog_height = 12;
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    let mut lines = vec![
        Line::from(Span::styled(
            " ❓ Question ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(&q.question, Style::default().fg(Color::White))),
        Line::from(Span::raw("")),
    ];

    if !q.options.is_empty() {
        for (i, opt) in q.options.iter().enumerate() {
            let marker = if i == q.selected { "▸" } else { " " };
            let style = if i == q.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(
                format!(" {} {}", marker, opt),
                style,
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!(" ❯ {}", q.answer_input),
            Style::default().fg(Color::Green),
        )));
    }

    if let Some(rec) = &q.recommended {
        lines.push(Line::from(Span::styled(
            format!(" (recomendado: {})", rec),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        " Enter: submit  |  Esc: cancel  |  ↑/↓: select option ",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
    )));

    let dialog = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Rgb(0, 30, 40))),
        )
        .alignment(ratatui::layout::Alignment::Left);

    f.render_widget(dialog, dialog_area);
}

/// Render the Ctrl+E edit-agent/subagent dialog.
fn render_edit_dialog(f: &mut Frame, area: Rect, app: &App) {
    if !app.edit_dialog.visible {
        return;
    }

    let ed = &app.edit_dialog;

    // Dialog dimensions
    let dialog_width = area.width.min(70);
    let dialog_height = if ed.is_root { 20 } else { 17 };
    let x = area.x + (area.width.saturating_sub(dialog_width)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_height)) / 2;
    let dialog_area = Rect::new(x, y, dialog_width, dialog_height);

    // Clear area behind dialog
    let overlay = ratatui::widgets::Clear;
    f.render_widget(overlay, dialog_area);

    let mut lines: Vec<Line> = Vec::new();

    // Title
    let role_label = if ed.is_root { "Agente" } else { "Subagente" };
    lines.push(Line::from(Span::styled(
        format!(" ✏️  Editando {}: {} ", role_label, ed.target_name),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw("")));

    // Section tabs header
    let section_labels: Vec<&str> = if ed.is_root {
        vec![" Skills ", " MCPs ", " SubAgentes "]
    } else {
        vec![" Skills ", " MCPs "]
    };
    let mut tab_spans: Vec<Span> = Vec::new();
    for (i, label) in section_labels.iter().enumerate() {
        let style = if i == ed.section {
            Style::default()
                .fg(Color::Black)
                .bg(app.theme.accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        tab_spans.push(Span::styled(*label, style));
        tab_spans.push(Span::raw(" │ "));
    }
    if !tab_spans.is_empty() {
        tab_spans.pop(); // remove trailing separator
    }
    lines.push(Line::from(tab_spans));
    lines.push(Line::from(Span::raw("")));

    // Current section items
    let items: &[String] = match ed.section {
        0 => &ed.all_skills,
        1 => &ed.all_mcps,
        _ => &ed.all_subagents,
    };
    let enabled: &[bool] = match ed.section {
        0 => &ed.skills_enabled,
        1 => &ed.mcps_enabled,
        _ => &ed.subagents_enabled,
    };

    if items.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no hay elementos)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        let start = ed.index.saturating_sub(8);
        let end = std::cmp::min(start + 10, items.len());
        for i in start..end {
            let checkbox = if enabled[i] { "[\u{2713}]" } else { "[ ]" };
            let marker = if i == ed.index { "\u{25b8}" } else { " " };
            let style = if i == ed.index {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!(" {} {} {} ", marker, checkbox, items[i]),
                style,
            )));
        }
    }

    // Footer
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  ←/→: sección  |  ↑/↓: navegar  |  Espacio: toggle  |  Enter: confirmar  |  Esc: cancelar ",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
    )));

    let dialog = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(0, 30, 40))),
    );

    f.render_widget(dialog, dialog_area);
}

/// Apply Shift mapping for a character under the Kitty keyboard enhancement protocol.
///
/// With `REPORT_ALL_KEYS_AS_ESCAPE_CODES`, Kitty sends the unshifted physical key
/// code plus a SHIFT modifier, instead of the pre-shifted character. The terminal
/// no longer performs keyboard-layout-dependent shift mapping, so we must do it
/// ourselves. This function uses the `$LANG` locale to determine the layout:
/// `es_*` → Spanish, anything else → US English.
pub(crate) fn shift_char(c: char, lang: &str) -> char {
    let es = lang.starts_with("es_");
    match c {
        'A'..='Z' => c, // already uppercased by crossterm parser
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => {
            if es {
                '"'
            } else {
                '@'
            }
        }
        '3' => {
            if es {
                '·'
            } else {
                '#'
            }
        }
        '4' => '$',
        '5' => '%',
        '6' => '&',
        '7' => {
            if es {
                '/'
            } else {
                '&'
            }
        }
        '8' => '(',
        '9' => ')',
        '0' => {
            if es {
                '='
            } else {
                ')'
            }
        }
        '-' => '_',
        '\'' => '?',
        '`' => {
            if es {
                '^'
            } else {
                '~'
            }
        }
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        ',' => {
            if es {
                ';'
            } else {
                '<'
            }
        }
        '.' => {
            if es {
                ':'
            } else {
                '>'
            }
        }
        '/' => '?',
        _ => c,
    }
}

/// Copy text to the system clipboard.
/// Tries `wl-copy` (Wayland) first, then `xclip` (X11).
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    // Try wl-copy (Wayland)
    if let Ok(mut child) = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write;
        let _ = child.stdin.as_mut().unwrap().write_all(text.as_bytes());
        let _ = child.wait();
        return Ok(());
    }

    // Try xclip (X11)
    if let Ok(mut child) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write;
        let _ = child.stdin.as_mut().unwrap().write_all(text.as_bytes());
        let _ = child.wait();
        return Ok(());
    }

    Err("No clipboard tool found. Install wl-clipboard (Wayland) or xclip (X11)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_styles() -> SectionStyles {
        SectionStyles {
            border: Style::default().fg(Color::White),
            thinking_border: Style::default().fg(Color::Yellow),
            thinking_text: Style::default().fg(Color::Gray),
            tool_border: Style::default().fg(Color::Blue),
            tool_exec: Style::default().fg(Color::Cyan),
            tool_output: Style::default().fg(Color::DarkGray),
            tool_success: Style::default().fg(Color::Green),
            tool_error: Style::default().fg(Color::Red),
            user_border: Style::default().fg(Color::Green),
            user_text: Style::default().fg(Color::Green),
            command_border: Style::default().fg(Color::Magenta),
            command_text: Style::default().fg(Color::Magenta),
        }
    }

    fn raw_line(s: &str) -> Line<'static> {
        Line::from(Span::raw(s.to_string()))
    }

    /// Join all span contents of a line into one String (ignores styles).
    fn line_text(l: &Line<'static>) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn consecutive_same_type_sections_share_border() {
        let styles = test_styles();
        let mut out: Vec<Line> = Vec::new();
        let mut buf1: Vec<Line> = vec![raw_line("a")];
        let mut buf2: Vec<Line> = vec![raw_line("b")];
        let mut cv = 0usize;
        let mut last: Option<&'static str> = None;

        flush_section(
            &mut out, &mut buf1, "tool", &styles, &mut cv, 80, &mut last, None, None, None,
        );
        flush_section(
            &mut out, &mut buf2, "tool", &styles, &mut cv, 80, &mut last, None, None, None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        // Top border, content a, shared separator (bottom of first == no new
        // top for second), content b, bottom border.
        assert_eq!(text, vec!["▐", "a", "▐", "b", "▐"]);
    }

    #[test]
    fn different_type_sections_each_get_top_border() {
        let styles = test_styles();
        let mut out: Vec<Line> = Vec::new();
        let mut buf1: Vec<Line> = vec![raw_line("a")];
        let mut buf2: Vec<Line> = vec![raw_line("x")];
        let mut cv = 0usize;
        let mut last: Option<&'static str> = None;

        flush_section(
            &mut out, &mut buf1, "tool", &styles, &mut cv, 80, &mut last, None, None, None,
        );
        flush_section(
            &mut out, &mut buf2, "thinking", &styles, &mut cv, 80, &mut last, None, None, None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        // Different types: both keep their own top border → two ▐ between a and x.
        assert_eq!(text, vec!["▐", "a", "▐", "▐", "x", "▐"]);
    }

    #[test]
    fn empty_buffer_does_not_update_last_flushed() {
        let styles = test_styles();
        let mut out: Vec<Line> = Vec::new();
        let mut empty: Vec<Line> = Vec::new();
        let mut cv = 0usize;
        let mut last: Option<&'static str> = None;

        flush_section(
            &mut out, &mut empty, "tool", &styles, &mut cv, 80, &mut last, None, None, None,
        );
        assert!(out.is_empty());
        assert_eq!(last, None);
    }

    #[test]
    fn three_consecutive_same_type_blocks_merge() {
        let styles = test_styles();
        let mut out: Vec<Line> = Vec::new();
        let mut buf1: Vec<Line> = vec![raw_line("a")];
        let mut buf2: Vec<Line> = vec![raw_line("b")];
        let mut buf3: Vec<Line> = vec![raw_line("c")];
        let mut cv = 0usize;
        let mut last: Option<&'static str> = None;

        flush_section(
            &mut out, &mut buf1, "normal", &styles, &mut cv, 80, &mut last, None, None, None,
        );
        flush_section(
            &mut out, &mut buf2, "normal", &styles, &mut cv, 80, &mut last, None, None, None,
        );
        flush_section(
            &mut out, &mut buf3, "normal", &styles, &mut cv, 80, &mut last, None, None, None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        assert_eq!(text, vec!["▐", "a", "▐", "b", "▐", "c", "▐"]);
    }

    /// End-to-end: replicate the /copy output (several [normal]-wrapped
    /// messages combined into ONE string, as flush_ai_batch does) and verify
    /// consecutive normal blocks share a border.
    #[test]
    fn end_to_end_copy_output_normal_blocks_merge() {
        let styles = test_styles();
        let config = SectionConfig {
            first_normal_prefix: "▐ ",
            subsequent_normal_prefix: "▐ ",
        };
        let base = Style::default();
        let hl = CodeBlockHighlighter::default();
        let mut positions = Vec::new();
        let mut cv = 0usize;

        let combined = "[normal]\nAnacleto started.\n[/normal]\n\
                        [normal]\nModel changed to: deepseek/deepseek-v4-flash\n[/normal]\n\
                        [normal]\nAgent 'dev-manager' created.\n[/normal]\n\
                        [normal]\n> /copy\n[/normal]";

        let out = render_sectioned_block(
            &combined,
            "",
            &config,
            &styles,
            base,
            |full_line, prefix, border| {
                let mut rendered = Line::from(Span::raw(full_line));
                rendered.spans.insert(0, Span::styled(prefix, border));
                rendered
            },
            &hl,
            &mut positions,
            true,
            &mut cv,
            80,
            None,
            None,
            None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        // 4 consecutive normal blocks → each subsequent block skips its top
        // border; the previous block's bottom border acts as the separator.
        // Result: top(1) + 4 content + 3 separators + bottom(1) = 9 lines,
        // with 5 pure-▐ border lines (1 top + 3 shared + 1 bottom). Without
        // the merge this would be 8 borders + 4 content = 12 lines.
        let border_count = text.iter().filter(|l| l.as_str() == "▐").count();
        assert_eq!(border_count, 5, "borders: {text:?}");
        assert_eq!(text.len(), 9, "lines: {text:?}");
        assert!(text.iter().any(|l| l.contains("Anacleto started.")));
        assert!(text.iter().any(|l| l.contains("> /copy")));
    }

    /// `[command]` blocks render with the command border/text styles, and
    /// consecutive command blocks share a border like other sections.
    #[test]
    fn generate_section_id_increments_per_type() {
        let mut counters = HashMap::new();
        assert_eq!(generate_section_id("thinking", &mut counters), "thinking_1");
        assert_eq!(generate_section_id("thinking", &mut counters), "thinking_2");
        assert_eq!(generate_section_id("tool", &mut counters), "tool_1");
        assert_eq!(generate_section_id("thinking", &mut counters), "thinking_3");
        assert_eq!(generate_section_id("normal", &mut counters), "normal_1");
    }

    #[test]
    fn command_sections_render_with_command_styles_and_merge() {
        let styles = test_styles();
        let config = SectionConfig {
            first_normal_prefix: "▐ ",
            subsequent_normal_prefix: "▐ ",
        };
        let base = Style::default();
        let hl = CodeBlockHighlighter::default();
        let mut positions = Vec::new();
        let mut cv = 0usize;

        let content = "[command]\n> /copy\n[/command]\n[command]\n> /sessions\n[/command]";
        let out = render_sectioned_block(
            &content,
            "",
            &config,
            &styles,
            base,
            |full_line, prefix, border| {
                let mut rendered = Line::from(Span::raw(full_line));
                rendered.spans.insert(0, Span::styled(prefix, border));
                rendered
            },
            &hl,
            &mut positions,
            true,
            &mut cv,
            80,
            None,
            None,
            None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        // 2 consecutive command blocks → shared border: top + 2 content +
        // 1 separator + bottom = 5 lines, 3 pure-▐ borders.
        assert_eq!(text.len(), 5, "lines: {text:?}");
        let border_count = text.iter().filter(|l| l.as_str() == "▐").count();
        assert_eq!(border_count, 3, "borders: {text:?}");
        assert!(text.iter().any(|l| l.contains("> /copy")));
        assert!(text.iter().any(|l| l.contains("> /sessions")));

        // Command lines are styled with the command (magenta) styles.
        let copy_line = out
            .iter()
            .find(|l| line_text(l).contains("> /copy"))
            .expect("command line");
        let border_span = &copy_line.spans[0];
        assert_eq!(border_span.style.fg, Some(Color::Magenta));
        let text_span = &copy_line.spans[1];
        assert_eq!(text_span.style.fg, Some(Color::Magenta));
    }

    #[test]
    fn table_renders_in_normal_section() {
        let styles = test_styles();
        let config = SectionConfig {
            first_normal_prefix: "▐ ",
            subsequent_normal_prefix: "▐ ",
        };
        let base = Style::default();
        let hl = CodeBlockHighlighter::default();
        let mut positions = Vec::new();
        let mut cv = 0usize;

        // Plain text with a table (no [normal] markers — just raw content)
        let content = "Here is a table:\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\nEnd of table.";
        let out = render_sectioned_block(
            &content,
            "",
            &config,
            &styles,
            base,
            |full_line, prefix, border| {
                let mut rendered = Line::from(Span::raw(full_line));
                rendered.spans.insert(0, Span::styled(prefix, border));
                rendered
            },
            &hl,
            &mut positions,
            true,
            &mut cv,
            80,
            None,
            None,
            None,
        );

        let text: Vec<String> = out.iter().map(line_text).collect();
        eprintln!("=== table_renders_in_normal_section ===");
        for (i, t) in text.iter().enumerate() {
            eprintln!("  {}: {}", i, t);
        }

        // Should contain box-drawing characters (table rendered)
        let has_box_drawing = text.iter().any(|l| l.contains('\u{250c}') || l.contains('\u{2510}') || l.contains('\u{2502}'));
        assert!(has_box_drawing, "No box-drawing chars found in output: {text:?}");

        // Should contain table content
        assert!(text.iter().any(|l| l.contains("A") && l.contains("B")));
        assert!(text.iter().any(|l| l.contains("1") && l.contains("2")));
    }
}
