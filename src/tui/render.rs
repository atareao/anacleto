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
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::app::{AgentInfo, App, Focus};
use super::markdown::{render_markdown_line, select_visible_start};
use super::palette::{render_agent_palette, render_command_palette, render_model_palette};
use crate::agent::types::{AgentRole, AgentStatus};

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

/// Render the right panel: 4 stacked info panels (Status, MCPs, Skills, Running agents).
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
    render_mcp_panel(f, chunks[1], app);
    render_skill_panel(f, chunks[2], app);
    render_agent_panel(f, chunks[3], app);
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

/// Panel 2: MCPs — connected MCP server names.
fn render_mcp_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_mcps: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .flat_map(|a| a.mcps.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Mcps;
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
            .title(" (2) MCPs "),
    );

    f.render_widget(list, area);
}

/// Panel 3: Skills — loaded skill names.
fn render_skill_panel(f: &mut Frame, area: Rect, app: &App) {
    let unique_skills: Vec<&str> = {
        let set: std::collections::BTreeSet<&str> = app
            .agents
            .iter()
            .flat_map(|a| a.skills.iter().map(|s| s.as_str()))
            .collect();
        set.into_iter().collect()
    };

    let focused = app.focus == Focus::Skills;
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
            .title(" (3) Skills "),
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
                let item_style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(app.theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if active { "▶ " } else { "  " },
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" {} ", dot),
                        Style::default().fg(dot_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        &a.name,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                            .bg(if active { Color::Magenta } else { Color::Reset }),
                    ),
                    if a.status == AgentStatus::Working {
                        Span::styled(
                            format!(
                                " {}",
                                SPINNER_FRAMES[(app.frame_count as usize) % SPINNER_FRAMES.len()]
                            ),
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        Span::raw("")
                    },
                    Span::styled(format!(" [{}]", role), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!(" ({})", status_str), Style::default().fg(dot_color)),
                ]))
                .style(item_style)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(" (4) Agents "),
    );

    f.render_widget(list, area);
}

