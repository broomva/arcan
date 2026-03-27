//! `arcan-provider-bubblewrap` — [`SandboxProvider`] implementation wrapping
//! the [`bwrap`](https://github.com/containers/bubblewrap) binary with a plain
//! subprocess fallback.
//!
//! # Backend selection
//!
//! At construction time the provider probes `PATH` for the `bwrap` binary.
//! If found, every [`run`](BubblewrapProvider::run) call isolates the command
//! inside a minimal bubblewrap namespace. If `bwrap` is absent the provider
//! falls back to a plain Tokio [`Command`](tokio::process::Command) confined
//! to the sandbox workspace directory — identical semantics to
//! [`LocalCommandRunner`](praxis_core::sandbox::LocalCommandRunner) but async.
//!
//! # Persistence
//!
//! Sandbox state lives in a per-sandbox subdirectory of `workspace_root`.
//! [`snapshot`](BubblewrapProvider::snapshot) tars that directory;
//! [`resume`](BubblewrapProvider::resume) untars it back.

use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, info, warn};
use uuid::Uuid;

use arcan_sandbox::capability::SandboxCapabilitySet;
use arcan_sandbox::error::SandboxError;
use arcan_sandbox::provider::SandboxProvider;
use arcan_sandbox::types::{
    ExecRequest, ExecResult, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus,
    SnapshotId,
};

// ── Provider ────────────────────────────────────────────────────────────────

/// Sandbox provider backed by bubblewrap (`bwrap`) or a plain subprocess.
///
/// Construct with [`BubblewrapProvider::new`] or
/// [`BubblewrapProvider::from_env`].
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

    /// Absolute path of the snapshot tarball for `id`.
    fn snapshot_path(&self, id: &SandboxId) -> PathBuf {
        self.workspace_root.join(format!("{}.tar.gz", id.0))
    }

    /// Derive the effective timeout for a request, falling back to the spec
    /// default (60 s).
    fn timeout_secs(req: &ExecRequest) -> u64 {
        req.timeout_secs.unwrap_or(60)
    }
}

