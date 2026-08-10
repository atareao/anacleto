use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

/// The kind of a single line in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// A context line (unchanged).
    Context,
    /// An added line (`+`).
    Add,
    /// A removed line (`-`).
    Remove,
    /// A header line (`---`, `+++`, `@@`).
    Header,
}

/// A single line of a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// A hunk of a diff, delimited by `@@` headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// A single file entry in a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffEntry {
    pub path: String,
    pub status: String,
    pub hunks: Vec<DiffHunk>,
}

/// Parse a unified diff into structured entries.
///
/// Recognizes `---`/`+++` file headers, `@@` hunk headers, and `+`/`-`/` `
/// prefixed lines. Lines that don't match any pattern are treated as context.
pub fn parse_unified_diff(input: &str) -> Vec<DiffEntry> {
    let mut entries: Vec<DiffEntry> = Vec::new();
    let mut current: Option<DiffEntry> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let mut lines = input.lines().peekable();
    while let Some(line) = lines.next() {
        if let Some(rest) = line.strip_prefix("--- ") {
            // Start of a new file entry. The next line should be `+++`.
            let path = rest.trim_start_matches("a/").to_string();
            // Consume the `+++` line if present.
            if let Some(next) = lines.peek()
                && next.starts_with("+++ ") {
                    lines.next();
                }
            // Finalize any previous entry/hunk.
            if let Some(hunk) = current_hunk.take()
                && let Some(entry) = current.as_mut() {
                    entry.hunks.push(hunk);
                }
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(DiffEntry {
                path,
                status: "modified".to_string(),
                hunks: Vec::new(),
            });
            continue;
        }

        if let Some(rest) = line.strip_prefix("+++ ") {
            // `+++` without a preceding `---`; treat as a new file header.
            if current.is_none() {
                let path = rest.trim_start_matches("b/").to_string();
                current = Some(DiffEntry {
                    path,
                    status: "new".to_string(),
                    hunks: Vec::new(),
                });
            }
            continue;
        }

        if line.starts_with("@@") {
            // New hunk.
            if let Some(hunk) = current_hunk.take()
                && let Some(entry) = current.as_mut() {
                    entry.hunks.push(hunk);
                }
            current_hunk = Some(DiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }

        // A content line.
        let (kind, text) = if let Some(rest) = line.strip_prefix('+') {
            (DiffLineKind::Add, rest.to_string())
        } else if let Some(rest) = line.strip_prefix('-') {
            (DiffLineKind::Remove, rest.to_string())
        } else if let Some(rest) = line.strip_prefix(' ') {
            (DiffLineKind::Context, rest.to_string())
        } else {
            (DiffLineKind::Context, line.to_string())
        };

        if let Some(hunk) = current_hunk.as_mut() {
            hunk.lines.push(DiffLine { kind, text });
        } else {
            // Content outside any hunk: create an implicit hunk.
            let mut hunk = DiffHunk {
                header: String::new(),
                lines: Vec::new(),
            };
            hunk.lines.push(DiffLine { kind, text });
            if let Some(entry) = current.as_mut() {
                entry.hunks.push(hunk);
            }
        }
    }

    // Finalize the last hunk/entry.
    if let Some(hunk) = current_hunk.take()
        && let Some(entry) = current.as_mut() {
            entry.hunks.push(hunk);
        }
    if let Some(entry) = current.take() {
        entries.push(entry);
    }

    entries
}

/// A scrollable, colorized diff viewer overlay.
pub struct DiffViewer {
    pub visible: bool,
    pub entries: Vec<DiffEntry>,
    pub scroll: usize,
    pub title: String,
}

impl DiffViewer {
    /// Create an empty, hidden diff viewer.
    pub fn new() -> Self {
        Self {
            visible: false,
            entries: Vec::new(),
            scroll: 0,
            title: " Diff ".to_string(),
        }
    }

    /// Parse `text` as a unified diff and replace the current content.
    pub fn push_diff(&mut self, text: &str, title: &str) {
        self.entries = parse_unified_diff(text);
        self.title = title.to_string();
        self.scroll = 0;
    }

    /// Scroll the view up by `n` lines (toward the start).
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    /// Scroll the view down by `n` lines (toward the end).
    pub fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_add(n);
    }

    /// Total number of visual lines in the diff.
    fn total_lines(&self) -> usize {
        let mut total = 0;
        for entry in &self.entries {
            total += 1; // path line
            for hunk in &entry.hunks {
                if !hunk.header.is_empty() {
                    total += 1;
                }
                total += hunk.lines.len();
            }
        }
        total
    }

    /// Render the diff viewer as a full-area overlay.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let overlay = Clear;
        f.render_widget(overlay, area);

        let visible_rows = (area.height.saturating_sub(2)).max(1) as usize;

        // Build all lines.
        let mut lines: Vec<Line> = Vec::new();
        for entry in &self.entries {
            lines.push(Line::from(Span::styled(
                format!(" {}  {}", entry.status, entry.path),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            for hunk in &entry.hunks {
                if !hunk.header.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!(" {}", hunk.header),
                        Style::default().fg(Color::Cyan),
                    )));
                }
                for dl in &hunk.lines {
                    let (color, prefix) = match dl.kind {
                        DiffLineKind::Add => (Color::Green, "+"),
                        DiffLineKind::Remove => (Color::Red, "-"),
                        DiffLineKind::Context => (Color::White, " "),
                        DiffLineKind::Header => (Color::Cyan, " "),
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", prefix, dl.text),
                        Style::default().fg(color),
                    )));
                }
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                " (sin cambios) ",
                Style::default().fg(Color::DarkGray),
            )));
        }

        // Clamp scroll to the end.
        let max_scroll = lines.len().saturating_sub(visible_rows);
        let scroll = self.scroll.min(max_scroll);

        let display: Vec<Line> = lines
            .iter()
            .skip(scroll)
            .take(visible_rows)
            .cloned()
            .collect();

        let title = format!(" {} — {} ", self.title, self.entries.len());
        let paragraph = Paragraph::new(display)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title)
                    .style(Style::default().bg(Color::Rgb(15, 20, 25))),
            )
            .wrap(Wrap { trim: false })
            .scroll((0, 0));

        f.render_widget(paragraph, area);

        // Footer hint.
        let footer = format!(
            " ↑/↓ scroll  ·  {} líneas  ·  Esc/tecla para cerrar ",
            self.total_lines()
        );
        let footer_y = area.y + area.height.saturating_sub(1);
        let footer_area = Rect::new(area.x, footer_y, area.width, 1);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(Color::DarkGray),
            ))),
            footer_area,
        );
    }
}

