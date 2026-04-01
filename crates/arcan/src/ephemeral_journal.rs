//! In-memory ephemeral journal — fallback when the redb journal is locked.
//!
//! Implements `lago_core::journal::Journal` with in-memory storage.
//! Sessions and events are lost when the process exits.

use lago_core::event::EventEnvelope;
use lago_core::id::{BranchId, EventId, SessionId};
use lago_core::journal::{EventQuery, EventStream, Journal};
use lago_core::session::Session;
use std::pin::Pin;
use std::sync::Mutex;

type BoxFut<'a, T> =
    Pin<Box<dyn std::future::Future<Output = lago_core::error::LagoResult<T>> + Send + 'a>>;

pub struct EphemeralJournal {
    events: Mutex<Vec<EventEnvelope>>,
    sessions: Mutex<Vec<Session>>,
}

impl EphemeralJournal {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
        }
    }
}

impl Journal for EphemeralJournal {
    fn append(&self, mut event: EventEnvelope) -> BoxFut<'_, u64> {
        Box::pin(async move {
            let mut events = self.events.lock().expect("lock");
            let seq = events.len() as u64 + 1;
            event.seq = seq;
            events.push(event);
            Ok(seq)
        })
    }

    fn append_batch(&self, mut batch: Vec<EventEnvelope>) -> BoxFut<'_, u64> {
        Box::pin(async move {
            let mut events = self.events.lock().expect("lock");
            let mut seq = events.len() as u64;
            for event in &mut batch {
                seq += 1;
                event.seq = seq;
            }
            events.extend(batch);
            Ok(seq)
        })
    }

    fn read(&self, query: EventQuery) -> BoxFut<'_, Vec<EventEnvelope>> {
        Box::pin(async move {
            let events = self.events.lock().expect("lock");
            let filtered = events
                .iter()
                .filter(|e| {
                    if let Some(ref sid) = query.session_id
                        && e.session_id != *sid
                    {
                        return false;
                    }
                    if let Some(ref bid) = query.branch_id
                        && e.branch_id != *bid
                    {
                        return false;
                    }
                    true
                })
                .cloned()
                .collect();
            Ok(filtered)
        })
    }

    fn get_event(&self, event_id: &EventId) -> BoxFut<'_, Option<EventEnvelope>> {
        let eid = event_id.clone();
        Box::pin(async move {
            let events = self.events.lock().expect("lock");
            Ok(events.iter().find(|e| e.event_id == eid).cloned())
        })
    }

    fn head_seq(&self, _session_id: &SessionId, _branch_id: &BranchId) -> BoxFut<'_, u64> {
        Box::pin(async move {
            let events = self.events.lock().expect("lock");
            Ok(events.len() as u64)
        })
    }

    fn stream(
        &self,
        _session_id: SessionId,
        _branch_id: BranchId,
        _after_seq: u64,
    ) -> BoxFut<'_, EventStream> {
        Box::pin(async move {
            // Return an empty stream
            Ok(Box::pin(futures_util::stream::empty()) as EventStream)
        })
    }

    fn put_session(&self, session: Session) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let mut sessions = self.sessions.lock().expect("lock");
            sessions.push(session);
            Ok(())
        })
    }

    fn get_session(&self, session_id: &SessionId) -> BoxFut<'_, Option<Session>> {
        let sid = session_id.clone();
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("lock");
            Ok(sessions.iter().find(|s| s.session_id == sid).cloned())
        })
    }

    fn list_sessions(&self) -> BoxFut<'_, Vec<Session>> {
        Box::pin(async move {
            let sessions = self.sessions.lock().expect("lock");
            Ok(sessions.clone())
        })
    }
}
