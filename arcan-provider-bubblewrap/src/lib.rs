//! `arcan-provider-bubblewrap` — [`HypervisorBackend`] implementation wrapping
//! the [`bwrap`](https://github.com/containers/bubblewrap) binary with a plain
//! subprocess fallback.
//!
//! # Backend selection
//!
//! At construction time the provider probes `PATH` for the `bwrap` binary.
//! If found, every exec call isolates the command inside a minimal bubblewrap
//! namespace. If `bwrap` is absent the provider falls back to a plain Tokio
//! [`Command`](tokio::process::Command) confined to the sandbox workspace
//! directory — identical semantics to
//! [`LocalCommandRunner`](praxis_core::sandbox::LocalCommandRunner) but async.
//!
//! # Persistence
//!
//! Sandbox state lives in a per-sandbox subdirectory of `workspace_root`.
//! The historical [`SandboxProvider::snapshot`] / [`SandboxProvider::resume`]
//! pair (which tarballed / untarballed the directory) is **not** exposed on
//! the new kernel surface — [`HypervisorBackend::snapshot`] returns
//! [`BackendError::NotSupported`]. Bubblewrap is intended as an ephemeral
//! developer loop, not a persistence-grade backend.
//!
//! # Kernel ABI (BRO-855)
//!
//! This crate implements [`aios_protocol::hypervisor::HypervisorBackend`] and
//! [`aios_protocol::hypervisor::HypervisorFilesystemExt`] directly. The legacy
//! `arcan_sandbox::SandboxProvider` surface is reached via the blanket
//! `impl<T: HypervisorBackend> SandboxProvider for T` exported by
//! `arcan-sandbox`, so existing callers keep compiling while the workspace
//! migrates off the deprecated trait.
//!
//! [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend
//! [`BackendError`]: aios_protocol::hypervisor::BackendError
//! [`HypervisorBackend::snapshot`]: aios_protocol::hypervisor::HypervisorBackend::snapshot

use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, info, warn};
use uuid::Uuid;

use arcan_sandbox::error::SandboxError;
use arcan_sandbox::types::{
    ExecRequest, ExecResult, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus,
};

// ── Provider ────────────────────────────────────────────────────────────────

/// [`HypervisorBackend`] backed by bubblewrap (`bwrap`) or a plain subprocess.
///
/// Construct with [`BubblewrapProvider::new`] or
/// [`BubblewrapProvider::from_env`].
///
/// [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend
#[derive(Debug, Clone)]
pub struct BubblewrapProvider {
    /// Root directory where per-sandbox workspace directories are created.
    pub workspace_root: PathBuf,
    /// `true` when the `bwrap` binary was found in `PATH` at construction.
    pub use_bwrap: bool,
}

impl BubblewrapProvider {
    /// Create a provider with an explicit workspace root.
    ///
    /// Auto-detects whether `bwrap` is available in `PATH`.
    pub fn new(workspace_root: PathBuf) -> Self {
        let use_bwrap = which_bwrap().is_some();
        if use_bwrap {
            info!("bwrap binary found — using bubblewrap isolation");
        } else {
            warn!("bwrap binary not found — falling back to plain subprocess");
        }
        Self {
            workspace_root,
            use_bwrap,
        }
    }

    /// Create a provider reading `ARCAN_SANDBOX_ROOT` (default:
    /// `/tmp/arcan-sandboxes`) and auto-detecting `bwrap`.
    pub fn from_env() -> Self {
        let root = std::env::var("ARCAN_SANDBOX_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/arcan-sandboxes"));
        Self::new(root)
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    /// Absolute path of the workspace directory for `id`.
    fn workspace_dir(&self, id: &SandboxId) -> PathBuf {
        self.workspace_root.join(&id.0)
    }

    /// Derive the effective timeout for a request, falling back to the spec
    /// default (60 s).
    fn timeout_secs(req: &ExecRequest) -> u64 {
        req.timeout_secs.unwrap_or(60)
    }

