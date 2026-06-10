//! Topology-B end-to-end wire test for BRO-1016.
//!
//! Boots a minimal `KernelRuntime`, exposes
//! `arcan.v1.AgentSubstrate` on a tempdir UDS, dials it via the
//! `arcan-proxy` crate's `ArcanProxy` builder, and asserts that:
//!
//! 1. `ArcanProxy::create_agent(sid)` actually causes the underlying
//!    `KernelRuntime` to gain that session — proving the wire moves
//!    args through the substrate (not a hardcoded shape).
//! 2. `ArcanProxy::dispatch_message(sid, content)` yields a real
//!    streamed `life.v1.AgentEvent` (≥ 1 event followed by a
//!    terminal frame) sourced from the substrate's `tick_on_branch`
//!    + broadcast pump.
//! 3. `ArcanProxy::destroy_agent(sid)` returns Ok — saga compensation
//!    paths stay clean.
//!
//! This is the contract the four-PR Topology-B audit (entity page
//! `research/entities/concept/topology-b-substrate-stub-gap.md`)
//! demanded a real wire for. Lifed isn't wired in here — adding it
//! would pull arcand into the `lifed`/`arcan-proxy` dep tree and
//! break `scripts/verify_dependencies_lifed.sh`. The lifed→
//! arcan-proxy boundary is already covered by lifed's own
//! integration suite (it exercises the proxy trait), so end-to-end
//! coverage in production is the COMPOSITION of those two suites.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::{
    ApprovalPort, Capability, EventStorePort, KernelResult, ModelCompletion,
    ModelCompletionRequest, ModelDirective, ModelProviderPort, ModelStopReason, PolicyGatePort,
    PolicySet, SessionId, ToolCall, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use aios_sandbox::LocalSandboxRunner;
use aios_tools::{ToolDispatcher, ToolRegistry};
use arcan_proxy::ArcanProxy;
use arcan_substrate_proto::arcan::v1::agent_substrate_server::AgentSubstrateServer;
use arcand::substrate::SubstrateService;
use async_trait::async_trait;
use futures::StreamExt;
use life_runtime_proto::life::v1::AgentEventKind;
use tempfile::TempDir;
use tokio::sync::oneshot;

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

/// Spin up the substrate gRPC server on a tempdir UDS socket and
/// return the socket path + shutdown handle. The server consumes a
/// shared `Arc<KernelRuntime>` so the test can read its state after
/// driving calls through the proxy.
struct SubstrateUnderTest {
    socket: PathBuf,
    _tempdir: TempDir,
    runtime: Arc<KernelRuntime>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_handle: Option<tokio::task::JoinHandle<()>>,
}

impl SubstrateUnderTest {
    async fn start() -> Self {
        let tempdir = TempDir::new().expect("tempdir");
        let socket = tempdir.path().join("arcand.sock");
        let runtime = build_runtime(tempdir.path().join("kernel-root"));

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let service = SubstrateService::new(Arc::clone(&runtime));
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind UDS");
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);

        let server_handle = tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .add_service(AgentSubstrateServer::new(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        // Wait for the socket to appear.
        for _ in 0..200 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "substrate socket bound");

        Self {
            socket,
            _tempdir: tempdir,
            runtime,
            shutdown_tx: Some(shutdown_tx),
            server_handle: Some(server_handle),
        }
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.server_handle.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
        }
    }
}

#[tokio::test]
async fn create_agent_actually_creates_a_substrate_session() {
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1016-create";
    let agent_id = proxy.create_agent(sid).await.expect("create_agent");

    // Substrate-side proof: BEFORE BRO-1016 this would have stayed
    // false because the proxy returned `format!("agent-{sid}")`
    // without touching the substrate. AFTER BRO-1016 the substrate
    // KernelRuntime now has the session.
    let session_id = SessionId::from_string(sid);
    assert!(
        env.runtime.session_exists(&session_id),
        "substrate KernelRuntime should have gained the session"
    );

    // Phase 1 invariant: agent_id mirrors sid (1:1 mapping) — see
    // proto comment in proto/arcan/v1/substrate.proto.
    assert_eq!(agent_id, sid);

    // Idempotency: re-issuing CreateAgent on the same sid returns the
    // same agent_id and doesn't blow up.
    let agent_id2 = proxy.create_agent(sid).await.expect("re-create idempotent");
    assert_eq!(agent_id2, sid);

    env.shutdown().await;
}

#[tokio::test]
async fn dispatch_message_streams_real_substrate_events() {
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1016-dispatch";
    let _ = proxy.create_agent(sid).await.expect("create_agent");

    // BRO-1206: `dispatch_message` accepts an optional per-call model
    // override. The arcan substrate gRPC wire ignores it (the substrate
    // proto doesn't carry a `model` field); HTTP-backed backends honour
    // it. This Topology B e2e test exercises the gRPC path so the
    // override is irrelevant — pass `None`.
    let mut stream = proxy
        .dispatch_message(sid, "Hello, substrate!", None)
        .await
        .expect("dispatch_message");

    let mut events = Vec::new();
    let mut terminal_seen = false;
    while let Some(evt) = stream.next().await {
        let evt = evt.expect("substrate event ok");
        let kind = evt.kind;
        events.push(evt);
        if kind == AgentEventKind::Finish as i32 || kind == AgentEventKind::Error as i32 {
            terminal_seen = true;
            break;
        }
        if events.len() > 32 {
            break;
        }
    }
    drop(stream);
    assert!(
        !events.is_empty(),
        "dispatch should yield at least one event from the real substrate"
    );
    assert!(
        terminal_seen,
        "dispatch stream should terminate with FINISH or ERROR"
    );
    env.shutdown().await;
}

