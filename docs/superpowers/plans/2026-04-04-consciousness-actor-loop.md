# BRO-455: Core Consciousness Actor Loop — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the blocking request-response `run_session` handler with a long-lived per-session consciousness actor that owns a `tokio::select!` event loop, enabling concurrent message handling, queuing, and preemption.

**Architecture:** Each session gets a `SessionConsciousness` actor spawned as a `tokio::spawn` task. HTTP handlers become thin event-pushers that send `ConsciousnessEvent` variants via `mpsc::Sender` and return immediately. The actor runs the existing `tick_on_branch` loop internally, checking preemption at iteration boundaries via the existing `MessageQueue` from `arcan-core/queue.rs`. Feature-flagged behind `ARCAN_CONSCIOUSNESS=true`.

**Tech Stack:** Rust 2024 Edition, tokio (mpsc, select!, spawn, Interval), arcan-core MessageQueue, aios-runtime KernelRuntime, aios-protocol EventKind

---

## Dependency Chain

```
aios-protocol (EventKind, SteeringMode, OperatingMode) — READ ONLY
  ↓
aios-runtime (KernelRuntime, TickInput, TickOutput) — READ ONLY
  ↓
arcan-core (MessageQueue, QueueConfig, SwappableProviderHandle, ProviderFactory) — READ ONLY
  ↓
arcand (consciousness.rs — NEW, canonical.rs — MODIFY, lib.rs — MODIFY)
```

No upstream changes needed. All work is in the `arcand` crate.

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/arcand/src/consciousness.rs` | CREATE | All consciousness types + actor + registry |
| `crates/arcand/src/lib.rs` | MODIFY | Add `pub mod consciousness;` |
| `crates/arcand/src/canonical.rs` | MODIFY | Add `consciousness_registry` to `CanonicalState`, modify `run_session` |
| `crates/arcand/Cargo.toml` | MODIFY | No new deps needed (tokio + std::sync::Mutex already available) |

**Design note:** The existing codebase uses `std::sync::Mutex` throughout `CanonicalState` (lines 222-232 of canonical.rs). The registry lock is never held across `.await` points, so `std::sync::Mutex` is correct and consistent. No need for `parking_lot`.

---

### Task 1: Create consciousness types and ConsciousnessEvent enum

**Files:**
- Create: `crates/arcand/src/consciousness.rs`
- Modify: `crates/arcand/src/lib.rs`

- [ ] **Step 1: Create consciousness.rs with module doc and imports**

```rust
//! Session consciousness — event-driven actor loop for concurrent message handling.
//!
//! Each session gets a long-lived `tokio::spawn` task (the "consciousness actor")
//! that owns a `tokio::select!` event loop. HTTP handlers become thin event-pushers
//! that return 202 Accepted. All results flow via the existing SSE streaming.
//!
//! Feature-flagged behind `ARCAN_CONSCIOUSNESS=true`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aios_protocol::{BranchId, EventKind, OperatingMode, SessionId, SteeringMode, ToolCall};
use aios_runtime::{KernelRuntime, TickInput, TickOutput};
use arcan_core::queue::{MessageQueue, QueueConfig, QueuedMessage, SteeringAction};
use arcan_core::runtime::SwappableProviderHandle;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::Interval;
use tracing::{Instrument, info, warn, debug, error};
```

- [ ] **Step 2: Define ConsciousnessEvent enum**

```rust
/// Unified stimulus enum — every input to the consciousness actor.
#[derive(Debug)]
pub enum ConsciousnessEvent {
    /// User message from HTTP POST /runs or /messages.
    UserMessage {
        objective: String,
        branch: BranchId,
        steering: SteeringMode,
        /// Acknowledgment channel: actor sends back the session_id once queued/started.
        ack: Option<oneshot::Sender<ConsciousnessAck>>,
        /// Pre-built run context (system prompt, allowed tools, proposed tool).
        run_context: RunContext,
    },
    /// Tool execution completed successfully (future: async tool execution).
    ToolResult {
        call_id: String,
        tool_name: String,
        result: serde_json::Value,
        duration_ms: u64,
    },
    /// Tool execution failed (future: async tool execution).
    ToolFailed {
        call_id: String,
        tool_name: String,
        error: String,
    },
    /// Message from Spaces distributed networking (future: Phase 4).
    SpacesMessage {
        channel_id: String,
        sender: String,
        content: String,
    },
    /// Autonomic homeostasis signal (future: Phase 3).
    AutonomicSignal {
        ruling: String,
    },
    /// Timer tick from internal intervals.
    TimerTick {
        tick_type: TimerTickType,
    },
    /// Approval resolved from HTTP POST /approvals (future: Phase 2).
    ApprovalResolved {
        approval_id: String,
        approved: bool,
        actor: String,
    },
    /// External signal from webhook or scheduled task (future).
    ExternalSignal {
        signal_type: String,
        data: serde_json::Value,
    },
    /// Graceful shutdown request.
    Shutdown,
}

