# Autonomic Context Regulation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hard-threshold context compaction with Autonomic-regulated compression that uses cognitive state, Nous quality signals, and hysteresis gating to decide when to compress, dilate, or hold.

**Architecture:** The shell measures context pressure after each turn and updates `CognitiveState` in a local `HomeostaticState`. The Autonomic controller's `ContextCompressionRule` evaluates this state against Nous quality trends, tool density, and error streaks to produce a `ContextRuling` (Breathe/Dilate/Compress/Emergency). A `HysteresisGate` prevents flapping between Dilate and Compress. The shell acts on the ruling — no threshold constants needed.

**Tech Stack:** Rust 2024, autonomic-core (types), autonomic-controller (rule engine), arcan shell (integration)

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `autonomic/crates/autonomic-core/src/gating.rs` | Modify | Add `tool_density`, `turns_since_compact` to `CognitiveState` |
| `autonomic/crates/autonomic-core/src/context.rs` | Create | `ContextRuling` enum + `ContextCompressionAdvice` struct |
| `autonomic/crates/autonomic-core/src/lib.rs` | Modify | Export `context` module |
| `autonomic/crates/autonomic-controller/src/cognitive_rules.rs` | Modify | Rewrite `ContextPressureRule` to return `ContextCompressionAdvice` via `GatingDecision.metadata` |
| `arcan/crates/arcan/src/shell.rs` | Modify | Replace hard threshold with Autonomic evaluation loop |

---

### Task 1: Extend CognitiveState with compression signals

**Files:**
- Modify: `autonomic/crates/autonomic-core/src/gating.rs` (CognitiveState struct, lines 81-101)

- [ ] **Step 1: Write the failing test**

In `autonomic/crates/autonomic-core/src/gating.rs`, add to the `tests` module:

```rust
#[test]
fn cognitive_state_has_compression_signals() {
    let mut cog = CognitiveState::default();
    assert_eq!(cog.tool_density, 0.0);
    assert_eq!(cog.turns_since_compact, 0);
    cog.tool_density = 3.5;
    cog.turns_since_compact = 12;
    assert!((cog.tool_density - 3.5).abs() < f64::EPSILON);
    assert_eq!(cog.turns_since_compact, 12);
}

#[test]
fn cognitive_state_compression_signals_serde() {
    let cog = CognitiveState {
        tool_density: 2.5,
        turns_since_compact: 8,
        ..Default::default()
    };
    let json = serde_json::to_string(&cog).unwrap();
    let back: CognitiveState = serde_json::from_str(&json).unwrap();
    assert!((back.tool_density - 2.5).abs() < f64::EPSILON);
    assert_eq!(back.turns_since_compact, 8);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-core -- cognitive_state_has_compression`
Expected: FAIL — `tool_density` and `turns_since_compact` don't exist

- [ ] **Step 3: Add fields to CognitiveState**

In `autonomic/crates/autonomic-core/src/gating.rs`, modify `CognitiveState`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveState {
    /// Total tokens consumed in the session.
    pub total_tokens_used: u64,
    /// Tokens remaining from budget.
    pub tokens_remaining: u64,
    /// Context pressure (0.0 = empty, 1.0 = full).
    pub context_pressure: f32,
    /// Number of model turns completed.
    pub turns_completed: u32,
    /// Average tool calls per turn (rolling window). High = active implementation.
    pub tool_density: f64,
    /// Turns elapsed since last compaction. High = stale old context.
    pub turns_since_compact: u32,
}

