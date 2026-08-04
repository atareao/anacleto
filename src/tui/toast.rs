use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// The severity/kind of a toast notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Neutral informational message.
    Info,
    /// Successful operation.
    Success,
    /// Error / failure.
    Error,
}

/// A single transient notification.
#[derive(Debug, Clone)]
pub struct Toast {
    /// The message body.
    pub message: String,
    /// The kind (drives color).
    pub kind: ToastKind,
    /// When the toast was created (for TTL expiry).
    pub created_at: Instant,
}

/// A queue of transient toasts rendered in the bottom-right corner.
pub struct ToastQueue {
    toasts: VecDeque<Toast>,
    ttl: Duration,
}

impl ToastQueue {
    /// Create an empty toast queue with the given time-to-live.
    pub fn new(ttl: Duration) -> Self {
        Self {
            toasts: VecDeque::new(),
            ttl,
        }
    }

    /// Push a new toast onto the queue (capped at a small size).
    pub fn push(&mut self, message: impl Into<String>, kind: ToastKind) {
        self.toasts.push_back(Toast {
            message: message.into(),
            kind,
            created_at: Instant::now(),
        });
        // Keep the queue small so toasts never flood the screen.
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
    }

    /// Expire toasts older than the TTL.
    pub fn tick(&mut self, now: Instant) {
        while let Some(t) = self.toasts.front() {
            if now.duration_since(t.created_at) > self.ttl {
                self.toasts.pop_front();
            } else {
                break;
            }
        }
    }

    /// Whether there are no active toasts.
    pub fn is_empty(&self) -> bool {
        self.toasts.is_empty()
    }

    /// Render all active toasts stacked in the bottom-right corner of `area`.
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if self.toasts.is_empty() {
            return;
        }

        let width = area.width.min(60);
        let mut y = area.y + area.height;

        // Newest toast at the bottom; iterate in reverse so the oldest is on top.
        for toast in self.toasts.iter().rev() {
            let (color, label) = match toast.kind {
                ToastKind::Info => (Color::Cyan, " INFO "),
                ToastKind::Success => (Color::Green, " OK "),
                ToastKind::Error => (Color::Red, " ERR "),
            };

            let height = 3u16;
            y = y.saturating_sub(height);
            let toast_area = Rect::new(area.x + area.width.saturating_sub(width), y, width, height);

            let overlay = Clear;
            f.render_widget(overlay, toast_area);

            let lines = vec![
                Line::from(Span::styled(
                    label,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    &toast.message,
                    Style::default().fg(Color::White),
                )),
            ];

            let paragraph = Paragraph::new(lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(color))
                        .style(Style::default().bg(Color::Rgb(25, 25, 35))),
                )
                .wrap(Wrap { trim: true });

            f.render_widget(paragraph, toast_area);
        }
    }
}

impl Default for ToastQueue {
    fn default() -> Self {
        Self::new(Duration::from_secs(4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_expire() {
        let mut q = ToastQueue::new(Duration::from_millis(10));
        q.push("hello", ToastKind::Info);
        assert!(!q.is_empty());
        q.tick(Instant::now() + Duration::from_millis(100));
        assert!(q.is_empty());
    }

    #[test]
    fn queue_is_capped() {
        let mut q = ToastQueue::new(Duration::from_secs(60));
        for i in 0..10 {
            q.push(format!("msg {}", i), ToastKind::Info);
        }
        assert_eq!(q.toasts.len(), 5);
    }
}