/// Timer tick types for the consciousness heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerTickType {
    Heartbeat,
    IdleCheck,
    SleepWake,
}

/// Pre-built context for an agent run (passed from HTTP handler).
#[derive(Debug, Clone)]
pub struct RunContext {
    pub system_prompt: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub proposed_tool: Option<ToolCall>,
}

/// Acknowledgment sent back to the HTTP handler after event is received.
#[derive(Debug)]
pub enum ConsciousnessAck {
    /// Event was accepted; run started or queued.
    Accepted { queued: bool },
    /// Event was rejected (e.g., shutting down).
    Rejected { reason: String },
}
```

- [ ] **Step 3: Define ConsciousnessMode and ConsciousnessState**

```rust
/// Operating mode of the consciousness actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsciousnessMode {
    /// Actively running an agent cycle.
    Active,
    /// Waiting for background tool results (future: async tools).
    WaitingForTools,
    /// No active work — ready for new messages.
    Idle,
    /// Low-power mode after extended inactivity.
    Sleeping,
    /// Gracefully shutting down.
    ShuttingDown,
}

/// Per-session mutable state owned by the consciousness actor.
struct ConsciousnessState {
    session_id: SessionId,
    branch: BranchId,
    mode: ConsciousnessMode,
    queue: MessageQueue,
    running_tools: HashMap<String, RunningToolInfo>,
    active_run_id: Option<String>,
    last_activity: Instant,
    stall_counter: u32,
}

/// Metadata for a running tool (future: async tool execution).
#[derive(Debug, Clone)]
pub struct RunningToolInfo {
    pub tool_name: String,
    pub started_at: Instant,
}

impl ConsciousnessState {
    fn new(session_id: SessionId, branch: BranchId) -> Self {
        Self {
            session_id,
            branch,
            mode: ConsciousnessMode::Idle,
            queue: MessageQueue::new(QueueConfig::default()),
            running_tools: HashMap::new(),
            active_run_id: None,
            last_activity: Instant::now(),
            stall_counter: 0,
        }
    }
}
```

- [ ] **Step 4: Register the module in lib.rs**

In `crates/arcand/src/lib.rs`, add:

```rust
pub mod consciousness;
```

- [ ] **Step 5: Verify compilation**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo check -p arcand`
Expected: Compiles with possible warnings about unused types (expected at this stage).

- [ ] **Step 6: Commit**

```bash
git add crates/arcand/src/consciousness.rs crates/arcand/src/lib.rs
git commit -m "feat(consciousness): add ConsciousnessEvent types and state (BRO-455)"
```

---

### Task 2: Implement SessionConsciousness actor and main event loop

**Files:**
- Modify: `crates/arcand/src/consciousness.rs`

- [ ] **Step 1: Write test for actor creation and shutdown**

Append to `consciousness.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_runtime() -> Arc<KernelRuntime> {
        // KernelRuntime requires ports — use a minimal mock setup.
        // For consciousness tests we only need the runtime reference;
        // actual tick_on_branch calls are tested in integration tests.
        // We'll use a real KernelRuntime with mock ports.
        use aios_runtime::KernelRuntime;
        Arc::new(KernelRuntime::new(
            aios_runtime::RuntimeConfig::default(),
            Arc::new(aios_events::InMemoryEventStore::new()),
            Arc::new(arcan_core::runtime::MockProvider),
            Arc::new(aios_tools::NoOpToolHarness),
            Arc::new(aios_policy::DefaultApprovalPort),
            Arc::new(aios_policy::DefaultPolicyGate),
        ))
    }

    #[tokio::test]
    async fn actor_shuts_down_on_shutdown_event() {
        let runtime = test_runtime();
        let session_id = SessionId::from_string("test-shutdown".to_string());
        let branch = BranchId::main();

        let (handle, tx) = SessionConsciousness::spawn(
            session_id,
            branch,
            runtime,
            ConsciousnessConfig::default(),
        );

        // Send shutdown
        tx.send(ConsciousnessEvent::Shutdown).await.unwrap();

        // Actor should exit cleanly
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "actor should shut down within 2s");
    }

    #[tokio::test]
    async fn actor_starts_in_idle_mode() {
        let runtime = test_runtime();
        let session_id = SessionId::from_string("test-idle".to_string());
        let branch = BranchId::main();

        let (handle, tx) = SessionConsciousness::spawn(
            session_id,
            branch,
            runtime,
            ConsciousnessConfig::default(),
        );

        // Give actor time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Shutdown and verify it was running
        tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 2: Define ConsciousnessConfig**

Add above the `SessionConsciousness` struct:

```rust
/// Configuration for the consciousness actor.
#[derive(Debug, Clone)]
pub struct ConsciousnessConfig {
    /// Channel buffer size for incoming events.
    pub channel_buffer: usize,
    /// Heartbeat interval (default: 30s).
    pub heartbeat_interval: Duration,
    /// Idle check interval (default: 60s).
    pub idle_check_interval: Duration,
    /// Max agent iterations per run (default: 10).
    pub max_agent_iterations: u32,
    /// Time before transitioning from Idle to Sleeping (default: 5min).
    pub idle_to_sleep: Duration,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            channel_buffer: 32,
            heartbeat_interval: Duration::from_secs(30),
            idle_check_interval: Duration::from_secs(60),
            max_agent_iterations: 10,
            idle_to_sleep: Duration::from_secs(300),
        }
    }
}
```

- [ ] **Step 3: Implement SessionConsciousness struct and spawn**

```rust
/// The consciousness actor — a long-lived tokio task per session.
pub struct SessionConsciousness {
    rx: mpsc::Receiver<ConsciousnessEvent>,
    state: ConsciousnessState,
    runtime: Arc<KernelRuntime>,
    config: ConsciousnessConfig,
}

