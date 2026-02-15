use crate::commands::{self, CommandResult};
use crate::r#loop::AgentLoop;
use arcan_core::aisdk::{UiStreamPart, to_ui_stream_parts};
use arcan_core::protocol::AgentEvent;
use arcan_core::runtime::{ApprovalResolver, Orchestrator};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{
        IntoResponse, Response,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
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
    let message = request.message.clone();

    tokio::spawn(async move {
        if let Err(e) = agent_loop.run(&session_id, message, tx.clone()).await {
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
