//! Substrate-plane gRPC service for arcand.
//!
//! Implements `arcan.v1.AgentSubstrate` (defined in
//! `proto/arcan/v1/substrate.proto`, generated in
//! `arcan-substrate-proto`). This is the UDS-bound entry point that
//! lifed reaches via `arcan-proxy` under Topology B. It is ADDITIVE
//! to arcand's existing HTTP `:3000` server (Topology A) — both can
//! run concurrently.
//!
//! Phase 1 scope (BRO-1016):
//! - `CreateAgent`: idempotent session-create over `KernelRuntime`.
//! - `DestroyAgent`: idempotent no-op stub (no drop-session API yet).
//! - `DispatchMessage`: drives a Direct tick loop and streams text +
//!   terminal events back to the caller.
//!
//! Phase 2 (separate ticket) will lift the full `life.v1.AgentEvent`
//! shape, ToolCall events, ApproveDispatch / CancelDispatch, etc.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aios_protocol::{
    BranchId, EventKind, EventRecord, ModelRouting, OperatingMode, PolicySet, SessionId,
};
use aios_runtime::{KernelRuntime, TickInput, TickKind};
use arcan_substrate_proto::arcan::v1::{
    AgentEvent, AgentEventKind, CreateAgentReq, CreateAgentResp, DestroyAgentReq, DestroyAgentResp,
    DispatchMessageReq, agent_substrate_server::AgentSubstrate,
};
use futures_util::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

// Max ticks per DispatchMessage. Matches `arcand::canonical::run_session`'s
// MAX_AGENT_ITERATIONS so behavior is consistent between Topology A and
// Topology B entry points.
const MAX_AGENT_ITERATIONS: u32 = 10;

// Bounded event channel for the streaming dispatch. Phase-1 emits text +
// terminal frames; 64 is sufficient headroom for the slowest reader.
const DISPATCH_CHANNEL_CAPACITY: usize = 64;

/// arcand's `arcan.v1.AgentSubstrate` impl. Holds a shared
/// `KernelRuntime` handle so every RPC reuses the same in-memory
/// session store, journal, and tick engine that the HTTP plane is
/// driving (Topology A) — `arcand` is internally single-runtime.
pub struct SubstrateService {
    runtime: Arc<KernelRuntime>,
}

impl SubstrateService {
    pub fn new(runtime: Arc<KernelRuntime>) -> Self {
        Self { runtime }
    }
}

#[tonic::async_trait]
impl AgentSubstrate for SubstrateService {
    type DispatchMessageStream =
        Pin<Box<dyn Stream<Item = Result<AgentEvent, Status>> + Send + 'static>>;

    async fn create_agent(
        &self,
        req: Request<CreateAgentReq>,
    ) -> Result<Response<CreateAgentResp>, Status> {
        let body = req.into_inner();
        let sid_proto = body
            .sid
            .ok_or_else(|| Status::invalid_argument("missing sid"))?;
        if sid_proto.value.is_empty() {
            return Err(Status::invalid_argument("empty sid"));
        }
        let session_id = SessionId::from_string(&sid_proto.value);

        // Idempotent: if the session already exists, return the same
        // agent_id (the sid itself in Phase 1 — see proto comment for
        // the 1:1 invariant).
        if !self.runtime.session_exists(&session_id) {
            // The `label` field is informational only — it is forwarded
            // into the owner string so it shows up in the session
            // manifest for operator introspection.
            let owner = if body.label.is_empty() {
                "lifed-routed".to_string()
            } else {
                format!("lifed-routed:{label}", label = body.label)
            };
            self.runtime
                .create_session_with_id(
                    session_id.clone(),
                    owner,
                    PolicySet::default(),
                    ModelRouting::default(),
                )
                .await
                .map_err(|e| {
                    tracing::warn!(sid = %sid_proto.value, error = %e, "create_session_with_id failed");
                    Status::internal(format!("create_session_with_id: {e}"))
                })?;
        }

        Ok(Response::new(CreateAgentResp {
            agent_id: sid_proto.value,
            created_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
        }))
    }

    async fn destroy_agent(
        &self,
        req: Request<DestroyAgentReq>,
    ) -> Result<Response<DestroyAgentResp>, Status> {
        // TODO(BRO-1016 Phase 2): KernelRuntime has no explicit
        // drop-session API today — sessions are in-memory + journaled
        // and survive for replay until process restart. This handler
        // returns Ok(empty) so saga compensation paths stay clean
        // (Spec C₂ §4.2 reverse compensation needs the call to be
        // idempotent and non-fatal). A future ticket should add
        // `KernelRuntime::destroy_session` and route here.
        let _ = req.into_inner();
        Ok(Response::new(DestroyAgentResp {}))
    }

