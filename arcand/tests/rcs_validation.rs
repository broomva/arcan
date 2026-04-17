//! RCS (Recursive Controlled Systems) runtime validation.
//!
//! Part of the F2 instrumentation deliverable — asserts the paper's Level 1
//! stability claim `lambda_1 > 0` against a running arcand session and against
//! homeostatic state evolved over a burst of synthetic ticks.
//!
//! The test spawns a `SessionConsciousness` with a mock provider (mirroring
//! `consciousness_test.rs`), pushes a user message through the agent loop to
//! prove the runtime is live, and in parallel evolves a `HomeostaticState`
//! forward over N ticks. The estimator folds the observations into a
//! `StabilityBudget` which must (a) be individually stable and (b) stay
//! within tolerance of the canonical L1 margin loaded from
//! `research/rcs/latex/parameters.toml`.
//!
//! See `research/rcs/latex/rcs-definitions.tex` Theorem 1 for the budget.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::{
    ApprovalPort, EventStorePort, KernelResult, ModelCompletion, ModelCompletionRequest,
    ModelDirective, ModelProviderPort, ModelStopReason, PolicyGatePort, PolicySet, SteeringMode,
    ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use aios_sandbox::LocalSandboxRunner;
use aios_tools::{ToolDispatcher, ToolRegistry};
use arcand::consciousness::{
    ConsciousnessAck, ConsciousnessConfig, ConsciousnessEvent, RunContext, SessionConsciousness,
    UserMessageEvent,
};
use async_trait::async_trait;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rcs_budget::{MarginEstimator, StabilityBudget};

use aios_protocol::{BranchId, SessionId};

// ─── Test helpers (mirrored from consciousness_test.rs) ─────────────────────

#[derive(Debug, Default)]
struct TestProvider;

#[async_trait]
impl ModelProviderPort for TestProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        Ok(ModelCompletion {
            provider: "test".to_owned(),
            model: "test-model".to_owned(),
            llm_call_record: None,
            directives: vec![ModelDirective::Message {
                role: "assistant".to_owned(),
                content: format!("ack: {}", request.objective),
            }],
            stop_reason: ModelStopReason::Completed,
            usage: None,
            final_answer: Some("ok".to_owned()),
        })
    }
}

fn unique_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{name}-{nanos}"))
}

fn build_runtime(root: PathBuf) -> Arc<KernelRuntime> {
    let event_store_backend = Arc::new(FileEventStore::new(root.join("kernel")));
    let journal = Arc::new(EventJournal::new(
        event_store_backend,
        EventStreamHub::new(1024),
    ));
    let event_store: Arc<dyn EventStorePort> = journal;

    let policy_engine = Arc::new(SessionPolicyEngine::new(PolicySet::default()));
    let policy_gate: Arc<dyn PolicyGatePort> = policy_engine.clone();
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ApprovalQueue::default());

    let registry = Arc::new(ToolRegistry::with_core_tools());
    let sandbox = Arc::new(LocalSandboxRunner::new(vec!["echo".to_owned()]));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, policy_engine, sandbox));
    let tool_harness: Arc<dyn ToolHarnessPort> = dispatcher;

    Arc::new(KernelRuntime::new(
        RuntimeConfig::new(root),
        event_store,
        Arc::new(TestProvider),
        tool_harness,
        approvals,
        policy_gate,
    ))
}

/// Number of synthetic autonomic ticks the test drives through the estimator.
/// Sized so the estimator's window covers at least one L0/L1 dwell period
/// without requiring a real wall-clock wait.
const SIMULATED_TICKS: u32 = 16;

/// Simulate a single autonomic tick on `state`.
///
/// Each tick:
/// - advances `last_event_ms` by 100ms (L1 time scale is seconds; 100ms is a
///   realistic inter-event gap for a chatty inner loop),
/// - increments turn/observation counters,
/// - nudges context pressure toward a moderate steady-state (0.4) rather than
///   leaving it at zero — this exercises the `rho` proxy in the estimator.
fn tick(state: &mut HomeostaticState, i: u32) {
    state.last_event_ms = (i as u64 + 1) * 100;
    state.last_event_seq = (i as u64) + 1;
    state.cognitive.turns_completed = i + 1;
    state.cognitive.observation_count = i + 1;
    // Ramp context pressure from 0 to ~0.4 over the window.
    let target = 0.4_f32;
    state.cognitive.context_pressure = (state.cognitive.context_pressure
        + (target - state.cognitive.context_pressure) * 0.25)
        .clamp(0.0, 1.0);
    state.operational.total_successes = state.operational.total_successes.saturating_add(1);
}

