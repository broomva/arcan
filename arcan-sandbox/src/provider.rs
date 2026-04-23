//! The `SandboxProvider` trait — deprecated compat shim over
//! [`aios_protocol::HypervisorBackend`].
//!
//! # Migration (BRO-852)
//!
//! Starting with `arcan-sandbox` 0.2.0 the `SandboxProvider` trait is a
//! **transitional alias**. New code should implement (and consume)
//! [`HypervisorBackend`] from `aios-protocol`; a blanket impl makes every
//! `HypervisorBackend` automatically satisfy `SandboxProvider`, so existing
//! callers keep working while the workspace migrates. See
//! `crates/aios/aios-protocol/src/hypervisor.rs` for the canonical contract.
//!
//! Filesystem operations (`write_files` / `read_file`) do **not** forward to
//! [`HypervisorFilesystemExt`] from the blanket — Rust has no specialisation.
//! Backends that want filesystem semantics through this shim should keep
//! their manual `impl SandboxProvider` until the full cut-over.

use async_trait::async_trait;
use chrono::Utc;

use aios_protocol::hypervisor::{
    BackendId, HypervisorBackend, VmHandle, VmId, VmResources, VmSpec, VmStatus,
};
use aios_protocol::ids::{AgentId, SessionId};

use crate::capability::SandboxCapabilitySet;
use crate::error::SandboxError;
use crate::types::{
    ExecRequest, ExecResult, FileWrite, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec,
    SandboxStatus, SnapshotId,
};

/// Provider-agnostic interface for sandbox lifecycle management.
///
/// # Deprecated (BRO-852)
///
/// Prefer [`aios_protocol::HypervisorBackend`]. A blanket impl forwards every
/// `HypervisorBackend` into this trait so legacy callers keep working.
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
#[deprecated(
    since = "0.2.0",
    note = "use aios_protocol::HypervisorBackend — SandboxProvider is retained as a transitional alias"
)]
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

// ── Blanket impl: HypervisorBackend → SandboxProvider ─────────────────────────
//
// BRO-852: any `HypervisorBackend` is automatically a `SandboxProvider`. The
// conversion preserves the legacy ID-based surface by reconstituting minimal
// `VmHandle`s from `SandboxId`s. Filesystem operations fall through to the
// `SandboxError::NotSupported` default — filesystem-aware backends should
// keep a manual `impl SandboxProvider` until callers migrate to
// `HypervisorFilesystemExt` directly.

/// Compat key baked into every reconstituted `VmHandle` so tracing picks up
/// that the call originated in the legacy shim rather than real metadata.
const COMPAT_SESSION_ID: &str = "arcan-sandbox-compat";
const COMPAT_AGENT_ID: &str = "arcan-sandbox-compat";

fn compat_vm_handle<T: HypervisorBackend + ?Sized>(provider: &T, id: &SandboxId) -> VmHandle {
    VmHandle {
        vm_id: VmId(id.0.clone()),
        backend: BackendId::from(provider.name()),
        session_id: SessionId::from_string(COMPAT_SESSION_ID),
        agent_id: AgentId::from_string(COMPAT_AGENT_ID),
        status: VmStatus::Running,
        created_at: Utc::now(),
        metadata: serde_json::Value::Null,
    }
}

fn sandbox_status_from_vm(status: VmStatus) -> SandboxStatus {
    match status {
        VmStatus::Starting => SandboxStatus::Starting,
        VmStatus::Running => SandboxStatus::Running,
        VmStatus::Snapshotted | VmStatus::Hibernated => SandboxStatus::Snapshotted,
        VmStatus::Stopping => SandboxStatus::Stopping,
        VmStatus::Stopped => SandboxStatus::Stopped,
        VmStatus::Failed { reason } => SandboxStatus::Failed { reason },
        // `VmStatus` is `#[non_exhaustive]`; future variants degrade to a
        // generic Failed state rather than breaking the build.
        other => SandboxStatus::Failed {
            reason: format!("unmapped vm status: {other:?}"),
        },
    }
}

