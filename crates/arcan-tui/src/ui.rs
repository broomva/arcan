use crate::focus::FocusTarget;
use crate::models::state::AppState;
use crate::theme::Theme;
use crate::widgets;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

/// Top-level draw function. Orchestrates the three-chunk layout:
/// chat log, status bar, and input box.
pub fn draw(f: &mut Frame, state: &mut AppState) {
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

    // Input area
    render_input(f, chunks[2], state, &theme);
}

fn render_input(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let prompt = if let Some(approval) = &state.pending_approval {
        format!(
            "Approval Req for {}: (yes/no) > {}",
            approval.tool_name, state.input_buffer
        )
    } else {
        format!("\u{276f} {}", state.input_buffer) // ❯
    };

    let style = if state.pending_approval.is_some() {
        theme.input_approval
    } else {
        theme.input_normal
    };

    let border_style = if state.focus == FocusTarget::InputBar {
        theme.title
    } else {
        theme.border
    };

    let input_block = Paragraph::new(Line::from(prompt))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Input "),
        )
        .style(style);
    f.render_widget(input_block, area);
}
