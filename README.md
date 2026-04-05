# Arcan

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-passing-green.svg)](#)
[![docs](https://img.shields.io/badge/docs-broomva.tech-purple.svg)](https://docs.broomva.tech/docs/life/arcan)

**Core agent runtime for the Life Agent OS** -- the foundation primitive that implements the aiOS kernel contract with event-sourced state, typed streaming, and replayable sessions.

Rust-first agent runtime and daemon focused on harness quality, typed streaming events, and replayable state.

## Install

```bash
cargo install arcan
```

> [!IMPORTANT]
> Be sure to add `/Users/broomva/.cargo/bin` to your `PATH` to be able to run the installed binaries.


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

### Dev Mode (Daemon + TUI)

Starts the daemon, waits for health check, and then launches the TUI.

```bash
# Run the dev harness
./scripts/harness/dev-tui.sh
```

Environment overrides:

```bash
# custom port/session/data directory
PORT=3200 SESSION=dev-1 DATA_DIR=/tmp/arcan-dev ./scripts/harness/dev-tui.sh

# use real provider env vars instead of mock mode
ARCAN_MOCK=0 ./scripts/harness/dev-tui.sh
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

## Documentation

Full documentation: [docs.broomva.tech/docs/life/arcan](https://docs.broomva.tech/docs/life/arcan)

## License

[MIT](LICENSE)