fn sandbox_handle_from_vm(handle: VmHandle, fallback_name: String) -> SandboxHandle {
    let name = handle
        .metadata
        .as_object()
        .and_then(|m| m.get("sandbox.name"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or(fallback_name);
    SandboxHandle {
        id: SandboxId(handle.vm_id.0),
        name,
        status: sandbox_status_from_vm(handle.status),
        created_at: handle.created_at,
        provider: handle.backend.0,
        metadata: handle.metadata,
    }
}

fn vm_resources_from_sandbox(r: &crate::types::SandboxResources) -> VmResources {
    VmResources {
        vcpus: r.vcpus,
        memory_kb: u64::from(r.memory_mb) * 1024,
        disk_kb: u64::from(r.disk_mb) * 1024,
        timeout_secs: r.timeout_secs,
    }
}

fn vm_spec_from_sandbox(spec: SandboxSpec) -> VmSpec {
    use aios_protocol::hypervisor::RuntimeHint;
    let runtime_hint = match spec.image {
        Some(image) if !image.is_empty() => RuntimeHint::Custom { image },
        _ => RuntimeHint::Shell,
    };
    let mut labels = spec.labels;
    labels.insert("sandbox.name".into(), spec.name.clone());
    VmSpec {
        backend_selector: Default::default(),
        resources: vm_resources_from_sandbox(&spec.resources),
        network_policy: Default::default(),
        mounts: Vec::new(),
        env: spec.env,
        runtime_hint,
        labels,
    }
}

fn vm_exec_request_from_sandbox(req: ExecRequest) -> aios_protocol::hypervisor::ExecRequest {
    aios_protocol::hypervisor::ExecRequest {
        command: req.command,
        working_dir: req.working_dir,
        env: req.env,
        timeout_secs: req.timeout_secs,
        stdin: req.stdin,
    }
}

fn exec_result_from_vm(r: aios_protocol::hypervisor::ExecResult) -> ExecResult {
    ExecResult {
        stdout: r.stdout,
        stderr: r.stderr,
        exit_code: r.exit_code,
        duration_ms: r.duration_ms,
    }
}

#[allow(deprecated)]
#[async_trait]
impl<T: HypervisorBackend> SandboxProvider for T {
    fn name(&self) -> &'static str {
        HypervisorBackend::name(self)
    }

    fn capabilities(&self) -> SandboxCapabilitySet {
        // Backend capabilities live on a different bitmask (BackendCapabilitySet).
        // The legacy surface does not need the full detail; we advertise the
        // default `FILESYSTEM_READ` so existing callers continue to see a
        // non-empty set. Callers that care about specifics should inspect
        // `HypervisorBackend::capabilities` directly.
        let backend_caps = HypervisorBackend::capabilities(self);
        let mut caps = SandboxCapabilitySet::empty();
        use aios_protocol::hypervisor::BackendCapabilitySet as B;
        if backend_caps.contains(B::FILESYSTEM_READ) {
            caps |= SandboxCapabilitySet::FILESYSTEM_READ;
        }
        if backend_caps.contains(B::FILESYSTEM_WRITE) {
            caps |= SandboxCapabilitySet::FILESYSTEM_WRITE;
        }
        if backend_caps.contains(B::NETWORK_EGRESS) {
            caps |= SandboxCapabilitySet::NETWORK_OUTBOUND;
        }
        if backend_caps.contains(B::NETWORK_INGRESS) {
            caps |= SandboxCapabilitySet::NETWORK_INBOUND;
        }
        if backend_caps.contains(B::PERSISTENCE) {
            caps |= SandboxCapabilitySet::PERSISTENCE;
        }
        if backend_caps.contains(B::CUSTOM_IMAGE) {
            caps |= SandboxCapabilitySet::CUSTOM_IMAGE;
        }
        if backend_caps.contains(B::GPU) {
            caps |= SandboxCapabilitySet::GPU;
        }
        if backend_caps.contains(B::TAGS) {
            caps |= SandboxCapabilitySet::TAGS;
        }
        caps
    }

    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let fallback_name = spec.name.clone();
        let vm_spec = vm_spec_from_sandbox(spec);
        let handle = HypervisorBackend::create(self, vm_spec)
            .await
            .map_err(SandboxError::from)?;
        Ok(sandbox_handle_from_vm(handle, fallback_name))
    }

    async fn resume(&self, _id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        // Legacy `resume(id)` has no counterpart on `HypervisorBackend` —
        // `HypervisorBackend::resume` takes a full `&VmHandle` and operates
        // on hibernated VMs, whereas this shim is called for snapshotted
        // sandboxes. Backends that need the legacy semantic should keep a
        // manual `impl SandboxProvider`.
        Err(SandboxError::NotSupported {
            provider: HypervisorBackend::name(self),
            reason: "resume via SandboxProvider (use HypervisorBackend::restore with VmSnapshotId)",
        })
    }

    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        let handle = compat_vm_handle(self, id);
        let vm_req = vm_exec_request_from_sandbox(req);
        let result = HypervisorBackend::exec(self, &handle, vm_req)
            .await
            .map_err(SandboxError::from)?;
        Ok(exec_result_from_vm(result))
    }

    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let handle = compat_vm_handle(self, id);
        let snap_id = HypervisorBackend::snapshot(self, &handle)
            .await
            .map_err(SandboxError::from)?;
        Ok(SnapshotId(snap_id.0))
    }

    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let handle = compat_vm_handle(self, id);
        HypervisorBackend::destroy(self, &handle)
            .await
            .map_err(SandboxError::from)
    }

    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        // `HypervisorBackend` itself does not expose a `list` method — that
        // moved to `HypervisorFilesystemExt` which we cannot dispatch to
        // from a generic `T` without specialisation. Return an empty slice
        // so legacy reconciliation loops keep working (they degrade to
        // "nothing to reconcile" rather than erroring).
        Ok(Vec::new())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(deprecated)]
