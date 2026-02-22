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
# Launch interactive TUI (default command).
# Re-attaches to the most recent session automatically.
cargo run -p arcan

# Run daemon explicitly
cargo run -p arcan -- serve

# Run daemon with Anthropic Claude
ANTHROPIC_API_KEY=sk-ant-... cargo run -p arcan -- serve

# Launch TUI explicitly
arcan chat

# CLI options
arcan --port 3000 --data-dir .arcan
```

### Dev Watch Mode (Daemon + TUI)

```bash
# one-time install
cargo install cargo-watch

# starts daemon in watch mode, waits for /health, then opens TUI
./scripts/harness/dev-tui-watch.sh
```

Environment overrides:

```bash
# custom port/session/data directory
PORT=3200 SESSION=dev-1 DATA_DIR=/tmp/arcan-dev ./scripts/harness/dev-tui-watch.sh

# use real provider env vars instead of mock mode
ARCAN_MOCK=0 ./scripts/harness/dev-tui-watch.sh
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
