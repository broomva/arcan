use std::collections::VecDeque;

/// Ring buffer storing command history with cursor-based navigation.
///
/// - Pushing deduplicates consecutive identical entries.
/// - Up/Down navigation preserves the current draft (unsent text).
/// - Cursor is `None` when not navigating history.
pub struct InputHistory {
    entries: VecDeque<String>,
    capacity: usize,
    /// `None` = at the live draft; `Some(idx)` = viewing `entries[idx]`
    cursor: Option<usize>,
    /// The text the user was typing before they started navigating
    draft: String,
}

impl InputHistory {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
            cursor: None,
            draft: String::new(),
        }
    }

    /// Record a submitted entry. Deduplicates consecutive identical entries.
    pub fn push(&mut self, entry: String) {
        if entry.is_empty() {
            return;
        }
        // Dedup: skip if identical to the most recent entry
        if self.entries.front().is_some_and(|last| last == &entry) {
            self.reset_cursor();
            return;
        }
        if self.entries.len() >= self.capacity {
            self.entries.pop_back();
        }
        self.entries.push_front(entry);
        self.reset_cursor();
    }

    /// Navigate up (older entries). Returns the entry to display, or `None` if
    /// already at the oldest entry.
    ///
    /// On the first Up press, `current_input` is saved as the draft.
    pub fn up(&mut self, current_input: &str) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        match self.cursor {
            None => {
                // Entering history mode: save current input as draft
                self.draft = current_input.to_string();
                self.cursor = Some(0);
                Some(&self.entries[0])
            }
            Some(idx) => {
                let next = idx + 1;
                if next < self.entries.len() {
                    self.cursor = Some(next);
                    Some(&self.entries[next])
                } else {
                    // Already at oldest — stay put
                    Some(&self.entries[idx])
                }
            }
        }
    }

    /// Navigate down (newer entries). Returns the entry to display.
    /// When reaching the bottom, restores the draft.
    pub fn down(&mut self) -> Option<&str> {
        match self.cursor {
            None => None, // Not navigating
            Some(0) => {
                // Back to draft
                self.cursor = None;
                Some(&self.draft)
            }
            Some(idx) => {
                self.cursor = Some(idx - 1);
                Some(&self.entries[idx - 1])
            }
        }
    }

    /// Reset navigation state (e.g., after submitting).
    pub fn reset_cursor(&mut self) {
        self.cursor = None;
        self.draft.clear();
    }

    /// Check if currently navigating history.
    pub fn is_navigating(&self) -> bool {
        self.cursor.is_some()
    }

    /// Number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_returns_none_on_up() {
        let mut h = InputHistory::new(10);
        assert!(h.up("draft").is_none());
    }

    #[test]
    fn up_saves_draft_and_returns_last_entry() {
        let mut h = InputHistory::new(10);
        h.push("first".to_string());
        h.push("second".to_string());
        let entry = h.up("typing").unwrap();
        assert_eq!(entry, "second");
    }

    #[test]
    fn down_restores_draft() {
        let mut h = InputHistory::new(10);
        h.push("cmd1".to_string());
        h.up("my draft");
        let restored = h.down().unwrap();
        assert_eq!(restored, "my draft");
    }

    #[test]
    fn full_cycle_up_and_down() {
        let mut h = InputHistory::new(10);
        h.push("a".to_string());
        h.push("b".to_string());
        h.push("c".to_string());
        // Up: c -> b -> a
        assert_eq!(h.up("draft").unwrap(), "c");
        assert_eq!(h.up("draft").unwrap(), "b");
        assert_eq!(h.up("draft").unwrap(), "a");
        // Can't go further up
        assert_eq!(h.up("draft").unwrap(), "a");
        // Down: b -> c -> draft
        assert_eq!(h.down().unwrap(), "b");
        assert_eq!(h.down().unwrap(), "c");
        assert_eq!(h.down().unwrap(), "draft");
        // Down again => None (not navigating)
        assert!(h.down().is_none());
    }

    #[test]
    fn deduplicates_consecutive() {
        let mut h = InputHistory::new(10);
        h.push("same".to_string());
        h.push("same".to_string());
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn respects_capacity() {
        let mut h = InputHistory::new(3);
        h.push("a".to_string());
        h.push("b".to_string());
        h.push("c".to_string());
        h.push("d".to_string());
        assert_eq!(h.len(), 3);
        // Oldest ("a") was evicted
        assert_eq!(h.up("").unwrap(), "d");
        assert_eq!(h.up("").unwrap(), "c");
        assert_eq!(h.up("").unwrap(), "b");
        assert_eq!(h.up("").unwrap(), "b"); // can't go further
    }

    #[test]
    fn push_resets_cursor() {
        let mut h = InputHistory::new(10);
        h.push("a".to_string());
        h.up("draft");
        assert!(h.is_navigating());
        h.push("b".to_string());
        assert!(!h.is_navigating());
    }

    #[test]
    fn empty_string_not_pushed() {
        let mut h = InputHistory::new(10);
        h.push(String::new());
        assert!(h.is_empty());
    }
}
