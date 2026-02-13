# Arcan Architecture (Grounding)

This document defines the initial architecture for Arcan as a Rust-first agent runtime and daemon (`agentd`).

## 1. Goals

- Build a provider-agnostic reasoning daemon.
- Treat harness quality as a first-class concern (tool schemas, sandboxing, deterministic loop behavior, safe edits).
- Keep state unified: agent state is app state.
- Stream typed events for rich clients (CLI/TUI/web/chat adapters).
- Guarantee replay and recovery via append-only persistence.

## 2. Design Principles

- Deterministic orchestration: identical inputs and tool outputs should produce reproducible event sequences.
- Strong boundaries: provider, tool execution, orchestration, and persistence are independent modules.
- Typed protocol first: all externally visible events and state patches are schema-backed Rust types.
- Safety layers:
  - policy checks before tool execution,
  - sandbox enforcement during execution,
  - append-only auditing after execution.
- Fail closed: unknown tools, stale edit tags, and policy violations become structured failures, not implicit fallbacks.

## 3. Workspace Topology

- `crates/arcan-core`
  - protocol schemas (events, directives, patches)
  - shared `AppState`
  - runtime traits (`Provider`, `Tool`, `Middleware`)
  - deterministic orchestrator loop
- `crates/arcan-harness`
  - sandbox policy model
  - filesystem guardrails
  - hashline-based edit primitives
- `crates/arcan-store`
  - append-only event record model
  - session tree repository API
  - in-memory and JSONL baseline implementations
- `crates/arcan-daemon`
  - `agentd` transport boundary
  - SSE encoder for typed events
  - HTTP/runtime integration (next step)

## 4. Core Runtime Model

Run lifecycle:

1. `RunStarted`
2. Loop per iteration:
   - emit `IterationStarted`
   - call provider with messages + tool schemas + current state snapshot
   - provider returns typed directives (text/tool/state/final)
   - execute directives in order while emitting events
3. Stop on:
   - `completed`
   - `needs_user`
   - `blocked_by_policy`
   - `budget_exceeded`
   - `error`
4. Emit `RunFinished`

Budget controls:

- max iterations per run
- optional tool timeout budget (harness layer)
- future: token/request budget

## 5. State Model (Agent State == App State)

`AppState`:

- `revision: u64`
- `data: serde_json::Value`

Patch contract:

- `StatePatch { format, patch, source }`
- `format` supports:
  - JSON Patch (RFC 6902)
  - JSON Merge Patch (RFC 7396)
- each successful patch increments `revision`

Rule:

- any tool side effect visible to the app should map to a `StatePatch` event.

## 6. Streaming Protocol

Transport-agnostic event schema is `AgentEvent`.

SSE compatibility:

- each event is serialized to one `data:` line with JSON payload
- payload includes `part_type` discriminator to support typed client rendering
- transient vs persistent events are represented in event type choice (future: explicit durability bit)

Core part types:

- run lifecycle (`run_started`, `iteration_started`, `run_finished`, `run_errored`)
- model output (`model_output`, `text_delta`)
- tool lifecycle (`tool_call_requested`, `tool_call_completed`, `tool_call_failed`)
- state sync (`state_patched`)

## 7. Harness Model

### 7.1 Sandboxing

`SandboxPolicy` captures:

- workspace root
- shell/network toggles
- env allowlist
- limits: timeout, output bytes, process count, memory

Execution path:

1. middleware pre-check
2. policy validation
3. sandbox executor run
4. result normalization + audit event

### 7.2 Filesystem + Edit Reliability

Hashline primitives:

- `hash_lines(content) -> [HashedLine]`
- `apply_tagged_edits(content, ops)`

Operation semantics:

- replace/delete/insert operations reference stable line tags
- stale or missing tags fail with explicit error
- supports optimistic concurrency for agent edits

## 8. Persistence and Recovery

Event sourcing baseline:

- append-only `EventRecord { id, session_id, parent_id, ts, event }`
- session tree via `parent_id`
- in-memory and JSONL repositories included initially

Recovery:

- replay records to rebuild run/session state
- future: snapshots for fast cold start

## 9. Extension and Control Plane

`Middleware` hooks:

- before/after model call
- before/after tool call
- on run finished

Expected uses:

- policy enforcement
- telemetry
- guardrail insertion
- dynamic context injection

## 10. Dependency-Ordered Build Strategy

Phase 1: protocol + state + orchestrator skeleton (`arcan-core`)  
Phase 2: harness policies + hashline edits (`arcan-harness`)  
Phase 3: append-only store and session tree (`arcan-store`)  
Phase 4: daemon SSE surface (`arcan-daemon`)  
Phase 5: provider adapters + real tool implementations + approvals

