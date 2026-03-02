use crate::command::{CommandInfo, filter_commands};
use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
};

/// State for the slash command autocomplete popup.
#[derive(Debug)]
pub struct AutocompleteState {
    /// Whether the popup is currently visible.
    pub active: bool,
    /// Filtered suggestions based on current input.
    pub suggestions: Vec<&'static CommandInfo>,
    /// Currently selected suggestion index.
    pub selected: usize,
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            active: false,
            suggestions: Vec::new(),
            selected: 0,
        }
    }

    /// Update suggestions based on current input buffer.
    /// Activates when input starts with `/` and has no spaces yet (still typing the command name).
    pub fn update(&mut self, input: &str) {
        if input.starts_with('/') && !input[1..].contains(' ') {
            self.suggestions = filter_commands(input);
            self.active = !self.suggestions.is_empty();
            if self.selected >= self.suggestions.len() {
                self.selected = self.suggestions.len().saturating_sub(1);
            }
        } else {
            self.dismiss();
        }
    }

    /// Move selection up (wraps around).
    pub fn previous(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.suggestions.len() - 1);
        }
    }

    /// Move selection down (wraps around).
    pub fn next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = (self.selected + 1) % self.suggestions.len();
        }
    }

    /// Accept the currently selected suggestion. Returns the command name + trailing space.
    pub fn accept(&mut self) -> Option<String> {
        if self.active && !self.suggestions.is_empty() {
            let name = self.suggestions[self.selected].name.to_string();
            self.dismiss();
            Some(name)
        } else {
            None
        }
    }

    /// Dismiss the popup and clear state.
    pub fn dismiss(&mut self) {
        self.active = false;
        self.suggestions.clear();
        self.selected = 0;
    }

    /// Height needed for the popup (capped at 8 visible rows + 2 for border).
    pub fn popup_height(&self) -> u16 {
        let rows = self.suggestions.len().min(8) as u16;
        rows + 2
    }
}

impl Default for AutocompleteState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the autocomplete popup as a floating overlay above the input area.
pub fn render(f: &mut Frame, input_area: Rect, state: &AutocompleteState, theme: &Theme) {
    if !state.active || state.suggestions.is_empty() {
        return;
    }

    let height = state.popup_height();
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(height),
        width: input_area.width.min(60),
        height,
    };

    // Clear the area first (overlay effect)
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = state
        .suggestions
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let (name_style, desc_style) = if i == state.selected {
                (theme.autocomplete_selected, theme.autocomplete_selected)
            } else {
                (theme.autocomplete_normal, theme.autocomplete_description)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:<12}", cmd.name), name_style),
                Span::styled(format!(" {}", cmd.description), desc_style),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border)
            .title(" Commands "),
    );

    f.render_widget(list, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_inactive() {
        let state = AutocompleteState::new();
        assert!(!state.active);
        assert!(state.suggestions.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn update_with_slash_activates() {
        let mut state = AutocompleteState::new();
        state.update("/");
        assert!(state.active);
        assert!(!state.suggestions.is_empty());
    }

    #[test]
    fn update_with_prefix_filters() {
        let mut state = AutocompleteState::new();
        state.update("/cl");
        assert!(state.active);
        assert_eq!(state.suggestions.len(), 1);
        assert_eq!(state.suggestions[0].name, "/clear");
    }

    #[test]
    fn update_no_match_deactivates() {
        let mut state = AutocompleteState::new();
        state.update("/xyz");
        assert!(!state.active);
    }

    #[test]
    fn update_no_slash_deactivates() {
        let mut state = AutocompleteState::new();
        state.update("hello");
        assert!(!state.active);
    }

    #[test]
    fn update_with_space_deactivates() {
        let mut state = AutocompleteState::new();
        state.update("/login openai");
        assert!(!state.active);
    }

    #[test]
    fn navigation_wraps() {
        let mut state = AutocompleteState::new();
        state.update("/");
        let count = state.suggestions.len();

        // Go past the end → wraps to 0
        for _ in 0..count {
            state.next();
        }
        assert_eq!(state.selected, 0);

        // Go before the start → wraps to last
        state.previous();
        assert_eq!(state.selected, count - 1);
    }

    #[test]
    fn accept_returns_name_and_dismisses() {
        let mut state = AutocompleteState::new();
        state.update("/cl");
        assert!(state.active);

        let accepted = state.accept();
        assert_eq!(accepted.as_deref(), Some("/clear"));
        assert!(!state.active);
    }

    #[test]
    fn accept_when_inactive_returns_none() {
        let mut state = AutocompleteState::new();
        assert!(state.accept().is_none());
    }

    #[test]
    fn dismiss_clears_state() {
        let mut state = AutocompleteState::new();
        state.update("/");
        state.next();
        state.dismiss();

        assert!(!state.active);
        assert!(state.suggestions.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn popup_height_capped_at_ten() {
        let mut state = AutocompleteState::new();
        state.update("/"); // all commands
        let h = state.popup_height();
        // max 8 visible rows + 2 border = 10
        assert!(h <= 10);
    }

    #[test]
    fn selected_clamps_on_filter_change() {
        let mut state = AutocompleteState::new();
        state.update("/");
        // Select last item
        state.selected = state.suggestions.len() - 1;

        // Now filter to fewer items
        state.update("/cl");
        assert!(state.selected < state.suggestions.len());
    }
}