    // ── SandboxProvider-semantics helpers (private) ─────────────────────────
    //
    // These mirror the method bodies of the (now removed) explicit
    // `SandboxProvider` impl. The new `HypervisorBackend` impl calls them
    // through type conversions so existing subprocess semantics stay intact.

    /// Create a new sandbox workspace directory.
    async fn bwrap_create(
        &self,
        spec: SandboxSpec,
    ) -> Result<arcan_sandbox::types::SandboxHandle, SandboxError> {
        let id = SandboxId(Uuid::new_v4().to_string());
        let workspace = self.workspace_dir(&id);

        fs::create_dir_all(&workspace)
            .await
            .map_err(|e| SandboxError::ProviderError {
                provider: "bubblewrap",
                message: format!("create workspace dir: {e}"),
            })?;

        debug!(sandbox_id = %id, workspace = %workspace.display(), "sandbox workspace created");

        Ok(arcan_sandbox::types::SandboxHandle {
            id,
            name: spec.name,
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: "bubblewrap".to_owned(),
            metadata: serde_json::json!({
                "workspace": workspace.display().to_string(),
                "use_bwrap": self.use_bwrap,
            }),
        })
    }

    /// Execute a command inside the sandbox workspace.
    async fn bwrap_run(
        &self,
        id: &SandboxId,
        req: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        let workspace = self.workspace_dir(id);
        if !workspace.exists() {
            return Err(SandboxError::NotFound(id.clone()));
        }

        if req.command.is_empty() {
            return Err(SandboxError::ProviderError {
                provider: "bubblewrap",
                message: "command vector is empty".into(),
            });
        }

        let timeout = std::time::Duration::from_secs(Self::timeout_secs(&req));
        let start = Instant::now();

        let mut cmd = build_command(&workspace, &req, self.use_bwrap);

        let result = tokio::time::timeout(timeout, cmd.output()).await;

        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code().unwrap_or(-1);
                debug!(sandbox_id = %id, exit_code, duration_ms = elapsed_ms, "exec completed");
                Ok(ExecResult {
                    stdout: output.stdout,
                    stderr: output.stderr,
                    exit_code,
                    duration_ms: elapsed_ms,
                })
            }
            Ok(Err(e)) => Err(SandboxError::ProviderError {
                provider: "bubblewrap",
                message: format!("spawn failed: {e}"),
            }),
            Err(_) => Err(SandboxError::ExecTimeout {
                sandbox_id: id.clone(),
                timeout_secs: Self::timeout_secs(&req),
            }),
        }
    }

    /// Remove the workspace directory.
    ///
    /// Succeeds even if it does not exist.
    async fn bwrap_destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let workspace = self.workspace_dir(id);

        if workspace.exists() {
            fs::remove_dir_all(&workspace)
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("remove workspace: {e}"),
                })?;
        }

        info!(sandbox_id = %id, "sandbox destroyed");
        Ok(())
    }

    /// Scan `workspace_root` for directories and return a [`SandboxInfo`] for
    /// each one found.
    async fn bwrap_list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        if !self.workspace_root.exists() {
            return Ok(vec![]);
        }

        let mut entries =
            fs::read_dir(&self.workspace_root)
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("read workspace_root: {e}"),
                })?;

        let mut infos = Vec::new();
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("read dir entry: {e}"),
                })?
        {
            let ft = entry
                .file_type()
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("file type: {e}"),
                })?;
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let created_at = entry
                .metadata()
                .await
                .and_then(|m| m.created())
                .map(|st| {
                    let dur = st.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                    let secs = dur.as_secs() as i64;
                    chrono::DateTime::from_timestamp(secs, 0).unwrap_or_else(Utc::now)
                })
                .unwrap_or_else(|_| Utc::now());

            infos.push(SandboxInfo {
                id: SandboxId(name.clone()),
                name,
                status: SandboxStatus::Running,
                created_at,
            });
        }

        Ok(infos)
    }
}

