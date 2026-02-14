# Arcan + Lago Integration

Lago is the **persistence and governance layer** for arcan's agent runtime. While arcan handles execution (LLM calls, tool dispatch, sandboxing, streaming), lago provides the durable infrastructure: an ACID event journal, content-addressed blob storage, a rule-based policy engine, session branching, and multi-format SSE output.

The integration lives in the `arcan-lago` bridge crate and the `agentd` unified daemon binary.

## Architecture

```
                        agentd (unified binary)

  +------------+  +-----------+  +--------------+
  | Provider   |  |   Tool    |  |  Middleware   |
  | (LLM)     |  | Registry  |  | (Policy)     |
  +-----+------+  +-----+-----+  +------+-------+
        |              |               |
        +--------------+---------------+
                       |
              +--------+--------+
              |  Orchestrator   | (arcan-core)
              |  Agent Loop     |
              +--------+--------+
                       | AgentEvent stream
                       v
         +----------------------------+
         |     arcan-lago bridge      |
         |                            |
         |  event_map.rs   - convert  |
         |  repository.rs  - persist  |
         |  policy_mw.rs   - enforce  |
         |  state_proj.rs  - project  |
         |  sse_bridge.rs  - format   |
         +------------+---------------+
                      |
         +------------v---------------+
         |      lago crates           |
         |  lago-core    (types)      |
         |  lago-journal (redb ACID)  |
         |  lago-policy  (rules)      |
         |  lago-store   (blobs)      |
         |  lago-api     (SSE fmt)    |
         +----------------------------+
```

### Design Principle: Bridge Crate, Not Direct Coupling

`arcan-core` has **zero** dependency on lago. Only `arcan-lago` and `agentd` depend on lago crates. This keeps arcan's core traits portable -- any backend (JSONL, SQLite, custom) can implement `SessionRepository` without pulling in lago.

The bridge crate translates between arcan's synchronous, event-sourced domain model and lago's async, ULID-indexed journal infrastructure.

---

## Modules

### 1. Event Mapping (`event_map.rs`)

Bidirectional conversion between arcan's `AgentEvent` and lago's `EventPayload`.

#### Arcan to Lago

| Arcan `AgentEvent` | Lago `EventPayload` | Notes |
|---|---|---|
| `TextDelta { delta, iteration }` | `MessageDelta { role: "assistant", delta, index }` | Direct mapping |
| `ToolCallRequested { call }` | `ToolInvoke { call_id, tool_name, arguments }` | Direct mapping |
| `ToolCallCompleted { result }` | `ToolResult { status: Ok, result }` | Direct mapping |
| `ToolCallFailed { error }` | `ToolResult { status: Error, result: {error} }` | Error in JSON |
| `RunFinished { final_answer: Some(a) }` | `Message { role: "assistant", content: a }` | Plus metadata |
| `RunFinished { final_answer: None }` | `Custom { event_type: "run_finished" }` | Serialized event |
| `RunStarted` | `Custom { event_type: "run_started" }` | Serialized event |
| `IterationStarted` | `Custom { event_type: "iteration_started" }` | Serialized event |
| `ModelOutput` | `Custom { event_type: "model_output" }` | Serialized event |
| `StatePatched` | `Custom { event_type: "state_patched" }` | Serialized event |
| `RunErrored` | `Custom { event_type: "run_errored" }` | Serialized event |

Every envelope carries `arcan_event_id` in its metadata map for round-trip tracking. Arcan-specific events (RunStarted, StatePatched, etc.) are stored as `Custom` payloads with the full `AgentEvent` serialized as JSON, enabling lossless reconstruction.

#### Lago to Arcan

The reverse mapping reconstructs `AgentEvent` from `EventEnvelope`:

- `MessageDelta` -> `TextDelta`
- `ToolInvoke` -> `ToolCallRequested`
- `ToolResult(Ok)` -> `ToolCallCompleted`
- `ToolResult(Error)` -> `ToolCallFailed`
- `Message(assistant)` -> `TextDelta` (for replay context)
- `Custom(run_started|iteration_started|...)` -> deserialized `AgentEvent`

Returns `Option<AgentEvent>` -- events that don't map back (user messages, file events not from arcan) are silently skipped.

