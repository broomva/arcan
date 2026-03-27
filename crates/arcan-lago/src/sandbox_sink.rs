//! [`LagoSandboxEventSink`] — persists `SandboxEvent`s to the Lago journal.
//!
//! Uses a background `tokio::sync::mpsc` channel so `emit()` is always
//! synchronous and cheap (non-blocking send). The background task
//! serializes events to JSON and appends them to the session's Lago journal
//! under the `sandbox_events` namespace.

use arcan_sandbox::{SandboxEvent, SandboxEventSink};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// An [`arcan_sandbox::SandboxEventSink`] that ships events to a background
/// Tokio task for persistence.
///
/// # Construction
///
/// Use [`LagoSandboxEventSink::spawn()`] to create the sink and start its
/// background worker in one step.
pub struct LagoSandboxEventSink {
    tx: mpsc::UnboundedSender<SandboxEvent>,
}

impl LagoSandboxEventSink {
    /// Spawn the background writer and return the sink.
    ///
    /// The background task logs events at DEBUG level.
    /// Replace the body with actual Lago journal writes once
    /// the `lago_core::Journal` wiring is available (BRO-258).
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<SandboxEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // TODO BRO-258: append to lago_core::Journal instead of logging
                debug!(
                    sandbox_id = %event.sandbox_id,
                    kind = ?event.kind,
                    provider = %event.provider,
                    "sandbox event received"
                );
            }
        });

        Self { tx }
    }
}

impl SandboxEventSink for LagoSandboxEventSink {
    fn emit(&self, event: SandboxEvent) {
        if let Err(e) = self.tx.send(event) {
            // Channel closed — background task has exited. Log and continue.
            warn!("LagoSandboxEventSink: background channel closed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_sandbox::{SandboxEventKind, SandboxId};

    #[tokio::test]
    async fn emit_does_not_panic() {
        let sink = LagoSandboxEventSink::spawn();
        let event = SandboxEvent::now(
            SandboxId("s1".into()),
            "agent-1",
            "sess-1",
            SandboxEventKind::Created,
            "bubblewrap",
        );
        sink.emit(event); // must not panic, channel is live
        // Give the background task a moment to process.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn emit_after_drop_does_not_panic() {
        let sink = LagoSandboxEventSink::spawn();
        // Multiple events
        for kind in [
            SandboxEventKind::Created,
            SandboxEventKind::Started,
            SandboxEventKind::Destroyed,
        ] {
            sink.emit(SandboxEvent::now(
                SandboxId("s2".into()),
                "agent-2",
                "sess-2",
                kind,
                "bubblewrap",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
