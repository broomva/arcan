//! Animated status line for the Arcan shell REPL.
//!
//! Renders a spinner with a random verb, elapsed time, token count, and cost
//! to stderr using ANSI escape codes. Disables itself when stderr is not a TTY.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Glyph sets
// ---------------------------------------------------------------------------

/// Neural pulse — primary animation for macOS.
const NEURAL_PULSE: &[char] = &['·', '◦', '○', '◎', '●', '◉', '●', '◎', '○', '◦'];

/// Arcane sigils — alternative animation.
#[allow(dead_code)]
const ARCANE_SIGILS: &[char] = &['✧', '✦', '✶', '✷', '✹', '✷', '✶', '✦'];

/// Fallback for limited terminals.
const FALLBACK_GLYPHS: &[char] = &['·', 'o', 'O', '@', 'O', 'o'];

/// Braille spinner for tool execution.
const TOOL_SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The idle/completion marker glyph.
const IDLE_MARKER: char = '◉';

/// Pick the appropriate glyph set for the current platform.
fn default_glyphs() -> &'static [char] {
    if cfg!(target_os = "macos") {
        NEURAL_PULSE
    } else {
        FALLBACK_GLYPHS
    }
}

// ---------------------------------------------------------------------------
// Spinner verbs (228 total: 188 Noesis base + 40 Life framework)
// ---------------------------------------------------------------------------

const SPINNER_VERBS: &[&str] = &[
    // --- Noesis / Claude Code base set (188) ---
    "Accomplishing",
    "Actioning",
    "Actualizing",
    "Architecting",
    "Baking",
    "Beaming",
    "Beboppin'",
    "Befuddling",
    "Billowing",
    "Blanching",
    "Bloviating",
    "Boogieing",
    "Boondoggling",
    "Booping",
    "Bootstrapping",
    "Brewing",
    "Bunning",
    "Burrowing",
    "Calculating",
    "Canoodling",
    "Caramelizing",
    "Cascading",
    "Catapulting",
    "Cerebrating",
    "Channeling",
    "Channelling",
    "Choreographing",
    "Churning",
    "Coalescing",
    "Cogitating",
    "Combobulating",
    "Composing",
    "Computing",
    "Concocting",
    "Considering",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Creating",
    "Crunching",
    "Crystallizing",
    "Cultivating",
    "Deciphering",
    "Deliberating",
    "Determining",
    "Dilly-dallying",
    "Discombobulating",
    "Doing",
    "Doodling",
    "Drizzling",
    "Ebbing",
    "Effecting",
    "Elucidating",
    "Embellishing",
    "Enchanting",
    "Envisioning",
    "Evaporating",
    "Fermenting",
    "Fiddle-faddling",
    "Finagling",
    "Flamb\u{00e9}ing",
    "Flibbertigibbeting",
    "Flowing",
    "Flummoxing",
    "Fluttering",
    "Forging",
    "Forming",
    "Frolicking",
    "Frosting",
    "Gallivanting",
    "Galloping",
    "Garnishing",
    "Generating",
    "Gesticulating",
    "Germinating",
    "Grooving",
    "Gusting",
    "Harmonizing",
    "Hashing",
    "Hatching",
    "Herding",
    "Honking",
    "Hullaballooing",
    "Hyperspacing",
    "Ideating",
    "Imagining",
    "Improvising",
    "Incubating",
    "Inferring",
    "Infusing",
    "Ionizing",
    "Jitterbugging",
    "Julienning",
    "Kneading",
    "Leavening",
    "Levitating",
    "Lollygagging",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Metamorphosing",
    "Misting",
    "Moonwalking",
    "Moseying",
    "Mulling",
    "Mustering",
    "Musing",
    "Nebulizing",
    "Nesting",
    "Newspapering",
    "Noodling",
    "Nucleating",
    "Orbiting",
    "Orchestrating",
    "Osmosing",
    "Perambulating",
    "Percolating",
    "Perusing",
    "Philosophising",
    "Photosynthesizing",
    "Pollinating",
    "Pondering",
    "Pontificating",
    "Pouncing",
    "Precipitating",
    "Prestidigitating",
    "Processing",
    "Proofing",
    "Propagating",
    "Puttering",
    "Puzzling",
    "Quantumizing",
    "Razzle-dazzling",
    "Razzmatazzing",
    "Recombobulating",
    "Reticulating",
    "Roosting",
    "Ruminating",
    "Saut\u{00e9}ing",
    "Scampering",
    "Schlepping",
    "Scurrying",
    "Seasoning",
    "Shenaniganing",
    "Shimmying",
    "Simmering",
    "Skedaddling",
    "Sketching",
    "Slithering",
    "Smooshing",
    "Sock-hopping",
    "Spelunking",
    "Spinning",
    "Sprouting",
    "Stewing",
    "Sublimating",
    "Swirling",
    "Swooping",
    "Symbioting",
    "Synthesizing",
    "Tempering",
    "Thinking",
    "Thundering",
    "Tinkering",
    "Tomfoolering",
    "Topsy-turvying",
    "Transfiguring",
    "Transmuting",
    "Twisting",
    "Undulating",
    "Unfurling",
    "Unravelling",
    "Vibing",
    "Waddling",
    "Wandering",
    "Warping",
    "Whatchamacalliting",
    "Whirlpooling",
    "Whirring",
    "Whisking",
    "Wibbling",
    "Working",
    "Wrangling",
    "Zesting",
    "Zigzagging",
    // --- Life framework additions (40) ---
    // Cognition (Arcan)
    "Arcaning",
    "Cognizing",
    "Reasoning",
    "Reconstructing",
    "Replaying",
    "Looping",
    // Persistence (Lago)
    "Journaling",
    "Appending",
    "Persisting",
    "Sourcing",
    "Hydrating",
    "Projecting",
    // Homeostasis (Autonomic)
    "Regulating",
    "Balancing",
    "Stabilizing",
    "Calibrating",
    "Adapting",
    "Homeostating",
    // Tool Execution (Praxis)
    "Sandboxing",
    "Executing",
    "Harnessing",
    "Bridging",
    // Networking (Spaces)
    "Networking",
    "Broadcasting",
    "Distributing",
    // Finance (Haima)
    "Circulating",
    "Settling",
    "Billing",
    // Observability (Vigil)
    "Observing",
    "Tracing",
    "Watching",
    // Biological / organic
    "Pulsing",
    "Breathing",
    "Gestating",
    "Metabolizing",
    "Synapsing",
    "Evolving",
    "Mutating",
    "Differentiating",
    "Mitosing",
];

