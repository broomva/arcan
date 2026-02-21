use crate::commands::{self, CommandResult};
use crate::r#loop::AgentLoop;
use aios_protocol::{
    ActorType, AgentId, ApprovalDecision, ApprovalId, BranchId as AiosBranchId, EventActor,
    EventId as AiosEventId, EventKind as AiosEventKind, EventRecord as AiosEventRecord,
    EventSchema, RiskLevel, SessionId as AiosSessionId, SpanStatus, ToolRunId,
};
use arcan_core::aisdk::{UiStreamPart, to_ui_stream_parts};
use arcan_core::protocol::{AgentEvent, ChatMessage, ModelStopReason, Role, RunStopReason};
use arcan_core::runtime::{ApprovalResolver, Orchestrator};
use arcan_core::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

/// Typed error for Axum handlers with proper HTTP status codes.
pub enum AppError {
    BadRequest(String),
    NotFound(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err.to_string())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub session_id: String,
    #[serde(default)]
    pub branch_id: Option<String>,
    pub message: String,
}

#[derive(Deserialize, Default)]
pub struct ChatQuery {
    /// SSE format: "arcan" (default), "aisdk_v6" (v6 UI Message Stream), or "aisdk_v5" (deprecated alias)
    #[serde(default)]
    pub format: Option<String>,
}

pub(crate) struct ServerState {
    pub(crate) agent_loop: Arc<AgentLoop>,
    pub(crate) orchestrator: Arc<Orchestrator>,
    pub(crate) approval_resolver: Option<Arc<dyn ApprovalResolver>>,
}

pub async fn create_router(agent_loop: Arc<AgentLoop>) -> Router {
    // Create a dummy orchestrator for backward compat (tests)
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(crate::mock::MockProvider),
        arcan_core::runtime::ToolRegistry::default(),
        Vec::new(),
        arcan_core::runtime::OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    create_router_full(agent_loop, orchestrator, None).await
}

pub async fn create_router_with_approvals(
    agent_loop: Arc<AgentLoop>,
    approval_resolver: Option<Arc<dyn ApprovalResolver>>,
) -> Router {
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(crate::mock::MockProvider),
        arcan_core::runtime::ToolRegistry::default(),
        Vec::new(),
        arcan_core::runtime::OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    create_router_full(agent_loop, orchestrator, approval_resolver).await
}