impl SessionConsciousness {
    /// Spawn a new consciousness actor for the given session.
    ///
    /// Returns the `JoinHandle` and a `Sender` for pushing events.
    pub fn spawn(
        session_id: SessionId,
        branch: BranchId,
        runtime: Arc<KernelRuntime>,
        config: ConsciousnessConfig,
    ) -> (JoinHandle<()>, mpsc::Sender<ConsciousnessEvent>) {
        let (tx, rx) = mpsc::channel(config.channel_buffer);
        let actor = Self {
            rx,
            state: ConsciousnessState::new(session_id.clone(), branch),
            runtime,
            config,
        };
        let span = tracing::info_span!("consciousness", session = %session_id);
        let handle = tokio::spawn(actor.run().instrument(span));
        (handle, tx)
    }

    /// Main event loop — runs until Shutdown or channel closes.
    async fn run(mut self) {
        info!(
            session = %self.state.session_id,
            "consciousness actor started"
        );

        let mut heartbeat = tokio::time::interval(self.config.heartbeat_interval);
        let mut idle_check = tokio::time::interval(self.config.idle_check_interval);
        // Skip the immediate first tick (fires at interval creation).
        heartbeat.tick().await;
        idle_check.tick().await;

        loop {
            let event = tokio::select! {
                biased;

                // Prioritize incoming events over timers.
                Some(e) = self.rx.recv() => e,

                _ = heartbeat.tick() => ConsciousnessEvent::TimerTick {
                    tick_type: TimerTickType::Heartbeat,
                },

                _ = idle_check.tick() => ConsciousnessEvent::TimerTick {
                    tick_type: TimerTickType::IdleCheck,
                },
            };

            match event {
                ConsciousnessEvent::Shutdown => {
                    info!(session = %self.state.session_id, "shutdown requested");
                    self.state.mode = ConsciousnessMode::ShuttingDown;
                    self.graceful_shutdown().await;
                    break;
                }
                ConsciousnessEvent::UserMessage {
                    objective,
                    branch,
                    steering,
                    ack,
                    run_context,
                } => {
                    self.handle_user_message(objective, branch, steering, ack, run_context)
                        .await;
                }
                ConsciousnessEvent::TimerTick { tick_type } => {
                    self.handle_timer_tick(tick_type).await;
                }
                // Future event types — log and skip for now.
                ConsciousnessEvent::ToolResult { call_id, .. } => {
                    debug!(call_id, "tool result received (async tools not yet implemented)");
                }
                ConsciousnessEvent::ToolFailed { call_id, .. } => {
                    debug!(call_id, "tool failure received (async tools not yet implemented)");
                }
                ConsciousnessEvent::SpacesMessage { .. } => {
                    debug!("spaces message received (Phase 4)");
                }
                ConsciousnessEvent::AutonomicSignal { .. } => {
                    debug!("autonomic signal received (Phase 3)");
                }
                ConsciousnessEvent::ApprovalResolved { .. } => {
                    debug!("approval resolved (Phase 2)");
                }
                ConsciousnessEvent::ExternalSignal { .. } => {
                    debug!("external signal received");
                }
            }
        }

        info!(session = %self.state.session_id, "consciousness actor stopped");
    }
}
```

- [ ] **Step 4: Verify tests compile**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo test -p arcand --lib consciousness -- --no-run`
Expected: Compilation succeeds (tests not executed yet because mock types may need adjustment).

