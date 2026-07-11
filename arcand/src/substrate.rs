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
    BranchId, ClientToolDefinition, EventKind, EventRecord, ModelRouting, OperatingMode, PolicySet,
    SessionId,
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

// Trust-boundary caps for client-supplied tool definitions. These are
// UNTRUSTED remote input (the chat surface forwards whatever a client
// declares) and are re-serialized into the provider request on every
// tick of a dispatch — without caps a remote user can inflate every
// model call (cost/context amplification) or smuggle a name the
// provider rejects (turning every tick into a wire ERROR). Limits are
// deliberately generous: real chat surfaces ship ~20 tools with ~1-4KB
// schemas. OpenAI's function-name charset is `[a-zA-Z0-9_-]{1,64}`.
const MAX_CLIENT_TOOL_DEFS: usize = 64;
const MAX_CLIENT_TOOL_DEF_BYTES: usize = 16 * 1024;

/// Provider-portable tool-name check (`[a-zA-Z0-9_-]{1,64}`).
fn valid_client_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Branch-name check (`[a-zA-Z0-9_-]{1,64}`). The dispatch branch is
/// UNTRUSTED remote input that keys directly into redb compound keys
/// (session + branch + seq) and lago-fs manifest keys, so it is
/// validated at this trust boundary and rejected — never sanitized
/// silently. The charset matches the provider-portable tool-name rule
/// for consistency; `main` and other simple identifiers pass.
fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

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

        // Resolve the target branch at this trust boundary (BRO-1479).
        // Empty ⇒ "main" (backward-compatible: pre-BRO-1479 callers
        // never set the field, so the dispatch keys onto main exactly
        // as before). A non-empty value is validated before it is
        // allowed to key into redb compound keys + lago-fs manifests —
        // an invalid name is rejected, never sanitized silently.
        let branch = if body.branch.is_empty() {
            BranchId::main()
        } else if valid_branch_name(&body.branch) {
            BranchId::from_string(&body.branch)
        } else {
            return Err(Status::invalid_argument(format!(
                "invalid branch name {branch:?}: must match [a-zA-Z0-9_-]{{1,64}}",
                branch = body.branch
            )));
        };

        let runtime = Arc::clone(&self.runtime);

        // BRO-1466: derive the tier tool surface from the session's policy
        // so the model only sees tools it can actually use. Visibility is a
        // pre-filter (any-grant based — see `tools_allowed_by_policy`); the
        // policy gate remains the authoritative enforcement at execution
        // time. `None` (unknown session policy, or a fully-permissive one)
        // ⇒ the full catalog, exactly as before this wiring.
        let allowed_tools = runtime
            .session_policy(&session_id)
            .and_then(|policy| arcan_aios_adapters::tools_allowed_by_policy(&policy, None));

        // Auto-fork semantics (BRO-1479): the kernel requires a branch to
        // EXIST before any tick can sequence events onto it
        // (`next_sequence` bails "branch not found"). Sessions are born
        // with only `main`, so a dispatch naming an unknown branch forks
        // it from main at the CURRENT head — the natural "this dispatch
        // explores a fork of the session" semantic, and it keeps the wire
        // ergonomic (no separate create-branch RPC for the common case).
        // An existing branch is reused as-is; a merged (read-only) branch
        // fails inside the tick with the kernel's own clear error.
        if branch != BranchId::main() {
            let branch_exists = |branches: Vec<aios_protocol::BranchInfo>| {
                branches.iter().any(|info| info.branch_id == branch)
            };
            let known = runtime
                .list_branches(&session_id)
                .await
                .map(branch_exists)
                .map_err(|e| Status::internal(format!("list_branches: {e}")))?;
            if !known
                && let Err(create_err) = runtime
                    .create_branch(&session_id, branch.clone(), None, None)
                    .await
            {
                // Idempotent under concurrency: two dispatches naming the
                // same new branch can both observe "unknown" and race the
                // fork — the loser's create fails ("branch already
                // exists") although the branch is now perfectly usable.
                // Re-check existence instead of string-matching the error;
                // only a still-missing branch is a real failure.
                let now_known = runtime
                    .list_branches(&session_id)
                    .await
                    .map(branch_exists)
                    .unwrap_or(false);
                if !now_known {
                    return Err(Status::internal(format!("create_branch: {create_err}")));
                }
            }
        }

        let content = body.content;
        // Client tool definitions arrive as JSON bytes on
        // `tool_definitions` (additive field — lifed forwards the chat
        // surface's tools, each entry one JSON object in the OpenAI
        // function shape `{"name","description","parameters"}`). Parse
        // them at this trust boundary: a malformed entry is warned and
        // skipped, never aborting an otherwise-valid turn. The parsed
        // set is surfaced to the model on every tick of this dispatch;
        // the kernel enforces registry-wins on name collisions and, when
        // the model proposes a client tool, hands the call back to the
        // caller as TOOL_CALL_PENDING (category "client") instead of
        // executing it through the harness.
        let client_tools: Vec<ClientToolDefinition> = if body.tool_definitions.is_empty() {
            Vec::new()
        } else {
            let total = body.tool_definitions.len();
            if total > MAX_CLIENT_TOOL_DEFS {
                tracing::warn!(
                    sid = %sid_proto.value,
                    total,
                    cap = MAX_CLIENT_TOOL_DEFS,
                    "dispatch_message: client tool definitions exceed cap; truncating"
                );
            }
            let parsed: Vec<ClientToolDefinition> = body
                .tool_definitions
                .iter()
                .take(MAX_CLIENT_TOOL_DEFS)
                .filter_map(|bytes| {
                    if bytes.len() > MAX_CLIENT_TOOL_DEF_BYTES {
                        tracing::warn!(
                            sid = %sid_proto.value,
                            bytes = bytes.len(),
                            cap = MAX_CLIENT_TOOL_DEF_BYTES,
                            "dispatch_message: skipping oversized client tool definition"
                        );
                        return None;
                    }
                    match ClientToolDefinition::from_wire_bytes(bytes) {
                        Ok(def) if !valid_client_tool_name(&def.name) => {
                            tracing::warn!(
                                sid = %sid_proto.value,
                                name = %def.name,
                                "dispatch_message: skipping client tool with provider-unsafe name"
                            );
                            None
                        }
                        Ok(def) => Some(def),
                        Err(err) => {
                            tracing::warn!(
                                sid = %sid_proto.value,
                                error = %err,
                                "dispatch_message: skipping malformed client tool definition"
                            );
                            None
                        }
                    }
                })
                .collect();
            tracing::info!(
                sid = %sid_proto.value,
                accepted = parsed.len(),
                skipped = total - parsed.len(),
                "dispatch_message: client tool definitions parsed; surfacing to model \
                 (registry tools win on collision; client tools are client-executed)"
            );
            parsed
        };

        let (tx, rx) = mpsc::channel::<Result<AgentEvent, Status>>(DISPATCH_CHANNEL_CAPACITY);

        // Subscribe to the broadcast stream BEFORE issuing the first
        // tick — events emitted during the tick get captured. Note:
        // `subscribe_events` returns a single subscriber that sees
        // every session's events; we filter by `session_id` AND
        // `branch_id` in the pump task so neither cross-session traffic
        // nor a concurrent dispatch on a SIBLING branch of the same
        // session leaks into this stream (BRO-1479: `EventRecord`
        // carries `branch_id`, so cross-branch frames are filtered
        // rather than interleaved).
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
            let branch_for_pump = branch.clone();
            let tx_pump = tx.clone();
            let terminal_for_pump = Arc::clone(&terminal_sent);
            // BRO-1322: consume-side stream-lag metrics for the substrate bus.
            // Shared bundle from the runtime so drains/lags land under the same
            // `arcan.substrate` channel label the send side publishes under.
            let pump_metrics = runtime.stream_metrics().clone();
            let pump_handle = tokio::spawn(async move {
                loop {
                    match events_rx.recv().await {
                        Ok(record) => {
                            // Count every drain (pre-filter): this is what the
                            // dispatch pump pulled off the channel, regardless
                            // of whether it belongs to this session/branch.
                            let event_type = record.kind.variant_name();
                            pump_metrics.on_consumed("substrate-dispatch", Some(event_type));
                            let latency_ms =
                                (chrono::Utc::now() - record.timestamp).num_milliseconds();
                            if latency_ms >= 0 {
                                pump_metrics.record_delta_latency(
                                    Some(event_type),
                                    latency_ms as f64 / 1000.0,
                                );
                            }
                            if record.session_id != session_for_pump
                                || record.branch_id != branch_for_pump
                            {
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
                            // BRO-1322: the silent-degradation surface made
                            // visible — a backed-up dispatch pump now increments
                            // lagged_total + skipped_messages_total instead of
                            // only logging.
                            pump_metrics.on_lagged("substrate-dispatch", skipped);
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
            // BRO-1465: a tick that finalizes `Recover` (tool denial or
            // execution failure) used to END the dispatch here — the model
            // never got a call after the failure event, so the user saw
            // dead air. Grant ONE wrap-up iteration per dispatch: the
            // failure is rendered into conversation history (the kernel's
            // tool transcript), so the follow-up call can verbalize what
            // happened. The flag caps Recover→Recover chains at one.
            let mut recover_wrapup_used = false;
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
                    // Tier tool surface derived from the session policy
                    // (BRO-1466) — pre-filter only; the gate still
                    // enforces scope at execution time.
                    allowed_tools: allowed_tools.clone(),
                    // Surfaced on every tick of this dispatch so the model
                    // keeps seeing the client tools across the multi-tick
                    // loop (e.g. a registry tool runs, then the model
                    // proposes a client tool on the follow-up call).
                    client_tools: client_tools.clone(),
                    kind: TickKind::Direct,
                };
                match runtime
                    .tick_on_branch(&session_id, &branch, tick_input)
                    .await
                {
                    Ok(output) => {
                        // Continue the loop only when the tick actually
                        // evaluated registry tool calls — the model needs
                        // another call to see their results. `mode ==
                        // Execute` alone is NOT the signal: it is also the
                        // homeostatic default for text-only ticks, and
                        // looping on it burned 4-5 wasted model calls per
                        // chat turn (prod, 2026-06-12). A Recover tick
                        // still gets ONE wrap-up call so failures are
                        // verbalized instead of dead air (BRO-1465).
                        match output.mode {
                            OperatingMode::Execute if output.tool_calls_executed > 0 => {}
                            OperatingMode::Recover if !recover_wrapup_used => {
                                recover_wrapup_used = true;
                            }
                            _ => break,
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