impl Default for CognitiveState {
    fn default() -> Self {
        Self {
            total_tokens_used: 0,
            tokens_remaining: 120_000,
            context_pressure: 0.0,
            turns_completed: 0,
            tool_density: 0.0,
            turns_since_compact: 0,
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-core -- cognitive_state_has_compression`
Expected: PASS

- [ ] **Step 5: Verify full workspace compiles**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo check --workspace && cd /Users/broomva/broomva/core/life/arcan && cargo check --workspace`
Expected: both pass (new fields have defaults, no breakage)

- [ ] **Step 6: Commit**

```bash
cd /Users/broomva/broomva/core/life/autonomic
git add crates/autonomic-core/src/gating.rs
git commit -m "feat(autonomic-core): add tool_density and turns_since_compact to CognitiveState"
```

---

### Task 2: Create ContextRuling enum and ContextCompressionAdvice

**Files:**
- Create: `autonomic/crates/autonomic-core/src/context.rs`
- Modify: `autonomic/crates/autonomic-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `autonomic/crates/autonomic-core/src/context.rs` with tests first:

```rust
//! Context compression regulation types.
//!
//! `ContextRuling` is the Autonomic controller's decision about whether
//! to compress, dilate, or hold the conversation context. It replaces
//! hard-coded token thresholds with a regulated control signal.

use serde::{Deserialize, Serialize};

/// The Autonomic controller's ruling on context compression.
///
/// Maps to biological analogy: breathing. The context window is a lung —
/// it fills (inspiration) and must periodically release (expiration).
/// Autonomic regulation decides the breathing rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRuling {
    /// Context pressure is low. No action needed.
    Breathe,
    /// Context is filling but the agent is doing valuable work.
    /// Delay compression — push threshold higher to preserve working context.
    Dilate,
    /// Context should be compressed. Extract memories and compact.
    Compress,
    /// Critical pressure. Compact immediately to avoid API errors.
    Emergency,
}

/// Advice package returned by the ContextCompressionRule.
///
/// Contains the ruling plus the rationale signals that informed it,
/// so the shell can log why a particular decision was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompressionAdvice {
    /// The ruling: what the shell should do.
    pub ruling: ContextRuling,
    /// Context pressure that triggered evaluation (0.0..1.0).
    pub pressure: f32,
    /// Target token count if compression is needed.
    pub target_tokens: Option<usize>,
    /// Human-readable rationale for the ruling.
    pub rationale: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_ruling_serde_roundtrip() {
        for ruling in [
            ContextRuling::Breathe,
            ContextRuling::Dilate,
            ContextRuling::Compress,
            ContextRuling::Emergency,
        ] {
            let json = serde_json::to_string(&ruling).unwrap();
            let back: ContextRuling = serde_json::from_str(&json).unwrap();
            assert_eq!(ruling, back);
        }
    }

    #[test]
    fn context_compression_advice_serde_roundtrip() {
        let advice = ContextCompressionAdvice {
            ruling: ContextRuling::Dilate,
            pressure: 0.68,
            target_tokens: None,
            rationale: "high tool density, quality stable".into(),
        };
        let json = serde_json::to_string(&advice).unwrap();
        let back: ContextCompressionAdvice = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ruling, ContextRuling::Dilate);
        assert!((back.pressure - 0.68).abs() < f32::EPSILON);
        assert!(back.target_tokens.is_none());
    }

    #[test]
    fn compress_advice_has_target() {
        let advice = ContextCompressionAdvice {
            ruling: ContextRuling::Compress,
            pressure: 0.75,
            target_tokens: Some(70_000),
            rationale: "quality degrading".into(),
        };
        assert_eq!(advice.target_tokens, Some(70_000));
    }
}
```

- [ ] **Step 2: Add module to lib.rs**

In `autonomic/crates/autonomic-core/src/lib.rs`, add:

```rust
pub mod context;
```

And add to the existing re-exports:

```rust
pub use context::{ContextCompressionAdvice, ContextRuling};
```

- [ ] **Step 3: Run tests**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-core -- context_ruling`
Expected: PASS (3 tests)

- [ ] **Step 4: Commit**

```bash
cd /Users/broomva/broomva/core/life/autonomic
git add crates/autonomic-core/src/context.rs crates/autonomic-core/src/lib.rs
git commit -m "feat(autonomic-core): add ContextRuling enum and ContextCompressionAdvice"
```

---

### Task 3: Rewrite ContextPressureRule with soft-zone evaluation

**Files:**
- Modify: `autonomic/crates/autonomic-controller/src/cognitive_rules.rs`

The existing `ContextPressureRule` fires at a single threshold. Replace it with a multi-zone rule that evaluates Nous quality signals and tool density in the soft zone (60-85%).

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `cognitive_rules.rs`:

```rust
use autonomic_core::context::ContextRuling;

#[test]
fn context_pressure_low_returns_breathe() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.40;
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Breathe);
}

#[test]
fn context_pressure_soft_zone_high_tool_density_dilates() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.70;
    state.cognitive.tool_density = 3.0; // active implementation
    state.eval.aggregate_quality_score = 0.85; // quality good
    state.eval.quality_trend = 0.01; // improving
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Dilate);
}

#[test]
fn context_pressure_soft_zone_degrading_quality_compresses() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.70;
    state.cognitive.tool_density = 0.5; // casual Q&A
    state.eval.aggregate_quality_score = 0.60; // quality poor
    state.eval.quality_trend = -0.05; // degrading
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Compress);
    assert!(advice.target_tokens.is_some());
}

#[test]
fn context_pressure_hard_zone_always_compresses() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.88;
    state.cognitive.tool_density = 5.0; // even with high tool density
    state.eval.aggregate_quality_score = 0.95; // even with great quality
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Compress);
}

#[test]
fn context_pressure_emergency_zone() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.96;
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Emergency);
}

#[test]
fn context_pressure_soft_zone_many_turns_since_compact_compresses() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.65;
    state.cognitive.turns_since_compact = 20; // stale
    state.cognitive.tool_density = 0.2; // casual
    state.eval.quality_trend = -0.01; // slightly degrading
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Compress);
}

#[test]
fn context_pressure_soft_zone_error_streak_compresses() {
    let rule = ContextPressureRule::default();
    let mut state = HomeostaticState::for_agent("test");
    state.cognitive.context_pressure = 0.70;
    state.operational.error_streak = 3; // errors may indicate confused context
    let advice = rule.evaluate_compression(&state);
    assert_eq!(advice.ruling, ContextRuling::Compress);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-controller -- context_pressure`
Expected: FAIL — `evaluate_compression` doesn't exist

- [ ] **Step 3: Implement the multi-zone ContextPressureRule**

Replace the existing `ContextPressureRule` in `cognitive_rules.rs`:

