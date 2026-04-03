use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
    time::Instant,
};

use arcan_aios_adapters::tools::ToolHarnessObserver;

use aios_protocol::{
    AgentIdentityProvider, AgentStateVector, BasicIdentity, BranchId, BranchInfo,
    BranchMergeResult, EventKind, EventRecord, ModelRouting, OperatingMode, PolicySet, SessionId,
    ToolCall,
};
use aios_runtime::{KernelRuntime, TickInput};
use arcan_core::runtime::{ProviderFactory, SwappableProviderHandle};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{fs, sync::mpsc};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tracing::Instrument;
use utoipa::{IntoParams, OpenApi, PartialSchema, ToSchema};
use utoipa_scalar::{Scalar, Servable};
use uuid::Uuid;

use crate::auth::{
    AuthConfig, IdentityClaims, JwtError, Tier, jwt_auth_middleware, validate_identity_token,
};

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
    /// Root directory for per-session sandbox directories (BRO-215).
    data_dir: Arc<std::path::PathBuf>,
    /// Workspace root (cwd at daemon startup) — used for git context and project instructions.
    workspace_root: Arc<std::path::PathBuf>,
    /// Cached project instructions (CLAUDE.md, AGENTS.md, docs/, .control/policy.yaml).
    /// Loaded once at startup; project instructions rarely change during a session.
    cached_project_instructions: Option<String>,
    /// Shared skill registry for activation and tool filtering.
    skill_registry: Option<Arc<praxis_skills::registry::SkillRegistry>>,
    /// Observers notified on run completion (async judge evaluators, EGRI bridge).
    run_observers: Vec<Arc<dyn ToolHarnessObserver>>,
    /// Agent identity provider — supplies persona, DID, capabilities, and policy.
    identity: Arc<dyn AgentIdentityProvider>,
    /// Routes memory events for anonymous sessions to ephemeral discard (BRO-217).
    session_selector: Option<Arc<arcan_lago::SessionJournalSelector>>,
    /// Retention journal — TTL-tags free-tier (7-day) and pro-tier (90-day) session events.
    /// BRO-218: free sessions registered with default config.
    /// BRO-219: pro sessions registered with LagoPolicyConfig::pro().
    free_tier_journal: Option<Arc<arcan_lago::FreeTierJournal>>,
    /// Secret for verifying Anima identity tokens (BRO-221).
    /// Resolves from `ANIMA_JWT_SECRET`, falling back to `AUTH_SECRET`.
    /// When `None`, identity token verification is skipped (local dev).
    anima_secret: Option<String>,
    /// In-memory token-bucket rate limiter (BRO-223).
    /// Shared across all request handlers via `Arc`.
    rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Frozen per-session prompt prefixes for provider cache preservation (BRO-424).
    frozen_prompt_prefixes: Arc<Mutex<HashMap<String, FrozenPromptPrefix>>>,
}

#[derive(Debug, Clone)]
struct FrozenPromptPrefix {
    system_prompt_prefix: String,
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
    /// Short-lived Anima identity token (BRO-221).
    ///
    /// When provided, arcand verifies the JWT signature and derives the
    /// session `PolicySet` from the embedded claims. This prevents clients
    /// from forging a higher tier by supplying a crafted policy in
    /// `CreateSessionRequest`. When absent, the session-stored policy is
    /// used (backward-compatible with clients that do not yet send tokens).
    identity_token: Option<String>,
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
    /// Unique ID for the assistant message (used as `messageId` in the Vercel
    /// AI SDK v6 `start` frame). Callers should supply a fresh UUID per turn so
    /// React has a unique key for each assistant message in the same session.
    /// Falls back to `session_id` when absent.
    message_id: Option<String>,
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
        upgrade_session_identity,
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
        list_mcp_servers,
    ),
    components(schemas(
        // Request / response types
        McpServerEntry,
        McpServerListResponse,
        UpgradeSessionIdentityRequest,
        UpgradeSessionIdentityResponse,
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
        (name = "mcp", description = "MCP server registry"),
    )
)]
struct ApiDoc;

// ─── MCP server registry endpoint (BRO-226) ──────────────────────────────────

/// A single entry in the MCP server registry response.
#[derive(Debug, Serialize, ToSchema)]
struct McpServerEntry {
    /// Server name as declared in SKILL.md `mcp_servers[].name`.
    name: &'static str,
    /// Human-readable description of the server.
    description: &'static str,
    /// Minimum tier name required to use this server (e.g. `"free"`, `"pro"`).
    min_tier: &'static str,
}

/// Response for `GET /mcp-servers`.
#[derive(Debug, Serialize, ToSchema)]
struct McpServerListResponse {
    /// All servers in the registry with their minimum tier requirement.
    servers: Vec<McpServerEntry>,
    /// The subset of servers accessible at the requested tier.
    allowed_for_tier: Vec<McpServerEntry>,
    /// The tier used for filtering (from the `?tier=` query parameter).
    tier: String,
}

#[derive(Debug, Deserialize, IntoParams)]
struct McpServerQuery {
    /// Tier to filter the server list for (default: `"anonymous"`).
    /// One of: `anonymous`, `free`, `pro`, `enterprise`.
    tier: Option<String>,
}

#[utoipa::path(
    get,
    path = "/mcp-servers",
    tag = "mcp",
    params(McpServerQuery),
    responses(
        (status = 200, description = "MCP server registry", body = McpServerListResponse)
    )
)]
async fn list_mcp_servers(Query(query): Query<McpServerQuery>) -> Json<McpServerListResponse> {
    let tier = match query.tier.as_deref().unwrap_or("anonymous") {
        "free" => crate::auth::Tier::Free,
        "pro" => crate::auth::Tier::Pro,
        "enterprise" => crate::auth::Tier::Enterprise,
        _ => crate::auth::Tier::Anonymous,
    };
    let tier_name = format!("{tier:?}").to_lowercase();

    let all: Vec<McpServerEntry> = crate::mcp_registry::APPROVED_MCP_SERVERS
        .iter()
        .map(|s| McpServerEntry {
            name: s.name,
            description: s.description,
            min_tier: s.min_tier_name,
        })
        .collect();

    let allowed: Vec<McpServerEntry> = crate::mcp_registry::allowed_servers_for_tier(&tier)
        .into_iter()
        .map(|s| McpServerEntry {
            name: s.name,
            description: s.description,
            min_tier: s.min_tier_name,
        })
        .collect();

    Json(McpServerListResponse {
        servers: all,
        allowed_for_tier: allowed,
        tier: tier_name,
    })
}

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
    create_canonical_router_with_skills(
        runtime,
        provider_handle,
        provider_factory,
        None,
        None,
        Vec::new(),
        None,
        std::env::temp_dir(),
        None, // workspace_root
        None, // session_selector (BRO-217)
        None, // free_tier_journal (BRO-218)
    )
}

