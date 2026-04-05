use arcan_core::protocol::AgentEvent;
use arcan_core::runtime::{ApprovalGateHook, ApprovalResolver};
use lago_core::event::ApprovalDecision;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Outcome of an approval request.
#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    pub decision: ApprovalDecision,
    pub reason: Option<String>,
}

type EventHandler = Arc<dyn Fn(AgentEvent) + Send + Sync>;

/// Gate that blocks tool execution pending human approval.
///
/// The middleware registers a pending approval via [`request_approval`], which
/// returns a oneshot receiver. An HTTP handler (or timeout task) later calls
/// [`resolve`] to unblock the middleware.
pub struct ApprovalGate {
    pending: Mutex<HashMap<String, oneshot::Sender<ApprovalOutcome>>>,
    event_handler: Mutex<Option<EventHandler>>,
    timeout: Duration,
}

impl ApprovalGate {
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            event_handler: Mutex::new(None),
            timeout,
        }
    }

    /// Set the event handler used to emit approval events to SSE stream + journal.
    /// Called by the AgentLoop before each run.
    pub fn set_event_handler(&self, handler: EventHandler) {
        let mut guard = self.event_handler.lock().unwrap();
        *guard = Some(handler);
    }

    /// Clear the event handler after a run completes.
    pub fn clear_event_handler(&self) {
        let mut guard = self.event_handler.lock().unwrap();
        *guard = None;
    }

    /// Emit an event through the handler if one is set.
    pub fn emit_event(&self, event: AgentEvent) {
        let guard = self.event_handler.lock().unwrap();
        if let Some(handler) = guard.as_ref() {
            handler(event);
        }
    }

    /// Register a pending approval and return a receiver that will deliver the outcome.
    ///
    /// Spawns a timeout task that auto-denies after the configured duration.
    /// Requires `Arc<Self>` so the timeout task can call [`resolve`].
    pub fn request_approval(
        self: &Arc<Self>,
        approval_id: &str,
    ) -> oneshot::Receiver<ApprovalOutcome> {
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(approval_id.to_string(), tx);
        }

        // Spawn timeout task
        let gate = Arc::clone(self);
        let aid = approval_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(gate.timeout).await;
            gate.resolve(
                &aid,
                ApprovalOutcome {
                    decision: ApprovalDecision::Timeout,
                    reason: None,
                },
            );
        });

        rx
    }

    /// Resolve a pending approval. Returns `true` if the approval was found and resolved.
    pub fn resolve(&self, approval_id: &str, outcome: ApprovalOutcome) -> bool {
        let tx = {
            let mut pending = self.pending.lock().unwrap();
            pending.remove(approval_id)
        };
        match tx {
            Some(sender) => sender.send(outcome).is_ok(),
            None => false,
        }
    }

    /// Return the IDs of all pending approvals.
    pub fn pending_ids(&self) -> Vec<String> {
        let pending = self.pending.lock().unwrap();
        pending.keys().cloned().collect()
    }

    /// Return the number of pending approvals.
    pub fn pending_count(&self) -> usize {
        let pending = self.pending.lock().unwrap();
        pending.len()
    }
}

impl ApprovalGateHook for ApprovalGate {
    fn set_event_handler(&self, handler: Arc<dyn Fn(AgentEvent) + Send + Sync>) {
        ApprovalGate::set_event_handler(self, handler);
    }

    fn clear_event_handler(&self) {
        ApprovalGate::clear_event_handler(self);
    }
}

impl ApprovalResolver for ApprovalGate {
    fn resolve_approval(&self, approval_id: &str, decision: &str, reason: Option<String>) -> bool {
        let decision = match decision {
            "approved" => ApprovalDecision::Approved,
            "denied" => ApprovalDecision::Denied,
            _ => ApprovalDecision::Denied,
        };
        self.resolve(approval_id, ApprovalOutcome { decision, reason })
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        self.pending_ids()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn register_and_resolve_approved() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(300)));
        let rx = gate.request_approval("a1");

        let resolved = gate.resolve(
            "a1",
            ApprovalOutcome {
                decision: ApprovalDecision::Approved,
                reason: Some("looks good".into()),
            },
        );
        assert!(resolved);

        let outcome = rx.await.unwrap();
        assert_eq!(outcome.decision, ApprovalDecision::Approved);
        assert_eq!(outcome.reason.as_deref(), Some("looks good"));
    }

    #[tokio::test]
    async fn resolve_denied() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(300)));
        let rx = gate.request_approval("a2");

        gate.resolve(
            "a2",
            ApprovalOutcome {
                decision: ApprovalDecision::Denied,
                reason: Some("too risky".into()),
            },
        );

        let outcome = rx.await.unwrap();
        assert_eq!(outcome.decision, ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn timeout_auto_denies() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_millis(50)));
        let rx = gate.request_approval("a3");

        // Don't resolve — let timeout fire
        let outcome = rx.await.unwrap();
        assert_eq!(outcome.decision, ApprovalDecision::Timeout);
    }

    #[test]
    fn resolve_unknown_id_returns_false() {
        let gate = ApprovalGate::new(Duration::from_secs(300));
        let result = gate.resolve(
            "nonexistent",
            ApprovalOutcome {
                decision: ApprovalDecision::Approved,
                reason: None,
            },
        );
        assert!(!result);
    }

    #[tokio::test]
    async fn pending_count_tracks_correctly() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(300)));
        assert_eq!(gate.pending_count(), 0);

        let _rx1 = gate.request_approval("a1");
        let _rx2 = gate.request_approval("a2");
        assert_eq!(gate.pending_count(), 2);

        let ids = gate.pending_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a1".to_string()));
        assert!(ids.contains(&"a2".to_string()));

        gate.resolve(
            "a1",
            ApprovalOutcome {
                decision: ApprovalDecision::Approved,
                reason: None,
            },
        );
        assert_eq!(gate.pending_count(), 1);
    }

    #[tokio::test]
    async fn concurrent_approvals_isolated() {
        let gate = Arc::new(ApprovalGate::new(Duration::from_secs(300)));
        let rx1 = gate.request_approval("c1");
        let rx2 = gate.request_approval("c2");

        gate.resolve(
            "c2",
            ApprovalOutcome {
                decision: ApprovalDecision::Denied,
                reason: None,
            },
        );
        gate.resolve(
            "c1",
            ApprovalOutcome {
                decision: ApprovalDecision::Approved,
                reason: None,
            },
        );

        let o1 = rx1.await.unwrap();
        let o2 = rx2.await.unwrap();
        assert_eq!(o1.decision, ApprovalDecision::Approved);
        assert_eq!(o2.decision, ApprovalDecision::Denied);
    }

    #[tokio::test]
    async fn event_handler_emits() {
        let gate = ApprovalGate::new(Duration::from_secs(300));
        let count = Arc::new(AtomicUsize::new(0));
        let count_clone = count.clone();

        gate.set_event_handler(Arc::new(move |_event| {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }));

        gate.emit_event(AgentEvent::RunErrored {
            run_id: "r1".into(),
            session_id: "s1".into(),
            error: "test".into(),
        });
        assert_eq!(count.load(Ordering::SeqCst), 1);

        gate.clear_event_handler();
        gate.emit_event(AgentEvent::RunErrored {
            run_id: "r1".into(),
            session_id: "s1".into(),
            error: "test".into(),
        });
        // Should not increment after clearing
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