#[tokio::test]
async fn rcs_l1_margin_is_positive_under_simulated_load() {
    // ── 1. Spawn a minimal arcand agent with a mock provider ──────────────
    let runtime = build_runtime(unique_root("rcs-validation-l1"));
    let session_id = SessionId::from_string("test-rcs-l1".to_string());

    runtime
        .create_session_with_id(
            session_id.clone(),
            "test",
            PolicySet::default(),
            aios_protocol::ModelRouting::default(),
        )
        .await
        .unwrap();

    let config = ConsciousnessConfig {
        max_agent_iterations: 1,
        ..Default::default()
    };
    let (handle, tx) = SessionConsciousness::spawn(session_id, BranchId::main(), runtime, config);

    // Drive at least one real cycle through the agent to prove the runtime
    // is exercised. The RCS state we measure is computed below against a
    // HomeostaticState evolved in parallel — the daemon does not surface its
    // internal HomeostaticState through the consciousness actor (it lives in
    // arcand's HTTP server layer), so we simulate the tick series that
    // autonomic would produce under equivalent load.
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage(Box::new(
        UserMessageEvent {
            objective: "rcs validation".to_string(),
            branch: BranchId::main(),
            steering: SteeringMode::Collect,
            ack: Some(ack_tx),
            run_context: RunContext::default(),
        },
    )))
    .await
    .unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(10), ack_rx)
        .await
        .expect("ack within 10s")
        .expect("ack channel open");
    assert!(
        matches!(ack, ConsciousnessAck::Accepted { .. }),
        "agent must accept the test message"
    );

    // ── 2. Run N simulated ticks through a parallel HomeostaticState ──────
    let mut state = HomeostaticState::for_agent("test-rcs-l1");
    let mut estimator = MarginEstimator::for_l1(state.clone());

    for i in 0..SIMULATED_TICKS {
        tick(&mut state, i);
        estimator.observe(&state);
    }

    assert_eq!(
        estimator.event_count(),
        SIMULATED_TICKS as u64,
        "estimator must fold every tick"
    );
    assert!(
        estimator.window_ms() > 0,
        "estimator must observe a non-empty window"
    );

    // ── 3. Estimate stability budget and assert the paper's claim ─────────
    let budget = estimator.estimate();

    // Primary claim: Level 1 is individually stable.
    let margin = budget.margin();
    assert!(
        budget.is_stable(),
        "L1 runtime estimate must be stable: margin = {margin:.6}"
    );

    // ── 4. Compare to canonical L1 from parameters.toml ───────────────────
    let canonical_l1 =
        StabilityBudget::from_canonical("L1").expect("L1 must exist in canonical params");
    let canonical_margin = canonical_l1.margin();

    // Tolerance rationale: the estimator perturbs rho, eta, and tau_bar using
    // proxies derived from observable HomeostaticState deltas. Under the
    // ramp-to-0.4 context-pressure scenario, rho grows by roughly
    // (0.5 + 0.4) = 0.9 of its prior, which pulls the margin down by at most
    // L_theta * rho_delta = 0.2 * (0.5 * 0.1) = 0.01. We allow 0.1 of slack on
    // top to tolerate future refinements of the proxy model.
    const TOLERANCE: f64 = 0.1;
    let drift = (margin - canonical_margin).abs();
    assert!(
        drift < TOLERANCE,
        "runtime L1 margin {margin:.6} diverges from canonical {canonical_margin:.6} \
         by {drift:.6} (> tolerance {TOLERANCE})"
    );

    // Estimator must not report a margin LARGER than the nominal gamma — that
    // would mean a runtime signal is claiming the system is safer than the
    // paper's theorem permits.
    assert!(
        margin <= canonical_l1.gamma,
        "runtime margin {margin} must not exceed nominal gamma {}",
        canonical_l1.gamma
    );

    // ── 5. Cleanup ────────────────────────────────────────────────────────
    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn rcs_canonical_parameters_reproduce_paper_lambdas() {
    // Pure validation: the canonical TOML parses and the derived margins
    // match the [derived.lambda] cache in the paper's parameters.toml.
    // If this fails, the in-crate mirror has drifted from the paper repo.
    let cases = [
        ("L0", 1.455_357_f64),
        ("L1", 0.411_484),
        ("L2", 0.069_274),
        ("L3", 0.006_398),
    ];
    for (id, expected) in cases {
        let b = StabilityBudget::from_canonical(id).unwrap();
        let got = b.margin();
        assert!(
            (got - expected).abs() < 1e-3,
            "level {id}: margin {got:.6} != paper derived {expected:.6}"
        );
        assert!(b.is_stable(), "canonical {id} must be stable");
    }
}
