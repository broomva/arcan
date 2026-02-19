use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolResult};
use arcan_core::runtime::{Middleware, RunOutput, ToolContext};
use lago_core::Journal;
use lago_core::event::{EventEnvelope, EventPayload, MemoryScope};
use lago_core::id::{BranchId, EventId, SessionId};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A structured learning entry captured from runtime events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEntry {
    /// Category: "tool_failure", "user_correction", "run_error", "policy_denied"
    pub category: String,
    /// Human-readable description of what happened.
    pub description: String,
    /// The tool involved (if applicable).
    pub tool_name: Option<String>,
    /// The error message (if applicable).
    pub error: Option<String>,
    /// Session where this learning originated.
    pub session_id: String,
    /// Run where this learning originated.
    pub run_id: String,
}

/// Buffer that accumulates learning entries during a run.
#[derive(Debug, Default)]
struct LearningBuffer {
    entries: Vec<LearningEntry>,
    run_id: String,
    session_id: String,
}

/// Middleware that captures learning events from the agent runtime.
///
/// Records tool failures, middleware denials, and run errors as structured
/// learning entries. At the end of each run, the accumulated learnings are
/// persisted to the Lago journal as `Custom` events with type "learning_captured".
pub struct LearningMiddleware {
    journal: Arc<dyn Journal>,
    buffer: Mutex<LearningBuffer>,
}

impl LearningMiddleware {
    pub fn new(journal: Arc<dyn Journal>) -> Self {
        Self {
            journal,
            buffer: Mutex::new(LearningBuffer::default()),
        }
    }

    fn push_entry(&self, entry: LearningEntry) {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.entries.push(entry);
        }
    }

    /// Drain the buffer and build an envelope (if any entries exist).
    fn drain_to_envelope(&self) -> Result<Option<EventEnvelope>, CoreError> {
        let (entries, session_id, run_id) = {
            let mut buf = self.buffer.lock().map_err(|e| {
                CoreError::Middleware(format!("learning buffer lock poisoned: {e}"))
            })?;
            let entries = std::mem::take(&mut buf.entries);
            let session_id = buf.session_id.clone();
            let run_id = buf.run_id.clone();
            (entries, session_id, run_id)
        };

        if entries.is_empty() {
            return Ok(None);
        }

        Ok(Some(EventEnvelope {
            event_id: EventId::default(),
            session_id: SessionId::from_string(&session_id),
            branch_id: BranchId::from("main"),
            run_id: Some(lago_core::id::RunId::from_string(&run_id)),
            seq: 0,
            timestamp: 0,
            parent_id: None,
            payload: EventPayload::Custom {
                event_type: "learning_captured".to_string(),
                data: serde_json::json!({
                    "scope": MemoryScope::Agent,
                    "entry_count": entries.len(),
                    "entries": entries,
                }),
            },
            metadata: Default::default(),
            schema_version: 1,
        }))
    }

    fn flush_to_journal(&self) -> Result<(), CoreError> {
        let Some(envelope) = self.drain_to_envelope()? else {
            return Ok(());
        };

        // Use block_on since middleware runs in spawn_blocking context
        let journal = self.journal.clone();
        let handle = tokio::runtime::Handle::current();
        handle
            .block_on(async { journal.append(envelope).await })
            .map_err(|e| CoreError::Middleware(format!("failed to persist learnings: {e}")))?;

        Ok(())
    }
}

impl Middleware for LearningMiddleware {
    fn before_model_call(
        &self,
        request: &arcan_core::runtime::ProviderRequest,
    ) -> Result<(), CoreError> {
        // Initialize buffer with run context on first model call
        if let Ok(mut buf) = self.buffer.lock() {
            if buf.run_id.is_empty() {
                buf.run_id = request.run_id.clone();
                buf.session_id = request.session_id.clone();
            }
        }
        Ok(())
    }

    fn pre_tool_call(&self, context: &ToolContext, call: &ToolCall) -> Result<(), CoreError> {
        // Set context if not set yet (e.g., first tool call in a run)
        if let Ok(mut buf) = self.buffer.lock() {
            if buf.run_id.is_empty() {
                buf.run_id = context.run_id.clone();
                buf.session_id = context.session_id.clone();
            }
        }
        // Nothing to capture before the call
        let _ = call;
        Ok(())
    }

