//! RCS (Recursive Controlled Systems) runtime validation — end-to-end.
//!
//! This test exercises the F2 instrumentation loop against a **real running
//! arcand consciousness actor**:
//!
//! ```text
//!   agent executes
//!     → tick_on_branch emits events + AgentStateVector
//!       → CycleCompleted(final_state) sent to actor
//!         → actor updates HomeostaticState INSIDE RcsObserver
//!           → MarginEstimator::for_l1 folds the state
//!             → StabilityBudget computed
//!               → life.control_margin_l1 gauge emitted via vigil
//!                 → test snapshots the observer and asserts lambda_1 > 0
//! ```
//!
//! This replaces the earlier reconstruction-style test that synthesised a
//! parallel `HomeostaticState` alongside the actor. Here we assert against
//! the **same** state the daemon's control loop is using — no doubles, no
//! parallel evolution.
//!
//! See:
//! - `research/rcs/latex/rcs-definitions.tex` Theorem 1 — the budget
//!   formula and individual-stability claim.
//! - `crates/autonomic/autonomic-core/src/rcs_budget.rs` — the
//!   canonical parameters + `MarginEstimator` implementation.
//! - `crates/arcan/arcand/src/rcs_observer.rs` — the daemon-side
//!   observer that owns the authoritative `HomeostaticState`.
//! - `crates/vigil/life-vigil/src/metrics.rs` — the
//!   `life.control_margin_l1` gauge that receives every emission.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    ConsciousnessAck, ConsciousnessConfig, ConsciousnessEvent, ConsciousnessRegistry, RunContext,
    SessionConsciousness, UserMessageEvent,
};
use arcand::rcs_observer::SharedRcsObserver;
use async_trait::async_trait;
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

/// Wait (up to `deadline`) for the RCS observer to fold at least one cycle.
///
/// Polls `observer.event_count()` with a small sleep so the actor has time to
/// deliver `CycleCompleted` through its mpsc channel and update the
/// `HomeostaticState`. Returns when the count is >= `target` or panics on
/// timeout. Keeps the lock held only for single reads — never across sleeps.
async fn wait_for_cycles(observer: &SharedRcsObserver, target: u64, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let count = {
            let guard = observer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.event_count()
        };
        if count >= target {
            return;
        }
        if Instant::now() > deadline {
            panic!("timed out waiting for {target} cycle(s) in RCS observer (seen {count})",);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn rcs_l1_margin_is_positive_from_real_daemon_state() {
    // ── 1. Spawn the full consciousness actor with an RCS observer ────────
    //
    // `spawn_with_rcs` is the end-to-end path: the same constructor the
    // production registry uses (see ConsciousnessRegistry::get_or_create),
    // just with the observer handle returned so the test can read it.
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
    let (join, tx, observer) =
        SessionConsciousness::spawn_with_rcs(session_id, BranchId::main(), runtime, config);

    // Baseline sanity: no cycles recorded yet, last_budget must be None.
    {
        let guard = observer.lock().unwrap();
        assert_eq!(
            guard.event_count(),
            0,
            "observer must start with zero folded cycles"
        );
        assert!(
            guard.last_budget().is_none(),
            "observer must not have a budget before the first cycle"
        );
    }

    // ── 2. Drive a real agent cycle through the consciousness loop ────────
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

    // ── 3. Wait for the cycle to flow through CycleCompleted and update ───
    //
    // This is the moment that matters: the actor has received
    // `CycleCompleted { final_state }` and called `RcsObserver::record_cycle`,
    // which means (a) the HomeostaticState inside the observer reflects the
    // run, (b) the MarginEstimator has folded it, and (c) the
    // `life.control_margin_l1` gauge has been emitted. No separate state
    // evolution, no parallel reconstruction.
    wait_for_cycles(&observer, 1, Duration::from_secs(15)).await;

    // ── 4. Snapshot the REAL observer state and assert lambda_1 > 0 ───────
    let snapshot = {
        let guard = observer.lock().unwrap();
        guard.snapshot()
    };

    assert_eq!(
        snapshot.event_count, 1,
        "exactly one cycle must have been folded into the estimator"
    );
    assert!(
        snapshot.last_budget.is_some(),
        "snapshot must carry a budget after the first cycle"
    );

    // The observer already computed a budget when it recorded the cycle.
    // Pull it out — this is the value that was pushed to vigil.
    let daemon_budget = snapshot.last_budget.expect("budget present");
    let daemon_margin = daemon_budget.margin();

    assert!(
        daemon_budget.is_stable(),
        "L1 margin must be positive from the daemon's real state: \
         margin = {daemon_margin:.6}"
    );

    // ── 5. Independently re-estimate from the observer's homeostatic ──────
    //
    // This sanity-checks that the value emitted by the observer matches what
    // a fresh `MarginEstimator::for_l1` would compute from the same state.
    // It's not a reconstruction of the daemon's state — it's validation that
    // the daemon's emission is reproducible from the same inputs.
    let mut verifier = MarginEstimator::for_l1(snapshot.homeostatic.clone());
    verifier.observe(&snapshot.homeostatic);
    let recomputed = verifier.estimate();
    assert!(
        (recomputed.margin() - daemon_margin).abs() < 1e-9,
        "recomputed margin {} must match daemon-emitted margin {}",
        recomputed.margin(),
        daemon_margin
    );

    // ── 6. Compare to canonical L1 from parameters.toml ───────────────────
    let canonical_l1 =
        StabilityBudget::from_canonical("L1").expect("L1 must exist in canonical params");
    let canonical_margin = canonical_l1.margin();

    // Tolerance rationale: the observer perturbs rho, eta, and tau_bar based
    // on proxies derived from observable HomeostaticState deltas (see
    // `MarginEstimator::estimate`). Under a single real cycle with the mock
    // provider, context_pressure/tool_density stay low so the estimator
    // should hug canonical closely. We allow 0.1 of slack to tolerate
    // future refinements of the proxy model.
    const TOLERANCE: f64 = 0.1;
    let drift = (daemon_margin - canonical_margin).abs();
    assert!(
        drift < TOLERANCE,
        "runtime L1 margin {daemon_margin:.6} diverges from canonical \
         {canonical_margin:.6} by {drift:.6} (> tolerance {TOLERANCE})"
    );

    // Nominal gamma is the upper bound: a runtime estimate claiming the
    // system is *safer* than the theorem permits would indicate a bug in
    // the estimator's proxy model.
    assert!(
        daemon_margin <= canonical_l1.gamma,
        "runtime margin {daemon_margin} must not exceed nominal gamma {}",
        canonical_l1.gamma
    );

    // ── 7. Cleanup ────────────────────────────────────────────────────────
    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
}

#[tokio::test]
async fn rcs_l1_observer_survives_multiple_cycles() {
    // Stronger variant: drive multiple messages sequentially and assert the
    // observer's event count tracks every cycle AND the margin stays stable
    // through the series.
    let runtime = build_runtime(unique_root("rcs-validation-l1-multi"));
    let session_id = SessionId::from_string("test-rcs-l1-multi".to_string());

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
    let (join, tx, observer) =
        SessionConsciousness::spawn_with_rcs(session_id, BranchId::main(), runtime, config);

    const MESSAGES: u64 = 3;
    for i in 0..MESSAGES {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        tx.send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: format!("rcs validation msg {i}"),
                branch: BranchId::main(),
                steering: SteeringMode::Collect,
                ack: Some(ack_tx),
                run_context: RunContext::default(),
            },
        )))
        .await
        .unwrap();

        // Wait for ack so messages don't pile up; then wait for the observer
        // to register this cycle before sending the next.
        let _ = tokio::time::timeout(Duration::from_secs(10), ack_rx)
            .await
            .expect("ack within 10s");
        wait_for_cycles(&observer, i + 1, Duration::from_secs(15)).await;
    }

    let snapshot = {
        let guard = observer.lock().unwrap();
        guard.snapshot()
    };
    assert_eq!(
        snapshot.event_count, MESSAGES,
        "observer must have folded every cycle"
    );

    // All cycles executed; margin must have stayed stable throughout.
    let last = snapshot.last_budget.expect("budget present");
    assert!(
        last.is_stable(),
        "L1 margin must remain stable after {MESSAGES} cycles: margin = {}",
        last.margin()
    );

    // Agent id must be the session id — we are reading the SAME state the
    // daemon is using for regulation decisions.
    assert_eq!(snapshot.homeostatic.agent_id, "test-rcs-l1-multi");

    // Cognitive pillar: turns_completed / observation_count must advance
    // per-cycle, proving the observer is driven by real ticks, not defaults.
    assert_eq!(
        snapshot.homeostatic.cognitive.turns_completed as u64, MESSAGES,
        "turns_completed must equal the number of cycles"
    );
    assert_eq!(
        snapshot.homeostatic.cognitive.observation_count as u64, MESSAGES,
        "observation_count must equal the number of cycles"
    );

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), join).await;
}

