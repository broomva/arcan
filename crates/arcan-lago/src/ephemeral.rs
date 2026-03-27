//! Ephemeral journal and session-scoped routing for anonymous/free-tier sessions (BRO-217).
//!
//! Anonymous and free-tier sessions must not persist memory events (`MemoryProposed`,
//! `MemoryCommitted`) to the Lago journal. This module provides two types:
//!
//! - [`EphemeralJournal`]: A no-op `Journal` that silently discards all writes and
//!   returns empty results for reads.
//! - [`SessionJournalSelector`]: Wraps a real `Journal` and routes memory event appends
//!   for registered "ephemeral" sessions to the discard path. All other events (audit
//!   events, non-memory events) pass through to the real journal unchanged.
//!
//! # Enforcement model
//!
//! The `SessionJournalSelector` is the `Journal` passed to `MemoryProposeTool` and
//! `MemoryCommitTool`. The raw journal is passed to `LagoAiosEventStoreAdapter` so
//! that audit events always persist regardless of session tier.
//!
//! ```text
//! MemoryProposeTool ──► SessionJournalSelector ─── ephemeral session ──► EphemeralJournal (discard)
//!                                                └── normal session   ──► RedbJournal (persist)
//!
//! LagoAiosEventStoreAdapter ──► RedbJournal (always persists audit events)
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! let selector = Arc::new(SessionJournalSelector::new(journal.clone()));
//!
//! // Memory tools route through the selector:
//! let memory_journal: Arc<dyn lago_core::Journal> = selector.clone();
//! registry.register(MemoryProposeTool::new(memory_journal.clone()));
//! registry.register(MemoryCommitTool::new(memory_journal.clone()));
//!
//! // Audit events bypass the selector:
//! Arc::new(LagoAiosEventStoreAdapter::new(journal.clone()));
//!
//! // In run_session — mark restricted sessions before tick, unmark after:
//! selector.mark_ephemeral(session_id.clone());
//! // ... tick ...
//! selector.unmark_ephemeral(&session_id);
//! ```

use lago_core::{
    EventQuery, EventStream, Journal,
    error::LagoResult,
    event::{EventEnvelope, EventPayload},
    id::{BranchId, EventId, SeqNo, SessionId},
    session::Session,
};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex},
};

// ─── EphemeralJournal ─────────────────────────────────────────────────────────

/// A no-op journal that silently discards all writes.
///
/// Used as the discard sink for memory events from anonymous/free-tier sessions.
/// All `append` and `append_batch` calls return `Ok(SeqNo(0))` without persisting.
/// All read calls return empty results.
pub struct EphemeralJournal;

impl Journal for EphemeralJournal {
    fn append(
        &self,
        _event: EventEnvelope,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        Box::pin(async move { Ok(0) })
    }

    fn append_batch(
        &self,
        _events: Vec<EventEnvelope>,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        Box::pin(async move { Ok(0) })
    }

