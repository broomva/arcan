use crate::focus::FocusTarget;
use crate::history::InputHistory;
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Wraps a text buffer with cursor, command history, and mode tracking.
pub struct InputBarState {
    /// Current text content
    buffer: String,
    /// Cursor position (byte offset in buffer)
    cursor: usize,
    /// Command history
    pub history: InputHistory,
}

impl InputBarState {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            history: InputHistory::new(100),
        }
    }

    /// Get the current text content.
    pub fn text(&self) -> &str {
        &self.buffer
    }

    /// Clear the input buffer and reset cursor.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Submit the current input: records to history and clears the buffer.
    /// Returns the submitted text.
    pub fn submit(&mut self) -> String {
        let text = self.buffer.clone();
        if !text.trim().is_empty() {
            self.history.push(text.clone());
        }
        self.clear();
        text
    }

    /// Navigate history upward (older entries).
    pub fn history_up(&mut self) {
        let current = self.buffer.clone();
        if let Some(entry) = self.history.up(&current) {
            let entry = entry.to_string();
            self.buffer = entry;
            self.cursor = self.buffer.len();
        }
    }

    /// Navigate history downward (newer entries / draft).
    pub fn history_down(&mut self) {
        if let Some(entry) = self.history.down() {
            let entry = entry.to_string();
            self.buffer = entry;
            self.cursor = self.buffer.len();
        }
    }

    /// Handle a key event for text editing.
    pub fn input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                self.buffer.insert(self.cursor, c);
                self.cursor += c.len_utf8();
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    // Find the previous character boundary
                    let prev = self.buffer[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.buffer.drain(prev..self.cursor);
                    self.cursor = prev;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.buffer.len() {
                    let next = self.buffer[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.buffer.len());
                    self.buffer.drain(self.cursor..next);
                }
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Word-left: skip to previous word boundary
                    self.cursor = self.prev_word_boundary();
                } else if self.cursor > 0 {
                    self.cursor = self.buffer[..self.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Word-right: skip to next word boundary
                    self.cursor = self.next_word_boundary();
                } else if self.cursor < self.buffer.len() {
                    self.cursor = self.buffer[self.cursor..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor + i)
                        .unwrap_or(self.buffer.len());
                }
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
            }
            _ => {}
        }
    }

    fn prev_word_boundary(&self) -> usize {
        let before = &self.buffer[..self.cursor];
        // Skip trailing whitespace, then skip word chars
        let trimmed = before.trim_end();
        if trimmed.is_empty() {
            return 0;
        }
        trimmed
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn next_word_boundary(&self) -> usize {
        let after = &self.buffer[self.cursor..];
        // Skip current word chars, then skip whitespace
        let skip_word = after
            .find(|c: char| c.is_whitespace())
            .unwrap_or(after.len());
        let remaining = &after[skip_word..];
        let skip_space = remaining
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(remaining.len());
        self.cursor + skip_word + skip_space
    }

    /// Get cursor column position (character count, not byte offset).
    fn cursor_col(&self) -> usize {
        self.buffer[..self.cursor].chars().count()
    }
}

impl Default for InputBarState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the input bar widget.
pub fn render(
    f: &mut Frame,
    area: Rect,
    input_bar: &InputBarState,
    focus: FocusTarget,
    has_approval: bool,
    theme: &Theme,
) {
    let style = if has_approval {
        theme.input_approval
    } else {
        theme.input_normal
    };

    let border_style = if focus == FocusTarget::InputBar {
        theme.title
    } else {
        theme.border
    };

    let title = if has_approval {
        " Approval (yes/no) "
    } else {
        " Input "
    };

    let prompt = "\u{276f} "; // ❯
    let display_text = format!("{prompt}{}", input_bar.text());

    let input_widget = Paragraph::new(Line::from(vec![Span::raw(display_text)]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        )
        .style(style);
    f.render_widget(input_widget, area);

    // Position cursor within the input box (account for border + prompt)
    if focus == FocusTarget::InputBar {
        let cursor_x = area.x + 1 + prompt.chars().count() as u16 + input_bar.cursor_col() as u16;
        let cursor_y = area.y + 1;
        if cursor_x < area.x + area.width - 1 {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_input_bar_is_empty() {
        let bar = InputBarState::new();
        assert_eq!(bar.text(), "");
    }

    #[test]
    fn submit_returns_text_and_clears() {
        let mut bar = InputBarState::new();
        bar.input(key('h'));
        bar.input(key('i'));
        let submitted = bar.submit();
        assert_eq!(submitted, "hi");
        assert_eq!(bar.text(), "");
    }

    #[test]
    fn history_navigation() {
        let mut bar = InputBarState::new();
        bar.buffer = "first".to_string();
        bar.cursor = 5;
        bar.submit();
        bar.buffer = "second".to_string();
        bar.cursor = 6;
        bar.submit();

        // Type something new, then navigate up
        bar.buffer = "draft".to_string();
        bar.cursor = 5;
        bar.history_up();
        assert_eq!(bar.text(), "second");

        bar.history_up();
        assert_eq!(bar.text(), "first");

        bar.history_down();
        assert_eq!(bar.text(), "second");

        bar.history_down();
        assert_eq!(bar.text(), "draft");
    }

    #[test]
    fn cursor_movement() {
        let mut bar = InputBarState::new();
        for c in "hello".chars() {
            bar.input(key(c));
        }
        assert_eq!(bar.cursor, 5);

        bar.input(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(bar.cursor, 4);

        bar.input(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(bar.cursor, 0);

        bar.input(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(bar.cursor, 5);
    }

    #[test]
    fn backspace_at_cursor() {
        let mut bar = InputBarState::new();
        for c in "abc".chars() {
            bar.input(key(c));
        }
        // Cursor at end: "abc|"
        bar.input(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        // Cursor: "ab|c"
        bar.input(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        // Should delete 'b': "a|c"
        assert_eq!(bar.text(), "ac");
        assert_eq!(bar.cursor, 1);
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
}
