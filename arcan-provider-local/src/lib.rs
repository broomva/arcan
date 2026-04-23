//! `arcan-provider-local` — [`HypervisorBackend`] with Docker primary backend
//! and nsjail fallback.
//!
//! # Backend selection
//!
//! [`LocalSandboxProvider::from_env`] probes the environment at construction:
//!
//! 1. If `/var/run/docker.sock` (or `$DOCKER_HOST`) exists → use Docker.
//! 2. If `nsjail` is in `PATH` → use nsjail.
//! 3. Otherwise → return an error.
//!
//! # Docker backend
//!
//! Each sandbox maps to a long-running Docker container named `arcan-{id}`.
//! The container is started with `sleep infinity` and commands are executed
//! via `docker exec`.
//!
//! # nsjail backend (fallback)
//!
//! When Docker is unavailable the provider falls back to `nsjail`, using a
//! workspace directory approach similar to `arcan-provider-bubblewrap`.
//!
//! # Kernel ABI (BRO-853)
//!
//! This crate implements [`aios_protocol::hypervisor::HypervisorBackend`] and
//! [`aios_protocol::hypervisor::HypervisorFilesystemExt`] directly. The legacy
//! `arcan_sandbox::SandboxProvider` surface is reached via the blanket
//! `impl<T: HypervisorBackend> SandboxProvider for T` exported by
//! `arcan-sandbox`, so existing callers keep compiling while the workspace
//! migrates off the deprecated trait.
//!
//! [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend

use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, info};
use uuid::Uuid;

use arcan_sandbox::error::SandboxError;
use arcan_sandbox::types::{
    ExecRequest, ExecResult, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus, SnapshotId,
};

// ── Backend ──────────────────────────────────────────────────────────────────

/// Describes which local isolation mechanism is in use.
#[derive(Debug, Clone)]
pub enum LocalBackend {
    /// Docker daemon is available at the given socket path.
    Docker {
        /// Path to the Docker daemon socket (e.g. `/var/run/docker.sock`).
        socket: PathBuf,
    },
    /// `nsjail` binary is available at the given path.
    Nsjail {
        /// Absolute path to the `nsjail` executable.
        nsjail_bin: PathBuf,
    },
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// Local sandbox provider with Docker primary backend and nsjail fallback.
///
/// Construct with [`LocalSandboxProvider::from_env`],
/// [`LocalSandboxProvider::with_docker`], or
/// [`LocalSandboxProvider::with_nsjail`].
#[derive(Debug, Clone)]
pub struct LocalSandboxProvider {
    /// Which backend is in use.
    pub backend: LocalBackend,
    /// Root directory where per-sandbox workspace directories are stored.
    pub workspace_root: PathBuf,
}

impl LocalSandboxProvider {
    /// Detect the best available local backend.
    ///
    /// 1. Checks `DOCKER_HOST` env var or the default socket path for Docker.
    /// 2. Falls back to searching `PATH` for `nsjail`.
    /// 3. Returns `Err` if neither is available.
    pub fn from_env() -> Result<Self, anyhow::Error> {
        let workspace_root = std::env::var("ARCAN_SANDBOX_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/arcan-sandboxes"));
        Self::detect(workspace_root, find_docker_socket(), which_binary("nsjail"))
    }

    /// Internal constructor used by [`from_env`](Self::from_env) and tests.
    ///
    /// Accepts explicit optional values so tests avoid unsafe env mutation.
    fn detect(
        workspace_root: PathBuf,
        docker_socket: Option<PathBuf>,
        nsjail_bin: Option<PathBuf>,
    ) -> Result<Self, anyhow::Error> {
        if let Some(socket) = docker_socket {
            info!(socket = %socket.display(), "Docker socket found — using Docker backend");
            return Ok(Self {
                backend: LocalBackend::Docker { socket },
                workspace_root,
            });
        }

        if let Some(bin) = nsjail_bin {
            info!(nsjail_bin = %bin.display(), "nsjail found — using nsjail backend");
            return Ok(Self {
                backend: LocalBackend::Nsjail { nsjail_bin: bin },
                workspace_root,
            });
        }

        anyhow::bail!(
            "no local sandbox backend available: Docker socket not found and nsjail not in PATH"
        )
    }

    /// Construct with a specific Docker socket path.
    pub fn with_docker(socket: PathBuf, workspace_root: PathBuf) -> Self {
        Self {
            backend: LocalBackend::Docker { socket },
            workspace_root,
        }
    }

