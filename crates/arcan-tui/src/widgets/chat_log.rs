use crate::focus::FocusTarget;
use crate::models::state::AppState;
use crate::models::ui_block::{ToolStatus, UiBlock};
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render the chat log into the given area, respecting scroll state.
pub fn render(f: &mut Frame, area: Rect, state: &mut AppState, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    for block in &state.blocks {
        match block {
            UiBlock::HumanMessage { text, timestamp } => {
                let ts = timestamp.format("%H:%M");
                lines.push(Line::from(vec![
                    Span::styled(format!("[{ts}] "), theme.timestamp),
                    Span::styled("You: ", theme.human_label),
                    Span::raw(text.clone()),
                ]));
            }
            UiBlock::AssistantMessage { text, timestamp } => {
                let ts = timestamp.format("%H:%M");
                lines.push(Line::from(vec![
                    Span::styled(format!("[{ts}] "), theme.timestamp),
                    Span::styled("Assistant: ", theme.assistant_label),
                    Span::raw(text.clone()),
                ]));
            }
            UiBlock::ToolExecution {
                tool_name,
                status,
                timestamp,
                ..
            } => {
                let ts = timestamp.format("%H:%M");
                let (status_str, status_style) = match status {
                    ToolStatus::Running => ("(Running...)", theme.tool_label),
                    ToolStatus::Success => ("[OK]", theme.tool_success),
                    ToolStatus::Error(_) => ("[ERR]", theme.tool_error),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("[{ts}] "), theme.timestamp),
                    Span::styled("Tool ", theme.tool_label),
                    Span::styled(format!("{tool_name} "), theme.tool_label),
                    Span::styled(status_str, status_style),
                ]));
            }
            UiBlock::SystemAlert { text, timestamp } => {
                let ts = timestamp.format("%H:%M");
                lines.push(Line::from(vec![
                    Span::styled(format!("[{ts}] "), theme.timestamp),
                    Span::styled("System: ", theme.system_label),
                    Span::styled(text.clone(), theme.system_label),
                ]));
            }
        }
    }

    // Streaming text with cursor
    if let Some(streaming) = &state.streaming_text {
        lines.push(Line::from(vec![
            Span::styled("Assistant: ", theme.assistant_label),
            Span::raw(streaming.clone()),
            Span::styled(" █", theme.streaming_cursor),
        ]));
    }

    // Busy indicator when no streaming text yet
    if state.is_busy && state.streaming_text.is_none() {
        lines.push(Line::from(vec![Span::styled(
            " Thinking...",
            theme.spinner,
        )]));
    }

    // Update scroll dimensions (viewport = area minus 2 border rows)
    let inner_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();
    state.scroll.update_dimensions(total_lines, inner_height);
    let scroll_pos = state.scroll.compute_scroll_position();

    let border_style = if state.focus == FocusTarget::ChatLog {
        theme.title
    } else {
        theme.border
    };

    let chat_block = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Session Log "),
        )
        .wrap(Wrap { trim: false })
        .scroll(scroll_pos);

    f.render_widget(chat_block, area);
}