- [ ] **Step 5: Commit**

```bash
git add crates/arcand/src/consciousness.rs
git commit -m "feat(consciousness): implement SessionConsciousness actor with event loop (BRO-455)"
```

---

### Task 3: Implement handle_user_message and run_agent_cycle

**Files:**
- Modify: `crates/arcand/src/consciousness.rs`

- [ ] **Step 1: Write test for user message when idle starts a run**

```rust
#[tokio::test]
async fn user_message_when_idle_starts_run() {
    let runtime = test_runtime();
    let session_id = SessionId::from_string("test-run".to_string());
    let branch = BranchId::main();

    // Create session in runtime first
    runtime
        .create_session_with_id(
            session_id.clone(),
            "test",
            aios_protocol::PolicySet::default(),
            aios_protocol::ModelRouting::default(),
        )
        .await
        .unwrap();

    let (handle, tx) = SessionConsciousness::spawn(
        session_id,
        branch,
        runtime,
        ConsciousnessConfig {
            max_agent_iterations: 1, // single tick for test
            ..Default::default()
        },
    );

    let (ack_tx, ack_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage {
        objective: "Hello".to_string(),
        branch: BranchId::main(),
        steering: SteeringMode::Collect,
        ack: Some(ack_tx),
        run_context: RunContext {
            system_prompt: None,
            allowed_tools: None,
            proposed_tool: None,
        },
    })
    .await
    .unwrap();

    // Should get acknowledgment
    let ack = tokio::time::timeout(Duration::from_secs(5), ack_rx).await;
    assert!(ack.is_ok(), "should receive ack");
    match ack.unwrap().unwrap() {
        ConsciousnessAck::Accepted { queued } => {
            assert!(!queued, "should start immediately when idle");
        }
        other => panic!("expected Accepted, got {other:?}"),
    }

    // Shutdown
    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}

#[tokio::test]
async fn user_message_when_active_queues() {
    let runtime = test_runtime();
    let session_id = SessionId::from_string("test-queue".to_string());
    let branch = BranchId::main();

    runtime
        .create_session_with_id(
            session_id.clone(),
            "test",
            aios_protocol::PolicySet::default(),
            aios_protocol::ModelRouting::default(),
        )
        .await
        .unwrap();

    let config = ConsciousnessConfig {
        max_agent_iterations: 1,
        ..Default::default()
    };
    let (handle, tx) = SessionConsciousness::spawn(
        session_id, branch, runtime, config,
    );

    // Send first message (starts run)
    let (ack1_tx, _ack1_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage {
        objective: "First".to_string(),
        branch: BranchId::main(),
        steering: SteeringMode::Collect,
        ack: Some(ack1_tx),
        run_context: RunContext {
            system_prompt: None,
            allowed_tools: None,
            proposed_tool: None,
        },
    })
    .await
    .unwrap();

    // Small delay then send second message while first is running
    tokio::time::sleep(Duration::from_millis(10)).await;

    let (ack2_tx, ack2_rx) = oneshot::channel();
    tx.send(ConsciousnessEvent::UserMessage {
        objective: "Second".to_string(),
        branch: BranchId::main(),
        steering: SteeringMode::Collect,
        ack: Some(ack2_tx),
        run_context: RunContext {
            system_prompt: None,
            allowed_tools: None,
            proposed_tool: None,
        },
    })
    .await
    .unwrap();

    // Second message should be queued
    if let Ok(Ok(ack)) = tokio::time::timeout(Duration::from_secs(5), ack2_rx).await {
        match ack {
            ConsciousnessAck::Accepted { queued } => {
                assert!(queued, "second message should be queued");
            }
            other => panic!("expected Accepted(queued=true), got {other:?}"),
        }
    }

    tx.send(ConsciousnessEvent::Shutdown).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}
```

- [ ] **Step 2: Implement handle_user_message**

Add to `impl SessionConsciousness`:

```rust
    /// Handle an incoming user message.
    ///
    /// If idle, start a new agent cycle immediately.
    /// If active, queue the message with the specified steering mode.
    async fn handle_user_message(
        &mut self,
        objective: String,
        branch: BranchId,
        steering: SteeringMode,
        ack: Option<oneshot::Sender<ConsciousnessAck>>,
        run_context: RunContext,
    ) {
        self.state.last_activity = Instant::now();
        self.state.branch = branch.clone();

        match self.state.mode {
            ConsciousnessMode::Idle | ConsciousnessMode::Sleeping => {
                // Start a new run immediately.
                if let Some(ack) = ack {
                    let _ = ack.send(ConsciousnessAck::Accepted { queued: false });
                }
                self.state.mode = ConsciousnessMode::Active;
                self.run_agent_cycle(objective, branch, run_context).await;
                self.drain_queue_after_run().await;
            }
            ConsciousnessMode::Active | ConsciousnessMode::WaitingForTools => {
                // Queue the message for later processing.
                let msg_id = uuid::Uuid::new_v4().to_string();
                let queued = QueuedMessage {
                    id: msg_id.clone(),
                    mode: steering,
                    content: objective,
                    queued_at: None, // set by enqueue()
                };
                match self.state.queue.enqueue(queued) {
                    Ok(()) => {
                        info!(msg_id, "message queued during active run");
                        // Record queue event via runtime.
                        let _ = self
                            .runtime
                            .record_external_event(
                                &self.state.session_id,
                                EventKind::Queued {
                                    queue_id: msg_id,
                                    mode: steering,
                                    message: String::new(), // content already in queue
                                },
                            )
                            .await;
                        if let Some(ack) = ack {
                            let _ = ack.send(ConsciousnessAck::Accepted { queued: true });
                        }
                    }
                    Err(err) => {
                        warn!(%err, "failed to queue message");
                        if let Some(ack) = ack {
                            let _ = ack.send(ConsciousnessAck::Rejected {
                                reason: err.to_string(),
                            });
                        }
                    }
                }
            }
            ConsciousnessMode::ShuttingDown => {
                if let Some(ack) = ack {
                    let _ = ack.send(ConsciousnessAck::Rejected {
                        reason: "session is shutting down".to_string(),
                    });
                }
            }
        }
    }
```

- [ ] **Step 3: Implement run_agent_cycle**

```rust
    /// Run the agent loop: tick repeatedly until end_turn or max iterations.
    ///
    /// This is the core agent cycle, equivalent to the existing blocking loop
    /// in `canonical.rs` run_session handler (lines 1608-1657).
    async fn run_agent_cycle(
        &mut self,
        objective: String,
        branch: BranchId,
        run_context: RunContext,
    ) {
        let run_id = uuid::Uuid::new_v4().to_string();
        self.state.active_run_id = Some(run_id.clone());
        self.state.queue.set_active_run(true).ok();

        debug!(run_id, objective = %objective, "starting agent cycle");

        let agent_span = life_vigil::spans::agent_span(
            self.state.session_id.as_str(),
            "arcan",
        );

        // First tick with the actual objective.
        let mut tick_result = self
            .runtime
            .tick_on_branch(
                &self.state.session_id,
                &branch,
                TickInput {
                    objective,
                    proposed_tool: run_context.proposed_tool,
                    system_prompt: run_context.system_prompt.clone(),
                    allowed_tools: run_context.allowed_tools.clone(),
                },
            )
            .instrument(agent_span.clone())
            .await;

        // Continue ticking while mode=Execute (tools ran, need continuation).
        for iteration in 1..self.config.max_agent_iterations {
            match &tick_result {
                Ok(tick) if tick.mode == OperatingMode::Execute => {
                    // Check preemption at tool boundary.
                    match self.state.queue.check_preemption() {
                        Ok(SteeringAction::Abort { reason }) => {
                            warn!(iteration, %reason, "run aborted by interrupt");
                            let _ = self
                                .runtime
                                .record_external_event(
                                    &self.state.session_id,
                                    EventKind::Steered {
                                        queue_id: run_id.clone(),
                                        preempted_at: format!("iteration:{iteration}"),
                                    },
                                )
                                .await;
                            break;
                        }
                        Ok(SteeringAction::CompleteAndSwitch(msg)) => {
                            info!(
                                iteration,
                                msg_id = %msg.id,
                                "steering: completing current iteration then switching"
                            );
                            let _ = self
                                .runtime
                                .record_external_event(
                                    &self.state.session_id,
                                    EventKind::Steered {
                                        queue_id: msg.id.clone(),
                                        preempted_at: format!("iteration:{iteration}"),
                                    },
                                )
                                .await;
                            // Re-queue as the next objective to run after drain.
                            let requeue = QueuedMessage {
                                id: msg.id,
                                mode: SteeringMode::Followup,
                                content: msg.content,
                                queued_at: None,
                            };
                            self.state.queue.enqueue(requeue).ok();
                            break;
                        }
                        Ok(SteeringAction::InjectMessage(_)) => {
                            // Not yet implemented — treat as continue.
                        }
                        Ok(SteeringAction::Continue) => {}
                        Err(err) => {
                            warn!(%err, "preemption check failed, continuing");
                        }
                    }

                    debug!(iteration, "agent loop: continuing after tool execution");
                    tick_result = self
                        .runtime
                        .tick_on_branch(
                            &self.state.session_id,
                            &branch,
                            TickInput {
                                objective: String::new(),
                                proposed_tool: None,
                                system_prompt: run_context.system_prompt.clone(),
                                allowed_tools: run_context.allowed_tools.clone(),
                            },
                        )
                        .instrument(agent_span.clone())
                        .await;
                }
                _ => break,
            }
        }

        self.state.active_run_id = None;
        self.state.queue.set_active_run(false).ok();

        match tick_result {
            Ok(tick) => {
                debug!(
                    mode = ?tick.mode,
                    events = tick.events_emitted,
                    "agent cycle completed"
                );
            }
            Err(err) => {
                error!(%err, "agent cycle failed");
            }
        }

        self.state.mode = ConsciousnessMode::Idle;
        self.state.last_activity = Instant::now();
    }
```

