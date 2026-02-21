use crate::models::{AppState, UiBlock};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub fn draw<B: Backend>(f: &mut Frame<B>, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Min(3),    // Chat log
            Constraint::Length(3), // Input box
        ])
        .split(f.size());

    // Messages Area
    let mut message_lines = Vec::new();
    for block in &state.blocks {
        match block {
            UiBlock::HumanMessage { text, timestamp: _ } => {
                message_lines.push(Line::from(vec![
                    Span::styled("You: ", Style::default().fg(Color::Blue)),
                    Span::raw(text.clone()),
                ]));
            }
            UiBlock::AssistantMessage { text, timestamp: _ } => {
                message_lines.push(Line::from(vec![
                    Span::styled("Assistant: ", Style::default().fg(Color::Green)),
                    Span::raw(text.clone()),
                ]));
            }
            UiBlock::ToolExecution { tool_name, status, .. } => {
                let status_str = match status {
                    crate::models::ToolStatus::Running => "(Running...)",
                    crate::models::ToolStatus::Success => "[OK]",
                    crate::models::ToolStatus::Error(_) => "[ERR]",
                };
                message_lines.push(Line::from(vec![
                    Span::styled("Tool ", Style::default().fg(Color::Yellow)),
                    Span::styled(format!("{} {}", tool_name, status_str), Style::default().fg(Color::Yellow)),
                ]));
            }
            UiBlock::SystemAlert { text, .. } => {
                message_lines.push(Line::from(vec![
                    Span::styled("System: ", Style::default().fg(Color::Red)),
                    Span::styled(text.clone(), Style::default().fg(Color::Red)),
                ]));
            }
        }
    }

    if let Some(streaming) = &state.streaming_text {
        message_lines.push(Line::from(vec![
            Span::styled("Assistant: ", Style::default().fg(Color::Green)),
            Span::raw(streaming.clone()),
            Span::styled(" █", Style::default().fg(Color::Gray)),
        ]));
    }

    let messages_block = Paragraph::new(message_lines)
        .block(Block::default().borders(Borders::ALL).title(" Session Log "))
        .wrap(Wrap { trim: false });
    f.render_widget(messages_block, chunks[0]);

    // Input Area
    let prompt = if let Some(approval) = &state.pending_approval {
        format!("Approval Req for {}: (yes/no) > {}", approval.tool_name, state.input_buffer)
    } else {
        format!("❯ {}", state.input_buffer)
    };

    let input_block = Paragraph::new(prompt)
        .block(Block::default().borders(Borders::ALL).title(" Input "))
        .style(if state.pending_approval.is_some() {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::White)
        });
    f.render_widget(input_block, chunks[1]);
}
