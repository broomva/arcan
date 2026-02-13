use crate::r#loop::AgentLoop;
use anyhow::Result;
use arcan_core::protocol::AgentEvent;
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::post,
    Json, Router,
};
use futures::stream::Stream;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
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
        // Run the loop
        if let Err(e) = agent_loop.run(&session_id, message, tx.clone()).await {
             // If loop fails, try to send an error event
             let _ = tx.send(AgentEvent::RunErrored {
                 run_id: "unknown".to_string(), // we might not have run_id if it failed early
                 session_id: session_id,
                 error: e.to_string(),
             }).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = event.as_sse_data().unwrap_or_else(|e| format!("data: error visualizing event: {}\n\n", e));
        // Remove "data: " prefix and "\n\n" suffix as Event::default().data() adds them? 
        // No, axum::response::sse::Event::default().data() takes string data and formats it.
        // But `as_sse_data` returns the full "data: ...\n\n" string.
        // We should just use raw data or parse it?
        // AgentEvent::as_sse_data returns "data: {json}\n\n".
        
        // Let's rely on axum's Event builder.
        // We should serialize event to json.
        match serde_json::to_string(&event) {
            Ok(json) => Ok(Event::default().data(json)),
            Err(e) => Ok(Event::default().data(format!(r#"{{"error": "{}"}}"#, e))),
        }
    });

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
}
