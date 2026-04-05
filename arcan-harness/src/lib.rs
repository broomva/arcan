//! Arcan tool harness — bridges Praxis canonical tools into Arcan's runtime.
//!
//! The actual tool implementations live in the `praxis-tools` crate.
//! This crate provides [`bridge::PraxisToolBridge`] which adapts any
//! `aios_protocol::tool::Tool` into `arcan_core::runtime::Tool`.

pub mod bridge;

// Re-export praxis types for convenience in downstream crates.
pub use praxis_core::sandbox::{CommandRunner, LocalCommandRunner, SandboxPolicy};
pub use praxis_core::workspace::FsPolicy;
pub use praxis_core::{FsDirEntry, FsMetadata, FsPort, LocalFs};
