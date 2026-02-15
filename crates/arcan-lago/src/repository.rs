use crate::event_map;
use arcan_core::protocol::AgentEvent;
use arcan_store::session::{AppendEvent, EventRecord, SessionRepository, StoreError};
use chrono::{DateTime, Utc};
use lago_core::{BranchId, EventQuery, Journal, SeqNo, SessionId};
use std::sync::Arc;

/// A [`SessionRepository`] implementation backed by a Lago [`Journal`].
///
/// This bridges Arcan's synchronous `SessionRepository` trait with Lago's
/// async `Journal` trait. The sync/async boundary is crossed using
/// `Handle::current().block_on()`, which is safe because the Arcan
/// `AgentLoop` runs the orchestrator inside `tokio::task::spawn_blocking`.
pub struct LagoSessionRepository {
    journal: Arc<dyn Journal>,
    default_branch: BranchId,
}

impl LagoSessionRepository {
    pub fn new(journal: Arc<dyn Journal>) -> Self {
        Self {
            journal,
            default_branch: BranchId::from("main"),
        }
    }

    /// Run an async future on the current tokio runtime from a sync context.
    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }

    fn session_id(&self, id: &str) -> SessionId {
        SessionId::from(id.to_string())
    }
}

impl SessionRepository for LagoSessionRepository {
    fn append(&self, request: AppendEvent) -> Result<EventRecord, StoreError> {
        let session_id = self.session_id(&request.session_id);
        let branch_id = self.default_branch.clone();

        // Get the next sequence number.
        let head_seq = self
            .block_on(self.journal.head_seq(&session_id, &branch_id))
            .map_err(|e| StoreError::Io {
                path: "lago-journal".into(),
                source: std::io::Error::other(e.to_string()),
            })?;
        let seq: SeqNo = head_seq + 1;

        // Extract run_id from the event for the envelope.
        let run_id = extract_run_id(&request.event);
        let arcan_event_id = uuid::Uuid::new_v4().to_string();

        let envelope = event_map::arcan_to_lago(
            &session_id,
            &branch_id,
            seq,
            &run_id,
            &request.event,
            &arcan_event_id,
        );

        let timestamp = DateTime::<Utc>::from_timestamp(
            (envelope.timestamp / 1_000_000) as i64,
            ((envelope.timestamp % 1_000_000) * 1_000) as u32,
        )
        .unwrap_or_else(Utc::now);

        self.block_on(self.journal.append(envelope))
            .map_err(|e| StoreError::Io {
                path: "lago-journal".into(),
                source: std::io::Error::other(e.to_string()),
            })?;

        Ok(EventRecord {
            id: arcan_event_id,
            session_id: request.session_id,
            parent_id: request.parent_id,
            timestamp,
            event: request.event,
        })
    }

    fn load_session(&self, session_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        let sid = self.session_id(session_id);
        let query = EventQuery::new()
            .session(sid.clone())
            .branch(self.default_branch.clone());

        let envelopes = self
            .block_on(self.journal.read(query))
            .map_err(|e| StoreError::Io {
                path: "lago-journal".into(),
                source: std::io::Error::other(e.to_string()),
            })?;

        let mut records = Vec::with_capacity(envelopes.len());
        for envelope in &envelopes {
            if let Some(agent_event) = event_map::lago_to_arcan(envelope) {
                let arcan_id = envelope
                    .metadata
                    .get("arcan_event_id")
                    .cloned()
                    .unwrap_or_else(|| envelope.event_id.to_string());

                let timestamp = DateTime::<Utc>::from_timestamp(
                    (envelope.timestamp / 1_000_000) as i64,
                    ((envelope.timestamp % 1_000_000) * 1_000) as u32,
                )
                .unwrap_or_else(Utc::now);

                records.push(EventRecord {
                    id: arcan_id,
                    session_id: session_id.to_string(),
                    parent_id: envelope.parent_id.as_ref().map(ToString::to_string),
                    timestamp,
                    event: agent_event,
                });
            }
        }

        Ok(records)
    }

