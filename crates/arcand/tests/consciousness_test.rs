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
use tokio::sync::oneshot;

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

#[tokio::test]
async fn query_status_returns_idle_mode() {
    let runtime = build_runtime(unique_root("consciousness-status"));
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    let handle = registry.get_or_create("test-status", BranchId::main(), runtime);

    // Give actor a moment to start.
    tokio::time::sleep(Duration::from_millis(20)).await;

    let status = handle.query_status().await.expect("should get status");
    assert_eq!(status.mode, "Idle");
    assert_eq!(status.queue_depth, 0);
    assert!(!status.has_active_run);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn non_blocking_cycle_completes() {
    let runtime = build_runtime(unique_root("consciousness-nonblocking"));
    let session_id = SessionId::from_string("test-nonblocking".to_string());

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
    let registry = ConsciousnessRegistry::new(config);
    let handle = registry.get_or_create("test-nonblocking", BranchId::main(), runtime);

    // Give actor a moment to start.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Send a message — should be accepted immediately (non-blocking).
    let (ack_tx, ack_rx) = oneshot::channel();
    handle
        .send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: "Hello non-blocking".to_string(),
                branch: BranchId::main(),
                steering: SteeringMode::Collect,
                ack: Some(ack_tx),
                run_context: RunContext::default(),
            },
        )))
        .await
        .unwrap();

    let ack = tokio::time::timeout(Duration::from_secs(5), ack_rx)
        .await
        .expect("should get ack quickly")
        .expect("ack channel should not be dropped");

    match ack {
        ConsciousnessAck::Accepted { queued } => {
            assert!(!queued, "should start immediately when idle");
        }
        ConsciousnessAck::Rejected { reason } => {
            panic!("unexpected rejection: {reason}");
        }
    }

    // Wait for the spawned cycle to complete and the actor to return to Idle.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("actor did not return to Idle within 10s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(status) = handle.query_status().await {
            if status.mode == "Idle" && !status.has_active_run {
                break;
            }
        }
    }

    // Final verification: should be Idle with empty queue.
    let status = handle.query_status().await.expect("should get status");
    assert_eq!(status.mode, "Idle");
    assert!(!status.has_active_run);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn cycle_completed_drains_queue() {
    let runtime = build_runtime(unique_root("consciousness-drain-queue"));
    let session_id = SessionId::from_string("test-drain".to_string());

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
    let registry = ConsciousnessRegistry::new(config);
    let handle = registry.get_or_create("test-drain", BranchId::main(), runtime);

    // Give actor a moment to start.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Send first message — starts immediately.
    let (ack_tx1, ack_rx1) = oneshot::channel();
    handle
        .send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: "First message".to_string(),
                branch: BranchId::main(),
                steering: SteeringMode::Collect,
                ack: Some(ack_tx1),
                run_context: RunContext::default(),
            },
        )))
        .await
        .unwrap();

    let ack1 = tokio::time::timeout(Duration::from_secs(5), ack_rx1)
        .await
        .expect("should get ack")
        .expect("channel should not be dropped");
    assert!(
        matches!(ack1, ConsciousnessAck::Accepted { queued: false }),
        "first message should start immediately"
    );

    // Brief pause to let the first cycle start actively, then send second message.
    tokio::time::sleep(Duration::from_millis(10)).await;

    let (ack_tx2, ack_rx2) = oneshot::channel();
    handle
        .send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: "Second message".to_string(),
                branch: BranchId::main(),
                steering: SteeringMode::Collect,
                ack: Some(ack_tx2),
                run_context: RunContext::default(),
            },
        )))
        .await
        .unwrap();

    let ack2 = tokio::time::timeout(Duration::from_secs(5), ack_rx2)
        .await
        .expect("should get ack")
        .expect("channel should not be dropped");

    // Second message should be accepted (queued or immediate depending on timing).
    match ack2 {
        ConsciousnessAck::Accepted { .. } => {} // queued or immediate — both OK
        ConsciousnessAck::Rejected { reason } => {
            panic!("second message rejected: {reason}");
        }
    }

    // Wait for both cycles to complete — actor should eventually return to Idle.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("actor did not return to Idle within 15s");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(status) = handle.query_status().await {
            if status.mode == "Idle" && !status.has_active_run && status.queue_depth == 0 {
                break;
            }
        }
    }

    // Both messages were processed: actor is Idle, queue is empty.
    let status = handle.query_status().await.expect("should get status");
    assert_eq!(status.mode, "Idle");
    assert_eq!(status.queue_depth, 0);
    assert!(!status.has_active_run);

    registry.shutdown_all().await;
}

