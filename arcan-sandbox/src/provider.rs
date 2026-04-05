//! The `SandboxProvider` trait — the central abstraction for all sandbox backends.
//!
//! Provider implementations (Vercel, E2B, Local, Bubblewrap) live in their
//! own crates and depend on this crate. Arcan tool code depends only on this
//! trait; swapping providers is a configuration change, not a refactor.

use async_trait::async_trait;

use crate::capability::SandboxCapabilitySet;
use crate::error::SandboxError;
use crate::types::{
    ExecRequest, ExecResult, FileWrite, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec,
    SnapshotId,
};

/// Provider-agnostic interface for sandbox lifecycle management.
///
/// # Dyn-safety
///
/// The trait uses [`async_trait`] to make async methods object-safe. Wrap
/// implementors in `Arc<dyn SandboxProvider>` for runtime dispatch.
///
/// # Provider contract
///
/// - `create()` returns immediately with a `SandboxHandle`; the sandbox may
///   still be in `Starting` state. Use `list()` or a status poll to wait.
/// - `resume()` restores a snapshotted sandbox. Providers that don't support
///   persistence MUST return `SandboxError::NotSupported`.
/// - `snapshot()` MUST be idempotent: calling it twice on a running sandbox
///   produces two distinct `SnapshotId`s.
/// - `destroy()` MUST succeed even if the sandbox is already stopped.
#[async_trait]
pub trait SandboxProvider: Send + Sync + 'static {
    /// Stable, unique name used for config routing and observability labels.
    ///
    /// Examples: `"local"`, `"vercel"`, `"e2b"`, `"bubblewrap"`.
    fn name(&self) -> &'static str;

    /// Capability bits this provider can honour.
    ///
    /// Arcan checks this before forwarding a spec — any spec capability not
    /// in this set causes `create()` to return `SandboxError::CapabilityDenied`.
    fn capabilities(&self) -> SandboxCapabilitySet;

    /// Provision a new sandbox from the given spec.
    ///
    /// Returns immediately once the provider has accepted the request. The
    /// returned handle's `status` may be `Starting` if the sandbox is not
    /// yet ready.
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError>;

    /// Resume a previously snapshotted sandbox.
    ///
    /// Providers that do not support persistence MUST return
    /// [`SandboxError::NotSupported`].
    async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError>;

    /// Execute a command inside a running sandbox and return the result.
    ///
    /// If the sandbox is in `Snapshotted` state the provider SHOULD
    /// transparently resume it before executing.
    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError>;

    /// Snapshot the current filesystem state and return an opaque ID.
    ///
    /// The sandbox continues running after a snapshot. Use `destroy()` to
    /// stop it.
    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError>;

    /// Permanently destroy a sandbox and all associated state.
    ///
    /// MUST succeed even if the sandbox is already stopped or not found.
    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError>;

    /// List all sandboxes currently visible to this provider.
    ///
    /// Used by `SandboxService` for periodic reconciliation (BRO-253).
    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError>;

    /// Write one or more files into the sandbox filesystem.
    ///
    /// Providers that do not support this operation return
    /// [`SandboxError::NotSupported`] via this default implementation.
    async fn write_files(
        &self,
        _id: &SandboxId,
        _files: Vec<FileWrite>,
    ) -> Result<(), SandboxError> {
        Err(SandboxError::NotSupported {
            provider: self.name(),
            reason: "write_files",
        })
    }

    /// Read the raw bytes of a file from the sandbox filesystem.
    ///
    /// Providers that do not support this operation return
    /// [`SandboxError::NotSupported`] via this default implementation.
    async fn read_file(&self, _id: &SandboxId, _path: &str) -> Result<Vec<u8>, SandboxError> {
        Err(SandboxError::NotSupported {
            provider: self.name(),
            reason: "read_file",
        })
    }
}