#[tokio::test]
async fn rcs_handle_exposes_live_observer_through_registry() {
    // Exercise the full production path: ConsciousnessRegistry::get_or_create
    // → ConsciousnessHandle → handle.rcs_observer() / rcs_snapshot().
    // This is how a diagnostics endpoint (e.g. a future GET /rcs route)
    // would publish the margin.
    let runtime = build_runtime(unique_root("rcs-validation-handle"));
    let session_id_str = "test-rcs-handle";

    runtime
        .create_session_with_id(
            SessionId::from_string(session_id_str.to_string()),
            "test",
            PolicySet::default(),
            aios_protocol::ModelRouting::default(),
        )
        .await
        .unwrap();

    let registry = ConsciousnessRegistry::new(ConsciousnessConfig {
        max_agent_iterations: 1,
        ..Default::default()
    });
    let handle = registry.get_or_create(session_id_str, BranchId::main(), runtime);
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Fresh handle: observer exists but has folded nothing yet.
    let initial = handle.rcs_snapshot();
    assert_eq!(initial.event_count, 0);
    assert!(initial.last_budget.is_none());

    // Drive one real cycle.
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    handle
        .send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: "handle-access test".to_string(),
                branch: BranchId::main(),
                steering: SteeringMode::Collect,
                ack: Some(ack_tx),
                run_context: RunContext::default(),
            },
        )))
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), ack_rx)
        .await
        .expect("ack within 10s");

    // The registry-held observer is the same Arc the actor writes into,
    // so `wait_for_cycles` on the handle's observer works identically.
    let observer = handle.rcs_observer();
    wait_for_cycles(&observer, 1, Duration::from_secs(15)).await;

    let snap = handle.rcs_snapshot();
    assert_eq!(snap.event_count, 1);
    let margin = snap.last_margin().expect("budget present after cycle");
    assert!(margin > 0.0, "L1 margin must be positive: {margin}");

    registry.shutdown_all().await;
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
