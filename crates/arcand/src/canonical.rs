use std::{convert::Infallible, sync::Arc, time::Duration, time::Instant};

use aios_protocol::{
    AgentStateVector, BranchId, BranchInfo, BranchMergeResult, EventKind, EventRecord,
    ModelRouting, OperatingMode, PolicySet, SessionId, ToolCall,
};
use aios_runtime::{KernelRuntime, TickInput};
use arcan_core::runtime::{ProviderFactory, SwappableProviderHandle};
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
use tracing::Instrument;
use utoipa::{IntoParams, OpenApi, PartialSchema, ToSchema};
use utoipa_scalar::{Scalar, Servable};
use uuid::Uuid;

// ─── Mirror schemas for external aios-protocol types ─────────────────────────
//
// These structs mirror the shape of aios-protocol types that don't derive
// `ToSchema`. They are used *only* in `#[openapi(components(schemas(...)))]`
// so Scalar renders proper typed definitions instead of opaque `Object`.

/// Operating mode of the agent kernel.
#[derive(ToSchema)]
#[schema(as = OperatingMode)]
#[allow(dead_code)]
enum OperatingModeSchema {
    /// High uncertainty — read-only tools preferred
    Explore,
    /// Default productive mode
    Execute,
    /// High side-effect pressure — validate before committing
    Verify,
    /// Error streak >= threshold
    Recover,
    /// Pending approvals or human input needed
    AskHuman,
    /// Progress >= 98% or awaiting next signal
    Sleep,
}

/// Budget remaining for the current agent run.
#[derive(ToSchema)]
#[schema(as = BudgetState)]
#[allow(dead_code)]
struct BudgetStateSchema {
    tokens_remaining: u64,
    time_remaining_ms: u64,
    cost_remaining_usd: f64,
    tool_calls_remaining: u32,
    error_budget_remaining: u32,
}

/// Risk level assessment.
#[derive(ToSchema)]
#[schema(as = RiskLevel)]
#[allow(dead_code)]
enum RiskLevelSchema {
    Low,
    Medium,
    High,
    Critical,
}

/// Homeostatic state vector of the agent.
#[derive(ToSchema)]
#[schema(as = AgentStateVector)]
#[allow(dead_code)]
struct AgentStateVectorSchema {
    progress: f32,
    uncertainty: f32,
    #[schema(schema_with = RiskLevelSchema::schema)]
    risk_level: RiskLevelSchema,
    #[schema(schema_with = BudgetStateSchema::schema)]
    budget: BudgetStateSchema,
    error_streak: u32,
    context_pressure: f32,
    side_effect_pressure: f32,
    human_dependency: f32,
}

/// Policy configuration for a session.
#[derive(ToSchema)]
#[schema(as = PolicySet)]
#[allow(dead_code)]
struct PolicySetSchema {
    /// Capabilities allowed without approval (e.g. "fs:read:**")
    allow_capabilities: Vec<String>,
    /// Capabilities requiring approval before use
    gate_capabilities: Vec<String>,
    max_tool_runtime_secs: u64,
    max_events_per_turn: u64,
}

/// Model routing configuration.
#[derive(ToSchema)]
#[schema(as = ModelRouting)]
#[allow(dead_code)]
struct ModelRoutingSchema {
    /// Primary model identifier (e.g. "claude-sonnet-4-5-20250929")
    primary_model: String,
    /// Fallback models tried in order if primary fails
    fallback_models: Vec<String>,
    temperature: f32,
}

/// Session manifest returned on creation.
#[derive(ToSchema)]
#[schema(as = SessionManifest)]
#[allow(dead_code)]
struct SessionManifestSchema {
    session_id: String,
    owner: String,
    created_at: chrono::DateTime<chrono::Utc>,
    workspace_root: String,
    #[schema(schema_with = ModelRoutingSchema::schema)]
    model_routing: ModelRoutingSchema,
    policy: serde_json::Value,
}

/// Information about a branch within a session.
#[derive(ToSchema)]
#[schema(as = BranchInfo)]
#[allow(dead_code)]
struct BranchInfoSchema {
    branch_id: String,
    parent_branch: Option<String>,
    fork_sequence: u64,
    head_sequence: u64,
    merged_into: Option<String>,
}

