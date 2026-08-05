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
    let w = line.width();
    if w == 0 || content_width == 0 {
        1
    } else {
        w.div_ceil(content_width)
    }
}

/// Walk backwards through `lines` accumulating visual row counts (accounting for
/// wrapping at `content_width` columns) and return the index of the first logical
/// line to display so that the bottommost content fits in `visible_rows`.
///
/// When `chat_scroll > 0` the user has manually scrolled up that many logical
/// lines from the auto-scroll position.
pub(crate) fn select_visible_start(
    lines: &[Line],
    visible_rows: usize,
    content_width: usize,
    chat_scroll: u16,
) -> u16 {
    if lines.is_empty() {
        return 0;
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

    // Apply manual scroll offset (if any)
    let scroll = bottom.saturating_sub(chat_scroll as usize);
    scroll as u16
}