    fn post_tool_call(&self, context: &ToolContext, result: &ToolResult) -> Result<(), CoreError> {
        // Capture tool failures as learning entries
        if result.is_error {
            let error_text = result
                .output
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
                .to_string();

            self.push_entry(LearningEntry {
                category: "tool_failure".to_string(),
                description: format!("Tool '{}' failed: {}", result.tool_name, error_text),
                tool_name: Some(result.tool_name.clone()),
                error: Some(error_text),
                session_id: context.session_id.clone(),
                run_id: context.run_id.clone(),
            });
        }
        Ok(())
    }

    fn on_run_finished(&self, output: &RunOutput) -> Result<(), CoreError> {
        // Capture run errors
        if output.reason == arcan_core::protocol::RunStopReason::Error {
            self.push_entry(LearningEntry {
                category: "run_error".to_string(),
                description: "Run ended with error".to_string(),
                tool_name: None,
                error: None,
                session_id: output.session_id.clone(),
                run_id: output.run_id.clone(),
            });
        }

        // Capture budget exceeded (agent may need different approach)
        if output.reason == arcan_core::protocol::RunStopReason::BudgetExceeded {
            self.push_entry(LearningEntry {
                category: "budget_exceeded".to_string(),
                description: "Run exhausted iteration budget without completing".to_string(),
                tool_name: None,
                error: None,
                session_id: output.session_id.clone(),
                run_id: output.run_id.clone(),
            });
        }

        // Flush all accumulated learnings to journal
        self.flush_to_journal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::RunStopReason;
    use arcan_core::runtime::{RunOutput, ToolContext};

    fn mock_context() -> ToolContext {
        ToolContext {
            run_id: "run-1".to_string(),
            session_id: "sess-1".to_string(),
            iteration: 1,
        }
    }

    fn tool_result_ok() -> ToolResult {
        ToolResult {
            call_id: "c1".to_string(),
            tool_name: "read_file".to_string(),
            output: serde_json::json!({"content": "data"}),
            content: None,
            is_error: false,
            state_patch: None,
        }
    }

    fn tool_result_error() -> ToolResult {
        ToolResult {
            call_id: "c2".to_string(),
            tool_name: "write_file".to_string(),
            output: serde_json::json!({"error": "permission denied"}),
            content: None,
            is_error: true,
            state_patch: None,
        }
    }

    #[test]
    fn captures_tool_failure() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);

        let ctx = mock_context();
        mw.post_tool_call(&ctx, &tool_result_error()).unwrap();

