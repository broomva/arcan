/// Focus targets in the TUI layout.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    ChatLog,
    #[default]
    InputBar,
}

impl FocusTarget {
    /// Cycle to the next focus target (Tab key behavior).
    pub fn next(self) -> Self {
        match self {
            Self::ChatLog => Self::InputBar,
            Self::InputBar => Self::ChatLog,
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
}
