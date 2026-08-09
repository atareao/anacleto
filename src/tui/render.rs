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
use super::markdown::{render_markdown_line, render_table_block, select_visible_start};
use super::palette::{render_agent_palette, render_command_palette, render_model_palette};
use super::types::{AgentInfo, Focus};
use crate::agent::types::{AgentRole, AgentStatus, TaskMode};

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
                Constraint::Length(4), // input
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
fn render_main_content(f: &mut Frame, area: Rect, app: &App) {
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
fn render_left_panel(f: &mut Frame, area: Rect, app: &App) {
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
    tool_ok: Style,
    tool_err: Style,
}

/// Configures whether/how the normal (non-thinking, non-tool) section draws
/// its `▐` borders and which prefix to use on its lines.
struct SectionConfig {
    /// Emit `▐` top/bottom borders around the normal section (true for
    /// committed messages, false for stream where normal is prefix-only).
    normal_has_borders: bool,
    /// Prefix for the first normal line (e.g. "\u{258c}" for stream vs "▐ " for
    /// committed).
    first_normal_prefix: &'static str,
    /// Prefix for subsequent normal lines (e.g. " " for stream vs "▐ " for
    /// committed).
    subsequent_normal_prefix: &'static str,
}

/// Helper: flush accumulated table lines into styled output.
fn flush_table(
    out: &mut Vec<Line>,
    table_buffer: &mut Vec<String>,
    first_normal: &mut bool,
    section_has_content: &mut bool,
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
fn render_sectioned_block(
    content: &str,
    ts: &str,
    config: &SectionConfig,
    styles: &SectionStyles,
    table_cell_style: Style,
    normal_line_render: impl Fn(String, &'static str, Style) -> Line<'static>,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = Vec::new();
    let mut section: &str = "normal";
    let mut section_has_content = false;
    let mut awaiting_ts = !ts.is_empty();
    let mut first_normal = true;
    let mut table_buffer: Vec<String> = Vec::new();

    // Helper to pick the right prefix
    let prefix = |first: bool| -> &'static str {
        if first {
            config.first_normal_prefix
        } else {
            config.subsequent_normal_prefix
        }
    };

    for line_text in content.split('\n') {
        let marker = line_text.trim();

        // ── [thinking] / [/thinking] markers ──
        if marker == "[thinking]" {
            // Flush any pending table before switching sections
            let p = prefix(first_normal);
            flush_table(
                &mut out,
                &mut table_buffer,
                &mut first_normal,
                &mut section_has_content,
                p,
                styles.border,
                table_cell_style,
            );
            // Close previous section only if it had actual content
            if section_has_content {
                let close_style = match section {
                    "thinking" => styles.thinking_border,
                    "tool" => styles.tool_border,
                    _ => styles.border,
                };
                out.push(Line::from(Span::styled("\u{2590}", close_style)));
            }
            section = "thinking";
            section_has_content = false;
            out.push(Line::from(Span::styled("\u{2590}", styles.thinking_border)));
            continue;
        }
        if marker == "[/thinking]" {
            // Flush any pending table before closing thinking section
            let p = prefix(first_normal);
            flush_table(
                &mut out,
                &mut table_buffer,
                &mut first_normal,
                &mut section_has_content,
                p,
                styles.border,
                table_cell_style,
            );
            if section_has_content {
                out.push(Line::from(Span::styled("\u{2590}", styles.thinking_border)));
            }
            section = "normal";
            section_has_content = false;
            continue;
        }

        // Build full line with optional timestamp prefix
        let ts_prefix = if awaiting_ts { ts } else { "" };
        awaiting_ts = false;
        let full_line = format!("{}{}", ts_prefix, line_text);

        // Detect tool markers (🔧 / ✅ / ❌ ) — only strip leading whitespace
        let trimmed = line_text.trim_start();
        let is_tool = trimmed.starts_with("\u{1f527}")
            || trimmed.starts_with("\u{2705}")
            || trimmed.starts_with("\u{274c}");

        // ── Tool section transitions (only outside thinking blocks) ──
        if section != "thinking" {
            let from_normal_to_tool = is_tool && section != "tool";
            let from_tool_to_normal = !is_tool && section == "tool";

            if from_normal_to_tool {
                // Flush any pending table before switching to tool section
                let p = prefix(first_normal);
                flush_table(
                    &mut out,
                    &mut table_buffer,
                    &mut first_normal,
                    &mut section_has_content,
                    p,
                    styles.border,
                    table_cell_style,
                );
                // Close normal section only if it had content AND normal has borders
                if section_has_content && config.normal_has_borders {
                    out.push(Line::from(Span::styled("\u{2590}", styles.border)));
                }
                section = "tool";
                out.push(Line::from(Span::styled("\u{2590}", styles.tool_border)));
            } else if from_tool_to_normal {
                if section_has_content {
                    out.push(Line::from(Span::styled("\u{2590}", styles.tool_border)));
                }
                section = "normal";
            }
        }

        // ── Render the line ──
        if section == "thinking" {
            out.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.thinking_border),
                Span::styled(full_line, styles.thinking_text),
            ]));
            section_has_content = true;
        } else if is_tool {
            let tool_style = if trimmed.starts_with("\u{1f527}") {
                styles.tool_exec
            } else if trimmed.starts_with("\u{2705}") {
                styles.tool_ok
            } else {
                styles.tool_err
            };
            out.push(Line::from(vec![
                Span::styled("\u{2590} ", styles.tool_border),
                Span::styled(full_line, tool_style),
            ]));
            section_has_content = true;
        } else {
            // ── Table detection (consecutive | lines) ──
            if marker.starts_with('|') {
                // Entering table mode: emit normal top border if first content
                if table_buffer.is_empty() && first_normal && config.normal_has_borders {
                    out.push(Line::from(Span::styled("\u{2590}", styles.border)));
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
                p,
                styles.border,
                table_cell_style,
            );

            // Normal line — delegate to caller for markdown / plain rendering
            // Emit top border for normal section when the first normal line appears
            // (not eagerly, so messages that start with [thinking] don't get a
            // spurious normal border before the thinking section).
            if first_normal && config.normal_has_borders {
                out.push(Line::from(Span::styled("\u{2590}", styles.border)));
            }
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
        p,
        styles.border,
        table_cell_style,
    );

    // Close the last section only if it had content
    if section_has_content {
        let close_style = match section {
            "thinking" => styles.thinking_border,
            "tool" => styles.tool_border,
            _ if config.normal_has_borders => styles.border,
            _ => return out,
        };
        out.push(Line::from(Span::styled("\u{2590}", close_style)));
    }

    out
}