```rust
use autonomic_core::context::{ContextCompressionAdvice, ContextRuling};
use autonomic_core::ModelTier;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Context pressure rule with multi-zone evaluation.
///
/// Zones:
/// - Below `soft_threshold` (0.60): Breathe — no action.
/// - `soft_threshold` to `hard_threshold` (0.60..0.85): Soft zone —
///   evaluate Nous quality, tool density, error streak to decide
///   Dilate vs. Compress.
/// - `hard_threshold` to `emergency_threshold` (0.85..0.95): Hard zone —
///   always Compress.
/// - Above `emergency_threshold` (0.95): Emergency — compact aggressively.
pub struct ContextPressureRule {
    /// Pressure below which no action is taken.
    pub soft_threshold: f32,
    /// Pressure above which compression is forced.
    pub hard_threshold: f32,
    /// Pressure above which emergency compaction triggers.
    pub emergency_threshold: f32,
    /// Target pressure after compression (as fraction of context window).
    pub target_fraction: f32,
    /// Target pressure after emergency compression.
    pub emergency_target_fraction: f32,
    /// Tool density above which the agent is considered to be in "deep work".
    pub deep_work_tool_density: f64,
    /// Error streak above which context is considered confused.
    pub error_streak_limit: u32,
    /// Turns since compact above which old context is considered stale.
    pub stale_turns_limit: u32,
}

impl Default for ContextPressureRule {
    fn default() -> Self {
        Self {
            soft_threshold: 0.60,
            hard_threshold: 0.85,
            emergency_threshold: 0.95,
            target_fraction: 0.35,
            emergency_target_fraction: 0.25,
            deep_work_tool_density: 2.0,
            error_streak_limit: 2,
            stale_turns_limit: 15,
        }
    }
}

impl ContextPressureRule {
    pub fn new(soft: f32, hard: f32, emergency: f32) -> Self {
        Self {
            soft_threshold: soft,
            hard_threshold: hard,
            emergency_threshold: emergency,
            ..Default::default()
        }
    }

    /// Evaluate context pressure and return compression advice.
    ///
    /// This is the primary method — richer than `HomeostaticRule::evaluate`
    /// which can only return a `GatingDecision`.
    pub fn evaluate_compression(&self, state: &HomeostaticState) -> ContextCompressionAdvice {
        let pressure = state.cognitive.context_pressure;

        // Zone 1: Below soft threshold — breathe
        if pressure < self.soft_threshold {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Breathe,
                pressure,
                target_tokens: None,
                rationale: format!("pressure {:.0}% below soft threshold {:.0}%",
                    pressure * 100.0, self.soft_threshold * 100.0),
            };
        }

        // Zone 4: Emergency
        if pressure >= self.emergency_threshold {
            let target = (state.cognitive.tokens_remaining + state.cognitive.total_tokens_used)
                as f32 * self.emergency_target_fraction;
            return ContextCompressionAdvice {
                ruling: ContextRuling::Emergency,
                pressure,
                target_tokens: Some(target as usize),
                rationale: format!("pressure {:.0}% — emergency compaction", pressure * 100.0),
            };
        }

        // Zone 3: Hard zone — always compress
        if pressure >= self.hard_threshold {
            let target = (state.cognitive.tokens_remaining + state.cognitive.total_tokens_used)
                as f32 * self.target_fraction;
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target as usize),
                rationale: format!("pressure {:.0}% exceeds hard threshold {:.0}%",
                    pressure * 100.0, self.hard_threshold * 100.0),
            };
        }

        // Zone 2: Soft zone (soft_threshold..hard_threshold) — evaluate signals
        let total_ctx = (state.cognitive.tokens_remaining + state.cognitive.total_tokens_used) as f32;
        let target = (total_ctx * self.target_fraction) as usize;

        // Signal 1: Error streak indicates confused context
        if state.operational.error_streak >= self.error_streak_limit {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + {} consecutive errors — context may be confusing model",
                    pressure * 100.0, state.operational.error_streak
                ),
            };
        }

        // Signal 2: Quality degrading — compress to help model focus
        if state.eval.quality_trend < -0.02 || state.eval.aggregate_quality_score < 0.65 {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + quality degrading (score={:.2}, trend={:.3})",
                    pressure * 100.0, state.eval.aggregate_quality_score, state.eval.quality_trend
                ),
            };
        }

        // Signal 3: Stale context with low tool activity — compress
        if state.cognitive.turns_since_compact >= self.stale_turns_limit
            && state.cognitive.tool_density < self.deep_work_tool_density
        {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + {} turns since compact (stale, low tool activity)",
                    pressure * 100.0, state.cognitive.turns_since_compact
                ),
            };
        }

        // Signal 4: High tool density + good quality — dilate (deep work)
        if state.cognitive.tool_density >= self.deep_work_tool_density
            && state.eval.quality_trend >= 0.0
        {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Dilate,
                pressure,
                target_tokens: None,
                rationale: format!(
                    "pressure {:.0}% but deep work (tool_density={:.1}, quality stable) — dilating",
                    pressure * 100.0, state.cognitive.tool_density
                ),
            };
        }

        // Signal 5: Quality stable/improving — dilate
        if state.eval.quality_trend >= 0.0 && state.eval.aggregate_quality_score >= 0.75 {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Dilate,
                pressure,
                target_tokens: None,
                rationale: format!(
                    "pressure {:.0}% but quality good (score={:.2}, trend={:.3}) — dilating",
                    pressure * 100.0, state.eval.aggregate_quality_score, state.eval.quality_trend
                ),
            };
        }

        // Default in soft zone: hold (treated as breathe — no action)
        ContextCompressionAdvice {
            ruling: ContextRuling::Breathe,
            pressure,
            target_tokens: None,
            rationale: format!(
                "pressure {:.0}% in soft zone, signals inconclusive — holding",
                pressure * 100.0
            ),
        }
    }
}

// Keep HomeostaticRule impl for compatibility with the engine's evaluate_all.
impl HomeostaticRule for ContextPressureRule {
    fn rule_id(&self) -> &str {
        "context_pressure"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let advice = self.evaluate_compression(state);
        match advice.ruling {
            ContextRuling::Breathe | ContextRuling::Dilate => None,
            ContextRuling::Compress | ContextRuling::Emergency => {
                Some(GatingDecision {
                    rule_id: self.rule_id().into(),
                    preferred_model: Some(ModelTier::Standard),
                    max_tokens_next_turn: Some(2048),
                    rationale: advice.rationale,
                    ..GatingDecision::noop(self.rule_id())
                })
            }
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-controller -- context_pressure`
Expected: PASS (all 7 new tests + 2 existing)