/// Result of merging one branch into another.
#[derive(ToSchema)]
#[schema(as = BranchMergeResult)]
#[allow(dead_code)]
struct BranchMergeResultSchema {
    source_branch: String,
    target_branch: String,
    source_head_sequence: u64,
    target_head_sequence: u64,
}

/// A single event record from the event journal.
#[derive(ToSchema)]
#[schema(as = EventRecord)]
#[allow(dead_code)]
struct EventRecordSchema {
    event_id: String,
    session_id: String,
    agent_id: String,
    branch_id: String,
    sequence: u64,
    timestamp: chrono::DateTime<chrono::Utc>,
    actor: serde_json::Value,
    schema: serde_json::Value,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    trace_id: Option<String>,
    span_id: Option<String>,
    digest: Option<String>,
    /// Discriminated union — the event payload
    kind: serde_json::Value,
}

// ─── Request / Response types ────────────────────────────────────────────────

#[derive(Clone)]
struct CanonicalState {
    runtime: Arc<KernelRuntime>,
    provider_handle: SwappableProviderHandle,
    provider_factory: Arc<dyn ProviderFactory>,
    started_at: Instant,
    /// Shared skill registry for activation and tool filtering.
    skill_registry: Option<Arc<praxis_skills::registry::SkillRegistry>>,
}

