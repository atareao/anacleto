//! Fenced code block detection, syntax highlighting via syntect,
//! and copy support for code blocks in the chat.
//! Code blocks are always fully visible (no collapse/expand).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// Position of a rendered code block in the chat, recorded during render
/// so mouse clicks on `[copy]` can be matched back to the right block.
#[derive(Clone)]
pub(crate) struct CodeBlockPosition {
    pub(crate) lang: String,
    pub(crate) code: String,
    /// Absolute index of the `[copy]` line within the full rendered chat lines.
    pub(crate) copy_line: usize,
}

/// Cached syntect syntax set and theme set.
pub(crate) struct CodeBlockHighlighter {
    ss: SyntaxSet,
    ts: ThemeSet,
}

impl Default for CodeBlockHighlighter {
    fn default() -> Self {
        Self {
            ss: SyntaxSet::load_defaults_newlines(),
            ts: ThemeSet::load_defaults(),
        }
    }
}

impl CodeBlockHighlighter {
    /// Map anacleto theme name to syntect theme name.
    fn syntect_theme(&self, is_dark: bool) -> &str {
        if is_dark {
            "base16-ocean.dark"
        } else {
            "InspiredGitHub"
        }
    }

    /// Convert a syntect FontStyle + Color to a ratatui Style.
    fn to_ratatui_style(fg: syntect::highlighting::Color, font_style: FontStyle) -> Style {
        let mut style = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
        if font_style.contains(FontStyle::BOLD) {
            style = style.add_modifier(Modifier::BOLD);
        }
        if font_style.contains(FontStyle::ITALIC) {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if font_style.contains(FontStyle::UNDERLINE) {
            style = style.add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    /// Highlight a fenced code block (```lang ... ```) into styled lines.
    /// Always renders the full block with a [copy] line appended.
    pub(crate) fn highlight_fenced_block(
        &self,
        lang: &str,
        code: &str,
        is_dark: bool,
    ) -> Vec<Line<'static>> {
        let theme_name = self.syntect_theme(is_dark);
        let theme = &self.ts.themes[theme_name];
        let syntax = self
            .ss
            .find_syntax_by_token(lang)
            .unwrap_or_else(|| self.ss.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut lines: Vec<Line> = Vec::new();

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter
                .highlight_line(line, &self.ss)
                .unwrap_or_else(|_| vec![]);
            let spans: Vec<Span> = ranges
                .iter()
                .map(|(style, text)| {
                    let s = Self::to_ratatui_style(style.foreground, style.font_style);
                    Span::styled(text.to_string(), s)
                })
                .collect();
            lines.push(Line::from(spans));
        }

        // Append [copy] indicator line
        lines.push(Line::from(vec![
            Span::styled(
                " [copy] ",
                Style::default()
                    .fg(Color::Rgb(120, 180, 255))
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(
                format!("({}, {} líneas)", lang, code.lines().count()),
                Style::default()
                    .fg(Color::Rgb(100, 140, 180))
                    .add_modifier(Modifier::DIM),
            ),
        ]));

        lines
    }

    /// Highlight inline code with syntect (no language hint — plain text / best guess).
    pub(crate) fn highlight_inline(
        &self,
        code: &str,
        is_dark: bool,
        base_style: Style,
    ) -> Vec<Span<'static>> {
        if code.is_empty() {
            return vec![];
        }

        let theme_name = self.syntect_theme(is_dark);
        let theme = &self.ts.themes[theme_name];
        // Use plain text syntax for inline — just applies theme's default foreground
        let _syntax = self.ss.find_syntax_plain_text();
        // For plain text, we just color the whole span in the theme's default foreground
        let default_fg = theme
            .settings
            .foreground
            .unwrap_or(syntect::highlighting::Color::WHITE);
        let s = base_style.fg(Color::Rgb(default_fg.r, default_fg.g, default_fg.b));
        vec![Span::styled(code.to_string(), s)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_fenced_block_rust() {
        let hl = CodeBlockHighlighter::default();
        let result = hl.highlight_fenced_block(
            "rust",
            "fn main() { }",
            true, // dark mode
        );
        assert!(!result.is_empty());
        // Last line should be the [copy] indicator
        assert!(result.last().unwrap().to_string().contains("copy"));
    }

    #[test]
    fn highlight_fenced_block_always_full() {
        let hl = CodeBlockHighlighter::default();
        let result = hl.highlight_fenced_block("rust", "fn main() { }", true);
        // Should have more than 1 line (code + [copy])
        assert!(result.len() > 1);
        // Should NOT contain [+] (no collapse)
        assert!(!result[0].to_string().contains("[+]"));
    }

    #[test]
    fn highlight_inline_empty() {
        let hl = CodeBlockHighlighter::default();
        let spans = hl.highlight_inline("", true, Style::default());
        assert!(spans.is_empty());
    }

    #[test]
    fn highlight_inline_text() {
        let hl = CodeBlockHighlighter::default();
        let spans = hl.highlight_inline("HashMap::new()", true, Style::default());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "HashMap::new()");
    }
}