    /// Construct with a specific nsjail binary path.
    pub fn with_nsjail(nsjail_bin: PathBuf, workspace_root: PathBuf) -> Self {
        Self {
            backend: LocalBackend::Nsjail { nsjail_bin },
            workspace_root,
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Workspace directory for `id`.
    fn workspace_dir(&self, id: &SandboxId) -> PathBuf {
        self.workspace_root.join(&id.0)
    }

    /// Snapshot tarball path for `id`.
    fn snapshot_path(&self, id: &SandboxId) -> PathBuf {
        self.workspace_root.join(format!("{}.tar.gz", id.0))
    }

    /// Docker container name for `id`.
    fn container_name(id: &SandboxId) -> String {
        format!("arcan-{}", id.0)
    }

    /// Map a general error message to [`SandboxError::ProviderError`].
    fn err(msg: impl Into<String>) -> SandboxError {
        SandboxError::ProviderError {
            provider: "local",
            message: msg.into(),
        }
    }

    // ── Docker impl ──────────────────────────────────────────────────────────

    async fn docker_create(&self, spec: &SandboxSpec, id: &SandboxId) -> Result<(), SandboxError> {
        let container = Self::container_name(id);
        let workspace = self.workspace_dir(id);

        fs::create_dir_all(&workspace)
            .await
            .map_err(|e| Self::err(format!("create workspace dir: {e}")))?;

        let image = spec.image.as_deref().unwrap_or("ubuntu:22.04");
        let mem_limit = format!("{}m", spec.resources.memory_mb);
        let cpus = spec.resources.vcpus.to_string();
        let volume = format!("{}:/workspace", workspace.display());
        let session_label = format!("arcan.session={}", id.0);

        // Build argv explicitly — no shell interpolation.
        let args: &[&str] = &[
            "run",
            "-d",
            "--name",
            &container,
            "--label",
            &session_label,
            "--memory",
            &mem_limit,
            "--cpus",
            &cpus,
            "-v",
            &volume,
            image,
            "sleep",
            "infinity",
        ];

        let out = Command::new("docker")
            .args(args)
            .output()
            .await
            .map_err(|e| Self::err(format!("docker run: {e}")))?;

        if !out.status.success() {
            return Err(Self::err(format!(
                "docker run failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }

        debug!(container, "docker container started");
        Ok(())
    }

    async fn docker_exec(
        &self,
        id: &SandboxId,
        req: &ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        let container = Self::container_name(id);
        let timeout_secs = req.timeout_secs.unwrap_or(60);
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let start = Instant::now();

        // Compose argv: docker exec [-e K=V ...] [-w dir] <container> <command...>
        let mut argv: Vec<String> = vec!["exec".into()];
        for (k, v) in &req.env {
            argv.push("-e".into());
            argv.push(format!("{k}={v}"));
        }
        if let Some(dir) = &req.working_dir {
            argv.push("-w".into());
            argv.push(dir.clone());
        }
        argv.push(container.clone());
        argv.extend(req.command.iter().cloned());

        let result =
            tokio::time::timeout(timeout, Command::new("docker").args(&argv).output()).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(out)) => Ok(ExecResult {
                stdout: out.stdout,
                stderr: out.stderr,
                exit_code: out.status.code().unwrap_or(-1),
                duration_ms: elapsed_ms,
            }),
            Ok(Err(e)) => Err(Self::err(format!("docker exec spawn: {e}"))),
            Err(_) => Err(SandboxError::ExecTimeout {
                sandbox_id: id.clone(),
                timeout_secs,
            }),
        }
    }

    async fn docker_snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let container = Self::container_name(id);
        let image_name = format!("arcan-snap-{}", id.0);

        let out = Command::new("docker")
            .args(["commit", &container, &image_name])
            .output()
            .await
            .map_err(|e| Self::err(format!("docker commit: {e}")))?;

        if !out.status.success() {
            return Err(Self::err(format!(
                "docker commit failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }

        Ok(SnapshotId(image_name))
    }

    async fn docker_destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let container = Self::container_name(id);
        let workspace = self.workspace_dir(id);

        // Ignore errors — container may already be gone.
        let _ = Command::new("docker")
            .args(["rm", "-f", &container])
            .output()
            .await;

        if workspace.exists() {
            fs::remove_dir_all(&workspace)
                .await
                .map_err(|e| Self::err(format!("remove workspace: {e}")))?;
        }

        Ok(())
    }

    async fn docker_list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        let out = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                "label=arcan.session",
                "--format",
                "{{.ID}}\t{{.Names}}\t{{.Status}}\t{{.CreatedAt}}",
            ])
            .output()
            .await
            .map_err(|e| Self::err(format!("docker ps: {e}")))?;

        if !out.status.success() {
            return Err(Self::err(format!(
                "docker ps failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }

        let text = String::from_utf8_lossy(&out.stdout);
        let mut infos = Vec::new();

        for line in text.lines() {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[1].trim_start_matches("arcan-").to_owned();
            let status_str = parts.get(2).copied().unwrap_or("").to_lowercase();
            let status = if status_str.contains("up") {
                SandboxStatus::Running
            } else {
                SandboxStatus::Stopped
            };

            infos.push(SandboxInfo {
                id: SandboxId(name.clone()),
                name,
                status,
                created_at: Utc::now(),
            });
        }

        Ok(infos)
    }

    // ── nsjail impl ──────────────────────────────────────────────────────────

    async fn nsjail_create(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let workspace = self.workspace_dir(id);
        fs::create_dir_all(&workspace)
            .await
            .map_err(|e| Self::err(format!("create workspace: {e}")))?;
        Ok(())
    }

    async fn nsjail_exec(
        &self,
        id: &SandboxId,
        req: &ExecRequest,
        bin: &Path,
    ) -> Result<ExecResult, SandboxError> {
        let workspace = self.workspace_dir(id);
        if !workspace.exists() {
            return Err(SandboxError::NotFound(id.clone()));
        }

        let timeout_secs = req.timeout_secs.unwrap_or(60);
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let start = Instant::now();

        let bindmount = format!("{workdir}:/workspace", workdir = workspace.display());
        let mut argv: Vec<String> = vec![
            "--mode".into(),
            "once".into(),
            "--chroot".into(),
            "/".into(),
            "--bindmount".into(),
            bindmount,
            "--".into(),
        ];
        argv.extend(req.command.iter().cloned());

        let mut cmd = Command::new(bin);
        cmd.args(&argv);
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let result = tokio::time::timeout(timeout, cmd.output()).await;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(out)) => Ok(ExecResult {
                stdout: out.stdout,
                stderr: out.stderr,
                exit_code: out.status.code().unwrap_or(-1),
                duration_ms: elapsed_ms,
            }),
            Ok(Err(e)) => Err(Self::err(format!("nsjail spawn: {e}"))),
            Err(_) => Err(SandboxError::ExecTimeout {
                sandbox_id: id.clone(),
                timeout_secs,
            }),
        }
    }

    async fn nsjail_snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let workspace = self.workspace_dir(id);
        if !workspace.exists() {
            return Err(SandboxError::NotFound(id.clone()));
        }

        let tarball = self.snapshot_path(id);
        let tarball_name = tarball
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();

        let status = Command::new("tar")
            .args([
                "-czf",
                tarball.to_str().unwrap_or_default(),
                "-C",
                self.workspace_root.to_str().unwrap_or_default(),
                &id.0,
            ])
            .status()
            .await
            .map_err(|e| Self::err(format!("tar create: {e}")))?;

        if !status.success() {
            return Err(Self::err(format!("tar exited with {status}")));
        }

        Ok(SnapshotId(tarball_name))
    }

    async fn nsjail_destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let workspace = self.workspace_dir(id);
        let tarball = self.snapshot_path(id);

        if workspace.exists() {
            fs::remove_dir_all(&workspace)
                .await
                .map_err(|e| Self::err(format!("remove workspace: {e}")))?;
        }
        if tarball.exists() {
            fs::remove_file(&tarball)
                .await
                .map_err(|e| Self::err(format!("remove tarball: {e}")))?;
        }

        Ok(())
    }

    async fn nsjail_list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        if !self.workspace_root.exists() {
            return Ok(vec![]);
        }

        let mut entries = fs::read_dir(&self.workspace_root)
            .await
            .map_err(|e| Self::err(format!("read workspace_root: {e}")))?;

        let mut infos = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| Self::err(format!("dir entry: {e}")))?
        {
            let ft = entry
                .file_type()
                .await
                .map_err(|e| Self::err(format!("file type: {e}")))?;
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

// ── HypervisorBackend impl (BRO-853) ──────────────────────────────────────────
//
// Explicit first-class impl of the canonical `aios_protocol::HypervisorBackend`
// contract. Delegates to the private `docker_*` / `nsjail_*` helpers directly
// so there is no round-trip through the deprecated `SandboxProvider` shim.
//
// The deprecated `SandboxProvider` trait is still available to legacy callers
// via the blanket `impl<T: HypervisorBackend> SandboxProvider for T` exposed by
// `arcan-sandbox`; we no longer maintain an explicit `impl SandboxProvider`
// because (a) it would conflict with the blanket impl and (b) all of its
// behaviour is preserved by delegating to the helpers below.

#[async_trait]
impl aios_protocol::hypervisor::HypervisorBackend for LocalSandboxProvider {
    fn name(&self) -> &'static str {
        "local"
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
        use aios_protocol::hypervisor::{BackendId, VmHandle, VmId, VmStatus};

        let id = SandboxId(Uuid::new_v4().to_string());
        // Translate VmSpec → legacy SandboxSpec for reuse of docker/nsjail helpers.
        let sandbox_spec = vm_spec_to_sandbox_spec(&spec, &id);
        let sandbox_name = sandbox_spec.name.clone();

        // Delegate to backend-specific helpers.
        match &self.backend {
            LocalBackend::Docker { .. } => {
                self.docker_create(&sandbox_spec, &id)
                    .await
                    .map_err(backend_error_from_sandbox)?;
                let container = Self::container_name(&id);
                Ok(VmHandle {
                    vm_id: VmId(id.0.clone()),
                    backend: BackendId::from("local"),
                    session_id: session_id_from_spec(&spec),
                    agent_id: agent_id_from_spec(&spec),
                    status: VmStatus::Running,
                    created_at: Utc::now(),
                    metadata: serde_json::json!({
                        "container": container,
                        "sandbox.name": sandbox_name,
                    }),
                })
            }
            LocalBackend::Nsjail { .. } => {
                self.nsjail_create(&id)
                    .await
                    .map_err(backend_error_from_sandbox)?;
                let workspace = self.workspace_dir(&id);
                Ok(VmHandle {
                    vm_id: VmId(id.0.clone()),
                    backend: BackendId::from("local"),
                    session_id: session_id_from_spec(&spec),
                    agent_id: agent_id_from_spec(&spec),
                    status: VmStatus::Running,
                    created_at: Utc::now(),
                    metadata: serde_json::json!({
                        "workspace": workspace.display().to_string(),
                        "sandbox.name": sandbox_name,
                    }),
                })
            }
        }
    }

    async fn exec(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
        req: aios_protocol::hypervisor::ExecRequest,
    ) -> Result<aios_protocol::hypervisor::ExecResult, aios_protocol::hypervisor::BackendError>
    {
        let id = SandboxId(vm.vm_id.0.clone());
        let legacy_req = vm_exec_request_to_sandbox(req);
        let legacy_result = match &self.backend {
            LocalBackend::Docker { .. } => self.docker_exec(&id, &legacy_req).await,
            LocalBackend::Nsjail { nsjail_bin } => {
                self.nsjail_exec(&id, &legacy_req, nsjail_bin).await
            }
        }
        .map_err(backend_error_from_sandbox)?;
        Ok(aios_protocol::hypervisor::ExecResult {
            stdout: legacy_result.stdout,
            stderr: legacy_result.stderr,
            exit_code: legacy_result.exit_code,
            duration_ms: legacy_result.duration_ms,
        })
    }

    async fn snapshot(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<aios_protocol::hypervisor::VmSnapshotId, aios_protocol::hypervisor::BackendError>
    {
        let id = SandboxId(vm.vm_id.0.clone());
        let snap = match &self.backend {
            LocalBackend::Docker { .. } => self.docker_snapshot(&id).await,
            LocalBackend::Nsjail { .. } => self.nsjail_snapshot(&id).await,
        }
        .map_err(backend_error_from_sandbox)?;
        Ok(aios_protocol::hypervisor::VmSnapshotId(snap.0))
    }

    async fn restore(
        &self,
        _snapshot: &aios_protocol::hypervisor::VmSnapshotId,
    ) -> Result<aios_protocol::hypervisor::VmHandle, aios_protocol::hypervisor::BackendError> {
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "local",
            reason: "restore",
        })
    }

    async fn destroy(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        let id = SandboxId(vm.vm_id.0.clone());
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_destroy(&id).await,
            LocalBackend::Nsjail { .. } => self.nsjail_destroy(&id).await,
        }
        .map_err(backend_error_from_sandbox)
    }

    // `hibernate` / `resume` inherit `BackendError::NotSupported` defaults.
}

#[async_trait]
impl aios_protocol::hypervisor::HypervisorFilesystemExt for LocalSandboxProvider {
    async fn write_files(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _files: Vec<aios_protocol::hypervisor::FileWrite>,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        // The underlying docker/nsjail helpers do not yet expose direct file
        // writes; the legacy `SandboxProvider::write_files` default returned
        // `NotSupported` and we preserve that semantic here.
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "local",
            reason: "write_files",
        })
    }

    async fn read_file(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _path: &str,
    ) -> Result<Vec<u8>, aios_protocol::hypervisor::BackendError> {
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "local",
            reason: "read_file",
        })
    }

    async fn list(
        &self,
    ) -> Result<Vec<aios_protocol::hypervisor::VmInfo>, aios_protocol::hypervisor::BackendError>
    {
        let legacy = match &self.backend {
            LocalBackend::Docker { .. } => self.docker_list().await,
            LocalBackend::Nsjail { .. } => self.nsjail_list().await,
        }
        .map_err(backend_error_from_sandbox)?;

        Ok(legacy
            .into_iter()
            .map(|info| aios_protocol::hypervisor::VmInfo {
                vm_id: aios_protocol::hypervisor::VmId(info.id.0),
                backend: aios_protocol::hypervisor::BackendId::from("local"),
                status: vm_status_from_sandbox(info.status),
                created_at: info.created_at,
            })
            .collect())
    }
}

