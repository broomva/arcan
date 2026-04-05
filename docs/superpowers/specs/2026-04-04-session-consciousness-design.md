# Session Consciousness — Event-Driven Agent Loop

**Date**: 2026-04-04
**Status**: Approved, ready for implementation
**Replaces**: Request-response `run_session` handler in `canonical.rs`

## Problem

The daemon uses request-response: `POST /runs` blocks until the agent finishes. Users can't send messages while the agent is thinking. Tool execution is synchronous. No concurrent message handling.

## Solution

Each session gets a long-lived `tokio::spawn` task (the "consciousness actor") that owns a `tokio::select!` event loop. HTTP handlers become thin event-pushers that return immediately (202 Accepted). All results flow via the existing SSE streaming infrastructure.

Inspired by Poke's iMessage-based continuous agent interaction model.

## Architecture

### New file: `crates/arcand/src/consciousness.rs`

**ConsciousnessEvent** — unified stimulus enum:
- `UserMessage` — from HTTP POST /runs or /messages
- `ToolResult` / `ToolFailed` — from background tool execution
- `SpacesMessage` — from SpacetimeDB subscription
- `AutonomicSignal` — from periodic evaluation
- `TimerTick` — heartbeat, idle check, sleep-wake
- `ApprovalResolved` — from HTTP POST /approvals
- `ExternalSignal` — webhook, scheduled task
- `Shutdown` — graceful exit

**ConsciousnessState** — per-session mutable state:
- `MessageQueue` (reuse existing from `arcan-core/queue.rs`)
- `HomeostaticState` (Autonomic regulation)
- `running_tools: HashMap<String, RunningToolInfo>`
- `mode: ConsciousnessMode` (Active/WaitingForTools/Idle/Sleeping/ShuttingDown)

**SessionConsciousness** — the actor:
```rust
loop {
    let event = tokio::select! {
        Some(e) = self.rx.recv() => e,
        _ = self.heartbeat.tick() => TimerTick(Heartbeat),
        _ = self.idle_check.tick() => TimerTick(IdleCheck),
    };
    // Perceive → Deliberate → Act
    match event { ... }
    self.maybe_deliberate().await;
}
```

**ConsciousnessRegistry** — session→actor map in CanonicalState

### Message Queuing (concurrent user messages)

If mode=Active, user messages go into `MessageQueue` with `SteeringMode`:
- **Collect** — add to queue, process after current run
- **Followup** — continue in same context
- **Steer** — complete current iteration then switch
- **Interrupt** — abort current run immediately

At iteration boundaries: `check_preemption()`. After run: `drain_queue_after_run()`.

EventKind::Queued, ::Steered, ::QueueDrained already exist in aios-protocol.

### HTTP Changes

**run_session** (modified): Push `ConsciousnessEvent::UserMessage`, return 202 Accepted
**POST /messages** (new): Lightweight push for follow-ups/interrupts during active processing
**GET /queue** (new): Queue introspection

### No Changes Needed

- `aios-runtime` — KernelRuntime unchanged
- `stream_events` SSE — works unchanged (events flow through same broadcast)
- TUI — works unchanged (consumes SSE regardless of source)

### Backward Compatibility

Feature flag `ARCAN_CONSCIOUSNESS=true`. When off, existing blocking behavior.

## Implementation Phases

1. **Core actor loop** — consciousness.rs, types, main loop, UserMessage handler
2. **HTTP integration** — non-blocking run_session, /messages, /queue endpoints
3. **Timer & Autonomic** — heartbeat, idle detection, auto-sleep, Autonomic signals
4. **Spaces integration** — SpacetimeDB subscription → SpacesMessage events
5. **Async tool execution** — tools as background tasks, results as events

## Key Files

- `crates/arcand/src/consciousness.rs` — NEW
- `crates/arcand/src/canonical.rs` — MODIFY (add registry, modify handler)
- `crates/arcan-core/src/queue.rs` — REUSE (already implemented)
- `crates/arcand/Cargo.toml` — ADD parking_lot
