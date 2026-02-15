use arcan_core::error::CoreError;
use arcan_core::protocol::{ModelDirective, ModelStopReason, ModelTurn};
use arcan_core::runtime::{
    Orchestrator, OrchestratorConfig, Provider, ProviderRequest, ToolRegistry,
};
use arcan_store::session::InMemorySessionRepository;
use arcand::r#loop::AgentLoop;
use arcand::server::{create_router, create_router_with_approvals};
use std::sync::Arc;

struct EchoProvider;

impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let last = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        Ok(ModelTurn {
            directives: vec![ModelDirective::Text {
                delta: format!("Echo: {last}"),
            }],
            stop_reason: ModelStopReason::EndTurn,
            usage: None,
        })
    }
}

async fn start_test_server() -> String {
    let repo = Arc::new(InMemorySessionRepository::default());
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(EchoProvider),
        ToolRegistry::default(),
        Vec::new(),
        OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    let agent_loop = Arc::new(AgentLoop::new(repo, orchestrator));
    let router = create_router(agent_loop).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // Brief yield to let server start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    url
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{url}/health"))
        .send()
        .await
        .expect("health request failed");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn chat_endpoint_returns_sse_stream() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat"))
        .json(&serde_json::json!({
            "session_id": "sse-test",
            "message": "Hello SSE"
        }))
        .send()
        .await
        .expect("chat request failed");

    assert_eq!(resp.status(), 200);

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/event-stream"),
        "Expected SSE content type, got: {content_type}"
    );

    // Read the full response body (SSE stream)
    let body = resp.text().await.unwrap();

    // SSE format: "data: {...}\n\n"
    assert!(
        body.contains("data:"),
        "Response should contain SSE data frames"
    );

    // Should contain agent event data (RunStarted, TextDelta, RunFinished)
    // Events are serialized as JSON with serde tags
    assert!(
        body.contains("run_id") && body.contains("session_id"),
        "SSE stream should contain agent event data with run_id/session_id, got: {}",
        &body[..body.len().min(500)]
    );

    // Should contain echo of our message
    assert!(
        body.contains("Echo: Hello SSE"),
        "SSE stream should contain echoed message, got: {}",
        &body[..body.len().min(500)]
    );
}

#[tokio::test]
async fn chat_with_aisdk_format() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v5"))
        .json(&serde_json::json!({
            "session_id": "aisdk-test",
            "message": "Hello AI SDK"
        }))
        .send()
        .await
        .expect("chat request failed");

    assert_eq!(resp.status(), 200);

    let body = resp.text().await.unwrap();

    // AI SDK format should have text-delta parts
    assert!(
        body.contains("text-delta") || body.contains("text_delta"),
        "AI SDK format should contain text-delta parts, got: {body}"
    );
}

#[tokio::test]
async fn concurrent_sessions_are_isolated() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    // Send two concurrent requests to different sessions
    let (resp1, resp2) = tokio::join!(
        client
            .post(format!("{url}/chat"))
            .json(&serde_json::json!({
                "session_id": "session-a",
                "message": "Message A"
            }))
            .send(),
        client
            .post(format!("{url}/chat"))
            .json(&serde_json::json!({
                "session_id": "session-b",
                "message": "Message B"
            }))
            .send(),
    );

    let body1 = resp1.unwrap().text().await.unwrap();
    let body2 = resp2.unwrap().text().await.unwrap();

    // Session A should echo "Message A"
    assert!(
        body1.contains("Message A"),
        "Session A should contain its own message"
    );

    // Session B should echo "Message B"
    assert!(
        body2.contains("Message B"),
        "Session B should contain its own message"
    );
}

#[tokio::test]
async fn invalid_request_returns_error() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    // Send malformed JSON
    let resp = client
        .post(format!("{url}/chat"))
        .header("content-type", "application/json")
        .body("{invalid json}")
        .send()
        .await
        .expect("request failed");

    // Should return 4xx (bad request or unprocessable)
    assert!(
        resp.status().is_client_error(),
        "Malformed JSON should return client error, got: {}",
        resp.status()
    );
}

// --- Approval endpoint tests ---

use arcan_core::runtime::ApprovalResolver;

