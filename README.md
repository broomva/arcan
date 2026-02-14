# Arcan

Rust-first agent runtime and daemon focused on harness quality, typed streaming events, and replayable state.

## Install

```bash
cargo install arcan
```

## Workspace

- `crates/arcan-core`: protocol, state, runtime contracts, orchestrator loop
- `crates/arcan-harness`: sandbox and filesystem guardrails, hashline edit primitives
- `crates/arcan-store`: append-only session event repositories
- `crates/arcan-provider`: LLM provider implementations (Anthropic Claude)
- `crates/arcand`: agent loop, SSE server, and HTTP routing
- `crates/arcan-lago`: Lago event-sourced persistence bridge
- `crates/arcan`: installable binary (`cargo install arcan`)

## Usage

```bash
# Run with mock provider
cargo run -p arcan

# Run with Anthropic Claude
ANTHROPIC_API_KEY=sk-ant-... cargo run -p arcan

# CLI options
arcan --port 3000 --data-dir .arcan --max-iterations 10
```

## Docs

- `docs/architecture.md`
- `docs/roadmap.md`
- `docs/vision-and-status.md`
- `docs/lago-integration.md`

## Verify

```bash
cargo check
cargo test
cargo clippy
```