/// Create the canonical router with an optional skill registry for activation.
///
/// Health check endpoints (`/health`, `/healthz`) are always unprotected
/// so Railway / Kubernetes probes can reach them without a token.
///
/// All other endpoints are protected by JWT auth middleware when
/// `ARCAN_JWT_SECRET` or `AUTH_SECRET` is configured. If neither env var
/// is set, auth is disabled and all routes are open (local dev mode).
#[allow(clippy::too_many_arguments)]
pub fn create_canonical_router_with_skills(
    runtime: Arc<KernelRuntime>,
    provider_handle: SwappableProviderHandle,
    provider_factory: Arc<dyn ProviderFactory>,
    skill_registry: Option<Arc<praxis_skills::registry::SkillRegistry>>,
    score_store: Option<nous_api::ScoreStore>,
    run_observers: Vec<Arc<dyn ToolHarnessObserver>>,
    identity: Option<Arc<dyn AgentIdentityProvider>>,
    data_dir: impl Into<std::path::PathBuf>,
    workspace_root: Option<std::path::PathBuf>,
    session_selector: Option<Arc<arcan_lago::SessionJournalSelector>>,
    free_tier_journal: Option<Arc<arcan_lago::FreeTierJournal>>,
) -> Router {
    let identity: Arc<dyn AgentIdentityProvider> =
        identity.unwrap_or_else(|| Arc::new(BasicIdentity::default()));
    tracing::info!(
        agent_id = %identity.agent_id(),
        agent_name = %identity.soul_profile().name,
        "agent identity initialized"
    );

    // BRO-221: Resolve the Anima identity-token secret.
    // `ANIMA_JWT_SECRET` is preferred (dedicated key for identity tokens).
    // Falls back to `AUTH_SECRET` (same shared secret used for bearer auth).
    // When neither is set, identity token verification is skipped (local dev).
    let anima_secret = std::env::var("ANIMA_JWT_SECRET")
        .or_else(|_| std::env::var("AUTH_SECRET"))
        .ok()
        .filter(|s| !s.is_empty());

    if anima_secret.is_some() {
        tracing::info!("Anima identity token verification enabled (BRO-221)");
    } else {
        tracing::warn!(
            "No ANIMA_JWT_SECRET or AUTH_SECRET set — identity token verification DISABLED. \
             Clients can supply any PolicySet without arcand verification."
        );
    }

    // BRO-366/375: Resolve workspace root and cache project instructions at startup.
    let ws_root = workspace_root
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(std::env::temp_dir);
    let cached_project_instructions = arcan_core::prompt::load_project_instructions(&ws_root);
    if cached_project_instructions.is_some() {
        tracing::info!(
            workspace = %ws_root.display(),
            "loaded project instructions for liquid prompt (BRO-375)"
        );
    }

    let state = CanonicalState {
        runtime,
        provider_handle,
        provider_factory,
        started_at: Instant::now(),
        data_dir: Arc::new(data_dir.into()),
        workspace_root: Arc::new(ws_root),
        cached_project_instructions,
        skill_registry,
        run_observers,
        identity,
        session_selector,
        free_tier_journal,
        anima_secret,
        rate_limiter: Arc::new(crate::rate_limit::RateLimiter::new()),
        frozen_prompt_prefixes: Arc::new(Mutex::new(HashMap::new())),
    };

    let auth_config = Arc::new(AuthConfig::from_env());

    // Public routes — no auth required (health checks, API docs, MCP registry).
    let public = Router::new()
        .route("/health", get(health))
        .route("/healthz", get(health))
        .route("/openapi.json", get(openapi_json))
        .route("/mcp-servers", get(list_mcp_servers))
        .merge(Scalar::with_url("/docs", ApiDoc::openapi()))
        .with_state(state.clone());

    // Protected routes — JWT auth middleware applied.
    let protected = Router::new()
        .route("/sessions", get(list_sessions).post(create_session))
        .route(
            "/sessions/{session_id}/identity",
            patch(upgrade_session_identity),
        )
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
        // BRO-219: Memory export and migration endpoints.
        .route("/user/memory/export", get(export_memory_jsonl))
        .route("/user/memory/migrate-to-pro", post(migrate_memory_to_pro))
        .layer(axum::middleware::from_fn_with_state(
            auth_config,
            jwt_auth_middleware,
        ))
        .with_state(state);

    let mut router = public.merge(protected);

    // Nest Nous eval routes at /nous when a ScoreStore is available.
    if let Some(store) = score_store {
        // Use the default heuristic count (6) as the evaluator_count for the API health response.
        // The actual evaluators are managed by NousToolObserver; this count is informational.
        router = router.nest("/nous", nous_api::nous_router(store, 6));
    }

    router
}

// ─── Identity → PolicySet mapping ────────────────────────────────────────────

/// Capability strings for the free tier (sandboxed read-only shell).
const FREE_TIER_CAPS: &[&str] = &[
    "exec:cmd:cat",
    "exec:cmd:ls",
    "exec:cmd:grep",
    "exec:cmd:find",
    "exec:cmd:wc",
    "exec:cmd:head",
    "exec:cmd:tail",
    "exec:cmd:echo",
];

/// Capability strings for enterprise `Member` role (BRO-222).
///
/// Sandboxed shell + project-workspace file writes; no admin APIs.
const MEMBER_CAPS: &[&str] = &[
    "exec:cmd:cat",
    "exec:cmd:ls",
    "exec:cmd:grep",
    "exec:cmd:find",
    "exec:cmd:wc",
    "exec:cmd:head",
    "exec:cmd:tail",
    "exec:cmd:echo",
    "exec:cmd:mkdir",
    "exec:cmd:cp",
    "exec:cmd:mv",
    "fs:read:**",
    "fs:write:project:**",
];

/// Capability strings for enterprise `Viewer` role (BRO-222).
///
/// Read-only: no shell mutations, no file writes.
const VIEWER_CAPS: &[&str] = &["fs:read:**", "exec:cmd:cat", "exec:cmd:ls", "exec:cmd:grep"];

/// Build a `PolicySet` from a slice of capability strings.
fn caps(raw: &[&str]) -> PolicySet {
    PolicySet {
        allow_capabilities: raw
            .iter()
            .map(|s| aios_protocol::Capability::new(s.to_string()))
            .collect(),
        ..PolicySet::default()
    }
}

