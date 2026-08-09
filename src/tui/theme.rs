//! Color themes selectable via `/themes`.

use ratatui::style::Color;

/// Color themes selectable via `/themes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Theme {
    Default,
    Nord,
    Dracula,
    Solarized,
}

impl Theme {
    /// Human-readable name of the theme.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Theme::Default => "default",
            Theme::Nord => "nord",
            Theme::Dracula => "dracula",
            Theme::Solarized => "solarized",
        }
    }

    /// The next theme in the cycle (used by `/themes`).
    pub(crate) fn next(&self) -> Theme {
        match self {
            Theme::Default => Theme::Nord,
            Theme::Nord => Theme::Dracula,
            Theme::Dracula => Theme::Solarized,
            Theme::Solarized => Theme::Default,
        }
    }

    /// Accent color used in the status bar and chat border.
    pub(crate) fn accent(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(255, 107, 107),
            Theme::Nord => Color::Rgb(136, 192, 208),
            Theme::Dracula => Color::Rgb(255, 121, 198),
            Theme::Solarized => Color::Rgb(38, 139, 210),
        }
    }

    /// Border color (▐) for AI response blocks.
    pub(crate) fn ai_border(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(230, 190, 60),
            Theme::Nord => Color::Rgb(235, 203, 139),
            Theme::Dracula => Color::Rgb(241, 250, 140),
            Theme::Solarized => Color::Rgb(181, 137, 0),
        }
    }

    /// Border color (▐) for tool lines within AI responses.
    /// Distinct from `ai_border` so tool output is visually distinguishable.
    pub(crate) fn tool_border(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(80, 140, 180),
            Theme::Nord => Color::Rgb(94, 129, 172),
            Theme::Dracula => Color::Rgb(98, 114, 164),
            Theme::Solarized => Color::Rgb(88, 110, 117),
        }
    }

    /// Color for tool execution text (🔧).
    /// Currently matches `tool_border` but is independently themeable.
    pub(crate) fn tool_text(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(80, 140, 180),
            Theme::Nord => Color::Rgb(94, 129, 172),
            Theme::Dracula => Color::Rgb(98, 114, 164),
            Theme::Solarized => Color::Rgb(88, 110, 117),
        }
    }

    /// Lighter variant of `tool_text` for finalized/committed messages.
    pub(crate) fn tool_text_dim(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(140, 180, 210),
            Theme::Nord => Color::Rgb(145, 170, 205),
            Theme::Dracula => Color::Rgb(150, 160, 200),
            Theme::Solarized => Color::Rgb(135, 155, 160),
        }
    }

    /// Color for tool success markers (✅).
    pub(crate) fn tool_ok(&self) -> Color {
        match self {
            Theme::Default => Color::LightGreen,
            Theme::Nord => Color::Rgb(163, 190, 140),
            Theme::Dracula => Color::Rgb(80, 250, 123),
            Theme::Solarized => Color::Rgb(133, 153, 0),
        }
    }

    /// Dimmed variant of `tool_ok` for finalized (non-streaming) messages.
    pub(crate) fn tool_ok_dim(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(90, 160, 90),
            Theme::Nord => Color::Rgb(110, 140, 90),
            Theme::Dracula => Color::Rgb(50, 180, 80),
            Theme::Solarized => Color::Rgb(90, 105, 0),
        }
    }

    /// Color for tool failure markers (❌).
    pub(crate) fn tool_err(&self) -> Color {
        match self {
            Theme::Default => Color::LightRed,
            Theme::Nord => Color::Rgb(191, 97, 106),
            Theme::Dracula => Color::Rgb(255, 85, 85),
            Theme::Solarized => Color::Rgb(220, 50, 47),
        }
    }

    /// Dimmed variant of `tool_err` for finalized (non-streaming) messages.
    pub(crate) fn tool_err_dim(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(180, 90, 90),
            Theme::Nord => Color::Rgb(140, 60, 70),
            Theme::Dracula => Color::Rgb(180, 50, 50),
            Theme::Solarized => Color::Rgb(155, 30, 30),
        }
    }

    /// Color for thinking/reasoning text.
    pub(crate) fn thinking(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(180, 180, 180),
            Theme::Nord => Color::Rgb(180, 185, 195),
            Theme::Dracula => Color::Rgb(200, 200, 210),
            Theme::Solarized => Color::Rgb(170, 170, 170),
        }
    }

    /// Dimmed variant of `thinking` for borders and finalized messages.
    pub(crate) fn thinking_dim(&self) -> Color {
        match self {
            Theme::Default => Color::Rgb(120, 120, 120),
            Theme::Nord => Color::Rgb(130, 135, 145),
            Theme::Dracula => Color::Rgb(140, 140, 150),
            Theme::Solarized => Color::Rgb(110, 110, 110),
        }
    }
}
