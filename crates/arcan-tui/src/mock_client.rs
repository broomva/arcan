use crate::client::{AgentClientPort, AgentStateFields, AgentStateResponse, SessionSummary};
use arcan_core::protocol::AgentEvent;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

/// A configurable mock implementation of `AgentClientPort` for testing.
///
/// Pre-loaded events are drained in order on `subscribe_events()`.
/// `submit_run` / `submit_approval` outcomes are configurable via closures
/// or default to `Ok(())`.
pub struct MockAgentClient {
    pub session_id: String,
    pub base_url: String,
    /// Events to emit when `subscribe_events()` is called.
    events: Arc<Mutex<Vec<AgentEvent>>>,
    /// If set, `submit_run` returns this error message.
    pub submit_run_error: Arc<Mutex<Option<String>>>,
    /// If set, `submit_approval` returns this error message.
    pub submit_approval_error: Arc<Mutex<Option<String>>>,
    /// Tracks calls to `submit_run` for assertion.
    pub submitted_messages: Arc<Mutex<Vec<String>>>,
}

impl MockAgentClient {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            base_url: "http://localhost:3000".to_string(),
            events: Arc::new(Mutex::new(Vec::new())),
            submit_run_error: Arc::new(Mutex::new(None)),
            submit_approval_error: Arc::new(Mutex::new(None)),
            submitted_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Pre-load events that will be emitted on the next `subscribe_events()`.
    pub async fn set_events(&self, events: Vec<AgentEvent>) {
        *self.events.lock().await = events;
    }

    /// Configure `submit_run` to fail with the given error message.
    pub async fn fail_submit_run(&self, error: &str) {
        *self.submit_run_error.lock().await = Some(error.to_string());
    }
}

#[async_trait]
impl AgentClientPort for MockAgentClient {
    async fn submit_run(&self, message: &str, _branch: Option<&str>) -> anyhow::Result<()> {
        self.submitted_messages
            .lock()
            .await
            .push(message.to_string());

        if let Some(ref err) = *self.submit_run_error.lock().await {
            anyhow::bail!("{}", err);
        }
        Ok(())
    }

    async fn submit_approval(
        &self,
        _approval_id: &str,
        _decision: &str,
        _reason: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(ref err) = *self.submit_approval_error.lock().await {
            anyhow::bail!("{}", err);
        }
        Ok(())
    }

    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>> {
        Ok(vec![SessionSummary {
            session_id: self.session_id.clone(),
            owner: "test".to_string(),
            created_at: Some("2026-03-01T00:00:00Z".to_string()),
        }])
    }

    async fn get_session_state(&self, _branch: Option<&str>) -> anyhow::Result<AgentStateResponse> {
        Ok(AgentStateResponse {
            session_id: self.session_id.clone(),
            branch: "main".to_string(),
            mode: "Explore".to_string(),
            state: AgentStateFields::default(),
            version: 1,
        })
    }

    async fn get_model(&self) -> anyhow::Result<String> {
        Ok("mock".to_string())
    }

    async fn set_model(&self, provider: &str, model: Option<&str>) -> anyhow::Result<String> {
        let result = match model {
            Some(m) => format!("{provider}:{m}"),
            None => provider.to_string(),
        };
        Ok(result)
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(256);
        let events = self.events.clone();

        tokio::spawn(async move {
            let event_list: Vec<AgentEvent> = {
                let mut guard = events.lock().await;
                std::mem::take(&mut *guard)
            };
            for event in event_list {
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            // Drop tx → channel closes → event_pump emits ConnectionLost
        });

        rx
    }

    fn session_id(&self) -> String {
        self.session_id.clone()
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    async fn switch_session(&self, new_id: &str) -> anyhow::Result<mpsc::Receiver<AgentEvent>> {
        let _ = new_id;
        Ok(self.subscribe_events())
    }
}