pub async fn create_router_full(
    agent_loop: Arc<AgentLoop>,
    orchestrator: Arc<Orchestrator>,
    approval_resolver: Option<Arc<dyn ApprovalResolver>>,
) -> Router {
    let state = Arc::new(ServerState {
        agent_loop,
        orchestrator,
        approval_resolver,
    });

    Router::new()
        .route("/health", get(health_handler))
        // Canonical MVP API surface
        .route("/v1/sessions/{session_id}/runs", post(v1_runs_handler))
        .route("/v1/sessions/{session_id}/signals", post(v1_signals_handler))
        .route("/v1/sessions/{session_id}/state", get(v1_state_handler))
        .route("/v1/sessions/{session_id}/stream", get(v1_stream_handler))
        // Legacy route (kept for transitional compatibility in tests/clients)
        .route("/chat", post(chat_handler))
        .route("/model", get(get_model_handler))
        .route("/model", post(set_model_handler))
        .route("/approve", post(approve_handler))
        .route("/approvals", get(list_approvals_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

fn default_branch() -> String {
    "main".to_string()
}

fn parse_last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.parse::<u64>().ok())
}

#[cfg(test)]
mod header_tests {
    use super::parse_last_event_id;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn parse_last_event_id_accepts_plain_id() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("42"));
        assert_eq!(parse_last_event_id(&headers), Some(42));
    }

    #[test]
    fn parse_last_event_id_accepts_dotted_id() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("42.1"));
        assert_eq!(parse_last_event_id(&headers), Some(42));
    }

    #[test]
    fn parse_last_event_id_rejects_invalid_id() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("abc"));
        assert_eq!(parse_last_event_id(&headers), None);
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1RunRequest {
    pub message: String,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1SignalRequest {
    pub signal_type: String,
    #[serde(default)]
    pub data: serde_json::Value,
    #[serde(default)]
    pub branch: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct V1StateQuery {
    #[serde(default = "default_branch")]
    pub branch: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct V1StreamQuery {
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default)]
    pub from_version: Option<u64>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct V1StateResponse {
    pub version: u64,
    pub state: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamFormat {
    CanonicalV1,
    VercelAiSdkV6,
}

fn parse_stream_format(value: Option<&str>) -> StreamFormat {
    match value {
        Some("vercel_ai_sdk_v6" | "aisdk_v6") => StreamFormat::VercelAiSdkV6,
        _ => StreamFormat::CanonicalV1,
    }
}

fn replay_records(records: &[arcan_store::session::EventRecord]) -> (AppState, Vec<ChatMessage>) {
    let mut state = AppState::default();
    let mut messages: Vec<ChatMessage> = Vec::new();

    for record in records {
        match &record.event {
            AgentEvent::StatePatched { patch, .. } => {
                let _ = state.apply_patch(patch);
            }
            AgentEvent::TextDelta { delta, .. } => {
                if let Some(last) = messages.last_mut() {
                    if last.role == Role::Assistant {
                        last.content.push_str(delta);
                    } else {
                        messages.push(ChatMessage::assistant(delta.clone()));
                    }
                } else {
                    messages.push(ChatMessage::assistant(delta.clone()));
                }
            }
            AgentEvent::ToolCallCompleted { result, .. } => {
                let output_str =
                    serde_json::to_string(&result.output).unwrap_or_else(|_| "{}".to_string());
                messages.push(ChatMessage::tool_result(&result.call_id, output_str));
            }
            _ => {}
        }
    }

    (state, messages)
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum VercelAiSdkV6Part {
    #[serde(rename = "start")]
    Start {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    #[serde(rename = "start-step")]
    StartStep,
    #[serde(rename = "data-aios-event")]
    DataAiosEvent {
        id: String,
        data: serde_json::Value,
        transient: bool,
    },
    #[serde(rename = "finish-step")]
    FinishStep {
        #[serde(rename = "finishReason")]
        finish_reason: String,
    },
    #[serde(rename = "finish")]
    Finish,
}

fn model_stop_reason_to_string(reason: ModelStopReason) -> &'static str {
    match reason {
        ModelStopReason::EndTurn => "end_turn",
        ModelStopReason::ToolUse => "tool_use",
        ModelStopReason::NeedsUser => "needs_user",
        ModelStopReason::MaxTokens => "max_tokens",
        ModelStopReason::Safety => "safety",
        ModelStopReason::Unknown => "unknown",
    }
}

fn run_stop_reason_to_string(reason: RunStopReason) -> &'static str {
    match reason {
        RunStopReason::Completed => "completed",
        RunStopReason::NeedsUser => "needs_user",
        RunStopReason::BlockedByPolicy => "blocked_by_policy",
        RunStopReason::BudgetExceeded => "budget_exceeded",
        RunStopReason::Cancelled => "cancelled",
        RunStopReason::Error => "error",
    }
}

fn parse_risk_level(level: &str) -> RiskLevel {
    match level.to_ascii_lowercase().as_str() {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        _ => RiskLevel::Medium,
    }
}

fn parse_approval_decision(value: &str) -> ApprovalDecision {
    match value.to_ascii_lowercase().as_str() {
        "approved" => ApprovalDecision::Approved,
        "denied" => ApprovalDecision::Denied,
        "timeout" => ApprovalDecision::Timeout,
        _ => ApprovalDecision::Denied,
    }
}

fn agent_event_to_aios_kind(event: &AgentEvent) -> AiosEventKind {
    match event {
        AgentEvent::RunStarted {
            provider,
            max_iterations,
            ..
        } => AiosEventKind::RunStarted {
            provider: provider.clone(),
            max_iterations: *max_iterations,
        },
        AgentEvent::IterationStarted { iteration, .. } => {
            AiosEventKind::StepStarted { index: *iteration }
        }
        AgentEvent::ModelOutput {
            iteration,
            stop_reason,
            directive_count,
            ..
        } => AiosEventKind::StepFinished {
            index: *iteration,
            stop_reason: model_stop_reason_to_string(*stop_reason).to_string(),
            directive_count: *directive_count,
        },
        AgentEvent::TextDelta {
            delta, iteration, ..
        } => AiosEventKind::AssistantTextDelta {
            delta: delta.clone(),
            index: Some(*iteration),
        },
        AgentEvent::ToolCallRequested { call, .. } => AiosEventKind::ToolCallRequested {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments: call.input.clone(),
            category: None,
        },
        AgentEvent::ToolCallCompleted { result, .. } => AiosEventKind::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some(result.call_id.clone()),
            tool_name: result.tool_name.clone(),
            result: result.output.clone(),
            duration_ms: 0,
            status: SpanStatus::Ok,
        },
        AgentEvent::ToolCallFailed {
            call_id,
            tool_name,
            error,
            ..
        } => AiosEventKind::ToolCallFailed {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            error: error.clone(),
        },
        AgentEvent::StatePatched {
            patch,
            revision,
            iteration,
            ..
        } => AiosEventKind::StatePatched {
            index: Some(*iteration),
            patch: serde_json::to_value(patch).unwrap_or_else(|_| json!({})),
            revision: *revision,
        },
        AgentEvent::ContextCompacted {
            dropped_count,
            tokens_before,
            tokens_after,
            ..
        } => AiosEventKind::ContextCompacted {
            dropped_count: *dropped_count,
            tokens_before: *tokens_before,
            tokens_after: *tokens_after,
        },
        AgentEvent::ApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            risk,
            ..
        } => AiosEventKind::ApprovalRequested {
            approval_id: ApprovalId::from_string(approval_id.clone()),
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            risk: parse_risk_level(risk),
        },
        AgentEvent::ApprovalResolved {
            approval_id,
            decision,
            reason,
            ..
        } => AiosEventKind::ApprovalResolved {
            approval_id: ApprovalId::from_string(approval_id.clone()),
            decision: parse_approval_decision(decision),
            reason: reason.clone(),
        },
        AgentEvent::RunErrored { error, .. } => AiosEventKind::RunErrored {
            error: error.clone(),
        },
        AgentEvent::RunFinished {
            reason,
            total_iterations,
            final_answer,
            ..
        } => AiosEventKind::RunFinished {
            reason: run_stop_reason_to_string(*reason).to_string(),
            total_iterations: *total_iterations,
            final_answer: final_answer.clone(),
            usage: None,
        },
    }
}

fn arcan_record_to_aios_record(record: &arcan_store::session::EventRecord) -> AiosEventRecord {
    AiosEventRecord {
        event_id: AiosEventId::from_string(record.id.clone()),
        session_id: AiosSessionId::from_string(record.session_id.clone()),
        agent_id: AgentId::default(),
        branch_id: AiosBranchId::from_string(record.branch_id.clone()),
        sequence: record.sequence,
        timestamp: record.timestamp,
        actor: EventActor {
            actor_type: ActorType::System,
            component: Some("arcan-daemon".to_string()),
        },
        schema: EventSchema::default(),
        causation_id: record
            .parent_id
            .as_ref()
            .map(|id| AiosEventId::from_string(id.clone())),
        correlation_id: None,
        trace_id: None,
        span_id: None,
        digest: None,
        kind: agent_event_to_aios_kind(&record.event),
    }
}

fn kernel_event_v6_parts(event: &AiosEventRecord) -> [VercelAiSdkV6Part; 5] {
    let message_id = format!("kernel-event-{}", event.event_id);
    let payload = serde_json::to_value(event).unwrap_or_else(|error| {
        json!({
            "error": error.to_string(),
            "sequence": event.sequence,
        })
    });

    [
        VercelAiSdkV6Part::Start { message_id },
        VercelAiSdkV6Part::StartStep,
        VercelAiSdkV6Part::DataAiosEvent {
            id: event.sequence.to_string(),
            data: payload,
            transient: false,
        },
        VercelAiSdkV6Part::FinishStep {
            finish_reason: "stop".to_string(),
        },
        VercelAiSdkV6Part::Finish,
    ]
}

fn v6_part_to_sse(part: &VercelAiSdkV6Part, id: String) -> Event {
    let payload = serde_json::to_string(part).unwrap_or_else(|error| {
        json!({
            "type": "error",
            "errorText": error.to_string(),
        })
        .to_string()
    });
    Event::default().id(id).data(payload)
}

fn canonical_data_parts(event: &AgentEvent) -> Vec<(String, serde_json::Value)> {
    match event {
        AgentEvent::ToolCallRequested { call, .. } => vec![
            (
                "intent.proposed".to_string(),
                json!({
                    "intent_id": call.call_id,
                    "kind": "tool_call",
                    "risk": "low",
                }),
            ),
            (
                "intent.evaluated".to_string(),
                json!({
                    "intent_id": call.call_id,
                    "allowed": true,
                    "requires_approval": false,
                    "reasons": [],
                }),
            ),
            (
                "tool.started".to_string(),
                json!({
                    "intent_id": call.call_id,
                    "tool_name": call.tool_name,
                    "arguments": call.input,
                }),
            ),
        ],
        AgentEvent::ToolCallCompleted { result, .. } => vec![(
            "tool.completed".to_string(),
            json!({
                "intent_id": result.call_id,
                "tool_name": result.tool_name,
                "status": "ok",
                "result": result.output,
            }),
        )],
        AgentEvent::ToolCallFailed {
            call_id,
            tool_name,
            error,
            ..
        } => vec![
            (
                "intent.rejected".to_string(),
                json!({
                    "intent_id": call_id,
                    "reasons": [error],
                }),
            ),
            (
                "tool.completed".to_string(),
                json!({
                    "intent_id": call_id,
                    "tool_name": tool_name,
                    "status": "error",
                    "error": error,
                }),
            ),
        ],
        AgentEvent::StatePatched {
            patch, revision, ..
        } => vec![(
            "state.patch".to_string(),
            json!({
                "revision": revision,
                "patch": patch.patch,
                "format": patch.format,
                "source": patch.source,
            }),
        )],
        AgentEvent::ApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            risk,
            ..
        } => vec![(
            "intent.evaluated".to_string(),
            json!({
                "intent_id": call_id,
                "allowed": true,
                "requires_approval": true,
                "risk": risk,
                "approval_id": approval_id,
                "tool_name": tool_name,
                "arguments": arguments,
            }),
        )],
        AgentEvent::ApprovalResolved {
            approval_id,
            decision,
            reason,
            ..
        } => {
            let part_name = if decision == "approved" {
                "intent.approved"
            } else {
                "intent.rejected"
            };
            vec![(
                part_name.to_string(),
                json!({
                    "approval_id": approval_id,
                    "decision": decision,
                    "reason": reason,
                }),
            )]
        }
        AgentEvent::TextDelta { delta, .. } => vec![(
            "assistant.text.delta".to_string(),
            json!({
                "delta": delta,
            }),
        )],
        AgentEvent::RunFinished { final_answer, .. } => final_answer
            .as_ref()
            .map(|message| {
                vec![(
                    "assistant.message.committed".to_string(),
                    json!({
                        "role": "assistant",
                        "content": message,
                    }),
                )]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn part_to_sse(name: &str, payload: &serde_json::Value, id: String) -> Event {
    Event::default()
        .event(name)
        .id(id)
        .data(serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string()))
}

async fn v1_runs_handler(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Json(request): Json<V1RunRequest>,
) -> Response {
    let branch = request.branch.unwrap_or_else(default_branch);
    let (tx, rx) = mpsc::channel(100);

    let agent_loop = state.agent_loop.clone();
    let sid = session_id.clone();
    let message = request.message.clone();
    let branch_for_run = branch.clone();

    tokio::spawn(async move {
        if let Err(e) = agent_loop
            .run(&sid, &branch_for_run, message, tx.clone())
            .await
        {
            let _ = tx
                .send(AgentEvent::RunErrored {
                    run_id: "unknown".to_string(),
                    session_id: sid,
                    error: e.to_string(),
                })
                .await;
        }
    });

    let (event_tx, event_rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    tokio::spawn(async move {
        let mut stream = ReceiverStream::new(rx);
        let mut seq: u64 = 0;
        while let Some(event) = stream.next().await {
            let parts = canonical_data_parts(&event);
            for (name, payload) in parts {
                seq = seq.saturating_add(1);
                let frame = part_to_sse(&name, &payload, seq.to_string());
                if event_tx.send(Ok(frame)).await.is_err() {
                    return;
                }
            }
        }
        let _ = event_tx
            .send(Ok(Event::default()
                .event("done")
                .data(r#"{"type":"done"}"#)))
            .await;
    });

    let out_stream = ReceiverStream::new(event_rx);
    let sse = Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)));

    (
        [(
            axum::http::header::HeaderName::from_static("x-vercel-ai-ui-message-stream"),
            axum::http::HeaderValue::from_static("v1"),
        )],
        sse,
    )
        .into_response()
}

async fn v1_state_handler(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Query(query): Query<V1StateQuery>,
) -> Result<Json<V1StateResponse>, AppError> {
    let repo = state.agent_loop.session_repo.clone();
    let records =
        tokio::task::spawn_blocking(move || repo.load_session(&session_id, &query.branch))
            .await
            .map_err(|e| AppError::Internal(format!("state load join error: {e}")))?
            .map_err(|e| AppError::Internal(format!("state load error: {e}")))?;

    let (state_snapshot, _) = replay_records(&records);
    Ok(Json(V1StateResponse {
        version: state_snapshot.revision,
        state: json!({
            "session": state_snapshot.data,
            "agent": {},
            "os": {},
            "memory": {},
        }),
    }))
}

async fn v1_stream_handler(
    State(state): State<Arc<ServerState>>,
    Path(session_id): Path<String>,
    Query(query): Query<V1StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let repo = state.agent_loop.session_repo.clone();
    let branch = query.branch.clone();
    let stream_format = parse_stream_format(query.format.as_deref());
    let from_version = parse_last_event_id(&headers)
        .or(query.from_version)
        .unwrap_or(0);

    let records = tokio::task::spawn_blocking(move || repo.load_session(&session_id, &branch))
        .await
        .map_err(|e| AppError::Internal(format!("stream load join error: {e}")))?
        .map_err(|e| AppError::Internal(format!("stream load error: {e}")))?;

    let mut frames: Vec<Result<Event, Infallible>> = Vec::new();
    for record in records.iter().filter(|r| r.sequence > from_version) {
        match stream_format {
            StreamFormat::CanonicalV1 => {
                for (idx, (name, payload)) in
                    canonical_data_parts(&record.event).into_iter().enumerate()
                {
                    let id = if idx == 0 {
                        record.sequence.to_string()
                    } else {
                        format!("{}.{}", record.sequence, idx)
                    };
                    frames.push(Ok(part_to_sse(&name, &payload, id)));
                }
            }
            StreamFormat::VercelAiSdkV6 => {
                let protocol_record = arcan_record_to_aios_record(record);
                for (idx, part) in kernel_event_v6_parts(&protocol_record).iter().enumerate() {
                    let id = if idx == 0 {
                        record.sequence.to_string()
                    } else {
                        format!("{}.{}", record.sequence, idx)
                    };
                    frames.push(Ok(v6_part_to_sse(part, id)));
                }
            }
        }
    }

    match stream_format {
        StreamFormat::CanonicalV1 => {
            frames.push(Ok(Event::default()
                .event("done")
                .data(r#"{"type":"done"}"#)));
        }
        StreamFormat::VercelAiSdkV6 => {
            frames.push(Ok(Event::default().data("[DONE]")));
        }
    }

    let sse = Sse::new(tokio_stream::iter(frames)).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    );

    Ok((
        [(
            axum::http::header::HeaderName::from_static("x-vercel-ai-ui-message-stream"),
            axum::http::HeaderValue::from_static("v1"),
        )],
        sse,
    )
        .into_response())
}

async fn v1_signals_handler(
    State(state): State<Arc<ServerState>>,
    Path(_session_id): Path<String>,
    Json(request): Json<V1SignalRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    if request.signal_type == "approve" {
        let approval_id = request
            .data
            .get("approval_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::BadRequest("missing data.approval_id".to_string()))?;
        let decision = request
            .data
            .get("decision")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::BadRequest("missing data.decision".to_string()))?;
        let reason = request
            .data
            .get("reason")
            .and_then(|v| v.as_str())
            .map(str::to_string);

        let resolver = state
            .approval_resolver
            .as_ref()
            .ok_or_else(|| AppError::BadRequest("approval workflow not configured".to_string()))?;

        let resolved = resolver.resolve_approval(approval_id, decision, reason);
        if !resolved {
            return Err(AppError::NotFound(format!(
                "approval '{}' not found or already resolved",
                approval_id
            )));
        }

        return Ok(Json(json!({ "accepted": true, "signal_type": "approve" })));
    }

    Ok(Json(json!({
        "accepted": true,
        "signal_type": request.signal_type,
    })))
}

async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    Query(query): Query<ChatQuery>,
    Json(request): Json<ChatRequest>,
) -> Response {
    // Handle `/` commands (e.g., /model, /help) without sending to LLM
    if request.message.trim().starts_with('/') {
        match commands::handle_command(&request.message, &state.orchestrator) {
            CommandResult::Response(text) => {
                return Json(serde_json::json!({
                    "command": true,
                    "response": text,
                }))
                .into_response();
            }
            CommandResult::NotACommand => {} // fall through to normal chat
        }
    }

    let (tx, rx) = mpsc::channel(100);

    let agent_loop = state.agent_loop.clone();
    let session_id = request.session_id.clone();
    let branch_id = request.branch_id.unwrap_or_else(|| "main".to_string());
    let message = request.message.clone();

    tokio::spawn(async move {
        if let Err(e) = agent_loop
            .run(&session_id, &branch_id, message, tx.clone())
            .await
        {
            let _ = tx
                .send(AgentEvent::RunErrored {
                    run_id: "unknown".to_string(),
                    session_id,
                    error: e.to_string(),
                })
                .await;
        }
    });

    let use_v6 = matches!(query.format.as_deref(), Some("aisdk_v6" | "aisdk_v5"));

    let (event_tx, event_rx) = mpsc::channel::<Result<Event, Infallible>>(200);

    tokio::spawn(async move {
        let mut stream = ReceiverStream::new(rx);
        let mut seq: u64 = 0;
        let mut text_block_open = false;
        let mut text_block_id: Option<String> = None;

        while let Some(event) = stream.next().await {
            if use_v6 {
                let parts = to_ui_stream_parts(&event);
                for part in parts {
                    // Text boundary tracking: insert TextStart/TextEnd around
                    // consecutive TextDelta events
                    let is_text_delta = matches!(&part, UiStreamPart::TextDelta { .. });

                    if !is_text_delta && text_block_open {
                        // Close the open text block
                        if let Some(ref id) = text_block_id {
                            let end = UiStreamPart::TextEnd { id: id.clone() };
                            if let Ok(json) = serde_json::to_string(&end) {
                                seq += 1;
                                let sse = Ok(Event::default().data(json).id(seq.to_string()));
                                if event_tx.send(sse).await.is_err() {
                                    return;
                                }
                            }
                        }
                        text_block_open = false;
                        text_block_id = None;
                    }

                    if is_text_delta && !text_block_open {
                        // Open a new text block
                        if let UiStreamPart::TextDelta { ref id, .. } = part {
                            let start = UiStreamPart::TextStart { id: id.clone() };
                            if let Ok(json) = serde_json::to_string(&start) {
                                seq += 1;
                                let sse = Ok(Event::default().data(json).id(seq.to_string()));
                                if event_tx.send(sse).await.is_err() {
                                    return;
                                }
                            }
                            text_block_open = true;
                            text_block_id = Some(id.clone());
                        }
                    }

                    // Emit the part itself
                    let sse = match serde_json::to_string(&part) {
                        Ok(json) => {
                            seq += 1;
                            Ok(Event::default().data(json).id(seq.to_string()))
                        }
                        Err(e) => Ok(Event::default()
                            .data(format!(r#"{{"type":"error","errorText":"{}"}}"#, e))),
                    };
                    if event_tx.send(sse).await.is_err() {
                        return;
                    }
                }
            } else {
                // Native Arcan format
                let sse = match serde_json::to_string(&event) {
                    Ok(json) => Ok(Event::default().data(json)),
                    Err(e) => Ok(Event::default().data(format!(r#"{{"error": "{}"}}"#, e))),
                };
                if event_tx.send(sse).await.is_err() {
                    return;
                }
            }
        }

        // Close any open text block at stream end
        if text_block_open {
            if let Some(ref id) = text_block_id {
                let end = UiStreamPart::TextEnd { id: id.clone() };
                if let Ok(json) = serde_json::to_string(&end) {
                    seq += 1;
                    let _ = event_tx
                        .send(Ok(Event::default().data(json).id(seq.to_string())))
                        .await;
                }
            }
        }

        // Send [DONE] termination signal for v6
        if use_v6 {
            let _ = event_tx.send(Ok(Event::default().data("[DONE]"))).await;
        }
    });

    let out_stream = ReceiverStream::new(event_rx);
    let sse = Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)));

    if use_v6 {
        // Add v6 protocol header
        (
            [(
                axum::http::header::HeaderName::from_static("x-vercel-ai-ui-message-stream"),
                axum::http::HeaderValue::from_static("v1"),
            )],
            sse,
        )
            .into_response()
    } else {
        sse.into_response()
    }
}

#[derive(Deserialize)]
pub struct ApproveRequest {
    pub approval_id: String,
    pub decision: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct ApproveResponse {
    pub resolved: bool,
}

async fn approve_handler(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<ApproveRequest>,
) -> Result<Json<ApproveResponse>, AppError> {
    match request.decision.as_str() {
        "approved" | "denied" => {}
        other => {
            return Err(AppError::BadRequest(format!(
                "invalid decision '{}': must be 'approved' or 'denied'",
                other
            )));
        }
    }

    let Some(resolver) = &state.approval_resolver else {
        return Err(AppError::BadRequest(
            "approval workflow not configured".to_string(),
        ));
    };

    let resolved =
        resolver.resolve_approval(&request.approval_id, &request.decision, request.reason);

    if resolved {
        Ok(Json(ApproveResponse { resolved: true }))
    } else {
        Err(AppError::NotFound(format!(
            "approval '{}' not found or already resolved",
            request.approval_id
        )))
    }
}

#[derive(Serialize)]
pub struct ListApprovalsResponse {
    pub pending: Vec<String>,
}

async fn list_approvals_handler(
    State(state): State<Arc<ServerState>>,
) -> Json<ListApprovalsResponse> {
    let pending = state
        .approval_resolver
        .as_ref()
        .map(|r| r.pending_approval_ids())
        .unwrap_or_default();
    Json(ListApprovalsResponse { pending })
}

// ─── Model management endpoints ─────────────────────────────────

async fn get_model_handler(State(state): State<Arc<ServerState>>) -> Json<serde_json::Value> {
    let name = state.orchestrator.provider_name();
    Json(serde_json::json!({ "model": name }))
}

#[derive(Deserialize)]
pub struct SetModelRequest {
    /// Provider name: "anthropic", "openai", "ollama", "mock"
    pub provider: String,
    /// Optional model override (e.g., "gpt-4-turbo", "qwen2.5")
    #[serde(default)]
    pub model: Option<String>,
}

async fn set_model_handler(
    State(state): State<Arc<ServerState>>,
    Json(request): Json<SetModelRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let new_provider = commands::create_provider(&request.provider, request.model.as_deref())
        .map_err(AppError::BadRequest)?;

    let name = state.orchestrator.swap_provider(new_provider);
    Ok(Json(serde_json::json!({
        "model": name,
        "switched": true,
    })))
}
