//! Markdown rendering helpers for the chat panel.
//!
//! These are pure functions that turn raw text lines into styled
//! [`Line`]s, and estimate how many visual rows a line occupies when
//! wrapped at a given width.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::tui::code_block::CodeBlockHighlighter;

/// Parse a line of text for inline markdown and return styled Spans.
/// Uses COLOR changes (not modifiers) for italic since color is
/// far more visible in terminals than font-weight/italic.
///
///   `**bold**`  -> bright white foreground
///   `*italic*`  -> warm yellow foreground
///   `` `code` `` -> amber on dark background
#[cfg(test)]
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

/// Same as [`render_markdown_line`] but uses syntect-based highlighting for inline
/// code (backtick spans) via the provided [`CodeBlockHighlighter`].
pub(crate) fn render_markdown_line_with_syntect(
    text: &str,
    base_style: Style,
    hl: &CodeBlockHighlighter,
    is_dark: bool,
) -> Line<'static> {
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
        spans.extend(parse_inline_with_syntect(content, base_style, hl, is_dark));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("> ") {
        let bar = Span::styled(" \u{2502} ", base_style.fg(Color::Rgb(100, 120, 140)));
        let quote_style = base_style
            .fg(Color::Rgb(140, 160, 180))
            .add_modifier(Modifier::DIM);
        let mut spans = vec![bar];
        spans.extend(parse_inline_with_syntect(content, quote_style, hl, is_dark));
        return Line::from(spans);
    }

    if let Some(content) = trimmed.strip_prefix("### ") {
        return Line::from(Span::styled(
            content.to_string(),
            base_style.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }

    // Regular paragraph line
    Line::from(parse_inline_with_syntect(text, base_style, hl, is_dark))
}

/// Parse inline markdown tokens with syntect-based highlighting for inline code.
///
/// Like [`parse_inline`] but passes backtick content through
/// [`CodeBlockHighlighter::highlight_inline`] for theme-aware coloring.
pub(crate) fn parse_inline_with_syntect(
    text: &str,
    base_style: Style,
    hl: &CodeBlockHighlighter,
    is_dark: bool,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check for `code` first (backtick) — use syntect for inline code
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
            let colored = hl.highlight_inline(&content, is_dark, base_style);
            spans.extend(colored);
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

/// Parse inline markdown tokens: `**bold**`, `*italic*`, `` `code` ``
#[cfg(test)]
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

/// Render a markdown table block (consecutive lines starting with `|`) into
/// styled lines using box-drawing characters.
///
/// The first non-separator row is treated as the header. The separator row
/// (`|---|---|`) is detected and skipped. All remaining rows are data rows.
///
/// Each output line is prefixed with `prefix` (e.g. `"▐ "` for committed
/// messages or `""` for streaming).
///
/// Cell content is parsed for inline markdown (`**bold**`, `*italic*`, `` `code` ``)
/// so that formatting renders inside table cells.
pub(crate) fn render_table_block(
    table_lines: &[&str],
    prefix: &str,
    border_style: Style,
    cell_style: Style,
) -> Vec<Line<'static>> {
    if table_lines.is_empty() {
        return vec![];
    }

    // Parse rows: split each line by `|`, trim cells.
    // Skip lines that don't look like table rows.
    struct TableRow {
        cells: Vec<String>,
        is_separator: bool,
    }

    let mut rows: Vec<TableRow> = Vec::new();

    for line in table_lines {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') || trimmed.len() < 2 {
            continue;
        }
        // Remove leading and trailing `|`
        let inner = &trimmed[1..trimmed.len() - 1];
        let cells: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();

        if cells.is_empty() {
            continue;
        }

        // Detect separator row: all cells contain only `-`, `:`, and spaces
        let is_separator = cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '));

        rows.push(TableRow {
            cells,
            is_separator,
        });
    }

    // Filter out separator rows for column-width calculation
    let data_rows: Vec<&TableRow> = rows.iter().filter(|r| !r.is_separator).collect();

    if data_rows.is_empty() {
        return vec![];
    }

    // Calculate column widths (based on plain-text length, ignoring markdown tokens)
    let num_cols = data_rows.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return vec![];
    }

    let mut col_widths: Vec<usize> = vec![0; num_cols];
    for row in &data_rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i < num_cols {
                // Use visual width (strip markdown tokens, then measure display width)
                let plain = strip_inline_markdown(cell);
                col_widths[i] = col_widths[i].max(plain.width());
            }
        }
    }

    // Clamp column widths to avoid insanely wide tables
    let max_col = 40usize;
    for w in &mut col_widths {
        *w = (*w).min(max_col);
    }

    let mut out: Vec<Line> = Vec::new();

    // Helper: build a horizontal separator line
    let build_sep = |left: &str, mid: &str, right: &str, h: &str| -> String {
        let mut s = String::from(prefix);
        s.push_str(left);
        for (i, w) in col_widths.iter().enumerate() {
            if i > 0 {
                s.push_str(mid);
            }
            // +2 for the mandatory space padding on each side
            s.push_str(&h.repeat(*w + 2));
        }
        s.push_str(right);
        s
    };

    // Helper: render a row of cells as a Line with styled spans.
    // Each cell's content is rendered as plain text (markdown tokens are
    // stripped for accurate width measurement). The display_text is
    // truncated to fit the column width when necessary.
    let render_row = |cells: &[String], style: Style| -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Prefix
        spans.push(Span::styled(prefix.to_string(), border_style));
        spans.push(Span::styled("│".to_string(), border_style));

        for (i, cell) in cells.iter().enumerate() {
            let w = col_widths.get(i).copied().unwrap_or(0);
            let plain = strip_inline_markdown(cell);
            let plain_width = plain.width();

            let display_text: String = if plain_width > w {
                // Truncate to visual width with ellipsis
                truncate_to_width(&plain, w.saturating_sub(1)) + "…"
            } else {
                plain
            };
            let display_width = display_text.width();

            // Space before cell content
            spans.push(Span::styled(" ".to_string(), border_style));

            // Render the (possibly truncated) display text
            spans.push(Span::styled(display_text, style));

            // Padding to fill remaining column width (based on visual width)
            let padding = w.saturating_sub(display_width);
            if padding > 0 {
                spans.push(Span::styled(" ".repeat(padding), style));
            }

            // Space and separator after cell
            spans.push(Span::styled(" │".to_string(), border_style));
        }

        Line::from(spans)
    };

    // Top border
    out.push(Line::from(Span::styled(
        build_sep("\u{250c}", "\u{252c}", "\u{2510}", "\u{2500}"),
        border_style,
    )));

    // Find the header row (first non-separator row)
    if let Some(header) = data_rows.first() {
        out.push(render_row(
            &header.cells,
            cell_style.add_modifier(Modifier::BOLD),
        ));
    }

    // Header/data separator
    out.push(Line::from(Span::styled(
        build_sep("\u{251c}", "\u{253c}", "\u{2524}", "\u{2500}"),
        border_style,
    )));

    // Data rows (skip header)
    for row in data_rows.iter().skip(1) {
        out.push(render_row(&row.cells, cell_style));
    }

    // Bottom border
    out.push(Line::from(Span::styled(
        build_sep("\u{2514}", "\u{2534}", "\u{2518}", "\u{2500}"),
        border_style,
    )));

    out
}

