# CLAUDE.md — `arcan-ergon` crate

> Instructions for AI agents working in this crate.
> Last updated: 2026-05-08.

## What this crate is

**arcan-ergon** is the kernel-side adapter that runs an
`ergon::Workflow` as the body of a single
`aios_runtime::KernelRuntime` tick. It's the substrate side of the
ergon harness — where `ergon` (and `ergon-life-hooks`) is
vendor-neutral and depends only on `aios-protocol`, this crate
translates between ergon's traits and the live kernel ports +
Life-substrate types.

## Position in the harness stack

```
L5 — Session orchestration (arcand::ConsciousnessActor)
L4 — Tick engine (aios_runtime::KernelRuntime)
L3.5 — Tick body — direct OR ergon::Workflow      ← THIS CRATE supplies the workflow shape
L3 — Port traits (aios-runtime / aios-protocol)
L2 — Substrate adapters (incl. arcan-ergon — THIS CRATE)
L1 — Substrate primitives (lago, praxis, anima, ...)
L0 — Kernel contract (aios-protocol)
```

## Spec & tracker

- Spec: `docs/superpowers/specs/2026-05-08-bro-1001-ergon-tick-body.md`
- Linear: [BRO-1001](https://linear.app/broomva/issue/BRO-1001)
- Umbrella: [BRO-994](https://linear.app/broomva/issue/BRO-994)
- Architecture: `docs/architecture/agent-harness.md`

## Module map

| Module | Role |
|---|---|
| `error`           | `AdapterError` covering boundary-translation failures |
| `registry`        | `WorkflowRegistry` (string → boxed `WorkflowExecutor`) |
| `runtime_handle`  | `ModeRuntimeHandle` — `ergon::RuntimeHandle` over a captured `OperatingMode` |
| `provider`        | `ModelProviderAdapter` — `ergon::Provider` over `ModelProviderPort` |
| `tools`           | `ToolHarnessAdapter` — `ergon::ToolRegistry` over `ToolHarnessPort` |
| `hooks`           | `KernelCapabilityResolver` (always real) + the three noop fallbacks used when the host wires no real adapter via `WorkflowRunInputs` |
| `runner`          | `run_workflow_as_tick` — the workflow body executor |
| `dispatcher`      | `ErgonWorkflowDispatcher` — `WorkflowTickDispatcher` impl wired into the kernel |

## Critical invariants

1. **Capability gating fires once.** It lives in `KernelCapabilityResolver`
   (a `Hook::on_pre_tool_use` adapter). The `ToolHarnessAdapter` does
   NOT call `policy_gate.evaluate(...)` — pushing it down would
   double-trigger the gate.

2. **Workflows are addressed by string name.** The kernel hands us
   `TickKind::Workflow { name, input }`. The registry resolves
   `name` → boxed executor; the boxed executor handles JSON
   `from_value`/`to_value` boundary translation.

3. **Failures from inside ergon → `AdapterError::Workflow`.** Failures
   from the ports → `AdapterError::Port`. JSON-boundary failures →
   `AdapterError::{InputDeserialize, OutputSerialize}`. Each gets a
   distinct variant so observability can categorize them.

4. **The hook registry composes auto-hooks first, user hooks after.**
   This is the spec's required ordering. User hooks aren't yet
   plumbed; when they are, they go AFTER the four `with(...)` calls
   in `run_workflow_as_tick`.

5. **No `unwrap()` / `expect()` / `panic!()` outside test code.**
   Workspace clippy lints catch these. The one exception:
   `WorkflowRegistry::register` panics on duplicate names —
   registries are built at startup and silent overwrites would mask
   configuration bugs.

## Spec deviations (documented)

1. **`KernelCapabilityResolver` requires a `ToolCapabilityMap`.** The
   spec assumed `praxis_core::ToolRegistry` would advertise
   per-tool capabilities natively. It doesn't yet, so the BRO-1001
   minimum supplies the map externally (arcand will compute it from
   the registered praxis tools at startup). Tools missing from the
   map are denied fail-closed.

2. **Real auto-hook adapters wire in via `WorkflowRunInputs`**
   *(closed 2026-06-10, harness Phase-2 gap closure)*. The runner
   uses host-supplied `budget_gate` / `response_scorer` /
   `soul_attester` when present and falls back to the permissive
   noops otherwise (tests, minimal embedders). The real adapters:
   `arcan::ergon_wiring::EconomicBudgetGate` (Autonomic economic
   advisory — Hibernate denies, Hustle clamps `max_tokens`),
   `ergon_nous_adapter::NousAdapter` (evaluator-registry fan-out,
   fail-open per evaluator), `ergon_anima_adapter::
   AgentAttestationAdapter` (custody-signed JWS session boundaries —
   implemented + tested; arcan serve wires it once a custody
   identity config exists, until then the noop fallback warns at
   boot).

3. **Durable stream sink fans in via `WorkflowRunInputs::
   stream_sink_factory`** *(closed 2026-06-10)*. The runner composes
   `FanoutSink([BufferSink, factory(session, branch)])` per
   invocation — the buffer still feeds `events_emitted` accounting;
   the factory-built sink (arcan serve wires
   `ergon_life_sinks::LagoSink` over the same lago journal the
   kernel's `EventStorePort` uses) makes workflow stream events
   visible to `lago replay`. Direct `EventStorePort::append` from
   the dispatcher is deliberately NOT used: the kernel owns a
   per-branch sequence counter and the store enforces strict
   sequence continuity — dispatcher-side appends would desync it.
   lago assigns its own sequences, so the `lago_core::Journal` path
   is collision-free.

## Useful commands

```bash
cargo check -p arcan-ergon
cargo test  -p arcan-ergon --all-targets
cargo clippy -p arcan-ergon --all-targets -- -D warnings
cargo fmt -p arcan-ergon
```

## Don't

- Do not add a dependency on `praxis-core` or any praxis-* crate. The
  capability map flows in via `WorkflowRunInputs`; tools and tool
  definitions flow in via the same struct. The `arcan-ergon` boundary
  is `aios-protocol` ports + `ergon` traits, no substrate crates.
- Do not add `lago-*` or `vigil-*` deps to this crate. The
  substrate-coupled stream sinks live in `ergon-life-sinks` (or its
  successor). When they're plumbed in, it's via `WorkflowRunInputs`,
  not new direct deps here.
- Do not touch `arcan-harness` from this crate. Workflows replace
  the *tick body*, not arcan-harness.
- Do not bypass the kernel's `WorkflowTickDispatcher` trait. The
  whole point of BRO-1001 is that the kernel calls into a registered
  dispatcher via a typed callback — there's no other entry path.