// ── HypervisorBackend impl (BRO-855) ──────────────────────────────────────────
//
// Explicit first-class impl of the canonical `aios_protocol::HypervisorBackend`
// contract. Delegates to the private `bwrap_*` helpers directly so there is
// no round-trip through the deprecated `SandboxProvider` shim.
//
// The deprecated `SandboxProvider` trait is still available to legacy callers
// via the blanket `impl<T: HypervisorBackend> SandboxProvider for T` exposed by
// `arcan-sandbox`; we no longer maintain an explicit `impl SandboxProvider`
// because (a) it would conflict with the blanket impl and (b) all of its
// behaviour is preserved by delegating to the helpers above.
//
// Capability reality-check:
//
// - `FILESYSTEM_READ` / `FILESYSTEM_WRITE` — the host filesystem under the
//   per-sandbox workspace directory is fully read/write via shell exec; the
//   capability bits are advertised so the kernel surfaces bubblewrap as a
//   filesystem-capable backend.
// - `FILESYSTEM_EXT` — advertised because this crate implements the extension
//   trait (including `list()` that enumerates workspace directories).
//   `write_files`/`read_file` currently return `NotSupported`; the direct-API
//   wiring is deferred because bubblewrap callers invariably use shell exec
//   for file IO already.
// - `NETWORK_EGRESS` — the plain subprocess fallback inherits full network
//   access. The `bwrap` branch uses `--unshare-net` but this is an OS-level
//   enforcement, not a capability advertised to the kernel router; advertising
//   `NETWORK_EGRESS` reflects what the backend *can* offer when callers do
//   not request network isolation explicitly via `VmSpec.network_policy`.
// - `PERSISTENCE` — intentionally **not** advertised. Bubblewrap is the
//   ephemeral developer loop; `snapshot`/`restore` return `NotSupported`.
// - `HIBERNATE` — intentionally **not** advertised. `hibernate` / `resume`
//   inherit the trait's default `NotSupported` impls.

#[async_trait]
impl aios_protocol::hypervisor::HypervisorBackend for BubblewrapProvider {
    fn name(&self) -> &'static str {
        "bubblewrap"
    }

    fn capabilities(&self) -> aios_protocol::hypervisor::BackendCapabilitySet {
        use aios_protocol::hypervisor::BackendCapabilitySet;
        BackendCapabilitySet::FILESYSTEM_READ
            | BackendCapabilitySet::FILESYSTEM_WRITE
            | BackendCapabilitySet::FILESYSTEM_EXT
            | BackendCapabilitySet::NETWORK_EGRESS
    }

    async fn create(
        &self,
        spec: aios_protocol::hypervisor::VmSpec,
    ) -> Result<aios_protocol::hypervisor::VmHandle, aios_protocol::hypervisor::BackendError> {
        use aios_protocol::hypervisor::{BackendId, VmHandle, VmId};

        let sandbox_spec = vm_spec_to_sandbox_spec(&spec);
        let handle = self
            .bwrap_create(sandbox_spec)
            .await
            .map_err(backend_error_from_sandbox)?;

        Ok(VmHandle {
            vm_id: VmId(handle.id.0),
            backend: BackendId::from("bubblewrap"),
            session_id: session_id_from_spec(&spec),
            agent_id: agent_id_from_spec(&spec),
            status: vm_status_from_sandbox(handle.status),
            created_at: handle.created_at,
            metadata: handle.metadata,
        })
    }

    async fn exec(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
        req: aios_protocol::hypervisor::ExecRequest,
    ) -> Result<aios_protocol::hypervisor::ExecResult, aios_protocol::hypervisor::BackendError>
    {
        let id = SandboxId(vm.vm_id.0.clone());
        let legacy_req = vm_exec_request_to_sandbox(req);
        let legacy = self
            .bwrap_run(&id, legacy_req)
            .await
            .map_err(backend_error_from_sandbox)?;
        Ok(aios_protocol::hypervisor::ExecResult {
            stdout: legacy.stdout,
            stderr: legacy.stderr,
            exit_code: legacy.exit_code,
            duration_ms: legacy.duration_ms,
        })
    }

    /// Bubblewrap does not implement snapshots through the kernel surface —
    /// the legacy tarball-based `SandboxProvider::snapshot` is retired with
    /// BRO-855. Callers that need persistence should use `arcan-provider-local`
    /// (Docker/nsjail) or `arcan-provider-vercel`.
    async fn snapshot(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<aios_protocol::hypervisor::VmSnapshotId, aios_protocol::hypervisor::BackendError>
    {
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "bubblewrap",
            reason: "snapshot",
        })
    }

    async fn restore(
        &self,
        _snapshot: &aios_protocol::hypervisor::VmSnapshotId,
    ) -> Result<aios_protocol::hypervisor::VmHandle, aios_protocol::hypervisor::BackendError> {
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "bubblewrap",
            reason: "restore",
        })
    }

    async fn destroy(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        let id = SandboxId(vm.vm_id.0.clone());
        self.bwrap_destroy(&id)
            .await
            .map_err(backend_error_from_sandbox)
    }

    // `hibernate` / `resume` inherit the trait's `BackendError::NotSupported`
    // defaults — bubblewrap has no pause-in-place semantics.
}

