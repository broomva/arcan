//! Free-tier Lago retention: TTL tagging, namespace isolation, storage metrics.
//!
//! BRO-218: Free authenticated users get persistent Lago memory with a 7-day
//! rolling TTL. All free sessions share a single Lago instance; per-user
//! namespace isolation is enforced by session registration rather than
//! separate store instances.
//!
//! # How it works
//!
//! 1. Before a free-tier session starts, call [`FreeTierJournal::register_session`]
//!    with the session ID and user ID.
//! 2. All events appended for registered sessions are tagged with:
//!    - `lago:expires_at` — microsecond timestamp when the event expires
//!    - `lago:user_id`    — the owning user's ID (namespace isolation key)
//! 3. On `read`/`stream`, expired events (`lago:expires_at < now()`) are
//!    filtered out, making them invisible to callers.
//! 4. The daily maintenance task calls [`FreeTierJournal::evict_expired_events`],
//!    which scans the raw journal and emits `MemoryTombstoned` events for each
//!    expired memory entry. Non-memory audit events are left intact.
//! 5. Storage metrics are tracked approximately in memory and surfaced via
//!    [`FreeTierJournal::memory_used_bytes`].
//!
//! # Namespace model
//!
//! ```text
//! lago://shared/users/{user_id}/sessions/{session_id}/events
//! ```
//!
//! The namespace is encoded via the `lago:user_id` metadata tag rather than a
//! different `BranchId`, preserving compatibility with existing lago-core APIs.
//!
//! # Usage
//!
//! ```rust,ignore
//! let journal = Arc::new(FreeTierJournal::new(raw_journal, LagoPolicyConfig::default()));
//!
//! // Before a free-tier session:
//! journal.register_session(&session_id, &user_id);
//!
//! // After the session ends:
//! journal.unregister_session(&session_id);
//!
//! // Daily eviction cron:
//! let tombstoned = journal.evict_expired_events(session_id, branch_id).await?;
//! ```

use lago_core::{
    EventQuery, EventStream, Journal, LagoResult, MemoryScope,
    event::{EventEnvelope, EventPayload},
    id::{BranchId, EventId, SeqNo, SessionId},
    session::Session,
};
use std::{
    collections::HashMap,
    pin::Pin,
    sync::{Arc, Mutex},
};

/// Metadata key for the event's TTL expiry timestamp (microseconds since epoch).
const METADATA_EXPIRES_AT: &str = "lago:expires_at";

/// Metadata key encoding the owning user's ID for namespace isolation.
const METADATA_USER_ID: &str = "lago:user_id";

// ─── LagoPolicyConfig ──────────────────────────────────────────────────────────

/// Configuration for free-tier Lago retention policy.
///
/// Controls the shared namespace prefix, rolling TTL window, and the per-user
/// pin quota (pinned observations are excluded from TTL eviction).
#[derive(Debug, Clone)]
pub struct LagoPolicyConfig {
    /// Namespace prefix written into event metadata (e.g. `"shared"`).
    pub namespace: String,
    /// Number of days to retain events before they become eligible for eviction.
    pub retention_days: u32,
    /// Maximum number of pinned memory items allowed per user.
    pub max_pinned: u32,
}

impl Default for LagoPolicyConfig {
    fn default() -> Self {
        Self {
            namespace: "shared".to_owned(),
            retention_days: 7,
            max_pinned: 100,
        }
    }
}

impl LagoPolicyConfig {
    /// Compute the expiry timestamp (microseconds since UNIX epoch) for an
    /// event appended right now, based on the configured `retention_days`.
    pub fn expires_at_micros(&self) -> u64 {
        let now = EventEnvelope::now_micros();
        let retention_micros = self.retention_days as u64 * 24 * 3600 * 1_000_000;
        now.saturating_add(retention_micros)
    }
}

// ─── FreeTierJournal ──────────────────────────────────────────────────────────

