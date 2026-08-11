# Chat Syntax Highlighting & Code Block UX

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add syntax highlighting to code blocks and inline code in the Chat using `syntect`, with expand/collapse and copy functionality for fenced blocks.

**Architecture:** `render_sectioned_block()` in `src/tui/render.rs` is the line-by-line pipeline that classifies messages into thinking/tool/normal sections. The normal section delegates to `render_markdown_line()` for inline markdown. We add a new `code_block` module that detects fenced ``` markers, tokenizes with syntect, and renders colored spans. Inline code uses the same syntect pipeline with plain-text fallback. State for expand/collapse lives in `App` as a `HashSet<usize>` tracking collapsed block IDs.

**Tech Stack:** `syntect 5.3` for syntax highlighting, `Paragraph` + `Span` for rendering (no TextArea — display-only)

## Global Constraints

- syntect 5.3 must be added to Cargo.toml as a direct dependency
- Theme mapping: `base16-ocean.dark` for dark mode, `InspiredGitHub` for light mode (default syntect themes)
- Supported languages: Rust, Python, Bash, Fish, YAML, JSON, JavaScript, TypeScript, Lua
- Fenced blocks are collapsed by default; inline code is always expanded
- Copy button: visual `[copy]` indicator on expanded blocks, triggerable via keybinding
- All new rendering is pure display — no TextArea editing
- Every syntect theme load must be done once, not per-frame
- MSRV stays 1.97 (already set)

---

### Task 1: Add syntect dependency and create code_block module

**Files:**
- Modify: `Cargo.toml`
- Create: `src/tui/code_block.rs`

**Interfaces:**
- Produces: `pub(crate) struct CodeBlockState { collapsed: HashSet<usize>, next_id: usize }`
- Produces: `pub(crate) struct CodeBlockHighlighter { ss: SyntaxSet, ts: ThemeSet }`
- Produces: `pub(crate) fn highlight_fenced_block(highlighter: &CodeBlockHighlighter, lang: &str, code: &str, theme: &str) -> Vec<Line<'static>>`
- Produces: `pub(crate) fn highlight_inline(highlighter: &CodeBlockHighlighter, code: &str, base_style: Style) -> Vec<Span<'static>>`

- [ ] **Step 1: Add syntect to Cargo.toml**

Insert after the existing dependencies:
```toml
# Syntax highlighting
syntect = "5.3"
```

- [ ] **Step 2: Run cargo check to verify dependency resolves**

Run: `cargo check`
Expected: syntect resolves and compiles

- [ ] **Step 3: Create `src/tui/code_block.rs`**

```rust
//! Fenced code block detection, syntax highlighting via syntect,
//! and expand/collapse state for code blocks in the chat.

use std::collections::HashSet;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{ThemeSet, FontStyle};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// State for expand/collapse of fenced code blocks.
#[derive(Default)]
pub(crate) struct CodeBlockState {
    /// IDs of currently collapsed blocks.
    pub(crate) collapsed: HashSet<usize>,
    /// Next block ID to assign.
    next_id: usize,
}

impl CodeBlockState {
    pub(crate) fn next_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn toggle(&mut self, id: usize) {
        if !self.collapsed.remove(&id) {
            self.collapsed.insert(id);
        }
    }

    pub(crate) fn is_collapsed(&self, id: usize) -> bool {
        self.collapsed.contains(&id)
    }
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
        if is_dark { "base16-ocean.dark" } else { "InspiredGitHub" }
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
    /// Returns the full block with a [copy] line appended.
    pub(crate) fn highlight_fenced_block(
        &self,
        lang: &str,
        code: &str,
        is_dark: bool,
        block_id: usize,
        is_collapsed: bool,
    ) -> Vec<Line<'static>> {
        if is_collapsed {
            return vec![Line::from(Span::styled(
                format!(" [+](/{} {} lines)", lang, code.lines().count()),
                Style::default().fg(Color::Rgb(100, 140, 180)),
            ))];
        }

        let theme_name = self.syntect_theme(is_dark);
        let theme = &self.ts.themes[theme_name];
        let syntax = self.ss
            .find_syntax_by_token(lang)
            .unwrap_or_else(|| self.ss.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut lines: Vec<Line> = Vec::new();

        // Determine the background color for the code block area
        let bg = Color::Rgb(
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).r,
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).g,
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).b,
        );

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter.highlight_line(line, &self.ss)
                .unwrap_or_else(|_| vec![]);
            let spans: Vec<Span> = ranges.iter().map(|(style, text)| {
                let s = Self::to_ratatui_style(style.foreground, style.font_style);
                Span::styled(text.to_string(), s.bg(bg))
            }).collect();
            lines.push(Line::from(spans));
        }

        // Append [copy] indicator line
        let copy_line = Line::from(vec![
            Span::styled(
                format!(" [copy #{}] ", block_id),
                Style::default()
                    .fg(Color::Rgb(120, 180, 255))
                    .add_modifier(Modifier::DIM),
            ),
            Span::styled(
                format!("({} {})", lang, code.lines().count()),
                Style::default()
                    .fg(Color::Rgb(100, 140, 180))
                    .add_modifier(Modifier::DIM),
            ),
        ]);
        lines.push(copy_line);

        lines
    }

    /// Highlight inline code with syntect (no language hint — plain text / best guess).
    pub(crate) fn highlight_inline(&self, code: &str, is_dark: bool, base_style: Style) -> Vec<Span<'static>> {
        if code.is_empty() {
            return vec![];
        }

        let theme_name = self.syntect_theme(is_dark);
        let theme = &self.ts.themes[theme_name];
        // Use plain text syntax for inline — just applies theme's default foreground
        let syntax = self.ss.find_syntax_plain_text();
        let bg = Color::Rgb(
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).r,
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).g,
            theme.settings.background.unwrap_or(syntect::highlighting::Color::BLACK).b,
        );
        // For plain text, we just color the whole span in the theme's default foreground
        let default_fg = theme.settings.foreground.unwrap_or(syntect::highlighting::Color::WHITE);
        let s = base_style
            .fg(Color::Rgb(default_fg.r, default_fg.g, default_fg.b))
            .bg(bg);
        vec![Span::styled(code.to_string(), s)]
    }
}
```

