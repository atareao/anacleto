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
}