#[async_trait]
impl SandboxProvider for BubblewrapProvider {
    fn name(&self) -> &'static str {
        "bubblewrap"
    }

    /// Advertised capabilities.
    ///
    /// - [`FILESYSTEM_READ`](SandboxCapabilitySet::FILESYSTEM_READ)
    /// - [`FILESYSTEM_WRITE`](SandboxCapabilitySet::FILESYSTEM_WRITE)
    /// - [`PERSISTENCE`](SandboxCapabilitySet::PERSISTENCE)
    ///
    /// `NETWORK_OUTBOUND` is deliberately omitted — bubblewrap uses
    /// `--unshare-net` by default.
    fn capabilities(&self) -> SandboxCapabilitySet {
        SandboxCapabilitySet::FILESYSTEM_READ
            | SandboxCapabilitySet::FILESYSTEM_WRITE
            | SandboxCapabilitySet::PERSISTENCE
    }

    /// Create a new sandbox workspace directory.
    ///
    /// Returns immediately with `status: Running`. The workspace directory is
    /// created at `{workspace_root}/{id}/`.
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let id = SandboxId(Uuid::new_v4().to_string());
        let workspace = self.workspace_dir(&id);

        fs::create_dir_all(&workspace)
            .await
            .map_err(|e| SandboxError::ProviderError {
                provider: "bubblewrap",
                message: format!("create workspace dir: {e}"),
            })?;

        debug!(sandbox_id = %id, workspace = %workspace.display(), "sandbox workspace created");

        Ok(SandboxHandle {
            id,
            name: spec.name,
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: self.name().to_owned(),
            metadata: serde_json::json!({
                "workspace": workspace.display().to_string(),
                "use_bwrap": self.use_bwrap,
            }),
        })
    }

    /// Restore a sandbox from its tarball snapshot.
    ///
    /// If the workspace directory already exists this is a no-op — the handle
    /// is returned with `status: Running`. If the workspace is missing but a
    /// tarball exists it is extracted.
    async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        let workspace = self.workspace_dir(id);
        let tarball = self.snapshot_path(id);

        if !workspace.exists() {
            if tarball.exists() {
                // Untar the snapshot into workspace_root.
                let status = Command::new("tar")
                    .args(["-xzf", tarball.to_str().unwrap_or_default()])
                    .args(["-C", self.workspace_root.to_str().unwrap_or_default()])
                    .status()
                    .await
                    .map_err(|e| SandboxError::ProviderError {
                        provider: "bubblewrap",
                        message: format!("tar extract failed: {e}"),
                    })?;

                if !status.success() {
                    return Err(SandboxError::ProviderError {
                        provider: "bubblewrap",
                        message: format!("tar exited with {status} while extracting {tarball:?}"),
                    });
                }

                info!(sandbox_id = %id, "sandbox restored from snapshot");
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
            metadata: serde_json::json!({
                "workspace": workspace.display().to_string(),
                "use_bwrap": self.use_bwrap,
            }),
        })
    }

    /// Execute a command inside the sandbox workspace.
    ///
    /// Uses `bwrap` when available, otherwise falls back to a plain Tokio
    /// subprocess with the current directory set to the workspace.
    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
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

    /// Tar the workspace directory to `{workspace_root}/{id}.tar.gz`.
    ///
    /// Returns the filename as the [`SnapshotId`].
    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
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

        // tar -czf {tarball} -C {workspace_root} {id}
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
            .map_err(|e| SandboxError::ProviderError {
                provider: "bubblewrap",
                message: format!("tar create failed: {e}"),
            })?;

        if !status.success() {
            return Err(SandboxError::ProviderError {
                provider: "bubblewrap",
                message: format!("tar exited with {status}"),
            });
        }

        info!(sandbox_id = %id, snapshot = %tarball_name, "snapshot created");
        Ok(SnapshotId(tarball_name))
    }

    /// Remove the workspace directory and any associated tarball.
    ///
    /// Succeeds even if neither exists.
    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let workspace = self.workspace_dir(id);
        let tarball = self.snapshot_path(id);

        if workspace.exists() {
            fs::remove_dir_all(&workspace)
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("remove workspace: {e}"),
                })?;
        }
        if tarball.exists() {
            fs::remove_file(&tarball)
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "bubblewrap",
                    message: format!("remove tarball: {e}"),
                })?;
        }

        info!(sandbox_id = %id, "sandbox destroyed");
        Ok(())
    }

    /// Scan `workspace_root` for directories and return a [`SandboxInfo`] for
    /// each one found.
    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

        let handle = provider
            .create(SandboxSpec::ephemeral("test"))
            .await
            .unwrap();
        let workspace = provider.workspace_dir(&handle.id);
        assert!(
            workspace.exists(),
            "workspace dir should exist after create"
        );

        provider.destroy(&handle.id).await.unwrap();
        assert!(
            !workspace.exists(),
            "workspace dir should be removed after destroy"
        );
    }

    #[tokio::test]
    async fn run_echo_command_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = provider
            .create(SandboxSpec::ephemeral("echo-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["echo".into(), "hello".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(10),
            stdin: None,
        };

        let result = provider.run(&handle.id, req).await.unwrap();
        assert_eq!(result.exit_code, 0, "echo should exit 0");
        assert!(
            result.stdout_str().contains("hello"),
            "stdout should contain 'hello'"
        );

        provider.destroy(&handle.id).await.unwrap();
    }

    #[tokio::test]
    async fn snapshot_produces_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = provider
            .create(SandboxSpec::ephemeral("snap-test"))
            .await
            .unwrap();
        let snapshot_id = provider.snapshot(&handle.id).await.unwrap();

        let tarball = provider.snapshot_path(&handle.id);
        assert!(tarball.exists(), "tarball should exist after snapshot");
        assert!(
            snapshot_id.0.ends_with(".tar.gz"),
            "snapshot id should be a .tar.gz filename"
        );

        provider.destroy(&handle.id).await.unwrap();
    }

    #[tokio::test]
    async fn resume_restores_from_tarball() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        // Create, write a file, snapshot, destroy workspace dir, then resume.
        let handle = provider
            .create(SandboxSpec::ephemeral("resume-test"))
            .await
            .unwrap();
        let workspace = provider.workspace_dir(&handle.id);

        // Write a marker file.
        tokio::fs::write(workspace.join("marker.txt"), b"alive")
            .await
            .unwrap();

        provider.snapshot(&handle.id).await.unwrap();

        // Remove workspace dir only (keep tarball).
        tokio::fs::remove_dir_all(&workspace).await.unwrap();
        assert!(!workspace.exists());

        let resumed = provider.resume(&handle.id).await.unwrap();
        assert_eq!(resumed.id, handle.id);
        assert!(
            workspace.exists(),
            "workspace dir should exist after resume"
        );

        provider.destroy(&handle.id).await.unwrap();
    }

    #[test]
    fn name_returns_bubblewrap() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());
        assert_eq!(provider.name(), "bubblewrap");
    }

    /// A non-zero exit code must be forwarded without an error.
    #[tokio::test]
    async fn run_exit_code_one() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = provider
            .create(SandboxSpec::ephemeral("exit-code-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["/bin/sh".into(), "-c".into(), "exit 1".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(10),
            stdin: None,
        };

        let result = provider.run(&handle.id, req).await.unwrap();
        assert_eq!(result.exit_code, 1, "exit code 1 must be forwarded");

        provider.destroy(&handle.id).await.unwrap();
    }

    /// A command that exceeds its timeout must return `ExecTimeout`.
    #[tokio::test]
    async fn run_timeout_returns_exec_timeout_error() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let handle = provider
            .create(SandboxSpec::ephemeral("timeout-test"))
            .await
            .unwrap();

        let req = ExecRequest {
            command: vec!["sleep".into(), "30".into()],
            working_dir: None,
            env: Default::default(),
            timeout_secs: Some(1),
            stdin: None,
        };

        let err = provider.run(&handle.id, req).await.unwrap_err();
        assert!(
            matches!(err, SandboxError::ExecTimeout { .. }),
            "expected ExecTimeout, got: {err:?}"
        );

        provider.destroy(&handle.id).await.unwrap();
    }

    /// `resume` on an ID with no workspace and no tarball must return `NotFound`.
    #[tokio::test]
    async fn resume_unknown_id_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let unknown_id = SandboxId("does-not-exist".into());
        let err = provider.resume(&unknown_id).await.unwrap_err();
        assert!(
            matches!(err, SandboxError::NotFound(_)),
            "expected NotFound, got: {err:?}"
        );
    }

    /// `list` must return a `SandboxInfo` for each created sandbox.
    #[tokio::test]
    async fn list_returns_all_created_sandboxes() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = provider_at(tmp.path());

        let h1 = provider
            .create(SandboxSpec::ephemeral("list-test-1"))
            .await
            .unwrap();
        let h2 = provider
            .create(SandboxSpec::ephemeral("list-test-2"))
            .await
            .unwrap();

        let infos = provider.list().await.unwrap();
        let ids: Vec<_> = infos.iter().map(|i| &i.id).collect();

        assert!(ids.contains(&&h1.id), "list must include first sandbox");
        assert!(ids.contains(&&h2.id), "list must include second sandbox");

        provider.destroy(&h1.id).await.unwrap();
        provider.destroy(&h2.id).await.unwrap();
    }
}