// ── Conversion helpers (VmSpec ↔ SandboxSpec, SandboxError → BackendError) ────

/// Translate a kernel-level [`aios_protocol::hypervisor::VmSpec`] into the
/// legacy [`SandboxSpec`] consumed by the private `docker_*` / `nsjail_*`
/// helpers. The conversion is lossy for labels/mounts (which the helpers do
/// not yet honour) but preserves the fields that actually drive provisioning.
fn vm_spec_to_sandbox_spec(
    spec: &aios_protocol::hypervisor::VmSpec,
    id: &SandboxId,
) -> arcan_sandbox::types::SandboxSpec {
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
        .unwrap_or_else(|| id.0.clone());

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
/// The reverse direction (`From<BackendError> for SandboxError`) landed in
/// BRO-852; this local mapping handles the forward direction specifically for
/// the `arcan-provider-local` cut-over and avoids a blanket `From` impl in
/// `arcan-sandbox` that would create a circular dependency.
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
        .unwrap_or_else(|| aios_protocol::ids::SessionId::from_string("arcan-provider-local"))
}

/// Extract an [`AgentId`] from optional `agent.id` labels on the spec.
fn agent_id_from_spec(spec: &aios_protocol::hypervisor::VmSpec) -> aios_protocol::ids::AgentId {
    spec.labels
        .get("agent.id")
        .map(|s| aios_protocol::ids::AgentId::from_string(s.as_str()))
        .unwrap_or_else(|| aios_protocol::ids::AgentId::from_string("arcan-provider-local"))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Find the Docker socket path.
///
/// Checks `DOCKER_HOST` env var first, then the default socket location.
fn find_docker_socket() -> Option<PathBuf> {
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        // DOCKER_HOST may be `unix:///path/to/sock`
        let path = host.trim_start_matches("unix://");
        let p = PathBuf::from(path);
        if p.exists() { Some(p) } else { None }
    } else {
        let default = PathBuf::from("/var/run/docker.sock");
        if default.exists() {
            Some(default)
        } else {
            None
        }
    }
}