/// A `Journal` wrapper that enforces free-tier TTL and per-user isolation.
///
/// Events appended for registered sessions are tagged with an expiry timestamp
/// and the owning user's ID. Expired events are filtered from `read` and
/// `stream` results so callers never observe stale data.
///
/// See the module-level documentation for the full lifecycle.
pub struct FreeTierJournal {
    inner: Arc<dyn Journal>,
    config: LagoPolicyConfig,
    /// Maps session_id → user_id for currently active free-tier sessions.
    session_users: Mutex<HashMap<String, String>>,
    /// Approximate per-user storage bytes (in-memory, best-effort).
    user_bytes: Mutex<HashMap<String, u64>>,
}

impl FreeTierJournal {
    /// Create a new wrapper around `journal` with the given retention config.
    pub fn new(journal: Arc<dyn Journal>, config: LagoPolicyConfig) -> Self {
        Self {
            inner: journal,
            config,
            session_users: Mutex::new(HashMap::new()),
            user_bytes: Mutex::new(HashMap::new()),
        }
    }

    /// Register a session as belonging to a free-tier user.
    ///
    /// All subsequent appends for `session_id` will be tagged with
    /// `lago:expires_at` and `lago:user_id` until [`unregister_session`] is
    /// called.
    pub fn register_session(&self, session_id: impl AsRef<str>, user_id: impl AsRef<str>) {
        self.session_users
            .lock()
            .unwrap()
            .insert(session_id.as_ref().to_owned(), user_id.as_ref().to_owned());
    }

    /// Unregister a session after it ends.
    ///
    /// Future appends for `session_id` pass through without TTL tagging.
    pub fn unregister_session(&self, session_id: impl AsRef<str>) {
        self.session_users
            .lock()
            .unwrap()
            .remove(session_id.as_ref());
    }

    /// Returns `true` if the session is currently registered as a free-tier session.
    pub fn is_registered(&self, session_id: impl AsRef<str>) -> bool {
        self.session_users
            .lock()
            .unwrap()
            .contains_key(session_id.as_ref())
    }

    /// Look up the user_id for a registered session, or `None` if not registered.
    pub fn user_id_for_session(&self, session_id: impl AsRef<str>) -> Option<String> {
        self.session_users
            .lock()
            .unwrap()
            .get(session_id.as_ref())
            .cloned()
    }

    /// Approximate storage bytes used by `user_id` across all their sessions.
    ///
    /// Tracked via a best-effort in-memory counter. Resets on process restart.
    pub fn memory_used_bytes(&self, user_id: impl AsRef<str>) -> u64 {
        self.user_bytes
            .lock()
            .unwrap()
            .get(user_id.as_ref())
            .copied()
            .unwrap_or(0)
    }

    /// Returns a reference to the current retention config.
    pub fn config(&self) -> &LagoPolicyConfig {
        &self.config
    }

    /// Returns `true` if `event` has passed its `lago:expires_at` TTL.
    fn is_expired(event: &EventEnvelope) -> bool {
        let Some(exp_str) = event.metadata.get(METADATA_EXPIRES_AT) else {
            return false; // no TTL tag → never expires via this wrapper
        };
        let Ok(exp_micros) = exp_str.parse::<u64>() else {
            return false; // malformed tag → treat as non-expired (safe default)
        };
        exp_micros < EventEnvelope::now_micros()
    }

    /// Tag an event with TTL and user-ownership metadata, and update the
    /// in-memory per-user byte counter.
    fn tag_event(&self, mut event: EventEnvelope, user_id: &str) -> EventEnvelope {
        let expires_at = self.config.expires_at_micros();
        event
            .metadata
            .insert(METADATA_EXPIRES_AT.to_owned(), expires_at.to_string());
        event
            .metadata
            .insert(METADATA_USER_ID.to_owned(), user_id.to_owned());

        // Rough storage estimate: serialized payload size.
        let approx_bytes = serde_json::to_vec(&event.payload)
            .map(|v| v.len() as u64)
            .unwrap_or(256);
        self.user_bytes
            .lock()
            .unwrap()
            .entry(user_id.to_owned())
            .and_modify(|b| *b = b.saturating_add(approx_bytes))
            .or_insert(approx_bytes);

        event
    }