impl Default for DiffViewer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_add_remove_context() {
        let input = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n     println!(\"ctx\");\n }\n";
        let entries = parse_unified_diff(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "foo.rs");
        assert_eq!(entries[0].hunks.len(), 1);
        let lines = &entries[0].hunks[0].lines;
        assert_eq!(lines[0].kind, DiffLineKind::Context);
        assert_eq!(lines[1].kind, DiffLineKind::Remove);
        assert_eq!(lines[2].kind, DiffLineKind::Add);
        assert_eq!(lines[3].kind, DiffLineKind::Context);
    }

    #[test]
    fn parses_multiple_hunks() {
        let input = "--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n a\n b\n@@ -10,1 +10,1 @@\n x\n y\n";
        let entries = parse_unified_diff(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].hunks.len(), 2);
    }

    #[test]
    fn parses_multiple_files() {
        let input = "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-c\n+d\n";
        let entries = parse_unified_diff(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "a.rs");
        assert_eq!(entries[1].path, "b.rs");
    }

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(parse_unified_diff("").is_empty());
    }

    #[test]
    fn push_diff_replaces_content() {
        let mut dv = DiffViewer::new();
        dv.push_diff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n", "test");
        assert_eq!(dv.entries.len(), 1);
        assert_eq!(dv.title, "test");
        assert_eq!(dv.scroll, 0);
    }
}
