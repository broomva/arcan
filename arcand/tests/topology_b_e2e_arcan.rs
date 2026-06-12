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
    ApprovalPort, BranchId, Capability, EventStorePort, KernelResult, ModelCompletion,
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
        .dispatch_message(sid, "Hello, substrate!", None, "", &[])
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
        .dispatch_message(sid, "write the e2e artifact", None, "", &[])
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

/// Provider that proposes a CLIENT tool (one declared via
/// `tool_definitions`, absent from the kernel registry) on its first
/// completion. A second completion would answer normally — but the
/// client-tool handoff must END the dispatch after the first call, so
/// the test asserts the counter never reaches 2.
#[derive(Debug, Default)]
struct ClientToolProvider {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl ModelProviderPort for ClientToolProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        // The merge contract: the provider-visible request must carry
        // the client tool defs the dispatch received on the wire.
        assert!(
            request.client_tools.iter().any(|t| t.name == "get_weather"),
            "client tool defs must reach the provider request"
        );
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(ModelCompletion {
                provider: "test".to_owned(),
                model: "test-model".to_owned(),
                llm_call_record: None,
                directives: vec![ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: "call-client-1".to_owned(),
                        tool_name: "get_weather".to_owned(),
                        input: serde_json::json!({ "city": "Medellín" }),
                        requested_capabilities: vec![],
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
                    content: "should never be asked".to_owned(),
                }],
                stop_reason: ModelStopReason::Completed,
                usage: None,
                final_answer: Some("unexpected".to_owned()),
            })
        }
    }
}

#[tokio::test]
async fn dispatch_message_hands_off_client_tool_calls() {
    // The #1697-completion contract on the kernel path: client tool
    // defs ride DispatchMessageReq.tool_definitions → the model sees
    // them → a proposal of a client tool surfaces as TOOL_CALL_PENDING
    // with category "client" and the turn ENDS (FINISH) without kernel
    // execution — the chat surface executes the tool and continues via
    // replayed history on its next dispatch.
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
    let dispatcher = Arc::new(ToolDispatcher::new(
        Arc::clone(&registry),
        policy_engine,
        sandbox,
    ));
    let tool_harness: Arc<dyn ToolHarnessPort> = dispatcher;
    let provider_calls = Arc::new(AtomicU32::new(0));
    let provider = ClientToolProvider {
        calls: Arc::clone(&provider_calls),
    };
    // Mirror `arcan serve`: declare the registry's tool names so the
    // kernel can enforce registry-wins on client-tool collisions (the
    // foot-gun the empty default would hide — see
    // `KernelRuntime::with_registry_tool_names`).
    let registry_tool_names: Vec<String> =
        registry.definitions().map(|def| def.name.clone()).collect();
    let runtime = Arc::new(
        KernelRuntime::new(
            RuntimeConfig::new(root),
            event_store,
            Arc::new(provider),
            tool_harness,
            approvals,
            policy_gate,
        )
        .with_registry_tool_names(registry_tool_names),
    );

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
    let sid = "client-tool-handoff";
    let _ = proxy.create_agent(sid).await.expect("create_agent");

    let tools = vec![serde_json::json!({
        "name": "get_weather",
        "description": "Get the weather for a city",
        "parameters": {
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }
    })];
    let mut stream = proxy
        .dispatch_message(sid, "what's the weather in Medellín?", None, "", &tools)
        .await
        .expect("dispatch_message");

    let mut pending: Option<serde_json::Value> = None;
    let mut saw_tool_result = false;
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
            saw_tool_result = true;
        }
        if evt.kind == AgentEventKind::Finish as i32 || evt.kind == AgentEventKind::Error as i32 {
            terminal_seen = true;
            assert_eq!(
                evt.kind,
                AgentEventKind::Finish as i32,
                "client-tool handoff must FINISH cleanly, not ERROR"
            );
            break;
        }
        if count > 64 {
            break;
        }
    }
    drop(stream);

    assert!(terminal_seen, "stream should reach a terminal frame");
    let pending = pending.expect("TOOL_CALL_PENDING frame should arrive at the proxy");
    assert_eq!(pending["call_id"], "call-client-1");
    assert_eq!(pending["tool_name"], "get_weather");
    assert_eq!(
        pending["category"], "client",
        "handoff frames must be marked category=client: {pending}"
    );
    assert_eq!(pending["arguments"]["city"], "Medellín");
    assert!(
        !saw_tool_result,
        "client tools are client-executed — the kernel must not emit TOOL_RESULT"
    );
    assert_eq!(
        provider_calls.load(Ordering::SeqCst),
        1,
        "the handoff ends the turn — no follow-up provider call"
    );

    let _ = shutdown_tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(5), server_handle).await;
}

