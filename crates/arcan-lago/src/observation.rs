use lago_core::error::LagoResult;
use lago_core::event::{EventEnvelope, EventPayload, MemoryScope};
use lago_core::projection::Projection;

/// A single observation extracted from a runtime event.
#[derive(Debug, Clone)]
pub struct Observation {
    /// Human-readable observation text.
    pub text: String,
    /// Sequence number of the source event.
    pub source_seq: u64,
    /// Timestamp of the source event (microseconds).
    pub timestamp: u64,
}

/// Extracts observations from runtime events within a given scope.
///
/// The observer watches the event stream and accumulates observations
/// until a compaction threshold is reached.
pub struct Observer {
    scope: MemoryScope,
    observations: Vec<Observation>,
    threshold: usize,
}

impl Observer {
    pub fn new(scope: MemoryScope, threshold: usize) -> Self {
        Self {
            scope,
            observations: Vec::new(),
            threshold,
        }
    }

    /// The scope this observer tracks.
    pub fn scope(&self) -> MemoryScope {
        self.scope
    }

    /// Current observation count.
    pub fn count(&self) -> usize {
        self.observations.len()
    }

    /// Whether the threshold has been reached.
    pub fn threshold_reached(&self) -> bool {
        self.observations.len() >= self.threshold
    }

    /// Drain all observations, resetting the observer.
    pub fn drain(&mut self) -> Vec<Observation> {
        std::mem::take(&mut self.observations)
    }

    /// Get observations as a slice.
    pub fn observations(&self) -> &[Observation] {
        &self.observations
    }

    /// Extract observation text from an event payload, if relevant.
    fn extract_observation(payload: &EventPayload) -> Option<String> {
        match payload {
            EventPayload::Message {
                role,
                content,
                model,
                ..
            } => {
                let model_info = model
                    .as_deref()
                    .map(|m| format!(" [{m}]"))
                    .unwrap_or_default();
                Some(format!("{role}{model_info}: {content}"))
            }
            EventPayload::MessageDelta { role, delta, .. } => {
                Some(format!("{role} delta: {delta}"))
            }
            EventPayload::ToolResult {
                tool_name, status, ..
            } => Some(format!("tool:{tool_name} → {status:?}")),
            EventPayload::StatePatched { patch, .. } => Some(format!("state patched: {patch}")),
            EventPayload::Error { error } => Some(format!("error: {error}")),
            EventPayload::RunFinished {
                reason,
                total_iterations,
                ..
            } => Some(format!(
                "run finished: {reason} ({total_iterations} iterations)"
            )),
            // Ignore lifecycle, sandbox, policy, branch, snapshot, and memory events
            _ => None,
        }
    }
}

impl Projection for Observer {
    fn on_event(&mut self, event: &EventEnvelope) -> LagoResult<()> {
        if let Some(text) = Self::extract_observation(&event.payload) {
            self.observations.push(Observation {
                text,
                source_seq: event.seq,
                timestamp: event.timestamp,
            });
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "arcan::observer"
    }
}

/// Text-based compaction of observations.
///
/// In M1.2, this uses simple text concatenation with a summary header.
/// Future versions will use LLM-based summarization.
pub struct Reflector;

impl Reflector {
    /// Compact observations into a summary string.
    pub fn compact(observations: &[Observation]) -> String {
        if observations.is_empty() {
            return String::new();
        }

        let mut lines = Vec::with_capacity(observations.len() + 1);
        lines.push(format!(
            "Summary of {} observations (seq {}..{}):",
            observations.len(),
            observations.first().map(|o| o.source_seq).unwrap_or(0),
            observations.last().map(|o| o.source_seq).unwrap_or(0),
        ));

        for obs in observations {
            lines.push(format!("- {}", obs.text));
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::event::*;
    use lago_core::id::*;
    use std::collections::HashMap;

    fn make_envelope(seq: u64, payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::from_string("EVT001"),
            session_id: SessionId::from_string("SESS001"),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq,
            timestamp: 1_700_000_000_000_000 + seq,
            parent_id: None,
            payload,
            metadata: HashMap::new(),
            schema_version: 1,
        }
    }

    #[test]
    fn observer_extracts_from_message() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        let env = make_envelope(
            1,
            EventPayload::Message {
                role: "user".to_string(),
                content: "Hello world".to_string(),
                model: None,
                token_usage: None,
            },
        );
        obs.on_event(&env).unwrap();
        assert_eq!(obs.count(), 1);
        assert!(obs.observations()[0].text.contains("Hello world"));
        assert_eq!(obs.observations()[0].source_seq, 1);
    }

