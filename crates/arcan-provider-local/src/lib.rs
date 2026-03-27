//! `arcan-provider-local` — [`SandboxProvider`] with Docker primary backend
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

use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, info};
use uuid::Uuid;

use arcan_sandbox::capability::SandboxCapabilitySet;
use arcan_sandbox::error::SandboxError;
use arcan_sandbox::provider::SandboxProvider;
use arcan_sandbox::types::{
    ExecRequest, ExecResult, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus,
    SnapshotId,
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
            return Ok(Self { backend: LocalBackend::Docker { socket }, workspace_root });
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
        Self { backend: LocalBackend::Docker { socket }, workspace_root }
    }

    /// Construct with a specific nsjail binary path.
    pub fn with_nsjail(nsjail_bin: PathBuf, workspace_root: PathBuf) -> Self {
        Self { backend: LocalBackend::Nsjail { nsjail_bin }, workspace_root }
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
        SandboxError::ProviderError { provider: "local", message: msg.into() }
    }

    // ── Docker impl ──────────────────────────────────────────────────────────

    async fn docker_create(&self, spec: &SandboxSpec, id: &SandboxId) -> Result<(), SandboxError> {
        let container = Self::container_name(id);
        let workspace = self.workspace_dir(id);

        fs::create_dir_all(&workspace).await.map_err(|e| {
            Self::err(format!("create workspace dir: {e}"))
        })?;

        let image = spec.image.as_deref().unwrap_or("ubuntu:22.04");
        let mem_limit = format!("{}m", spec.resources.memory_mb);
        let cpus = spec.resources.vcpus.to_string();
        let volume = format!("{}:/workspace", workspace.display());
        let session_label = format!("arcan.session={}", id.0);

        // Build argv explicitly — no shell interpolation.
        let args: &[&str] = &[
            "run", "-d",
            "--name", &container,
            "--label", &session_label,
            "--memory", &mem_limit,
            "--cpus", &cpus,
            "-v", &volume,
            image,
            "sleep", "infinity",
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
            Err(_) => Err(SandboxError::ExecTimeout { sandbox_id: id.clone(), timeout_secs }),
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

    async fn docker_resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        let container = Self::container_name(id);
        let snapshot_image = format!("arcan-snap-{}", id.0);

        // Try `docker start` first (container already exists).
        let out = Command::new("docker")
            .args(["start", &container])
            .output()
            .await
            .map_err(|e| Self::err(format!("docker start: {e}")))?;

        if !out.status.success() {
            // Container might not exist — try running from snapshot image.
            let workspace = self.workspace_dir(id);
            let volume = format!("{}:/workspace", workspace.display());
            let session_label = format!("arcan.session={}", id.0);
            let run_args: &[&str] = &[
                "run", "-d",
                "--name", &container,
                "--label", &session_label,
                "-v", &volume,
                &snapshot_image,
                "sleep", "infinity",
            ];
            let run_out = Command::new("docker")
                .args(run_args)
                .output()
                .await
                .map_err(|e| Self::err(format!("docker run from snapshot: {e}")))?;

            if !run_out.status.success() {
                return Err(SandboxError::NotFound(id.clone()));
            }
        }

        Ok(SandboxHandle {
            id: id.clone(),
            name: id.0.clone(),
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: self.name().to_owned(),
            metadata: serde_json::json!({ "container": container }),
        })
    }

    async fn docker_destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let container = Self::container_name(id);
        let workspace = self.workspace_dir(id);

        // Ignore errors — container may already be gone.
        let _ = Command::new("docker").args(["rm", "-f", &container]).output().await;

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
                "ps", "-a",
                "--filter", "label=arcan.session",
                "--format", "{{.ID}}\t{{.Names}}\t{{.Status}}\t{{.CreatedAt}}",
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
            Err(_) => Err(SandboxError::ExecTimeout { sandbox_id: id.clone(), timeout_secs }),
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

    async fn nsjail_resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        let workspace = self.workspace_dir(id);
        let tarball = self.snapshot_path(id);

        if !workspace.exists() {
            if tarball.exists() {
                let status = Command::new("tar")
                    .args([
                        "-xzf",
                        tarball.to_str().unwrap_or_default(),
                        "-C",
                        self.workspace_root.to_str().unwrap_or_default(),
                    ])
                    .status()
                    .await
                    .map_err(|e| Self::err(format!("tar extract: {e}")))?;

                if !status.success() {
                    return Err(Self::err(format!("tar extract exited with {status}")));
                }
            } else {
                return Err(SandboxError::NotFound(id.clone()));
            }
        }

        Ok(SandboxHandle {
            id: id.clone(),
            name: id.0.clone(),
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: self.name().to_owned(),
            metadata: serde_json::json!({ "workspace": workspace.display().to_string() }),
        })
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
        while let Some(entry) =
            entries.next_entry().await.map_err(|e| Self::err(format!("dir entry: {e}")))?
        {
            let ft =
                entry.file_type().await.map_err(|e| Self::err(format!("file type: {e}")))?;
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
                    chrono::DateTime::from_timestamp(secs, 0).unwrap_or_else(|| Utc::now())
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

// ── SandboxProvider impl ──────────────────────────────────────────────────────

#[async_trait]
impl SandboxProvider for LocalSandboxProvider {
    fn name(&self) -> &'static str {
        "local"
    }

    /// Advertised capabilities.
    ///
    /// - [`FILESYSTEM_READ`](SandboxCapabilitySet::FILESYSTEM_READ)
    /// - [`FILESYSTEM_WRITE`](SandboxCapabilitySet::FILESYSTEM_WRITE)
    /// - [`CUSTOM_IMAGE`](SandboxCapabilitySet::CUSTOM_IMAGE) (Docker only)
    /// - [`PERSISTENCE`](SandboxCapabilitySet::PERSISTENCE)
    fn capabilities(&self) -> SandboxCapabilitySet {
        let base = SandboxCapabilitySet::FILESYSTEM_READ
            | SandboxCapabilitySet::FILESYSTEM_WRITE
            | SandboxCapabilitySet::PERSISTENCE;

        match &self.backend {
            LocalBackend::Docker { .. } => base | SandboxCapabilitySet::CUSTOM_IMAGE,
            LocalBackend::Nsjail { .. } => base,
        }
    }

    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let id = SandboxId(Uuid::new_v4().to_string());

        match &self.backend {
            LocalBackend::Docker { .. } => {
                self.docker_create(&spec, &id).await?;
                let container = Self::container_name(&id);
                Ok(SandboxHandle {
                    id,
                    name: spec.name,
                    status: SandboxStatus::Running,
                    created_at: Utc::now(),
                    provider: self.name().to_owned(),
                    metadata: serde_json::json!({ "container": container }),
                })
            }
            LocalBackend::Nsjail { .. } => {
                self.nsjail_create(&id).await?;
                let workspace = self.workspace_dir(&id);
                Ok(SandboxHandle {
                    id,
                    name: spec.name,
                    status: SandboxStatus::Running,
                    created_at: Utc::now(),
                    provider: self.name().to_owned(),
                    metadata: serde_json::json!({
                        "workspace": workspace.display().to_string()
                    }),
                })
            }
        }
    }

    async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_resume(id).await,
            LocalBackend::Nsjail { .. } => self.nsjail_resume(id).await,
        }
    }

    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_exec(id, &req).await,
            LocalBackend::Nsjail { nsjail_bin } => {
                self.nsjail_exec(id, &req, nsjail_bin).await
            }
        }
    }

    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_snapshot(id).await,
            LocalBackend::Nsjail { .. } => self.nsjail_snapshot(id).await,
        }
    }

    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_destroy(id).await,
            LocalBackend::Nsjail { .. } => self.nsjail_destroy(id).await,
        }
    }

    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        match &self.backend {
            LocalBackend::Docker { .. } => self.docker_list().await,
            LocalBackend::Nsjail { .. } => self.nsjail_list().await,
        }
    }
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
        if default.exists() { Some(default) } else { None }
    }
}

