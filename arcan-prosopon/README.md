# arcan-prosopon

`Pneuma<L0ToExternal>` for [Arcan](../arcan). Subscribes to the runtime's
`aios_protocol::EventRecord` broadcast, translates each `EventKind` into a
`prosopon_core::ProsoponEvent`, and publishes envelopes into a
`prosopon_daemon::EnvelopeFanout` for downstream compositors (text, glass,
field, …).

## Architecture

```
KernelRuntime::subscribe_events()                   <- tokio::broadcast<EventRecord>
        │
        ▼
translator::translate(&mut state, &record.kind) -> Vec<ProsoponEvent>
        │
        ▼
prosopon_sdk::Session::envelope(event) -> Envelope
        │
        ▼
EnvelopeFanout::send(envelope)                      -> fan out to compositors
```

`Bridge` is producer-only. A caller (e.g. `arcan` binary with `--features prosopon`) constructs the fanout + daemon and hands it to `ArcanProsoponBridge::new(fanout).spawn(events)`; the spawned task exits cleanly when the upstream broadcast closes.

## Public API

```rust
use arcan_prosopon::{ArcanProsoponBridge, TranslationState, BridgeError};
use prosopon_daemon::EnvelopeFanout;
use tokio::sync::broadcast;

let fanout = EnvelopeFanout::new();
let bridge = ArcanProsoponBridge::new(fanout);
let handle = bridge.spawn(event_rx);   // event_rx: broadcast::Receiver<aios_protocol::EventRecord>
```

## Translation table

| `EventKind` (aios-protocol) | `ProsoponEvent`(s) emitted |
|---|---|
| `SessionCreated { name }` | `SceneReset { scene }` — root node has stable id `"session-root"` |
| `RunStarted` | 3× `SignalChanged` — `run.status`, `run.provider`, `run.max_iterations` |
| `RunErrored { error }` | `NodeAdded` error prose + `SignalChanged { topic: "run.status", value: "errored" }` |
| `UserMessage { content }` | `NodeAdded` section("User") > prose |
| `AssistantTextDelta` / `TextDelta { delta, index }` | First delta per iteration: `NodeAdded` `Intent::Stream` + `StreamChunk`. Subsequent deltas: `StreamChunk` only (monotonic per-stream `seq`) |
| `AssistantMessageCommitted` / `Message { content }` | `NodeAdded` section("Assistant") > prose |
| `ToolCallRequested { call_id, tool_name, arguments }` | `NodeAdded` `Intent::ToolCall` with stable id `tool:{call_id}` |
| `ToolCallCompleted { call_id, result, status }` | `NodeUpdated` — appends `Intent::ToolResult` child. Orphan events (no `call_id`) are dropped. |
| `ToolCallFailed { call_id, error }` | `NodeUpdated` — appends `Intent::ToolResult { success: false, payload: {"error": …} }` |
| `ApprovalRequested { risk, .. }` | `NodeAdded` `Intent::Confirm` with mapped `Severity` |
| `ApprovalResolved { decision }` | `NodeUpdated` lifecycle → `Resolved` + `SignalChanged { topic: approval.{id}, value: decision }` |
| `StatePatched { revision }` | `SignalChanged { topic: state.revision }` |
| `ContextCompacted { tokens_before, tokens_after }` | `SignalChanged { topic: context.tokens }` + low-emphasis prose node |
| `StepStarted { index }` | `SignalChanged { topic: iteration }` |
| `PolicyEvaluated { tool_name, decision }` | `SignalChanged { topic: policy.{tool_name} }` |
| `KnowledgeSearched { query, result_count }` | `NodeAdded` low-emphasis prose |
| every other variant | wildcard → empty `Vec` |

Severity mapping: `RiskLevel::{Low → Info, Medium → Notice, High → Warning, Critical → Danger}`.

## Integration with `arcan` (binary)

Build with the `prosopon` feature and pass `--prosopon-port`:

```sh
cargo run -p arcan --features prosopon -- serve --prosopon-port 127.0.0.1:4321
```

Bind failure logs a warning and arcan continues normally — the `arcan-console` React UI keeps working independently of Prosopon.

## Links

- Design plan: [`docs/superpowers/plans/2026-04-23-bro-773-arcan-prosopon.md`](../../../docs/superpowers/plans/2026-04-23-bro-773-arcan-prosopon.md)
- Linear: [BRO-773](https://linear.app/broomva/issue/BRO-773)
- Prosopon repo: `core/prosopon` (v0.2.0-alpha.2)
