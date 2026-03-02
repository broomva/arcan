/// Scroll state using offset-from-bottom semantics.
///
/// `offset = 0` means the latest content is visible (auto-follow mode).
/// Scrolling up increases offset; reaching bottom re-enables auto-follow.
#[derive(Debug, Clone)]
pub struct ScrollState {
    /// Distance from bottom (0 = latest message visible).
    pub offset: usize,
    /// Whether to auto-follow new messages.
    pub auto_follow: bool,
    /// Total number of rendered lines (set each frame).
    pub total_lines: usize,
    /// Visible height of the viewport (set each frame).
    pub viewport_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            offset: 0,
            auto_follow: true,
            total_lines: 0,
            viewport_height: 0,
        }
    }
}

impl ScrollState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Scroll up by `lines` rows. Disables auto-follow.
    pub fn scroll_up(&mut self, lines: usize) {
        let max_offset = self.max_offset();
        self.offset = (self.offset + lines).min(max_offset);
        if self.offset > 0 {
            self.auto_follow = false;
        }
    }

    /// Scroll down by `lines` rows. Re-enables auto-follow if we reach bottom.
    pub fn scroll_down(&mut self, lines: usize) {
        self.offset = self.offset.saturating_sub(lines);
        if self.offset == 0 {
            self.auto_follow = true;
        }
    }

    /// Scroll up by one page (viewport height - 1).
    pub fn page_up(&mut self) {
        self.scroll_up(self.viewport_height.saturating_sub(1).max(1));
    }

    /// Scroll down by one page (viewport height - 1).
    pub fn page_down(&mut self) {
        self.scroll_down(self.viewport_height.saturating_sub(1).max(1));
    }

    /// Jump to the bottom and re-enable auto-follow.
    pub fn scroll_to_bottom(&mut self) {
        self.offset = 0;
        self.auto_follow = true;
    }

    /// Compute the scroll position for ratatui's `Paragraph::scroll()`.
    /// Returns `(row_offset_from_top, 0)`.
    pub fn compute_scroll_position(&self) -> (u16, u16) {
        let max = self.max_offset();
        let from_top = max.saturating_sub(self.offset);
        (from_top as u16, 0)
    }

    /// Update dimensions each frame. Clamps offset if content shrinks.
    pub fn update_dimensions(&mut self, total_lines: usize, viewport_height: usize) {
        self.total_lines = total_lines;
        self.viewport_height = viewport_height;
        if self.auto_follow {
            self.offset = 0;
        } else {
            self.offset = self.offset.min(self.max_offset());
        }
    }

    fn max_offset(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_starts_at_bottom() {
        let s = ScrollState::new();
        assert_eq!(s.offset, 0);
        assert!(s.auto_follow);
    }

    #[test]
    fn scroll_up_disables_auto_follow() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        s.scroll_up(5);
        assert_eq!(s.offset, 5);
        assert!(!s.auto_follow);
    }

    #[test]
    fn scroll_down_to_bottom_enables_auto_follow() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        s.scroll_up(10);
        assert!(!s.auto_follow);
        s.scroll_down(10);
        assert_eq!(s.offset, 0);
        assert!(s.auto_follow);
    }

    #[test]
    fn scroll_up_clamps_to_max() {
        let mut s = ScrollState::new();
        s.update_dimensions(50, 20);
        s.scroll_up(999);
        assert_eq!(s.offset, 30); // 50 - 20
    }

    #[test]
    fn page_up_and_down() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        s.page_up();
        assert_eq!(s.offset, 19); // viewport_height - 1
        s.page_down();
        assert_eq!(s.offset, 0);
        assert!(s.auto_follow);
    }

    #[test]
    fn compute_scroll_position_at_bottom() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        // offset=0 means we're at bottom, so from_top = max_offset
        let (row, _) = s.compute_scroll_position();
        assert_eq!(row, 80); // 100 - 20
    }

    #[test]
    fn compute_scroll_position_scrolled_up() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        s.scroll_up(30);
        let (row, _) = s.compute_scroll_position();
        assert_eq!(row, 50); // (100-20) - 30
    }

    #[test]
    fn update_dimensions_clamps_offset() {
        let mut s = ScrollState::new();
        s.update_dimensions(100, 20);
        s.scroll_up(50);
        assert_eq!(s.offset, 50);
        // Content shrinks
        s.update_dimensions(30, 20);
        assert_eq!(s.offset, 10); // clamped to 30 - 20
    }
}
