const FRAMES: &[char] = &[
    '\u{280b}', '\u{2819}', '\u{2839}', '\u{2838}', '\u{283c}', '\u{2834}', '\u{2826}', '\u{2827}',
    '\u{2807}', '\u{280f}',
]; // ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏

/// A simple Unicode spinner for indicating busy state.
#[derive(Debug, Default, Clone)]
pub struct Spinner {
    frame: usize,
}

impl Spinner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance to the next frame.
    pub fn tick(&mut self) {
        self.frame = (self.frame + 1) % FRAMES.len();
    }

    /// Return the current spinner character.
    pub fn current(&self) -> char {
        FRAMES[self.frame]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_cycles_through_frames() {
        let mut s = Spinner::new();
        let first = s.current();
        s.tick();
        let second = s.current();
        assert_ne!(first, second);

        // Cycle through all frames
        for _ in 0..FRAMES.len() - 1 {
            s.tick();
        }
        // Back to the start
        assert_eq!(s.current(), first);
    }
}