#[async_trait]
impl aios_protocol::hypervisor::HypervisorFilesystemExt for BubblewrapProvider {
    async fn write_files(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _files: Vec<aios_protocol::hypervisor::FileWrite>,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        // Bubblewrap callers use shell exec for file IO; a direct-API path
        // is not wired. The legacy `SandboxProvider::write_files` default
        // returned `NotSupported` and we preserve that semantic.
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "bubblewrap",
            reason: "write_files",
        })
    }

    async fn read_file(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _path: &str,
    ) -> Result<Vec<u8>, aios_protocol::hypervisor::BackendError> {
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "bubblewrap",
            reason: "read_file",
        })
    }

    async fn list(
        &self,
    ) -> Result<Vec<aios_protocol::hypervisor::VmInfo>, aios_protocol::hypervisor::BackendError>
    {
        let legacy = self
            .bwrap_list()
            .await
            .map_err(backend_error_from_sandbox)?;

        Ok(legacy
            .into_iter()
            .map(|info| aios_protocol::hypervisor::VmInfo {
                vm_id: aios_protocol::hypervisor::VmId(info.id.0),
                backend: aios_protocol::hypervisor::BackendId::from("bubblewrap"),
                status: vm_status_from_sandbox(info.status),
                created_at: info.created_at,
            })
            .collect())
    }
}

// ── Conversion helpers (VmSpec ↔ SandboxSpec, SandboxError → BackendError) ────

/// Translate a kernel-level [`aios_protocol::hypervisor::VmSpec`] into the
/// legacy [`SandboxSpec`] consumed by the private `bwrap_*` helpers. The
/// conversion is lossy for mounts/network_policy (which the shell helpers do
/// not honour) but preserves the fields that actually drive provisioning.
fn vm_spec_to_sandbox_spec(spec: &aios_protocol::hypervisor::VmSpec) -> SandboxSpec {
    use aios_protocol::hypervisor::RuntimeHint;
    use arcan_sandbox::types::{SandboxResources, SandboxSpec};

    let image = match &spec.runtime_hint {
        RuntimeHint::Custom { image } if !image.is_empty() => Some(image.clone()),
        _ => None,
    };

    let name = spec
        .labels
        .get("sandbox.name")
        .cloned()
        .unwrap_or_else(|| format!("bwrap-{}", chrono::Utc::now().timestamp_millis()));

    SandboxSpec {
        name,
        image,
        resources: SandboxResources {
            vcpus: spec.resources.vcpus,
            memory_mb: (spec.resources.memory_kb / 1024).min(u64::from(u32::MAX)) as u32,
            disk_mb: (spec.resources.disk_kb / 1024).min(u64::from(u32::MAX)) as u32,
            timeout_secs: spec.resources.timeout_secs,
        },
        env: spec.env.clone(),
        persistence: arcan_sandbox::types::PersistencePolicy::Ephemeral,
        capabilities: arcan_sandbox::capability::SandboxCapabilitySet::FILESYSTEM_READ
            | arcan_sandbox::capability::SandboxCapabilitySet::FILESYSTEM_WRITE,
        labels: spec.labels.clone(),
    }
}

