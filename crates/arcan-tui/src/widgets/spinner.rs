/// Neural pulse — primary animation for the TUI (matches shell spinner).
const NEURAL_PULSE: &[char] = &['·', '◦', '○', '◎', '●', '◉', '●', '◎', '○', '◦'];

/// Braille spinner for tool execution.
const TOOL_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Random verbs for the thinking indicator (subset from shell spinner).
const VERBS: &[&str] = &[
    "Thinking",
    "Pondering",
    "Cogitating",
    "Reasoning",
    "Deliberating",
    "Synthesizing",
    "Brewing",
    "Percolating",
    "Crystallizing",
    "Arcaning",
    "Perceiving",
    "Orchestrating",
    "Computing",
    "Contemplating",
    "Mulling",
    "Manifesting",
    "Ruminating",
    "Ideating",
    "Crafting",
    "Composing",
    "Pulsing",
    "Looping",
    "Cognizing",
    "Calibrating",
    "Evolving",
];

/// A Unicode spinner for indicating busy state, with animated glyphs and verbs.
#[derive(Debug, Clone)]
pub struct Spinner {
    frame: usize,
    verb: &'static str,
    kind: SpinnerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerKind {
    /// Neural pulse animation for LLM thinking.
    Neural,
    /// Braille animation for tool execution.
    Tool,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            frame: 0,
            verb: pick_verb(),
            kind: SpinnerKind::Neural,
        }
    }

    /// Advance to the next frame.
    pub fn tick(&mut self) {
        let frames = self.frames();
        self.frame = (self.frame + 1) % frames.len();
    }

    /// Return the current spinner character.
    pub fn current(&self) -> char {
        let frames = self.frames();
        frames[self.frame]
    }

    /// Return the current verb.
    pub fn verb(&self) -> &str {
        self.verb
    }

    /// Pick a new random verb (called when a new run starts).
    pub fn new_verb(&mut self) {
        self.verb = pick_verb();
        self.frame = 0;
    }

    /// Switch to tool spinner mode.
    pub fn set_tool_mode(&mut self) {
        self.kind = SpinnerKind::Tool;
        self.frame = 0;
    }

    /// Switch back to neural pulse mode.
    pub fn set_neural_mode(&mut self) {
        self.kind = SpinnerKind::Neural;
        self.frame = 0;
    }

    fn frames(&self) -> &'static [char] {
        match self.kind {
            SpinnerKind::Neural => NEURAL_PULSE,
            SpinnerKind::Tool => TOOL_FRAMES,
        }
    }
}

fn pick_verb() -> &'static str {
    VERBS[fastrand::usize(..VERBS.len())]
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
        for _ in 0..NEURAL_PULSE.len() - 1 {
            s.tick();
        }
        assert_eq!(s.current(), first);
    }

    #[test]
    fn spinner_has_verb() {
        let s = Spinner::new();
        assert!(!s.verb().is_empty());
        assert!(VERBS.contains(&s.verb()));
    }

    #[test]
    fn new_verb_picks_valid_verb() {
        let mut s = Spinner::new();
        for _ in 0..10 {
            s.new_verb();
            assert!(!s.verb().is_empty());
            assert!(VERBS.contains(&s.verb()));
        }
    }

    #[test]
    fn tool_mode_uses_braille() {
        let mut s = Spinner::new();
        s.set_tool_mode();
        assert!(TOOL_FRAMES.contains(&s.current()));
    }
}
