# Arcan: Implementation Status

**Date**: 2026-03-28
**Version**: 0.2.0 (canonical baseline active)

This document describes the active Arcan implementation state.

---

## Current State

Arcan now runs as a canonical host stack:

- Host runtime: `aios-runtime`
- Contract boundary: `aios-protocol`
- Persistence backend: Lago via canonical event-store adapter
- Integration layer: `arcan-aios-adapters`
- HTTP runtime surface: `arcand::canonical`
- Sandbox execution: provider-agnostic chain (Vercel, Bubblewrap, Local)

All production runtime flow is aligned to the canonical session API family.

## Workspace Health

Current workspace gates pass:

- `cargo fmt`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo check --workspace`

---

## Canonical API Surface

Arcand exposes:

- `POST /sessions`
- `POST /sessions/{session_id}/runs`
- `GET /sessions/{session_id}/state`
- `GET /sessions/{session_id}/events`
- `GET /sessions/{session_id}/events/stream`
- `POST /sessions/{session_id}/branches`
- `GET /sessions/{session_id}/branches`
- `POST /sessions/{session_id}/branches/{branch_id}/merge`
- `POST /sessions/{session_id}/approvals/{approval_id}`

Streaming notes:

- Canonical event stream payloads are available from `/events/stream`.
- Vercel AI SDK v6 framing path is available via format query handling on canonical stream route.

---

## Crate Status

### `arcan`

- Role: production daemon binary (`cargo install arcan`).
- Status: active canonical host.
- Behavior: composes runtime + adapters + Lago persistence + sandbox routing and serves canonical routes.
- CLI UX: running `arcan` with no subcommand launches TUI and auto-attaches to the most recent session via `${data_dir}/last_session` (with journal fallback).
- Sandbox routing: `sandbox_router::build_sandbox_provider()` selects backend via `ARCAN_SANDBOX_BACKEND` env var (`vercel|local|bwrap|none`).
- Autonomic gating: optional `--autonomic-url` flag for advisory policy enforcement.

### `arcand`

- Role: canonical API router crate.
- Status: active.
- Exports: `canonical`, `mock`.
- Integration tests: canonical API lifecycle, named session auto-create on run, canonical v6 stream replay path, frozen system-prompt prefix reuse within a session.
- Session continuity: canonical `POST /sessions` and `POST /sessions/{session_id}/runs` persist `${runtime_root}/last_session` for CLI/TUI resume behavior.
- Prompt caching: canonical runs freeze the base system-prompt prefix on first session use so mid-session persona/skill-catalog changes do not invalidate provider prompt caches; new sessions recompute the prefix.

### `arcan-aios-adapters`

- Role: Arcan-to-canonical port adapter layer.
- Status: active (~85 tests).
- Adapters: provider, tool harness, policy gate, approvals, memory.
- Sandbox lifecycle: `SandboxLifecycleObserver` implements `ToolHarnessObserver` with tier-aware cleanup (anonymous: destroy, free/pro: snapshot, enterprise: no-op).

### `arcan-core`

- Role: runtime/protocol primitives used by Arcan internals and adapter implementations.
- Status: active and tested.

### `arcan-harness`

- Role: tool and sandbox/harness capabilities (filesystem ops, Hashline editing, sandboxing).
- Status: active and tested.

### `arcan-provider`

- Role: LLM model provider implementations (Anthropic Claude Messages API).
- Status: active and tested.

### `arcan-store`

- Role: repository abstractions/backends (append-only JSONL log, session history).
- Status: active and tested.

### `arcan-lago`

- Role: Lago bridge, projections, policy middleware, stream formatting helpers.
- Status: active and tested (~143 tests).
- Recent additions:
  - `sandbox_sink`: `LagoSandboxEventSink` with background mpsc channel; `spawn()` (logging-only) and `spawn_with_manifest()` (full blob store + manifest sync).
  - `sandbox_manifest`: `SandboxManifest` in-memory index + `sync_file_written()` for content-addressed blob persistence (BRO-258, in PR #28).
  - `remote_journal`: `RemoteLagoJournal` with lazy `reqwest::Client` (fixed Tokio runtime panic).

### `arcan-sandbox`

- Role: provider-agnostic sandbox execution trait and core protocol types.
- Status: active (~45 tests).
- Key exports: `SandboxProvider` trait (`create/resume/run/snapshot/destroy/list/write_files/read_file`), `SandboxSpec`, `ExecRequest`, `ExecResult`, `FileWrite`, `SandboxHandle`, `SandboxCapabilitySet` (bitflags), `SandboxEvent`/`SandboxEventKind`, `SandboxEventSink`, `SandboxSessionStore` + `InMemorySessionStore` (tier-aware TTLs).
- `JournaledSandboxProvider`: decorator emitting lifecycle events via `SandboxEventSink` with SHA-256 pre-hashing for `write_files` (BRO-258, in PR #28).

### `arcan-provider-vercel`

- Role: Vercel Sandbox v2 HTTP provider (Firecracker microVM isolation).
- Status: active (~12 tests).
- Features: named sandboxes (`SandboxId` = user-defined name), auto-resume via `GET /v2/sandboxes/{name}?resume=true`, auto-persistence (`persistent: true`), tag support, `VERCEL_PROJECT_ID` for project-scoped listing.
- Env vars: `VERCEL_TOKEN` (preferred) or `VERCEL_SANDBOX_API_KEY`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID`.
- Retry: `send_with_retry()` — 3 attempts with exponential backoff on HTTP 429/503.

