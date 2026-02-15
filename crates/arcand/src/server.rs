use crate::r#loop::AgentLoop;
use arcan_core::aisdk::to_aisdk_parts;
use arcan_core::protocol::AgentEvent;
use arcan_core::runtime::ApprovalResolver;
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
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
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
    /// SSE format: "arcan" (default) or "aisdk_v5"
    #[serde(default)]
    pub format: Option<String>,
}

pub(crate) struct ServerState {
    pub(crate) agent_loop: Arc<AgentLoop>,
    pub(crate) approval_resolver: Option<Arc<dyn ApprovalResolver>>,
}

pub async fn create_router(agent_loop: Arc<AgentLoop>) -> Router {
    create_router_with_approvals(agent_loop, None).await
}

pub async fn create_router_with_approvals(
    agent_loop: Arc<AgentLoop>,
    approval_resolver: Option<Arc<dyn ApprovalResolver>>,
) -> Router {
    let state = Arc::new(ServerState {
        agent_loop,
        approval_resolver,
    });

    Router::new()
        .route("/health", get(health_handler))
        .route("/chat", post(chat_handler))
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
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
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

    let use_aisdk = query.format.as_deref() == Some("aisdk_v5");

    // Bridge: convert each AgentEvent into one or more SSE Events
    let (event_tx, event_rx) = mpsc::channel::<Result<Event, Infallible>>(200);

    tokio::spawn(async move {
        let mut stream = ReceiverStream::new(rx);
        while let Some(event) = stream.next().await {
            if use_aisdk {
                for part in to_aisdk_parts(&event) {
                    let sse = match serde_json::to_string(&part) {
                        Ok(json) => Ok(Event::default().data(json)),
                        Err(e) => Ok(Event::default().data(format!(r#"{{"error": "{}"}}"#, e))),
                    };
                    if event_tx.send(sse).await.is_err() {
                        return;
                    }
                }
            } else {
                let sse = match serde_json::to_string(&event) {
                    Ok(json) => Ok(Event::default().data(json)),
                    Err(e) => Ok(Event::default().data(format!(r#"{{"error": "{}"}}"#, e))),
                };
                if event_tx.send(sse).await.is_err() {
                    return;
                }
            }
        }
    });

    let out_stream = ReceiverStream::new(event_rx);

    Sse::new(out_stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
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
    // Validate decision
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