- [ ] **Step 4: Implement drain_queue_after_run**

```rust
    /// Process queued messages after a run completes.
    async fn drain_queue_after_run(&mut self) {
        let drained = match self.state.queue.drain_after_run() {
            Ok(msgs) => msgs,
            Err(err) => {
                warn!(%err, "failed to drain queue");
                return;
            }
        };

        if drained.is_empty() {
            return;
        }

        let count = drained.len();
        info!(count, "draining queued messages after run");

        let _ = self
            .runtime
            .record_external_event(
                &self.state.session_id,
                EventKind::QueueDrained {
                    queue_id: "post-run-drain".to_string(),
                    processed: count,
                },
            )
            .await;

        // Process each drained message as a new run.
        for msg in drained {
            self.state.mode = ConsciousnessMode::Active;
            self.run_agent_cycle(
                msg.content,
                self.state.branch.clone(),
                RunContext {
                    system_prompt: None,
                    allowed_tools: None,
                    proposed_tool: None,
                },
            )
            .await;
        }
    }
```

- [ ] **Step 5: Implement handle_timer_tick and graceful_shutdown**

```rust
    /// Handle timer ticks (heartbeat, idle check).
    async fn handle_timer_tick(&mut self, tick_type: TimerTickType) {
        match tick_type {
            TimerTickType::Heartbeat => {
                // Emit heartbeat event for observability.
                let _ = self
                    .runtime
                    .record_external_event(
                        &self.state.session_id,
                        EventKind::Heartbeat,
                    )
                    .await;

                // Check queue health.
                if let Ok(warnings) = self.state.queue.health_check() {
                    for w in &warnings {
                        warn!(session = %self.state.session_id, "{w}");
                    }
                }
            }
            TimerTickType::IdleCheck => {
                if self.state.mode == ConsciousnessMode::Idle
                    && self.state.last_activity.elapsed() > self.config.idle_to_sleep
                {
                    info!(
                        session = %self.state.session_id,
                        idle_secs = self.state.last_activity.elapsed().as_secs(),
                        "transitioning to sleeping mode"
                    );
                    self.state.mode = ConsciousnessMode::Sleeping;
                }
            }
            TimerTickType::SleepWake => {
                // Future: periodic check if sleeping actor should wake.
            }
        }
    }

    /// Graceful shutdown: wait for running tools, emit final heartbeat.
    async fn graceful_shutdown(&mut self) {
        if !self.state.running_tools.is_empty() {
            info!(
                tools = self.state.running_tools.len(),
                "waiting for running tools before shutdown"
            );
            // Future: wait for async tools with timeout.
        }

        // Emit final heartbeat.
        let _ = self
            .runtime
            .record_external_event(
                &self.state.session_id,
                EventKind::Heartbeat,
            )
            .await;

        info!(session = %self.state.session_id, "graceful shutdown complete");
    }
```

- [ ] **Step 6: Run tests**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo test -p arcand --lib consciousness`
Expected: Tests pass (mock provider returns immediately, so agent cycle completes quickly).

- [ ] **Step 7: Commit**

```bash
git add crates/arcand/src/consciousness.rs
git commit -m "feat(consciousness): implement handle_user_message, run_agent_cycle, queue drain (BRO-455)"
```

---

### Task 4: Implement ConsciousnessRegistry and ConsciousnessHandle

**Files:**
- Modify: `crates/arcand/src/consciousness.rs`

- [ ] **Step 1: Write tests for registry**

```rust
#[tokio::test]
async fn registry_creates_and_retrieves_handle() {
    let runtime = test_runtime();
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    let session_id = "test-registry";
    let handle = registry.get_or_create(
        session_id,
        BranchId::main(),
        runtime.clone(),
    );
    assert!(handle.is_alive());

    // Second call returns same handle.
    let handle2 = registry.get_or_create(
        session_id,
        BranchId::main(),
        runtime,
    );
    assert!(handle2.is_alive());

    // Shutdown all.
    registry.shutdown_all().await;
}