/// Strip inline markdown tokens from a string, returning only the visible text.
/// Used to calculate column widths without markdown syntax characters.
fn strip_inline_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip `code` backticks
        if chars[i] == '`' {
            i += 1;
            while i < len && chars[i] != '`' {
                result.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing backtick
            }
            continue;
        }

        // Skip **bold** markers
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '*') {
                result.push(chars[i]);
                i += 1;
            }
            if i + 1 < len {
                i += 2; // skip closing **
            }
            continue;
        }

        // Skip *italic* markers
        if chars[i] == '*' {
            i += 1;
            while i < len && chars[i] != '*' {
                result.push(chars[i]);
                i += 1;
            }
            if i < len {
                i += 1; // skip closing *
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Truncate a string so its visual width (as measured by `unicode-width`)
/// does not exceed `max_width`. Uses character-level granularity.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut out = String::with_capacity(s.len());
    let mut w = 0;
    for c in s.chars() {
        let cw = UnicodeWidthStr::width(c.to_string().as_str());
        if w + cw > max_width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
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

/// Result of [`select_visible_start`]: the first logical line to display and
/// the number of visual rows of that line that are hidden above the visible
/// area (0 when the line is fully visible or all lines fit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisibleStart {
    pub(crate) start_idx: u16,
    /// Visual rows of the first line that are ABOVE the visible area.
    /// Only non-zero when the first line is partially visible (wrapping
    /// overflow). The click handler adds this to `start_visual` so that
    /// `click_visual = start_visual + row` maps correctly.
    pub(crate) visual_offset: usize,
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
) -> VisibleStart {
    if lines.is_empty() {
        return VisibleStart {
            start_idx: 0,
            visual_offset: 0,
        };
    }

    // Walk backwards, accumulating visual rows until we fill the visible area
    let mut remaining = visible_rows;
    let mut bottom: usize = 0;
    let mut visual_offset: usize = 0;

    'walk: for (i, line) in lines.iter().enumerate().rev() {
        let visual = visual_line_count(line, content_width);

        if remaining == 0 {
            // Filled the visible area; start from the line AFTER this one
            bottom = (i + 1).min(lines.len() - 1);
            break 'walk;
        }

        if visual > remaining {
            // This line overflows but must be partially shown.
            // `remaining` visual rows of this line are visible at the top;
            // the rest (`visual - remaining`) are hidden above.
            bottom = i;
            visual_offset = visual - remaining;
            break 'walk;
        }

        remaining -= visual;
    }
    // If loop completes without break: all lines fit (bottom stays 0)

    // Apply manual scroll offset (if any)
    let scroll = bottom.saturating_sub(chat_scroll as usize);
    VisibleStart {
        start_idx: scroll as u16,
        visual_offset,
    }
}

/// Given a visual row offset from `start_idx`, return the corresponding logical
/// line index, accounting for soft-wrapping (where one logical line may occupy
/// This is the inverse of the visual-row accumulation in `select_visible_start`:
/// it walks forward from `start_idx` accumulating visual rows until reaching
/// `visual_offset`, then returns the logical line at that position.

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
        let start_idx = select_visible_start(lines, visible, content_width, 0).start_idx;
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