/// Derive the `PolicySet` for an enterprise tenant role (BRO-222).
///
/// Role matrix:
///
/// | Role    | Shell           | File Write         | Admin APIs |
/// |---------|-----------------|--------------------|------------|
/// | Admin   | Full            | Full               | Full       |
/// | Member  | Sandboxed       | Project workspace  | None       |
/// | Viewer  | Read-only cmds  | None               | None       |
/// | Agent   | Configurable    | Configurable       | None       |
///
/// For `Agent`, `custom_capabilities` in the claims is the configurable
/// capability set (falls back to wildcard if absent).
fn enterprise_policy_for_role(
    role: &crate::auth::TenantRole,
    custom: Option<&Vec<String>>,
) -> PolicySet {
    use crate::auth::TenantRole;
    match role {
        TenantRole::Admin => caps(&["*"]),
        TenantRole::Member => caps(MEMBER_CAPS),
        TenantRole::Viewer => caps(VIEWER_CAPS),
        TenantRole::Agent => {
            // Agent role is configurable via custom_capabilities.
            if let Some(cc) = custom {
                if !cc.is_empty() {
                    return PolicySet {
                        allow_capabilities: cc
                            .iter()
                            .map(|c| aios_protocol::Capability::new(c.clone()))
                            .collect(),
                        ..PolicySet::default()
                    };
                }
            }
            caps(&["*"])
        }
    }
}

