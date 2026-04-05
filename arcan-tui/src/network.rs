use crate::client::{
    AgentClientPort, AgentStateResponse, ProviderInfo, SessionSummary,
    agent_event_from_protocol_record,
};
use aios_protocol::EventRecord as ProtocolEventRecord;
use arcan_core::protocol::AgentEvent;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Error as EventSourceError, Event, EventSource};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, mpsc};

/// Configuration for the daemon connection.
pub struct NetworkConfig {
    pub base_url: String,
    pub session_id: String,
}

/// HTTP/SSE-based implementation of `AgentClientPort`.
///
/// Connects to the Arcan daemon over HTTP for commands and SSE for streaming
/// events. The session ID is held behind an `RwLock` to support switching.
pub struct HttpAgentClient {
    client: Client,
    base_url: String,
    session_id: Arc<RwLock<String>>,
}

// ── SSE parsing helpers (private) ───────────────────────────────────────────

fn parse_protocol_record(data: &str) -> Option<AgentEvent> {
    let record: ProtocolEventRecord = serde_json::from_str(data).ok()?;
    agent_event_from_protocol_record(&record)
}

fn parse_canonical_event(event_name: &str, data: &str, session_id: &str) -> Option<AgentEvent> {
    let payload: Value = serde_json::from_str(data).ok()?;
    let run_id = "stream".to_string();
    let session_id = session_id.to_string();

    match event_name {
        "assistant.text.delta" => {
            let delta = payload.get("delta")?.as_str()?.to_string();
            Some(AgentEvent::TextDelta {
                run_id,
                session_id,
                iteration: 0,
                delta,
            })
        }
        // assistant.message.committed is redundant — RunFinished already
        // carries the final_answer. Mapping it to RunFinished caused duplicates.
        "assistant.message.committed" => None,
        "tool.started" => {
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let arguments = payload
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Some(AgentEvent::ToolCallRequested {
                run_id,
                session_id,
                iteration: 0,
                call: arcan_core::protocol::ToolCall {
                    call_id,
                    tool_name,
                    input: arguments,
                },
            })
        }
        "tool.completed" => {
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();
            if status == "ok" {
                Some(AgentEvent::ToolCallCompleted {
                    run_id,
                    session_id,
                    iteration: 0,
                    result: arcan_core::protocol::ToolResultSummary {
                        call_id,
                        tool_name,
                        output: payload.get("result").cloned().unwrap_or(Value::Null),
                    },
                })
            } else {
                let error = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("tool call failed")
                    .to_string();
                Some(AgentEvent::ToolCallFailed {
                    run_id,
                    session_id,
                    iteration: 0,
                    call_id,
                    tool_name,
                    error,
                })
            }
        }
        "intent.evaluated" => {
            if !payload
                .get("requires_approval")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            let approval_id = payload.get("approval_id")?.as_str()?.to_string();
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let arguments = payload
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let risk = payload
                .get("risk")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            Some(AgentEvent::ApprovalRequested {
                run_id,
                session_id,
                approval_id,
                call_id,
                tool_name,
                arguments,
                risk,
            })
        }
        "intent.approved" | "intent.rejected" => {
            let approval_id = payload.get("approval_id")?.as_str()?.to_string();
            let decision = payload
                .get("decision")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    if event_name == "intent.approved" {
                        "approved"
                    } else {
                        "denied"
                    }
                })
                .to_string();
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(AgentEvent::ApprovalResolved {
                run_id,
                session_id,
                approval_id,
                decision,
                reason,
            })
        }
        _ => None,
    }
}

fn parse_vercel_v6_part(data: &str) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(data).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("data-aios-event") {
        return None;
    }
    let record_value = value.get("data")?.clone();
    let record: ProtocolEventRecord = serde_json::from_value(record_value).ok()?;
    agent_event_from_protocol_record(&record)
}