/// Pick a random spinner verb.
fn pick_verb() -> &'static str {
    SPINNER_VERBS[fastrand::usize(..SPINNER_VERBS.len())]
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a duration as a human-readable string: "3.2s", "1m 12s", "1h 5m".
fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        let tenths = d.subsec_millis() / 100;
        format!("{secs}.{tenths}s")
    } else if secs < 3600 {
        let mins = secs / 60;
        let remainder = secs % 60;
        format!("{mins}m {remainder}s")
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{hours}h {mins}m")
    }
}

/// Format a token count with K/M suffix: "847", "1.2k", "2.5M".
fn format_tokens(tokens: u64) -> String {
    if tokens < 1_000 {
        format!("{tokens}")
    } else if tokens < 1_000_000 {
        let k = tokens as f64 / 1_000.0;
        if k < 10.0 {
            format!("{k:.1}k")
        } else {
            format!("{:.0}k", k)
        }
    } else {
        let m = tokens as f64 / 1_000_000.0;
        format!("{m:.1}M")
    }
}

// ---------------------------------------------------------------------------
// Spinner phases
// ---------------------------------------------------------------------------

const PHASE_THINKING: u8 = 0;
const PHASE_STREAMING: u8 = 1;
pub(crate) const PHASE_REASONING: u8 = 2;

// ---------------------------------------------------------------------------
// SpinnerState — shared between main thread and render thread
// ---------------------------------------------------------------------------

pub(crate) struct SpinnerState {
    pub(crate) phase: AtomicU8,
    pub(crate) tokens: AtomicU64,
    pub(crate) first_token_at: Mutex<Option<Instant>>,
    stop: AtomicBool,
    started_at: Instant,
    verb: String,
    tool_name: Option<String>,
}

// ---------------------------------------------------------------------------
// ShellSpinner — public API
// ---------------------------------------------------------------------------

/// Animated status line for the Arcan shell.
///
/// Start with `ShellSpinner::start()` before a provider call, then signal
/// `set_streaming()` when the first token arrives, `add_tokens()` as deltas
/// stream in, and `finish()` when the turn is complete.
///
/// When stderr is not a TTY (piped, test runner), the spinner is a no-op.
pub struct ShellSpinner {
    pub(crate) state: Arc<SpinnerState>,
    handle: Option<std::thread::JoinHandle<()>>,
    is_tty: bool,
}

