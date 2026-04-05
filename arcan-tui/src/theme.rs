use ratatui::style::{Color, Modifier, Style};

/// Centralized theme for the TUI.
/// All widget renderers should use this instead of hardcoding colors.
pub struct Theme {
    pub human_label: Style,
    pub assistant_label: Style,
    pub system_label: Style,
    pub tool_label: Style,
    pub tool_success: Style,
    pub tool_error: Style,
    pub streaming_cursor: Style,
    pub input_normal: Style,
    pub input_approval: Style,
    pub border: Style,
    pub title: Style,
    pub status_bar_bg: Style,
    pub status_connected: Style,
    pub status_disconnected: Style,
    pub status_connecting: Style,
    pub error_flash: Style,
    pub timestamp: Style,
    pub spinner: Style,
    pub autocomplete_selected: Style,
    pub autocomplete_normal: Style,
    pub autocomplete_description: Style,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            human_label: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            assistant_label: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            system_label: Style::default().fg(Color::Red),
            tool_label: Style::default().fg(Color::Yellow),
            tool_success: Style::default().fg(Color::Green),
            tool_error: Style::default().fg(Color::Red),
            streaming_cursor: Style::default().fg(Color::Gray),
            input_normal: Style::default().fg(Color::White),
            input_approval: Style::default().fg(Color::Red),
            border: Style::default().fg(Color::DarkGray),
            title: Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            status_bar_bg: Style::default().fg(Color::White).bg(Color::DarkGray),
            status_connected: Style::default().fg(Color::Green),
            status_disconnected: Style::default().fg(Color::Red),
            status_connecting: Style::default().fg(Color::Yellow),
            error_flash: Style::default().fg(Color::White).bg(Color::Red),
            timestamp: Style::default().fg(Color::DarkGray),
            spinner: Style::default().fg(Color::Cyan),
            autocomplete_selected: Style::default().fg(Color::Black).bg(Color::Cyan),
            autocomplete_normal: Style::default().fg(Color::White),
            autocomplete_description: Style::default().fg(Color::DarkGray),
        }
    }
}

impl Theme {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_default_has_distinct_label_colors() {
        let t = Theme::new();
        assert_ne!(t.human_label.fg, t.assistant_label.fg);
        assert_ne!(t.system_label.fg, t.tool_label.fg);
    }
}