/// Internal SSE listener. Continuously reads from the event stream and sends
/// parsed events to the given channel.
async fn listen_events(
    base_url: &str,
    session_id: &str,
    sender: mpsc::Sender<AgentEvent>,
) -> anyhow::Result<()> {
    // Use raw aiOS protocol format (default) so lifecycle events
    // (RunStarted, RunErrored, RunFinished) are included in the stream.
    // The Vercel v6 format only emits content-level events and drops
    // lifecycle events, causing the TUI to get stuck on "Thinking...".
    let url = format!("{base_url}/sessions/{session_id}/events/stream?branch=main&cursor=0");

    let mut es = EventSource::get(url);

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {
                tracing::info!("SSE Connection Opened");
            }
            Ok(Event::Message(message)) => {
                let event_name = message.event.as_str();
                let data = message.data.trim();
                if event_name == "done" || data == "[DONE]" || data == "{\"type\": \"done\"}" {
                    continue;
                }

                if let Some(agent_event) =
                    parse_protocol_record(data).or_else(|| parse_vercel_v6_part(data))
                {
                    if sender.send(agent_event).await.is_err() {
                        break;
                    }
                } else if let Some(agent_event) =
                    parse_canonical_event(event_name, data, session_id)
                {
                    if sender.send(agent_event).await.is_err() {
                        break;
                    }
                } else {
                    tracing::debug!("Ignored SSE event '{}': {}", event_name, data);
                }
            }
            Err(EventSourceError::StreamEnded) => {
                tracing::debug!("SSE stream ended; waiting for reconnect");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(e) => {
                tracing::warn!("SSE stream error: {}", e);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
    Ok(())
}

// ── Construction ────────────────────────────────────────────────────────────

impl HttpAgentClient {
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base_url: config.base_url,
            session_id: Arc::new(RwLock::new(config.session_id)),
        }
    }
}

// ── AgentClientPort implementation ──────────────────────────────────────────

#[async_trait]
impl AgentClientPort for HttpAgentClient {
    async fn submit_run(&self, message: &str, branch: Option<&str>) -> anyhow::Result<()> {
        let sid = self.session_id.read().await.clone();
        let url = format!("{}/sessions/{}/runs", self.base_url, sid);

        let body = json!({
            "objective": message,
            "branch": branch,
        });

        let res = self.client.post(&url).json(&body).send().await?;

        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit run: {}", error_text);
        }