- [ ] **Step 5: Run full workspace check**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
cd /Users/broomva/broomva/core/life/autonomic
git add crates/autonomic-controller/src/cognitive_rules.rs
git commit -m "feat(autonomic-controller): multi-zone ContextPressureRule with quality-aware soft zone"
```

---

### Task 4: Add HysteresisGate for context dilation

**Files:**
- Modify: `autonomic/crates/autonomic-core/src/gating.rs` (add field to CognitiveState)

- [ ] **Step 1: Write the failing test**

In `autonomic-core/src/gating.rs` tests:

```rust
#[test]
fn cognitive_state_has_dilation_gate() {
    let cog = CognitiveState::default();
    // Dilation gate: enter at 0.60, exit at 0.45, min-hold 3 turns (0ms for instant)
    assert!(!cog.dilation_gate.active);
    assert!((cog.dilation_gate.threshold_enter - 0.60).abs() < f64::EPSILON);
    assert!((cog.dilation_gate.threshold_exit - 0.45).abs() < f64::EPSILON);
}
```

- [ ] **Step 2: Add dilation_gate field to CognitiveState**

```rust
use crate::hysteresis::HysteresisGate;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveState {
    pub total_tokens_used: u64,
    pub tokens_remaining: u64,
    pub context_pressure: f32,
    pub turns_completed: u32,
    pub tool_density: f64,
    pub turns_since_compact: u32,
    /// Hysteresis gate for context dilation — prevents flapping between
    /// Dilate and Compress decisions in the soft zone.
    pub dilation_gate: HysteresisGate,
}

