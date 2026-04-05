//! Integration tests for the consciousness actor loop (BRO-455).
//!
//! These tests use a real `KernelRuntime` with mock ports to validate
//! the consciousness actor's event loop, queuing, and mode transitions.

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
    ConsciousnessAck, ConsciousnessConfig, ConsciousnessEvent, ConsciousnessRegistry, RunContext,
    SessionConsciousness, UserMessageEvent,
};
use async_trait::async_trait;

use aios_protocol::{BranchId, SessionId};

// ─── Test helpers ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct TestProvider;

#[async_trait]
impl ModelProviderPort for TestProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        Ok(ModelCompletion {
            provider: "test".to_owned(),
            model: "test-model".to_owned(),
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

// ─── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn actor_shuts_down_cleanly() {
    let runtime = build_runtime(unique_root("consciousness-shutdown"));
    let session_id = SessionId::from_string("test-shutdown".to_string());
    let branch = BranchId::main();

    let (handle, tx) =
        SessionConsciousness::spawn(session_id, branch, runtime, ConsciousnessConfig::default());

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "actor should shut down within 5s");
}

#[tokio::test]
async fn actor_accepts_message_when_idle() {
    let runtime = build_runtime(unique_root("consciousness-idle-msg"));
    let session_id = SessionId::from_string("test-idle-msg".to_string());

    runtime
        .create_session_with_id(
            session_id.clone(),
            "test",
            PolicySet::default(),
            aios_protocol::ModelRouting::default(),
        )
        .await
        .unwrap();

    let (handle, tx) = SessionConsciousness::spawn(
        session_id,
        BranchId::main(),
        runtime,
        ConsciousnessConfig {
            max_agent_iterations: 1,
            ..Default::default()
        },
    );

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage(Box::new(
        UserMessageEvent {
            objective: "Hello consciousness".to_string(),
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
        .expect("should get ack within 10s")
        .expect("ack channel should not be dropped");

    match ack {
        ConsciousnessAck::Accepted { queued } => {
            assert!(!queued, "should start immediately when idle");
        }
        ConsciousnessAck::Rejected { reason } => {
            panic!("unexpected rejection: {reason}");
        }
    }

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn registry_creates_and_retrieves_handle() {
    let runtime = build_runtime(unique_root("consciousness-registry"));
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    let handle = registry.get_or_create("test-registry", BranchId::main(), runtime.clone());
    assert!(handle.is_alive());

    let handle2 = registry.get_or_create("test-registry", BranchId::main(), runtime);
    assert!(handle2.is_alive());

    assert_eq!(registry.session_count(), 1);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn registry_shutdown_all_stops_actors() {
    let runtime = build_runtime(unique_root("consciousness-shutdown-all"));
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    registry.get_or_create("s1", BranchId::main(), runtime.clone());
    registry.get_or_create("s2", BranchId::main(), runtime);

    assert_eq!(registry.session_count(), 2);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn channel_close_stops_actor() {
    let runtime = build_runtime(unique_root("consciousness-channel-close"));
    let session_id = SessionId::from_string("test-channel-close".to_string());

    let (handle, tx) = SessionConsciousness::spawn(
        session_id,
        BranchId::main(),
        runtime,
        ConsciousnessConfig::default(),
    );

    // Drop the sender — actor should detect channel close and exit.
    drop(tx);

    let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
    assert!(result.is_ok(), "actor should stop when channel closes");
}