        Ok(())
    }

    async fn submit_approval(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<()> {
        let sid = self.session_id.read().await.clone();
        let url = format!(
            "{}/sessions/{}/approvals/{}",
            self.base_url, sid, approval_id
        );
        let approved = matches!(
            decision.to_ascii_lowercase().as_str(),
            "approved" | "approve" | "yes" | "y" | "true"
        );
        let actor = reason.unwrap_or("tui");

        let body = json!({
            "approved": approved,
            "actor": actor,
        });

        let res = self.client.post(&url).json(&body).send().await?;

        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit approval: {}", error_text);
        }

        Ok(())
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        let url = format!("{}/sessions", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to list sessions: {}", error_text);
        }
        let sessions: Vec<SessionSummary> = res.json().await?;
        Ok(sessions)
    }

    async fn get_session_state(&self, branch: Option<&str>) -> anyhow::Result<AgentStateResponse> {
        let sid = self.session_id.read().await.clone();
        let branch_param = branch.unwrap_or("main");
        let url = format!(
            "{}/sessions/{}/state?branch={}",
            self.base_url, sid, branch_param
        );
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to get state: {}", error_text);
        }
        let state: AgentStateResponse = res.json().await?;
        Ok(state)
    }

    async fn get_model(&self) -> anyhow::Result<String> {
        let info = self.get_provider_info().await?;
        Ok(info.provider)
    }

    async fn get_provider_info(&self) -> anyhow::Result<ProviderInfo> {
        let url = format!("{}/provider", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to get provider: {}", error_text);
        }
        let info: ProviderInfo = res.json().await?;
        Ok(info)
    }

    async fn set_model(&self, provider: &str, model: Option<&str>) -> anyhow::Result<String> {
        let spec = match model {
            Some(m) => format!("{provider}:{m}"),
            None => provider.to_string(),
        };
        let url = format!("{}/provider", self.base_url);
        let body = json!({ "provider": spec });
        let res = self.client.put(&url).json(&body).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to switch provider: {}", error_text);
        }
        let resp: Value = res.json().await?;
        Ok(resp
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string())
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(256);
        let base_url = self.base_url.clone();
        let session_id = self.session_id.clone();

        tokio::spawn(async move {
            let sid = session_id.read().await.clone();
            if let Err(e) = listen_events(&base_url, &sid, tx).await {
                tracing::error!("Event listener ended: {}", e);
            }
        });

        rx
    }

    async fn get_daemon_version(&self) -> anyhow::Result<String> {
        let url = format!("{}/health", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            anyhow::bail!("Health endpoint returned {}", res.status());
        }
        let body: Value = res.json().await?;
        body.get("version")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("No version field in health response"))
    }

    fn session_id(&self) -> String {
        // Use try_read to avoid blocking; fall back to empty string (rare).
        self.session_id
            .try_read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    async fn get_autonomic(&self) -> anyhow::Result<crate::client::AutonomicInfo> {
        let url = format!("{}/autonomic", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to get autonomic: {}", error_text);
        }
        Ok(res.json().await?)
    }

    async fn get_context(&self) -> anyhow::Result<crate::client::ContextInfo> {
        let url = format!("{}/context", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to get context: {}", error_text);
        }
        Ok(res.json().await?)
    }

    async fn get_cost(&self) -> anyhow::Result<crate::client::CostInfo> {
        let url = format!("{}/cost", self.base_url);
        let res = self.client.get(&url).send().await?;
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to get cost: {}", error_text);
        }
        Ok(res.json().await?)
    }

    async fn switch_session(&self, new_id: &str) -> anyhow::Result<mpsc::Receiver<AgentEvent>> {
        // Update the session ID
        {
            let mut sid = self.session_id.write().await;
            *sid = new_id.to_string();
        }

        // Spawn a new event listener for the new session
        let (tx, rx) = mpsc::channel(256);
        let base_url = self.base_url.clone();
        let new_id = new_id.to_string();

        tokio::spawn(async move {
            if let Err(e) = listen_events(&base_url, &new_id, tx).await {
                tracing::error!("Event listener ended after session switch: {}", e);
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::{
        BranchId as ProtocolBranchId, EventKind as ProtocolEventKind,
        EventRecord as ProtocolEventRecord, SessionId as ProtocolSessionId,
    };
    use arcan_core::protocol::AgentEvent;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_assistant_delta_event() {
        let event = parse_canonical_event("assistant.text.delta", r#"{"delta":"hello"}"#, "sess-1")
            .expect("event");

        match event {
            AgentEvent::TextDelta {
                session_id, delta, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(delta, "hello");
            }
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn parses_tool_started_event() {
        let event = parse_canonical_event(
            "tool.started",
            r#"{"intent_id":"call-1","tool_name":"fs.read","arguments":{"path":"foo.txt"}}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ToolCallRequested { call, .. } => {
                assert_eq!(call.call_id, "call-1");
                assert_eq!(call.tool_name, "fs.read");
                assert_eq!(call.input, json!({"path":"foo.txt"}));
            }
            _ => panic!("expected ToolCallRequested"),
        }
    }

    #[test]
    fn parses_tool_completed_error_event() {
        let event = parse_canonical_event(
            "tool.completed",
            r#"{"intent_id":"call-2","tool_name":"shell.exec","status":"error","error":"denied"}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ToolCallFailed {
                call_id,
                tool_name,
                error,
                ..
            } => {
                assert_eq!(call_id, "call-2");
                assert_eq!(tool_name, "shell.exec");
                assert_eq!(error, "denied");
            }
            _ => panic!("expected ToolCallFailed"),
        }
    }

    #[test]
    fn parses_approval_requested_event() {
        let event = parse_canonical_event(
            "intent.evaluated",
            r#"{"intent_id":"call-3","requires_approval":true,"approval_id":"ap-1","tool_name":"shell.exec","arguments":{"cmd":"rm -rf /"},"risk":"high"}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ApprovalRequested {
                approval_id,
                call_id,
                risk,
                ..
            } => {
                assert_eq!(approval_id, "ap-1");
                assert_eq!(call_id, "call-3");
                assert_eq!(risk, "high");
            }
            _ => panic!("expected ApprovalRequested"),
        }
    }

    #[test]
    fn parses_vercel_v6_data_aios_event_part() {
        let record = ProtocolEventRecord::new(
            ProtocolSessionId::from_string("sess-1"),
            ProtocolBranchId::main(),
            7,
            ProtocolEventKind::AssistantTextDelta {
                delta: "hello v6".to_string(),
                index: Some(1),
            },
        );
        let payload = serde_json::json!({
            "type": "data-aios-event",
            "id": "7",
            "data": record,
            "transient": false
        });

        let event = parse_vercel_v6_part(&payload.to_string()).expect("event");
        match event {
            AgentEvent::TextDelta {
                session_id, delta, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(delta, "hello v6");
            }
            _ => panic!("expected TextDelta"),
        }
    }

    // --- HTTP client tests with wiremock ---

    #[tokio::test]
    async fn list_sessions_returns_parsed_sessions() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"session_id": "s1", "owner": "alice", "created_at": "2026-03-01T00:00:00Z"},
                {"session_id": "s2", "owner": "bob", "created_at": "2026-02-28T00:00:00Z"}
            ])))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let sessions = client.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "s1");
        assert_eq!(sessions[1].owner, "bob");
    }

    #[tokio::test]
    async fn list_sessions_returns_error_on_failure() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sessions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let result = client.list_sessions().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_session_state_returns_typed_response() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sessions/sess-123/state"))
            .and(query_param("branch", "main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "session_id": "sess-123",
                "branch": "main",
                "mode": "Explore",
                "state": {
                    "progress": 0.5,
                    "uncertainty": 0.2,
                    "risk_level": "Low",
                    "error_streak": 0,
                    "budget": {
                        "tokens_remaining": 100000,
                        "tool_calls_remaining": 50
                    }
                },
                "version": 42
            })))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "sess-123".to_string(),
        });

        let state = client.get_session_state(None).await.unwrap();
        assert_eq!(state.session_id, "sess-123");
        assert_eq!(state.mode, "Explore");
        assert!((state.state.progress - 0.5).abs() < f64::EPSILON);
        assert_eq!(state.version, 42);
        assert_eq!(state.state.budget.tokens_remaining, 100000);
    }

    #[tokio::test]
    async fn get_session_state_with_custom_branch() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/sessions/sess-123/state"))
            .and(query_param("branch", "feature"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "session_id": "sess-123",
                "branch": "feature",
                "mode": "Execute",
                "state": {},
                "version": 1
            })))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "sess-123".to_string(),
        });

        let state = client.get_session_state(Some("feature")).await.unwrap();
        assert_eq!(state.branch, "feature");
    }

    #[tokio::test]
    async fn submit_run_sends_correct_payload() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sessions/sess-1/runs"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "sess-1".to_string(),
        });

        client.submit_run("hello agent", None).await.unwrap();
    }

    #[tokio::test]
    async fn submit_approval_sends_correct_payload() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/sessions/sess-1/approvals/ap-42"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "sess-1".to_string(),
        });

        client
            .submit_approval("ap-42", "yes", Some("looks good"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_model_returns_provider_name() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/provider"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "provider": "anthropic",
                "available": ["anthropic", "openai", "ollama", "mock"]
            })))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let model = client.get_model().await.unwrap();
        assert_eq!(model, "anthropic");
    }

    #[tokio::test]
    async fn get_provider_info_returns_full_info() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/provider"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "provider": "anthropic",
                "available": ["anthropic", "openai", "ollama", "mock"]
            })))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let info = client.get_provider_info().await.unwrap();
        assert_eq!(info.provider, "anthropic");
        assert_eq!(
            info.available,
            vec!["anthropic", "openai", "ollama", "mock"]
        );
    }

    #[tokio::test]
    async fn set_model_sends_spec_and_returns_new_provider() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/provider"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "provider": "ollama",
                "available": ["anthropic", "openai", "ollama", "mock"]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let result = client.set_model("ollama", Some("llama3.2")).await.unwrap();
        assert_eq!(result, "ollama");
    }

    #[tokio::test]
    async fn set_model_returns_error_on_invalid_provider() {
        let server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/provider"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error": "unknown provider: \"nope\""})),
            )
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let result = client.set_model("nope", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_daemon_version_returns_version_string() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status": "ok", "version": "0.2.1"})),
            )
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let version = client.get_daemon_version().await.unwrap();
        assert_eq!(version, "0.2.1");
    }

    #[tokio::test]
    async fn get_daemon_version_errors_on_missing_field() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
            .mount(&server)
            .await;

        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "test".to_string(),
        });

        let result = client.get_daemon_version().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn switch_session_updates_session_id() {
        let server = MockServer::start().await;
        let client = HttpAgentClient::new(NetworkConfig {
            base_url: server.uri(),
            session_id: "old-session".to_string(),
        });

        assert_eq!(client.session_id(), "old-session");

        // switch_session will fail to connect SSE (no mock), but session_id should update
        let _rx = client.switch_session("new-session").await.unwrap();
        assert_eq!(client.session_id(), "new-session");
    }
}