**Trade-off**: Iteration index defaults to 0 for ToolInvoke/ToolResult reverse mapping (lago doesn't preserve this field natively). The full event is recoverable from the Custom payload's serialized data.

### 2. Journal Repository (`repository.rs`)

`LagoSessionRepository` implements arcan's `SessionRepository` trait backed by lago's `Journal`.

```rust
pub struct LagoSessionRepository {
    journal: Arc<dyn Journal>,
    default_branch: BranchId,  // "main" by default
}
```

#### Sync/Async Boundary

Arcan's `SessionRepository` trait is synchronous. Lago's `Journal` trait is async. The boundary is crossed via `tokio::runtime::Handle::current().block_on()`.

This is safe because the orchestrator always runs inside `tokio::task::spawn_blocking`, which executes on a dedicated thread pool outside the async runtime. `block_on()` can safely drive async futures on that thread.

#### Operations

- **`append()`**: Gets next sequence number from journal, generates UUID for `arcan_event_id`, converts event via `arcan_to_lago()`, appends `EventEnvelope` to journal, returns `EventRecord` with timestamp.

- **`load_session()`**: Queries journal with `EventQuery` filtering by session and branch. Converts each envelope back via `lago_to_arcan()`. Reconstructs `EventRecord` with arcan_event_id from metadata.

- **`load_children()`**: Scans the default branch for events with matching `parent_id`. Currently O(n) -- lago doesn't index by parent_id natively. Acceptable because branching/child lookups are rare in the agent loop.

- **`head()`**: Loads the full session and returns the last event.

### 3. Policy Middleware (`policy_middleware.rs`)

`LagoPolicyMiddleware` wraps lago's `PolicyEngine` as an arcan `Middleware`, enabling rule-based tool governance.

```rust
pub struct LagoPolicyMiddleware {
    engine: PolicyEngine,
    tool_annotations: HashMap<String, ToolAnnotations>,
}
```

#### Risk Level Derivation

Tool annotations are mapped to lago's `RiskLevel` enum:

| Annotation | Risk Level |
|---|---|
| `requires_confirmation = true` | `High` |
| `destructive = true` | `Medium` |
| `read_only = true` | `Low` |
| (default / unknown tool) | `Low` |

#### Evaluation Flow

On every `pre_tool_call`:

1. Build `PolicyContext` from `ToolCall` + derived risk level + session_id
2. Call `engine.evaluate(&context)` to get `PolicyDecision`
3. Handle decision:
   - `Allow` -> return `Ok(())`
   - `Deny` -> return `Err(CoreError::Middleware("tool 'X' blocked: reason"))`
   - `RequireApproval` -> return `Err(CoreError::Middleware("tool 'X' requires approval"))` (no interactive approval flow yet)

#### Rule Examples

```rust
// Block all bash execution
Rule {
    id: "deny-bash",
    condition: MatchCondition::ToolName("bash"),
    decision: PolicyDecisionKind::Deny,
    explanation: Some("bash is not allowed"),
    priority: 100,
}

// Require approval for high-risk tools
Rule {
    id: "approve-high-risk",
    condition: MatchCondition::RiskAtLeast(RiskLevel::High),
    decision: PolicyDecisionKind::RequireApproval,
    priority: 50,
}
```

### 4. State Projection (`state_projection.rs`)

`AppStateProjection` implements lago's `Projection` trait to rebuild `AppState` + conversation history from the event stream.

```rust
pub struct AppStateProjection {
    state: AppState,
    messages: Vec<ChatMessage>,
}
```

#### Projection Logic

For each `EventEnvelope`, convert to `AgentEvent` via `lago_to_arcan()`, then:

- `StatePatched { patch }` -> apply patch to `AppState`
- `TextDelta { delta }` -> aggregate into last assistant message (or create new one)
- `ToolCallCompleted { result }` -> add tool result message
- All other events -> ignored

This replaces the ad-hoc replay logic in `loop.rs` with a reusable, testable component that plugs into lago's projection system.

#### Usage

```rust
let mut proj = AppStateProjection::new();
for envelope in journal.read(query).await? {
    proj.on_event(&envelope)?;
}
let (state, messages) = proj.into_parts();
```

### 5. SSE Bridge (`sse_bridge.rs`)

`SseBridge` converts arcan's `AgentEvent` stream into SSE frames using any lago `SseFormat` implementation.

```rust
pub struct SseBridge {
    format: Box<dyn SseFormat>,  // OpenAI, Anthropic, Vercel, Lago
    session_id: SessionId,
    branch_id: BranchId,
    seq: u64,
}
```

#### Supported Formats

| Format | Description | Filtering |
|---|---|---|
| `"openai"` | OpenAI Chat Completion Stream chunks | Only Message/MessageDelta |
| `"anthropic"` | Anthropic streaming format | Only Message/MessageDelta |
| `"vercel"` | Vercel AI SDK v5 data parts | Message/MessageDelta + headers |
| `"lago"` | Lago native (all events, typed) | Everything emitted |

#### Flow

```
AgentEvent -> arcan_to_lago() -> EventEnvelope -> SseFormat::format() -> Vec<SseFrame>
```

Each `SseFrame` has:
- `event`: Optional SSE event type
- `data`: JSON string body
- `id`: Optional sequence ID for reconnection

#### Format Selection

```rust
let format = arcan_lago::select_format("openai").unwrap();
let mut bridge = SseBridge::new(format, session_id, "main");

for event in agent_events {
    let frames = bridge.format_event(&event, &run_id);
    for frame in frames {
        // send as SSE
    }
}
if let Some(done) = bridge.done_frame() {
    // send final frame
}
```

---

## Wiring: agentd Binary

The `agentd` binary (`crates/agentd/src/main.rs`) wires everything together:

```
1. Open RedbJournal (ACID embedded database)
2. Open BlobStore (content-addressed file storage)
3. Create LagoSessionRepository (journal-backed)
4. Register all tools (fs, edit, bash, memory)
5. Extract tool annotations from registry
6. Create PolicyEngine + LagoPolicyMiddleware
7. Create Orchestrator with provider + tools + middleware
8. Create AgentLoop with session_repo + orchestrator
9. Start Axum HTTP server on port 3000
```

CLI arguments:
- `--data-dir` (default `.arcan`) -- journal.redb and blobs stored here
- `--port` (default 3000) -- HTTP listen port
- `--max-iterations` (default 10) -- orchestrator iteration budget

---

## Dependency Graph

```
agentd
  +-- arcan-core        (traits, protocol, state)
  +-- arcan-harness     (tools, sandbox)
  +-- arcan-provider    (Anthropic, Rig)
  +-- arcan-daemon      (AgentLoop, server)
  +-- arcan-lago        (bridge)
  |     +-- arcan-core
  |     +-- arcan-store
  |     +-- lago-core
  |     +-- lago-journal
  |     +-- lago-store
  |     +-- lago-policy
  |     +-- lago-api
  +-- lago-journal      (RedbJournal)
  +-- lago-store        (BlobStore)
  +-- lago-policy       (PolicyEngine)
```

`arcan-core` remains lago-free. Only the bridge and daemon depend on lago.

---

## Test Coverage

The `arcan-lago` crate has 33 tests across all modules:

- **event_map** (9 tests): Round-trip for all event types, metadata preservation, Message vs Custom for RunFinished
- **repository** (4 tests): Append + load, head, empty session, tool event round-trip through journal
- **policy_middleware** (6 tests): Allow, deny by name, deny by risk, require-approval, non-matching, risk derivation
- **state_projection** (6 tests): Text delta aggregation, tool results, state patches, mixed events, into_parts
- **sse_bridge** (8 tests): OpenAI frame output, format selection, filtering, done frame, sequence numbers

---

## Future: Phase 5

Not yet implemented, reserved for later:

- **Content-addressed files**: Arcan file tools emit `FileWrite`/`FileDelete` events to lago journal. File reads optionally go through lago's `Mount` + `BlobStore` for versioning.
- **Session branching**: Fork sessions from any event to explore alternatives. Merge branches back.
- **Interactive approval**: `RequireApproval` policy decisions pause the agent loop and await user input instead of failing.
- **Custom projections**: User-defined event projections for analytics, dashboards, and debugging.
