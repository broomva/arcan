//! Cross-session event search tool for the Arcan runtime.
//!
//! Builds a BM25 full-text index over Lago journal events and exposes
//! it as a canonical [`Tool`] that agents can invoke to search their
//! own history across sessions.

use std::sync::{Arc, RwLock};
use std::time::Instant;

use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use lago_core::journal::Journal;
use lago_knowledge::event_index::{self, EventSearchIndex};
use serde_json::json;

fn tool_err(msg: impl Into<String>) -> CoreError {
    CoreError::ToolExecution {
        tool_name: "knowledge_search".to_string(),
        message: msg.into(),
    }
}

/// Agent tool that searches across session events using BM25 full-text ranking.
///
/// The index is built lazily from the journal on first invocation and cached
/// for subsequent calls. Rebuild can be forced by passing `"reindex": true`.
pub struct EventSearchTool {
    journal: Arc<dyn Journal>,
    index: Arc<RwLock<Option<EventSearchIndex>>>,
    /// Session to exclude from indexing (typically the current session).
    exclude_session: Option<String>,
}

impl EventSearchTool {
    /// Create a new event search tool backed by a Lago journal.
    ///
    /// `exclude_session` prevents indexing the current session's events
    /// (those are already in the conversation context).
    pub fn new(journal: Arc<dyn Journal>, exclude_session: Option<String>) -> Self {
        Self {
            journal,
            index: Arc::new(RwLock::new(None)),
            exclude_session,
        }
    }

    /// Build (or rebuild) the search index from journal events.
    ///
    /// Reads all sessions, extracts searchable text from events,
    /// and builds a BM25 index. Returns the number of indexed entries.
    async fn build_index(&self) -> Result<usize, CoreError> {
        let t0 = Instant::now();

        // Read events from all sessions
        let sessions = self
            .journal
            .list_sessions()
            .await
            .map_err(|e| tool_err(format!("failed to list sessions: {e}")))?;

        let mut entries = Vec::new();

        for session in &sessions {
            let sid = session.session_id.to_string();

            // Skip current session
            if self.exclude_session.as_deref() == Some(&sid) {
                continue;
            }

            let query = lago_core::journal::EventQuery::new()
                .session(session.session_id.clone())
                .branch(lago_core::id::BranchId::from_string("main"));

            let events = self
                .journal
                .read(query)
                .await
                .map_err(|e| tool_err(format!("failed to read session {sid}: {e}")))?;

            for env in &events {
                let event_kind_name = event_kind_label(&env.payload);
                let payload_json = serde_json::to_value(&env.payload).unwrap_or_default();

                if let Some(entry) = event_index::extract_searchable_text(
                    env.event_id.as_ref(),
                    &sid,
                    env.timestamp,
                    &event_kind_name,
                    &payload_json,
                ) {
                    entries.push(entry);
                }
            }
        }

        let count = entries.len();
        let idx = EventSearchIndex::build(entries);

        // Cache the index
        if let Ok(mut guard) = self.index.write() {
            *guard = Some(idx);
        }

        tracing::info!(
            entries = count,
            sessions = sessions.len(),
            duration_ms = t0.elapsed().as_millis(),
            "event search index built"
        );

        Ok(count)
    }
}

impl Tool for EventSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "knowledge_search".to_string(),
            description: "Search across all past session events (messages, tool results, \
                          decisions, errors) using full-text BM25 ranking. Use this to \
                          find relevant context from previous sessions."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query — keywords, concepts, or questions to find in session history"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 10)"
                    },
                    "reindex": {
                        "type": "boolean",
                        "description": "Force rebuild of the search index (default: false)"
                    }
                },
                "required": ["query"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("knowledge".to_string()),
            tags: vec![
                "knowledge".to_string(),
                "search".to_string(),
                "cross-session".to_string(),
            ],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let query = call
            .input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_err("missing 'query' parameter"))?;

        let max_results = call
            .input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;

        let force_reindex = call
            .input
            .get("reindex")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let t0 = Instant::now();

        // Build index if needed
        let needs_build = force_reindex
            || self
                .index
                .read()
                .map(|guard| guard.is_none())
                .unwrap_or(true);

        if needs_build {
            // Run async build in blocking context
            let this_journal = self.journal.clone();
            let this_exclude = self.exclude_session.clone();
            let this_index = self.index.clone();

            // We need a runtime handle to run the async build
            let handle = tokio::runtime::Handle::try_current()
                .map_err(|_| tool_err("no tokio runtime available"))?;

            let built = handle.block_on(async {
                let tool = EventSearchTool {
                    journal: this_journal,
                    index: this_index,
                    exclude_session: this_exclude,
                };
                tool.build_index().await
            })?;

            tracing::debug!(entries = built, "event search index ready");
        }

        // Search
        let guard = self
            .index
            .read()
            .map_err(|_| tool_err("index lock poisoned"))?;

        let idx = guard
            .as_ref()
            .ok_or_else(|| tool_err("index not available"))?;

        let results = idx.search(query, max_results);
        let duration_ms = t0.elapsed().as_millis();

        // Format output
        if results.is_empty() {
            return Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: json!({
                    "query": query,
                    "results": "No matching events found across past sessions.",
                    "indexed_events": idx.len(),
                    "duration_ms": duration_ms,
                }),
                content: None,
                is_error: false,
                state_patch: None,
            });
        }

        let mut output_lines = Vec::new();
        for (i, r) in results.iter().enumerate() {
            output_lines.push(format!(
                "{}. [{}] session:{} kind:{} score:{:.2}\n   {}",
                i + 1,
                format_timestamp(r.timestamp),
                r.session_id,
                r.event_kind,
                r.score,
                r.excerpt,
            ));
        }

        // Record Vigil span attributes
        tracing::info!(
            query = query,
            results_count = results.len(),
            indexed_events = idx.len(),
            duration_ms = duration_ms as u64,
            "knowledge_search completed"
        );

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "query": query,
                "results": output_lines.join("\n\n"),
                "result_count": results.len(),
                "indexed_events": idx.len(),
                "duration_ms": duration_ms,
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Extract a human-readable label from an EventKind variant.
fn event_kind_label(kind: &aios_protocol::event::EventKind) -> String {
    use aios_protocol::event::EventKind;
    match kind {
        EventKind::Message { .. } => "Message".to_string(),
        EventKind::UserMessage { .. } => "UserMessage".to_string(),
        EventKind::ToolCallRequested { .. } => "ToolCallRequested".to_string(),
        EventKind::ToolCallCompleted { .. } => "ToolCallCompleted".to_string(),
        EventKind::ToolCallFailed { .. } => "ToolCallFailed".to_string(),
        EventKind::ErrorRaised { .. } => "ErrorRaised".to_string(),
        EventKind::Custom { event_type, .. } => format!("Custom:{event_type}"),
        other => format!("{other:?}")
            .split_whitespace()
            .next()
            .unwrap_or("Unknown")
            .to_string(),
    }
}