/// Translate a kernel-level [`aios_protocol::hypervisor::ExecRequest`] into the
/// legacy [`arcan_sandbox::types::ExecRequest`] used by the private helpers.
/// The two structs have identical fields today; the copy keeps the coupling
/// explicit should they diverge.
fn vm_exec_request_to_sandbox(
    req: aios_protocol::hypervisor::ExecRequest,
) -> arcan_sandbox::types::ExecRequest {
    arcan_sandbox::types::ExecRequest {
        command: req.command,
        working_dir: req.working_dir,
        env: req.env,
        timeout_secs: req.timeout_secs,
        stdin: req.stdin,
    }
}

/// Map a legacy [`SandboxStatus`] to the canonical
/// [`aios_protocol::hypervisor::VmStatus`].
fn vm_status_from_sandbox(status: SandboxStatus) -> aios_protocol::hypervisor::VmStatus {
    use aios_protocol::hypervisor::VmStatus;
    match status {
        SandboxStatus::Starting => VmStatus::Starting,
        SandboxStatus::Running => VmStatus::Running,
        SandboxStatus::Snapshotted => VmStatus::Snapshotted,
        SandboxStatus::Stopping => VmStatus::Stopping,
        SandboxStatus::Stopped => VmStatus::Stopped,
        SandboxStatus::Failed { reason } => VmStatus::Failed { reason },
    }
}

/// Bridge from the legacy [`SandboxError`] (returned by internal helpers) to
/// the canonical [`aios_protocol::hypervisor::BackendError`].
///
/// Symmetric counterpart to `From<BackendError> for SandboxError` in
/// `arcan-sandbox::error`. A local forward mapping avoids a circular
/// dependency that would arise from placing `From<SandboxError>` inside
/// `aios-protocol`.
fn backend_error_from_sandbox(e: SandboxError) -> aios_protocol::hypervisor::BackendError {
    use aios_protocol::hypervisor::{BackendError, VmId as NewVmId};
    match e {
        SandboxError::NotFound(id) => BackendError::VmNotFound(NewVmId(id.0)),
        SandboxError::NotSupported { provider, reason } => BackendError::NotSupported {
            backend: provider,
            reason,
        },
        SandboxError::ProviderError {
            provider: _,
            message,
        } => BackendError::Internal(message),
        SandboxError::ExecTimeout { timeout_secs, .. } => BackendError::Timeout {
            duration_ms: timeout_secs.saturating_mul(1_000),
        },
        SandboxError::CapabilityDenied { capability } => {
            BackendError::Internal(format!("capability denied: {capability}"))
        }
        SandboxError::Serialization(err) => BackendError::Internal(err.to_string()),
    }
}

/// Extract a [`SessionId`] from optional `session.id` labels on the spec.
fn session_id_from_spec(spec: &aios_protocol::hypervisor::VmSpec) -> aios_protocol::ids::SessionId {
    spec.labels
        .get("session.id")
        .map(|s| aios_protocol::ids::SessionId::from_string(s.as_str()))
        .unwrap_or_else(|| aios_protocol::ids::SessionId::from_string("arcan-provider-bubblewrap"))
}