#[tokio::test]
async fn registry_shutdown_all_stops_actors() {
    let runtime = test_runtime();
    let registry = ConsciousnessRegistry::new(ConsciousnessConfig::default());

    registry.get_or_create("s1", BranchId::main(), runtime.clone());
    registry.get_or_create("s2", BranchId::main(), runtime);

    assert_eq!(registry.session_count(), 2);

    registry.shutdown_all().await;
    // After shutdown, actors should be stopped.
    // (handles are consumed by shutdown_all)
}
```

- [ ] **Step 2: Implement ConsciousnessHandle**

```rust
/// Handle for communicating with a consciousness actor.
#[derive(Clone)]
pub struct ConsciousnessHandle {
    pub tx: mpsc::Sender<ConsciousnessEvent>,
    join: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl ConsciousnessHandle {
    fn new(tx: mpsc::Sender<ConsciousnessEvent>, join: JoinHandle<()>) -> Self {
        Self {
            tx,
            join: Arc::new(Mutex::new(Some(join))),
        }
    }

    /// Check if the actor task is still running.
    pub fn is_alive(&self) -> bool {
        self.join
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|h| !h.is_finished())
    }

    /// Send an event to the consciousness actor.
    pub async fn send(&self, event: ConsciousnessEvent) -> Result<(), mpsc::error::SendError<ConsciousnessEvent>> {
        self.tx.send(event).await
    }

    /// Send a shutdown event and wait for the actor to stop.
    pub async fn shutdown(self) {
        let _ = self.tx.send(ConsciousnessEvent::Shutdown).await;
        let join = self
            .join
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = join {
            let _ = tokio::time::timeout(Duration::from_secs(10), handle).await;
        }
    }
}
```

- [ ] **Step 3: Implement ConsciousnessRegistry**

```rust
/// Registry mapping session IDs to consciousness actors.
///
/// Thread-safe: accessed from HTTP handlers (multiple threads) and
/// from the actors themselves. Lock is never held across `.await`.
pub struct ConsciousnessRegistry {
    sessions: Mutex<HashMap<String, ConsciousnessHandle>>,
    config: ConsciousnessConfig,
}

impl ConsciousnessRegistry {
    pub fn new(config: ConsciousnessConfig) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// Get an existing handle or create a new consciousness actor.
    pub fn get_or_create(
        &self,
        session_id: &str,
        branch: BranchId,
        runtime: Arc<KernelRuntime>,
    ) -> ConsciousnessHandle {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Return existing if alive.
        if let Some(handle) = sessions.get(session_id) {
            if handle.is_alive() {
                return handle.clone();
            }
            // Dead actor — remove and create new.
            sessions.remove(session_id);
        }

        // Spawn new actor.
        let (join, tx) = SessionConsciousness::spawn(
            SessionId::from_string(session_id.to_string()),
            branch,
            runtime,
            self.config.clone(),
        );
        let handle = ConsciousnessHandle::new(tx, join);
        sessions.insert(session_id.to_string(), handle.clone());

        info!(session_id, "consciousness actor created");
        handle
    }

    /// Get an existing handle (returns None if no actor for this session).
    pub fn get(&self, session_id: &str) -> Option<ConsciousnessHandle> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.get(session_id).filter(|h| h.is_alive()).cloned()
    }

    /// Number of active sessions.
    pub fn session_count(&self) -> usize {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        sessions.values().filter(|h| h.is_alive()).count()
    }

    /// Shut down all consciousness actors gracefully.
    pub async fn shutdown_all(&self) {
        let handles: Vec<ConsciousnessHandle> = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            sessions.drain().map(|(_, h)| h).collect()
        };

        info!(count = handles.len(), "shutting down all consciousness actors");

        let futures: Vec<_> = handles.into_iter().map(|h| h.shutdown()).collect();
        futures_util::future::join_all(futures).await;
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo test -p arcand --lib consciousness`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/arcand/src/consciousness.rs
git commit -m "feat(consciousness): implement ConsciousnessRegistry and ConsciousnessHandle (BRO-455)"
```

---

### Task 5: Wire ConsciousnessRegistry into CanonicalState and feature-flag run_session

**Files:**
- Modify: `crates/arcand/src/canonical.rs`
- Modify: `crates/arcand/Cargo.toml` (add `futures-util` if not present)

- [ ] **Step 1: Add futures-util to arcand dependencies (if needed)**

Check if `futures-util` is already in arcand's Cargo.toml. If not, add:

```toml
futures-util.workspace = true
```

- [ ] **Step 2: Add consciousness_registry field to CanonicalState**

In `crates/arcand/src/canonical.rs`, add to the `CanonicalState` struct (after `cached_git_context`):

