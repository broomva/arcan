use crate::models::state::{AppState, ConnectionStatus};
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

/// Render the status bar showing session, branch, mode, and errors.
///
/// Displays connection indicator, session ID, branch, mode (busy/approval/idle),
/// and an optional error flash.
pub fn render(f: &mut Frame, area: Rect, state: &AppState, theme: &Theme) {
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

    let provider_str = state.provider.as_deref().unwrap_or("?");
    // Show first 8 chars of session ID to save space on narrow terminals
    let session_short = state
        .session_id
        .as_deref()
        .map(|s| if s.len() > 8 { &s[..8] } else { s })
        .unwrap_or("--");

    let mut spans = vec![
        Span::styled(format!(" {conn_dot} "), conn_style),
        Span::styled(provider_str.to_string(), theme.status_bar_bg),
        Span::styled(" \u{2502} ", theme.status_bar_bg), // │
        Span::styled(format!("\u{2387} {branch_str}"), theme.status_bar_bg), // ⎇
        Span::styled(" \u{2502} ", theme.status_bar_bg),
        Span::styled(mode_str.to_string(), theme.status_bar_bg),
        Span::styled(" \u{2502} ", theme.status_bar_bg),
        Span::styled(session_short.to_string(), theme.status_bar_bg),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::state::ErrorFlash;
    use crate::models::ui_block::ApprovalRequest;
    use crate::test_utils::render_to_string;
    use chrono::Utc;

    #[test]
    fn snapshot_connected_idle() {
        let mut state = AppState::new();
        state.connection_status = ConnectionStatus::Connected;
        state.session_id = Some("sess-001".to_string());
        let theme = Theme::new();

        let output = render_to_string(60, 1, |f, area| {
            render(f, area, &state, &theme);
        });
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_disconnected() {
        let mut state = AppState::new();
        state.connection_status = ConnectionStatus::Disconnected;
        state.session_id = Some("sess-002".to_string());
        let theme = Theme::new();

        let output = render_to_string(60, 1, |f, area| {
            render(f, area, &state, &theme);
        });
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_connecting() {
        let state = AppState::new();
        let theme = Theme::new();

        let output = render_to_string(60, 1, |f, area| {
            render(f, area, &state, &theme);
        });
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_connected_with_error() {
        let mut state = AppState::new();
        state.connection_status = ConnectionStatus::Connected;
        state.session_id = Some("sess-003".to_string());
        state.last_error = Some(ErrorFlash {
            message: "timeout".to_string(),
            timestamp: Utc::now(),
        });
        let theme = Theme::new();

        let output = render_to_string(80, 1, |f, area| {
            render(f, area, &state, &theme);
        });
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_approval_mode() {
        let mut state = AppState::new();
        state.connection_status = ConnectionStatus::Connected;
        state.session_id = Some("sess-004".to_string());
        state.pending_approval = Some(ApprovalRequest {
            approval_id: "ap-1".to_string(),
            call_id: "c-1".to_string(),
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({}),
            risk_level: "high".to_string(),
        });
        let theme = Theme::new();

        let output = render_to_string(60, 1, |f, area| {
            render(f, area, &state, &theme);
        });
        insta::assert_snapshot!(output);
    }
}
