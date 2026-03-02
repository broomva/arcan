use crate::focus::FocusTarget;
use crate::models::state::AppState;
use crate::models::ui_block::UiBlock;
use crate::theme::Theme;
use crate::widgets::markdown::MarkdownRenderer;
use crate::widgets::tool_panel;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

/// Render the chat log into the given area, respecting scroll state.
///
/// Assistant messages are rendered through the `MarkdownRenderer` for rich
/// formatting (bold, italic, code blocks with syntax highlighting, lists, etc.).
/// Tool executions are rendered through the `tool_panel` module showing
/// arguments and results inline.
pub fn render(
    f: &mut Frame,
    area: Rect,
    state: &mut AppState,
    theme: &Theme,
    md: &mut MarkdownRenderer,
) {
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
                // Label line
                lines.push(Line::from(vec![
                    Span::styled(format!("[{ts}] "), theme.timestamp),
                    Span::styled("Assistant:", theme.assistant_label),
                ]));
                // Render markdown content (or plain text for short messages)
                if MarkdownRenderer::has_markdown(text) {
                    let md_lines = md.render(text);
                    lines.extend(md_lines);
                } else {
                    lines.push(Line::from(Span::raw(text.clone())));
                }
            }
            UiBlock::ToolExecution {
                tool_name,
                status,
                arguments,
                result,
                timestamp,
                ..
            } => {
                let tool_lines = tool_panel::render_tool_lines(
                    tool_name, status, arguments, result, timestamp, theme,
                );
                lines.extend(tool_lines);
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