/// Format a microsecond timestamp as a compact ISO-like string.
fn format_timestamp(us: u64) -> String {
    let secs = us / 1_000_000;
    let nanos = ((us % 1_000_000) * 1000) as u32;
    let dt = chrono::DateTime::from_timestamp(secs as i64, nanos).unwrap_or_default();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::event::EventKind;
    use lago_core::event::EventEnvelope;
    use lago_core::id::{BranchId, EventId, SessionId};
    use lago_core::session::{Session, SessionConfig};
    use lago_journal::RedbJournal;
    use std::collections::HashMap;

    fn open_journal(dir: &std::path::Path) -> Arc<dyn Journal> {
        let db_path = dir.join("test.redb");
        Arc::new(RedbJournal::open(db_path).unwrap()) as Arc<dyn Journal>
    }

    fn make_envelope(session_id: &str, payload: EventKind) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: SessionId::from_string(session_id),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq: 0,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload,
            metadata: HashMap::new(),
            schema_version: 1,
        }
    }

    fn make_session(id: &str) -> Session {
        Session {
            session_id: SessionId::from_string(id),
            config: SessionConfig {
                name: format!("test-{id}"),
                model: "mock".into(),
                params: HashMap::new(),
            },
            created_at: EventEnvelope::now_micros(),
            branches: vec![BranchId::from_string("main")],
        }
    }

    #[tokio::test]
    async fn event_search_tool_definition() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());
        let tool = EventSearchTool::new(journal, None);
        let def = tool.definition();
        assert_eq!(def.name, "knowledge_search");
    }

    #[tokio::test]
    async fn event_search_empty_journal() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());
        let tool = EventSearchTool::new(journal, None);

        let count = tool.build_index().await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn event_search_finds_cross_session_events() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        // Create two sessions with different content
        journal.put_session(make_session("sess-A")).await.unwrap();
        journal.put_session(make_session("sess-B")).await.unwrap();

        journal
            .append(make_envelope(
                "sess-A",
                EventKind::Message {
                    role: "assistant".into(),
                    content: "The Rust borrow checker ensures memory safety".into(),
                    model: Some("mock".into()),
                    token_usage: None,
                },
            ))
            .await
            .unwrap();

        journal
            .append(make_envelope(
                "sess-B",
                EventKind::Message {
                    role: "assistant".into(),
                    content: "Python garbage collection handles memory automatically".into(),
                    model: Some("mock".into()),
                    token_usage: None,
                },
            ))
            .await
            .unwrap();

        let tool = EventSearchTool::new(journal, None);
        let count = tool.build_index().await.unwrap();
        assert_eq!(count, 2);

        // Search should find the Rust message
        let guard = tool.index.read().unwrap();
        let idx = guard.as_ref().unwrap();
        let results = idx.search("Rust borrow checker", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "sess-A");
    }

    #[tokio::test]
    async fn event_search_excludes_current_session() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        journal.put_session(make_session("current")).await.unwrap();
        journal.put_session(make_session("past")).await.unwrap();

        journal
            .append(make_envelope(
                "current",
                EventKind::Message {
                    role: "assistant".into(),
                    content: "This is the current session about Rust".into(),
                    model: Some("mock".into()),
                    token_usage: None,
                },
            ))
            .await
            .unwrap();

        journal
            .append(make_envelope(
                "past",
                EventKind::Message {
                    role: "assistant".into(),
                    content: "Past session discussing Rust patterns".into(),
                    model: Some("mock".into()),
                    token_usage: None,
                },
            ))
            .await
            .unwrap();

        let tool = EventSearchTool::new(journal, Some("current".into()));
        let count = tool.build_index().await.unwrap();
        assert_eq!(count, 1); // Only past session indexed

        let guard = tool.index.read().unwrap();
        let idx = guard.as_ref().unwrap();
        let results = idx.search("Rust", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, "past");
    }

    #[test]
    fn event_kind_label_variants() {
        let msg = EventKind::Message {
            role: "assistant".into(),
            content: "hi".into(),
            model: None,
            token_usage: None,
        };
        assert_eq!(event_kind_label(&msg), "Message");

        let custom = EventKind::Custom {
            event_type: "eval.InlineCompleted".into(),
            data: serde_json::json!({}),
        };
        assert_eq!(event_kind_label(&custom), "Custom:eval.InlineCompleted");
    }
}
