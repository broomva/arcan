//! HTTP client for pushing events to opsisd.

use opsis_core::clock::WorldTick;
use opsis_core::event::{EventId, EventSource, OpsisEvent, OpsisEventKind};
use opsis_core::feed::SchemaKey;
use opsis_core::spatial::GeoPoint;
use opsis_core::state::StateDomain;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum OpsisClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
}

pub type OpsisClientResult<T> = Result<T, OpsisClientError>;

#[derive(Debug, Serialize)]
struct InjectRequest {
    events: Vec<OpsisEvent>,
}

#[derive(Debug, Deserialize)]
pub struct InjectResponse {
    pub accepted: usize,
    pub warnings: Vec<String>,
}

/// Thin async HTTP client for opsisd.
#[derive(Clone)]
pub struct OpsisClient {
    base_url: Url,
    http: reqwest::Client,
    agent_id: String,
}

impl OpsisClient {
    pub fn new(base_url: &str, agent_id: String) -> OpsisClientResult<Self> {
        let base_url = Url::parse(base_url)?;
        Ok(Self {
            base_url,
            http: reqwest::Client::new(),
            agent_id,
        })
    }

    /// Inject events into opsisd.
    pub async fn inject(&self, events: Vec<OpsisEvent>) -> OpsisClientResult<InjectResponse> {
        let url = self.base_url.join("/events/inject")?;
        let resp = self
            .http
            .post(url)
            .json(&InjectRequest { events })
            .send()
            .await?
            .json::<InjectResponse>()
            .await?;
        Ok(resp)
    }

    /// Publish an agent observation.
    pub async fn observe(
        &self,
        insight: String,
        confidence: f32,
        domain: StateDomain,
        location: Option<GeoPoint>,
    ) -> OpsisClientResult<()> {
        let event = OpsisEvent {
            id: EventId::default(),
            tick: WorldTick::zero(),
            timestamp: chrono::Utc::now(),
            source: EventSource::Agent(self.agent_id.clone()),
            kind: OpsisEventKind::AgentObservation {
                insight,
                confidence,
            },
            location,
            domain: Some(domain),
            severity: Some(confidence),
            schema_key: SchemaKey::new("arcan.agent.v1"),
            tags: vec![],
        };
        self.inject(vec![event]).await?;
        Ok(())
    }

    /// Publish an agent alert.
    pub async fn alert(
        &self,
        message: String,
        domain: StateDomain,
        severity: f32,
    ) -> OpsisClientResult<()> {
        let event = OpsisEvent {
            id: EventId::default(),
            tick: WorldTick::zero(),
            timestamp: chrono::Utc::now(),
            source: EventSource::Agent(self.agent_id.clone()),
            kind: OpsisEventKind::AgentAlert { message },
            location: None,
            domain: Some(domain),
            severity: Some(severity),
            schema_key: SchemaKey::new("arcan.agent.v1"),
            tags: vec![],
        };
        self.inject(vec![event]).await?;
        Ok(())
    }

    /// Health check — returns true if opsisd is reachable.
    pub async fn health(&self) -> bool {
        let Ok(url) = self.base_url.join("/health") else {
            return false;
        };
        self.http.get(url).send().await.is_ok()
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation() {
        let client = OpsisClient::new("http://localhost:3010", "test-agent".into());
        assert!(client.is_ok());
    }

    #[test]
    fn invalid_url_returns_error() {
        let client = OpsisClient::new("not a url", "test".into());
        assert!(client.is_err());
    }

    #[test]
    fn agent_id_stored() {
        let client = OpsisClient::new("http://localhost:3010", "my-agent".into()).unwrap();
        assert_eq!(client.agent_id(), "my-agent");
    }
}