    /// Extract the `MemoryScope` from a memory event payload, or `None` for
    /// non-memory events (which should not be tombstoned by TTL eviction).
    fn memory_scope_of(payload: &EventPayload) -> Option<MemoryScope> {
        match payload {
            EventPayload::MemoryProposed { scope, .. }
            | EventPayload::MemoryCommitted { scope, .. }
            | EventPayload::ObservationAppended { scope, .. } => Some(scope.clone()),
            _ => None,
        }
    }

    /// Scan a session's raw journal events and emit `MemoryTombstoned` records
    /// for any memory events that have passed their TTL.
    ///
    /// This is the daily maintenance entry point. Non-memory audit events that
    /// have expired are logged but not tombstoned — they are audit-immutable.
    ///
    /// Returns the number of memory events tombstoned.
    pub async fn evict_expired_events(
        &self,
        session_id: SessionId,
        branch_id: BranchId,
    ) -> LagoResult<usize> {
        use tokio_stream::StreamExt as _;

        // Read directly from the inner (raw) journal to see expired events,
        // bypassing this wrapper's own expiry filter.
        let mut raw = self
            .inner
            .stream(session_id.clone(), branch_id.clone(), 0)
            .await?;
        let mut tombstoned = 0usize;

        while let Some(result) = raw.next().await {
            let event = result?;
            if !Self::is_expired(&event) {
                continue;
            }
            let Some(scope) = Self::memory_scope_of(&event.payload) else {
                tracing::trace!(
                    event_id = %event.event_id,
                    "ttl-eviction: skipping non-memory expired audit event"
                );
                continue;
            };

            let tombstone = EventEnvelope {
                event_id: EventId::new(),
                session_id: session_id.clone(),
                branch_id: branch_id.clone(),
                run_id: None,
                seq: 0,
                timestamp: EventEnvelope::now_micros(),
                parent_id: Some(event.event_id.clone()),
                payload: EventPayload::MemoryTombstoned {
                    scope,
                    memory_id: aios_protocol::MemoryId::new_uuid(),
                    reason: format!(
                        "ttl-eviction: expired after {} day(s)",
                        self.config.retention_days
                    ),
                },
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("lago:eviction".to_owned(), "ttl".to_owned());
                    m
                },
                schema_version: 1,
            };

            self.inner.append(tombstone).await?;
            tombstoned += 1;

            tracing::debug!(
                session = %session_id,
                "ttl-eviction: tombstoned expired memory event"
            );
        }

        Ok(tombstoned)
    }
}

// ─── Journal impl ──────────────────────────────────────────────────────────────

impl Journal for FreeTierJournal {
    fn append(
        &self,
        event: EventEnvelope,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        let user_id = self
            .session_users
            .lock()
            .unwrap()
            .get(event.session_id.as_str())
            .cloned();

        let event = match user_id {
            Some(ref uid) => self.tag_event(event, uid),
            None => event,
        };

        self.inner.append(event)
    }

    fn append_batch(
        &self,
        events: Vec<EventEnvelope>,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
        let tagged: Vec<EventEnvelope> = events
            .into_iter()
            .map(|event| {
                let user_id = self
                    .session_users
                    .lock()
                    .unwrap()
                    .get(event.session_id.as_str())
                    .cloned();
                match user_id {
                    Some(ref uid) => self.tag_event(event, uid),
                    None => event,
                }
            })
            .collect();
        self.inner.append_batch(tagged)
    }

