//! HTTP API DTOs for arcand — schema-only crate.
//!
//! This crate intentionally contains **no runtime code**. It exists so
//! `life-kernel-facade` can depend on typed request/response shapes without
//! pulling in arcand's server runtime. Types are filled in by Phase 0 tasks
//! that mirror the canonical HTTP surface at
//! `core/life/crates/arcan/arcand/src/canonical.rs`.

#![forbid(unsafe_code)]
