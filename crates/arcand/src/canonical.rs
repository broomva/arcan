use std::{convert::Infallible, sync::Arc, time::Duration};

use aios_protocol::{
    AgentStateVector, BranchId, BranchInfo, BranchMergeResult, EventKind, EventRecord,
    ModelRouting, OperatingMode, PolicySet, SessionId, ToolCall,
};
use aios_runtime::{KernelRuntime, TickInput};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{fs, sync::mpsc};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use uuid::Uuid;

#[derive(Clone)]
struct CanonicalState {
    runtime: Arc<KernelRuntime>,
}

#[derive(Debug, Deserialize, Default)]
struct CreateSessionRequest {
    session_id: Option<String>,
    owner: Option<String>,
    policy: Option<PolicySet>,
    model_routing: Option<ModelRouting>,
}

#[derive(Debug, Deserialize)]
struct RunRequest {
    objective: String,
    branch: Option<String>,
    proposed_tool: Option<ProposedToolRequest>,
}

#[derive(Debug, Deserialize)]
struct ProposedToolRequest {
    tool_name: String,
    input: serde_json::Value,
    #[serde(default)]
    requested_capabilities: Vec<aios_protocol::Capability>,
}

#[derive(Debug, Deserialize, Default)]
struct EventQuery {
    branch: Option<String>,
    from_sequence: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct StreamQuery {
    branch: Option<String>,
    cursor: Option<u64>,
    replay_limit: Option<usize>,
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateBranchRequest {
    branch: String,
    from_branch: Option<String>,
    fork_sequence: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct MergeBranchRequest {
    target_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResolveApprovalRequest {
    approved: bool,
    actor: Option<String>,
}

#[derive(Debug, Serialize)]
struct RunResponse {
    session_id: SessionId,
    mode: OperatingMode,
    state: AgentStateVector,
    events_emitted: u64,
    last_sequence: u64,
}

#[derive(Debug, Serialize)]
struct StateResponse {
    session_id: SessionId,
    branch: BranchId,
    mode: OperatingMode,
    state: AgentStateVector,
    version: u64,
}

#[derive(Debug, Serialize)]
struct EventListResponse {
    session_id: SessionId,
    branch: BranchId,
    from_sequence: u64,
    events: Vec<EventRecord>,
}

#[derive(Debug, Serialize)]
struct BranchListResponse {
    session_id: SessionId,
    branches: Vec<BranchInfo>,
}

#[derive(Debug, Serialize)]
struct BranchMergeResponse {
    session_id: SessionId,
    result: BranchMergeResult,
}

pub fn create_canonical_router(runtime: Arc<KernelRuntime>) -> Router {
    let state = CanonicalState { runtime };
    Router::new()
        .route("/health", get(health))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{session_id}/runs", post(run_session))
        .route("/sessions/{session_id}/state", get(get_state))
        .route("/sessions/{session_id}/events", get(list_events))
        .route("/sessions/{session_id}/events/stream", get(stream_events))
        .route(
            "/sessions/{session_id}/branches",
            post(create_branch).get(list_branches),
        )
        .route(
            "/sessions/{session_id}/branches/{branch_id}/merge",
            post(merge_branch),
        )
        .route(
            "/sessions/{session_id}/approvals/{approval_id}",
            post(resolve_approval),
        )
        .with_state(state)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamFormat {
    Canonical,
    VercelAiSdkV6,
}

impl StreamFormat {
    fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("vercel_ai_sdk_v6" | "aisdk_v6") => Self::VercelAiSdkV6,
            _ => Self::Canonical,
        }
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Serialize)]
struct SessionSummary {
    session_id: String,
    owner: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

async fn list_sessions(State(state): State<CanonicalState>) -> Json<Vec<SessionSummary>> {
    let manifests = state.runtime.list_sessions();
    let mut summaries: Vec<SessionSummary> = manifests
        .into_iter()
        .map(|m| SessionSummary {
            session_id: m.session_id.as_str().to_owned(),
            owner: m.owner,
            created_at: m.created_at,
        })
        .collect();
    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(summaries)
}

async fn persist_last_session_hint(runtime: &KernelRuntime, session_id: &SessionId) {
    let path = runtime.root_path().join("last_session");
    let _ = fs::write(path, session_id.as_str()).await;
}

async fn create_session(
    State(state): State<CanonicalState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<aios_protocol::SessionManifest>, (StatusCode, Json<serde_json::Value>)> {
    let owner = request.owner.unwrap_or_else(|| "arcan".to_owned());
    let policy = request.policy.unwrap_or_default();
    let routing = request.model_routing.unwrap_or_default();
    let manifest = if let Some(session_id) = request.session_id {
        state
            .runtime
            .create_session_with_id(SessionId::from_string(session_id), owner, policy, routing)
            .await
            .map_err(internal_error)?
    } else {
        state
            .runtime
            .create_session(owner, policy, routing)
            .await
            .map_err(internal_error)?
    };
    persist_last_session_hint(state.runtime.as_ref(), &manifest.session_id).await;
    Ok(Json(manifest))
}

async fn run_session(
    Path(session_id): Path<String>,
    State(state): State<CanonicalState>,
    Json(request): Json<RunRequest>,
) -> Result<Json<RunResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let branch = request
        .branch
        .as_deref()
        .map(BranchId::from_string)
        .unwrap_or_else(BranchId::main);
    if !state.runtime.session_exists(&session_id) {
        state
            .runtime
            .create_session_with_id(
                session_id.clone(),
                "arcan",
                PolicySet::default(),
                ModelRouting::default(),
            )
            .await
            .map_err(internal_error)?;
    }
    let proposed_tool = request
        .proposed_tool
        .map(|tool| ToolCall::new(tool.tool_name, tool.input, tool.requested_capabilities));

    let tick = state
        .runtime
        .tick_on_branch(
            &session_id,
            &branch,
            TickInput {
                objective: request.objective,
                proposed_tool,
            },
        )
        .await
        .map_err(internal_error)?;

    persist_last_session_hint(state.runtime.as_ref(), &tick.session_id).await;

    Ok(Json(RunResponse {
        session_id: tick.session_id,
        mode: tick.mode,
        state: tick.state,
        events_emitted: tick.events_emitted,
        last_sequence: tick.last_sequence,
    }))
}

async fn get_state(
    Path(session_id): Path<String>,
    Query(query): Query<EventQuery>,
    State(state): State<CanonicalState>,
) -> Result<Json<StateResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let branch = query
        .branch
        .as_deref()
        .map(BranchId::from_string)
        .unwrap_or_else(BranchId::main);
    let events = state
        .runtime
        .read_events_on_branch(&session_id, &branch, 1, 4096)
        .await
        .map_err(internal_error)?;

    let mut latest_state = AgentStateVector::default();
    let mut mode = OperatingMode::Explore;
    let mut version = 0_u64;
    for event in events {
        version = event.sequence;
        if let EventKind::StateEstimated {
            state,
            mode: event_mode,
        } = event.kind
        {
            latest_state = state;
            mode = event_mode;
        }
    }

    Ok(Json(StateResponse {
        session_id,
        branch,
        mode,
        state: latest_state,
        version,
    }))
}

async fn list_events(
    Path(session_id): Path<String>,
    Query(query): Query<EventQuery>,
    State(state): State<CanonicalState>,
) -> Result<Json<EventListResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let branch = query
        .branch
        .as_deref()
        .map(BranchId::from_string)
        .unwrap_or_else(BranchId::main);
    let from_sequence = query.from_sequence.unwrap_or(1);
    let limit = query.limit.unwrap_or(256).min(10_000);

    let events = state
        .runtime
        .read_events_on_branch(&session_id, &branch, from_sequence, limit)
        .await
        .map_err(internal_error)?;

    Ok(Json(EventListResponse {
        session_id,
        branch,
        from_sequence,
        events,
    }))
}

async fn stream_events(
    Path(session_id): Path<String>,
    Query(query): Query<StreamQuery>,
    State(state): State<CanonicalState>,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let format = StreamFormat::from_query(query.format.as_deref());
    let branch = query
        .branch
        .as_deref()
        .map(BranchId::from_string)
        .unwrap_or_else(BranchId::main);
    let cursor = query.cursor.unwrap_or(0);
    let replay_limit = query.replay_limit.unwrap_or(512).min(10_000);

    let replay = state
        .runtime
        .read_events_on_branch(&session_id, &branch, cursor.saturating_add(1), replay_limit)
        .await
        .map_err(internal_error)?;

    let mut subscription = state.runtime.subscribe_events();
    let (tx, rx) = mpsc::channel::<EventRecord>(256);
    let session_filter = session_id.clone();
    let branch_filter = branch.clone();

    tokio::spawn(async move {
        for event in replay {
            let _ = tx.send(event).await;
        }
        while let Ok(event) = subscription.recv().await {
            if event.session_id == session_filter
                && event.branch_id == branch_filter
                && (event.sequence > cursor || event.sequence == 0)
            {
                let _ = tx.send(event).await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(move |event| {
        let base = Event::default().id(event.sequence.to_string());
        let frame = match format {
            StreamFormat::Canonical => {
                let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                base.data(payload)
            }
            StreamFormat::VercelAiSdkV6 => {
                let payload =
                    serde_json::to_string(&json!({ "type": "data-aios-event", "data": event }))
                        .unwrap_or_else(|_| "{}".to_owned());
                base.data(payload)
            }
        };
        Ok::<Event, Infallible>(frame)
    });

    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)));
    let mut response = sse.into_response();
    if format == StreamFormat::VercelAiSdkV6 {
        response.headers_mut().insert(
            "x-vercel-ai-ui-message-stream",
            "v1".parse().expect("static header value"),
        );
    }
    Ok(response)
}

async fn create_branch(
    Path(session_id): Path<String>,
    State(state): State<CanonicalState>,
    Json(request): Json<CreateBranchRequest>,
) -> Result<Json<BranchInfo>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let branch_id = BranchId::from_string(request.branch);
    let from_branch = request.from_branch.map(BranchId::from_string);
    let branch = state
        .runtime
        .create_branch(&session_id, branch_id, from_branch, request.fork_sequence)
        .await
        .map_err(internal_error)?;
    Ok(Json(branch))
}

async fn list_branches(
    Path(session_id): Path<String>,
    State(state): State<CanonicalState>,
) -> Result<Json<BranchListResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let branches = state
        .runtime
        .list_branches(&session_id)
        .await
        .map_err(internal_error)?;
    Ok(Json(BranchListResponse {
        session_id,
        branches,
    }))
}

async fn merge_branch(
    Path((session_id, branch_id)): Path<(String, String)>,
    State(state): State<CanonicalState>,
    Json(request): Json<MergeBranchRequest>,
) -> Result<Json<BranchMergeResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let source_branch = BranchId::from_string(branch_id);
    let target_branch = request
        .target_branch
        .map(BranchId::from_string)
        .unwrap_or_else(BranchId::main);
    let result = state
        .runtime
        .merge_branch(&session_id, source_branch, target_branch)
        .await
        .map_err(internal_error)?;
    Ok(Json(BranchMergeResponse { session_id, result }))
}

async fn resolve_approval(
    Path((session_id, approval_id)): Path<(String, String)>,
    State(state): State<CanonicalState>,
    Json(request): Json<ResolveApprovalRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);
    let parsed = Uuid::parse_str(&approval_id).map_err(bad_request)?;
    let actor = request.actor.unwrap_or_else(|| "api".to_owned());

    state
        .runtime
        .resolve_approval(&session_id, parsed, request.approved, actor)
        .await
        .map_err(internal_error)?;

    Ok((StatusCode::NO_CONTENT, Json(json!({}))))
}

fn internal_error(error: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
}

fn bad_request(error: impl std::fmt::Display) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error.to_string() })),
    )
}