/// Flush the accumulated AI message batch: join all messages and render them
/// as a single sectioned block so that thinking → tool → response transitions
/// produce only one `▐` separator between sections, not two.
fn flush_ai_batch(lines: &mut Vec<Line<'static>>, batch: &mut Vec<(String, String)>, app: &App) {
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
        tool_exec: Style::default().fg(themes.tool_text_dim()),
        tool_ok: Style::default()
            .fg(themes.tool_ok_dim())
            .add_modifier(Modifier::DIM),
        tool_err: Style::default()
            .fg(themes.tool_err_dim())
            .add_modifier(Modifier::DIM),
    };
    let config = SectionConfig {
        normal_has_borders: true,
        first_normal_prefix: "▐ ",
        subsequent_normal_prefix: "▐ ",
    };
    let base = Style::default().fg(Color::Rgb(200, 220, 255));
    lines.extend(render_sectioned_block(
        &combined,
        &ts,
        &config,
        &styles,
        base,
        |full_line, prefix, border| {
            let mut rendered = render_markdown_line(&full_line, base);
            rendered.spans.insert(0, Span::styled(prefix, border));
            rendered
        },
    ));
}
fn render_chat(f: &mut Frame, area: Rect, app: &App) {
    if app.show_welcome {
        render_welcome_banner(f, area, app);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(app.messages.len() + 4);
    let mut ai_batch: Vec<(String, String)> = Vec::new();

    for (idx, m) in app.messages.iter().enumerate() {
        let ts = if app.show_timestamps {
            app.message_timestamps
                .get(idx)
                .map(|t| format!("[{}] ", t.format("%H:%M:%S")))
                .unwrap_or_default()
        } else {
            String::new()
        };

        // AI-batchable messages
        if m.starts_with("[thinking]")
            || m.starts_with("\u{1f527}")
            || m.starts_with("\u{2705}")
            || m.starts_with("\u{274c}")
        {
            ai_batch.push((m.clone(), ts));
            continue;
        }

        // Every other message type flushes the AI batch first.
        flush_ai_batch(&mut lines, &mut ai_batch, app);

        if m.starts_with("> ") && !m.starts_with("> /") {
            let content_style = Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD);

            // Top border extension
            lines.push(Line::from(Span::styled(
                "\u{2590}",
                Style::default().fg(Color::Rgb(60, 80, 60)),
            )));

            for line_text in m.split("\n") {
                lines.push(Line::from(vec![
                    Span::styled("\u{2590} ", Style::default().fg(Color::Rgb(60, 80, 60))),
                    Span::styled(
                        format!("{}{}", ts, line_text.trim_start_matches("> ")),
                        content_style,
                    ),
                ]));
            }

            lines.push(Line::from(Span::styled(
                "\u{2590}",
                Style::default().fg(Color::Rgb(60, 80, 60)),
            )));
        } else if m.starts_with("> /") {
            let style = Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else if m.starts_with("Error:") || m.starts_with("Error :") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("Subagent '") && m.contains("created") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Yellow),
            )));
        } else if m.starts_with("Subagent '") && m.contains("completed") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Green),
            )));
        } else if m.starts_with("Switched to session:")
            || m.starts_with("Session renamed to:")
            || (m.starts_with("Session ") && (m.contains("deleted") || m.contains("deleted.")))
            || m.starts_with("Anacleto shutting down")
            || m.starts_with("Anacleto started")
        {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Blue),
            )));
        } else if m.starts_with("Agent '") && m.contains("created") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Cyan),
            )));
        } else if m.starts_with("Unknown command") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("Usage:") || m.starts_with("Commands:") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Blue),
            )));
        } else if m.starts_with("$ ") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("\u{2502} ") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(160, 160, 180))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{2514} ") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(220, 120, 120))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{1f50d}") {
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else if m.starts_with("  ") && app.debug_mode {
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::DIM);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else {
            ai_batch.push((m.clone(), ts));
        }
    }

    // Flush any remaining AI messages.
    flush_ai_batch(&mut lines, &mut ai_batch, app);

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
            tool_ok: Style::default()
                .fg(themes.tool_ok())
                .add_modifier(Modifier::DIM),
            tool_err: Style::default()
                .fg(themes.tool_err())
                .add_modifier(Modifier::DIM),
        };
        let config = SectionConfig {
            normal_has_borders: false,
            first_normal_prefix: "\u{258c}",
            subsequent_normal_prefix: " ",
        };
        lines.extend(render_sectioned_block(
            stream,
            "",
            &config,
            &styles,
            stream_style,
            |full_line, prefix, _border| {
                Line::from(Span::styled(
                    format!("{}{}", prefix, full_line),
                    stream_style,
                ))
            },
        ));
    }

    let title = format!(" (1) \u{1f4ac} Chat [{}] ", app.session_name);
    let chat_border = if app.focus == Focus::Chat {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    // Instead of using Paragraph::scroll() which can leave content hidden when
    // wrapping creates more visual lines than logical ones we pre-select the
    // subset of lines that fits the visible area then render with scroll(0,0).
    let content_width = (area.width.saturating_sub(2)).max(1) as usize; // minus borders
    let visible = (area.height.max(2) as usize) - 2; // minus borders

    // Select visible portion: walk backwards from the end accumulating visual rows
    // (accounting for wrapping) until we fill the visible rows.
    let start_idx = select_visible_start(&lines, visible, content_width, app.chat_scroll);
    let display_lines: Vec<Line> = lines.into_iter().skip(start_idx as usize).collect();

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
    let input_style = Style::default()
        .fg(app.theme.accent())
        .add_modifier(Modifier::BOLD);
    let prompt = Span::styled(" ❯ ", input_style);

    // Split input into logical lines
    let lines: Vec<&str> = app.input.split('\n').collect();

    // Available widths (minus borders).
    let inner_w = area.width.saturating_sub(2) as usize;
    let first_row_text_w = inner_w.saturating_sub(3); // prompt/indent consumes 3 cols
    let wrap_text_w = inner_w; // continuation rows have full width

    // ── Manual character-wrap: build one Line per visual row ────────────
    // We do our own wrapping (character-by-character, NOT word-wrap) so that
    // cursor-position math matches the visual layout exactly.  Ratatui's
    // built-in `Wrap` does word-wrap, which causes the cursor to land in the
    // wrong column when long words straddle the wrap boundary.
    let mut rendered: Vec<Line> = Vec::new();
    // For cursor positioning: for each visual row, store its logical line
    // index and the range of characters it displays.
    struct VisRow {
        line_idx: usize,
        char_start: usize, // first character of this visual row in the logical line
        char_count: usize, // how many characters this row shows
    }
    let mut vis_rows: Vec<VisRow> = Vec::new();

    for (line_idx, line_text) in lines.iter().enumerate() {
        let chars: Vec<char> = line_text.chars().collect();
        let line_len = chars.len();

        if line_len == 0 {
            // Empty logical line still occupies one visual row.
            let prefix = if line_idx == 0 {
                prompt.clone()
            } else {
                Span::raw("   ")
            };
            rendered.push(Line::from(vec![prefix, Span::raw("")]));
            vis_rows.push(VisRow {
                line_idx,
                char_start: 0,
                char_count: 0,
            });
            continue;
        }

        let mut pos = 0usize;
        let mut first = true;
        while pos < line_len {
            let row_width = if first { first_row_text_w } else { wrap_text_w };
            let end = (pos + row_width).min(line_len);
            let chunk: String = chars[pos..end].iter().collect();

            let prefix = if line_idx == 0 && first {
                prompt.clone()
            } else if first {
                Span::raw("   ")
            } else {
                Span::raw("")
            };
            rendered.push(Line::from(vec![prefix, Span::raw(chunk)]));
            vis_rows.push(VisRow {
                line_idx,
                char_start: pos,
                char_count: end - pos,
            });

            pos = end;
            first = false;
        }
    }

    let total_visual = rendered.len();

    // Bottom-anchored scroll: show last N visual rows
    let visible_rows = (area.height.saturating_sub(2)) as usize; // 2 for borders
    let scroll_offset = total_visual.saturating_sub(visible_rows);

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

    // NOTE: no `.wrap()` — we already did character-level wrapping above.
    let paragraph = Paragraph::new(rendered)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(input_border))
                .title(title),
        )
        .scroll((scroll_offset as u16, 0))
        .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, area);

    // ── Cursor positioning ──────────────────────────────────────────────
    // Find which logical line contains `input_cursor`.
    let cursor_char = app.input_cursor.min(app.input.chars().count());
    let mut remaining = cursor_char;
    let mut cursor_line_idx = 0usize;
    let mut col_in_line = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let line_chars = line.chars().count();
        if remaining <= line_chars {
            cursor_line_idx = i;
            col_in_line = remaining;
            break;
        }
        remaining = remaining.saturating_sub(line_chars + 1); // +1 for '\n'
        cursor_line_idx = i + 1;
    }

    // Walk the manually-built visual rows to find which row has this
    // character, and what column within that row.
    let mut cursor_vis_idx = 0usize;
    let mut cursor_col_in_row = 0usize;
    for (vi, vr) in vis_rows.iter().enumerate() {
        if vr.line_idx == cursor_line_idx
            && vr.char_start <= col_in_line
            && col_in_line < vr.char_start + vr.char_count
        {
            cursor_vis_idx = vi;
            cursor_col_in_row = col_in_line - vr.char_start;
            break;
        }
        // If we reach the last visual row of this line and didn't match,
        // the cursor is past the end — place it at the end of the last row.
        if vr.line_idx == cursor_line_idx
            && (vi + 1 >= vis_rows.len() || vis_rows[vi + 1].line_idx != cursor_line_idx)
        {
            cursor_vis_idx = vi;
            cursor_col_in_row = vr.char_count;
            break;
        }
    }

    let cursor_row = area.y + 1 + (cursor_vis_idx.saturating_sub(scroll_offset)) as u16;

    // Column offset: first visual row of its logical line has prompt/indent (3).
    let is_first = vis_rows
        .get(cursor_vis_idx)
        .map(|vr| vr.char_start == 0)
        .unwrap_or(true);
    let col_offset: u16 = if is_first { 3 } else { 0 };
    let cursor_col = area.x + 1 + col_offset + cursor_col_in_row as u16;
    f.set_cursor_position((cursor_col, cursor_row));
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
