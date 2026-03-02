use crate::models::state::AppState;
use crate::theme::Theme;
use crate::widgets;
use crate::widgets::input_bar::InputBarState;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

/// Top-level draw function. Orchestrates the three-chunk layout:
/// chat log, status bar, and input box.
pub fn draw(f: &mut Frame, state: &mut AppState, input_bar: &InputBarState) {
    let theme = Theme::new();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(3),    // Chat log
            Constraint::Length(1), // Status bar
            Constraint::Length(3), // Input box
        ])
        .split(f.area());

    // Chat log (scrollable)
    widgets::chat_log::render(f, chunks[0], state, &theme);

    // Status bar
    widgets::status_bar::render(f, chunks[1], state, &theme);

    // Input area (tui-textarea)
    let has_approval = state.pending_approval.is_some();
    widgets::input_bar::render(f, chunks[2], input_bar, state.focus, has_approval, &theme);
}