/// Derive a `PolicySet` from verified Anima identity claims (BRO-221 / BRO-222).
///
/// Priority order:
/// 1. `custom_capabilities` in claims — tenant admin override (always wins)
/// 2. Role-based policy for Enterprise tier (BRO-222)
/// 3. Tier default
///
/// | Tier        | Default allow_capabilities                                    |
/// |-------------|---------------------------------------------------------------|
/// | Anonymous   | `[]` — no shell, no file writes, no memory persist            |
/// | Free        | Named read-only shell commands (sandboxed)                    |
/// | Pro         | `["*"]` — full wildcard                                       |
/// | Enterprise  | Role-based (Admin=`*`, Member=sandboxed+write, Viewer=read)   |
fn policy_from_identity_claims(claims: &IdentityClaims) -> PolicySet {
    // custom_capabilities always overrides role/tier defaults.
    if let Some(custom) = &claims.custom_capabilities {
        if !custom.is_empty() {
            return PolicySet {
                allow_capabilities: custom
                    .iter()
                    .map(|c| aios_protocol::Capability::new(c.clone()))
                    .collect(),
                ..PolicySet::default()
            };
        }
    }

    match claims.tier {
        // Anonymous: no capability grant — all tool calls are blocked.
        Tier::Anonymous => PolicySet {
            allow_capabilities: vec![],
            ..PolicySet::default()
        },
        Tier::Free => caps(FREE_TIER_CAPS),
        Tier::Pro => caps(&["*"]),
        Tier::Enterprise => {
            // Use the first role when multiple are present (BRO-222: caller
            // should supply the most-specific / least-privileged role).
            if let Some(role) = claims.roles.first() {
                enterprise_policy_for_role(role, claims.custom_capabilities.as_ref())
            } else {
                // No roles: full enterprise access (backward compat / service accounts).
                caps(&["*"])
            }
        }
    }
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
            "sdk": "life-vigil",
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

// ─── BRO-227: Anonymous identity upgrade ─────────────────────────────────────

/// Request body for `PATCH /sessions/{session_id}/identity`.
#[derive(Debug, Deserialize, ToSchema)]
struct UpgradeSessionIdentityRequest {
    /// New authenticated user ID to assign as session owner.
    user_id: String,
    /// Optional Anima identity token. When supplied and arcand has a secret
    /// configured, the token is verified and the `PolicySet` is re-derived from
    /// the embedded claims. When omitted (or when no secret is set), the policy
    /// defaults to `free` tier.
    identity_token: Option<String>,
}

/// Response from `PATCH /sessions/{session_id}/identity`.
#[derive(Debug, Serialize, ToSchema)]
struct UpgradeSessionIdentityResponse {
    session_id: String,
    new_owner: String,
    /// Effective tier after upgrade (`"free"`, `"pro"`, `"enterprise"`).
    tier: String,
    /// Number of capabilities in the upgraded `PolicySet`.
    capabilities_count: usize,
}

#[utoipa::path(
    patch,
    path = "/sessions/{session_id}/identity",
    tag = "sessions",
    params(("session_id" = String, Path, description = "Session identifier")),
    request_body = UpgradeSessionIdentityRequest,
    responses(
        (status = 200, description = "Identity upgraded", body = UpgradeSessionIdentityResponse),
        (status = 404, description = "Session not found", body = ErrorResponse),
        (status = 401, description = "Invalid identity token", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse),
    )
)]
async fn upgrade_session_identity(
    Path(session_id): Path<String>,
    State(state): State<CanonicalState>,
    Json(body): Json<UpgradeSessionIdentityRequest>,
) -> Result<Json<UpgradeSessionIdentityResponse>, (StatusCode, Json<serde_json::Value>)> {
    let session_id = SessionId::from_string(session_id);

    if !state.runtime.session_exists(&session_id) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("session not found: {}", session_id) })),
        ));
    }

    // Resolve the new PolicySet from the identity token when one is supplied.
    let (new_policy, tier_label) = match (&body.identity_token, &state.anima_secret) {
        (Some(token), Some(secret)) => match validate_identity_token(token, secret) {
            Ok(claims) => {
                let tier = match &claims.tier {
                    Tier::Enterprise => "enterprise",
                    Tier::Pro => "pro",
                    _ => "free",
                };
                (policy_from_identity_claims(&claims), tier.to_owned())
            }
            Err(e) => {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": format!("invalid identity token: {}", e) })),
                ));
            }
        },
        // No token or no secret — upgrade to free-tier policy.
        _ => (caps(FREE_TIER_CAPS), "free".to_owned()),
    };

    let capabilities_count = new_policy.allow_capabilities.len();

    // BRO-227: Register the session with the tier journal under the new user_id
    // so subsequent memory events are retained (TTL-tagged) rather than discarded
    // as anonymous ephemeral events.
    // NOTE: runtime.upgrade_session_owner() requires aiOS KernelRuntime changes
    // (tracked separately); for now we register the journal tier and log the upgrade.
    if let Some(ref ftj) = state.free_tier_journal {
        ftj.register_session(session_id.as_str(), &body.user_id);
    }
    if let Some(ref selector) = state.session_selector {
        selector.unmark_ephemeral(session_id.as_str());
    }

    tracing::info!(
        session_id = %session_id,
        new_owner = %body.user_id,
        tier = %tier_label,
        "session identity upgraded (BRO-227)"
    );

    Ok(Json(UpgradeSessionIdentityResponse {
        session_id: session_id.as_str().to_owned(),
        new_owner: body.user_id,
        tier: tier_label,
        capabilities_count,
    }))
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

    // --- Tier-aware tool catalog filtering + sandbox (BRO-214 / BRO-215 / BRO-218) ---
    // Fetch the session manifest once: used for policy, owner (user_id), and tier detection.
    let session_manifest = state
        .runtime
        .list_sessions()
        .into_iter()
        .find(|m| m.session_id == session_id);

    // BRO-221: If the caller supplies an identity_token, verify it and derive
    // PolicySet from the claims. This prevents clients from forging a higher
    // tier by supplying a crafted policy in CreateSessionRequest.
    // When no token is supplied (or no secret is configured), fall back to
    // the session-stored policy (backward-compatible).
    let verified_identity_claims: Option<IdentityClaims> =
        match (&request.identity_token, &state.anima_secret) {
            (Some(token), Some(secret)) => {
                match validate_identity_token(token, secret) {
                    Ok(claims) => {
                        tracing::info!(
                            session = %session_id,
                            sub = %claims.sub,
                            tier = ?claims.tier,
                            "identity token verified; PolicySet derived from claims"
                        );
                        Some(claims)
                    }
                    Err(err) => {
                        // BRO-224: Emit audit event before returning the error.
                        let violation_type = match err {
                            JwtError::Expired => {
                                arcan_lago::policy_violation::ViolationType::TokenExpired
                            }
                            _ => arcan_lago::policy_violation::ViolationType::AuthenticationError,
                        };
                        let _ = state
                            .runtime
                            .record_external_event(
                                &session_id,
                                arcan_lago::policy_violation::event_kind(
                                    &arcan_lago::policy_violation::PolicyViolationData {
                                        violation_type,
                                        capability: None,
                                        attempted_value: None,
                                        tier: "unknown".to_string(),
                                        subject: session_id.as_str().to_string(),
                                    },
                                ),
                            )
                            .await;
                        tracing::warn!(
                            session = %session_id,
                            error = %err,
                            "identity token verification failed"
                        );
                        return Err((
                            StatusCode::UNAUTHORIZED,
                            Json(json!({
                                "error": "authentication_error",
                                "message": err.to_string()
                            })),
                        ));
                    }
                }
            }
            // No token provided, or no secret configured — skip verification.
            _ => None,
        };

    let session_policy: aios_protocol::PolicySet = verified_identity_claims
        .as_ref()
        .map(policy_from_identity_claims)
        .or_else(|| {
            session_manifest.as_ref().and_then(|m| {
                serde_json::from_value::<aios_protocol::PolicySet>(m.policy.clone()).ok()
            })
        })
        .unwrap_or_default();

    // Session owner: use verified sub claim when available; else session manifest owner.
    // The sub is used as user_id for free-tier Lago namespace isolation (BRO-218).
    let session_owner: String = verified_identity_claims
        .as_ref()
        .map(|c| c.sub.clone())
        .or_else(|| session_manifest.as_ref().map(|m| m.owner.clone()))
        .unwrap_or_else(|| "anonymous".to_owned());

    // BRO-223: Enforce per-user rate limits in arcand (defense-in-depth).
    // Key: "{tier_name}:{user_id}" — one bucket per user, scoped to their tier.
    // Anonymous users are keyed by session_id to avoid shared exhaustion between
    // unrelated guests (no persistent user_id available for anon requests).
    {
        let tier = verified_identity_claims
            .as_ref()
            .map(|c| &c.tier)
            .map(|t| match t {
                Tier::Anonymous => Tier::Anonymous,
                Tier::Free => Tier::Free,
                Tier::Pro => Tier::Pro,
                Tier::Enterprise => Tier::Enterprise,
            })
            .unwrap_or_else(|| {
                // Fall back to tier derived from session policy capability inspection.
                if session_policy
                    .allow_capabilities
                    .iter()
                    .any(|c| c.as_str() == "*")
                {
                    Tier::Pro
                } else if session_policy
                    .allow_capabilities
                    .iter()
                    .any(|c| c.as_str().starts_with("exec:cmd:"))
                {
                    Tier::Free
                } else {
                    Tier::Anonymous
                }
            });

        let tier_name = format!("{tier:?}").to_lowercase();
        let bucket_key = if matches!(tier, Tier::Anonymous) {
            // Anonymous: key by session to avoid cross-session interference
            format!("{tier_name}:{session_id}")
        } else {
            format!("{tier_name}:{session_owner}")
        };

        if let Err(rl_err) = state.rate_limiter.check(&bucket_key, &tier) {
            // BRO-224: Emit audit event before returning the error.
            let _ = state
                .runtime
                .record_external_event(
                    &session_id,
                    arcan_lago::policy_violation::event_kind(
                        &arcan_lago::policy_violation::PolicyViolationData {
                            violation_type:
                                arcan_lago::policy_violation::ViolationType::RateLimitExceeded,
                            capability: None,
                            attempted_value: None,
                            tier: tier_name.clone(),
                            subject: session_owner.clone(),
                        },
                    ),
                )
                .await;
            tracing::warn!(
                session = %session_id,
                user = %session_owner,
                tier = %tier_name,
                retry_after = rl_err.retry_after,
                "rate limit exceeded (BRO-223)"
            );
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": "rate_limit_exceeded",
                    "retry_after": rl_err.retry_after,
                    "limit": rl_err.limit_per_minute,
                    "tier": rl_err.tier,
                })),
            ));
        }
    }

    // BRO-214: Derive which tools are safe to expose for this tier.
    // The authoritative enforcement is still policy evaluation at execution time
    // (BRO-213); this layer only affects what the LLM sees.
    let tier_allowed_tools: Option<Vec<String>> =
        arcan_aios_adapters::tools_allowed_by_policy(&session_policy);

    // BRO-217: Detect anonymous sessions (no exec:cmd:* in allow_capabilities, no wildcard).
    // Anonymous sessions have memory events discarded (ephemeral Lago isolation).
    // Free sessions have exec:cmd:cat/ls/etc and keep their memory (BRO-218 adds TTL).
    // Pro/enterprise have allow_capabilities: ["*"] and are never marked ephemeral.
    let is_anonymous_tier = !session_policy
        .allow_capabilities
        .iter()
        .any(|c| c.as_str().starts_with("exec:cmd:") || c.as_str() == "*");

    // BRO-218: Detect free-tier sessions (has exec:cmd:* capabilities but no wildcard).
    // Free sessions get TTL-tagged memory events (7-day rolling retention).
    let has_wildcard = session_policy
        .allow_capabilities
        .iter()
        .any(|c| c.as_str() == "*");
    let is_free_tier = !is_anonymous_tier && !has_wildcard;
    // BRO-219: Pro/enterprise sessions have the wildcard capability grant.
    let is_pro_tier = has_wildcard;

    // BRO-215: Prepare a per-session sandbox directory for restricted tiers.
    // Pro/enterprise sessions return None (full workspace root access).
    let sandbox_path: Option<std::path::PathBuf> = {
        let enforcer = arcan_aios_adapters::SandboxEnforcer::new(state.data_dir.as_ref());
        match enforcer.prepare(session_id.as_str(), &session_policy) {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!(
                    session = %session_id,
                    error = %err,
                    "failed to create session sandbox directory (non-fatal)"
                );
                None
            }
        }
    };

    // --- Skill activation: detect `/skill-name` prefix in objective ---
    // Activation is blocked when the skill's declared allowed_tools fall outside
    // the tier's safe set, preventing privilege escalation via skill invocation.
    let (objective, skill_prompt, skill_allowed_tools) =
        if let Some(ref registry) = state.skill_registry {
            match praxis_skills::registry::try_activate_skill(registry, &request.objective) {
                Ok(Some((skill_state, remaining))) => {
                    // BRO-226: Derive the current tier for MCP server gating.
                    let tier_for_mcp = verified_identity_claims
                        .as_ref()
                        .map(|c| c.tier.clone())
                        .unwrap_or_else(|| {
                            if is_anonymous_tier {
                                Tier::Anonymous
                            } else if is_free_tier {
                                Tier::Free
                            } else {
                                Tier::Pro
                            }
                        });

                    // Tier gating: block if skill requires tools beyond this tier.
                    let tier_blocked = if let Some(ref safe_tools) = tier_allowed_tools {
                        let safe_set: std::collections::HashSet<&str> =
                            safe_tools.iter().map(String::as_str).collect();
                        match &skill_state.allowed_tools {
                            Some(tools) => !tools.iter().all(|t| safe_set.contains(t.as_str())),
                            None => true, // unknown tool requirements → block for restricted tiers
                        }
                    } else {
                        false // pro/enterprise: no restriction
                    };

                    // BRO-226: Also block if any declared MCP server is not allowed for the tier.
                    // Anonymous sessions may not connect to any MCP server.
                    // Free/Pro sessions may only use pre-approved servers.
                    let mcp_blocked = skill_state.mcp_servers.as_ref().is_some_and(|servers| {
                        servers.iter().any(|s| {
                            !crate::mcp_registry::is_mcp_server_allowed(&s.name, &tier_for_mcp)
                        })
                    });

                    if tier_blocked || mcp_blocked {
                        // BRO-224: Emit audit event for skill-not-allowed violation.
                        let tier_str = format!("{tier_for_mcp:?}").to_lowercase();
                        let violation_type = if mcp_blocked && !tier_blocked {
                            arcan_lago::policy_violation::ViolationType::CapabilityBlocked
                        } else {
                            arcan_lago::policy_violation::ViolationType::SkillNotAllowed
                        };
                        let attempted = Some(if mcp_blocked {
                            let blocked_server = skill_state
                                .mcp_servers
                                .as_ref()
                                .and_then(|s| {
                                    s.iter().find(|srv| {
                                        !crate::mcp_registry::is_mcp_server_allowed(
                                            &srv.name,
                                            &tier_for_mcp,
                                        )
                                    })
                                })
                                .map(|s| s.name.as_str())
                                .unwrap_or("unknown");
                            format!("{}:{}", skill_state.name, blocked_server)
                        } else {
                            skill_state.name.clone()
                        });
                        let _ = state
                            .runtime
                            .record_external_event(
                                &session_id,
                                arcan_lago::policy_violation::event_kind(
                                    &arcan_lago::policy_violation::PolicyViolationData {
                                        violation_type,
                                        capability: Some("mcp:connect".to_string()),
                                        attempted_value: attempted,
                                        tier: tier_str,
                                        subject: session_owner.clone(),
                                    },
                                ),
                            )
                            .await;
                        tracing::warn!(
                            skill = %skill_state.name,
                            mcp_blocked,
                            tier_blocked,
                            tier = ?tier_for_mcp,
                            "skill activation blocked by tier policy (BRO-226)"
                        );
                        (request.objective.clone(), None, None)
                    } else {
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

    // Build tier-filtered skill catalog for per-session injection.
    let skill_catalog = state
        .skill_registry
        .as_ref()
        .map(|registry| build_tiered_skill_catalog(registry, tier_allowed_tools.as_deref()))
        .unwrap_or_default();

    // Build the stable system-prompt prefix once per session so mid-session
    // memory, git context, or skill-registry changes do not invalidate provider
    // prompt caches. Dynamic active-skill prompts remain appended per turn.
    let persona = state.identity.persona_block();
    let sandbox_note = sandbox_path
        .as_ref()
        .map(|p| {
            format!(
                "\n\n## Session Workspace\n\
                 Your isolated workspace for this session is: `{}`\n\
                 Use this directory for all file read and write operations.",
                p.display()
            )
        })
        .unwrap_or_default();
    let provider_name = state
        .provider_handle
        .read()
        .ok()
        .map(|p| p.name().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let model_name = std::env::var("ARCAN_MODEL")
        .or_else(|_| std::env::var("MODEL"))
        .unwrap_or_else(|_| "default".to_string());
    let frozen_prompt_prefix = state.frozen_prompt_prefix(
        session_id.as_str(),
        build_system_prompt_prefix(
            &state.workspace_root,
            state.cached_project_instructions.as_deref(),
            &state.data_dir.join("memory"),
            &provider_name,
            &model_name,
            &skill_catalog,
            &persona,
            &sandbox_note,
        ),
    );
    let system_prompt = Some(append_prompt_suffix(
        &frozen_prompt_prefix.system_prompt_prefix,
        skill_prompt.as_deref(),
    ));

    // Combine tier restriction with any active skill's allowed_tools.
    // When both restrict, use their intersection (more restrictive always wins).
    let combined_allowed_tools: Option<Vec<String>> =
        match (tier_allowed_tools, skill_allowed_tools) {
            (None, skill) => skill,
            (tier, None) => tier,
            (Some(tier), Some(skill)) => {
                let tier_set: std::collections::HashSet<&str> =
                    tier.iter().map(String::as_str).collect();
                let intersection: Vec<String> = skill
                    .into_iter()
                    .filter(|t| tier_set.contains(t.as_str()))
                    .collect();
                Some(intersection)
            }
        };

    tracing::debug!(
        agent_id = %state.identity.agent_id(),
        did = state.identity.did().unwrap_or("none"),
        tool_filter = ?combined_allowed_tools.as_ref().map(Vec::len),
        "running agent tick with identity"
    );

    // BRO-217: For anonymous sessions, mark as ephemeral so that MemoryProposed /
    // MemoryCommitted events are discarded instead of persisted to Lago.
    // Free sessions retain memory with 7-day TTL (BRO-218).
    // Pro/enterprise sessions are never marked ephemeral.
    if is_anonymous_tier {
        if let Some(ref selector) = state.session_selector {
            selector.mark_ephemeral(session_id.as_str());
        }
    }

    // BRO-218/219: Register with the retention journal before the tick so that
    // all memory events appended during this tick are TTL-tagged with the
    // appropriate tier policy.
    if is_free_tier {
        if let Some(ref ftj) = state.free_tier_journal {
            ftj.register_session(session_id.as_str(), &session_owner);
        }
    } else if is_pro_tier {
        if let Some(ref ftj) = state.free_tier_journal {
            ftj.register_session_with_config(
                session_id.as_str(),
                &session_owner,
                arcan_lago::LagoPolicyConfig::pro(),
            );
        }
    }

    let agent_span = life_vigil::spans::agent_span(session_id.as_str(), "arcan");
    let tick_result = state
        .runtime
        .tick_on_branch(
            &session_id,
            &branch,
            TickInput {
                objective,
                proposed_tool,
                system_prompt,
                allowed_tools: combined_allowed_tools,
            },
        )
        .instrument(agent_span)
        .await;

    // Always unmark after tick completes (success or error) to avoid leaking the
    // ephemeral registration across future requests on the same session.
    if is_anonymous_tier {
        if let Some(ref selector) = state.session_selector {
            selector.unmark_ephemeral(session_id.as_str());
        }
    }

    // BRO-218/219: Always unregister after tick completes (free or pro) so the
    // retention journal does not accumulate stale session → user_id mappings.
    if is_free_tier || is_pro_tier {
        if let Some(ref ftj) = state.free_tier_journal {
            ftj.unregister_session(session_id.as_str());
        }
    }

    let tick = tick_result.map_err(internal_error)?;

    persist_last_session_hint(state.runtime.as_ref(), &tick.session_id).await;

    // Fire-and-forget: notify run observers (async judge evaluators, EGRI bridge).
    // This never blocks the HTTP response — observers run in background tasks.
    // Extract final answer and assistant messages from session events for judge context.
    if !state.run_observers.is_empty() {
        let observers = state.run_observers.clone();
        let sid = tick.session_id.as_str().to_owned();
        let obj = Some(request.objective.clone());
        let runtime_for_events = state.runtime.clone();
        let tick_session = tick.session_id.clone();
        tokio::spawn(async move {
            // Query recent events to extract final answer and assistant text.
            let (final_answer, assistant_messages) =
                extract_run_context(&runtime_for_events, &tick_session).await;

            for observer in &observers {
                observer
                    .on_run_finished(
                        sid.clone(),
                        obj.clone(),
                        final_answer.clone(),
                        assistant_messages.clone(),
                    )
                    .await;
            }
        });
    }

    Ok(Json(RunResponse {
        session_id: tick.session_id,
        mode: tick.mode,
        state: tick.state,
        events_emitted: tick.events_emitted,
        last_sequence: tick.last_sequence,
    }))
}

/// Extract final answer and assistant messages from the most recent run events.
///
/// Queries the session's event history and collects `Message` and `TextDelta`
/// events to reconstruct what the agent said. Returns `(final_answer, assistant_messages)`.
async fn extract_run_context(
    runtime: &KernelRuntime,
    session_id: &SessionId,
) -> (Option<String>, Option<String>) {
    let Ok(events) = runtime.read_events(session_id, 0, 1000).await else {
        return (None, None);
    };

    let mut assistant_texts = Vec::new();
    let mut final_answer = None;

    for record in &events {
        match &record.kind {
            aios_protocol::event::EventKind::Message { content, role, .. }
                if role == "assistant" =>
            {
                assistant_texts.push(content.clone());
                final_answer = Some(content.clone());
            }
            aios_protocol::event::EventKind::TextDelta { delta, .. } => {
                assistant_texts.push(delta.clone());
                final_answer = Some(delta.clone());
            }
            aios_protocol::event::EventKind::RunFinished {
                final_answer: Some(fa),
                ..
            } => {
                final_answer = Some(fa.clone());
            }
            _ => {}
        }
    }

    let messages = if assistant_texts.is_empty() {
        None
    } else {
        Some(assistant_texts.join("\n"))
    };

    (final_answer, messages)
}

fn build_system_prompt_prefix(
    workspace_root: &std::path::Path,
    cached_project_instructions: Option<&str>,
    memory_dir: &std::path::Path,
    provider_name: &str,
    model_name: &str,
    skill_catalog: &str,
    persona: &str,
    sandbox_note: &str,
) -> String {
    let mut sections = Vec::new();

    sections.push(format!("{persona}{sandbox_note}"));
    sections.push(arcan_core::prompt::build_environment_section(
        workspace_root,
        provider_name,
        model_name,
    ));

    if let Some(git) = arcan_core::prompt::build_git_section(workspace_root) {
        sections.push(git);
    }

    if let Some(instructions) = cached_project_instructions {
        sections.push(format!("# Project Instructions\n\n{instructions}"));
    }

    if let Some(memory) = arcan_core::prompt::build_memory_section(memory_dir) {
        sections.push(memory);
    }

    if !skill_catalog.is_empty() {
        sections.push(skill_catalog.to_owned());
    }

    sections.push(arcan_core::prompt::build_guidelines_section());
    sections.join("\n\n---\n\n")
}

fn append_prompt_suffix(prefix: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(suffix) => format!("{prefix}\n\n---\n\n{suffix}"),
        None => prefix.to_owned(),
    }
}

impl CanonicalState {
    fn frozen_prompt_prefix(
        &self,
        session_id: &str,
        computed_prefix: String,
    ) -> FrozenPromptPrefix {
        let mut prefixes = self
            .frozen_prompt_prefixes
            .lock()
            .expect("frozen prompt prefix cache poisoned");
        prefixes
            .entry(session_id.to_owned())
            .or_insert_with(|| FrozenPromptPrefix {
                system_prompt_prefix: computed_prefix,
            })
            .clone()
    }
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

/// Format an `EventRecord` as Vercel AI SDK v6 SSE data strings.
///
/// Returns zero or more raw data strings to be sent as individual SSE `data:`
/// lines. Each string is already JSON-serialised.
///
/// Mapping:
/// - `Message` / `AssistantMessageCommitted` → full lifecycle frames
///   (`start-step`, `text-start`, `text-delta`, `text-end`, `finish-step`)
/// - `TextDelta` / `AssistantTextDelta` → full lifecycle frames with own id
///   (`text-start`, `text-delta`, `text-end`) — each streaming delta gets its
///   own text part so `useChat` always has an active text part to append to
/// - Everything else → no frames (filtered out)
fn vercel_frames(event: &EventRecord) -> Vec<String> {
    let id = event.event_id.to_string();
    match &event.kind {
        EventKind::Message { content, .. }
        | EventKind::AssistantMessageCommitted { content, .. } => vec![
            json!({"type": "start-step"}).to_string(),
            json!({"type": "text-start", "id": id}).to_string(),
            json!({"type": "text-delta", "id": id, "delta": content}).to_string(),
            json!({"type": "text-end", "id": id}).to_string(),
            json!({"type": "finish-step"}).to_string(),
        ],
        EventKind::TextDelta { delta, .. } | EventKind::AssistantTextDelta { delta, .. } => {
            vec![
                json!({"type": "text-start", "id": id}).to_string(),
                json!({"type": "text-delta", "id": id, "delta": delta}).to_string(),
                json!({"type": "text-end", "id": id}).to_string(),
            ]
        }
        _ => vec![],
    }
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

    // Channel carries (sse_id, data_string) pairs.
    // For canonical format, sse_id = Some(sequence) for reconnection support.
    // For Vercel format, sse_id = None (not needed by the client).
    let (tx, rx) = mpsc::channel::<(Option<u64>, String)>(256);
    let session_filter = session_id.clone();
    let branch_filter = branch.clone();
    let session_id_str = session_id.to_string();
    // Use the caller-supplied message_id for the Vercel start frame so each
    // assistant turn in the same session gets a unique React key.
    let message_id = query.message_id.unwrap_or_else(|| session_id_str.clone());

    tokio::spawn(async move {
        // Vercel format: emit a `start` frame before any events.
        if format == StreamFormat::VercelAiSdkV6 {
            let start = json!({"type": "start", "messageId": message_id}).to_string();
            let _ = tx.send((None, start)).await;
        }

        // Replay historical events.
        for event in replay {
            let is_run_finished = matches!(event.kind, EventKind::RunFinished { .. });
            match format {
                StreamFormat::Canonical => {
                    let seq = event.sequence;
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                    let _ = tx.send((Some(seq), data)).await;
                }
                StreamFormat::VercelAiSdkV6 => {
                    for frame in vercel_frames(&event) {
                        let _ = tx.send((None, frame)).await;
                    }
                }
            }
            if format == StreamFormat::VercelAiSdkV6 && is_run_finished {
                let finish = json!({"type": "finish", "finishReason": "stop"}).to_string();
                let _ = tx.send((None, finish)).await;
                return;
            }
        }

        // Live events from the broadcast subscription.
        while let Ok(event) = subscription.recv().await {
            if event.session_id == session_filter
                && event.branch_id == branch_filter
                && (event.sequence > cursor || event.sequence == 0)
            {
                let is_run_finished = matches!(event.kind, EventKind::RunFinished { .. });
                match format {
                    StreamFormat::Canonical => {
                        let seq = event.sequence;
                        let data =
                            serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_owned());
                        let _ = tx.send((Some(seq), data)).await;
                    }
                    StreamFormat::VercelAiSdkV6 => {
                        for frame in vercel_frames(&event) {
                            let _ = tx.send((None, frame)).await;
                        }
                    }
                }
                if format == StreamFormat::VercelAiSdkV6 && is_run_finished {
                    let finish = json!({"type": "finish", "finishReason": "stop"}).to_string();
                    let _ = tx.send((None, finish)).await;
                    return;
                }
            }
        }

        // Subscription ended (runtime shutdown). Close Vercel streams cleanly.
        if format == StreamFormat::VercelAiSdkV6 {
            let finish = json!({"type": "finish", "finishReason": "stop"}).to_string();
            let _ = tx.send((None, finish)).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|(id, data)| {
        let mut evt = Event::default().data(data);
        if let Some(seq) = id {
            evt = evt.id(seq.to_string());
        }
        Ok::<Event, Infallible>(evt)
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

// ─── Skill catalog helpers ────────────────────────────────────────────────────

/// Build a tier-filtered skill catalog system prompt for per-session injection.
///
/// If `tier_tools` is `None` (pro/enterprise — no restriction), all skills are
/// included.  If `tier_tools` is `Some(allowed)`, only skills whose declared
/// `allowed_tools` are entirely within `allowed` are included.  Skills with no
/// `allowed_tools` field are excluded from restricted tiers because their tool
/// requirements are unknown.
fn build_tiered_skill_catalog(
    registry: &praxis_skills::registry::SkillRegistry,
    tier_tools: Option<&[String]>,
) -> String {
    let safe_set: Option<std::collections::HashSet<&str>> =
        tier_tools.map(|tools| tools.iter().map(String::as_str).collect());

    let mut lines = vec!["Available skills:".to_string()];
    for name in registry.skill_names() {
        let Some(skill) = registry.activate(&name) else {
            continue;
        };
        // If the tier restricts tools, only include skills whose declared
        // allowed_tools are fully within the safe set.
        if let Some(ref safe) = safe_set {
            match &skill.meta.allowed_tools {
                Some(tools) if tools.iter().all(|t| safe.contains(t.as_str())) => {}
                _ => continue,
            }
        }
        let invocable = if skill.meta.user_invocable == Some(true) {
            " [user-invocable]"
        } else {
            ""
        };
        lines.push(format!(
            "- {}: {}{}",
            skill.meta.name, skill.meta.description, invocable
        ));
    }

    if lines.len() <= 1 {
        return String::new();
    }

    let catalog = lines.join("\n");
    format!(
        "<skills>\n{catalog}\n\n\
         To activate a skill, the user types `/skill-name` as their message.\n\
         When a skill is active, follow its instructions for that interaction.\n\
         </skills>"
    )
}

// ─── BRO-219: Memory export and migration handlers ────────────────────────────

/// Query parameters for the memory export endpoint.
#[derive(Debug, Deserialize)]
struct ExportMemoryQuery {
    user_id: String,
}

/// `GET /user/memory/export?user_id=<id>`
///
/// Returns all non-expired memory events for the given user as JSONL
/// (newline-delimited JSON). Each line is a serialized `EventEnvelope`.
async fn export_memory_jsonl(
    State(state): State<CanonicalState>,
    Query(params): Query<ExportMemoryQuery>,
) -> impl IntoResponse {
    let Some(ref ftj) = state.free_tier_journal else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::http::HeaderMap::new(),
            "memory export not available".to_owned(),
        )
            .into_response();
    };

    match ftj.export_user_events(&params.user_id).await {
        Ok(events) => {
            let jsonl = events
                .iter()
                .filter_map(|e| serde_json::to_string(e).ok())
                .collect::<Vec<_>>()
                .join("\n");
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(
                axum::http::header::CONTENT_TYPE,
                axum::http::HeaderValue::from_static("application/x-ndjson"),
            );
            (StatusCode::OK, headers, jsonl).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::http::HeaderMap::new(),
            e.to_string(),
        )
            .into_response(),
    }
}

/// Request body for the migration endpoint.
#[derive(Debug, Deserialize)]
struct MigrateMemoryBody {
    user_id: String,
}

/// `POST /user/memory/migrate-to-pro`
///
/// Re-tags free-tier (`lago:namespace=shared`) events for `user_id` with
/// pro-tier metadata (`lago:namespace=pro`, 90-day TTL). The original
/// free-tier events are left intact and expire naturally after 7 days.
async fn migrate_memory_to_pro(
    State(state): State<CanonicalState>,
    Json(body): Json<MigrateMemoryBody>,
) -> impl IntoResponse {
    let Some(ref ftj) = state.free_tier_journal else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "memory migration not available" })),
        )
            .into_response();
    };

    match ftj.migrate_user_to_pro(&body.user_id).await {
        Ok(migrated) => (StatusCode::OK, Json(json!({ "migrated": migrated }))).into_response(),
        Err(e) => internal_error(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{IdentityClaims, TenantRole, Tier};

    // ─── policy_from_identity_claims tests (BRO-221 / BRO-222) ───────────────

    fn make_claims(
        tier: Tier,
        roles: Vec<TenantRole>,
        custom: Option<Vec<String>>,
    ) -> IdentityClaims {
        IdentityClaims {
            sub: "test-user".to_string(),
            tier,
            org_id: None,
            roles,
            custom_capabilities: custom,
            iat: 0,
            exp: u64::MAX,
        }
    }

    fn allow_caps(policy: &PolicySet) -> Vec<String> {
        policy
            .allow_capabilities
            .iter()
            .map(|c| c.as_str().to_string())
            .collect()
    }

    #[test]
    fn anonymous_tier_yields_empty_capabilities() {
        let claims = make_claims(Tier::Anonymous, vec![], None);
        let policy = policy_from_identity_claims(&claims);
        assert!(allow_caps(&policy).is_empty());
    }

    #[test]
    fn free_tier_yields_sandboxed_commands() {
        let claims = make_claims(Tier::Free, vec![], None);
        let policy = policy_from_identity_claims(&claims);
        let caps = allow_caps(&policy);
        assert!(!caps.is_empty());
        // Must have read-only commands
        assert!(caps.contains(&"exec:cmd:cat".to_string()));
        assert!(caps.contains(&"exec:cmd:ls".to_string()));
        // Must NOT have wildcard
        assert!(!caps.contains(&"*".to_string()));
    }

    #[test]
    fn pro_tier_yields_wildcard() {
        let claims = make_claims(Tier::Pro, vec![], None);
        let policy = policy_from_identity_claims(&claims);
        let caps = allow_caps(&policy);
        assert_eq!(caps, vec!["*".to_string()]);
    }

    #[test]
    fn enterprise_admin_role_yields_wildcard() {
        let claims = make_claims(Tier::Enterprise, vec![TenantRole::Admin], None);
        let policy = policy_from_identity_claims(&claims);
        assert!(allow_caps(&policy).contains(&"*".to_string()));
    }

    #[test]
    fn enterprise_member_role_yields_sandboxed_writes() {
        let claims = make_claims(Tier::Enterprise, vec![TenantRole::Member], None);
        let policy = policy_from_identity_claims(&claims);
        let caps = allow_caps(&policy);
        assert!(
            !caps.contains(&"*".to_string()),
            "member must not have wildcard"
        );
        assert!(
            caps.contains(&"fs:read:**".to_string()),
            "member needs fs:read"
        );
        assert!(
            caps.contains(&"fs:write:project:**".to_string()),
            "member needs project writes"
        );
        assert!(caps.contains(&"exec:cmd:ls".to_string()));
    }

    #[test]
    fn enterprise_viewer_role_is_read_only() {
        let claims = make_claims(Tier::Enterprise, vec![TenantRole::Viewer], None);
        let policy = policy_from_identity_claims(&claims);
        let caps = allow_caps(&policy);
        assert!(
            !caps.contains(&"*".to_string()),
            "viewer must not have wildcard"
        );
        assert!(
            caps.contains(&"fs:read:**".to_string()),
            "viewer needs fs:read"
        );
        // No write capabilities
        assert!(
            !caps.iter().any(|c| c.contains("write")),
            "viewer must have no write caps"
        );
    }

    #[test]
    fn enterprise_no_roles_yields_wildcard() {
        let claims = make_claims(Tier::Enterprise, vec![], None);
        let policy = policy_from_identity_claims(&claims);
        assert!(allow_caps(&policy).contains(&"*".to_string()));
    }

    #[test]
    fn custom_capabilities_override_role() {
        let custom = vec!["exec:cmd:git".to_string(), "fs:read:**".to_string()];
        // Even an Admin role is overridden by custom_capabilities
        let claims = make_claims(
            Tier::Enterprise,
            vec![TenantRole::Admin],
            Some(custom.clone()),
        );
        let policy = policy_from_identity_claims(&claims);
        let caps = allow_caps(&policy);
        assert_eq!(
            caps, custom,
            "custom_capabilities must override role-based policy"
        );
        assert!(!caps.contains(&"*".to_string()));
    }

    #[test]
    fn enterprise_agent_role_uses_custom_capabilities() {
        let custom = vec!["exec:cmd:python".to_string(), "fs:read:data/**".to_string()];
        let claims = make_claims(
            Tier::Enterprise,
            vec![TenantRole::Agent],
            Some(custom.clone()),
        );
        let policy = policy_from_identity_claims(&claims);
        // custom_capabilities wins (checked first before role)
        assert_eq!(allow_caps(&policy), custom);
    }

    #[test]
    fn enterprise_agent_role_without_custom_gets_wildcard() {
        let claims = make_claims(Tier::Enterprise, vec![TenantRole::Agent], None);
        let policy = policy_from_identity_claims(&claims);
        assert!(allow_caps(&policy).contains(&"*".to_string()));
    }

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