impl ShellSpinner {
    /// Start the spinner for an LLM provider call. Picks a random verb.
    pub fn start() -> Self {
        let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
        let state = Arc::new(SpinnerState {
            phase: AtomicU8::new(PHASE_THINKING),
            tokens: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            started_at: Instant::now(),
            first_token_at: Mutex::new(None),
            verb: pick_verb().to_string(),
            tool_name: None,
        });

        let handle = if is_tty {
            let s = Arc::clone(&state);
            Some(std::thread::spawn(move || render_loop(&s, false)))
        } else {
            None
        };

        Self {
            state,
            handle,
            is_tty,
        }
    }

    /// Start a tool-execution spinner.
    pub fn start_tool(tool_name: &str) -> Self {
        let is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
        let state = Arc::new(SpinnerState {
            phase: AtomicU8::new(PHASE_THINKING),
            tokens: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            started_at: Instant::now(),
            first_token_at: Mutex::new(None),
            verb: "Running".to_string(),
            tool_name: Some(tool_name.to_string()),
        });

        let handle = if is_tty {
            let s = Arc::clone(&state);
            Some(std::thread::spawn(move || render_loop(&s, true)))
        } else {
            None
        };

        Self {
            state,
            handle,
            is_tty,
        }
    }

    /// Signal that streaming tokens have begun.
    #[allow(dead_code)]
    pub fn set_streaming(&self) {
        self.state.phase.store(PHASE_STREAMING, Ordering::Relaxed);
        let mut ft = self.state.first_token_at.lock().unwrap();
        if ft.is_none() {
            *ft = Some(Instant::now());
        }
    }

    /// Increment the accumulated token counter.
    #[allow(dead_code)]
    pub fn add_tokens(&self, count: u64) {
        self.state.tokens.fetch_add(count, Ordering::Relaxed);
    }

    /// Stop the spinner and print a completion summary.
    #[allow(clippy::print_stderr)]
    pub fn finish(mut self, cost: f64) {
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if self.is_tty {
            let elapsed = self.state.started_at.elapsed();
            let tokens = self.state.tokens.load(Ordering::Relaxed);
            let mut parts = vec![format_duration(elapsed)];
            if tokens > 0 {
                parts.push(format!("\u{2193} {} tokens", format_tokens(tokens)));
            }
            if cost > 0.0001 {
                parts.push(format!("${cost:.4}"));
            }
            eprint!("\r\x1b[2K");
            eprintln!("{IDLE_MARKER} Done ({})", parts.join(" \u{00b7} "));
        }
    }

    /// Stop a tool-execution spinner and print result.
    #[allow(clippy::print_stderr)]
    pub fn finish_tool(mut self, success: bool) {
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        if self.is_tty {
            let elapsed = self.state.started_at.elapsed();
            let name = self.state.tool_name.as_deref().unwrap_or("tool");
            let marker = if success {
                "\x1b[32m\u{2713}\x1b[0m"
            } else {
                "\x1b[31m\u{2717}\x1b[0m"
            };
            eprint!("\r\x1b[2K");
            eprintln!("  {marker} {name} ({})", format_duration(elapsed));
        }
    }
}

