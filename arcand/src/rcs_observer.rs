//! RCS Level 1 runtime observer — wires the daemon's live `HomeostaticState`
//! into a `MarginEstimator` and emits the `life.control_margin_l1` gauge via
//! `life_vigil::GenAiMetrics`.
//!
//! This closes the F2 instrumentation loop by keeping the authoritative L1
//! state inside the daemon (not reconstructed by tests or other crates) and
//! publishing the derived stability margin through the same OTel pipeline
//! used for budget, token, and cost metrics. See
//! `research/rcs/latex/rcs-definitions.tex` Theorem 1 and the F2 PR (#802).
//!
//! The observer is deliberately minimal:
//!
//! - One `HomeostaticState` per session consciousness actor (L1 scope).
//! - One `MarginEstimator::for_l1` baseline, captured at construction.
//! - One `GenAiMetrics` handle so the margin emission path matches the
//!   established vigil semantics (see `crates/vigil/life-vigil/src/metrics.rs`).
//!
//! Callers drive the observer from the `CycleCompleted` handler in
//! `consciousness::SessionConsciousness`, mirroring the production path in
//! `canonical.rs::AutonomicDaemonState::evaluate_after_run`. Tests get a
//! snapshot via `snapshot()` and can compute the margin directly off the
//! observer's internal state, not a parallel reconstruction.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use aios_protocol::AgentStateVector;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rcs_budget::{MarginEstimator, StabilityBudget};
use life_vigil::GenAiMetrics;

/// Shared, lock-guarded RCS observer for a single consciousness actor.
pub type SharedRcsObserver = Arc<Mutex<RcsObserver>>;

/// Snapshot of the observer's state at a point in time.
///
/// Returned by [`RcsObserver::snapshot`] so callers can assert on the
/// `HomeostaticState` and the most recently computed margin without holding
/// the observer lock across `.await` boundaries.
#[derive(Debug, Clone)]
pub struct RcsObservationSnapshot {
    /// Current in-daemon homeostatic state (the authoritative source).
    pub homeostatic: HomeostaticState,
    /// Most recent budget estimate (`None` until the first `record_cycle`).
    pub last_budget: Option<StabilityBudget>,
    /// Number of cycles folded into the estimator.
    pub event_count: u64,
    /// Cumulative observation window (ms).
    pub window_ms: u64,
}

impl RcsObservationSnapshot {
    /// Convenience: compute the margin from `last_budget`, if any.
    pub fn last_margin(&self) -> Option<f64> {
        self.last_budget.as_ref().map(StabilityBudget::margin)
    }
}

/// Runtime observer for Level 1 (autonomic) of the RCS hierarchy.
///
/// Owns a `HomeostaticState` that is updated after each agent cycle and a
/// `MarginEstimator` that folds those updates into a `StabilityBudget`.
///
/// The observer also owns a `GenAiMetrics` handle so every `record_cycle`
/// emits the derived margin to the `life.control_margin_l1` gauge. When OTel
/// is not configured, the gauge is a no-op — emission is always safe.
pub struct RcsObserver {
    homeostatic: HomeostaticState,
    estimator: MarginEstimator,
    last_budget: Option<StabilityBudget>,
    metrics: GenAiMetrics,
}

impl RcsObserver {
    /// Build a new L1 observer for the given session id.
    ///
    /// The initial `HomeostaticState::for_agent(session_id)` seeds both the
    /// observer's live state and the estimator's baseline.
    pub fn new_l1(session_id: &str) -> Self {
        let homeostatic = HomeostaticState::for_agent(session_id);
        let estimator = MarginEstimator::for_l1(homeostatic.clone());
        Self {
            homeostatic,
            estimator,
            last_budget: None,
            metrics: GenAiMetrics::new("arcan"),
        }
    }

    /// Build a shared `Arc<Mutex<RcsObserver>>` in one call.
    pub fn shared(session_id: &str) -> SharedRcsObserver {
        Arc::new(Mutex::new(Self::new_l1(session_id)))
    }

    /// Fold the result of a completed agent cycle into the observer.
    ///
    /// This mirrors `AutonomicDaemonState::evaluate_after_run`: the cognitive
    /// pillar absorbs `context_pressure` and `tokens_remaining` from the tick
    /// output, and the operational pillar bumps `total_successes` /
    /// `last_tick_ms` so the estimator can measure an inter-event gap.
    ///
    /// After the state is updated the estimator observes it, a fresh
    /// `StabilityBudget` is computed, and `life.control_margin_l1` is
    /// recorded. Returns the budget so callers can log or assert on it.
    pub fn record_cycle(&mut self, state: &AgentStateVector) -> StabilityBudget {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);

        // ── Cognitive: mirror what ContextPressureRule consumes ────────────
        self.homeostatic.cognitive.context_pressure = state.context_pressure;
        self.homeostatic.cognitive.tokens_remaining = state.budget.tokens_remaining;
        self.homeostatic.cognitive.turns_completed =
            self.homeostatic.cognitive.turns_completed.saturating_add(1);
        self.homeostatic.cognitive.observation_count = self
            .homeostatic
            .cognitive
            .observation_count
            .saturating_add(1);

