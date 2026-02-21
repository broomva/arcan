use arcan_core::protocol::AgentEvent;
use chrono::Utc;
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;

/// Configuration for the daemon connection
pub struct NetworkConfig {
    pub base_url: String,
    pub session_id: String,
}

pub struct NetworkClient {
    client: Client,
    config: NetworkConfig,
}

impl NetworkClient {
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            config,
        }
    }

    /// Submits a message to the agent's run endpoint
    pub async fn submit_run(&self, message: &str, branch: Option<&str>) -> anyhow::Result<()> {
        let url = format!("{}/v1/sessions/{}/runs", self.config.base_url, self.config.session_id);
        
        let body = json!({
            "message": message,
            "branch": branch,
        });

        let res = self.client.post(&url).json(&body).send().await?;
        
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit run: {}", error_text);
        }

        Ok(())
    }

    /// Submits an approval decision
    pub async fn submit_approval(&self, approval_id: &str, decision: &str, reason: Option<&str>) -> anyhow::Result<()> {
        let url = format!("{}/approvals/{}", self.config.base_url, approval_id);
        
        let body = json!({
            "decision": decision,
            "reason": reason,
        });

        let res = self.client.post(&url).json(&body).send().await?;
        
        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit approval: {}", error_text);
        }

        Ok(())
    }

    /// Continuously listens to the SSE stream and pushes parsed events to the channel
    pub async fn listen_events(&self, sender: mpsc::Sender<AgentEvent>) -> anyhow::Result<()> {
        let url = format!("{}/v1/sessions/{}/stream", self.config.base_url, self.config.session_id);
        
        let mut es = EventSource::get(url);
        
        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => {
                    tracing::info!("SSE Connection Opened");
                }
                Ok(Event::Message(message)) => {
                    // Try to parse the event payload
                    let data = message.data.trim();
                    if data == "[DONE]" || data == "{\"type\": \"done\"}" {
                        continue;
                    }

                    match serde_json::from_str::<AgentEvent>(data) {
                        Ok(agent_event) => {
                            if sender.send(agent_event).await.is_err() {
                                // Receiver dropped, break
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to deserialize AgentEvent: {} \nData: {}", e, data);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("SSE Stream error: {}", e);
                    // Slight backoff on error before reqwest-eventsource auto-reconnects
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Ok(())
    }
}