impl Drop for ShellSpinner {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Render loop (runs on background thread)
// ---------------------------------------------------------------------------

#[allow(clippy::print_stderr)]
fn render_loop(state: &Arc<SpinnerState>, is_tool: bool) {
    let glyphs = if is_tool {
        TOOL_SPINNER
    } else {
        default_glyphs()
    };
    let mut frame: usize = 0;
    let tick = Duration::from_millis(50);
    // Advance glyph every ~150ms (every 3 ticks at 50ms each).
    let frames_per_glyph: usize = 3;
    let mut tick_count: usize = 0;

    while !state.stop.load(Ordering::Relaxed) {
        tick_count += 1;
        if tick_count % frames_per_glyph == 0 {
            frame = (frame + 1) % glyphs.len();
        }

        let glyph = glyphs[frame];
        let elapsed = state.started_at.elapsed();
        let phase = state.phase.load(Ordering::Relaxed);
        let tokens = state.tokens.load(Ordering::Relaxed);

        let line = if is_tool {
            let name = state.tool_name.as_deref().unwrap_or("tool");
            format!("  {glyph} {} {name}\u{2026}", state.verb)
        } else {
            let mut status = format!(
                "{glyph} {}\u{2026} ({}",
                state.verb,
                format_duration(elapsed)
            );
            if phase == PHASE_STREAMING && tokens > 0 {
                status.push_str(&format!(
                    " \u{00b7} \u{2193} {} tokens",
                    format_tokens(tokens)
                ));
            } else if phase == PHASE_REASONING {
                status.push_str(&format!(
                    " \u{00b7} reasoning {} tokens",
                    format_tokens(tokens)
                ));
            }
            status.push(')');
            status
        };

        // Truncate to terminal width to avoid line wrapping.
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        let display = if line.len() > width {
            format!("{}\u{2026}", &line[..width.saturating_sub(1)])
        } else {
            line
        };

        let mut stderr = std::io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K{display}");
        let _ = stderr.flush();

        std::thread::sleep(tick);
    }

    // Clear the spinner line on exit.
    let mut stderr = std::io::stderr().lock();
    let _ = write!(stderr, "\r\x1b[2K");
    let _ = stderr.flush();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Glyph tests ---

    #[test]
    fn neural_pulse_has_10_frames() {
        assert_eq!(NEURAL_PULSE.len(), 10);
    }

    #[test]
    fn tool_spinner_has_10_frames() {
        assert_eq!(TOOL_SPINNER.len(), 10);
    }

    #[test]
    fn default_glyphs_returns_non_empty() {
        assert!(!default_glyphs().is_empty());
    }

    #[test]
    fn glyph_frame_wraps_around() {
        let glyphs = NEURAL_PULSE;
        for i in 0..30 {
            let frame = i % glyphs.len();
            let _ = glyphs[frame];
        }
    }

    // --- Verb tests ---

    #[test]
    fn verb_list_has_expected_count() {
        assert!(
            SPINNER_VERBS.len() >= 220,
            "Expected 220+ verbs, got {}",
            SPINNER_VERBS.len()
        );
    }

    #[test]
    fn pick_verb_returns_valid_verb() {
        let verb = pick_verb();
        assert!(!verb.is_empty());
        assert!(SPINNER_VERBS.contains(&verb));
    }

    #[test]
    fn all_verbs_are_non_empty() {
        for verb in SPINNER_VERBS {
            assert!(!verb.is_empty());
            assert!(verb.len() >= 4, "Verb '{verb}' is too short");
        }
    }

    // --- Duration formatting tests ---

    #[test]
    fn format_duration_sub_minute() {
        assert_eq!(format_duration(Duration::from_millis(3200)), "3.2s");
        assert_eq!(format_duration(Duration::from_millis(500)), "0.5s");
        assert_eq!(format_duration(Duration::from_secs(0)), "0.0s");
        assert_eq!(format_duration(Duration::from_secs(59)), "59.0s");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(Duration::from_secs(72)), "1m 12s");
        assert_eq!(format_duration(Duration::from_secs(600)), "10m 0s");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(Duration::from_secs(3661)), "1h 1m");
    }

    // --- Token formatting tests ---

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(847), "847");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(1_200), "1.2k");
        assert_eq!(format_tokens(5_000), "5.0k");
        assert_eq!(format_tokens(15_000), "15k");
        assert_eq!(format_tokens(999_999), "1000k");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(1_500_000), "1.5M");
        assert_eq!(format_tokens(2_000_000), "2.0M");
    }

    // --- ShellSpinner lifecycle tests ---

    #[test]
    fn spinner_start_and_finish_does_not_panic() {
        let spinner = ShellSpinner::start();
        std::thread::sleep(Duration::from_millis(100));
        spinner.finish(0.001);
    }

    #[test]
    fn spinner_set_streaming_and_add_tokens() {
        let spinner = ShellSpinner::start();
        spinner.set_streaming();
        spinner.add_tokens(100);
        spinner.add_tokens(200);
        assert_eq!(spinner.state.tokens.load(Ordering::Relaxed), 300);
        spinner.finish(0.0);
    }

    #[test]
    fn spinner_finish_tool() {
        let spinner = ShellSpinner::start_tool("bash");
        std::thread::sleep(Duration::from_millis(50));
        spinner.finish_tool(true);
    }

    #[test]
    fn spinner_finish_tool_failure() {
        let spinner = ShellSpinner::start_tool("write_file");
        spinner.finish_tool(false);
    }

    #[test]
    fn spinner_is_noop_when_not_tty() {
        // In test context, stderr is not a TTY.
        let spinner = ShellSpinner::start();
        assert!(!spinner.is_tty);
        spinner.set_streaming();
        spinner.add_tokens(500);
        spinner.finish(0.05);
    }

    #[test]
    fn spinner_drop_stops_cleanly() {
        let spinner = ShellSpinner::start();
        spinner.add_tokens(10);
        drop(spinner); // should not hang or panic
    }
}
