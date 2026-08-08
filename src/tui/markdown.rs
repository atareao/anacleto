//! Markdown rendering helpers for the chat panel.
//!
//! These are pure functions that turn raw text lines into styled
//! [`Line`]s, and estimate how many visual rows a line occupies when
//! wrapped at a given width.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Parse a line of text for inline markdown and return styled Spans.
/// Uses COLOR changes (not modifiers) for italic since color is
/// far more visible in terminals than font-weight/italic.
///
///   `**bold**`  -> bright white foreground
///   `*italic*`  -> warm yellow foreground
///   `` `code` `` -> amber on dark background
pub(crate) fn render_markdown_line(text: &str, base_style: Style) -> Line<'static> {
    if text.is_empty() {
        return Line::from("");
    }

    let trimmed = text.trim_start();

    // Line-level constructs (must be at start of trimmed line)
    if let Some(content) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let bullet = Span::styled(" \u{2022} ", base_style.fg(Color::Rgb(255, 180, 100)));
        let mut spans = vec![bullet];
        spans.extend(parse_inline(content, base_style));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("> ") {
        let bar = Span::styled(" \u{2502} ", base_style.fg(Color::Rgb(100, 120, 140)));
        let quote_style = base_style
            .fg(Color::Rgb(140, 160, 180))
            .add_modifier(Modifier::DIM);
        let mut spans = vec![bar];
        spans.extend(parse_inline(content, quote_style));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("### ") {
        return Line::from(Span::styled(
            content.to_string(),
            base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }

    // Regular paragraph line
    Line::from(parse_inline(text, base_style))
}

/// Parse inline markdown tokens: `**bold**`, `*italic*`, `` `code` ``
pub(crate) fn parse_inline(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check for `code` first (backtick)
        if chars[i] == '`' {
            let mut content = String::new();
            i += 1;
            while i < len && chars[i] != '`' {
                content.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            spans.push(Span::styled(
                content,
                Style::default()
                    .fg(Color::Rgb(255, 200, 100))
                    .bg(Color::Rgb(50, 35, 15)),
            ));
            continue;
        }

        // Check for **bold**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            let mut content = String::new();
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                content.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            spans.push(Span::styled(
                content,
                base_style
                    .fg(Color::Rgb(255, 255, 255))
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }

        // Check for *italic*
        if chars[i] == '*' {
            let mut content = String::new();
            i += 1;
            while i < len && chars[i] != '*' {
                content.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1;
            }
            // Italic = warm yellow (more visible than italic modifier)
            spans.push(Span::styled(
                content,
                base_style.fg(Color::Rgb(255, 220, 120)),
            ));
            continue;
        }

        // Regular character
        let mut plain = String::new();
        while i < len && chars[i] != '*' && chars[i] != '`' {
            plain.push(chars[i]);
            i += 1;
        }
        if !plain.is_empty() {
            spans.push(Span::styled(plain, base_style));
        }
    }

    spans
}

/// Estimate how many visual rows a logical Line occupies when wrapped at
/// `content_width` columns.
pub(crate) fn visual_line_count(line: &Line, content_width: usize) -> usize {
    if line.spans.is_empty() || content_width == 0 {
        return 1;
    }

    let mut remaining = content_width;
    let mut rows: u32 = 1;

    for span in &line.spans {
        let w = span.width();
        if w <= remaining {
            // Span fits entirely on the current row.
            remaining -= w;
        } else if remaining == content_width {
            // Span starts on a fresh row but is wider than content_width.
            // First row of this span is already counted — add extra rows.
            let span_rows = w.div_ceil(content_width);
            rows += span_rows as u32 - 1;
            let rem = w % content_width;
            remaining = if rem == 0 {
                content_width
            } else {
                content_width - rem
            };
        } else {
            // Span doesn't fit on current (partially filled) row.
            // ratatui moves the entire span to the next row.
            // The current row remains "wasted".
            rows += w.div_ceil(content_width) as u32;
            let rem = w % content_width;
            remaining = if rem == 0 {
                content_width
            } else {
                content_width - rem
            };
        }
    }

    rows as usize
}

/// Walk backwards through `lines` accumulating visual row counts (accounting for
/// wrapping at `content_width` columns) and return the index of the first logical
/// line to display so that the bottommost content fits in `visible_rows`.
///
/// When `chat_scroll > 0` the user has manually scrolled up that many logical
/// lines from the auto-scroll position.
///
/// Returns `(start_index, max_scroll)` where `max_scroll` is the maximum useful
/// scroll value (the auto-scroll bottom position), used to clamp `chat_scroll`
/// and prevent k/j asymmetry.
pub(crate) fn select_visible_start(
    lines: &[Line],
    visible_rows: usize,
    content_width: usize,
    chat_scroll: u16,
) -> (u16, u16) {
    if lines.is_empty() {
        return (0, 0);
    }

    // Walk backwards, accumulating visual rows until we fill the visible area
    let mut remaining = visible_rows;
    let mut bottom: usize = 0;

    'walk: for (i, line) in lines.iter().enumerate().rev() {
        let visual = visual_line_count(line, content_width);

        if remaining == 0 {
            // Filled the visible area; start from the line AFTER this one
            bottom = (i + 1).min(lines.len() - 1);
            break 'walk;
        }

        if visual > remaining {
            // This line overflows but must be partially shown
            bottom = i;
            break 'walk;
        }

        remaining -= visual;
    }
    // If loop completes without break: all lines fit (bottom stays 0)

    // Clamp chat_scroll to the maximum useful value (bottom) to prevent
    // asymmetry between k (scroll up) and j (scroll down).
    let clamped_scroll = (chat_scroll as usize).min(bottom);

    // Apply manual scroll offset (if any)
    let scroll = bottom.saturating_sub(clamped_scroll);
    (scroll as u16, bottom as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

    fn count_used_rows(buf: &ratatui::buffer::Buffer, width: u16, visible: usize) -> usize {
        // Only inspect the content columns (between the left/right borders).
        let mut used = 0;
        for y in 1..=visible {
            let mut nonempty = false;
            for x in 1..(width - 1) {
                let cell = &buf.content()[(y as usize) * (width as usize) + (x as usize)];
                if cell.symbol() != " " && cell.symbol() != "" {
                    nonempty = true;
                    break;
                }
            }
            if nonempty {
                used += 1;
            }
        }
        used
    }

    /// Render a set of lines the same way render_chat does and return the
    /// number of rows actually occupied by non-empty content (excluding the
    /// border rows).
    fn actual_used_rows(lines: &[Line], content_width: usize, visible: usize) -> usize {
        let (start_idx, _max_scroll) = select_visible_start(lines, visible, content_width, 0);
        let display: Vec<Line> = lines.iter().skip(start_idx as usize).cloned().collect();
        let paragraph = Paragraph::new(display)
            .block(Block::default().borders(Borders::ALL))
            .scroll((0, 0))
            .wrap(Wrap { trim: false });
        let w = (content_width + 2) as u16;
        let h = (visible + 2) as u16;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(paragraph, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        count_used_rows(&buf, w, visible)
    }

    fn ai_line(text: &str) -> Line<'static> {
        let mut rendered = render_markdown_line(text, Style::default());
        rendered
            .spans
            .insert(0, Span::styled("▐ ", Style::default()));
        rendered
    }

    #[test]
    fn visual_line_count_matches_actual_wrap() {
        let text = "a".repeat(50);
        let line = ai_line(&text);
        let est = visual_line_count(&line, 20);
        let w = 22u16;
        let h = 10u16;
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let p = Paragraph::new(vec![line.clone()])
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        terminal
            .draw(|f| {
                let area = f.area();
                f.render_widget(p, area);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let actual = count_used_rows(&buf, w, 8);
        eprintln!("=== rendered buffer (width {}) ===", w);
        for y in 0..h {
            let mut row = String::new();
            for x in 0..w {
                row.push_str(buf.content()[(y as usize) * (w as usize) + (x as usize)].symbol());
            }
            eprintln!("{:02}|{}|", y, row);
        }
        assert_eq!(
            est, actual,
            "estimate {} != actual {} for width-52 line at content_width 20",
            est, actual
        );
    }

    #[test]
    fn no_blank_gap_when_filling_visible_area() {
        let content_width = 20;
        let visible = 10;
        let mut lines: Vec<Line> = Vec::new();
        for _ in 0..3 {
            lines.push(Line::from(Span::styled("▐", Style::default())));
            for _ in 0..3 {
                lines.push(ai_line("hello world this is a test line"));
            }
            lines.push(Line::from(Span::styled("▐", Style::default())));
        }
        for (i, seg) in ["first streaming line", "second streaming line"]
            .iter()
            .enumerate()
        {
            let prefix = if i == 0 { "\u{258c}" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("{}{}", prefix, seg),
                Style::default(),
            )));
        }

        let used = actual_used_rows(&lines, content_width, visible);
        assert!(
            used >= visible - 1,
            "blank gap: only {} of {} visible rows used",
            used,
            visible
        );
    }
}