#[derive(Debug, Deserialize, Default, ToSchema)]
struct CreateSessionRequest {
    /// Explicit session ID (auto-generated if omitted)
    session_id: Option<String>,
    /// Session owner (defaults to "arcan")
    owner: Option<String>,
    #[schema(schema_with = PolicySetSchema::schema)]
    policy: Option<PolicySet>,
    #[schema(schema_with = ModelRoutingSchema::schema)]
    model_routing: Option<ModelRouting>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct RunRequest {
    /// The objective / user message for this agent run
    objective: String,
    /// Target branch (defaults to "main")
    branch: Option<String>,
    /// Optional pre-proposed tool call
    proposed_tool: Option<ProposedToolRequest>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ProposedToolRequest {
    tool_name: String,
    input: serde_json::Value,
    /// Capability strings required (e.g. "fs:read:**")
    #[serde(default)]
    requested_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
struct EventQuery {
    /// Branch name (defaults to "main")
    branch: Option<String>,
    /// Start reading from this sequence number
    from_sequence: Option<u64>,
    /// Max events to return (default 256, max 10000)
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Default, IntoParams)]
struct StreamQuery {
    /// Branch name (defaults to "main")
    branch: Option<String>,
    /// Resume from this sequence (0 = replay all)
    cursor: Option<u64>,
    /// Max events to replay before switching to live (default 512)
    replay_limit: Option<usize>,
    /// Stream format: "canonical" (default) or "vercel_ai_sdk_v6"
    format: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct CreateBranchRequest {
    /// Name for the new branch
    branch: String,
    /// Parent branch to fork from (defaults to "main")
    from_branch: Option<String>,
    /// Sequence number to fork at (defaults to head)
    fork_sequence: Option<u64>,
}

#[derive(Debug, Deserialize, Default, ToSchema)]
struct MergeBranchRequest {
    /// Target branch to merge into (defaults to "main")
    target_branch: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct ResolveApprovalRequest {
    /// Whether the approval is granted
    approved: bool,
    /// Actor resolving the approval (defaults to "api")
    actor: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
struct SetProviderRequest {
    /// Provider spec: `"provider_name"` or `"provider_name:model"`.
    provider: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct ProviderResponse {
    /// Currently active provider name.
    provider: String,
    /// Available provider names that can be switched to.
    available: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
struct RunResponse {
    #[schema(value_type = String, example = "sess-abc123")]
    session_id: SessionId,
    #[schema(schema_with = OperatingModeSchema::schema)]
    mode: OperatingMode,
    #[schema(schema_with = AgentStateVectorSchema::schema)]
    state: AgentStateVector,
    events_emitted: u64,
    last_sequence: u64,
}

#[derive(Debug, Serialize, ToSchema)]
struct StateResponse {
    #[schema(value_type = String, example = "sess-abc123")]
    session_id: SessionId,
    #[schema(value_type = String, example = "main")]
    branch: BranchId,
    #[schema(schema_with = OperatingModeSchema::schema)]
    mode: OperatingMode,
    #[schema(schema_with = AgentStateVectorSchema::schema)]
    state: AgentStateVector,
    version: u64,
}

#[derive(Debug, Serialize, ToSchema)]
struct EventListResponse {
    #[schema(value_type = String)]
    session_id: SessionId,
    #[schema(value_type = String)]
    branch: BranchId,
    from_sequence: u64,
    #[schema(schema_with = Vec::<EventRecordSchema>::schema)]
    events: Vec<EventRecord>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BranchListResponse {
    #[schema(value_type = String)]
    session_id: SessionId,
    #[schema(schema_with = Vec::<BranchInfoSchema>::schema)]
    branches: Vec<BranchInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
struct BranchMergeResponse {
    #[schema(value_type = String)]
    session_id: SessionId,
    #[schema(schema_with = BranchMergeResultSchema::schema)]
    result: BranchMergeResult,
}

#[derive(Debug, Serialize, ToSchema)]
struct SessionSummary {
    session_id: String,
    owner: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
struct ErrorResponse {
    error: String,
}

// ─── OpenAPI document ────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Arcan Agent Runtime API",
        version = "0.2.1",
        description = "HTTP API for the Arcan agent runtime daemon — session management, agent runs, event streaming, branching, and approval workflows.",
        license(name = "MIT"),
    ),
    paths(
        health,
        list_sessions,
        create_session,
        run_session,
        get_state,
        list_events,
        stream_events,
        create_branch,
        list_branches,
        merge_branch,
        resolve_approval,
        get_provider,
        set_provider,
    ),
    components(schemas(
        // Request / response types
        CreateSessionRequest,
        RunRequest,
        ProposedToolRequest,
        RunResponse,
        StateResponse,
        EventListResponse,
        BranchListResponse,
        BranchMergeResponse,
        SessionSummary,
        CreateBranchRequest,
        MergeBranchRequest,
        ResolveApprovalRequest,
        SetProviderRequest,
        ProviderResponse,
        ErrorResponse,
        // Mirror schemas for external aios-protocol types
        OperatingModeSchema,
        AgentStateVectorSchema,
        BudgetStateSchema,
        RiskLevelSchema,
        PolicySetSchema,
        ModelRoutingSchema,
        SessionManifestSchema,
        BranchInfoSchema,
        BranchMergeResultSchema,
        EventRecordSchema,
    )),
    tags(
        (name = "health", description = "Health check"),
        (name = "sessions", description = "Session lifecycle"),
        (name = "runs", description = "Agent run execution"),
        (name = "events", description = "Event log and streaming"),
        (name = "branches", description = "Branch management"),
        (name = "approvals", description = "Approval workflow"),
        (name = "provider", description = "Live provider switching"),
    )
)]
struct ApiDoc;

/// Return the OpenAPI specification.
pub fn openapi_spec() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

// ─── Router ──────────────────────────────────────────────────────────────────

pub fn create_canonical_router(
    runtime: Arc<KernelRuntime>,
    provider_handle: SwappableProviderHandle,
    provider_factory: Arc<dyn ProviderFactory>,
) -> Router {
    create_canonical_router_with_skills(runtime, provider_handle, provider_factory, None)
}

/// Create the canonical router with an optional skill registry for activation.
pub fn create_canonical_router_with_skills(
    runtime: Arc<KernelRuntime>,
    provider_handle: SwappableProviderHandle,
    provider_factory: Arc<dyn ProviderFactory>,
    skill_registry: Option<Arc<praxis_skills::registry::SkillRegistry>>,
) -> Router {
    let state = CanonicalState {
        runtime,
        provider_handle,
        provider_factory,
        started_at: Instant::now(),
        skill_registry,
    };
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi_json))
        .merge(Scalar::with_url("/docs", ApiDoc::openapi()))
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
        .route("/provider", get(get_provider).put(set_provider))
        .with_state(state)
}

// ─── Stream format ───────────────────────────────────────────────────────────

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

// ─── Handlers ────────────────────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Daemon is healthy", body = Object)
    )
)]
async fn health(State(state): State<CanonicalState>) -> Json<serde_json::Value> {
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let otlp_configured = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();
    Json(json!({
        "status": "ok",
        "service": "arcan",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime_seconds,
        "telemetry": {
            "sdk": "vigil",
            "otlp_configured": otlp_configured,
        },
    }))
}

#[utoipa::path(
    get,
    path = "/sessions",
    tag = "sessions",
    responses(
        (status = 200, description = "List of sessions ordered by creation time", body = Vec<SessionSummary>)
    )
)]
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