- [ ] **Step 4: Verify module compiles**

Run: `cargo check`
Expected: module compiles (may have unused warnings until wired in)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/tui/code_block.rs
git commit -m "feat(chat): add syntect dependency and code_block module"
```


### Task 2: Integrate CodeBlockState and CodeBlockHighlighter into App

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/render.rs`

**Interfaces:**
- Consumes: `CodeBlockState` and `CodeBlockHighlighter` from Task 1
- Produces: `app.code_block_state: CodeBlockState` and `app.code_block_hl: CodeBlockHighlighter` in App
- Produces: `code_block_hl` passed into render functions

- [ ] **Step 1: Add fields to App struct**

In `src/tui/app.rs`, add the import and new fields:

```rust
use crate::tui::code_block::{CodeBlockHighlighter, CodeBlockState};

pub(crate) struct App {
    // ... existing fields ...
    /// State for code block expand/collapse.
    pub(crate) code_block_state: CodeBlockState,
    /// Cached syntect highlighter (loaded once).
    pub(crate) code_block_hl: CodeBlockHighlighter,
}
```

Initialize in the constructor:
```rust
code_block_state: CodeBlockState::default(),
code_block_hl: CodeBlockHighlighter::default(),
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check`
Expected: compiles

- [ ] **Step 3: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(chat): add CodeBlockState and CodeBlockHighlighter to App"
```


### Task 3: Add code-block awareness to render_sectioned_block

**Files:**
- Modify: `src/tui/render.rs`

**Interfaces:**
- Consumes: `app.code_block_state`, `app.code_block_hl`, `app.theme` from Task 2
- Consumes: `CodeBlockHighlighter::highlight_fenced_block` from Task 1
- Modifies: `render_sectioned_block` to detect ``` markers

- [ ] **Step 1: Add import in render.rs**

```rust
use crate::tui::code_block::CodeBlockHighlighter;
```

- [ ] **Step 2: Track code block state in `render_sectioned_block`**

Pass two new parameters:
```rust
fn render_sectioned_block(
    content: &str,
    ts: &str,
    config: &SectionConfig,
    styles: &SectionStyles,
    table_cell_style: Style,
    normal_line_render: impl Fn(String, &'static str, Style) -> Line<'static>,
    code_block_hl: &CodeBlockHighlighter,
    code_block_state: &mut CodeBlockState,
    is_dark: bool,
) -> Vec<Line<'static>> {
```

Inside the loop (after the tool/thinking marker detection, before the normal line rendering), add:

```rust
// ── Fenced code block detection ──
if marker.starts_with("```") {
    // Extract language
    let lang = marker.trim_start_matches("```").trim();
    // Collect all lines until closing ``` or end
    let mut code_lines: Vec<String> = Vec::new();
    // Advance i (need to change from for loop to indexed or collect ahead)
    // ... (see implementation details below)
    continue;
}
```

The actual implementation requires restructuring the current `for line_text in content.split('\n')` loop to also handle multi-line code blocks. The approach:

```rust
// Before the main loop, we need an iterator we can advance manually
let mut lines_iter = content.split('\n').peekable();
let mut line_idx = 0;

while let Some(line_text) = lines_iter.next() {
    let marker = line_text.trim();

    // ... existing [thinking], [/thinking], tool detection ...

    // ── Fenced code block detection ──
    if marker.starts_with("```") {
        let lang = marker.trim_start_matches("```").trim();
        let block_id = code_block_state.next_id();
        let is_collapsed = code_block_state.is_collapsed(block_id);

        // Collect code content
        let mut code_content = String::new();
        while let Some(code_line) = lines_iter.next() {
            if code_line.trim() == "```" {
                break;
            }
            code_content.push_str(code_line);
            code_content.push('\n');
        }
        if code_content.ends_with('\n') {
            code_content.pop();
        }

        // Highlight and render
        let highlighted = code_block_hl.highlight_fenced_block(
            lang, &code_content, is_dark, block_id, is_collapsed,
        );

        // If not collapsed, emit top border
        if !is_collapsed {
            section_has_content = true;
        }

        // Render a togglable [+]/[-] line: clicking toggles collapse
        // For TUI, toggle via keyboard when focus is on the block
        // ...
        for hl_line in highlighted {
            // Prefix each line with the section prefix
            let p = prefix(first_normal);
            out.push(Line::from(vec![
                Span::styled(p, styles.border),
                Span::styled(" ", Style::default()),
                hl_line,
            ]));
        }

        section_has_content = true;
        line_idx += 1 + code_content.lines().count();
        continue;
    }

    // ... rest of existing loop body ...
    line_idx += 1;
}
```

Note: the existing loop uses `for line_text in content.split('\n')` — this needs to change to a `while let` loop with a peekable iterator since code blocks consume multiple lines. The implementation must preserve all existing [thinking], [/thinking], and tool detection logic.

- [ ] **Step 3: Update callers of render_sectioned_block**

Both call sites in `render.rs` (normal section rendering and streaming section rendering) need the new parameters:

```rust
render_sectioned_block(
    &combined, &ts, &config, &styles, base,
    |full_line, prefix, border| { /* existing lambda */ },
    &app.code_block_hl,
    &mut app.code_block_state,
    app.is_dark(),  // or equivalent
)
```

- [ ] **Step 4: Pass is_dark from App**

Add a helper on App to determine dark mode:
```rust
pub(crate) fn is_dark(&self) -> bool {
    // Use the theme's background color to determine dark/light
    self.theme.background_intensity() < 128  // or similar heuristics
}
```

Or simpler: directly pass a boolean based on the theme.

- [ ] **Step 5: Run cargo check and test**

Run: `cargo check && cargo test --lib tui`
Expected: compiles, tests pass

- [ ] **Step 6: Commit**

```bash
git add src/tui/render.rs
git commit -m "feat(chat): detect fenced code blocks in render_sectioned_block"
```


### Task 4: Add inline syntax highlighting to parse_inline

**Files:**
- Modify: `src/tui/markdown.rs`
- Modify: `src/tui/render.rs` (pass code_block_hl into markdown functions)

**Interfaces:**
- Consumes: `CodeBlockHighlighter::highlight_inline` from Task 1
- Modifies: `parse_inline` to accept an optional highlighter

- [ ] **Step 1: Add optional highlighter parameter to parse_inline**

```rust
pub(crate) fn parse_inline(
    text: &str,
    base_style: Style,
) -> Vec<Span<'static>> {
```

Add an overload or a parameter for syntect highlighter:

```rust
pub(crate) fn parse_inline_with_syntect(
    text: &str,
    base_style: Style,
    hl: &CodeBlockHighlighter,
    is_dark: bool,
) -> Vec<Span<'static>> {
```

In the backtick handling section:
```rust
if chars[i] == '`' {
    let mut content = String::new();
    i += 1;
    while i < len && chars[i] != '`' {
        content.push(chars[i]);
        i += 1;
    }
    if i < len { i += 1; }
    // Use syntect for inline code coloring
    let colored = hl.highlight_inline(&content, is_dark, base_style);
    spans.extend(colored);
    continue;
}
```

For the existing `render_markdown_line`, add a parallel function `render_markdown_line_with_syntect` that accepts the highlighter and passes it through to `parse_inline_with_syntect`.

- [ ] **Step 2: Update callers**

In `render.rs`, the `normal_line_render` closure in `flush_ai_batch` calls `render_markdown_line`. Update it to use `render_markdown_line_with_syntect` when the highlighter is available.

- [ ] **Step 3: Run cargo check and test**

Run: `cargo check && cargo test --lib tui`
Expected: compiles, tests pass

- [ ] **Step 4: Commit**

```bash
git add src/tui/markdown.rs src/tui/render.rs
git commit -m "feat(chat): add syntect-based inline code highlighting"
```


### Task 5: Wire expand/collapse and copy keybindings

**Files:**
- Modify: `src/tui/keys.rs`
- Modify: `src/tui/input.rs` (or a new handler for code block interactions)

**Interfaces:**
- Consumes: `CodeBlockState::toggle` from Task 1
- Produces: keyboard handler for toggling the current code block

- [ ] **Step 1: Add code_block_keybind action**

In `src/tui/keymap.rs`, add a new action:
```rust
/// Toggle expand/collapse of the current code block.
ToggleCodeBlock,
```

Bind default key:
```rust
km.bind(
    Action::ToggleCodeBlock,
    vec![key_event('e', true)],  // Ctrl+E
);
```

- [ ] **Step 2: Add copy action**

```rust
/// Copy the content of the current code block.
CopyCodeBlock,
```

Bind default key:
```rust
km.bind(
    Action::CopyCodeBlock,
    vec![KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)],  // Ctrl+C on block
);
```

- [ ] **Step 3: Handle in keys.rs**

When focus is in Chat and a code block is active, Ctrl+E toggles expand/collapse, Ctrl+C copies the block content to clipboard.

- [ ] **Step 4: Commit**

```bash
git add src/tui/keys.rs src/tui/input.rs src/tui/keymap.rs
git commit -m "feat(chat): add expand/collapse and copy keybindings for code blocks"
```


### Task 6: Tests

**Files:**
- Create: `src/tui/code_block_test.rs` or add `#[cfg(test)]` in `code_block.rs`

