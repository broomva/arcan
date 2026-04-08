//! Knowledge lifecycle event helpers.
//!
//! Emit knowledge-related events to the Lago journal for observability,
//! Autonomic monitoring, and EGRI optimization.

use lago_core::event::{EventEnvelope, EventPayload};
use lago_core::id::*;
use serde_json::json;

/// Create an event recording that a knowledge index was built/rebuilt.
pub fn knowledge_indexed_event(
    session_id: &SessionId,
    branch_id: &BranchId,
    seq: u64,
    note_count: usize,
    health_score: f32,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        seq,
        timestamp: now_micros(),
        run_id: None,
        parent_id: None,
        metadata: Default::default(),
        schema_version: 1,
        payload: EventPayload::Custom {
            event_type: "knowledge.indexed".to_string(),
            data: json!({
                "note_count": note_count,
                "health_score": health_score,
            }),
        },
    }
}

/// Create an event recording a knowledge search query.
pub fn knowledge_searched_event(
    session_id: &SessionId,
    branch_id: &BranchId,
    seq: u64,
    query: &str,
    result_count: usize,
    top_score: f64,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        seq,
        timestamp: now_micros(),
        run_id: None,
        parent_id: None,
        metadata: Default::default(),
        schema_version: 1,
        payload: EventPayload::Custom {
            event_type: "knowledge.searched".to_string(),
            data: json!({
                "query": query,
                "result_count": result_count,
                "top_score": top_score,
            }),
        },
    }
}

fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_event_has_correct_type() {
        let sid = SessionId::new();
        let bid = BranchId::new();
        let event = knowledge_indexed_event(&sid, &bid, 1, 53, 0.95);
        match &event.payload {
            EventPayload::Custom { event_type, data } => {
                assert_eq!(event_type, "knowledge.indexed");
                assert_eq!(data["note_count"], 53);
                let health = data["health_score"].as_f64().unwrap();
                assert!((health - 0.95).abs() < 0.01, "health_score was {health}");
            }
            _ => panic!("expected Custom event"),
        }
    }

    #[test]
    fn searched_event_has_correct_type() {
        let sid = SessionId::new();
        let bid = BranchId::new();
        let event = knowledge_searched_event(&sid, &bid, 2, "lago events", 7, 8.5);
        match &event.payload {
            EventPayload::Custom { event_type, data } => {
                assert_eq!(event_type, "knowledge.searched");
                assert_eq!(data["query"], "lago events");
                assert_eq!(data["result_count"], 7);
            }
            _ => panic!("expected Custom event"),
        }
    }
}
