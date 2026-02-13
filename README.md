# Arcan

Rust-first agent runtime and daemon scaffold focused on harness quality, typed streaming events, and replayable state.

## Workspace

- `crates/arcan-core`: protocol, state, runtime contracts, orchestrator loop
- `crates/arcan-harness`: sandbox and filesystem guardrails, hashline edit primitives
- `crates/arcan-store`: append-only session event repositories
- `crates/arcan-daemon`: `agentd` surface and SSE encoding helpers

## Docs

- `docs/architecture.md`
- `docs/roadmap.md`

## Verify

```bash
cargo check
cargo test
```
