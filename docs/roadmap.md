# Arcan Implementation Roadmap

## Phase 0 - Foundations (Now)

- Define crate boundaries and protocol types.
- Build deterministic orchestrator skeleton.
- Add hashline edit primitives and policy interfaces.
- Add append-only store abstractions.
- Add SSE event encoder.

Exit criteria:

- `cargo check` passes.
- Example run can emit a full event stream from `RunStarted` to `RunFinished`.

## Phase 1 - Minimal Vertical Slice

Dependency chain:

1. Add one provider adapter (mock/local first).
2. Implement core tools (`fs.read`, `fs.write`, `fs.glob`, `bash.run`).
3. Wire store persistence around run execution.
4. Expose `/v1/runs` with SSE streaming in `agentd`.
5. Build CLI client for end-to-end loop testing.

Exit criteria:

- One full run persisted and replayable.
- SSE stream consumed by CLI with typed rendering.

## Phase 2 - Safety Hardening

Dependency chain:

1. Enforce sandbox policy in command executor.
2. Add approval middleware for high-risk tools.
3. Add stale-edit rejection and conflict retry strategy.
4. Add run cancellation and timeout propagation.

Exit criteria:

- Policy violations block execution with structured events.
- canceled run leaves consistent persisted state.

## Phase 3 - Stateful Product Surface

Dependency chain:

1. State patch events surfaced to clients.
2. State replay and version checks implemented.
3. Add session branch/fork API with parent linkage.
4. Add branch-aware resume semantics.

Exit criteria:

- UI/client can reconstruct app state from event replay alone.
- session fork and resume both deterministic.

## Phase 4 - Multi-Interface Integration

Dependency chain:

1. Next.js/TS client integration package.
2. Bot adapters (Telegram/Discord/WhatsApp).
3. Shared auth/session identity model.

Exit criteria:

- Same `agentd` run protocol works across web and chat transports.

## Phase 5 - Advanced Runtime

Dependency chain:

1. Extension SDK (WASM/native middleware).
2. Sub-agent execution model.
3. Parallel tool planning with strict conflict controls.
4. Snapshotting and compaction for long sessions.

Exit criteria:

- Extensions can add tools/middleware without core changes.
- Sub-agent runs are replayable and auditable in session tree.

## Cross-Cutting Quality Gates

- Schema compatibility tests for events and patches.
- Property tests for hashline edit correctness.
- Fuzzing for parser/sandbox boundary logic.
- Performance baseline:
  - p50/p95 run latency
  - event throughput
  - store append/replay speed