    async fn dispatch_message(
        &self,
        req: Request<DispatchMessageReq>,
    ) -> Result<Response<Self::DispatchMessageStream>, Status> {
        let body = req.into_inner();
        let sid_proto = body
            .sid
            .ok_or_else(|| Status::invalid_argument("missing sid"))?;
        if sid_proto.value.is_empty() {
            return Err(Status::invalid_argument("empty sid"));
        }
        let session_id = SessionId::from_string(&sid_proto.value);
        if !self.runtime.session_exists(&session_id) {
            return Err(Status::failed_precondition(format!(
                "session not found: {sid}",
                sid = sid_proto.value
            )));
        }

        let runtime = Arc::clone(&self.runtime);
        let content = body.content;

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Subscribe to the broadcast stream BEFORE issuing the first
        // tick — events emitted during the tick get captured. Note:
        // `subscribe_events` returns a single subscriber that sees
        // every session's events; we filter by `session_id` in the
        // pump task so cross-session traffic doesn't leak into this
        // stream.
        let mut events_rx = runtime.subscribe_events();

        tokio::spawn(async move {
            // Shared flag: either the broadcast pump or the tick-driver
            // task may produce the terminal event. The first one wins;
            // the loser silently exits without double-emitting.
            let terminal_sent = Arc::new(AtomicBool::new(false));

            // Run the tick loop in the background while a sibling
            // task drains the event broadcast and forwards filtered
            // events to the streaming client.
            let session_for_pump = session_id.clone();
            let tx_pump = tx.clone();
            let terminal_for_pump = Arc::clone(&terminal_sent);
            let pump_handle = tokio::spawn(async move {
                loop {
                    match events_rx.recv().await {
                        Ok(record) => {
                            if record.session_id != session_for_pump {
                                continue;
                            }
                            if let Some(evt) = translate_event(&record) {
                                let is_terminal = matches!(
                                    evt.kind(),
                                    AgentEventKind::Finish | AgentEventKind::Error
                                );
                                if is_terminal && terminal_for_pump.swap(true, Ordering::SeqCst) {
                                    // A terminal was already emitted by
                                    // the driver — drop this one and stop.
                                    break;
                                }
                                if tx_pump.send(Ok(evt)).await.is_err() {
                                    // Receiver dropped — stop pumping.
                                    break;
                                }
                                if is_terminal {
                                    break;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                sid = %session_for_pump,
                                skipped,
                                "dispatch event pump lagged; some events skipped"
                            );
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Drive the tick loop. We perform one initial tick with
            // the caller's content as the objective; subsequent ticks
            // continue with empty objectives so the model sees the
            // tool results and can respond (mirrors
            // `canonical.rs::run_session`).
            let mut errored = false;
            for iteration in 0..MAX_AGENT_ITERATIONS {
                let objective = if iteration == 0 {
                    content.clone()
                } else {
                    String::new()
                };
                let tick_input = TickInput {
                    objective,
                    proposed_tool: None,
                    system_prompt: None,
                    allowed_tools: None,
                    kind: TickKind::Direct,
                };
                match runtime
                    .tick_on_branch(&session_id, &BranchId::main(), tick_input)
                    .await
                {
                    Ok(output) => {
                        // Continue the loop only when the model executed
                        // tools and needs another call to see their
                        // results — same predicate as the HTTP path.
                        if !matches!(output.mode, OperatingMode::Execute) {
                            break;
                        }
                    }
                    Err(e) => {
                        if !terminal_sent.swap(true, Ordering::SeqCst) {
                            let _ = tx
                                .send(Ok(AgentEvent {
                                    kind: AgentEventKind::Error as i32,
                                    text: String::new(),
                                    error: format!("tick failed: {e}"),
                                }))
                                .await;
                        }
                        errored = true;
                        break;
                    }
                }
            }

            // If the broadcast pump hasn't already emitted a terminal
            // (e.g. the test harness's MockProvider returns
            // immediately without a RunFinished event), synthesize a
            // FINISH so the client stream closes cleanly. CAS guards
            // against double-emit when the broadcast wins the race.
            if !errored && !terminal_sent.swap(true, Ordering::SeqCst) {
                let _ = tx
                    .send(Ok(AgentEvent {
                        kind: AgentEventKind::Finish as i32,
                        text: String::new(),
                        error: String::new(),
                    }))
                    .await;
            }
            // Drop our sender; the pump-side sender is held by
            // pump_handle's task and drops when it exits. We then
            // await the pump so the task exits before this future
            // resolves — ReceiverStream::next() returns None once all
            // senders are dropped.
            drop(tx);
            let _ = pump_handle.await;
        });

        let stream = ReceiverStream::new(rx);
        Ok(Response::new(
            Box::pin(stream) as Self::DispatchMessageStream
        ))
    }
}

/// Translate a kernel `EventRecord` into a substrate-plane
/// `AgentEvent`. Returns `None` for event kinds outside the Phase 1
/// scope (which the pump skips silently). Phase 2 will expand to
/// ToolCall + structured event records.
fn translate_event(record: &EventRecord) -> Option<AgentEvent> {
    match &record.kind {
        EventKind::AssistantTextDelta { delta, .. } | EventKind::TextDelta { delta, .. } => {
            Some(AgentEvent {
                kind: AgentEventKind::Token as i32,
                text: delta.clone(),
                error: String::new(),
            })
        }
        EventKind::RunFinished { .. } => Some(AgentEvent {
            kind: AgentEventKind::Finish as i32,
            text: String::new(),
            error: String::new(),
        }),
        EventKind::RunErrored { error } => Some(AgentEvent {
            kind: AgentEventKind::Error as i32,
            text: String::new(),
            error: error.clone(),
        }),
        _ => None,
    }
}
