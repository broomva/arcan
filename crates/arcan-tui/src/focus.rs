/// Focus targets in the TUI layout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    ChatLog,
    #[default]
    InputBar,
    SessionBrowser,
    StateInspector,
}

impl FocusTarget {
    /// Cycle to the next focus target (Tab key behavior).
    /// Only cycles through targets that are currently visible.
    pub fn next(self) -> Self {
        match self {
            Self::ChatLog => Self::InputBar,
            Self::InputBar => Self::ChatLog,
            Self::SessionBrowser => Self::InputBar,
            Self::StateInspector => Self::InputBar,
        }
    }

    /// Cycle through all panels including side panels (Shift+Tab or explicit).
    pub fn next_all(self) -> Self {
        match self {
            Self::ChatLog => Self::InputBar,
            Self::InputBar => Self::SessionBrowser,
            Self::SessionBrowser => Self::StateInspector,
            Self::StateInspector => Self::ChatLog,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_cycles_between_targets() {
        let focus = FocusTarget::InputBar;
        assert_eq!(focus.next(), FocusTarget::ChatLog);
        assert_eq!(focus.next().next(), FocusTarget::InputBar);
    }

    #[test]
    fn default_focus_is_input_bar() {
        assert_eq!(FocusTarget::default(), FocusTarget::InputBar);
    }

    #[test]
    fn next_all_cycles_through_all_panels() {
        let mut focus = FocusTarget::ChatLog;
        focus = focus.next_all(); // InputBar
        assert_eq!(focus, FocusTarget::InputBar);
        focus = focus.next_all(); // SessionBrowser
        assert_eq!(focus, FocusTarget::SessionBrowser);
        focus = focus.next_all(); // StateInspector
        assert_eq!(focus, FocusTarget::StateInspector);
        focus = focus.next_all(); // ChatLog (wrap)
        assert_eq!(focus, FocusTarget::ChatLog);
    }

    #[test]
    fn side_panels_tab_back_to_input() {
        assert_eq!(FocusTarget::SessionBrowser.next(), FocusTarget::InputBar);
        assert_eq!(FocusTarget::StateInspector.next(), FocusTarget::InputBar);
    }
}
