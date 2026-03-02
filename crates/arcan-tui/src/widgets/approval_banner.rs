use crate::models::ui_block::ApprovalRequest;
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render an inline approval banner when a tool needs user authorization.
pub fn render(f: &mut Frame, area: Rect, approval: &ApprovalRequest, _theme: &Theme) {
    let risk_style = risk_color(&approval.risk_level);

    let args_preview =
        serde_json::to_string(&approval.arguments).unwrap_or_else(|_| "{}".to_string());
    let args_short = if args_preview.len() > 60 {
        format!("{}...", &args_preview[..57])
    } else {
        args_preview
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                " \u{26a0} Approval Required ".to_string(), // ⚠
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format!("Risk: {}", approval.risk_level), risk_style),
        ]),
        Line::from(vec![
            Span::styled("Tool: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&approval.tool_name),
            Span::raw("  "),
            Span::styled("ID: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(&approval.approval_id),
        ]),
        Line::from(vec![
            Span::styled("Args: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(args_short),
        ]),
        Line::from(vec![Span::styled(
            " /approve <id> yes | /approve <id> no ",
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let banner = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(risk_style)
                .title(" Pending Approval "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(banner, area);
}

fn risk_color(level: &str) -> Style {
    match level.to_ascii_lowercase().as_str() {
        "critical" => Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
        "high" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "medium" => Style::default().fg(Color::Yellow),
        "low" => Style::default().fg(Color::Green),
        _ => Style::default().fg(Color::Gray),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_color_critical_has_red_bg() {
        let style = risk_color("critical");
        assert_eq!(style.bg, Some(Color::Red));
    }

    #[test]
    fn risk_color_is_case_insensitive() {
        let high = risk_color("High");
        let high_lower = risk_color("high");
        assert_eq!(high.fg, high_lower.fg);
    }

    #[test]
    fn risk_color_unknown_is_gray() {
        let style = risk_color("unknown-level");
        assert_eq!(style.fg, Some(Color::Gray));
    }
}