- [ ] **Step 1: Add unit tests for code_block module**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_block_state_starts_empty() {
        let state = CodeBlockState::default();
        assert!(state.collapsed.is_empty());
    }

    #[test]
    fn code_block_state_toggle() {
        let mut state = CodeBlockState::default();
        let id = state.next_id();
        assert!(!state.is_collapsed(id));
        state.toggle(id);
        assert!(state.is_collapsed(id));
        state.toggle(id);
        assert!(!state.is_collapsed(id));
    }

    #[test]
    fn highlight_fenced_block_rust() {
        let hl = CodeBlockHighlighter::default();
        let result = hl.highlight_fenced_block(
            "rust",
            "fn main() {\n    println!(\"hello\");\n}",
            true,  // dark mode
            0,
            false, // expanded
        );
        assert!(!result.is_empty());
        // Last line should be the [copy] indicator
        assert!(result.last().unwrap().to_string().contains("copy"));
    }

    #[test]
    fn highlight_fenced_block_collapsed() {
        let hl = CodeBlockHighlighter::default();
        let result = hl.highlight_fenced_block(
            "rust",
            "fn main() { }",
            true,
            0,
            true, // collapsed
        );
        assert_eq!(result.len(), 1);
        assert!(result[0].to_string().contains("[+]"));
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib tui`
Expected: all tests pass (existing 91 + new)

- [ ] **Step 3: Commit**

```bash
git add src/tui/code_block.rs
git commit -m "test(chat): add unit tests for code_block module"
```


### Task 7: Visual review and final integration

**Files:**
- Modify: `src/tui/app.rs` (final wiring)
- Modify: `src/tui/render.rs` (final wiring)

- [ ] **Step 1: Final check — ensure all callers pass the new parameters**

Verify that every call to `render_sectioned_block`, `render_markdown_line`, and `parse_inline` has been updated.

- [ ] **Step 2: Manual smoke test**

Run: `cargo run`
Expected: Chat renders with syntax-highlighted code blocks. Fenced blocks are collapsed by default, expand on Ctrl+E. Inline code is colored by syntect theme.

- [ ] **Step 3: Final clippy and test**

Run: `cargo clippy && cargo test`
Expected: clean

- [ ] **Step 4: Commit**

```bash
git commit -m "chore(chat): final integration of syntax highlighting"
```