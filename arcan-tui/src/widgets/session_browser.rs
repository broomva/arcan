use crate::theme::Theme;
use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

/// Summary of a session returned by the daemon.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub session_id: String,
    pub owner: String,
    pub created_at: DateTime<Utc>,
}

/// State for the session browser panel.
#[derive(Debug, Default)]
pub struct SessionBrowserState {
    pub sessions: Vec<SessionEntry>,
    pub list_state: ListState,
    pub loading: bool,
    pub error: Option<String>,
}

impl SessionBrowserState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Select the next session in the list.
    pub fn next(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map(|i| (i + 1) % self.sessions.len())
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    /// Select the previous session in the list.
    pub fn previous(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        let i = self
            .list_state
            .selected()
            .map(|i| {
                if i == 0 {
                    self.sessions.len() - 1
                } else {
                    i - 1
                }
            })
            .unwrap_or(0);
        self.list_state.select(Some(i));
    }

    /// Get the currently selected session ID, if any.
    pub fn selected_session_id(&self) -> Option<&str> {
        self.list_state
            .selected()
            .and_then(|i| self.sessions.get(i))
            .map(|s| s.session_id.as_str())
    }

    /// Update the session list (typically after a fetch).
    pub fn set_sessions(&mut self, sessions: Vec<SessionEntry>) {
        self.sessions = sessions;
        self.loading = false;
        self.error = None;
        // Select first if none selected
        if self.list_state.selected().is_none() && !self.sessions.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Mark as loading.
    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    /// Mark as errored.
    pub fn set_error(&mut self, msg: String) {
        self.loading = false;
        self.error = Some(msg);
    }
}

/// Render the session browser panel.
pub fn render(
    f: &mut Frame,
    area: Rect,
    state: &mut SessionBrowserState,
    focused: bool,
    theme: &Theme,
) {
    let border_style = if focused { theme.title } else { theme.border };

    let items: Vec<ListItem> = if state.loading {
        vec![ListItem::new(Line::from(Span::styled(
            "Loading sessions...",
            theme.timestamp,
        )))]
    } else if let Some(ref err) = state.error {
        vec![ListItem::new(Line::from(Span::styled(
            format!("Error: {err}"),
            theme.tool_error,
        )))]
    } else if state.sessions.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No sessions found",
            theme.timestamp,
        )))]
    } else {
        state
            .sessions
            .iter()
            .map(|s| {
                let time = s.created_at.format("%m-%d %H:%M");
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{time} "), theme.timestamp),
                    Span::styled(&s.session_id, theme.assistant_label),
                    Span::raw(" "),
                    Span::styled(format!("({})", s.owner), theme.timestamp),
                ]))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Sessions "),
        )
        .highlight_style(theme.title.add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ");

    f.render_stateful_widget(list, area, &mut state.list_state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_around() {
        let mut state = SessionBrowserState::new();
        state.set_sessions(vec![
            SessionEntry {
                session_id: "s1".into(),
                owner: "alice".into(),
                created_at: Utc::now(),
            },
            SessionEntry {
                session_id: "s2".into(),
                owner: "bob".into(),
                created_at: Utc::now(),
            },
        ]);
        state.list_state.select(Some(0));

        state.previous(); // wraps to last
        assert_eq!(state.list_state.selected(), Some(1));

        state.next(); // wraps to first
        assert_eq!(state.list_state.selected(), Some(0));
    }

    #[test]
    fn selected_session_id_returns_correct_id() {
        let mut state = SessionBrowserState::new();
        state.set_sessions(vec![SessionEntry {
            session_id: "sess-abc".into(),
            owner: "user".into(),
            created_at: Utc::now(),
        }]);
        assert_eq!(state.selected_session_id(), Some("sess-abc"));
    }

    #[test]
    fn set_sessions_selects_first_entry() {
        let mut state = SessionBrowserState::new();
        assert_eq!(state.list_state.selected(), None);

        state.set_sessions(vec![SessionEntry {
            session_id: "s1".into(),
            owner: "o".into(),
            created_at: Utc::now(),
        }]);
        assert_eq!(state.list_state.selected(), Some(0));
    }

    #[test]
    fn empty_list_navigation_is_safe() {
        let mut state = SessionBrowserState::new();
        state.next(); // should not panic
        state.previous(); // should not panic
        assert_eq!(state.selected_session_id(), None);
    }
}