/// Mock approval resolver for testing the HTTP endpoints.
struct MockApprovalResolver {
    pending: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl MockApprovalResolver {
    fn new() -> Self {
        Self {
            pending: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn add_pending(&self, id: &str) {
        self.pending
            .lock()
            .unwrap()
            .insert(id.to_string(), "pending".to_string());
    }
}

impl ApprovalResolver for MockApprovalResolver {
    fn resolve_approval(&self, approval_id: &str, decision: &str, _reason: Option<String>) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if pending.remove(approval_id).is_some() {
            let _ = decision; // consume
            true
        } else {
            false
        }
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        self.pending.lock().unwrap().keys().cloned().collect()
    }
}

async fn start_test_server_with_resolver(resolver: Arc<dyn ApprovalResolver>) -> String {
    let repo = Arc::new(InMemorySessionRepository::default());
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(EchoProvider),
        ToolRegistry::default(),
        Vec::new(),
        OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    let agent_loop = Arc::new(AgentLoop::new(repo, orchestrator));
    let router = create_router_with_approvals(agent_loop, Some(resolver)).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    url
}

#[tokio::test]
async fn approve_resolves_pending() {
    let resolver = Arc::new(MockApprovalResolver::new());
    resolver.add_pending("appr-1");

    let url = start_test_server_with_resolver(resolver).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/approve"))
        .json(&serde_json::json!({
            "approval_id": "appr-1",
            "decision": "approved"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["resolved"], true);
}

#[tokio::test]
async fn approve_unknown_returns_not_found() {
    let resolver = Arc::new(MockApprovalResolver::new());
    let url = start_test_server_with_resolver(resolver).await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/approve"))
        .json(&serde_json::json!({
            "approval_id": "nonexistent",
            "decision": "approved"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn list_approvals_returns_pending() {
    let resolver = Arc::new(MockApprovalResolver::new());
    resolver.add_pending("appr-a");
    resolver.add_pending("appr-b");

    let url = start_test_server_with_resolver(resolver).await;
    let client = reqwest::Client::new();

    let resp = client.get(format!("{url}/approvals")).send().await.unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let pending = body["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
}

// --- AI SDK v6 protocol tests ---

#[tokio::test]
async fn sse_v6_includes_protocol_header() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v6"))
        .json(&serde_json::json!({
            "session_id": "v6-header-test",
            "message": "Hello v6"
        }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);

    let header = resp
        .headers()
        .get("x-vercel-ai-ui-message-stream")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        header,
        Some("v1"),
        "Should include x-vercel-ai-ui-message-stream: v1 header"
    );
}

#[tokio::test]
async fn sse_v6_events_have_monotonic_ids() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v6"))
        .json(&serde_json::json!({
            "session_id": "v6-id-test",
            "message": "Hello IDs"
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();

    // Parse SSE id: fields — they should be monotonically increasing
    let ids: Vec<u64> = body
        .lines()
        .filter(|line| line.starts_with("id:"))
        .filter_map(|line| line[3..].trim().parse().ok())
        .collect();

    assert!(
        !ids.is_empty(),
        "v6 stream should have SSE event IDs, body: {}",
        &body[..body.len().min(500)]
    );

    // Verify monotonic
    for window in ids.windows(2) {
        assert!(
            window[1] > window[0],
            "IDs should be monotonically increasing: {} > {}",
            window[1],
            window[0]
        );
    }
}

#[tokio::test]
async fn sse_v6_stream_terminates_with_done() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v6"))
        .json(&serde_json::json!({
            "session_id": "v6-done-test",
            "message": "Hello DONE"
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();

    assert!(
        body.contains("[DONE]"),
        "v6 stream should terminate with [DONE], body tail: {}",
        &body[body.len().saturating_sub(200)..]
    );
}

#[tokio::test]
async fn sse_v6_has_text_boundaries() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v6"))
        .json(&serde_json::json!({
            "session_id": "v6-boundary-test",
            "message": "Hello boundaries"
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();

    // Should have text-start before text-delta and text-end after
    assert!(
        body.contains("text-start"),
        "v6 stream should contain text-start boundary, body: {}",
        &body[..body.len().min(500)]
    );
    assert!(
        body.contains("text-end"),
        "v6 stream should contain text-end boundary"
    );
    assert!(
        body.contains("text-delta"),
        "v6 stream should contain text-delta"
    );
}

#[tokio::test]
async fn sse_v6_has_step_boundaries() {
    let url = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{url}/chat?format=aisdk_v6"))
        .json(&serde_json::json!({
            "session_id": "v6-step-test",
            "message": "Hello steps"
        }))
        .send()
        .await
        .unwrap();

    let body = resp.text().await.unwrap();

    assert!(
        body.contains("start-step"),
        "v6 stream should contain start-step"
    );
    assert!(
        body.contains("finish-step"),
        "v6 stream should contain finish-step"
    );
}