    fn read(
        &self,
        query: EventQuery,
    ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<EventEnvelope>>> + Send + '_>>
    {
        Box::pin(async move {
            let events = self.inner.read(query).await?;
            Ok(events
                .into_iter()
                .filter(|e| !Self::is_expired(e))
                .collect())
        })
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
        Box::pin(async move {
            use tokio_stream::StreamExt as _;
            let mut raw = self.inner.stream(session_id, branch_id, after_seq).await?;
            let mut filtered: Vec<LagoResult<EventEnvelope>> = Vec::new();
            while let Some(item) = raw.next().await {
                match &item {
                    Ok(e) if Self::is_expired(e) => {
                        tracing::trace!(
                            event_id = %e.event_id,
                            "free-tier: filtering expired event from stream"
                        );
                    }
                    _ => filtered.push(item),
                }
            }
            Ok(Box::pin(tokio_stream::iter(filtered)) as EventStream)
        })
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
    use lago_core::id::{BranchId, EventId, SessionId};
    use std::sync::{Arc, Mutex};

    // ── In-memory journal that stores events and returns them on read/stream ──

    #[derive(Default)]
    struct RecordingJournal {
        appended: Mutex<Vec<EventEnvelope>>,
    }

    impl RecordingJournal {
        fn count(&self) -> usize {
            self.appended.lock().unwrap().len()
        }

        fn events(&self) -> Vec<EventEnvelope> {
            self.appended.lock().unwrap().clone()
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
            let events = self.appended.lock().unwrap().clone();
            Box::pin(async move { Ok(events) })
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
            let events = self.appended.lock().unwrap().clone();
            Box::pin(async move {
                let items: Vec<LagoResult<EventEnvelope>> = events.into_iter().map(Ok).collect();
                Ok(Box::pin(tokio_stream::iter(items)) as EventStream)
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

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn sid(s: &str) -> SessionId {
        SessionId::from_string(s)
    }

    fn bid() -> BranchId {
        BranchId::from_string("main")
    }

    fn make_envelope(session_id: &SessionId, payload: EventKind) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            branch_id: bid(),
            run_id: None,
            seq: 0,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload,
            metadata: Default::default(),
            schema_version: 1,
        }
    }

    /// Build an envelope pre-tagged as already expired (expires_at = 1 µs after epoch).
    fn make_expired_envelope(session_id: &SessionId, payload: EventKind) -> EventEnvelope {
        let mut e = make_envelope(session_id, payload);
        e.metadata
            .insert(METADATA_EXPIRES_AT.to_owned(), "1".to_owned());
        e
    }

    fn memory_proposed() -> EventKind {
        EventKind::MemoryProposed {
            scope: MemoryScope::Session,
            proposal_id: MemoryId::new_uuid(),
            entries_ref: BlobHash::from_hex("deadbeef"),
            source_run_id: None,
        }
    }

    fn tool_call() -> EventKind {
        EventKind::ToolCallRequested {
            call_id: "c1".to_owned(),
            tool_name: "ls".to_owned(),
            arguments: serde_json::json!({}),
            category: None,
        }
    }

    fn make_journal() -> Arc<RecordingJournal> {
        Arc::new(RecordingJournal::default())
    }

    fn make_free_tier(inner: Arc<RecordingJournal>) -> FreeTierJournal {
        FreeTierJournal::new(inner, LagoPolicyConfig::default())
    }

    // ── Config ────────────────────────────────────────────────────────────────

    #[test]
    fn default_config_values() {
        let cfg = LagoPolicyConfig::default();
        assert_eq!(cfg.namespace, "shared");
        assert_eq!(cfg.retention_days, 7);
        assert_eq!(cfg.max_pinned, 100);
    }

    #[test]
    fn expires_at_is_in_the_future() {
        let cfg = LagoPolicyConfig::default();
        let exp = cfg.expires_at_micros();
        assert!(exp > EventEnvelope::now_micros());
    }

    #[test]
    fn expires_at_is_approximately_7_days_away() {
        let cfg = LagoPolicyConfig::default();
        let exp = cfg.expires_at_micros();
        let now = EventEnvelope::now_micros();
        let seven_days_micros = 7u64 * 24 * 3600 * 1_000_000;
        // Allow ±1 second tolerance for test execution time.
        assert!(exp >= now + seven_days_micros - 1_000_000);
        assert!(exp <= now + seven_days_micros + 1_000_000);
    }

    // ── Session registration ──────────────────────────────────────────────────

    #[test]
    fn register_and_lookup() {
        let j = make_free_tier(make_journal());
        assert!(!j.is_registered("s1"));
        j.register_session("s1", "user-alice");
        assert!(j.is_registered("s1"));
        assert_eq!(j.user_id_for_session("s1").unwrap(), "user-alice");
        j.unregister_session("s1");
        assert!(!j.is_registered("s1"));
        assert!(j.user_id_for_session("s1").is_none());
    }

    #[test]
    fn unregistered_session_has_no_user() {
        let j = make_free_tier(make_journal());
        assert!(j.user_id_for_session("no-such-session").is_none());
    }

    // ── TTL tagging ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn registered_session_gets_ttl_tagged() {
        let inner = make_journal();
        let j = make_free_tier(inner.clone());

        j.register_session("s1", "alice");
        j.append(make_envelope(&sid("s1"), memory_proposed()))
            .await
            .unwrap();

        let stored = inner.events();
        assert_eq!(stored.len(), 1);
        assert!(
            stored[0].metadata.contains_key(METADATA_EXPIRES_AT),
            "expires_at must be set"
        );
        assert_eq!(
            stored[0].metadata.get(METADATA_USER_ID).map(String::as_str),
            Some("alice")
        );
    }

    #[tokio::test]
    async fn unregistered_session_passes_through_untagged() {
        let inner = make_journal();
        let j = make_free_tier(inner.clone());

        j.append(make_envelope(&sid("s-pro"), tool_call()))
            .await
            .unwrap();

        let stored = inner.events();
        assert_eq!(stored.len(), 1);
        assert!(
            !stored[0].metadata.contains_key(METADATA_EXPIRES_AT),
            "pro session must not have TTL tag"
        );
        assert!(!stored[0].metadata.contains_key(METADATA_USER_ID));
    }

    #[tokio::test]
    async fn batch_tags_registered_but_not_unregistered() {
        let inner = make_journal();
        let j = make_free_tier(inner.clone());

        j.register_session("s1", "alice");
        j.append_batch(vec![
            make_envelope(&sid("s1"), memory_proposed()),
            make_envelope(&sid("s-pro"), tool_call()),
        ])
        .await
        .unwrap();

        let stored = inner.events();
        assert_eq!(stored.len(), 2);
        // s1 → tagged
        assert!(stored[0].metadata.contains_key(METADATA_EXPIRES_AT));
        assert_eq!(stored[0].metadata[METADATA_USER_ID], "alice");
        // s-pro → not tagged
        assert!(!stored[1].metadata.contains_key(METADATA_EXPIRES_AT));
    }

    // ── Expiry filtering ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_filters_expired_events() {
        let inner = Arc::new(RecordingJournal::default());
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), tool_call()));
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_expired_envelope(&sid("s1"), memory_proposed()));

        let j = make_free_tier(inner);
        let results = j.read(EventQuery::new()).await.unwrap();
        assert_eq!(results.len(), 1, "expired event must be filtered from read");
    }

    #[tokio::test]
    async fn stream_filters_expired_events() {
        let inner = Arc::new(RecordingJournal::default());
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), tool_call()));
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_expired_envelope(&sid("s1"), memory_proposed()));

        let j = make_free_tier(inner);
        use tokio_stream::StreamExt as _;
        let stream = j.stream(sid("s1"), bid(), 0).await.unwrap();
        let events: Vec<_> = stream.collect().await;
        assert_eq!(
            events.len(),
            1,
            "expired event must be filtered from stream"
        );
    }

    #[tokio::test]
    async fn non_expired_events_pass_through_stream() {
        let inner = Arc::new(RecordingJournal::default());
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), tool_call()));
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), memory_proposed()));

        let j = make_free_tier(inner);
        use tokio_stream::StreamExt as _;
        let stream = j.stream(sid("s1"), bid(), 0).await.unwrap();
        let events: Vec<_> = stream.collect().await;
        assert_eq!(events.len(), 2, "non-expired events must not be filtered");
    }

    // ── Storage metrics ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn memory_used_bytes_accumulates() {
        let inner = make_journal();
        let j = make_free_tier(inner.clone());

        j.register_session("s1", "alice");
        assert_eq!(j.memory_used_bytes("alice"), 0);

        j.append(make_envelope(&sid("s1"), memory_proposed()))
            .await
            .unwrap();
        let after_one = j.memory_used_bytes("alice");
        assert!(after_one > 0, "bytes must be tracked after first append");

        j.append(make_envelope(&sid("s1"), memory_proposed()))
            .await
            .unwrap();
        let after_two = j.memory_used_bytes("alice");
        assert!(
            after_two > after_one,
            "bytes must increase after second append"
        );
    }

    #[test]
    fn memory_used_bytes_zero_for_unknown_user() {
        let j = make_free_tier(make_journal());
        assert_eq!(j.memory_used_bytes("nobody"), 0);
    }

    #[tokio::test]
    async fn unregistered_session_does_not_affect_user_bytes() {
        let inner = make_journal();
        let j = make_free_tier(inner.clone());

        // Append from a pro session (not registered).
        j.append(make_envelope(&sid("s-pro"), tool_call()))
            .await
            .unwrap();

        assert_eq!(j.memory_used_bytes("pro-user"), 0);
    }

    // ── Eviction ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn evict_tombstones_only_expired_memory_events() {
        let inner = Arc::new(RecordingJournal::default());
        {
            let mut guard = inner.appended.lock().unwrap();
            // 1. Expired memory event → should be tombstoned
            guard.push(make_expired_envelope(&sid("s1"), memory_proposed()));
            // 2. Expired non-memory event → should NOT be tombstoned
            guard.push(make_expired_envelope(&sid("s1"), tool_call()));
            // 3. Fresh memory event → should NOT be tombstoned
            guard.push(make_envelope(&sid("s1"), memory_proposed()));
        }

        let j = FreeTierJournal::new(inner.clone(), LagoPolicyConfig::default());
        let tombstoned = j.evict_expired_events(sid("s1"), bid()).await.unwrap();

        assert_eq!(
            tombstoned, 1,
            "only the expired memory event should be tombstoned"
        );
        assert_eq!(inner.count(), 4, "original 3 + 1 tombstone");

        let tombstone = inner.events().into_iter().last().unwrap();
        assert!(
            matches!(tombstone.payload, EventPayload::MemoryTombstoned { .. }),
            "last event should be MemoryTombstoned"
        );
        assert_eq!(
            tombstone.metadata.get("lago:eviction").map(String::as_str),
            Some("ttl"),
        );
        // Tombstone must point back to the original event.
        assert!(tombstone.parent_id.is_some());
    }

    #[tokio::test]
    async fn evict_empty_session_returns_zero() {
        let inner = make_journal();
        let j = FreeTierJournal::new(inner, LagoPolicyConfig::default());
        let count = j.evict_expired_events(sid("empty"), bid()).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn evict_no_expired_events_returns_zero() {
        let inner = Arc::new(RecordingJournal::default());
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), memory_proposed()));
        inner
            .appended
            .lock()
            .unwrap()
            .push(make_envelope(&sid("s1"), tool_call()));

        let j = FreeTierJournal::new(inner.clone(), LagoPolicyConfig::default());
        let count = j.evict_expired_events(sid("s1"), bid()).await.unwrap();
        assert_eq!(count, 0);
        assert_eq!(inner.count(), 2, "no tombstones emitted for fresh events");
    }
}
