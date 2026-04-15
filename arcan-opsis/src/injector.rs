//! WorldStateInjector — subscribes to opsisd SSE, filters high-severity
//! world events for injection into arcand as ExternalSignals.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::RwLock;
use url::Url;

use opsis_core::event::{OpsisEvent, OpsisEventKind, WorldDelta};

/// Filter thresholds for which world events get injected into the agent.
#[derive(Debug, Clone)]
pub struct InjectorThresholds {
    pub min_severity: f32,
    pub gaia_anomaly_min_sigma: f32,
    pub gaia_correlation_min_confidence: f32,
    pub debounce_ticks: u64,
    pub critical_severity: f32,
}

impl Default for InjectorThresholds {
    fn default() -> Self {
        Self {
            min_severity: 0.7,
            gaia_anomaly_min_sigma: 3.0,
            gaia_correlation_min_confidence: 0.8,
            debounce_ticks: 10,
            critical_severity: 0.9,
        }
    }
}

/// Maintains a local world state snapshot and filters high-severity events.
pub struct WorldStateInjector {
    _opsisd_url: Url,
    thresholds: InjectorThresholds,
    last_signal_tick: Arc<AtomicU64>,
    /// Local world state snapshot for Praxis tool queries.
    snapshot: Arc<RwLock<Option<WorldDelta>>>,
}

impl WorldStateInjector {
    pub fn new(opsisd_url: &str) -> Result<Self, url::ParseError> {
        Ok(Self {
            _opsisd_url: Url::parse(opsisd_url)?,
            thresholds: InjectorThresholds::default(),
            last_signal_tick: Arc::new(AtomicU64::new(0)),
            snapshot: Arc::new(RwLock::new(None)),
        })
    }

