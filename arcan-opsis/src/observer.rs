//! ConsciousnessObserver — ambient push from arcand to opsisd.
//!
//! Observes arcand's event stream and forwards selected events to Opsis world state.

use std::sync::Arc;

use opsis_core::clock::WorldTick;
use opsis_core::event::{EventId, EventSource, OpsisEvent, OpsisEventKind};
use opsis_core::feed::SchemaKey;
use opsis_core::state::StateDomain;

use crate::client::OpsisClient;

/// Configures which event types the observer forwards to Opsis.
#[derive(Debug, Clone)]
pub struct AmbientFilter {
    pub forward_tool_completed: bool,
    pub forward_run_finished: bool,
    pub forward_errors: bool,
}

impl Default for AmbientFilter {
    fn default() -> Self {
        Self {
            forward_tool_completed: true,
            forward_run_finished: true,
            forward_errors: true,
        }
    }
}

/// Observes arcand's consciousness stream and pushes to Opsis.
pub struct ConsciousnessObserver {
    client: Arc<OpsisClient>,
    agent_id: String,
    filter: AmbientFilter,
}

impl ConsciousnessObserver {
    pub fn new(client: Arc<OpsisClient>, agent_id: String) -> Self {
        Self {
            client,
            agent_id,
            filter: AmbientFilter::default(),
        }
    }

    pub fn with_filter(mut self, filter: AmbientFilter) -> Self {
        self.filter = filter;
        self
    }

    /// Build an OpsisEvent from an agent observation.
    pub fn build_observation(
        &self,
        insight: String,
        confidence: f32,
        domain: Option<StateDomain>,
    ) -> OpsisEvent {
        OpsisEvent {
            id: EventId::default(),
            tick: WorldTick::zero(),
            timestamp: chrono::Utc::now(),
            source: EventSource::Agent(self.agent_id.clone()),
            kind: OpsisEventKind::AgentObservation {
                insight,
                confidence,
            },
            location: None,
            domain,
            severity: Some(confidence),
            schema_key: SchemaKey::new("arcan.agent.v1"),
            tags: vec![],
        }
    }

    /// Build an OpsisEvent from an agent alert.
    pub fn build_alert(&self, message: String, severity: f32) -> OpsisEvent {
        OpsisEvent {
            id: EventId::default(),
            tick: WorldTick::zero(),
            timestamp: chrono::Utc::now(),
            source: EventSource::Agent(self.agent_id.clone()),
            kind: OpsisEventKind::AgentAlert { message },
            location: None,
            domain: None,
            severity: Some(severity),
            schema_key: SchemaKey::new("arcan.agent.v1"),
            tags: vec![],
        }
    }

    /// Push an observation event to Opsis. Fire-and-forget — logs on error.
    pub async fn push_observation(
        &self,
        insight: String,
        confidence: f32,
        domain: Option<StateDomain>,
    ) {
        let event = self.build_observation(insight, confidence, domain);
        if let Err(e) = self.client.inject(vec![event]).await {
            tracing::warn!(error = %e, "ConsciousnessObserver: failed to push observation");
        }
    }

    /// Push an alert event to Opsis. Fire-and-forget — logs on error.
    pub async fn push_alert(&self, message: String, severity: f32) {
        let event = self.build_alert(message, severity);
        if let Err(e) = self.client.inject(vec![event]).await {
            tracing::warn!(error = %e, "ConsciousnessObserver: failed to push alert");
        }
    }

    pub fn filter(&self) -> &AmbientFilter {
        &self.filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_builds_observation_event() {
        let client = OpsisClient::new("http://localhost:3010", "test-agent".into()).unwrap();
        let observer = ConsciousnessObserver::new(Arc::new(client), "test-agent".into());
        let event = observer.build_observation("test insight".into(), 0.8, None);

        assert!(matches!(
            event.kind,
            OpsisEventKind::AgentObservation { .. }
        ));
        assert_eq!(event.schema_key, SchemaKey::new("arcan.agent.v1"));
        assert!(matches!(event.source, EventSource::Agent(ref id) if id == "test-agent"));
    }

    #[test]
    fn observer_builds_alert_event() {
        let client = OpsisClient::new("http://localhost:3010", "test-agent".into()).unwrap();
        let observer = ConsciousnessObserver::new(Arc::new(client), "test-agent".into());
        let event = observer.build_alert("test alert".into(), 0.9);

        assert!(matches!(event.kind, OpsisEventKind::AgentAlert { .. }));
        assert_eq!(event.severity, Some(0.9));
        assert!(event.domain.is_none());
    }

    #[test]
    fn default_filter_forwards_all() {
        let filter = AmbientFilter::default();
        assert!(filter.forward_tool_completed);
        assert!(filter.forward_run_finished);
        assert!(filter.forward_errors);
    }
}