/// Search `PATH` for `binary_name` and return its path if found.
fn which_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join(name);
            if candidate.is_file() {
                Some(candidate)
            } else {
                None
            }
        })
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::hypervisor::{BackendCapabilitySet, HypervisorBackend};

    /// Build a nsjail-backed provider at a temp directory.
    fn nsjail_provider(root: &Path) -> LocalSandboxProvider {
        LocalSandboxProvider::with_nsjail(PathBuf::from("/usr/sbin/nsjail"), root.to_path_buf())
    }

    /// Build a Docker-backed provider at a temp directory.
    fn docker_provider(root: &Path) -> LocalSandboxProvider {
        LocalSandboxProvider::with_docker(PathBuf::from("/var/run/docker.sock"), root.to_path_buf())
    }

    #[test]
    fn from_env_returns_err_when_no_backend_available() {
        // Pass None for both socket and nsjail to simulate no backend available.
        // Uses detect() directly so we avoid unsafe env mutation.
        let result = LocalSandboxProvider::detect(
            PathBuf::from("/tmp/arcan-sandboxes"),
            None, // no docker socket
            None, // no nsjail binary
        );
        assert!(
            result.is_err(),
            "should fail when neither Docker nor nsjail is available"
        );
    }

    #[test]
    fn with_nsjail_name_is_local() {
        let tmp = tempfile::tempdir().unwrap();
        let p = nsjail_provider(tmp.path());
        assert_eq!(HypervisorBackend::name(&p), "local");
    }

    #[test]
    fn with_docker_name_is_local() {
        let tmp = tempfile::tempdir().unwrap();
        let p = docker_provider(tmp.path());
        assert_eq!(HypervisorBackend::name(&p), "local");
    }

    #[test]
    fn capabilities_include_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let nsjail = nsjail_provider(tmp.path());
        let docker = docker_provider(tmp.path());

        // Both backends advertise the same kernel-level capability set now —
        // the docker-vs-nsjail distinction moved into runtime-hint handling.
        for caps in [nsjail.capabilities(), docker.capabilities()] {
            assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_READ));
            assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_WRITE));
            assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_EXT));
            assert!(caps.contains(BackendCapabilitySet::NETWORK_EGRESS));
        }
    }
}