    pub fn with_thresholds(mut self, thresholds: InjectorThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Get the shared snapshot handle (used by opsis_world_state Praxis tool).
    pub fn snapshot_handle(&self) -> Arc<RwLock<Option<WorldDelta>>> {
        self.snapshot.clone()
    }

    /// Spawn a background task that subscribes to the opsisd SSE stream
    /// and continuously updates the local snapshot.
    pub fn spawn_sse_loop(self: Arc<Self>, opsisd_url: &str) {
        let url = format!("{}/stream", opsisd_url.trim_end_matches('/'));
        tokio::spawn(async move {
            tracing::info!(%url, "opsis injector: starting SSE subscription loop");
            let client = reqwest::Client::new();
            loop {
                match client.get(&url).send().await {
                    Ok(resp) => {
                        use futures_util::StreamExt;
                        let mut stream = resp.bytes_stream();
                        let mut buffer = String::new();

                        while let Some(chunk) = stream.next().await {
                            match chunk {
                                Ok(bytes) => {
                                    buffer.push_str(&String::from_utf8_lossy(&bytes));
                                    // Process complete SSE events (double newline delimited).
                                    while let Some(pos) = buffer.find("\n\n") {
                                        let event_text = buffer[..pos].to_string();
                                        buffer = buffer[pos + 2..].to_string();

                                        // Parse SSE: "event: world_delta\ndata: {...}"
                                        if let Some(data_line) =
                                            event_text.lines().find(|l| l.starts_with("data: "))
                                        {
                                            let json_str = &data_line[6..];
                                            if let Ok(delta) =
                                                serde_json::from_str::<WorldDelta>(json_str)
                                            {
                                                self.process_delta(&delta).await;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "opsis injector: SSE read error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "opsis injector: SSE connect failed");
                    }
                }
                // Reconnect after 5 seconds.
                tracing::debug!("opsis injector: reconnecting in 5s...");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    /// Process a WorldDelta — update snapshot and return signal-worthy events.
    pub async fn process_delta(&self, delta: &WorldDelta) -> Vec<OpsisEvent> {
        // Update snapshot.
        {
            let mut snap = self.snapshot.write().await;
            *snap = Some(delta.clone());
        }

        let current_tick = delta.tick.0;
        let last_tick = self.last_signal_tick.load(Ordering::Relaxed);

        let mut signal_events = Vec::new();

        // Check state line deltas.
        for sld in &delta.state_line_deltas {
            for event in &sld.new_events {
                if self.should_signal(event, current_tick, last_tick) {
                    signal_events.push(event.clone());
                }
            }
        }

        // Check Gaia insights.
        for event in &delta.gaia_insights {
            if self.should_signal(event, current_tick, last_tick) {
                signal_events.push(event.clone());
            }
        }

        if !signal_events.is_empty() {
            self.last_signal_tick.store(current_tick, Ordering::Relaxed);
        }

        signal_events
    }

    fn should_signal(&self, event: &OpsisEvent, current_tick: u64, last_tick: u64) -> bool {
        let severity = event.severity.unwrap_or(0.0);

        // Critical events always pass.
        if severity >= self.thresholds.critical_severity {
            return true;
        }

        // Debounce check.
        if current_tick.saturating_sub(last_tick) < self.thresholds.debounce_ticks {
            return false;
        }

        // Severity threshold.
        if severity >= self.thresholds.min_severity {
            return true;
        }

        // Gaia-specific thresholds.
        match &event.kind {
            OpsisEventKind::GaiaAnomaly { sigma, .. } => {
                *sigma >= self.thresholds.gaia_anomaly_min_sigma
            }
            OpsisEventKind::GaiaCorrelation { confidence, .. } => {
                *confidence >= self.thresholds.gaia_correlation_min_confidence
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsis_core::clock::WorldTick;
    use opsis_core::event::{EventId, EventSource, StateLineDelta};
    use opsis_core::feed::{FeedSource, SchemaKey};
    use opsis_core::state::StateDomain;

    fn make_event(severity: f32) -> OpsisEvent {
        OpsisEvent {
            id: EventId::default(),
            tick: WorldTick(100),
            timestamp: chrono::Utc::now(),
            source: EventSource::Feed(FeedSource::new("test")),
            kind: OpsisEventKind::WorldObservation {
                summary: "test".into(),
            },
            location: None,
            domain: Some(StateDomain::Emergency),
            severity: Some(severity),
            schema_key: SchemaKey::new("test.v1"),
            tags: vec![],
        }
    }

    fn make_delta(tick: u64, events: Vec<OpsisEvent>) -> WorldDelta {
        WorldDelta {
            tick: WorldTick(tick),
            timestamp: chrono::Utc::now(),
            state_line_deltas: vec![StateLineDelta {
                domain: StateDomain::Emergency,
                activity: 0.8,
                trend: opsis_core::state::Trend::Spike,
                new_events: events,
                hotspots: vec![],
            }],
            gaia_insights: vec![],
            unrouted_events: vec![],
        }
    }

    #[tokio::test]
    async fn high_severity_triggers_signal() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();
        let delta = make_delta(100, vec![make_event(0.85)]);
        let signals = injector.process_delta(&delta).await;
        assert_eq!(signals.len(), 1);
    }

    #[tokio::test]
    async fn low_severity_no_signal() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();
        let delta = make_delta(100, vec![make_event(0.3)]);
        let signals = injector.process_delta(&delta).await;
        assert!(signals.is_empty());
    }

    #[tokio::test]
    async fn critical_bypasses_debounce() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();

        // First delta at tick 100 — triggers (severity 0.8 >= 0.7).
        let delta1 = make_delta(100, vec![make_event(0.8)]);
        let signals1 = injector.process_delta(&delta1).await;
        assert_eq!(signals1.len(), 1);

        // Second delta at tick 102 (within debounce window of 10).
        // Non-critical (0.8) would be debounced, but critical (0.95) passes.
        let delta2 = make_delta(102, vec![make_event(0.95)]);
        let signals2 = injector.process_delta(&delta2).await;
        assert_eq!(signals2.len(), 1);
    }

    #[tokio::test]
    async fn debounce_blocks_non_critical() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();

        // First signal at tick 100.
        let delta1 = make_delta(100, vec![make_event(0.8)]);
        assert_eq!(injector.process_delta(&delta1).await.len(), 1);

        // Within debounce window, non-critical blocked.
        let delta2 = make_delta(105, vec![make_event(0.8)]);
        assert!(injector.process_delta(&delta2).await.is_empty());

        // After debounce window, passes again.
        let delta3 = make_delta(115, vec![make_event(0.8)]);
        assert_eq!(injector.process_delta(&delta3).await.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_updated_on_delta() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();
        let snapshot = injector.snapshot_handle();

        assert!(snapshot.read().await.is_none());

        let delta = WorldDelta {
            tick: WorldTick(42),
            timestamp: chrono::Utc::now(),
            state_line_deltas: vec![],
            gaia_insights: vec![],
            unrouted_events: vec![],
        };
        injector.process_delta(&delta).await;

        let snap = snapshot.read().await;
        assert_eq!(snap.as_ref().unwrap().tick, WorldTick(42));
    }

    #[tokio::test]
    async fn gaia_anomaly_triggers_on_sigma() {
        let injector = WorldStateInjector::new("http://localhost:3010").unwrap();

        let mut gaia_event = make_event(0.3); // low severity
        gaia_event.kind = OpsisEventKind::GaiaAnomaly {
            domain: StateDomain::Finance,
            sigma: 3.5,
            description: "extreme outlier".into(),
        };

        let delta = WorldDelta {
            tick: WorldTick(100),
            timestamp: chrono::Utc::now(),
            state_line_deltas: vec![],
            gaia_insights: vec![gaia_event],
            unrouted_events: vec![],
        };

        let signals = injector.process_delta(&delta).await;
        assert_eq!(signals.len(), 1);
    }
}