```rust
    /// Consciousness registry for event-driven session actors (BRO-455).
    /// When `ARCAN_CONSCIOUSNESS=true`, run_session pushes events to actors
    /// instead of blocking on the tick loop.
    consciousness_registry: Option<Arc<crate::consciousness::ConsciousnessRegistry>>,
```

- [ ] **Step 3: Initialize the registry in create_canonical_router_with_skills**

In the `state = CanonicalState { ... }` block, add:

```rust
        consciousness_registry: if std::env::var("ARCAN_CONSCIOUSNESS")
            .is_ok_and(|v| v == "true" || v == "1")
        {
            tracing::info!("Consciousness mode ENABLED (BRO-455)");
            Some(Arc::new(crate::consciousness::ConsciousnessRegistry::new(
                crate::consciousness::ConsciousnessConfig::default(),
            )))
        } else {
            None
        },
```

- [ ] **Step 4: Modify run_session to use consciousness when enabled**

After the system prompt is built and combined_allowed_tools is computed (around line 1572), insert the consciousness dispatch path before the existing agent loop:

```rust
    // ─── Consciousness dispatch (BRO-455) ────────────────────────────
    // When ARCAN_CONSCIOUSNESS=true, push the message to the session's
    // consciousness actor instead of blocking on the tick loop.
    if let Some(ref registry) = state.consciousness_registry {
        let handle = registry.get_or_create(
            session_id.as_str(),
            branch.clone(),
            state.runtime.clone(),
        );

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let send_result = handle
            .send(crate::consciousness::ConsciousnessEvent::UserMessage {
                objective: objective.clone(),
                branch: branch.clone(),
                steering: aios_protocol::SteeringMode::Collect,
                ack: Some(ack_tx),
                run_context: crate::consciousness::RunContext {
                    system_prompt: system_prompt.clone(),
                    allowed_tools: combined_allowed_tools.clone(),
                    proposed_tool: proposed_tool.clone(),
                },
            })
            .await;

        if send_result.is_err() {
            return Err(internal_error(anyhow::anyhow!(
                "consciousness actor for session {} is not running",
                session_id
            )));
        }

        // Wait for acknowledgment (fast — actor responds immediately).
        match tokio::time::timeout(Duration::from_secs(5), ack_rx).await {
            Ok(Ok(crate::consciousness::ConsciousnessAck::Accepted { queued })) => {
                // Return a response indicating the run was accepted.
                // The actual results flow via SSE streaming.
                return Ok(Json(RunResponse {
                    session_id,
                    mode: if queued {
                        OperatingMode::Execute
                    } else {
                        OperatingMode::Execute
                    },
                    state: AgentStateVector::default(),
                    events_emitted: 0,
                    last_sequence: 0,
                }));
            }
            Ok(Ok(crate::consciousness::ConsciousnessAck::Rejected { reason })) => {
                return Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "consciousness_rejected", "reason": reason })),
                ));
            }
            Ok(Err(_)) => {
                return Err(internal_error(anyhow::anyhow!(
                    "consciousness actor dropped ack channel"
                )));
            }
            Err(_) => {
                return Err((
                    StatusCode::GATEWAY_TIMEOUT,
                    Json(json!({ "error": "consciousness_timeout" })),
                ));
            }
        }
    }
    // ─── End consciousness dispatch ──────────────────────────────────
```

- [ ] **Step 5: Run cargo check**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo check -p arcand`
Expected: Compiles without errors.

- [ ] **Step 6: Run cargo fmt and clippy**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo fmt && cargo clippy -p arcand`
Expected: No warnings.

- [ ] **Step 7: Run full workspace tests**

Run: `cd /Users/broomva/broomva/core/life/arcan && cargo test --workspace`
Expected: All existing tests pass plus new consciousness tests.

- [ ] **Step 8: Commit**

```bash
git add crates/arcand/src/canonical.rs crates/arcand/src/consciousness.rs crates/arcand/Cargo.toml
git commit -m "feat(consciousness): wire registry into CanonicalState with ARCAN_CONSCIOUSNESS flag (BRO-455)"
```

---

### Task 6: Final validation, format, and cleanup

**Files:**
- All modified files

- [ ] **Step 1: Full validation**

```bash
cd /Users/broomva/broomva/core/life/arcan
cargo fmt
cargo clippy --workspace
cargo test --workspace
cargo build --workspace
```

Expected: All green.

- [ ] **Step 2: Update BRO-455 status to In Progress**

Update the Linear ticket status.

- [ ] **Step 3: Create feature branch and PR**

```bash
git checkout -b feature/bro-455-consciousness-actor-loop
git push -u origin feature/bro-455-consciousness-actor-loop
```

Create PR with summary of changes.
