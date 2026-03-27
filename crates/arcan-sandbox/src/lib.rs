//! `arcan-sandbox` — provider-agnostic sandbox execution layer.
//!
//! This crate defines the [`SandboxProvider`] trait and all associated
//! protocol types. It has **zero external network dependencies** and is the
//! single source of truth for the sandbox contract.
//!
//! # Architecture
//!
//! ```text
//! arcan-sandbox          ← this crate: trait + types only
//!   ├── arcan-provider-vercel   ← VercelSandboxProvider
//!   ├── arcan-provider-e2b      ← E2BSandboxProvider
//!   ├── arcan-provider-local    ← LocalSandboxProvider (Docker/nsjail)
//!   └── arcan-provider-bwrap    ← BubblewrapProvider
//! ```
//!
//! # Quick start
//!
//! ```rust,ignore
//! use arcan_sandbox::{SandboxProvider, SandboxSpec};
//!
//! async fn run_hello(provider: &dyn SandboxProvider) {
//!     let handle = provider.create(SandboxSpec::ephemeral("hello")).await.unwrap();
//!     let result = provider.run(&handle.id, arcan_sandbox::ExecRequest::shell("echo hi")).await.unwrap();
//!     assert_eq!(result.stdout_str().trim(), "hi");
//!     provider.destroy(&handle.id).await.unwrap();
//! }
//! ```

pub mod capability;
pub mod error;
pub mod event;
pub mod provider;
pub mod session_store;
pub mod types;

// Flat re-exports for ergonomic use as `arcan_sandbox::SandboxProvider`, etc.
pub use capability::SandboxCapabilitySet;
pub use error::SandboxError;
pub use event::{SandboxEvent, SandboxEventKind};
pub use provider::SandboxProvider;
pub use session_store::{
    InMemorySessionStore, SandboxSessionStore, SandboxSessionStoreExt, UpstashSessionStore,
    tier_ttl,
};
pub use types::{
    ExecRequest, ExecResult, PersistencePolicy, SandboxHandle, SandboxId, SandboxInfo,
    SandboxResources, SandboxSpec, SandboxStatus, SnapshotId,
};