/// Render the current working directory (left) and active model (right).
/// When the directory path is too long, it is truncated with an ellipsis (...)
/// to ensure the model name always fits on the right side.
fn render_working_dir(f: &mut Frame, area: Rect, app: &App) {
    let dir_text = format!(" 📁 {}", app.working_dir);
    let model_text = format!("🤖 {}", app.current_model);
    let width = area.width as usize;

    // Use display width (emoji count as 2 columns) so the model ends exactly
    // at the right edge of the terminal.
    let dir_width = dir_text.width();
    let model_width = model_text.width();
    // Leave at least 1 space between dir and model
    let max_dir_width = width.saturating_sub(model_width + 1);

    let truncated_dir = if dir_width > max_dir_width && max_dir_width > 3 {
        // Truncate with ellipsis at the end
        let keep = max_dir_width.saturating_sub(1);
        let mut s: String = String::new();
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

    let padding = width.saturating_sub(truncated_dir.width() + model_width);
    let line = Line::from(vec![
        Span::styled(truncated_dir, Style::default().fg(Color::DarkGray)),
        Span::raw(" ".repeat(padding)),
        Span::styled(model_text, Style::default().fg(Color::Cyan)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn render_chat(f: &mut Frame, area: Rect, app: &App) {
    if app.show_welcome {
        render_welcome_banner(f, area, app);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(app.messages.len() + 4);

    for (idx, m) in app.messages.iter().enumerate() {
        let ts = if app.show_timestamps {
            app.message_timestamps
                .get(idx)
                .map(|t| format!("[{}] ", t.format("%H:%M:%S")))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if m.starts_with("> ") && !m.starts_with("> /") {
            let style = Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD);
            for line_text in m.split('\n') {
                lines.push(Line::from(Span::styled(
                    format!("{}{}", ts, line_text),
                    style,
                )));
            }
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
                Style::default().fg(Color::Red),
            )));
        } else if m.starts_with("Usage:") || m.starts_with("Commands:") {
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Blue),
            )));
        } else if m.starts_with("$ ") {
            // !command prompt — yellow bold
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if m.starts_with("\u{2502} ") {
            // stdout from !command — gray, dimmed
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(160, 160, 180))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{2514} ") {
            // stderr from !command — red, dimmed
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Rgb(220, 120, 120))
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{1f527}") {
            // Tool execution tracing — cyan
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{2705}") {
            // Tool result success — green dim
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{274c}") {
            // Tool result failure — red dim
            lines.push(Line::from(Span::styled(
                m.clone(),
                Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
            )));
        } else if m.starts_with("\u{1f50d}") {
            // Debug header — purple bold
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else if m.starts_with("  ") && app.debug_mode {
            // Debug payload — purple dim
            let style = Style::default()
                .fg(Color::Rgb(180, 100, 255))
                .add_modifier(Modifier::DIM);
            lines.push(Line::from(Span::styled(m.clone(), style)));
        } else {
            // AI responses — split by newline, render markdown per line
            let base = Style::default().fg(Color::Rgb(200, 220, 255));
            for (i, line_text) in m.split('\n').enumerate() {
                let prefix = if i == 0 { ts.as_str() } else { "" };
                lines.push(render_markdown_line(
                    &format!("{}{}", prefix, line_text),
                    base,
                ));
            }
        }
    }

    // Add streaming indicator if active
    // IMPORTANT: split by newlines so each logical Line = (roughly) one visual line.
    // Without this split, a long streaming response wraps to many visual lines but
    // counts as a single logical line, making bottom content invisible & unscrollable.
    if let Some(stream) = &app.current_stream {
        let style = Style::default()
            .fg(Color::Rgb(100, 200, 255))
            .add_modifier(Modifier::DIM);
        for (idx, line_text) in stream.split('\n').enumerate() {
            let prefix = if idx == 0 { "\u{258c}" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, line_text),
                style,
            )));
        }
    }

    let title = format!(" (1) \u{1f4ac} Chat [{}] ", app.session_name);
    let chat_border = if app.focus == Focus::Chat {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    // Instead of using Paragraph::scroll() — which can leave content hidden when
    // wrapping creates more visual lines than logical ones — we pre-select the
    // subset of lines that fits the visible area, then render with scroll(0,0).
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

/// Render a welcome banner centered in the chat area when there are no messages yet.
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

    // Split input into lines
    let lines: Vec<&str> = app.input.split('\n').collect();

    // Build rendered lines: first line gets prompt, rest get 3-space indent
    let mut rendered: Vec<Line> = Vec::with_capacity(lines.len());
    for (i, line_text) in lines.iter().enumerate() {
        if i == 0 {
            rendered.push(Line::from(vec![prompt.clone(), Span::raw(*line_text)]));
        } else {
            rendered.push(Line::from(vec![
                Span::raw("   "), // 3-space indent to align with text after " ❯ "
                Span::raw(*line_text),
            ]));
        }
    }

    // Content width available for text (minus borders and prompt/indent).
    let inner_width = area.width.saturating_sub(2) as usize; // 2 for borders
    let first_line_width = inner_width.saturating_sub(3); // " ❯ " prompt
    let rest_line_width = inner_width.saturating_sub(3); // 3-space indent

    // Compute how many visual rows each logical line occupies when wrapped.
    let mut visual_rows: Vec<usize> = Vec::with_capacity(lines.len());
    for (i, line_text) in lines.iter().enumerate() {
        let w = if i == 0 {
            first_line_width
        } else {
            rest_line_width
        };
        let len = line_text.chars().count();
        visual_rows.push(if w == 0 { 1 } else { len.div_ceil(w).max(1) });
    }
    let total_visual: usize = visual_rows.iter().sum();

    // Bottom-anchored scroll: show last N visual rows where N = visible rows minus borders
    let visible_rows = (area.height.saturating_sub(2)) as usize; // 2 for borders
    let scroll_offset = total_visual.saturating_sub(visible_rows);

    let title = if let Some(flow) = &app.init_flow {
        format!(" Init — {} ", flow.prompt())
    } else {
        " (5) Input ".to_string()
    };
    let input_border = if app.focus == Focus::Input {
        app.theme.accent()
    } else {
        Color::DarkGray
    };

    let paragraph = Paragraph::new(rendered)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(input_border))
                .title(title),
        )
        .scroll((scroll_offset as u16, 0))
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, area);

    // Cursor: position at `input_cursor` (char index), accounting for wrap.
    let cursor_char = app.input_cursor.min(app.input.chars().count());
    // Find the logical line containing the cursor and the char offset within it.
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
        remaining -= line_chars + 1; // +1 for the '\n' separator
        cursor_line_idx = i + 1;
    }
    let cursor_w = if cursor_line_idx == 0 {
        first_line_width
    } else {
        rest_line_width
    };
    // Visual row of the cursor within its logical line (0-based).
    let cursor_visual_in_line = col_in_line.checked_div(cursor_w).unwrap_or(0);
    // Column within the wrapped row (0-based), plus prompt/indent offset.
    let col_in_row = col_in_line.checked_rem(cursor_w).unwrap_or(0);
    // Visual row of the cursor's logical line start (sum of previous lines' visual rows).
    let cursor_line_start: usize = visual_rows[..cursor_line_idx].iter().sum();
    let cursor_visual = cursor_line_start + cursor_visual_in_line;
    let cursor_row = area.y + 1 + (cursor_visual.saturating_sub(scroll_offset)) as u16;
    let cursor_col = area.x + 1 + 3 + col_in_row as u16;
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