    fn load_children(&self, parent_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        // Lago's journal doesn't index by parent_id natively.
        // For now, we scan the default branch and filter in memory.
        // This is acceptable because branching/child lookups are rare in the agent loop.
        let query = EventQuery::new().branch(self.default_branch.clone());

        let envelopes = self
            .block_on(self.journal.read(query))
            .map_err(|e| StoreError::Io {
                path: "lago-journal".into(),
                source: std::io::Error::other(e.to_string()),
            })?;

        let mut results = Vec::new();
        for envelope in &envelopes {
            let is_child = envelope
                .parent_id
                .as_ref()
                .is_some_and(|pid| pid.to_string() == parent_id);

            if is_child {
                if let Some(agent_event) = event_map::lago_to_arcan(envelope) {
                    let arcan_id = envelope
                        .metadata
                        .get("arcan_event_id")
                        .cloned()
                        .unwrap_or_else(|| envelope.event_id.to_string());

                    let timestamp = DateTime::<Utc>::from_timestamp(
                        (envelope.timestamp / 1_000_000) as i64,
                        ((envelope.timestamp % 1_000_000) * 1_000) as u32,
                    )
                    .unwrap_or_else(Utc::now);

                    results.push(EventRecord {
                        id: arcan_id,
                        session_id: envelope.session_id.to_string(),
                        parent_id: Some(parent_id.to_string()),
                        timestamp,
                        event: agent_event,
                    });
                }
            }
        }

        Ok(results)
    }

    fn head(&self, session_id: &str) -> Result<Option<EventRecord>, StoreError> {
        let records = self.load_session(session_id)?;
        Ok(records.into_iter().last())
    }
}

fn extract_run_id(event: &AgentEvent) -> String {
    match event {
        AgentEvent::RunStarted { run_id, .. }
        | AgentEvent::IterationStarted { run_id, .. }
        | AgentEvent::ModelOutput { run_id, .. }
        | AgentEvent::TextDelta { run_id, .. }
        | AgentEvent::ToolCallRequested { run_id, .. }
        | AgentEvent::ToolCallCompleted { run_id, .. }
        | AgentEvent::ToolCallFailed { run_id, .. }
        | AgentEvent::StatePatched { run_id, .. }
        | AgentEvent::ContextCompacted { run_id, .. }
        | AgentEvent::ApprovalRequested { run_id, .. }
        | AgentEvent::ApprovalResolved { run_id, .. }
        | AgentEvent::RunErrored { run_id, .. }
        | AgentEvent::RunFinished { run_id, .. } => run_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
    use lago_journal::RedbJournal;

    fn make_repo() -> Arc<LagoSessionRepository> {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.redb");
        // Leak the tempdir so the file survives the test.
        std::mem::forget(dir);
        let journal = RedbJournal::open(db_path).unwrap();
        Arc::new(LagoSessionRepository::new(Arc::new(journal)))
    }

    fn make_event(run_id: &str, session_id: &str) -> AgentEvent {
        AgentEvent::RunFinished {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("done".to_string()),
        }
    }

    #[tokio::test]
    async fn append_and_load_session() {
        let repo = make_repo();

        let record = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || {
                repo.append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: make_event("r1", "s1"),
                })
            }
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(record.session_id, "s1");

        let records = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || repo.load_session("s1")
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn head_returns_last_event() {
        let repo = make_repo();

        tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || {
                repo.append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: make_event("r1", "s1"),
                })
                .unwrap();
                repo.append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: make_event("r2", "s1"),
                })
                .unwrap();
            }
        })
        .await
        .unwrap();

        let head = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || repo.head("s1")
        })
        .await
        .unwrap()
        .unwrap();

        assert!(head.is_some());
    }

    #[tokio::test]
    async fn empty_session_returns_empty() {
        let repo = make_repo();

        let records = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || repo.load_session("nonexistent")
        })
        .await
        .unwrap()
        .unwrap();

        assert!(records.is_empty());

        let head = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || repo.head("nonexistent")
        })
        .await
        .unwrap()
        .unwrap();

        assert!(head.is_none());
    }

    #[tokio::test]
    async fn tool_events_round_trip_through_journal() {
        let repo = make_repo();

        tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || {
                repo.append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: AgentEvent::ToolCallRequested {
                        run_id: "r1".into(),
                        session_id: "s1".into(),
                        iteration: 1,
                        call: ToolCall {
                            call_id: "c1".into(),
                            tool_name: "read_file".into(),
                            input: serde_json::json!({"path": "test.txt"}),
                        },
                    },
                })
                .unwrap();

                repo.append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: AgentEvent::ToolCallCompleted {
                        run_id: "r1".into(),
                        session_id: "s1".into(),
                        iteration: 1,
                        result: ToolResultSummary {
                            call_id: "c1".into(),
                            tool_name: "read_file".into(),
                            output: serde_json::json!({"content": "file data"}),
                        },
                    },
                })
                .unwrap();
            }
        })
        .await
        .unwrap();

        let records = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || repo.load_session("s1")
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(records.len(), 2);
        assert!(matches!(
            records[0].event,
            AgentEvent::ToolCallRequested { .. }
        ));
        assert!(matches!(
            records[1].event,
            AgentEvent::ToolCallCompleted { .. }
        ));
    }
}
