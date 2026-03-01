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
use arcand::canonical::create_canonical_router;
use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;
use tokio_stream::StreamExt;

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
    let router = create_canonical_router(runtime);

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
    let router = create_canonical_router(runtime);

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
    let router = create_canonical_router(runtime);

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
