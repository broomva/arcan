use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::{
    ApprovalPort, EventStorePort, KernelResult, ModelCompletion, ModelCompletionRequest,
    ModelDirective, ModelProviderPort, ModelStopReason, PolicyGatePort, PolicySet, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use aios_sandbox::LocalSandboxRunner;
use aios_tools::{ToolDispatcher, ToolRegistry};
use arcan_core::error::CoreError;
use arcan_core::runtime::{Provider, ProviderFactory, SwappableProviderHandle};
use arcand::canonical::{create_canonical_router, openapi_spec};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;
use tokio_stream::StreamExt;

/// Minimal arcan-core Provider impl for test handle.
struct StubProvider;
impl Provider for StubProvider {
    fn name(&self) -> &str {
        "test-stub"
    }
    fn complete(
        &self,
        _: &arcan_core::runtime::ProviderRequest,
    ) -> Result<arcan_core::protocol::ModelTurn, CoreError> {
        todo!("stub provider for canonical API tests")
    }
}

/// Minimal ProviderFactory for tests.
struct StubFactory;
impl ProviderFactory for StubFactory {
    fn build(&self, _spec: &str) -> Result<Arc<dyn Provider>, CoreError> {
        Ok(Arc::new(StubProvider))
    }
    fn available_providers(&self) -> Vec<String> {
        vec!["test-stub".to_string()]
    }
}

fn test_provider_handle() -> SwappableProviderHandle {
    Arc::new(std::sync::RwLock::new(
        Arc::new(StubProvider) as Arc<dyn Provider>
    ))
}

fn test_provider_factory() -> Arc<dyn ProviderFactory> {
    Arc::new(StubFactory)
}

#[derive(Debug, Default)]
struct TestProvider;

#[async_trait]
impl ModelProviderPort for TestProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        Ok(ModelCompletion {
            provider: "test".to_owned(),
            model: "test-model".to_owned(),
            directives: vec![ModelDirective::Message {
                role: "assistant".to_owned(),
                content: format!("ack: {}", request.objective),
            }],
            stop_reason: ModelStopReason::Completed,
            usage: None,
            final_answer: Some("ok".to_owned()),
        })
    }
}

fn unique_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{name}-{nanos}"))
}

fn build_runtime(root: PathBuf) -> Arc<KernelRuntime> {
    let event_store_backend = Arc::new(FileEventStore::new(root.join("kernel")));
    let journal = Arc::new(EventJournal::new(
        event_store_backend,
        EventStreamHub::new(1024),
    ));
    let event_store: Arc<dyn EventStorePort> = journal;

    let policy_engine = Arc::new(SessionPolicyEngine::new(PolicySet::default()));
    let policy_gate: Arc<dyn PolicyGatePort> = policy_engine.clone();
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ApprovalQueue::default());

    let registry = Arc::new(ToolRegistry::with_core_tools());
    let sandbox = Arc::new(LocalSandboxRunner::new(vec!["echo".to_owned()]));
    let dispatcher = Arc::new(ToolDispatcher::new(registry, policy_engine, sandbox));
    let tool_harness: Arc<dyn ToolHarnessPort> = dispatcher;

    let provider: Arc<dyn ModelProviderPort> = Arc::new(TestProvider);

    Arc::new(KernelRuntime::new(
        RuntimeConfig::new(root),
        event_store,
        provider,
        tool_harness,
        approvals,
        policy_gate,
    ))
}