/// Extract an [`AgentId`] from optional `agent.id` labels on the spec.
fn agent_id_from_spec(spec: &aios_protocol::hypervisor::VmSpec) -> aios_protocol::ids::AgentId {
    spec.labels
        .get("agent.id")
        .map(|s| aios_protocol::ids::AgentId::from_string(s.as_str()))
        .unwrap_or_else(|| aios_protocol::ids::AgentId::from_string("arcan-provider-bubblewrap"))
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Return the path to `bwrap` if it can be found in `PATH`.
fn which_bwrap() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join("bwrap");
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

/// Build a [`tokio::process::Command`] for the given request.
///
/// When `use_bwrap` is `true` the command is wrapped with `bwrap` arguments
/// that bind-mount the workspace and common system directories.
fn build_command(workspace: &Path, req: &ExecRequest, use_bwrap: bool) -> Command {
    if use_bwrap {
        let mut cmd = Command::new("bwrap");
        cmd.args([
            "--bind",
            workspace.to_str().unwrap_or("/workspace"),
            "/workspace",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--ro-bind",
            "/usr",
            "/usr",
            "--ro-bind",
            "/lib",
            "/lib",
        ]);

        // /lib64 may not exist on all systems; skip gracefully
        if Path::new("/lib64").exists() {
            cmd.args(["--ro-bind", "/lib64", "/lib64"]);
        }

        cmd.args([
            "--ro-bind",
            "/bin",
            "/bin",
            "--unshare-net",
            "--new-session",
            "--",
        ]);

        cmd.args(&req.command);
        apply_exec_env(&mut cmd, req);
        cmd
    } else {
        let mut cmd = Command::new(&req.command[0]);
        cmd.args(&req.command[1..]);
        cmd.current_dir(workspace);
        apply_exec_env(&mut cmd, req);
        cmd
    }
}

/// Apply environment variables and optional working directory from the request.
fn apply_exec_env(cmd: &mut Command, req: &ExecRequest) {
    for (k, v) in &req.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &req.working_dir {
        cmd.current_dir(dir);
    }
}

// ── Legacy tests (via SandboxProvider blanket impl) ──────────────────────────

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use arcan_sandbox::provider::SandboxProvider;

    fn provider_at(root: &Path) -> BubblewrapProvider {
        BubblewrapProvider {
            workspace_root: root.to_path_buf(),
            use_bwrap: false,
        }
    }

    #[tokio::test]
    async fn create_and_destroy_creates_and_removes_workspace_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = SandboxProvider::create(&provider, SandboxSpec::ephemeral("test"))
            .await
            .unwrap();
        let workspace = provider.workspace_dir(&handle.id);
        assert!(
            workspace.exists(),
            "workspace dir should exist after create"
        );

        SandboxProvider::destroy(&provider, &handle.id)
            .await
            .unwrap();
        assert!(
            !workspace.exists(),
            "workspace dir should be removed after destroy"
        );
    }

    #[tokio::test]
    async fn run_echo_command_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = SandboxProvider::create(&provider, SandboxSpec::ephemeral("echo-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["echo".into(), "hello".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(10),
            stdin: None,
        };

        let result = SandboxProvider::run(&provider, &handle.id, req)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0, "echo should exit 0");
        assert!(
            result.stdout_str().contains("hello"),
            "stdout should contain 'hello'"
        );

        SandboxProvider::destroy(&provider, &handle.id)
            .await
            .unwrap();
    }

    #[test]
    fn name_returns_bubblewrap() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        assert_eq!(SandboxProvider::name(&provider), "bubblewrap");
    }

    /// A non-zero exit code must be forwarded without an error.
    #[tokio::test]
    async fn run_exit_code_one() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = SandboxProvider::create(&provider, SandboxSpec::ephemeral("exit-code-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["/bin/sh".into(), "-c".into(), "exit 1".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(10),
            stdin: None,
        };

        let result = SandboxProvider::run(&provider, &handle.id, req)
            .await
            .unwrap();
        assert_eq!(result.exit_code, 1, "exit code 1 must be forwarded");

        SandboxProvider::destroy(&provider, &handle.id)
            .await
            .unwrap();
    }

    /// A command that exceeds its timeout must return `ExecTimeout`.
    #[tokio::test]
    async fn run_timeout_returns_exec_timeout_error() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = SandboxProvider::create(&provider, SandboxSpec::ephemeral("timeout-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["sleep".into(), "30".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(1),
            stdin: None,
        };

        let err = SandboxProvider::run(&provider, &handle.id, req)
            .await
            .unwrap_err();
        assert!(
            matches!(err, SandboxError::ExecTimeout { .. }),
            "expected ExecTimeout, got: {err:?}"
        );

        SandboxProvider::destroy(&provider, &handle.id)
            .await
            .unwrap();
    }
}

