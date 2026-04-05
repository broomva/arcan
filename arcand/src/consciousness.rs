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

use aios_protocol::{BranchId, EventKind, OperatingMode, SessionId, SteeringMode};
use aios_runtime::{KernelRuntime, TickInput};
use arcan_core::queue::{MessageQueue, QueueConfig, QueuedMessage, SteeringAction};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{Instrument, debug, error, info, warn};

// ─── Configuration ──────────────────────────────────────────────────────────

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
    /// Whether Spaces integration is enabled (default: false).
    pub spaces_enabled: bool,
    /// How often to poll Spaces for new messages (default: 10s).
    pub spaces_poll_interval: Duration,
}

impl Default for ConsciousnessConfig {
    fn default() -> Self {
        Self {
            channel_buffer: 32,
            heartbeat_interval: Duration::from_secs(30),
            idle_check_interval: Duration::from_secs(60),
            max_agent_iterations: 10,
            idle_to_sleep: Duration::from_secs(300),
            spaces_enabled: false,
            spaces_poll_interval: Duration::from_secs(10),
        }
    }
}

// ─── Event types ────────────────────────────────────────────────────────────

/// Unified stimulus enum — every input to the consciousness actor.
#[derive(Debug)]
pub enum ConsciousnessEvent {
    /// User message from HTTP POST /runs or /messages.
    UserMessage(Box<UserMessageEvent>),
    /// Query the actor's current status (mode + queue snapshot).
    QueryStatus { reply: oneshot::Sender<ActorStatus> },
    /// Timer tick from internal intervals.
    TimerTick { tick_type: TimerTickType },
    /// External stimulus from the Spaces distributed networking layer.
    SpacesMessage {
        /// The Spaces channel where the message originated.
        channel_id: String,
        /// Sender identity (hex string from SpacetimeDB).
        sender: String,
        /// Message content.
        content: String,
    },
    /// Autonomic context-pressure signal (e.g. Breathe/Compress/Emergency).
    AutonomicSignal { ruling: String },
    /// Graceful shutdown request.
    Shutdown,
    /// A tool execution completed successfully (future per-tool async).
    ToolResult {
        call_id: String,
        tool_name: String,
        result: serde_json::Value,
        duration_ms: u64,
    },
    /// A tool execution failed (future per-tool async).
    ToolFailed {
        call_id: String,
        tool_name: String,
        error: String,
    },
    /// An agent cycle (spawned task) completed.
    CycleCompleted { run_id: String },
}

/// Snapshot of the actor's status for the GET /queue endpoint.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ActorStatus {
    pub mode: String,
    pub queue_depth: usize,
    pub queue_pending: Vec<PendingMessage>,
    pub has_active_run: bool,
    pub oldest_message_age_ms: Option<u64>,
}

/// A pending message in the queue (serializable for API responses).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingMessage {
    pub id: String,
    pub mode: String,
    pub content: String,
}

/// Payload for a user message event (boxed to keep enum small).
#[derive(Debug)]
pub struct UserMessageEvent {
    pub objective: String,
    pub branch: BranchId,
    pub steering: SteeringMode,
    /// Acknowledgment channel: actor sends back once queued/started.
    pub ack: Option<oneshot::Sender<ConsciousnessAck>>,
    /// Pre-built run context (system prompt, allowed tools, proposed tool).
    pub run_context: RunContext,
}

/// Timer tick types for the consciousness heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerTickType {
    Heartbeat,
    IdleCheck,
}

/// Pre-built context for an agent run (passed from HTTP handler).
#[derive(Debug, Clone, Default)]
pub struct RunContext {
    pub system_prompt: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
    pub proposed_tool: Option<aios_protocol::ToolCall>,
}

/// Acknowledgment sent back to the HTTP handler after event is received.
#[derive(Debug)]
pub enum ConsciousnessAck {
    /// Event was accepted; run started or queued.
    Accepted { queued: bool },
    /// Event was rejected (e.g., shutting down).
    Rejected { reason: String },
}

// ─── Actor state ────────────────────────────────────────────────────────────

/// Operating mode of the consciousness actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsciousnessMode {
    /// Actively running an agent cycle.
    Active,
    /// No active work — ready for new messages.
    Idle,
    /// Low-power mode after extended inactivity.
    Sleeping,
    /// Gracefully shutting down.
    ShuttingDown,
    /// Waiting for spawned tool executions to complete (future per-tool async).
    WaitingForTools,
}

