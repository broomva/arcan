use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Snapshot of the agent state from the daemon's `/state` endpoint.
#[derive(Debug, Clone, Default)]
pub struct AgentStateSnapshot {
    pub session_id: String,
    pub branch: String,
    pub mode: String,
    pub progress: f32,
    pub uncertainty: f32,
    pub risk_level: String,
    pub error_streak: u32,
    pub context_pressure: f32,
    pub side_effect_pressure: f32,
    pub human_dependency: f32,
    pub tokens_remaining: u64,
    pub time_remaining_ms: u64,
    pub cost_remaining_usd: f64,
    pub tool_calls_remaining: u32,
    pub error_budget_remaining: u32,
    pub version: u64,
}

/// State for the inspector panel.
#[derive(Debug, Default)]
pub struct StateInspectorState {
    pub snapshot: Option<AgentStateSnapshot>,
    pub loading: bool,
    pub error: Option<String>,
}

impl StateInspectorState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_snapshot(&mut self, snapshot: AgentStateSnapshot) {
        self.snapshot = Some(snapshot);
        self.loading = false;
        self.error = None;
    }

    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn set_error(&mut self, msg: String) {
        self.loading = false;
        self.error = Some(msg);
    }
}

/// Render the state inspector panel.
pub fn render(
    f: &mut Frame,
    area: Rect,
    state: &StateInspectorState,
    focused: bool,
    theme: &Theme,
) {
    let border_style = if focused { theme.title } else { theme.border };

    let lines = if state.loading {
        vec![Line::from(Span::styled(
            "Loading state...",
            theme.timestamp,
        ))]
    } else if let Some(ref err) = state.error {
        vec![Line::from(Span::styled(
            format!("Error: {err}"),
            theme.tool_error,
        ))]
    } else if let Some(ref snap) = state.snapshot {
        build_state_lines(snap, theme)
    } else {
        vec![Line::from(Span::styled(
            "No state loaded. Use /state to fetch.",
            theme.timestamp,
        ))]
    };

    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Agent State "),
        )
        .wrap(Wrap { trim: false });

    f.render_widget(widget, area);
}

fn build_state_lines<'a>(snap: &'a AgentStateSnapshot, theme: &'a Theme) -> Vec<Line<'a>> {
    let bold = theme.title;
    let val = theme.assistant_label;
    let dim = theme.timestamp;

    let mode_style = match snap.mode.as_str() {
        "Explore" => theme.assistant_label,
        "Execute" => theme.human_label,
        "Verify" => theme.tool_label,
        "Recover" | "AskHuman" => theme.tool_error,
        "Sleep" => theme.timestamp,
        _ => theme.timestamp,
    };

    let risk_style = match snap.risk_level.as_str() {
        "Critical" => theme.tool_error.add_modifier(Modifier::BOLD),
        "High" => theme.tool_error,
        "Medium" => theme.tool_label,
        "Low" => theme.tool_success,
        _ => dim,
    };

    vec![
        Line::from(vec![
            Span::styled("Mode: ", bold),
            Span::styled(&snap.mode, mode_style),
            Span::styled("  Branch: ", bold),
            Span::styled(&snap.branch, val),
            Span::styled(format!("  v{}", snap.version), dim),
        ]),
        Line::from(vec![
            Span::styled("Progress: ", bold),
            Span::styled(format_bar(snap.progress), val),
            Span::styled(format!(" {:.0}%", snap.progress * 100.0), dim),
            Span::styled("  Risk: ", bold),
            Span::styled(&snap.risk_level, risk_style),
        ]),
        Line::from(vec![
            Span::styled("Uncertainty: ", bold),
            Span::styled(format_bar(snap.uncertainty), val),
            Span::styled("  Errors: ", bold),
            Span::styled(
                format!("{}", snap.error_streak),
                if snap.error_streak > 0 {
                    theme.tool_error
                } else {
                    val
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Context: ", bold),
            Span::styled(format_bar(snap.context_pressure), val),
            Span::styled("  Side-FX: ", bold),
            Span::styled(format_bar(snap.side_effect_pressure), val),
            Span::styled("  Human: ", bold),
            Span::styled(format_bar(snap.human_dependency), val),
        ]),
        Line::from(Span::styled("─── Budget ───", dim)),
        Line::from(vec![
            Span::styled("Tokens: ", bold),
            Span::styled(format!("{}", snap.tokens_remaining), val),
            Span::styled("  Tools: ", bold),
            Span::styled(format!("{}", snap.tool_calls_remaining), val),
            Span::styled("  Err budget: ", bold),
            Span::styled(format!("{}", snap.error_budget_remaining), val),
        ]),
        Line::from(vec![
            Span::styled("Cost: ", bold),
            Span::styled(format!("${:.4}", snap.cost_remaining_usd), val),
            Span::styled("  Time: ", bold),
            Span::styled(format!("{}ms", snap.time_remaining_ms), val),
        ]),
    ]
}

/// Render a simple 10-char bar: ████░░░░░░
fn format_bar(value: f32) -> String {
    let filled = (value.clamp(0.0, 1.0) * 10.0).round() as usize;
    let empty = 10 - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bar_zero() {
        assert_eq!(format_bar(0.0), "░░░░░░░░░░");
    }

    #[test]
    fn format_bar_full() {
        assert_eq!(format_bar(1.0), "██████████");
    }

    #[test]
    fn format_bar_half() {
        assert_eq!(format_bar(0.5), "█████░░░░░");
    }

    #[test]
    fn format_bar_clamps_above_one() {
        assert_eq!(format_bar(1.5), "██████████");
    }

    #[test]
    fn snapshot_defaults() {
        let snap = AgentStateSnapshot::default();
        assert_eq!(snap.mode, "");
        assert_eq!(snap.progress, 0.0);
    }

    #[test]
    fn inspector_state_set_snapshot() {
        let mut state = StateInspectorState::new();
        state.set_loading();
        assert!(state.loading);

        state.set_snapshot(AgentStateSnapshot {
            mode: "Explore".into(),
            progress: 0.5,
            ..Default::default()
        });
        assert!(!state.loading);
        assert!(state.snapshot.is_some());
    }
}
