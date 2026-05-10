//! Library surface of the `arcan` binary crate.
//!
//! The vast majority of `arcan` lives in the binary (`src/main.rs`)
//! plus its private submodules. The library target exists to give
//! integration tests under `tests/` a stable hook to exercise the
//! pieces of the binary that benefit from end-to-end coverage without
//! spawning a subprocess.
//!
//! Currently re-exported:
//!
//! - [`agent_cmd`] — handlers for the `arcan agent
//!   list/show/new/test --dry-run` subcommands. Shipped under
//!   BRO-1008 to give operators an offline tooling surface for the
//!   authored-agent substrate (see
//!   `core/life/docs/superpowers/specs/2026-05-09-bro-1006-authored-agents-architecture.md`
//!   §M5).
//!
//! Adding a module to this re-export list is a deliberate boundary
//! expansion. The default for binary-internal helpers is to stay
//! private to `main.rs`.

pub mod agent_cmd;
