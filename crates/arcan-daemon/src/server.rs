use crate::r#loop::AgentLoop;
use arcan_core::protocol::AgentEvent;
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::cors::CorsLayer;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub session_id: String,
    pub message: String,
}

pub struct ServerState {
    pub agent_loop: Arc<AgentLoop>,
}

pub async fn create_router(agent_loop: Arc<AgentLoop>) -> Router {
    let state = Arc::new(ServerState { agent_loop });

    Router::new()
        .route("/chat", post(chat_handler))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn chat_handler(
    State(state): State<Arc<ServerState>>,
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

    let stream = ReceiverStream::new(rx).map(|event| match serde_json::to_string(&event) {
        Ok(json) => Ok(Event::default().data(json)),
        Err(e) => Ok(Event::default().data(format!(r#"{{"error": "{}"}}"#, e))),
    });

    Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}