mod blanket_impl_tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use aios_protocol::hypervisor::{
        BackendCapabilitySet, BackendError, ExecRequest as VmExecRequest,
        ExecResult as VmExecResult, VmHandle, VmSnapshotId, VmSpec,
    };
    use async_trait::async_trait;

    use super::*;
    use crate::types::SandboxResources;

    /// Minimal hypervisor backend for exercising the blanket impl.
    struct StubBackend {
        create_calls: AtomicUsize,
        exec_calls: AtomicUsize,
        snapshot_calls: AtomicUsize,
        destroy_calls: AtomicUsize,
        last_exec_handle: Mutex<Option<VmHandle>>,
    }

    impl StubBackend {
        fn new() -> Self {
            Self {
                create_calls: AtomicUsize::new(0),
                exec_calls: AtomicUsize::new(0),
                snapshot_calls: AtomicUsize::new(0),
                destroy_calls: AtomicUsize::new(0),
                last_exec_handle: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl HypervisorBackend for StubBackend {
        fn name(&self) -> &'static str {
            "stub-hv"
        }

        fn capabilities(&self) -> BackendCapabilitySet {
            BackendCapabilitySet::FILESYSTEM_READ
                | BackendCapabilitySet::FILESYSTEM_WRITE
                | BackendCapabilitySet::PERSISTENCE
        }

        async fn create(&self, spec: VmSpec) -> Result<VmHandle, BackendError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            Ok(VmHandle {
                vm_id: VmId::from("vm-1"),
                backend: BackendId::from("stub-hv"),
                session_id: SessionId::from_string("compat-test"),
                agent_id: AgentId::from_string("compat-test"),
                status: VmStatus::Running,
                created_at: Utc::now(),
                metadata: serde_json::json!({
                    "sandbox.name": spec.labels.get("sandbox.name").cloned().unwrap_or_default()
                }),
            })
        }

        async fn exec(
            &self,
            vm: &VmHandle,
            _req: VmExecRequest,
        ) -> Result<VmExecResult, BackendError> {
            self.exec_calls.fetch_add(1, Ordering::SeqCst);
            *self.last_exec_handle.lock().unwrap() = Some(vm.clone());
            Ok(VmExecResult {
                stdout: b"ok\n".to_vec(),
                stderr: Vec::new(),
                exit_code: 0,
                duration_ms: 7,
            })
        }

        async fn snapshot(&self, _vm: &VmHandle) -> Result<VmSnapshotId, BackendError> {
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            Ok(VmSnapshotId::from("snap-1"))
        }

        async fn restore(&self, _snap: &VmSnapshotId) -> Result<VmHandle, BackendError> {
            Err(BackendError::NotSupported {
                backend: "stub-hv",
                reason: "restore (test)",
            })
        }

        async fn destroy(&self, _vm: &VmHandle) -> Result<(), BackendError> {
            self.destroy_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn minimal_spec(name: &str) -> SandboxSpec {
        SandboxSpec {
            name: name.into(),
            image: None,
            resources: SandboxResources::default(),
            env: Default::default(),
            persistence: Default::default(),
            capabilities: SandboxCapabilitySet::FILESYSTEM_READ,
            labels: Default::default(),
        }
    }

    #[tokio::test]
    async fn blanket_name_forwards_to_backend() {
        let backend: &dyn SandboxProvider = &StubBackend::new();
        assert_eq!(backend.name(), "stub-hv");
    }

    #[tokio::test]
    async fn blanket_capabilities_translate_from_backend_bits() {
        let backend = StubBackend::new();
        let caps = SandboxProvider::capabilities(&backend);
        assert!(caps.contains(SandboxCapabilitySet::FILESYSTEM_READ));
        assert!(caps.contains(SandboxCapabilitySet::FILESYSTEM_WRITE));
        assert!(caps.contains(SandboxCapabilitySet::PERSISTENCE));
        assert!(!caps.contains(SandboxCapabilitySet::GPU));
    }

    #[tokio::test]
    async fn blanket_create_preserves_spec_name_in_handle() {
        let backend = StubBackend::new();
        let handle = SandboxProvider::create(&backend, minimal_spec("pilot"))
            .await
            .unwrap();
        assert_eq!(backend.create_calls.load(Ordering::SeqCst), 1);
        assert_eq!(handle.name, "pilot");
        assert_eq!(handle.provider, "stub-hv");
        assert!(matches!(handle.status, SandboxStatus::Running));
    }

    #[tokio::test]
    async fn blanket_run_reconstitutes_compat_handle() {
        let backend = StubBackend::new();
        let id = SandboxId("vm-from-legacy".into());
        let result = SandboxProvider::run(&backend, &id, ExecRequest::shell("true"))
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, b"ok\n");

        let recorded = backend
            .last_exec_handle
            .lock()
            .unwrap()
            .clone()
            .expect("exec should have received a handle");
        assert_eq!(recorded.vm_id.0, "vm-from-legacy");
        assert_eq!(recorded.backend.0, "stub-hv");
    }

    #[tokio::test]
    async fn blanket_snapshot_returns_backend_snapshot_id() {
        let backend = StubBackend::new();
        let snap = SandboxProvider::snapshot(&backend, &SandboxId("vm-1".into()))
            .await
            .unwrap();
        assert_eq!(snap.0, "snap-1");
        assert_eq!(backend.snapshot_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn blanket_destroy_is_idempotent_forward() {
        let backend = StubBackend::new();
        SandboxProvider::destroy(&backend, &SandboxId("vm-1".into()))
            .await
            .unwrap();
        assert_eq!(backend.destroy_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn blanket_resume_returns_not_supported() {
        let backend = StubBackend::new();
        let err = SandboxProvider::resume(&backend, &SandboxId("vm-1".into()))
            .await
            .unwrap_err();
        match err {
            SandboxError::NotSupported { provider, .. } => assert_eq!(provider, "stub-hv"),
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn blanket_list_returns_empty() {
        let backend = StubBackend::new();
        let v = SandboxProvider::list(&backend).await.unwrap();
        assert!(v.is_empty());
    }

    #[tokio::test]
    async fn blanket_write_files_falls_through_to_not_supported_default() {
        let backend = StubBackend::new();
        let err = SandboxProvider::write_files(
            &backend,
            &SandboxId("vm-1".into()),
            vec![FileWrite {
                path: "/tmp/x".into(),
                content: b"x".to_vec(),
                mode: 0o644,
            }],
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SandboxError::NotSupported { .. }));
    }
}