#[utoipa::path(
    post,
    path = "/sessions",
    tag = "sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 200, description = "Session created", body = SessionManifestSchema),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/runs",
    tag = "runs",
    params(("session_id" = String, Path, description = "Session identifier")),
    request_body = RunRequest,
    responses(
        (status = 200, description = "Run completed", body = RunResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

    let capabilities: Vec<aios_protocol::Capability> = request
        .proposed_tool
        .as_ref()
        .map(|t| {
            t.requested_capabilities
                .iter()
                .map(|s| aios_protocol::Capability::new(s.clone()))
                .collect()
        })
        .unwrap_or_default();

    let proposed_tool = request
        .proposed_tool
        .map(|tool| ToolCall::new(tool.tool_name, tool.input, capabilities.clone()));

    // --- Skill activation: detect `/skill-name` prefix in objective ---
    let (objective, skill_prompt, skill_allowed_tools) =
        if let Some(ref registry) = state.skill_registry {
            match praxis_skills::registry::try_activate_skill(registry, &request.objective) {
                Ok(Some((skill_state, remaining))) => {
                    let prompt = praxis_skills::registry::active_skill_prompt(&skill_state);
                    tracing::info!(
                        skill = %skill_state.name,
                        "skill activated via liquid prompt"
                    );
                    let allowed = skill_state.allowed_tools.clone();
                    // Use remaining text as objective, or a default if no remaining text
                    let obj = if remaining.is_empty() {
                        format!(
                            "The user activated the '{}' skill. Follow its instructions.",
                            skill_state.name
                        )
                    } else {
                        remaining
                    };
                    (obj, Some(prompt), allowed)
                }
                Ok(None) => (request.objective.clone(), None, None),
                Err(err) => {
                    tracing::warn!(error = %err, "skill activation failed");
                    (request.objective.clone(), None, None)
                }
            }
        } else {
            (request.objective.clone(), None, None)
        };

    let agent_span = vigil::spans::agent_span(session_id.as_str(), "arcan");
    let tick = state
        .runtime
        .tick_on_branch(
            &session_id,
            &branch,
            TickInput {
                objective,
                proposed_tool,
                system_prompt: skill_prompt,
                allowed_tools: skill_allowed_tools,
            },
        )
        .instrument(agent_span)
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

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/state",
    tag = "sessions",
    params(
        ("session_id" = String, Path, description = "Session identifier"),
        EventQuery,
    ),
    responses(
        (status = 200, description = "Current session state", body = StateResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/events",
    tag = "events",
    params(
        ("session_id" = String, Path, description = "Session identifier"),
        EventQuery,
    ),
    responses(
        (status = 200, description = "List of events", body = EventListResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/events/stream",
    tag = "events",
    params(
        ("session_id" = String, Path, description = "Session identifier"),
        StreamQuery,
    ),
    responses(
        (status = 200, description = "SSE event stream", content_type = "text/event-stream"),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/branches",
    tag = "branches",
    params(("session_id" = String, Path, description = "Session identifier")),
    request_body = CreateBranchRequest,
    responses(
        (status = 200, description = "Branch created", body = BranchInfoSchema),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    get,
    path = "/sessions/{session_id}/branches",
    tag = "branches",
    params(("session_id" = String, Path, description = "Session identifier")),
    responses(
        (status = 200, description = "List of branches", body = BranchListResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/branches/{branch_id}/merge",
    tag = "branches",
    params(
        ("session_id" = String, Path, description = "Session identifier"),
        ("branch_id" = String, Path, description = "Source branch to merge"),
    ),
    request_body = MergeBranchRequest,
    responses(
        (status = 200, description = "Branch merged", body = BranchMergeResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

#[utoipa::path(
    post,
    path = "/sessions/{session_id}/approvals/{approval_id}",
    tag = "approvals",
    params(
        ("session_id" = String, Path, description = "Session identifier"),
        ("approval_id" = String, Path, description = "Approval UUID"),
    ),
    request_body = ResolveApprovalRequest,
    responses(
        (status = 204, description = "Approval resolved"),
        (status = 400, description = "Invalid approval ID", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
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

// ─── Provider switching ──────────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/provider",
    tag = "provider",
    responses(
        (status = 200, description = "Current provider info", body = ProviderResponse)
    )
)]
async fn get_provider(
    State(state): State<CanonicalState>,
) -> Result<Json<ProviderResponse>, (StatusCode, Json<serde_json::Value>)> {
    let provider_name = state
        .provider_handle
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .name()
        .to_owned();
    let factory = state.provider_factory.clone();
    let available = tokio::task::spawn_blocking(move || factory.available_providers())
        .await
        .map_err(|e| internal_error(format!("available_providers task panicked: {e}")))?;
    Ok(Json(ProviderResponse {
        provider: provider_name,
        available,
    }))
}

#[utoipa::path(
    put,
    path = "/provider",
    tag = "provider",
    request_body = SetProviderRequest,
    responses(
        (status = 200, description = "Provider switched", body = ProviderResponse),
        (status = 400, description = "Invalid provider spec", body = ErrorResponse)
    )
)]
async fn set_provider(
    State(state): State<CanonicalState>,
    Json(request): Json<SetProviderRequest>,
) -> Result<Json<ProviderResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Provider constructors create `reqwest::blocking::Client`, which spawns an
    // internal Tokio runtime and panics if called from an async worker thread.
    // Move the build into a blocking thread to avoid the conflict.
    let factory = state.provider_factory.clone();
    let spec = request.provider.clone();
    let new_provider = tokio::task::spawn_blocking(move || factory.build(&spec))
        .await
        .map_err(|e| internal_error(format!("provider build task panicked: {e}")))?
        .map_err(bad_request)?;

    let new_name = new_provider.name().to_owned();
    {
        let mut guard = state
            .provider_handle
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = new_provider;
    }

    let factory = state.provider_factory.clone();
    let available = tokio::task::spawn_blocking(move || factory.available_providers())
        .await
        .map_err(|e| internal_error(format!("available_providers task panicked: {e}")))?;

    tracing::info!(provider = %new_name, spec = %request.provider, "Provider switched via API");

    Ok(Json(ProviderResponse {
        provider: new_name,
        available,
    }))
}

// ─── Error helpers ───────────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid() {
        let spec = openapi_spec();
        let json = serde_json::to_value(&spec).expect("spec should serialize to JSON");

        assert_eq!(json["info"]["title"], "Arcan Agent Runtime API");
        assert_eq!(json["info"]["version"], "0.2.1");

        let paths = json["paths"]
            .as_object()
            .expect("paths should be an object");
        assert!(paths.contains_key("/health"), "missing /health path");
        assert!(paths.contains_key("/sessions"), "missing /sessions path");
        assert!(
            paths.contains_key("/sessions/{session_id}/runs"),
            "missing runs path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/state"),
            "missing state path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/events"),
            "missing events path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/events/stream"),
            "missing stream path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/branches"),
            "missing branches path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/branches/{branch_id}/merge"),
            "missing merge path"
        );
        assert!(
            paths.contains_key("/sessions/{session_id}/approvals/{approval_id}"),
            "missing approval path"
        );
        assert!(paths.contains_key("/provider"), "missing provider path");

        // Verify typed schemas exist (not just Object)
        let schemas = json["components"]["schemas"]
            .as_object()
            .expect("schemas should be an object");
        assert!(schemas.contains_key("RunResponse"));
        assert!(schemas.contains_key("CreateSessionRequest"));
        assert!(schemas.contains_key("AgentStateVector"));
        assert!(schemas.contains_key("OperatingMode"));
        assert!(schemas.contains_key("BudgetState"));
        assert!(schemas.contains_key("RiskLevel"));
        assert!(schemas.contains_key("PolicySet"));
        assert!(schemas.contains_key("ModelRouting"));
        assert!(schemas.contains_key("SessionManifest"));
        assert!(schemas.contains_key("BranchInfo"));
        assert!(schemas.contains_key("BranchMergeResult"));
        assert!(schemas.contains_key("EventRecord"));
    }
}
