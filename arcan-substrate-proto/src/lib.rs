//! Generated `arcan.v1` proto types — the substrate-plane wire contract
//! between lifed (via `arcan-proxy`) and arcand.
//!
//! All types live under `arcan::v1` (the proto package path). The
//! generated client + server stubs are used by `arcan-proxy` and the
//! `arcand::substrate` UDS server respectively. `aios.v1.*` types are
//! re-exported from `aios-proto` via `extern_path` (Spec C₂ §10.3).
//!
//! Reference: `docs/superpowers/specs/2026-04-25-life-runtime-architecture-spec.md`
//! and BRO-1016 (closes the Topology-B substrate-stub gap audit
//! captured in `research/entities/concept/topology-b-substrate-stub-gap.md`).

#![deny(unsafe_code)]
#![allow(missing_docs)] // generated code

#[allow(unused_qualifications, clippy::all)]
pub mod arcan {
    pub mod v1 {
        tonic::include_proto!("arcan.v1");
    }
}

// Re-export aios-proto for callers that want a single import path.
pub use aios_proto::aios as aios_v1;
