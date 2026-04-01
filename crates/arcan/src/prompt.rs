//! Re-export the liquid prompt system from `arcan-core`.
//!
//! The implementation now lives in `arcan_core::prompt` so that both the
//! shell REPL (this binary) and the daemon HTTP server (`arcand`) share
//! the same prompt builder.
//!
//! This module provides backward-compatible imports for code that previously
//! referenced `crate::prompt::*`.

pub use arcan_core::prompt::*;