/// Per-session mutable state owned by the consciousness actor.
struct ConsciousnessState {
    session_id: SessionId,
    branch: BranchId,
    mode: ConsciousnessMode,
    queue: MessageQueue,
    last_activity: Instant,
    /// Set by AutonomicSignal when a Compress or Emergency ruling is received.
    /// Cleared after being consumed by the next `run_agent_cycle`.
    needs_compaction: bool,
}

impl ConsciousnessState {
    fn new(session_id: SessionId, branch: BranchId) -> Self {
        Self {
            session_id,
            branch,
            mode: ConsciousnessMode::Idle,
            queue: MessageQueue::new(QueueConfig::default()),
            last_activity: Instant::now(),
            needs_compaction: false,
        }
    }
}

// ─── Actor ──────────────────────────────────────────────────────────────────

/// The consciousness actor — a long-lived tokio task per session.
pub struct SessionConsciousness {
    rx: mpsc::Receiver<ConsciousnessEvent>,
    /// Weak sender for the actor to send events to itself (from spawned tasks).
    ///
    /// Uses `WeakSender` so the channel can still close when all external
    /// `Sender` handles are dropped (e.g. `ConsciousnessHandle` is dropped).
    /// Spawned tasks upgrade to a strong `Sender` which keeps the channel
    /// alive only while the task is running.
    self_tx: mpsc::WeakSender<ConsciousnessEvent>,
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
        let span = tracing::info_span!("consciousness", session = %session_id);
        let actor = Self {
            rx,
            self_tx: tx.downgrade(),
            state: ConsciousnessState::new(session_id, branch),
            runtime,
            config,
        };
        let handle = tokio::spawn(actor.run().instrument(span));
        (handle, tx)
    }

    /// Main event loop — runs until Shutdown or channel closes.
    async fn run(mut self) {
        info!(session = %self.state.session_id, "consciousness actor started");

        let mut heartbeat = tokio::time::interval(self.config.heartbeat_interval);
        let mut idle_check = tokio::time::interval(self.config.idle_check_interval);
        let mut spaces_poll = tokio::time::interval(self.config.spaces_poll_interval);
        // Skip the immediate first tick (fires at interval creation).
        heartbeat.tick().await;
        idle_check.tick().await;
        spaces_poll.tick().await;

        loop {
            let event = tokio::select! {
                biased;

                // Prioritize incoming events over timers.
                msg = self.rx.recv() => match msg {
                    Some(e) => e,
                    None => {
                        info!(session = %self.state.session_id, "channel closed, shutting down");
                        break;
                    }
                },

                _ = heartbeat.tick() => ConsciousnessEvent::TimerTick {
                    tick_type: TimerTickType::Heartbeat,
                },

                _ = idle_check.tick() => ConsciousnessEvent::TimerTick {
                    tick_type: TimerTickType::IdleCheck,
                },

                // Spaces polling: periodically check for new distributed messages.
                // Guarded by config flag — does nothing when spaces_enabled is false.
                _ = spaces_poll.tick(), if self.config.spaces_enabled => {
                    // Stub: actual Spaces SDK integration requires SpacetimeDB client
                    // which uses blocking I/O. Future work will use spawn_blocking
                    // to poll SpacesPort::read_messages here.
                    debug!(
                        session = %self.state.session_id,
                        "spaces polling tick (not yet connected)"
                    );
                    continue;
                },
            };

            match event {
                ConsciousnessEvent::Shutdown => {
                    info!(session = %self.state.session_id, "shutdown requested");
                    self.state.mode = ConsciousnessMode::ShuttingDown;
                    break;
                }
                ConsciousnessEvent::UserMessage(msg) => {
                    self.handle_user_message(
                        msg.objective,
                        msg.branch,
                        msg.steering,
                        msg.ack,
                        msg.run_context,
                    )
                    .await;
                }
                ConsciousnessEvent::QueryStatus { reply } => {
                    let status = self.build_status();
                    let _ = reply.send(status);
                }
                ConsciousnessEvent::TimerTick { tick_type } => {
                    self.handle_timer_tick(tick_type).await;
                }
                ConsciousnessEvent::SpacesMessage {
                    channel_id,
                    sender,
                    content,
                } => {
                    self.handle_spaces_message(channel_id, sender, content)
                        .await;
                }
                ConsciousnessEvent::AutonomicSignal { ruling } => {
                    self.handle_autonomic_signal(&ruling);
                }
                ConsciousnessEvent::CycleCompleted { run_id } => {
                    debug!(run_id, "agent cycle completed");
                    self.state.mode = ConsciousnessMode::Idle;
                    self.state.last_activity = Instant::now();
                    self.state.queue.set_active_run(false).ok();
                    self.drain_queue_after_run().await;
                }
                ConsciousnessEvent::ToolResult { call_id, .. } => {
                    debug!(
                        call_id,
                        "tool result received (per-tool async not yet implemented)"
                    );
                }
                ConsciousnessEvent::ToolFailed { call_id, .. } => {
                    debug!(
                        call_id,
                        "tool failure received (per-tool async not yet implemented)"
                    );
                }
            }
        }

        // Emit final heartbeat before stopping.
        let _ = self
            .runtime
            .record_external_event(
                &self.state.session_id,
                EventKind::Heartbeat {
                    summary: "consciousness-tick".to_string(),
                    checkpoint_id: None,
                },
            )
            .await;

        info!(session = %self.state.session_id, "consciousness actor stopped");
    }

    /// Handle an incoming user message.
    ///
    /// If idle/sleeping, start a new agent cycle immediately.
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
                // Start a new run immediately in a spawned task.
                if let Some(ack) = ack {
                    let _ = ack.send(ConsciousnessAck::Accepted { queued: false });
                }
                self.state.mode = ConsciousnessMode::Active;
                self.state.queue.set_active_run(true).ok();
                self.spawn_agent_cycle(objective, branch, run_context);
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
                        let _ = self
                            .runtime
                            .record_external_event(
                                &self.state.session_id,
                                EventKind::Queued {
                                    queue_id: msg_id,
                                    mode: steering,
                                    message: String::new(),
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

    /// Spawn the agent cycle as a non-blocking tokio task.
    ///
    /// The spawned task runs the tick loop and sends `CycleCompleted` back
    /// to the actor via `self_tx` when done. Queue state management
    /// (set_active_run, drain) stays in the actor event loop.
    fn spawn_agent_cycle(&self, objective: String, branch: BranchId, run_context: RunContext) {
        let run_id = uuid::Uuid::new_v4().to_string();
        let runtime = self.runtime.clone();
        let weak_tx = self.self_tx.clone();
        let session_id = self.state.session_id.clone();
        let queue = self.state.queue.clone();
        let max_iterations = self.config.max_agent_iterations;

        // Upgrade the weak sender to a strong sender for the spawned task.
        // If upgrade fails, all external senders were dropped — skip spawning.
        let Some(self_tx) = weak_tx.upgrade() else {
            warn!(run_id, "cannot spawn agent cycle: channel closed");
            return;
        };

        let span = tracing::info_span!("agent_cycle", run_id = %run_id, session = %session_id);

        tokio::spawn(
            async move {
                Self::run_agent_cycle_inner(
                    &run_id,
                    objective,
                    branch,
                    run_context,
                    runtime,
                    session_id,
                    queue,
                    max_iterations,
                )
                .await;

                // Notify the actor that the cycle is done.
                if let Err(err) = self_tx
                    .send(ConsciousnessEvent::CycleCompleted {
                        run_id: run_id.clone(),
                    })
                    .await
                {
                    error!(run_id, %err, "failed to send CycleCompleted back to actor");
                }
            }
            .instrument(span),
        );
    }

    /// Inner tick loop extracted for use by spawned tasks.
    ///
    /// This is a static-like method that takes all state by value/ref so it
    /// can run independently of the actor's `&mut self`.
    async fn run_agent_cycle_inner(
        run_id: &str,
        objective: String,
        branch: BranchId,
        run_context: RunContext,
        runtime: Arc<KernelRuntime>,
        session_id: SessionId,
        queue: MessageQueue,
        max_iterations: u32,
    ) {
        debug!(run_id, objective = %objective, "starting agent cycle");

        let agent_span = life_vigil::spans::agent_span(session_id.as_str(), "arcan");

        // First tick with the actual objective.
        let mut tick_result = runtime
            .tick_on_branch(
                &session_id,
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
        let mut stall_counter: u32 = 0;
        for iteration in 1..max_iterations {
            match &tick_result {
                Ok(tick) if tick.mode == OperatingMode::Execute => {
                    // ── Stall detection ──
                    // Track whether events_emitted advances between iterations.
                    let last_events = tick.events_emitted;

                    // Check preemption at tool boundary.
                    match queue.check_preemption() {
                        Ok(SteeringAction::Abort { reason }) => {
                            warn!(iteration, %reason, "run aborted by interrupt");
                            let _ = runtime
                                .record_external_event(
                                    &session_id,
                                    EventKind::Steered {
                                        queue_id: run_id.to_owned(),
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
                                "steering: switch after current iteration"
                            );
                            let _ = runtime
                                .record_external_event(
                                    &session_id,
                                    EventKind::Steered {
                                        queue_id: msg.id.clone(),
                                        preempted_at: format!("iteration:{iteration}"),
                                    },
                                )
                                .await;
                            // Re-queue as followup for processing after drain.
                            let requeue = QueuedMessage {
                                id: msg.id,
                                mode: SteeringMode::Followup,
                                content: msg.content,
                                queued_at: None,
                            };
                            queue.enqueue(requeue).ok();
                            break;
                        }
                        Ok(SteeringAction::InjectMessage(_) | SteeringAction::Continue) => {}
                        Err(err) => {
                            warn!(%err, "preemption check failed, continuing");
                        }
                    }

                    debug!(iteration, "agent loop: continuing after tool execution");
                    tick_result = runtime
                        .tick_on_branch(
                            &session_id,
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

                    // ── Stall detection (post-tick) ──
                    if let Ok(ref new_tick) = tick_result {
                        if new_tick.events_emitted == last_events
                            && new_tick.mode == OperatingMode::Execute
                        {
                            stall_counter += 1;
                            debug!(iteration, stall_counter, "no new events — potential stall");

                            if stall_counter >= 3 {
                                warn!(
                                    session = %session_id,
                                    iteration,
                                    stall_counter,
                                    "stall detected — breaking agent cycle"
                                );
                                let _ = runtime
                                    .record_external_event(
                                        &session_id,
                                        EventKind::Custom {
                                            event_type: "consciousness.stall_detected".to_string(),
                                            data: serde_json::json!({
                                                "iteration": iteration,
                                                "stall_counter": stall_counter,
                                                "run_id": run_id,
                                            }),
                                        },
                                    )
                                    .await;
                                break;
                            }
                        } else {
                            stall_counter = 0;
                        }
                    }
                }
                _ => break,
            }
        }

        match &tick_result {
            Ok(tick) => {
                debug!(run_id, mode = ?tick.mode, events = tick.events_emitted, "agent cycle completed");
            }
            Err(err) => {
                error!(run_id, %err, "agent cycle failed");
            }
        }
    }

    /// Process queued messages after a run completes.
    ///
    /// Spawns the next queued message as a non-blocking agent cycle.
    /// Only the first drained message is spawned; remaining messages stay
    /// queued and will be drained when the spawned cycle completes.
    async fn drain_queue_after_run(&mut self) {
        let drained: Vec<QueuedMessage> = match self.state.queue.drain_after_run() {
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

        // Re-enqueue all but the first message so they are processed
        // sequentially as each CycleCompleted triggers another drain.
        let mut iter = drained.into_iter();
        if let Some(next) = iter.next() {
            // Re-enqueue remaining messages for future drains.
            for msg in iter {
                let requeue = QueuedMessage {
                    id: msg.id,
                    mode: msg.mode,
                    content: msg.content,
                    queued_at: None,
                };
                self.state.queue.enqueue(requeue).ok();
            }

            // Spawn the next message as a non-blocking cycle.
            self.state.mode = ConsciousnessMode::Active;
            self.state.queue.set_active_run(true).ok();
            self.spawn_agent_cycle(
                next.content,
                self.state.branch.clone(),
                RunContext::default(),
            );
        }
    }

    /// Handle timer ticks (heartbeat, idle check).
    async fn handle_timer_tick(&mut self, tick_type: TimerTickType) {
        match tick_type {
            TimerTickType::Heartbeat => {
                let _ = self
                    .runtime
                    .record_external_event(
                        &self.state.session_id,
                        EventKind::Heartbeat {
                            summary: "consciousness-tick".to_string(),
                            checkpoint_id: None,
                        },
                    )
                    .await;

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
        }
    }

    /// Handle an incoming Spaces message (external distributed stimulus).
    ///
    /// If the actor is idle or sleeping, the message is re-injected as a
    /// `UserMessage` to trigger a deliberation cycle. If active, the message
    /// is logged as context. Own messages (matching the session_id) are ignored.
    async fn handle_spaces_message(&mut self, channel_id: String, sender: String, content: String) {
        // Filter out own messages to avoid feedback loops.
        if sender == self.state.session_id.as_str() {
            debug!(
                session = %self.state.session_id,
                %channel_id,
                "ignoring own Spaces message"
            );
            return;
        }

        let content_preview = if content.len() > 80 {
            format!("{}...", &content[..80])
        } else {
            content.clone()
        };

        info!(
            session = %self.state.session_id,
            %channel_id,
            %sender,
            content_preview,
            "received Spaces message"
        );

        match self.state.mode {
            ConsciousnessMode::Idle | ConsciousnessMode::Sleeping => {
                // Re-inject as a user message to trigger a deliberation cycle.
                info!(
                    session = %self.state.session_id,
                    mode = ?self.state.mode,
                    "Spaces message triggering deliberation cycle"
                );
                let objective = format!("[spaces:{channel_id}] @{sender}: {content}");
                self.handle_user_message(
                    objective,
                    self.state.branch.clone(),
                    SteeringMode::Collect,
                    None,
                    RunContext::default(),
                )
                .await;
            }
            ConsciousnessMode::Active | ConsciousnessMode::WaitingForTools => {
                // Don't interrupt the current run — log as context for future use.
                debug!(
                    session = %self.state.session_id,
                    %channel_id,
                    %sender,
                    "Spaces message received during active run (logged as context)"
                );
            }
            ConsciousnessMode::ShuttingDown => {
                debug!(
                    session = %self.state.session_id,
                    "ignoring Spaces message during shutdown"
                );
            }
        }
    }

    /// Handle an autonomic context-pressure signal.
    ///
    /// If the ruling indicates Compress or Emergency, set the `needs_compaction`
    /// flag so the next `run_agent_cycle` injects a compaction hint.
    fn handle_autonomic_signal(&mut self, ruling: &str) {
        info!(
            session = %self.state.session_id,
            ruling,
            "received autonomic signal"
        );

        if ruling.contains("Compress") || ruling.contains("Emergency") {
            self.state.needs_compaction = true;
            warn!(
                session = %self.state.session_id,
                ruling,
                "context pressure elevated — compaction flag set"
            );
        }
    }

    /// Build a status snapshot for the QueryStatus response.
    fn build_status(&self) -> ActorStatus {
        let queue_status = self.state.queue.status().ok();
        let pending = queue_status
            .as_ref()
            .map(|s| {
                s.pending
                    .iter()
                    .map(|m| PendingMessage {
                        id: m.id.clone(),
                        mode: format!("{:?}", m.mode),
                        content: m.content.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        ActorStatus {
            mode: format!("{:?}", self.state.mode),
            queue_depth: queue_status.as_ref().map(|s| s.depth).unwrap_or(0),
            queue_pending: pending,
            has_active_run: queue_status.as_ref().is_some_and(|s| s.has_active_run),
            oldest_message_age_ms: queue_status.and_then(|s| s.oldest_message_age_ms),
        }
    }
}

// ─── Handle ─────────────────────────────────────────────────────────────────

/// Handle for communicating with a consciousness actor.
#[derive(Clone)]
pub struct ConsciousnessHandle {
    /// Sender for pushing events to the actor.
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
    pub async fn send(
        &self,
        event: ConsciousnessEvent,
    ) -> Result<(), mpsc::error::SendError<ConsciousnessEvent>> {
        self.tx.send(event).await
    }

    /// Query the actor's current status (mode + queue snapshot).
    pub async fn query_status(&self) -> Option<ActorStatus> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ConsciousnessEvent::QueryStatus { reply: reply_tx })
            .await
            .ok()?;
        tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .ok()?
            .ok()
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

// ─── Registry ───────────────────────────────────────────────────────────────

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

        info!(
            count = handles.len(),
            "shutting down all consciousness actors"
        );

        let futures: Vec<_> = handles
            .into_iter()
            .map(ConsciousnessHandle::shutdown)
            .collect();
        futures_util::future::join_all(futures).await;
    }
}

/// Check whether consciousness mode is enabled via env var.
pub fn is_consciousness_enabled() -> bool {
    std::env::var("ARCAN_CONSCIOUSNESS").is_ok_and(|v| v == "true" || v == "1")
}