#[tokio::test]
async fn canonical_session_api_round_trip() {
    let runtime = build_runtime(unique_root("arcand-canonical"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let session_response = client
        .post(format!("{base}/sessions"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(session_response.status(), StatusCode::OK);
    let session_payload: serde_json::Value = session_response.json().await.unwrap();
    let session_id = session_payload
        .get("session_id")
        .and_then(|value| value.as_str())
        .expect("session_id in response");

    let run_response = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "validate canonical route" }))
        .send()
        .await
        .unwrap();
    assert_eq!(run_response.status(), StatusCode::OK);

    let events_response = client
        .get(format!("{base}/sessions/{session_id}/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(events_response.status(), StatusCode::OK);
    let events_payload: serde_json::Value = events_response.json().await.unwrap();
    assert!(
        events_payload
            .get("events")
            .and_then(|value| value.as_array())
            .map(|events| !events.is_empty())
            .unwrap_or(false)
    );

    let create_branch_response = client
        .post(format!("{base}/sessions/{session_id}/branches"))
        .json(&json!({ "branch": "feature-api" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_branch_response.status(), StatusCode::OK);

    let list_branches_response = client
        .get(format!("{base}/sessions/{session_id}/branches"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_branches_response.status(), StatusCode::OK);
    let branches_payload: serde_json::Value = list_branches_response.json().await.unwrap();
    let branch_ids: Vec<String> = branches_payload
        .get("branches")
        .and_then(|value| value.as_array())
        .map(|branches| {
            branches
                .iter()
                .filter_map(|branch| branch.get("branch_id").and_then(|value| value.as_str()))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    assert!(branch_ids.iter().any(|branch| branch == "main"));
    assert!(branch_ids.iter().any(|branch| branch == "feature-api"));

    server.abort();
}

#[tokio::test]
async fn canonical_runs_auto_create_named_sessions() {
    let runtime = build_runtime(unique_root("arcand-canonical-auto-create"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "named-session";

    let run_response = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "auto-create this session" }))
        .send()
        .await
        .unwrap();
    assert_eq!(run_response.status(), StatusCode::OK);
    let run_payload: serde_json::Value = run_response.json().await.unwrap();
    assert_eq!(run_payload["session_id"], session_id);

    let state_response = client
        .get(format!("{base}/sessions/{session_id}/state"))
        .send()
        .await
        .unwrap();
    assert_eq!(state_response.status(), StatusCode::OK);

    server.abort();
}

#[tokio::test]
async fn canonical_stream_vercel_v6_replays_protocol_events() {
    let runtime = build_runtime(unique_root("arcand-canonical-v6-stream"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "v6-session";

    let _ = client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": session_id }))
        .send()
        .await
        .unwrap();

    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "stream replay seed" }))
        .send()
        .await
        .unwrap();

    let response = client
        .get(format!(
            "{base}/sessions/{session_id}/events/stream?branch=main&cursor=0&replay_limit=64&format=vercel_ai_sdk_v6"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-vercel-ai-ui-message-stream")
            .and_then(|value| value.to_str().ok()),
        Some("v1")
    );

    let mut stream = response.bytes_stream();
    let mut body = String::new();
    for _ in 0..12 {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .ok()
            .flatten();
        let Some(Ok(chunk)) = next else {
            continue;
        };
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.contains("\"type\":\"data-aios-event\"") {
            break;
        }
    }

    assert!(
        body.contains("\"type\":\"data-aios-event\""),
        "expected v6 event wrapper, body: {body}"
    );
    assert!(
        body.contains("\"type\":\"SessionCreated\"")
            || body.contains("\"type\":\"RunStarted\"")
            || body.contains("\"type\":\"RunFinished\""),
        "expected canonical event payload in stream, body: {body}"
    );

    server.abort();
}

// ─── stream cursor/replay invariants ────────────────────────────────────────

#[tokio::test]
async fn canonical_stream_cursor_past_head() {
    let runtime = build_runtime(unique_root("arcand-cursor-past-head"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "cursor-past-head";

    // Create session and run to generate events
    let _ = client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": session_id }))
        .send()
        .await
        .unwrap();
    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "seed events" }))
        .send()
        .await
        .unwrap();

    // Open stream with cursor far past head — should get no replay events
    let response = client
        .get(format!(
            "{base}/sessions/{session_id}/events/stream?branch=main&cursor=999999&replay_limit=64"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Try reading for a short window — no data events should arrive (only keepalive or nothing)
    let mut stream = response.bytes_stream();
    let mut body = String::new();
    for _ in 0..3 {
        let next = tokio::time::timeout(Duration::from_millis(200), stream.next())
            .await
            .ok()
            .flatten();
        let Some(Ok(chunk)) = next else {
            continue;
        };
        body.push_str(&String::from_utf8_lossy(&chunk));
    }

    // No event data should have been replayed
    assert!(
        !body.contains("\"type\":\"SessionCreated\"")
            && !body.contains("\"type\":\"RunStarted\"")
            && !body.contains("\"type\":\"Message\""),
        "cursor past head should replay no events, but got: {body}"
    );

    server.abort();
}

#[tokio::test]
async fn canonical_stream_cursor_zero_replays_all() {
    let runtime = build_runtime(unique_root("arcand-cursor-zero"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "cursor-zero";

    // Create session and run
    let _ = client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": session_id }))
        .send()
        .await
        .unwrap();
    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "generate events for replay" }))
        .send()
        .await
        .unwrap();

    // Get event count for reference
    let events_response = client
        .get(format!("{base}/sessions/{session_id}/events"))
        .send()
        .await
        .unwrap();
    let events_payload: serde_json::Value = events_response.json().await.unwrap();
    let event_count = events_payload["events"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    assert!(event_count > 0, "should have generated events");

    // Stream with cursor=0 should replay all events
    let response = client
        .get(format!(
            "{base}/sessions/{session_id}/events/stream?branch=main&cursor=0&replay_limit=64"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut stream = response.bytes_stream();
    let mut body = String::new();
    for _ in 0..20 {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .ok()
            .flatten();
        let Some(Ok(chunk)) = next else {
            continue;
        };
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.contains("\"type\":\"RunFinished\"") {
            break;
        }
    }

    // cursor=0 should have replayed SessionCreated and run lifecycle events
    assert!(
        body.contains("\"type\":\"SessionCreated\""),
        "cursor=0 should replay SessionCreated, body: {body}"
    );
    assert!(
        body.contains("\"type\":\"RunStarted\"") || body.contains("\"type\":\"RunFinished\""),
        "cursor=0 should replay run events, body: {body}"
    );

    server.abort();
}

#[tokio::test]
async fn canonical_stream_branch_isolation() {
    let runtime = build_runtime(unique_root("arcand-branch-isolation"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "branch-isolation";

    // Create session and run on main
    let _ = client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": session_id }))
        .send()
        .await
        .unwrap();
    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "main branch work" }))
        .send()
        .await
        .unwrap();

    // Create a second branch and run on it
    let _ = client
        .post(format!("{base}/sessions/{session_id}/branches"))
        .json(&json!({ "branch": "feature-b" }))
        .send()
        .await
        .unwrap();
    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "feature-b branch work", "branch": "feature-b" }))
        .send()
        .await
        .unwrap();

    // Stream main branch only
    let response = client
        .get(format!(
            "{base}/sessions/{session_id}/events/stream?branch=main&cursor=0&replay_limit=64"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let mut stream = response.bytes_stream();
    let mut body = String::new();
    for _ in 0..20 {
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .ok()
            .flatten();
        let Some(Ok(chunk)) = next else {
            continue;
        };
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.contains("\"type\":\"RunFinished\"") {
            break;
        }
    }

    // Main branch stream should contain main-branch events
    assert!(
        body.contains("main branch work") || body.contains("\"type\":\"SessionCreated\""),
        "main branch stream should contain main events, body: {body}"
    );

    // Main branch stream should NOT contain feature-b objective
    assert!(
        !body.contains("feature-b branch work"),
        "main branch stream should not contain feature-b events, body: {body}"
    );

    server.abort();
}

#[tokio::test]
async fn canonical_merge_branch_round_trip() {
    let runtime = build_runtime(unique_root("arcand-merge-branch"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session_id = "merge-test";

    // Create session and run on main
    let _ = client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": session_id }))
        .send()
        .await
        .unwrap();
    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "main work" }))
        .send()
        .await
        .unwrap();

    // Create branch and run on it
    let create_branch_resp = client
        .post(format!("{base}/sessions/{session_id}/branches"))
        .json(&json!({ "branch": "feature-merge" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_branch_resp.status(), StatusCode::OK);

    let _ = client
        .post(format!("{base}/sessions/{session_id}/runs"))
        .json(&json!({ "objective": "feature work", "branch": "feature-merge" }))
        .send()
        .await
        .unwrap();

    // Merge feature-merge into main
    let merge_response = client
        .post(format!(
            "{base}/sessions/{session_id}/branches/feature-merge/merge"
        ))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(merge_response.status(), StatusCode::OK);

    let merge_payload: serde_json::Value = merge_response.json().await.unwrap();
    assert_eq!(merge_payload["session_id"], session_id);
    assert_eq!(
        merge_payload["result"]["source_branch"], "feature-merge",
        "merge result should identify source branch"
    );
    assert_eq!(
        merge_payload["result"]["target_branch"], "main",
        "merge result should identify target branch"
    );
    assert!(
        merge_payload["result"]["source_head_sequence"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "source head sequence should be > 0"
    );
    assert!(
        merge_payload["result"]["target_head_sequence"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "target head sequence should be > 0"
    );

    server.abort();
}

// ─── OpenAPI spec tests ──────────────────────────────────────────────────────

#[test]
fn openapi_spec_contains_all_paths() {
    let spec = openapi_spec();
    let json = serde_json::to_value(&spec).expect("spec should serialize");
    let paths = json["paths"]
        .as_object()
        .expect("paths should be an object");

    let expected = [
        "/health",
        "/sessions",
        "/sessions/{session_id}/runs",
        "/sessions/{session_id}/state",
        "/sessions/{session_id}/events",
        "/sessions/{session_id}/events/stream",
        "/sessions/{session_id}/branches",
        "/sessions/{session_id}/branches/{branch_id}/merge",
        "/sessions/{session_id}/approvals/{approval_id}",
    ];
    for path in expected {
        assert!(paths.contains_key(path), "missing path: {path}");
    }
}

#[tokio::test]
async fn openapi_json_endpoint_returns_valid_spec() {
    let runtime = build_runtime(unique_root("arcand-openapi-json"));
    let router = create_canonical_router(runtime, test_provider_handle(), test_provider_factory());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let response = client
        .get(format!("{base}/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let spec: serde_json::Value = response.json().await.unwrap();
    assert_eq!(spec["info"]["title"], "Arcan Agent Runtime API");
    assert_eq!(spec["info"]["version"], "0.2.1");
    assert!(spec["paths"].as_object().is_some());
    assert!(spec["components"]["schemas"].as_object().is_some());

    server.abort();
}