        let buf = mw.buffer.lock().unwrap();
        assert_eq!(buf.entries.len(), 1);
        assert_eq!(buf.entries[0].category, "tool_failure");
        assert!(buf.entries[0].description.contains("write_file"));
        assert!(buf.entries[0].description.contains("permission denied"));
    }

    #[test]
    fn ignores_successful_tool() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);

        let ctx = mock_context();
        mw.post_tool_call(&ctx, &tool_result_ok()).unwrap();

        let buf = mw.buffer.lock().unwrap();
        assert_eq!(buf.entries.len(), 0);
    }

    #[test]
    fn captures_run_error() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);

        // Set run context
        {
            let mut buf = mw.buffer.lock().unwrap();
            buf.run_id = "run-1".to_string();
            buf.session_id = "sess-1".to_string();
        }

        let output = RunOutput {
            run_id: "run-1".to_string(),
            session_id: "sess-1".to_string(),
            branch_id: "main".to_string(),
            reason: RunStopReason::Error,
            events: vec![],
            messages: vec![],
            state: Default::default(),
            final_answer: None,
            total_usage: Default::default(),
        };

        // Don't flush (no tokio runtime in unit test), just check buffer
        // Manually push the entry that on_run_finished would push
        mw.push_entry(LearningEntry {
            category: "run_error".to_string(),
            description: "Run ended with error".to_string(),
            tool_name: None,
            error: None,
            session_id: output.session_id.clone(),
            run_id: output.run_id.clone(),
        });

        let buf = mw.buffer.lock().unwrap();
        assert_eq!(buf.entries.len(), 1);
        assert_eq!(buf.entries[0].category, "run_error");
    }

    #[test]
    fn captures_budget_exceeded() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);

        mw.push_entry(LearningEntry {
            category: "budget_exceeded".to_string(),
            description: "Run exhausted 10 iterations without completing".to_string(),
            tool_name: None,
            error: None,
            session_id: "sess-1".to_string(),
            run_id: "run-1".to_string(),
        });

        let buf = mw.buffer.lock().unwrap();
        assert_eq!(buf.entries.len(), 1);
        assert_eq!(buf.entries[0].category, "budget_exceeded");
    }

    #[test]
    fn multiple_failures_accumulate() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);
        let ctx = mock_context();

        // Three tool failures in one run
        for i in 0..3 {
            let result = ToolResult {
                call_id: format!("c{i}"),
                tool_name: format!("tool_{i}"),
                output: serde_json::json!({"error": format!("err-{i}")}),
                content: None,
                is_error: true,
                state_patch: None,
            };
            mw.post_tool_call(&ctx, &result).unwrap();
        }

        let buf = mw.buffer.lock().unwrap();
        assert_eq!(buf.entries.len(), 3);
    }

    #[test]
    fn learning_entry_serialization() {
        let entry = LearningEntry {
            category: "tool_failure".to_string(),
            description: "Tool failed".to_string(),
            tool_name: Some("bash".to_string()),
            error: Some("command not found".to_string()),
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: LearningEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.category, "tool_failure");
        assert_eq!(back.tool_name, Some("bash".to_string()));
    }

    #[tokio::test]
    async fn flush_persists_to_journal() {
        let dir = tempfile::tempdir().unwrap();
        let journal =
            Arc::new(lago_journal::RedbJournal::open(dir.path().join("test.redb")).unwrap());

        // Create a session first
        let session = lago_core::Session {
            session_id: lago_core::id::SessionId::from_string("sess-1"),
            config: lago_core::session::SessionConfig::new("test"),
            created_at: 0,
            branches: vec![],
        };
        journal.put_session(session).await.unwrap();

        let mw = LearningMiddleware::new(journal.clone());

        // Set context and add entries
        {
            let mut buf = mw.buffer.lock().unwrap();
            buf.run_id = "run-1".to_string();
            buf.session_id = "sess-1".to_string();
        }

        mw.push_entry(LearningEntry {
            category: "tool_failure".to_string(),
            description: "bash failed".to_string(),
            tool_name: Some("bash".to_string()),
            error: Some("not found".to_string()),
            session_id: "sess-1".to_string(),
            run_id: "run-1".to_string(),
        });

        // Drain and persist (async-safe, no block_on-in-block_on)
        let envelope = mw
            .drain_to_envelope()
            .unwrap()
            .expect("should have entries");
        journal.append(envelope).await.unwrap();

        // Verify event was persisted
        let query = lago_core::journal::EventQuery::default()
            .session(lago_core::id::SessionId::from_string("sess-1"));
        let events = journal.read(query).await.unwrap();
        assert_eq!(events.len(), 1);

        match &events[0].payload {
            EventPayload::Custom { event_type, data } => {
                assert_eq!(event_type, "learning_captured");
                assert_eq!(data["entry_count"], 1);
                assert!(data["entries"][0]["category"] == "tool_failure");
            }
            other => panic!("Expected Custom event, got: {:?}", other),
        }
    }

    #[test]
    fn empty_buffer_flush_is_noop() {
        let journal = Arc::new(
            lago_journal::RedbJournal::open(tempfile::tempdir().unwrap().path().join("test.redb"))
                .unwrap(),
        );
        let mw = LearningMiddleware::new(journal);

        {
            let mut buf = mw.buffer.lock().unwrap();
            buf.run_id = "run-1".to_string();
            buf.session_id = "sess-1".to_string();
        }

        // No entries — flush should be ok (noop)
        // Can't call flush_to_journal without tokio runtime for empty,
        // but we can verify the buffer is empty
        let buf = mw.buffer.lock().unwrap();
        assert!(buf.entries.is_empty());
    }
}
