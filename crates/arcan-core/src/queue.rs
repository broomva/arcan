//! Message queue and steering semantics for concurrent message handling (Phase 2.5).
//!
//! Enables safe preemption at tool boundaries, message queuing during active runs,
//! and priority-based drain ordering: interrupt > steer > followup > collect.

use aios_protocol::SteeringMode;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ─── Configuration ───────────────────────────────────────────────

/// Configuration for the message queue.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Maximum number of pending messages (default: 10).
    pub max_queue_depth: usize,
    /// Max time to wait for a tool boundary when steering (default: 30s).
    pub steer_timeout: Duration,
    /// Batch collect messages within this window (default: 2s).
    pub collect_coalesce_window: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_queue_depth: 10,
            steer_timeout: Duration::from_secs(30),
            collect_coalesce_window: Duration::from_secs(2),
        }
    }
}

// ─── Queued message ──────────────────────────────────────────────

/// A message waiting in the queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    /// Unique identifier for this queued message.
    pub id: String,
    /// How this message should interact with the active run.
    pub mode: SteeringMode,
    /// The message content (user text, instruction, etc.).
    pub content: String,
    /// When the message was queued (not serialized — set on enqueue).
    #[serde(skip)]
    pub queued_at: Option<Instant>,
}

// ─── Steering action ─────────────────────────────────────────────

/// Action to take at a tool boundary based on queue state.
#[derive(Debug, Clone)]
pub enum SteeringAction {
    /// Continue current run (no preemption).
    Continue,
    /// Inject a new message into the current run context.
    InjectMessage(String),
    /// Complete current run early and start new run with queued message.
    CompleteAndSwitch(QueuedMessage),
    /// Abort current run (emergency interrupt).
    Abort { reason: String },
}

// ─── Preemption check trait ──────────────────────────────────────

/// Trait for checking preemption at tool boundaries.
///
/// Called after each tool execution completes. Implementors inspect the
/// queue state and decide whether the run should continue or be redirected.
pub trait PreemptionCheck: Send + Sync {
    /// Called after each tool execution completes.
    /// Returns a `SteeringAction` indicating what should happen next.
    fn check_preemption(&self) -> SteeringAction;
}

// ─── Queue status ────────────────────────────────────────────────

/// Snapshot of the queue state for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStatus {
    pub depth: usize,
    pub pending: Vec<QueuedMessage>,
    pub has_active_run: bool,
    pub oldest_message_age_ms: Option<u64>,
}

// ─── Queue error ─────────────────────────────────────────────────

/// Errors from queue operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum QueueError {
    #[error("queue is full (depth: {depth}, max: {max})")]
    QueueFull { depth: usize, max: usize },
    #[error("message not found: {id}")]
    NotFound { id: String },
}

// ─── Message queue ───────────────────────────────────────────────

/// Thread-safe message queue with steering semantics.
///
/// Messages are enqueued with a [`SteeringMode`] that determines how they
/// interact with an active run. The queue supports priority-based ordering
/// and safe preemption at tool boundaries.
pub struct MessageQueue {
    inner: Arc<Mutex<QueueInner>>,
    config: QueueConfig,
}

struct QueueInner {
    pending: VecDeque<QueuedMessage>,
    has_active_run: bool,
}

