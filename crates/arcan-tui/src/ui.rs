use crate::models::state::AppState;
use crate::theme::Theme;
use crate::widgets;
use crate::widgets::input_bar::InputBarState;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

/// Top-level draw function. Orchestrates the layout:
/// - Chat log (scrollable, fills remaining space)
/// - Approval banner (shown only when pending, 6 lines)
/// - Status bar (1 line)
/// - Input box (3 lines)
pub fn draw(f: &mut Frame, state: &mut AppState, input_bar: &InputBarState) {
    let theme = Theme::new();

    let has_approval = state.pending_approval.is_some();

    let mut constraints = vec![Constraint::Min(3)]; // Chat log
    if has_approval {
        constraints.push(Constraint::Length(6)); // Approval banner
    }
    constraints.push(Constraint::Length(1)); // Status bar
    constraints.push(Constraint::Length(3)); // Input box

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(constraints)
        .split(f.area());

    let mut idx = 0;

    // Chat log (scrollable)
    widgets::chat_log::render(f, chunks[idx], state, &theme);
    idx += 1;

    // Approval banner (conditional)
    if let Some(ref approval) = state.pending_approval {
        widgets::approval_banner::render(f, chunks[idx], approval, &theme);
        idx += 1;
    }

    // Status bar
    widgets::status_bar::render(f, chunks[idx], state, &theme);
    idx += 1;

    // Input area
    widgets::input_bar::render(f, chunks[idx], input_bar, state.focus, has_approval, &theme);
}