impl Default for CognitiveState {
    fn default() -> Self {
        Self {
            total_tokens_used: 0,
            tokens_remaining: 120_000,
            context_pressure: 0.0,
            turns_completed: 0,
            tool_density: 0.0,
            turns_since_compact: 0,
            // Dilation gate: enters dilation at 60% pressure, exits at 45%.
            // min_hold_ms=0 because the shell tracks turns, not wall-clock time.
            dilation_gate: HysteresisGate::new(0.60, 0.45, 0),
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo test -p autonomic-core -- cognitive_state_has_dilation`
Expected: PASS

- [ ] **Step 4: Verify workspace**

Run: `cd /Users/broomva/broomva/core/life/autonomic && cargo check --workspace && cd /Users/broomva/broomva/core/life/arcan && cargo check --workspace`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /Users/broomva/broomva/core/life/autonomic
git add crates/autonomic-core/src/gating.rs
git commit -m "feat(autonomic-core): add HysteresisGate for context dilation to CognitiveState"
```

---

### Task 5: Wire Autonomic context regulation into arcan shell

**Files:**
- Modify: `arcan/crates/arcan/src/shell.rs`

This is the integration task. The shell currently has a hard threshold at lines ~1580-1595. Replace it with:
1. Update `CognitiveState` after each turn
2. Evaluate `ContextPressureRule`
3. Act on the `ContextRuling`

- [ ] **Step 1: Add autonomic-core and autonomic-controller deps to arcan binary Cargo.toml**

Check if they're already in workspace deps (they should be — arcan already uses autonomic-core via arcan-aios-adapters). If not, add:

```toml
autonomic-core.workspace = true
autonomic-controller.workspace = true
```

- [ ] **Step 2: Add imports to shell.rs**

At the top of `shell.rs`, add:

```rust
use autonomic_core::context::ContextRuling;
use autonomic_core::gating::{CognitiveState, HomeostaticState};
use autonomic_controller::ContextPressureRule;
```

- [ ] **Step 3: Initialize HomeostaticState and ContextPressureRule in run_shell**

After the provider is built (~line 935), add:

```rust
// --- Autonomic context regulation ---
let context_rule = ContextPressureRule::default();
let context_window = provider.context_window().unwrap_or(200_000) as usize;
let mut homeostatic_state = HomeostaticState::for_agent(&lago_session_id.to_string());
// Initialize cognitive state with actual context window
homeostatic_state.cognitive.tokens_remaining = context_window as u64;
```

- [ ] **Step 4: Replace the hard-threshold auto-compact block**

Replace the block at ~lines 1580-1595 (the `let tokens = estimate_tokens(...)` block) with:

```rust
// --- Autonomic-regulated context compression ---
let tokens = estimate_tokens(&messages);
let pressure = tokens as f32 / context_window as f32;

// Update cognitive state for Autonomic evaluation
homeostatic_state.cognitive.context_pressure = pressure;
homeostatic_state.cognitive.total_tokens_used = cmd_ctx.session_input_tokens + cmd_ctx.session_output_tokens;
homeostatic_state.cognitive.tokens_remaining = context_window.saturating_sub(tokens) as u64;
homeostatic_state.cognitive.turns_completed += 1;
homeostatic_state.cognitive.turns_since_compact += 1;
homeostatic_state.cognitive.tool_density = if homeostatic_state.cognitive.turns_completed > 0 {
    cmd_ctx.tool_call_count as f64 / homeostatic_state.cognitive.turns_completed as f64
} else {
    0.0
};

// Feed Nous quality scores into homeostatic state
// (eval scores are already accumulated by nous_observer — read from cmd_ctx)
homeostatic_state.operational.error_streak = 0; // reset on success (set on error below)

// Evaluate context pressure rule
let advice = context_rule.evaluate_compression(&homeostatic_state);

match advice.ruling {
    ContextRuling::Breathe => {
        // No action — context is healthy
    }
    ContextRuling::Dilate => {
        // Log dilation — context is filling but agent is doing valuable work
        tracing::info!(
            pressure = format!("{:.0}%", pressure * 100.0),
            rationale = %advice.rationale,
            "context: dilating (preserving working context)"
        );
    }
    ContextRuling::Compress => {
        let target = advice.target_tokens.unwrap_or(context_window * 35 / 100);
        eprintln!("[context] {tokens} tokens ({:.0}%) — compressing to ~{target} ({advice_rationale})",
            pressure * 100.0, advice_rationale = advice.rationale);
        compact_with_extraction(&mut messages, &memory_dir, target);
        homeostatic_state.cognitive.turns_since_compact = 0;
        let after = estimate_tokens(&messages);
        eprintln!("[context] now ~{after} tokens (memories extracted)");
    }
    ContextRuling::Emergency => {
        let target = advice.target_tokens.unwrap_or(context_window * 25 / 100);
        eprintln!("[context] EMERGENCY {tokens} tokens ({:.0}%) — compacting to ~{target}",
            pressure * 100.0);
        compact_with_extraction(&mut messages, &memory_dir, target);
        homeostatic_state.cognitive.turns_since_compact = 0;
        let after = estimate_tokens(&messages);
        eprintln!("[context] now ~{after} tokens");
    }
}
```

Also, in the error handler (the `Err(e)` branch at ~line 1571), increment the error streak:

```rust
Err(e) => {
    eprintln!("Error: {e}");
    homeostatic_state.operational.error_streak += 1;
}
```

- [ ] **Step 5: Remove old constants**

Remove these constants from shell.rs (no longer needed):

```rust
// DELETE:
// const COMPACT_THRESHOLD_PCT: usize = 60;
// const COMPACT_TARGET_PCT: usize = 35;
// const DEFAULT_CONTEXT_WINDOW: usize = 200_000;
```

- [ ] **Step 6: Verify compilation**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo check --workspace`
Expected: PASS

- [ ] **Step 7: Run all tests**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo test -p arcan-core -p arcan-provider`
Expected: PASS

- [ ] **Step 8: Integration test with apfel**

```bash
# Ensure apfel is running
apfel --serve --port 11435 &>/tmp/apfel-server.log &
sleep 3

SESSION="autonomic-ctx-$(date +%s)"
ARCAN_PROVIDER=apfel cargo run -p arcan -- --bare shell --session "$SESSION" <<'INPUT'
What is a linked list?
Explain binary search trees.
What is the difference between a stack and a queue?
Describe hash tables and their collision handling.
What is a red-black tree?
Explain B-trees and their use in databases.
What is dynamic programming?
Give an example of memoization.
What is the time complexity of quicksort?
Describe merge sort step by step.
INPUT
```

Verify: compaction messages show Autonomic rulings (Breathe/Dilate/Compress), not hard thresholds.

- [ ] **Step 9: Commit**

```bash
cd /Users/broomva/broomva/core/life/arcan
git add crates/arcan/src/shell.rs
git commit -m "feat(arcan): replace hard context thresholds with Autonomic-regulated compression

Context compression is now driven by ContextPressureRule from the
Autonomic controller. Four zones (Breathe/Dilate/Compress/Emergency)
replace the old 60%/35% hard thresholds. The soft zone (60-85%)
evaluates Nous quality trends, tool density, error streaks, and
turn staleness to decide whether to preserve context (Dilate) or
compress (Compress)."
```

---

### Task 6: Wire Nous eval scores into HomeostateState

**Files:**
- Modify: `arcan/crates/arcan/src/shell.rs`

The Nous evaluator registry already runs after each turn via `nous_observer`. We need to feed its aggregate scores back into `homeostatic_state.eval`.

- [ ] **Step 1: Find where Nous scores are collected in the shell**

Search for `nous_registry` and `eval_scores` usage in shell.rs to understand the current flow. The Nous middleware runs via `on_run_finished` hook.

- [ ] **Step 2: After the agent loop returns, update eval state**

After the `match response_text { Ok(text) => { ... } }` block, add:

```rust
// Feed Nous quality signal into Autonomic homeostatic state
if let Some(ref nous) = nous_registry {
    let eval_ctx = nous_core::EvalContext {
        session_id: lago_session_id.to_string(),
        input_tokens: Some(cmd_ctx.session_input_tokens),
        output_tokens: Some(cmd_ctx.session_output_tokens),
        tool_call_count: Some(cmd_ctx.tool_call_count as u32),
        ..nous_core::EvalContext::new(&lago_session_id.to_string())
    };
    if let Ok(scores) = nous.evaluate_all(&eval_ctx, nous_core::EvalHook::OnRunFinished) {
        if !scores.is_empty() {
            let avg = scores.iter().map(|s| s.value).sum::<f64>() / scores.len() as f64;
            let prev = homeostatic_state.eval.aggregate_quality_score;
            // Exponential moving average (alpha=0.3)
            homeostatic_state.eval.aggregate_quality_score = prev * 0.7 + avg * 0.3;
            homeostatic_state.eval.quality_trend = homeostatic_state.eval.aggregate_quality_score - prev;
            homeostatic_state.eval.inline_eval_count += scores.len() as u32;
        }
    }
}
```

- [ ] **Step 3: Verify compilation and test**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo check --workspace`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
cd /Users/broomva/broomva/core/life/arcan
git add crates/arcan/src/shell.rs
git commit -m "feat(arcan): feed Nous eval scores into Autonomic homeostatic state for context regulation"
```

---

### Task 7: Update /context command to show Autonomic ruling

**Files:**
- Modify: `arcan/crates/arcan-commands/src/lib.rs` (or wherever /context is defined)

- [ ] **Step 1: Find the /context command handler**

Search for `CompactRequested` or `context` command in `arcan-commands`. The `/context` command already shows token breakdowns — add the current Autonomic ruling.

- [ ] **Step 2: Add ContextRuling display to the output**

Add a line showing:
```
Autonomic ruling: Dilate (pressure 68%, quality 0.85, tool_density 3.2)
```

This requires passing the latest `ContextCompressionAdvice` into the command context. Add a field to `CommandContext`:

```rust
pub context_ruling: Option<String>,
```

Set it after each Autonomic evaluation in the main loop.

- [ ] **Step 3: Test manually**

Run arcan shell with apfel, have a few turns, type `/context`, verify the ruling appears.

- [ ] **Step 4: Commit**

```bash
cd /Users/broomva/broomva/core/life/arcan
git add crates/arcan-commands/src/lib.rs crates/arcan/src/shell.rs
git commit -m "feat(arcan): show Autonomic context ruling in /context command"
```