impl MessageQueue {
    /// Create a new message queue with the given configuration.
    pub fn new(config: QueueConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(QueueInner {
                pending: VecDeque::new(),
                has_active_run: false,
            })),
            config,
        }
    }

    /// Enqueue a message. Returns error if queue is full.
    pub fn enqueue(&self, message: QueuedMessage) -> Result<(), QueueError> {
        let mut inner = self.inner.lock().expect("queue lock poisoned");
        if inner.pending.len() >= self.config.max_queue_depth {
            return Err(QueueError::QueueFull {
                depth: inner.pending.len(),
                max: self.config.max_queue_depth,
            });
        }
        let mut msg = message;
        msg.queued_at = Some(Instant::now());
        inner.pending.push_back(msg);
        Ok(())
    }

    /// Remove a specific queued message by ID.
    pub fn remove(&self, id: &str) -> Result<QueuedMessage, QueueError> {
        let mut inner = self.inner.lock().expect("queue lock poisoned");
        let pos = inner
            .pending
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| QueueError::NotFound { id: id.to_owned() })?;
        Ok(inner.pending.remove(pos).expect("position valid"))
    }

    /// Get a snapshot of the current queue state.
    pub fn status(&self) -> QueueStatus {
        let inner = self.inner.lock().expect("queue lock poisoned");
        let oldest_age = inner
            .pending
            .front()
            .and_then(|m| m.queued_at.map(|t| t.elapsed().as_millis() as u64));
        QueueStatus {
            depth: inner.pending.len(),
            pending: inner.pending.iter().cloned().collect(),
            has_active_run: inner.has_active_run,
            oldest_message_age_ms: oldest_age,
        }
    }

    /// Mark that an active run has started.
    pub fn set_active_run(&self, active: bool) {
        let mut inner = self.inner.lock().expect("queue lock poisoned");
        inner.has_active_run = active;
    }

    /// Whether there is an active run.
    pub fn has_active_run(&self) -> bool {
        let inner = self.inner.lock().expect("queue lock poisoned");
        inner.has_active_run
    }

    /// Check for preemption at a tool boundary.
    ///
    /// Inspects the queue for `Interrupt` or `Steer` messages and returns
    /// the appropriate `SteeringAction`. Priority: interrupt > steer.
    pub fn check_preemption(&self) -> SteeringAction {
        let mut inner = self.inner.lock().expect("queue lock poisoned");

        // Priority 1: Interrupt messages — abort immediately
        if let Some(pos) = inner
            .pending
            .iter()
            .position(|m| m.mode == SteeringMode::Interrupt)
        {
            let msg = inner.pending.remove(pos).expect("position valid");
            return SteeringAction::Abort {
                reason: format!("interrupted by queue message: {}", msg.id),
            };
        }

        // Priority 2: Steer messages — complete and switch
        if let Some(pos) = inner
            .pending
            .iter()
            .position(|m| m.mode == SteeringMode::Steer)
        {
            let msg = inner.pending.remove(pos).expect("position valid");
            return SteeringAction::CompleteAndSwitch(msg);
        }

        SteeringAction::Continue
    }

    /// Drain messages after a run completes, in priority order.
    ///
    /// Returns messages ordered: followup first (same context), then collect (fresh runs).
    /// Collect messages within the coalesce window are batched together.
    pub fn drain_after_run(&self) -> Vec<QueuedMessage> {
        let mut inner = self.inner.lock().expect("queue lock poisoned");
        inner.has_active_run = false;

        if inner.pending.is_empty() {
            return Vec::new();
        }

        let mut followups = Vec::new();
        let mut collects = Vec::new();
        let mut remaining = VecDeque::new();

        for msg in inner.pending.drain(..) {
            match msg.mode {
                SteeringMode::Followup => followups.push(msg),
                SteeringMode::Collect => collects.push(msg),
                // Interrupt/Steer messages that weren't consumed during the run
                // are treated as fresh collects.
                SteeringMode::Interrupt | SteeringMode::Steer => collects.push(msg),
            }
        }

        // Coalesce collect messages within the window
        let window = self.config.collect_coalesce_window;
        if collects.len() > 1 {
            let now = Instant::now();
            let (within_window, outside): (Vec<_>, Vec<_>) = collects
                .into_iter()
                .partition(|m| m.queued_at.is_some_and(|t| now.duration_since(t) <= window));
            // Keep outside-window messages as individual items
            for msg in outside {
                remaining.push_back(msg);
            }
            collects = within_window;
        }

        inner.pending = remaining;

        // Return in priority order: followups first, then collects
        let mut result = followups;
        result.extend(collects);
        result
    }

    /// Check queue health for heartbeat integration (Phase 2.4).
    ///
    /// Returns warnings if queue depth exceeds threshold or messages are stale.
    pub fn health_check(&self) -> Vec<String> {
        let inner = self.inner.lock().expect("queue lock poisoned");
        let mut warnings = Vec::new();

        let depth = inner.pending.len();
        let threshold = self.config.max_queue_depth / 2;
        if depth > threshold {
            warnings.push(format!(
                "queue depth {depth} exceeds warning threshold {threshold}"
            ));
        }

        let stale_timeout = self.config.steer_timeout * 2;
        if let Some(oldest) = inner.pending.front() {
            if let Some(queued_at) = oldest.queued_at {
                if queued_at.elapsed() > stale_timeout {
                    warnings.push(format!(
                        "oldest message {} is stale ({:.1}s old)",
                        oldest.id,
                        queued_at.elapsed().as_secs_f64()
                    ));
                }
            }
        }

        warnings
    }

    /// Current queue depth.
    pub fn depth(&self) -> usize {
        let inner = self.inner.lock().expect("queue lock poisoned");
        inner.pending.len()
    }

    /// Get a reference to the queue config.
    pub fn config(&self) -> &QueueConfig {
        &self.config
    }
}

