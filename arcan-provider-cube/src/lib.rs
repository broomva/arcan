//! `arcan-provider-cube` — [`HypervisorBackend`] implementation backed by
//! the CubeSandbox HTTP API v1.
//!
//! See `README.md` for backend identity, env vars, and capability set.
//! Real impl lands in Tasks 4–8 of the Phase 3 plan.
//!
//! [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod types;

pub use error::CubeError;
