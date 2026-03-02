use crate::app::App;
use crate::focus::FocusTarget;
use crate::theme::Theme;
use crate::widgets;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
};

/// Top-level draw function. Orchestrates the layout:
///
/// **Normal mode** (no panels):
/// - Chat log (scrollable, fills remaining space)
/// - Approval banner (shown only when pending, 6 lines)
/// - Status bar (1 line)
/// - Input box (3 lines)
///
/// **Panel mode** (`/sessions` or `/state`):
/// - Left 70%: normal chat layout
/// - Right 30%: session browser (top) + state inspector (bottom)
pub fn draw(f: &mut Frame, app: &mut App) {
    let theme = Theme::new();

    if app.show_panels {
        draw_with_panels(f, app, &theme);
    } else {
        draw_main(f, f.area(), app, &theme);
    }
}

/// Draw the main chat area within the given rect.
fn draw_main(f: &mut Frame, area: ratatui::layout::Rect, app: &mut App, theme: &Theme) {
    let has_approval = app.state.pending_approval.is_some();

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
        .split(area);

    let mut idx = 0;

    // Chat log (scrollable)
    widgets::chat_log::render(f, chunks[idx], &mut app.state, theme);
    idx += 1;

    // Approval banner (conditional)
    if let Some(ref approval) = app.state.pending_approval {
        widgets::approval_banner::render(f, chunks[idx], approval, theme);
        idx += 1;
    }

    // Status bar
    widgets::status_bar::render(f, chunks[idx], &app.state, theme);
    idx += 1;

    // Input area
    widgets::input_bar::render(
        f,
        chunks[idx],
        &app.input_bar,
        app.state.focus,
        has_approval,
        theme,
    );
}

/// Draw with side panels (session browser + state inspector).
fn draw_with_panels(f: &mut Frame, app: &mut App, theme: &Theme) {
    // Split horizontally: 70% main, 30% panels
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(f.area());

    // Left: main chat area
    draw_main(f, h_chunks[0], app, theme);

    // Right: split vertically — session browser (top) + state inspector (bottom)
    let panel_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(h_chunks[1]);

    widgets::session_browser::render(
        f,
        panel_chunks[0],
        &mut app.session_browser,
        app.state.focus == FocusTarget::SessionBrowser,
        theme,
    );

    widgets::state_inspector::render(
        f,
        panel_chunks[1],
        &app.state_inspector,
        app.state.focus == FocusTarget::StateInspector,
        theme,
    );
}