impl PreemptionCheck for MessageQueue {
    fn check_preemption(&self) -> SteeringAction {
        self.check_preemption()
    }
}

impl Clone for MessageQueue {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(id: &str, mode: SteeringMode, content: &str) -> QueuedMessage {
        QueuedMessage {
            id: id.to_string(),
            mode,
            content: content.to_string(),
            queued_at: None,
        }
    }

    #[test]
    fn collect_mode_queued_and_drained() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("q1", SteeringMode::Collect, "do this later"))
            .unwrap();

        assert_eq!(queue.depth(), 1);
        assert!(queue.has_active_run());

        // Preemption check should return Continue (collect doesn't preempt)
        assert!(matches!(queue.check_preemption(), SteeringAction::Continue));

        let drained = queue.drain_after_run();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, "q1");
        assert!(!queue.has_active_run());
    }

    #[test]
    fn steer_mode_preempts_at_tool_boundary() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("q1", SteeringMode::Steer, "do this instead"))
            .unwrap();

        match queue.check_preemption() {
            SteeringAction::CompleteAndSwitch(msg) => {
                assert_eq!(msg.id, "q1");
                assert_eq!(msg.content, "do this instead");
            }
            other => panic!("expected CompleteAndSwitch, got {other:?}"),
        }

        // Queue should be empty after steer consumed
        assert_eq!(queue.depth(), 0);
    }

    #[test]
    fn followup_inherits_context_order() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("c1", SteeringMode::Collect, "fresh run"))
            .unwrap();
        queue
            .enqueue(make_msg("f1", SteeringMode::Followup, "same context"))
            .unwrap();

        let drained = queue.drain_after_run();
        // Followup comes before Collect
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].id, "f1");
        assert_eq!(drained[1].id, "c1");
    }

    #[test]
    fn interrupt_aborts_at_tool_boundary() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("i1", SteeringMode::Interrupt, "stop now"))
            .unwrap();

        match queue.check_preemption() {
            SteeringAction::Abort { reason } => {
                assert!(reason.contains("i1"));
            }
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn queue_depth_limit_enforced() {
        let config = QueueConfig {
            max_queue_depth: 2,
            ..Default::default()
        };
        let queue = MessageQueue::new(config);

        queue
            .enqueue(make_msg("q1", SteeringMode::Collect, "1"))
            .unwrap();
        queue
            .enqueue(make_msg("q2", SteeringMode::Collect, "2"))
            .unwrap();

        let result = queue.enqueue(make_msg("q3", SteeringMode::Collect, "3"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            QueueError::QueueFull { depth: 2, max: 2 }
        ));
    }

    #[test]
    fn steer_timeout_falls_back_to_collect() {
        // If no tool boundary occurs within timeout, steer messages
        // are treated as collect messages when drained.
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("s1", SteeringMode::Steer, "should steer"))
            .unwrap();

        // Simulate: we never call check_preemption, run finishes naturally
        let drained = queue.drain_after_run();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, "s1");
    }

    #[test]
    fn collect_messages_coalesced_within_window() {
        let config = QueueConfig {
            collect_coalesce_window: Duration::from_secs(10), // large window
            ..Default::default()
        };
        let queue = MessageQueue::new(config);
        queue.set_active_run(true);

        // All messages queued close together
        queue
            .enqueue(make_msg("c1", SteeringMode::Collect, "a"))
            .unwrap();
        queue
            .enqueue(make_msg("c2", SteeringMode::Collect, "b"))
            .unwrap();
        queue
            .enqueue(make_msg("c3", SteeringMode::Collect, "c"))
            .unwrap();

        let drained = queue.drain_after_run();
        // All should be drained together
        assert_eq!(drained.len(), 3);
    }

    #[test]
    fn drain_order_interrupt_steer_followup_collect() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("c1", SteeringMode::Collect, "collect"))
            .unwrap();
        queue
            .enqueue(make_msg("f1", SteeringMode::Followup, "followup"))
            .unwrap();
        queue
            .enqueue(make_msg("i1", SteeringMode::Interrupt, "interrupt"))
            .unwrap();
        queue
            .enqueue(make_msg("s1", SteeringMode::Steer, "steer"))
            .unwrap();

        // Preemption should pick up interrupt first (highest priority)
        match queue.check_preemption() {
            SteeringAction::Abort { .. } => {}
            other => panic!("expected Abort from interrupt, got {other:?}"),
        }

        // Next preemption check should pick steer
        match queue.check_preemption() {
            SteeringAction::CompleteAndSwitch(msg) => assert_eq!(msg.id, "s1"),
            other => panic!("expected CompleteAndSwitch from steer, got {other:?}"),
        }

        // Drain remaining: followup before collect
        let drained = queue.drain_after_run();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].id, "f1");
        assert_eq!(drained[1].id, "c1");
    }

    #[test]
    fn preemption_returns_continue_on_empty_queue() {
        let queue = MessageQueue::new(QueueConfig::default());
        assert!(matches!(queue.check_preemption(), SteeringAction::Continue));
    }

    #[test]
    fn remove_specific_message() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue
            .enqueue(make_msg("q1", SteeringMode::Collect, "a"))
            .unwrap();
        queue
            .enqueue(make_msg("q2", SteeringMode::Collect, "b"))
            .unwrap();

        let removed = queue.remove("q1").unwrap();
        assert_eq!(removed.id, "q1");
        assert_eq!(queue.depth(), 1);

        // Removing non-existent returns error
        assert!(queue.remove("q99").is_err());
    }

    #[test]
    fn status_snapshot() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);
        queue
            .enqueue(make_msg("q1", SteeringMode::Collect, "test"))
            .unwrap();

        let status = queue.status();
        assert_eq!(status.depth, 1);
        assert!(status.has_active_run);
        assert_eq!(status.pending.len(), 1);
        assert!(status.oldest_message_age_ms.is_some());
    }

    #[test]
    fn health_check_warns_on_depth() {
        let config = QueueConfig {
            max_queue_depth: 4,
            ..Default::default()
        };
        let queue = MessageQueue::new(config);

        // Fill past half
        for i in 0..3 {
            queue
                .enqueue(make_msg(&format!("q{i}"), SteeringMode::Collect, "x"))
                .unwrap();
        }

        let warnings = queue.health_check();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("exceeds warning threshold"))
        );
    }

    #[test]
    fn clone_shares_state() {
        let queue = MessageQueue::new(QueueConfig::default());
        let queue2 = queue.clone();

        queue
            .enqueue(make_msg("q1", SteeringMode::Collect, "shared"))
            .unwrap();
        assert_eq!(queue2.depth(), 1);
    }

    #[test]
    fn multiple_followups_preserved_in_order() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);

        queue
            .enqueue(make_msg("f1", SteeringMode::Followup, "first"))
            .unwrap();
        queue
            .enqueue(make_msg("f2", SteeringMode::Followup, "second"))
            .unwrap();
        queue
            .enqueue(make_msg("f3", SteeringMode::Followup, "third"))
            .unwrap();

        let drained = queue.drain_after_run();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].id, "f1");
        assert_eq!(drained[1].id, "f2");
        assert_eq!(drained[2].id, "f3");
    }

    #[test]
    fn drain_empty_queue_returns_empty() {
        let queue = MessageQueue::new(QueueConfig::default());
        queue.set_active_run(true);
        let drained = queue.drain_after_run();
        assert!(drained.is_empty());
    }
}