// ── HypervisorBackend trait tests (BRO-853) ──────────────────────────────────

#[cfg(test)]
mod kernel_tests {
    use super::*;
    use aios_protocol::hypervisor::{BackendCapabilitySet, HypervisorBackend};

    /// Build a nsjail-backed provider at a temp directory — mirrors the helper
    /// in the legacy tests module so this file-scoped module stays self-contained.
    fn nsjail_provider(root: &Path) -> LocalSandboxProvider {
        LocalSandboxProvider::with_nsjail(PathBuf::from("/usr/sbin/nsjail"), root.to_path_buf())
    }

    #[test]
    fn local_provider_impls_hypervisor_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = nsjail_provider(tmp.path());
        assert_eq!(HypervisorBackend::name(&provider), "local");
        assert!(
            HypervisorBackend::capabilities(&provider)
                .contains(BackendCapabilitySet::FILESYSTEM_EXT),
            "local provider must advertise FILESYSTEM_EXT now that \
             HypervisorFilesystemExt is implemented"
        );
    }

    #[test]
    fn local_provider_advertises_full_capability_set() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = nsjail_provider(tmp.path());
        let caps = HypervisorBackend::capabilities(&provider);
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_READ));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_WRITE));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_EXT));
        assert!(caps.contains(BackendCapabilitySet::NETWORK_EGRESS));
    }

    #[tokio::test]
    async fn local_provider_restore_returns_not_supported() {
        use aios_protocol::hypervisor::{BackendError, VmSnapshotId};

        let tmp = tempfile::tempdir().unwrap();
        let provider = nsjail_provider(tmp.path());
        let err = HypervisorBackend::restore(&provider, &VmSnapshotId::from("snap-1"))
            .await
            .expect_err("restore should be unsupported for LocalSandboxProvider");
        match err {
            BackendError::NotSupported { backend, reason } => {
                assert_eq!(backend, "local");
                assert_eq!(reason, "restore");
            }
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_provider_hibernate_returns_not_supported_default() {
        use aios_protocol::hypervisor::{BackendError, BackendId, VmHandle, VmId, VmStatus};
        use aios_protocol::ids::{AgentId, SessionId};

        let tmp = tempfile::tempdir().unwrap();
        let provider = nsjail_provider(tmp.path());
        let handle = VmHandle {
            vm_id: VmId::from("vm-1"),
            backend: BackendId::from("local"),
            session_id: SessionId::from_string("sess-1"),
            agent_id: AgentId::from_string("agent-1"),
            status: VmStatus::Running,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        };
        let err = HypervisorBackend::hibernate(&provider, &handle)
            .await
            .expect_err("default hibernate impl should return NotSupported");
        assert!(matches!(err, BackendError::NotSupported { .. }));
    }
}
