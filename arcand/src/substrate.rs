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
//! Phase 2 (harness arc, 2026-06-10): `DispatchMessage` additionally
//! emits the tool lifecycle — `TOOL_CALL_PENDING` when the model
//! requests a tool and `TOOL_RESULT` when execution completes or
//! fails, each carrying a structured JSON payload (`payload_json`).
//! The kernel already journals + broadcasts these as durable
//! `EventKind::ToolCall*` records during the Direct tick; this module
//! translates them onto the substrate wire. Still future:
//! ApproveDispatch / CancelDispatch and APPROVAL_REQUIRED surfacing.

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
        // Client tool definitions arrive as JSON bytes on
        // `tool_definitions` (additive field — lifed forwards the chat
        // surface's tools). The kernel's tick path executes tools from
        // its own governed registry; merging client-declared tools into
        // the per-session tool surface is a follow-up (the HTTP-backed
        // `ArcanCall` impls in arcan-proxy honour them today). Log so
        // operators can see when a client declared tools that the
        // kernel path does not yet surface to the model.
        if !body.tool_definitions.is_empty() {
            tracing::debug!(
                sid = %sid_proto.value,
                tool_count = body.tool_definitions.len(),
                "dispatch_message: client tool definitions received; kernel \
                 tick uses the registry-driven tool set (client-tool merge \
                 is a follow-up)"
            );
        }

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
                                    payload_json: String::new(),
                                    sequence: 0,
                                }))
                                .await;
                        }
                        errored = true;
                        break;
                    }
                }
            }

            // Give the broadcast pump a bounded window to drain queued
            // frames and forward the kernel's own terminal before we
            // synthesize one. Without this, a starved pump could see
            // our synthesized FINISH win the CAS while real TOKEN /
            // TOOL_RESULT frames are still queued behind it (frames
            // after FINISH). Normal flows break out on the first
            // check; the full window is only paid when the kernel
            // never emitted RunFinished/RunErrored at all.
            if !errored {
                for _ in 0..25 {
                    if terminal_sent.load(Ordering::SeqCst) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
            // If the broadcast pump still hasn't emitted a terminal
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
                        payload_json: String::new(),
                        sequence: 0,
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

/// Cap on the serialized `result` value embedded in a TOOL_RESULT
/// wire frame. Tool outcomes (file reads, shell output) can exceed
/// tonic's default 4 MB receive limit on the proxy side and would
/// kill the stream mid-turn; oversized results are truncated on the
/// wire while the full value stays in the durable kernel journal.
const MAX_TOOL_RESULT_WIRE_BYTES: usize = 64 * 1024;

/// Clamp a tool result for wire transport. Returns the (possibly
/// truncated) value and whether truncation happened.
fn wire_result(result: &serde_json::Value) -> (serde_json::Value, bool) {
    let serialized = result.to_string();
    if serialized.len() <= MAX_TOOL_RESULT_WIRE_BYTES {
        return (result.clone(), false);
    }
    let mut cut = MAX_TOOL_RESULT_WIRE_BYTES;
    while !serialized.is_char_boundary(cut) {
        cut -= 1;
    }
    let truncated = format!(
        "{head}…[truncated {dropped} of {total} bytes; full result in the session journal]",
        head = &serialized[..cut],
        dropped = serialized.len() - cut,
        total = serialized.len(),
    );
    (serde_json::Value::String(truncated), true)
}

/// Translate a kernel `EventRecord` into a substrate-plane
/// `AgentEvent`. Returns `None` for event kinds outside the wire
/// scope (which the pump skips silently).
///
/// Phase 2 (harness arc): the tool lifecycle maps as
/// `ToolCallRequested` → `TOOL_CALL_PENDING` and
/// `ToolCallCompleted` / `ToolCallFailed` → `TOOL_RESULT`, each with
/// a structured JSON payload (see the proto for the shape).
/// `ToolCallStarted` is deliberately skipped — `TOOL_CALL_PENDING`
/// already announced the call and Started carries no additional
/// client-visible information. Every translated frame carries the
/// kernel's durable `record.sequence` so downstream consumers keep
/// real per-session monotonic cursors.
fn translate_event(record: &EventRecord) -> Option<AgentEvent> {
    let event =
        |kind: AgentEventKind, text: String, error: String, payload_json: String| AgentEvent {
            kind: kind as i32,
            text,
            error,
            payload_json,
            sequence: record.sequence,
        };
    match &record.kind {
        EventKind::AssistantTextDelta { delta, .. } | EventKind::TextDelta { delta, .. } => {
            Some(event(
                AgentEventKind::Token,
                delta.clone(),
                String::new(),
                String::new(),
            ))
        }
        EventKind::ToolCallRequested {
            call_id,
            tool_name,
            arguments,
            category,
        } => {
            let mut payload = serde_json::json!({
                "call_id": call_id,
                "tool_name": tool_name,
                "arguments": arguments,
            });
            if let Some(cat) = category {
                payload["category"] = serde_json::Value::String(cat.clone());
            }
            Some(event(
                AgentEventKind::ToolCallPending,
                String::new(),
                String::new(),
                payload.to_string(),
            ))
        }
        EventKind::ToolCallCompleted {
            call_id,
            tool_name,
            result,
            duration_ms,
            status,
            ..
        } => {
            let (result, truncated) = wire_result(result);
            let mut payload = serde_json::json!({
                "call_id": call_id,
                "tool_name": tool_name,
                "result": result,
                "duration_ms": duration_ms,
                "status": status,
            });
            if truncated {
                payload["result_truncated"] = serde_json::Value::Bool(true);
            }
            Some(event(
                AgentEventKind::ToolResult,
                String::new(),
                String::new(),
                payload.to_string(),
            ))
        }
        EventKind::ToolCallFailed {
            call_id,
            tool_name,
            error,
        } => Some(event(
            AgentEventKind::ToolResult,
            String::new(),
            String::new(),
            serde_json::json!({
                "call_id": call_id,
                "tool_name": tool_name,
                "error": error,
                "status": "error",
            })
            .to_string(),
        )),
        EventKind::RunFinished { .. } => Some(event(
            AgentEventKind::Finish,
            String::new(),
            String::new(),
            String::new(),
        )),
        EventKind::RunErrored { error } => Some(event(
            AgentEventKind::Error,
            String::new(),
            error.clone(),
            String::new(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use aios_protocol::{SpanStatus, ToolRunId};

    use super::*;

    fn record(kind: EventKind) -> EventRecord {
        EventRecord::new(
            SessionId::from_string("sid-substrate-test"),
            BranchId::main(),
            1,
            kind,
        )
    }

    fn payload(evt: &AgentEvent) -> serde_json::Value {
        serde_json::from_str(&evt.payload_json).expect("payload_json parses")
    }

    #[test]
    fn translates_text_delta_to_token() {
        let evt = translate_event(&record(EventKind::TextDelta {
            delta: "hello".into(),
            index: None,
        }))
        .expect("token event");
        assert_eq!(evt.kind(), AgentEventKind::Token);
        assert_eq!(evt.text, "hello");
        assert!(evt.payload_json.is_empty());
    }

    #[test]
    fn translates_tool_call_requested_to_pending_with_payload() {
        let evt = translate_event(&record(EventKind::ToolCallRequested {
            call_id: "call-1".into(),
            tool_name: "fs.read".into(),
            arguments: serde_json::json!({"path": "/tmp/x"}),
            category: Some("fs".into()),
        }))
        .expect("pending event");
        assert_eq!(evt.kind(), AgentEventKind::ToolCallPending);
        assert!(evt.text.is_empty());
        let p = payload(&evt);
        assert_eq!(p["call_id"], "call-1");
        assert_eq!(p["tool_name"], "fs.read");
        assert_eq!(p["arguments"]["path"], "/tmp/x");
        assert_eq!(p["category"], "fs");
    }

    #[test]
    fn translates_tool_call_completed_to_result_with_payload() {
        let evt = translate_event(&record(EventKind::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some("call-1".into()),
            tool_name: "fs.read".into(),
            result: serde_json::json!({"content": "data"}),
            duration_ms: 42,
            status: SpanStatus::Ok,
        }))
        .expect("result event");
        assert_eq!(evt.kind(), AgentEventKind::ToolResult);
        let p = payload(&evt);
        assert_eq!(p["call_id"], "call-1");
        assert_eq!(p["tool_name"], "fs.read");
        assert_eq!(p["result"]["content"], "data");
        assert_eq!(p["duration_ms"], 42);
        assert_eq!(p["status"], "ok");
    }

    #[test]
    fn translates_tool_call_failed_to_result_with_error_payload() {
        let evt = translate_event(&record(EventKind::ToolCallFailed {
            call_id: "call-2".into(),
            tool_name: "shell.run".into(),
            error: "denied by policy".into(),
        }))
        .expect("result event");
        assert_eq!(evt.kind(), AgentEventKind::ToolResult);
        let p = payload(&evt);
        assert_eq!(p["call_id"], "call-2");
        assert_eq!(p["error"], "denied by policy");
        assert_eq!(p["status"], "error");
    }

    #[test]
    fn category_is_omitted_when_absent() {
        let evt = translate_event(&record(EventKind::ToolCallRequested {
            call_id: "call-3".into(),
            tool_name: "fs.read".into(),
            arguments: serde_json::json!({}),
            category: None,
        }))
        .expect("pending event");
        let p = payload(&evt);
        assert!(
            p.get("category").is_none(),
            "category key should be omitted, not null"
        );
    }

    #[test]
    fn kernel_sequence_passes_through_to_the_wire() {
        let rec = EventRecord::new(
            SessionId::from_string("sid-substrate-test"),
            BranchId::main(),
            7,
            EventKind::TextDelta {
                delta: "x".into(),
                index: None,
            },
        );
        let evt = translate_event(&rec).expect("token event");
        assert_eq!(evt.sequence, 7);
    }

    #[test]
    fn oversized_tool_result_is_truncated_on_the_wire() {
        let big = "x".repeat(MAX_TOOL_RESULT_WIRE_BYTES * 2);
        let evt = translate_event(&record(EventKind::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some("call-big".into()),
            tool_name: "fs.read".into(),
            result: serde_json::json!({ "content": big }),
            duration_ms: 1,
            status: SpanStatus::Ok,
        }))
        .expect("result event");
        assert!(
            evt.payload_json.len() < MAX_TOOL_RESULT_WIRE_BYTES + 4096,
            "wire payload stays near the cap (got {})",
            evt.payload_json.len()
        );
        let p = payload(&evt);
        assert_eq!(p["result_truncated"], true);
        assert!(
            p["result"]
                .as_str()
                .expect("truncated result is a string")
                .contains("truncated"),
        );
    }

    #[test]
    fn tool_call_started_is_skipped() {
        let translated = translate_event(&record(EventKind::ToolCallStarted {
            tool_run_id: ToolRunId::default(),
            tool_name: "fs.read".into(),
        }));
        assert!(translated.is_none());
    }

    #[test]
    fn terminal_kinds_still_translate() {
        let finish = translate_event(&record(EventKind::RunFinished {
            reason: "done".into(),
            total_iterations: 1,
            final_answer: None,
            usage: None,
        }));
        assert!(matches!(
            finish.map(|e| e.kind()),
            Some(AgentEventKind::Finish)
        ));
        let errored = translate_event(&record(EventKind::RunErrored {
            error: "boom".into(),
        }))
        .expect("error event");
        assert_eq!(errored.kind(), AgentEventKind::Error);
        assert_eq!(errored.error, "boom");
    }
}
