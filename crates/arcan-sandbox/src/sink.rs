//! `SandboxEventSink` — observer trait for sandbox lifecycle events.
//!
//! Providers emit `SandboxEvent`s by calling `SandboxEventSink::emit()`.
//! The sink decouples event production (provider) from event persistence
//! (Lago, SpacetimeDB, PostHog). Multiple sinks can be composed via
//! `FanoutSink`.

use crate::event::SandboxEvent;

/// Fire-and-forget observer for sandbox lifecycle events.
///
/// Implementations MUST be cheap to call — expensive work (network I/O,
/// disk writes) should be deferred via a background channel.
pub trait SandboxEventSink: Send + Sync + 'static {
    /// Record a single sandbox lifecycle event.
    fn emit(&self, event: SandboxEvent);
}

/// No-op sink — discards all events. Used in tests and when no sink is wired.
pub struct NoopSink;

impl SandboxEventSink for NoopSink {
    fn emit(&self, _event: SandboxEvent) {}
}

/// Broadcasts each event to all inner sinks in order.
pub struct FanoutSink {
    sinks: Vec<Box<dyn SandboxEventSink>>,
}

impl FanoutSink {
    pub fn new(sinks: Vec<Box<dyn SandboxEventSink>>) -> Self {
        Self { sinks }
    }
}

impl SandboxEventSink for FanoutSink {
    fn emit(&self, event: SandboxEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SandboxEvent, SandboxEventKind, SandboxId};
    use std::sync::{Arc, Mutex};

    fn make_event() -> SandboxEvent {
        SandboxEvent::now(
            SandboxId("test".into()),
            "agent-1",
            "sess-1",
            SandboxEventKind::Created,
            "test",
        )
    }

    #[test]
    fn noop_sink_discards_silently() {
        let sink = NoopSink;
        sink.emit(make_event()); // must not panic
    }

    #[test]
    fn fanout_sink_broadcasts_to_all() {
        let counter = Arc::new(Mutex::new(0usize));

        struct CountSink(Arc<Mutex<usize>>);
        impl SandboxEventSink for CountSink {
            fn emit(&self, _event: SandboxEvent) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let sink = FanoutSink::new(vec![
            Box::new(CountSink(Arc::clone(&counter))),
            Box::new(CountSink(Arc::clone(&counter))),
        ]);

        sink.emit(make_event());
        assert_eq!(*counter.lock().unwrap(), 2);
    }
}