        // ── Operational: advance the event clock and success counter ──────
        self.homeostatic.operational.last_tick_ms = now_ms;
        self.homeostatic.operational.total_successes = self
            .homeostatic
            .operational
            .total_successes
            .saturating_add(1);

        // ── Eval: translate AgentStateVector.uncertainty into quality ─────
        self.homeostatic.eval.aggregate_quality_score =
            f64::from(1.0 - state.uncertainty).clamp(0.0, 1.0);

        // ── Event bookkeeping for the estimator's window arithmetic ───────
        self.homeostatic.last_event_ms = now_ms;
        self.homeostatic.last_event_seq = self.homeostatic.last_event_seq.saturating_add(1);

        // Fold and estimate.
        self.estimator.observe(&self.homeostatic);
        let budget = self.estimator.estimate();
        self.last_budget = Some(budget);

        // Emit to vigil (OTel). `record_control_margin` validates finiteness
        // and silently ignores non-finite values, so this is always safe.
        self.metrics.record_control_margin("L1", budget.margin());

        budget
    }

    /// Read-only accessor for the live daemon state.
    pub fn homeostatic(&self) -> &HomeostaticState {
        &self.homeostatic
    }

    /// Most recent budget (`None` before the first cycle).
    pub fn last_budget(&self) -> Option<&StabilityBudget> {
        self.last_budget.as_ref()
    }

    /// Estimator event count (useful in tests and diagnostics).
    pub fn event_count(&self) -> u64 {
        self.estimator.event_count()
    }

    /// Cumulative observation window in milliseconds.
    pub fn window_ms(&self) -> u64 {
        self.estimator.window_ms()
    }

    /// Take a clonable snapshot of the current observer state.
    ///
    /// Consumers that need to perform their own estimation (e.g. integration
    /// tests constructing a fresh `MarginEstimator`) can pull
    /// `snapshot.homeostatic` out and feed it directly.
    pub fn snapshot(&self) -> RcsObservationSnapshot {
        RcsObservationSnapshot {
            homeostatic: self.homeostatic.clone(),
            last_budget: self.last_budget,
            event_count: self.estimator.event_count(),
            window_ms: self.estimator.window_ms(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::BudgetState;

    fn synthetic_state() -> AgentStateVector {
        AgentStateVector {
            progress: 0.5,
            uncertainty: 0.3,
            risk_level: aios_protocol::RiskLevel::Low,
            budget: BudgetState {
                tokens_remaining: 90_000,
                time_remaining_ms: 60_000,
                cost_remaining_usd: 3.0,
                tool_calls_remaining: 20,
                error_budget_remaining: 5,
            },
            error_streak: 0,
            context_pressure: 0.35,
            side_effect_pressure: 0.0,
            human_dependency: 0.0,
        }
    }

    #[test]
    fn observer_starts_at_canonical_l1_baseline() {
        let observer = RcsObserver::new_l1("test-session");
        assert!(observer.last_budget.is_none());
        assert_eq!(observer.event_count(), 0);
        assert_eq!(observer.window_ms(), 0);
        assert_eq!(observer.homeostatic().agent_id, "test-session");
    }

    #[test]
    fn record_cycle_updates_state_and_budget() {
        let mut observer = RcsObserver::new_l1("test-session");
        let state = synthetic_state();
        let budget = observer.record_cycle(&state);

        assert!(
            budget.is_stable(),
            "L1 budget must be stable after one cycle"
        );
        assert!(
            (observer.homeostatic().cognitive.context_pressure - 0.35).abs() < 1e-6,
            "context_pressure must reflect the AgentStateVector value"
        );
        assert_eq!(observer.homeostatic().cognitive.tokens_remaining, 90_000);
        assert_eq!(observer.homeostatic().cognitive.turns_completed, 1);
        assert_eq!(observer.event_count(), 1);
        assert!(observer.last_budget().is_some());
    }

    #[test]
    fn snapshot_matches_live_state() {
        let mut observer = RcsObserver::new_l1("snap");
        let state = synthetic_state();
        let budget = observer.record_cycle(&state);

        let snap = observer.snapshot();
        assert_eq!(snap.event_count, 1);
        assert!(snap.last_margin().is_some());
        assert!(
            (snap.last_margin().unwrap() - budget.margin()).abs() < 1e-9,
            "snapshot margin must equal the live margin"
        );
        assert_eq!(snap.homeostatic.agent_id, "snap");
    }

    #[test]
    fn repeated_cycles_fold_into_estimator_window() {
        let mut observer = RcsObserver::new_l1("rolling");
        for _ in 0..5 {
            let _ = observer.record_cycle(&synthetic_state());
            // Each record_cycle advances last_event_ms via SystemTime; with
            // real wall clock the window grows monotonically.
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(observer.event_count(), 5);
        assert!(
            observer.window_ms() > 0,
            "window_ms must be positive after multiple cycles"
        );
        assert!(observer.last_budget().unwrap().is_stable());
    }
}
