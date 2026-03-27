//! # arcan-praxis — Praxis Tool Integration for Arcan
//!
//! Configures and registers the full suite of Praxis canonical tools
//! (filesystem, editing, shell, memory) into Arcan's [`ToolRegistry`],
//! bridging them through the [`PraxisToolBridge`] adapter from arcan-harness.
//!
//! ## Usage
//!
//! ```ignore
//! use arcan_praxis::{PraxisConfig, register_praxis_tools};
//! use arcan_core::runtime::ToolRegistry;
//!
//! let config = PraxisConfig::new("/path/to/workspace");
//! let mut registry = ToolRegistry::default();
//! register_praxis_tools(&config, &mut registry);
//! ```
//!
//! ## Architecture
//!
//! This crate is intentionally thin. It:
//! 1. Accepts a [`PraxisConfig`] that captures workspace root + sandbox constraints
//! 2. Constructs Praxis tool instances with the appropriate [`FsPolicy`] and [`SandboxPolicy`]
//! 3. Wraps each tool in [`PraxisToolBridge`] to satisfy `arcan_core::runtime::Tool`
//! 4. Registers them all into an Arcan [`ToolRegistry`]
//!
//! The actual tool implementations live in `praxis-tools`. The adapter lives
//! in `arcan-harness`. This crate is the glue that wires them together.

pub mod config;
pub mod registry;
pub mod sandbox_runner;

pub use config::PraxisConfig;
pub use registry::{register_praxis_tools, register_praxis_tools_for_session};
pub use sandbox_runner::{
    SandboxCommandRunner, SandboxServiceRunner, SandboxSessionLifecycle, build_provider,
    derive_sandbox_spec,
};

// Re-export key types for convenience.
pub use arcan_harness::bridge::PraxisToolBridge;
pub use praxis_core::sandbox::SandboxPolicy;
pub use praxis_core::workspace::FsPolicy;