/// Search `PATH` for `binary_name` and return its path if found.
fn which_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path_var| {
        std::env::split_paths(&path_var).find_map(|dir| {
            let candidate = dir.join(name);
            if candidate.is_file() { Some(candidate) } else { None }
        })
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a nsjail-backed provider at a temp directory.
    fn nsjail_provider(root: &Path) -> LocalSandboxProvider {
        LocalSandboxProvider::with_nsjail(
            PathBuf::from("/usr/sbin/nsjail"),
            root.to_path_buf(),
        )
    }

    /// Build a Docker-backed provider at a temp directory.
    fn docker_provider(root: &Path) -> LocalSandboxProvider {
        LocalSandboxProvider::with_docker(
            PathBuf::from("/var/run/docker.sock"),
            root.to_path_buf(),
        )
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
        assert!(result.is_err(), "should fail when neither Docker nor nsjail is available");
    }

    #[test]
    fn with_nsjail_name_is_local() {
        let tmp = tempfile::tempdir().unwrap();
        let p = nsjail_provider(tmp.path());
        assert_eq!(p.name(), "local");
    }

    #[test]
    fn with_docker_name_is_local() {
        let tmp = tempfile::tempdir().unwrap();
        let p = docker_provider(tmp.path());
        assert_eq!(p.name(), "local");
    }

    #[test]
    fn capabilities_include_filesystem() {
        let tmp = tempfile::tempdir().unwrap();
        let nsjail = nsjail_provider(tmp.path());
        let docker = docker_provider(tmp.path());

        let nsjail_caps = nsjail.capabilities();
        assert!(nsjail_caps.contains(SandboxCapabilitySet::FILESYSTEM_READ));
        assert!(nsjail_caps.contains(SandboxCapabilitySet::FILESYSTEM_WRITE));
        assert!(nsjail_caps.contains(SandboxCapabilitySet::PERSISTENCE));

        let docker_caps = docker.capabilities();
        assert!(docker_caps.contains(SandboxCapabilitySet::FILESYSTEM_READ));
        assert!(docker_caps.contains(SandboxCapabilitySet::FILESYSTEM_WRITE));
        assert!(docker_caps.contains(SandboxCapabilitySet::PERSISTENCE));
        assert!(
            docker_caps.contains(SandboxCapabilitySet::CUSTOM_IMAGE),
            "Docker backend should advertise CUSTOM_IMAGE"
        );
    }
}