// ── HypervisorBackend trait tests (BRO-855) ──────────────────────────────────

#[cfg(test)]
mod kernel_tests {
    use super::*;
    use aios_protocol::hypervisor::{
        BackendCapabilitySet, BackendError, BackendId, HypervisorBackend, VmHandle, VmId,
        VmSnapshotId, VmStatus,
    };
    use aios_protocol::ids::{AgentId, SessionId};

    fn provider_at(root: &Path) -> BubblewrapProvider {
        BubblewrapProvider {
            workspace_root: root.to_path_buf(),
            use_bwrap: false,
        }
    }

    fn test_vm_handle() -> VmHandle {
        VmHandle {
            vm_id: VmId::from("vm-test"),
            backend: BackendId::from("bubblewrap"),
            session_id: SessionId::from_string("sess-test"),
            agent_id: AgentId::from_string("agent-test"),
            status: VmStatus::Running,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn bubblewrap_provider_impls_hypervisor_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        assert_eq!(HypervisorBackend::name(&provider), "bubblewrap");
        assert!(
            HypervisorBackend::capabilities(&provider)
                .contains(BackendCapabilitySet::FILESYSTEM_EXT),
            "bubblewrap provider must advertise FILESYSTEM_EXT now that \
             HypervisorFilesystemExt is implemented"
        );
    }

    #[test]
    fn bubblewrap_provider_advertises_expected_capability_set() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        let caps = HypervisorBackend::capabilities(&provider);
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_READ));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_WRITE));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_EXT));
        assert!(caps.contains(BackendCapabilitySet::NETWORK_EGRESS));
        // Bubblewrap is an ephemeral developer loop — no persistence / hibernate.
        assert!(
            !caps.contains(BackendCapabilitySet::PERSISTENCE),
            "bubblewrap must not advertise PERSISTENCE — snapshot/restore return NotSupported"
        );
        assert!(
            !caps.contains(BackendCapabilitySet::HIBERNATE),
            "bubblewrap must not advertise HIBERNATE — no pause-in-place semantics"
        );
    }

    #[tokio::test]
    async fn bubblewrap_snapshot_returns_not_supported() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        let handle = test_vm_handle();
        let err = HypervisorBackend::snapshot(&provider, &handle)
            .await
            .expect_err("snapshot should be NotSupported for BubblewrapProvider");
        match err {
            BackendError::NotSupported { backend, reason } => {
                assert_eq!(backend, "bubblewrap");
                assert_eq!(reason, "snapshot");
            }
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bubblewrap_restore_returns_not_supported() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        let err = HypervisorBackend::restore(&provider, &VmSnapshotId::from("snap-1"))
            .await
            .expect_err("restore should be NotSupported for BubblewrapProvider");
        match err {
            BackendError::NotSupported { backend, reason } => {
                assert_eq!(backend, "bubblewrap");
                assert_eq!(reason, "restore");
            }
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bubblewrap_hibernate_returns_not_supported_default() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        let handle = test_vm_handle();
        let err = HypervisorBackend::hibernate(&provider, &handle)
            .await
            .expect_err("default hibernate impl should return NotSupported");
        assert!(matches!(err, BackendError::NotSupported { .. }));
    }
}
