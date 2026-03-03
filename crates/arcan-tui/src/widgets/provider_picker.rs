use crate::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
};

/// State for the interactive provider picker popup.
#[derive(Debug)]
pub struct ProviderPickerState {
    /// Whether the popup is currently visible.
    pub active: bool,
    /// Available provider names.
    pub providers: Vec<String>,
    /// The currently active provider (marked with a checkmark).
    pub current_provider: String,
    /// Index of the highlighted row.
    pub selected: usize,
    /// True while waiting for the daemon response.
    pub loading: bool,
}

impl ProviderPickerState {
    pub fn new() -> Self {
        Self {
            active: false,
            providers: Vec::new(),
            current_provider: String::new(),
            selected: 0,
            loading: false,
        }
    }

    /// Show the picker in a loading state while fetching provider info.
    pub fn show_loading(&mut self) {
        self.active = true;
        self.loading = true;
        self.providers.clear();
        self.current_provider.clear();
        self.selected = 0;
    }

    /// Populate the picker with provider data. Pre-selects the current provider.
    pub fn set_providers(&mut self, current: String, available: Vec<String>) {
        self.loading = false;
        self.current_provider = current.clone();
        self.providers = available;

        // Pre-select the current provider
        self.selected = self
            .providers
            .iter()
            .position(|p| p == &current)
            .unwrap_or(0);
    }

    /// Move selection up (wraps around).
    pub fn previous(&mut self) {
        if !self.providers.is_empty() {
            self.selected = self
                .selected
                .checked_sub(1)
                .unwrap_or(self.providers.len() - 1);
        }
    }

    /// Move selection down (wraps around).
    pub fn next(&mut self) {
        if !self.providers.is_empty() {
            self.selected = (self.selected + 1) % self.providers.len();
        }
    }

    /// Accept the currently selected provider. Returns the provider name.
    pub fn accept(&mut self) -> Option<String> {
        if self.active && !self.providers.is_empty() {
            let name = self.providers[self.selected].clone();
            self.dismiss();
            Some(name)
        } else {
            None
        }
    }

    /// Dismiss the popup and clear state.
    pub fn dismiss(&mut self) {
        self.active = false;
        self.loading = false;
        self.providers.clear();
        self.current_provider.clear();
        self.selected = 0;
    }

    /// Height needed for the popup (capped at 8 visible rows + 2 for border).
    pub fn popup_height(&self) -> u16 {
        if self.loading {
            return 3; // border + 1 line "Loading..."
        }
        let rows = self.providers.len().min(8) as u16;
        rows + 2
    }
}

impl Default for ProviderPickerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the provider picker popup as a floating overlay above the input area.
pub fn render(f: &mut Frame, input_area: Rect, state: &ProviderPickerState, theme: &Theme) {
    if !state.active {
        return;
    }

    let height = state.popup_height();
    let popup_area = Rect {
        x: input_area.x,
        y: input_area.y.saturating_sub(height),
        width: input_area.width.min(50),
        height,
    };

    // Clear the area first (overlay effect)
    f.render_widget(Clear, popup_area);

    if state.loading {
        let items = vec![ListItem::new(Line::from(Span::styled(
            " Loading providers...",
            theme.autocomplete_description,
        )))];
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(" Providers "),
        );
        f.render_widget(list, popup_area);
        return;
    }

    let items: Vec<ListItem> = state
        .providers
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let is_current = name == &state.current_provider;
            let marker = if is_current { " \u{2713} " } else { "   " };

            let (name_style, marker_style) = if i == state.selected {
                (theme.autocomplete_selected, theme.autocomplete_selected)
            } else {
                (
                    theme.autocomplete_normal,
                    if is_current {
                        theme.status_connected
                    } else {
                        theme.autocomplete_description
                    },
                )
            };

            ListItem::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(name.clone(), name_style),
            ]))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border)
            .title(" Providers (Enter to switch, Esc to cancel) "),
    );

    f.render_widget(list, popup_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_inactive() {
        let state = ProviderPickerState::new();
        assert!(!state.active);
        assert!(state.providers.is_empty());
        assert_eq!(state.selected, 0);
        assert!(!state.loading);
    }

    #[test]
    fn show_loading_activates_with_loading_flag() {
        let mut state = ProviderPickerState::new();
        state.show_loading();
        assert!(state.active);
        assert!(state.loading);
        assert!(state.providers.is_empty());
    }

    #[test]
    fn set_providers_clears_loading_and_pre_selects_current() {
        let mut state = ProviderPickerState::new();
        state.show_loading();

        state.set_providers(
            "openai".to_string(),
            vec![
                "mock".to_string(),
                "anthropic".to_string(),
                "openai".to_string(),
            ],
        );

        assert!(!state.loading);
        assert_eq!(state.providers.len(), 3);
        assert_eq!(state.current_provider, "openai");
        assert_eq!(state.selected, 2); // "openai" is at index 2
    }

    #[test]
    fn set_providers_defaults_to_zero_when_current_not_found() {
        let mut state = ProviderPickerState::new();
        state.show_loading();
        state.set_providers(
            "unknown".to_string(),
            vec!["mock".to_string(), "anthropic".to_string()],
        );
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn navigation_wraps() {
        let mut state = ProviderPickerState::new();
        state.set_providers(
            "mock".to_string(),
            vec![
                "mock".to_string(),
                "anthropic".to_string(),
                "openai".to_string(),
            ],
        );
        state.active = true;

        // At index 0, go previous → wraps to last
        state.selected = 0;
        state.previous();
        assert_eq!(state.selected, 2);

        // At last, go next → wraps to 0
        state.next();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn accept_returns_selected_and_dismisses() {
        let mut state = ProviderPickerState::new();
        state.active = true;
        state.providers = vec![
            "mock".to_string(),
            "anthropic".to_string(),
            "openai".to_string(),
        ];
        state.selected = 1;

        let accepted = state.accept();
        assert_eq!(accepted.as_deref(), Some("anthropic"));
        assert!(!state.active);
        assert!(state.providers.is_empty());
    }

    #[test]
    fn accept_when_inactive_returns_none() {
        let mut state = ProviderPickerState::new();
        assert!(state.accept().is_none());
    }

    #[test]
    fn dismiss_clears_all_state() {
        let mut state = ProviderPickerState::new();
        state.active = true;
        state.loading = true;
        state.providers = vec!["mock".to_string()];
        state.selected = 1;

        state.dismiss();
        assert!(!state.active);
        assert!(!state.loading);
        assert!(state.providers.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn popup_height_loading_is_three() {
        let mut state = ProviderPickerState::new();
        state.show_loading();
        assert_eq!(state.popup_height(), 3);
    }

    #[test]
    fn popup_height_with_providers() {
        let mut state = ProviderPickerState::new();
        state.providers = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // 3 rows + 2 border = 5
        assert_eq!(state.popup_height(), 5);
    }

    #[test]
    fn popup_height_capped_at_ten() {
        let mut state = ProviderPickerState::new();
        state.providers = (0..20).map(|i| format!("provider-{i}")).collect();
        // max 8 rows + 2 border = 10
        assert_eq!(state.popup_height(), 10);
    }
}