#[tokio::test]
async fn dispatch_message_on_branch_lands_under_that_branch() {
    // BRO-1479: a dispatch with a non-empty `branch` auto-forks the
    // branch from main at the current head (sessions are born with only
    // `main`; the kernel's `next_sequence` requires the branch to exist)
    // and keys every tick onto it. Assert the events land under `exp-1`
    // AND that main's event stream is unchanged by the dispatch, proving
    // the branch threads end-to-end through the substrate wire (not a
    // hardcoded `BranchId::main()`).
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1479-branch";
    let _ = proxy.create_agent(sid).await.expect("create_agent");

    // Snapshot main BEFORE the branch dispatch: session creation seeds
    // events on main, and the fork must not add to them.
    let session_id = SessionId::from_string(sid);
    let main_before = env
        .runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1000)
        .await
        .expect("read main before")
        .len();

    let mut stream = proxy
        .dispatch_message(sid, "Hello, exp branch!", None, "exp-1", &[])
        .await
        .expect("dispatch_message on branch");

    let mut terminal_seen = false;
    let mut count = 0;
    while let Some(evt) = stream.next().await {
        let evt = evt.expect("substrate event ok");
        count += 1;
        if evt.kind == AgentEventKind::Finish as i32 || evt.kind == AgentEventKind::Error as i32 {
            terminal_seen = true;
            break;
        }
        if count > 32 {
            break;
        }
    }
    drop(stream);
    assert!(terminal_seen, "dispatch on branch should reach a terminal");

    let exp_branch = BranchId::from_string("exp-1");
    let on_branch = env
        .runtime
        .read_events_on_branch(&session_id, &exp_branch, 0, 1000)
        .await
        .expect("read exp-1 events");
    assert!(
        !on_branch.is_empty(),
        "dispatch on exp-1 should have journaled events under that branch"
    );
    assert!(
        on_branch.iter().all(|e| e.branch_id == exp_branch),
        "every journaled event should carry the exp-1 branch id"
    );
    // The auto-fork is itself journaled ON the new branch — its first
    // event is the BranchCreated marker carrying the fork point.
    assert!(
        matches!(
            on_branch.first().map(|e| &e.kind),
            Some(aios_protocol::EventKind::BranchCreated { .. })
        ),
        "the forked branch's first event should be BranchCreated"
    );

    // The sibling main branch must be unchanged — the dispatch forked
    // off it without writing to it.
    let main_after = env
        .runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1000)
        .await
        .expect("read main events");
    assert_eq!(
        main_after.len(),
        main_before,
        "dispatching on exp-1 must not add events to main"
    );

    env.shutdown().await;
}

#[tokio::test]
async fn dispatch_message_default_branch_lands_on_main() {
    // BRO-1479 backward-compat: an empty branch behaves exactly as
    // before — events land on main. This mirrors the pre-existing
    // `dispatch_message_streams_real_substrate_events` contract, now
    // pinned against the branch field so the default can never regress.
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1479-default-main";
    let _ = proxy.create_agent(sid).await.expect("create_agent");

    let mut stream = proxy
        .dispatch_message(sid, "Hello, default branch!", None, "", &[])
        .await
        .expect("dispatch_message default branch");

    let mut terminal_seen = false;
    let mut count = 0;
    while let Some(evt) = stream.next().await {
        let evt = evt.expect("substrate event ok");
        count += 1;
        if evt.kind == AgentEventKind::Finish as i32 || evt.kind == AgentEventKind::Error as i32 {
            terminal_seen = true;
            break;
        }
        if count > 32 {
            break;
        }
    }
    drop(stream);
    assert!(terminal_seen, "default dispatch should reach a terminal");

    let session_id = SessionId::from_string(sid);
    let on_main = env
        .runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1000)
        .await
        .expect("read main events");
    assert!(
        !on_main.is_empty(),
        "an empty branch must journal events on main"
    );
    assert!(
        on_main.iter().all(|e| e.branch_id == BranchId::main()),
        "every journaled event should carry the main branch id"
    );

    env.shutdown().await;
}

#[tokio::test]
async fn dispatch_message_invalid_branch_name_is_rejected() {
    // BRO-1479: the branch is UNTRUSTED remote input that keys directly
    // into redb compound keys + lago-fs manifests. An invalid name is
    // rejected with INVALID_ARGUMENT at the substrate trust boundary,
    // never sanitized silently.
    let env = SubstrateUnderTest::start().await;
    let proxy = ArcanProxy::connect(env.socket.clone())
        .await
        .expect("dial substrate UDS");

    let sid = "bro-1479-invalid-branch";
    let _ = proxy.create_agent(sid).await.expect("create_agent");

    // Session creation itself journals events on main — snapshot the
    // count so the assertion below isolates what the REJECTED dispatch
    // added (which must be nothing).
    let session_id = SessionId::from_string(sid);
    let main_before = env
        .runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1000)
        .await
        .expect("read main before")
        .len();

    // A slash is outside `[a-zA-Z0-9_-]` — must be rejected before any
    // tick runs.
    let err = proxy
        .dispatch_message(sid, "should be rejected", None, "../etc/passwd", &[])
        .await
        .err()
        .expect("invalid branch name must error");
    // The proxy maps the gRPC Status into its own error type; the
    // important contract is that the dispatch did NOT succeed and
    // journaled nothing beyond the pre-existing session events.
    let _ = err;

    let main_after = env
        .runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1000)
        .await
        .expect("read main events");
    assert_eq!(
        main_after.len(),
        main_before,
        "a rejected dispatch must not journal anything: {main_after:?}"
    );

    env.shutdown().await;
}