### `arcan-provider-bubblewrap`

- Role: Linux namespace sandbox isolation via bubblewrap (`bwrap`) with plain subprocess fallback.
- Status: active (~9 tests).
- Features: workspace-dir sandbox model, tar-based snapshot/resume.

### `arcan-provider-local`

- Role: local process sandbox with Docker primary backend and nsjail fallback.
- Status: active (~4 tests).
- Auto-detects available backend via `from_env()`.

### `arcan-praxis`

- Role: bridge integrating Praxis canonical tools into Arcan's runtime.
- Status: active (~23 tests).
- Key exports: `SandboxCommandRunner` (sync→async bridge via `block_in_place`), `SandboxServiceRunner` (session-scoped), `SandboxSessionLifecycle`, `build_provider()` (reads `ARCAN_SANDBOX_PROVIDER`), `register_praxis_tools` / `register_praxis_tools_for_session`.

### `arcan-fleet`

- Role: vertical agent fleet for Life marketplace (Coding, Data Processing, Support agents).
- Status: active (~36 tests).
- Key exports: `VerticalConfig`, `AgentVertical`, `ToolPermissions`, `all_verticals()`, agent health monitoring, message queue with preemption.

### `arcan-anima`

- Role: bridge between Arcan agent runtime and Anima identity/soul layer.
- Status: active (~7 tests).
- Key exports: `reconstruct_agent_self`, `emit_soul_genesis`, `inject_anima_context`.

### `arcan-console`

- Role: embedded web admin console for Arcan agent runtime.
- Status: active (~3 tests).
- Key exports: `console_router`, `ConsoleConfig`.

### `arcan-spaces`

- Role: bridge between Arcan agent runtime and Spaces distributed networking.
- Status: active (~42 tests).
- Key exports: `HiveSpacesCoordinator`, `SpacesPort`, `register_spaces_tools`.
- Optional SpacetimeDB feature.

### `arcan-tui`

- Role: terminal client.
- Status: aligned with canonical session endpoints and canonical approval endpoint.
- Stream consumption: canonical event records + canonical v6 wrapper parsing support.

---

## Integration Boundaries

Arcan baseline boundaries:

1. Runtime boundary uses canonical protocol types.
2. Host runtime is `aios-runtime`.
3. Persistence is Lago-backed through canonical adapter path.
4. Adapter responsibilities are isolated in `arcan-aios-adapters`.
5. Sandbox execution is provider-agnostic via `arcan-sandbox` trait + per-provider crates.

---

## Validation Snapshot

Canonical host behavior validated by passing tests including:

1. Canonical session API round-trip.
2. Named-session run auto-creation behavior.
3. Canonical stream replay framing and Vercel v6 envelope/header path.
4. Frozen system-prompt prefix stays stable within a session and refreshes on new sessions.
5. Arcan-Lago replay/bridge integration tests.
6. Sandbox provider lifecycle (create/run/snapshot/destroy) across all backends.
7. Session store tier-aware TTL expiration (anonymous/free/pro/enterprise).

---

## Known Active Gaps

These are active engineering gaps in the current baseline:

1. **PR #28 (BRO-258)**: `JournaledSandboxProvider` + filesystem manifest sync — open with merge conflicts and lint CI failure. Needs rebase onto current `main`.
2. Observability depth can be expanded across runtime/adapters.
3. Cross-project golden fixture breadth can be expanded further.
4. Canonical API intentionally does not expose runtime provider switching endpoints today.
5. Lago journal wiring for non-`FileWritten` sandbox events (snapshotted, created, destroyed) remains a follow-up.
6. `SandboxManifest` persistence across process restarts (rebuild from Lago journal on startup) not yet implemented.

---

## Recent Changes (2026-03-27)

Major sandbox execution chain landed across 8 PRs:

| PR | Ticket | Summary |
|----|--------|---------|
| #22 | BRO-235 | `arcan-sandbox` crate — `SandboxProvider` trait + core types |
| #23 | BRO-235/242/244/245/247/250/252 | Full provider-agnostic sandbox chain |
| #20 | BRO-256 | `SandboxEventSink` + `LagoSandboxEventSink` |
| #26 | BRO-248 | Wire `VercelSandboxProvider` into `build_provider()` |
| #27 | BRO-263 | Vercel Sandbox v2 named-sandbox + auto-persistence |
| #24 | BRO-253/257 | `SandboxService` orchestration + Lago Postgres sink |
| #25 | BRO-259 | `SandboxServiceRunner` + session lifecycle wiring |
| #29 | — | Fix lazy `reqwest::Client` in `RemoteLagoJournal` (Railway crash-loop) |

---

## Baseline Checklist

- [x] Canonical runtime host path active in `arcan` daemon.
- [x] Canonical router active in `arcand`.
- [x] Adapter crate (`arcan-aios-adapters`) active.
- [x] Canonical session + approval endpoints consumed by TUI network client.
- [x] Workspace lint/build/test gates clean.
- [x] Provider-agnostic sandbox chain (Vercel, Bubblewrap, Local).
- [x] Sandbox event sink with Lago persistence.
- [x] Session-scoped sandbox execution via `SandboxServiceRunner`.
- [x] Tier-aware session store with TTL expiration.
- [ ] BRO-258: `JournaledSandboxProvider` + filesystem manifest (PR #28 — needs rebase).
- [ ] Sandbox manifest persistence across restarts.
