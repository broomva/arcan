# Arcan: Implementation Status

**Date**: 2026-02-22  
**Version**: 0.2.0 (canonical baseline active)

This document describes the active Arcan implementation state for `/Users/broomva/broomva.tech/live/arcan`.

---

## Current State

Arcan now runs as a canonical host stack:

- Host runtime: `aios-runtime`
- Contract boundary: `aios-protocol`
- Persistence backend: Lago via canonical event-store adapter
- Integration layer: `arcan-aios-adapters`
- HTTP runtime surface: `arcand::canonical`

All production runtime flow is aligned to the canonical session API family.

## Workspace Health

Current workspace gates pass:

- `cargo fmt`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo check --workspace`

Conformance-dependent Arcan checks also pass through:

- `/Users/broomva/broomva.tech/live/conformance/run.sh`

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

- Role: production daemon binary.
- Status: active canonical host.
- Behavior: composes runtime + adapters + Lago persistence and serves canonical routes.

### `arcand`

- Role: canonical API router crate.
- Status: active.
- Exports: `canonical`, `mock`.
- Integration tests: canonical API lifecycle, named session auto-create on run, canonical v6 stream replay path.

### `arcan-aios-adapters`

- Role: Arcan-to-canonical port adapter layer.
- Status: active.
- Adapters:
  - provider
  - tool harness
  - policy gate
  - approvals
  - memory

### `arcan-core`

- Role: runtime/protocol primitives used by Arcan internals and adapter implementations.
- Status: active and tested.

### `arcan-harness`

- Role: tool and sandbox/harness capabilities.
- Status: active and tested.

### `arcan-provider`

- Role: model provider implementations.
- Status: active and tested.

### `arcan-store`

- Role: repository abstractions/backends used by Arcan subsystems where applicable.
- Status: active and tested.

### `arcan-lago`

- Role: Lago bridge, projections, policy middleware, stream formatting helpers.
- Status: active and tested.

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

Dependency governance is audited by:

- `/Users/broomva/broomva.tech/live/scripts/architecture/verify_dependencies.sh`
- `make audit` from workspace root

---

## Validation Snapshot

Canonical host behavior validated by passing tests including:

1. Canonical session API round-trip.
2. Named-session run auto-creation behavior.
3. Canonical stream replay framing and Vercel v6 envelope/header path.
4. Arcan-Lago replay/bridge integration tests.

---

## Known Active Gaps

These are active engineering gaps in the current baseline (not migration items):

1. OS-level sandbox isolation remains a hardening target (beyond process-level policy controls).
2. Observability depth can be expanded across runtime/adapters.
3. Cross-project golden fixture breadth can be expanded further.
4. Canonical API intentionally does not expose runtime provider switching endpoints today.

---

## Baseline Checklist

- [x] Canonical runtime host path active in `arcan` daemon.
- [x] Canonical router active in `arcand`.
- [x] Adapter crate (`arcan-aios-adapters`) active.
- [x] Canonical session + approval endpoints consumed by TUI network client.
- [x] Workspace lint/build/test gates clean.
- [x] Conformance checks clean.

