use crate::models::state::{AppState, ConnectionStatus};
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

/// Render the status bar showing session, branch, mode, and errors.
pub fn render(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
    let session_str = state.session_id.as_deref().unwrap_or("no session");
    let branch_str = &state.current_branch;

    let (conn_dot, conn_style) = match state.connection_status {
        ConnectionStatus::Connected => ("\u{25cf}", theme.status_connected), // ●
        ConnectionStatus::Disconnected => ("\u{25cb}", theme.status_disconnected), // ○
        ConnectionStatus::Connecting => ("\u{25cc}", theme.status_connecting), // ◌
    };

    let mode_str = if state.is_busy {
        "busy"
    } else if state.pending_approval.is_some() {
        "approval"
    } else {
        "idle"
    };

    let mut spans = vec![
        Span::styled(format!(" {conn_dot} "), conn_style),
        Span::styled(session_str.to_string(), theme.status_bar_bg),
        Span::styled(" \u{2502} ", theme.status_bar_bg), // │
        Span::styled(format!("\u{2387} {branch_str}"), theme.status_bar_bg), // ⎇
        Span::styled(" \u{2502} ", theme.status_bar_bg),
        Span::styled(mode_str, theme.status_bar_bg),
    ];

    // Error flash
    if let Some(ref flash) = state.last_error {
        spans.push(Span::styled(" \u{2502} ", theme.status_bar_bg));
        spans.push(Span::styled(
            format!("\u{26a0} {}", flash.message), // ⚠
            theme.error_flash,
        ));
    }

    let status_line = Paragraph::new(Line::from(spans)).style(theme.status_bar_bg);
    f.render_widget(status_line, area);
}