/// Provider that requests one `fs.write` tool call on its first
/// completion, then answers normally — drives the Phase-2 tool
/// lifecycle through a real Direct tick.
#[derive(Debug, Default)]
struct ToolCallingProvider {
    calls: AtomicU32,
}

#[async_trait]
impl ModelProviderPort for ToolCallingProvider {
    async fn complete(&self, _request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ModelCompletion {
                provider: "test".to_owned(),
                model: "test-model".to_owned(),
                llm_call_record: None,
                directives: vec![ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: "call-e2e-1".to_owned(),
                        tool_name: "fs.write".to_owned(),
                        input: serde_json::json!({
                            "path": "artifacts/e2e.txt",
                            "content": "phase-2 tool lifecycle"
                        }),
                        requested_capabilities: vec![Capability::fs_write("/session/artifacts/**")],
                    },
                }],
                stop_reason: ModelStopReason::ToolCall,
                usage: None,
                final_answer: None,
            })
        } else {
            Ok(ModelCompletion {
                provider: "test".to_owned(),
                model: "test-model".to_owned(),
                llm_call_record: None,
                directives: vec![ModelDirective::Message {
                    role: "assistant".to_owned(),
                    content: "wrote the file".to_owned(),
                }],
                stop_reason: ModelStopReason::Completed,
                usage: None,
                final_answer: Some("done".to_owned()),
            })
        }
    }
}

#[tokio::test]
async fn dispatch_message_streams_tool_lifecycle_events() {
    // Same harness as `dispatch_message_streams_real_substrate_events`
    // but with a provider that requests a tool call — asserts the
    // Phase-2 wire: TOOL_CALL_PENDING and TOOL_RESULT frames arrive at
    // the proxy as structured `life.v1.AgentEvent`s with payloads.
    let tempdir = TempDir::new().expect("tempdir");
    let socket = tempdir.path().join("arcand.sock");
    let root = tempdir.path().join("kernel-root");

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
    let runtime = Arc::new(KernelRuntime::new(
        RuntimeConfig::new(root),
        event_store,
        Arc::new(ToolCallingProvider::default()),
        tool_harness,
        approvals,
        policy_gate,
    ));

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let service = SubstrateService::new(Arc::clone(&runtime));
    let listener = tokio::net::UnixListener::bind(&socket).expect("bind UDS");
    let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);
    let server_handle = tokio::spawn(async move {
        let _ = tonic::transport::Server::builder()
            .add_service(AgentSubstrateServer::new(service))
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let proxy = ArcanProxy::connect(socket.clone())
        .await
        .expect("dial substrate UDS");
    let sid = "phase2-tool-lifecycle";
    let _ = proxy.create_agent(sid).await.expect("create_agent");
    let mut stream = proxy
        .dispatch_message(sid, "write the e2e artifact", None)
        .await
        .expect("dispatch_message");

    let mut pending: Option<serde_json::Value> = None;
    let mut result: Option<serde_json::Value> = None;
    let mut terminal_seen = false;
    let mut count = 0;
    while let Some(evt) = stream.next().await {
        let evt = evt.expect("substrate event ok");
        count += 1;
        let payload = evt.record.as_ref().map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.payload).expect("record payload JSON")
        });
        if evt.kind == AgentEventKind::ToolCallPending as i32 {
            pending = payload.clone();
        }
        if evt.kind == AgentEventKind::ToolResult as i32 {
            result = payload.clone();
        }
        if evt.kind == AgentEventKind::Finish as i32 || evt.kind == AgentEventKind::Error as i32 {
            terminal_seen = true;
            break;
        }
        if count > 64 {
            break;
        }
    }
    drop(stream);

    assert!(terminal_seen, "stream should reach a terminal frame");
    let pending = pending.expect("TOOL_CALL_PENDING frame should arrive at the proxy");
    assert_eq!(pending["call_id"], "call-e2e-1");
    assert_eq!(pending["tool_name"], "fs.write");
    assert_eq!(pending["arguments"]["path"], "artifacts/e2e.txt");
    let result = result.expect("TOOL_RESULT frame should arrive at the proxy");
    assert_eq!(result["tool_name"], "fs.write");
    assert_eq!(
        result["status"], "ok",
        "fs.write should actually execute (policy allows /session/artifacts/**): {result}"
    );

    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
}

#[tokio::test]
async fn destroy_agent_is_idempotent_ok() {
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1016-destroy";
    let _ = proxy.create_agent(sid).await.expect("create_agent");
    // destroy is a no-op stub for Phase 1; the contract is that it
    // returns Ok so saga compensation paths stay clean.
    proxy.destroy_agent(sid).await.expect("destroy_agent ok");
    // Idempotent: a second destroy on the same sid is also Ok.
    proxy
        .destroy_agent(sid)
        .await
        .expect("destroy_agent ok 2nd");
    // Even an unknown sid is Ok (substrate doesn't have a drop API yet).
    proxy
        .destroy_agent("never-existed")
        .await
        .expect("destroy_agent ok unknown");
    env.shutdown().await;
}