    fn read(
        &self,
        _query: EventQuery,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<EventEnvelope>>> + Send + '_>>
    {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn get_event(
        &self,
        _event_id: &EventId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<EventEnvelope>>> + Send + '_>>
    {
        Box::pin(async move { Ok(None) })
    }

    fn head_seq(
        &self,
        _session_id: &SessionId,
        _branch_id: &BranchId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        Box::pin(async move { Ok(0) })
    }

    fn stream(
        &self,
        _session_id: SessionId,
        _branch_id: BranchId,
        _after_seq: SeqNo,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<EventStream>> + Send + '_>> {
        Box::pin(async move {
            let empty: EventStream = Box::pin(tokio_stream::iter(std::iter::empty::<
                LagoResult<EventEnvelope>,
            >()));
            Ok(empty)
        })
    }

    fn put_session(
        &self,
        _session: Session,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }

    fn get_session(
        &self,
        _session_id: &SessionId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<Session>>> + Send + '_>> {
        Box::pin(async move { Ok(None) })
    }

    fn list_sessions(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<Session>>> + Send + '_>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

// ─── SessionJournalSelector ───────────────────────────────────────────────────

/// A `Journal` wrapper that routes memory event appends for ephemeral sessions
/// to [`EphemeralJournal`] (discard), letting all other events pass through.
///
/// Only two event kinds are intercepted:
/// - `EventPayload::MemoryProposed { .. }`
/// - `EventPayload::MemoryCommitted { .. }`
///
/// All other event kinds — and all non-`append` methods — delegate to the
/// inner journal unconditionally.
///
/// Sessions are registered as ephemeral by calling [`mark_ephemeral`] before
/// the agent tick and deregistered by calling [`unmark_ephemeral`] after. The
/// canonical router in `arcand` manages this lifecycle based on the session's
/// tier policy.
///
/// [`mark_ephemeral`]: SessionJournalSelector::mark_ephemeral
/// [`unmark_ephemeral`]: SessionJournalSelector::unmark_ephemeral
pub struct SessionJournalSelector {
    inner: Arc<dyn Journal>,
    /// Ref-counted set of sessions whose memory events should be discarded.
    ///
    /// The `usize` value is the number of outstanding `mark_ephemeral` calls
    /// for that session ID. An entry is removed when the counter reaches zero,
    /// which prevents a race where two concurrent callers mark the same session
    /// and the first `unmark_ephemeral` incorrectly re-enables persistence.
    ephemeral: Mutex<HashMap<String, usize>>,
}

impl SessionJournalSelector {
    /// Wrap `journal` with session-scoped ephemeral routing.
    pub fn new(journal: Arc<dyn Journal>) -> Self {
        Self {
            inner: journal,
            ephemeral: Mutex::new(HashMap::new()),
        }
    }

    /// Register `session_id` as ephemeral.
    ///
    /// Subsequent memory event appends for this session are discarded until
    /// [`unmark_ephemeral`] is called.
    ///
    /// [`unmark_ephemeral`]: Self::unmark_ephemeral
    pub fn mark_ephemeral(&self, session_id: impl AsRef<str>) {
        *self
            .ephemeral
            .lock()
            .unwrap()
            .entry(session_id.as_ref().to_owned())
            .or_insert(0) += 1;
    }

    /// Deregister `session_id` from the ephemeral set.
    ///
    /// Typically called after the agent tick completes. Idempotent — does
    /// nothing if the session was not registered.
    pub fn unmark_ephemeral(&self, session_id: impl AsRef<str>) {
        let mut map = self.ephemeral.lock().unwrap();
        if let Some(count) = map.get_mut(session_id.as_ref()) {
            *count -= 1;
            if *count == 0 {
                map.remove(session_id.as_ref());
            }
        }
    }

    /// Returns `true` if `session_id` is currently registered as ephemeral.
    pub fn is_ephemeral(&self, session_id: &SessionId) -> bool {
        self.ephemeral
            .lock()
            .unwrap()
            .get(session_id.as_str())
            .copied()
            .unwrap_or(0)
            > 0
    }

    /// Returns `true` if the event payload is a memory event that should be
    /// intercepted for ephemeral sessions.
    fn is_memory_event(payload: &EventPayload) -> bool {
        matches!(
            payload,
            EventPayload::MemoryProposed { .. } | EventPayload::MemoryCommitted { .. }
        )
    }
}

impl Journal for SessionJournalSelector {
    fn append(
        &self,
        event: EventEnvelope,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        // Check once under the lock, then release before the async boundary.
        let discard = self.is_ephemeral(&event.session_id) && Self::is_memory_event(&event.payload);

        if discard {
            tracing::trace!(
                session = %event.session_id,
                kind = ?std::mem::discriminant(&event.payload),
                "ephemeral session: discarding memory event"
            );
            Box::pin(async move { Ok(0) })
        } else {
            self.inner.append(event)
        }
    }

    fn append_batch(
        &self,
        events: Vec<EventEnvelope>,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        // Partition: retain events that should not be discarded.
        let filtered: Vec<EventEnvelope> = events
            .into_iter()
            .filter(|e| {
                let discard = self.is_ephemeral(&e.session_id) && Self::is_memory_event(&e.payload);
                if discard {
                    tracing::trace!(
                        session = %e.session_id,
                        "ephemeral session: discarding memory event in batch"
                    );
                }
                !discard
            })
            .collect();

        if filtered.is_empty() {
            Box::pin(async move { Ok(0) })
        } else {
            self.inner.append_batch(filtered)
        }
    }

    // ── All other methods delegate unconditionally ────────────────────────────

    fn read(
        &self,
        query: EventQuery,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<EventEnvelope>>> + Send + '_>>
    {
        self.inner.read(query)
    }

    fn get_event(
        &self,
        event_id: &EventId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<EventEnvelope>>> + Send + '_>>
    {
        self.inner.get_event(event_id)
    }

    fn head_seq(
        &self,
        session_id: &SessionId,
        branch_id: &BranchId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        self.inner.head_seq(session_id, branch_id)
    }

    fn stream(
        &self,
        session_id: SessionId,
        branch_id: BranchId,
        after_seq: SeqNo,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<EventStream>> + Send + '_>> {
        self.inner.stream(session_id, branch_id, after_seq)
    }

    fn put_session(
        &self,
        session: Session,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<()>> + Send + '_>> {
        self.inner.put_session(session)
    }

    fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<Session>>> + Send + '_>> {
        self.inner.get_session(session_id)
    }

    fn list_sessions(
        &self,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<Session>>> + Send + '_>> {
        self.inner.list_sessions()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::{BlobHash, EventKind, MemoryId, MemoryScope};
    use lago_core::event::EventEnvelope;
    use lago_core::id::{BranchId, EventId, SeqNo, SessionId};
    use std::sync::{Arc, Mutex};

    // ── Minimal in-memory journal for testing ─────────────────────────────────

    #[derive(Default)]
    struct RecordingJournal {
        appended: Mutex<Vec<EventEnvelope>>,
    }

    impl RecordingJournal {
        fn count(&self) -> usize {
            self.appended.lock().unwrap().len()
        }
    }

    impl Journal for RecordingJournal {
        fn append(
            &self,
            event: EventEnvelope,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            self.appended.lock().unwrap().push(event);
            Box::pin(async move { Ok(1) })
        }

        fn append_batch(
            &self,
            events: Vec<EventEnvelope>,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            let len = events.len() as SeqNo;
            self.appended.lock().unwrap().extend(events);
            Box::pin(async move { Ok(len) })
        }

        fn read(
            &self,
            _query: EventQuery,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<EventEnvelope>>> + Send + '_>>
        {
            Box::pin(async move { Ok(Vec::new()) })
        }

        fn get_event(
            &self,
            _event_id: &EventId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<EventEnvelope>>> + Send + '_>>
        {
            Box::pin(async move { Ok(None) })
        }

        fn head_seq(
            &self,
            _session_id: &SessionId,
            _branch_id: &BranchId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            Box::pin(async move { Ok(0) })
        }

        fn stream(
            &self,
            _session_id: SessionId,
            _branch_id: BranchId,
            _after_seq: SeqNo,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<EventStream>> + Send + '_>>
        {
            Box::pin(async move {
                let empty: EventStream = Box::pin(tokio_stream::iter(std::iter::empty::<
                    LagoResult<EventEnvelope>,
                >()));
                Ok(empty)
            })
        }

        fn put_session(
            &self,
            _session: Session,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<()>> + Send + '_>> {
            Box::pin(async move { Ok(()) })
        }

        fn get_session(
            &self,
            _session_id: &SessionId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<Session>>> + Send + '_>>
        {
            Box::pin(async move { Ok(None) })
        }

        fn list_sessions(
            &self,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<Session>>> + Send + '_>>
        {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    // ── Helper to build a minimal EventEnvelope ───────────────────────────────

    fn make_envelope(session_id: &SessionId, payload: EventKind) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq: 0,
            timestamp: 0,
            parent_id: None,
            payload,
            metadata: Default::default(),
            schema_version: 1,
        }
    }

    fn memory_proposed_event() -> EventKind {
        EventKind::MemoryProposed {
            scope: MemoryScope::Session,
            proposal_id: MemoryId::new_uuid(),
            entries_ref: BlobHash::from_hex("deadbeef"),
            source_run_id: None,
        }
    }

    fn memory_committed_event() -> EventKind {
        EventKind::MemoryCommitted {
            scope: MemoryScope::Session,
            memory_id: MemoryId::new_uuid(),
            committed_ref: BlobHash::from_hex("cafebabe"),
            supersedes: None,
        }
    }

    fn tool_call_event() -> EventKind {
        EventKind::ToolCallRequested {
            call_id: "call-1".to_owned(),
            tool_name: "bash".to_owned(),
            arguments: serde_json::json!({"command": "ls"}),
            category: None,
        }
    }

    // ── EphemeralJournal ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn ephemeral_journal_discards_appends() {
        let j = EphemeralJournal;
        let env = make_envelope(&SessionId::from_string("test"), memory_proposed_event());
        let seq = j.append(env).await.unwrap();
        assert_eq!(seq, 0, "ephemeral journal must return seq 0 (discarded)");
    }

    #[tokio::test]
    async fn ephemeral_journal_returns_empty_reads() {
        let j = EphemeralJournal;
        let events = j.read(EventQuery::default()).await.unwrap();
        assert!(events.is_empty());
        let sessions = j.list_sessions().await.unwrap();
        assert!(sessions.is_empty());
    }

    // ── SessionJournalSelector ────────────────────────────────────────────────

    #[tokio::test]
    async fn selector_passes_through_for_non_ephemeral_session() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("pro-session");

        let env = make_envelope(&session, memory_proposed_event());
        selector.append(env).await.unwrap();
        assert_eq!(inner.count(), 1, "non-ephemeral session must persist");
    }

    #[tokio::test]
    async fn selector_discards_memory_proposed_for_ephemeral_session() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("anon-session");
        selector.mark_ephemeral(session.clone());

        let env = make_envelope(&session, memory_proposed_event());
        let seq = selector.append(env).await.unwrap();
        assert_eq!(seq, 0, "memory event must be discarded");
        assert_eq!(inner.count(), 0, "inner journal must not be written");
    }

    #[tokio::test]
    async fn selector_discards_memory_committed_for_ephemeral_session() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("free-session");
        selector.mark_ephemeral(session.clone());

        let env = make_envelope(&session, memory_committed_event());
        selector.append(env).await.unwrap();
        assert_eq!(inner.count(), 0, "memory committed must be discarded");
    }

    #[tokio::test]
    async fn selector_allows_non_memory_events_for_ephemeral_session() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("anon-session");
        selector.mark_ephemeral(session.clone());

        let env = make_envelope(&session, tool_call_event());
        selector.append(env).await.unwrap();
        assert_eq!(
            inner.count(),
            1,
            "non-memory event must persist even for ephemeral session"
        );
    }

    #[tokio::test]
    async fn selector_resumes_persistence_after_unmark() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("anon-session");
        selector.mark_ephemeral(session.clone());

        // Memory event while ephemeral → discarded.
        selector
            .append(make_envelope(&session, memory_proposed_event()))
            .await
            .unwrap();
        assert_eq!(inner.count(), 0);

        // Unmark (session upgraded or tick completed).
        selector.unmark_ephemeral(&session);

        // Now memory event must persist.
        selector
            .append(make_envelope(&session, memory_proposed_event()))
            .await
            .unwrap();
        assert_eq!(inner.count(), 1, "after unmark, memory events must persist");
    }

    #[tokio::test]
    async fn selector_batch_filters_memory_events_for_ephemeral_session() {
        let inner = Arc::new(RecordingJournal::default());
        let selector = SessionJournalSelector::new(inner.clone());
        let session = SessionId::from_string("free-session");
        selector.mark_ephemeral(session.clone());

        let events = vec![
            make_envelope(&session, memory_proposed_event()),
            make_envelope(&session, tool_call_event()),
            make_envelope(&session, memory_committed_event()),
        ];
        selector.append_batch(events).await.unwrap();
        // Only the ToolCallRequested must reach the inner journal.
        assert_eq!(inner.count(), 1);
    }

    #[tokio::test]
    async fn selector_ref_counts_concurrent_marks() {
        // Two concurrent callers both mark the same session as ephemeral.
        // The session must remain ephemeral until *both* have called unmark.
        let inner = Arc::new(RecordingJournal::default());
        let selector = Arc::new(SessionJournalSelector::new(inner.clone()));
        let session = SessionId::from_string("shared-session");

        // Simulate two concurrent "mark" holders (e.g. two overlapping ticks).
        selector.mark_ephemeral(session.clone()); // ref-count = 1
        selector.mark_ephemeral(session.clone()); // ref-count = 2

        // First unmark: ref-count drops to 1 — session is still ephemeral.
        selector.unmark_ephemeral(&session);
        assert!(
            selector.is_ephemeral(&session),
            "session must still be ephemeral after first unmark (ref-count = 1)"
        );
        selector
            .append(make_envelope(&session, memory_proposed_event()))
            .await
            .unwrap();
        assert_eq!(inner.count(), 0, "memory event must still be discarded");

        // Second unmark: ref-count reaches 0 — session is no longer ephemeral.
        selector.unmark_ephemeral(&session);
        assert!(
            !selector.is_ephemeral(&session),
            "session must no longer be ephemeral after second unmark (ref-count = 0)"
        );
        selector
            .append(make_envelope(&session, memory_proposed_event()))
            .await
            .unwrap();
        assert_eq!(inner.count(), 1, "memory event must persist after full unmark");
    }
}