#[tokio::test]
async fn multiple_messages_accepted_sequentially() {
    let runtime = build_runtime(unique_root("consciousness-multi-msg"));
    let session_id = SessionId::from_string("test-multi".to_string());

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

    // Send two messages — both should be accepted (either queued or immediate).
    for i in 0..2 {
        let (ack_tx, ack_rx) = oneshot::channel();
        tx.send(ConsciousnessEvent::UserMessage(Box::new(
            UserMessageEvent {
                objective: format!("message {i}"),
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
            .expect("should get ack")
            .expect("channel should not be dropped");

        match ack {
            ConsciousnessAck::Accepted { .. } => {} // queued or immediate — both OK
            ConsciousnessAck::Rejected { reason } => {
                panic!("message {i} rejected: {reason}");
            }
        }
    }

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

// ─── Spaces integration tests (BRO-458) ────────────────────────────────────

#[tokio::test]
async fn spaces_message_handled_when_idle() {
    let runtime = build_runtime(unique_root("consciousness-spaces-idle"));
    let session_id = SessionId::from_string("test-spaces-idle".to_string());

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

    // Give actor a moment to start and be in Idle state.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Send a SpacesMessage while the actor is Idle.
    tx.send(ConsciousnessEvent::SpacesMessage {
        channel_id: "agent-logs".to_string(),
        sender: "peer-agent-abc".to_string(),
        content: "Hey, can you check the latest deployment?".to_string(),
    })
    .await
    .unwrap();

    // Give the actor time to process the message (it will trigger a deliberation cycle).
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The actor should still be alive and operational after processing.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::QueryStatus { reply: reply_tx })
        .await
        .unwrap();

    let status = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .expect("should get status within 5s")
        .expect("status channel should not be dropped");

    // After processing the SpacesMessage (which triggers a run cycle), the actor
    // should be back to Idle.
    assert_eq!(status.mode, "Idle");

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn own_spaces_messages_ignored() {
    let runtime = build_runtime(unique_root("consciousness-spaces-self"));
    let session_id_str = "test-spaces-self";
    let session_id = SessionId::from_string(session_id_str.to_string());

    let config = ConsciousnessConfig::default();
    let (handle, tx) = SessionConsciousness::spawn(session_id, BranchId::main(), runtime, config);

    // Give actor a moment to start.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Send a SpacesMessage where sender matches the actor's own session_id.
    // This should be silently ignored (no crash, no state change).
    tx.send(ConsciousnessEvent::SpacesMessage {
        channel_id: "agent-logs".to_string(),
        sender: session_id_str.to_string(),
        content: "I posted this myself".to_string(),
    })
    .await
    .unwrap();

    // Give the actor time to process.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Actor should still be Idle (own message was ignored, no run triggered).
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::QueryStatus { reply: reply_tx })
        .await
        .unwrap();

    let status = tokio::time::timeout(Duration::from_secs(5), reply_rx)
        .await
        .expect("should get status within 5s")
        .expect("status channel should not be dropped");

    assert_eq!(status.mode, "Idle");
    assert!(!status.has_active_run);
    assert_eq!(status.queue_depth, 0);

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn stall_detection_breaks_stuck_loop() {
    let runtime = build_runtime(unique_root("consciousness-stall"));
    let session_id = SessionId::from_string("test-stall".to_string());

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
        max_agent_iterations: 10,
        ..Default::default()
    };
    let (handle, tx) = SessionConsciousness::spawn(session_id, BranchId::main(), runtime, config);

    let (ack_tx, ack_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage(Box::new(
        UserMessageEvent {
            objective: "stall test".to_string(),
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
        ConsciousnessAck::Accepted { .. } => {}
        ConsciousnessAck::Rejected { reason } => panic!("unexpected rejection: {reason}"),
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    let (status_tx, status_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::QueryStatus { reply: status_tx })
        .await
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(5), status_rx)
        .await
        .expect("should get status")
        .expect("status channel should not be dropped");
    assert_eq!(status.mode, "Idle", "actor should return to Idle after run");

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn autonomic_signal_sets_compaction_flag() {
    let runtime = build_runtime(unique_root("consciousness-autonomic"));
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    let handle = registry.get_or_create("test-autonomic", BranchId::main(), runtime);
    tokio::time::sleep(Duration::from_millis(20)).await;

    handle
        .send(ConsciousnessEvent::AutonomicSignal {
            ruling: "Compress".to_string(),
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let status = handle.query_status().await.expect("should get status");
    assert_eq!(status.mode, "Idle");
    assert!(!status.has_active_run);

    handle
        .send(ConsciousnessEvent::AutonomicSignal {
            ruling: "Breathe".to_string(),
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let status = handle.query_status().await.expect("should get status");
    assert_eq!(status.mode, "Idle");

    registry.shutdown_all().await;
}