    #[test]
    fn observer_extracts_from_tool_result() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        let env = make_envelope(
            2,
            EventPayload::ToolResult {
                call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
                result: serde_json::json!({"content": "data"}),
                duration_ms: 42,
                status: SpanStatus::Ok,
            },
        );
        obs.on_event(&env).unwrap();
        assert_eq!(obs.count(), 1);
        assert!(obs.observations()[0].text.contains("read_file"));
    }

    #[test]
    fn observer_extracts_from_error() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        let env = make_envelope(
            3,
            EventPayload::Error {
                error: "connection refused".to_string(),
            },
        );
        obs.on_event(&env).unwrap();
        assert_eq!(obs.count(), 1);
        assert!(obs.observations()[0].text.contains("connection refused"));
    }

    #[test]
    fn observer_extracts_from_state_patched() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        let env = make_envelope(
            4,
            EventPayload::StatePatched {
                index: 1,
                patch: serde_json::json!({"cwd": "/home"}),
                revision: 1,
            },
        );
        obs.on_event(&env).unwrap();
        assert_eq!(obs.count(), 1);
        assert!(obs.observations()[0].text.contains("state patched"));
    }

    #[test]
    fn observer_ignores_lifecycle_events() {
        let mut obs = Observer::new(MemoryScope::Session, 10);

        // StepStarted — should be ignored
        obs.on_event(&make_envelope(1, EventPayload::StepStarted { index: 1 }))
            .unwrap();

        // StepFinished — should be ignored
        obs.on_event(&make_envelope(
            2,
            EventPayload::StepFinished {
                index: 1,
                stop_reason: "end_turn".to_string(),
                directive_count: 1,
            },
        ))
        .unwrap();

        // BranchCreated — should be ignored
        obs.on_event(&make_envelope(
            3,
            EventPayload::BranchCreated {
                new_branch_id: BranchId::from_string("feature"),
                fork_point_seq: 0,
                name: "feature".to_string(),
            },
        ))
        .unwrap();

        // SessionCreated — should be ignored
        obs.on_event(&make_envelope(
            4,
            EventPayload::SessionCreated {
                name: "test".to_string(),
                config: serde_json::json!({}),
            },
        ))
        .unwrap();

        assert_eq!(obs.count(), 0);
    }

    #[test]
    fn observer_tracks_source_seq() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        obs.on_event(&make_envelope(
            42,
            EventPayload::Error {
                error: "boom".to_string(),
            },
        ))
        .unwrap();
        assert_eq!(obs.observations()[0].source_seq, 42);
    }

    #[test]
    fn threshold_triggers() {
        let mut obs = Observer::new(MemoryScope::Session, 3);
        for i in 1..=3 {
            obs.on_event(&make_envelope(
                i,
                EventPayload::Error {
                    error: format!("err-{i}"),
                },
            ))
            .unwrap();
        }
        assert!(obs.threshold_reached());
    }

    #[test]
    fn drain_clears_observations() {
        let mut obs = Observer::new(MemoryScope::Session, 10);
        obs.on_event(&make_envelope(
            1,
            EventPayload::Error {
                error: "boom".to_string(),
            },
        ))
        .unwrap();
        assert_eq!(obs.count(), 1);

        let drained = obs.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(obs.count(), 0);
        assert!(!obs.threshold_reached());
    }

    #[test]
    fn reflector_compact_works() {
        let observations = vec![
            Observation {
                text: "user: hello".to_string(),
                source_seq: 1,
                timestamp: 100,
            },
            Observation {
                text: "tool:read_file → Ok".to_string(),
                source_seq: 2,
                timestamp: 200,
            },
            Observation {
                text: "error: boom".to_string(),
                source_seq: 3,
                timestamp: 300,
            },
        ];
        let summary = Reflector::compact(&observations);
        assert!(summary.contains("Summary of 3 observations"));
        assert!(summary.contains("seq 1..3"));
        assert!(summary.contains("- user: hello"));
        assert!(summary.contains("- error: boom"));
    }

    #[test]
    fn reflector_compact_empty() {
        let summary = Reflector::compact(&[]);
        assert!(summary.is_empty());
    }
}
