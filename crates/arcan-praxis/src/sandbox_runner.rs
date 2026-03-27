//! `SandboxCommandRunner` — async [`SandboxProvider`] bridge for [`CommandRunner`].
//!
//! The Praxis tool layer uses the synchronous [`CommandRunner`] trait (from
//! `praxis-core`) while the sandbox execution layer is fully async
//! ([`SandboxProvider`] from `arcan-sandbox`).  This module bridges the gap.
//!
//! # Approach
//!
//! `SandboxCommandRunner::run` is a sync function that drives an async
//! `SandboxProvider::run` call.  Depending on the calling context:
//!
//! - **Inside a tokio multi-thread runtime** (normal arcand operation):
//!   uses `tokio::task::block_in_place` + `Handle::current().block_on(…)` to
//!   park the current worker thread while the future runs.
//!
//! - **No runtime present** (unit tests, CLI contexts):
//!   spins up a temporary single-thread runtime to drive the future.
//!
//! Each `run` call creates an ephemeral sandbox, executes the command, and
//! destroys the sandbox.  Sandbox reuse across calls is a future optimisation.
//!
//! # Provider selection
//!
//! [`build_provider`] reads the `ARCAN_SANDBOX_PROVIDER` environment variable:
//!
//! | Value | Provider |
//! |-------|----------|
//! | `"local"` | [`LocalSandboxProvider`] (Docker or nsjail — falls back to bwrap on error) |
//! | `"bubblewrap"` / `"bwrap"` / *(unset)* | [`BubblewrapProvider`] (Linux namespaces, falls back to plain subprocess) |

use std::sync::Arc;

use arcan_provider_bubblewrap::BubblewrapProvider;
use arcan_provider_local::LocalSandboxProvider;
use arcan_sandbox::{ExecRequest, SandboxProvider, SandboxService, SandboxSpec};
use praxis_core::error::{PraxisError, PraxisResult};
use praxis_core::sandbox::{CommandRequest, CommandResult, CommandRunner, SandboxPolicy};
use tracing::{debug, warn};

// ── Provider factory ──────────────────────────────────────────────────────────

/// Build a [`SandboxProvider`] from the `ARCAN_SANDBOX_PROVIDER` environment
/// variable (or the compiled-in default).
///
/// | `ARCAN_SANDBOX_PROVIDER` | Provider |
/// |--------------------------|----------|
/// | `"local"` | [`LocalSandboxProvider`] (Docker/nsjail) — falls back to bwrap if unavailable |
/// | `"bubblewrap"` / `"bwrap"` / *(anything else / unset)* | [`BubblewrapProvider`] |
pub fn build_provider() -> Arc<dyn SandboxProvider> {
    let name = std::env::var("ARCAN_SANDBOX_PROVIDER").unwrap_or_default();
    match name.to_lowercase().as_str() {
        "local" => match LocalSandboxProvider::from_env() {
            Ok(p) => {
                debug!("sandbox provider: local (Docker/nsjail)");
                Arc::new(p)
            }
            Err(e) => {
                warn!(error = %e, "local provider unavailable, falling back to bubblewrap");
                Arc::new(BubblewrapProvider::from_env())
            }
        },
        _ => {
            // "bubblewrap", "bwrap", or unset — default.
            debug!("sandbox provider: bubblewrap (namespace isolation when available)");
            Arc::new(BubblewrapProvider::from_env())
        }
    }
}

// ── derive_sandbox_spec ───────────────────────────────────────────────────────

/// Derive a [`SandboxSpec`] from a Praxis [`SandboxPolicy`] and a logical
/// entity (e.g. run or session) ID.
///
/// The spec uses ephemeral persistence so sandboxes are destroyed at the end
/// of each logical session and never linger between runs.
pub fn derive_sandbox_spec(policy: &SandboxPolicy, entity_id: &str) -> SandboxSpec {
    use arcan_sandbox::{PersistencePolicy, SandboxResources};
    use std::collections::HashMap;

    let timeout_secs = policy.max_execution_ms.div_ceil(1000);

    SandboxSpec {
        name: format!("praxis-{entity_id}"),
        image: None,
        resources: SandboxResources {
            vcpus: 1,
            memory_mb: 512,
            disk_mb: 1024,
            timeout_secs,
        },
        env: HashMap::new(),
        persistence: PersistencePolicy::Ephemeral,
        capabilities: arcan_sandbox::SandboxCapabilitySet::FILESYSTEM_READ
            | arcan_sandbox::SandboxCapabilitySet::FILESYSTEM_WRITE,
        labels: {
            let mut m = HashMap::new();
            m.insert("entity_id".into(), entity_id.into());
            m
        },
    }
}

// ── SandboxCommandRunner ──────────────────────────────────────────────────────

/// Synchronous [`CommandRunner`] that delegates to an async [`SandboxProvider`].
///
/// Each `run` call creates an ephemeral sandbox, executes the command, and
/// destroys the sandbox.
pub struct SandboxCommandRunner {
    provider: Arc<dyn SandboxProvider>,
}

impl SandboxCommandRunner {
    /// Create a new runner backed by the given provider.
    pub fn new(provider: Arc<dyn SandboxProvider>) -> Self {
        Self { provider }
    }

    /// Create a new runner using the provider selected by `ARCAN_SANDBOX_PROVIDER`.
    pub fn from_env() -> Self {
        Self::new(build_provider())
    }
}

impl CommandRunner for SandboxCommandRunner {
    fn run(&self, policy: &SandboxPolicy, request: &CommandRequest) -> PraxisResult<CommandResult> {
        if !policy.shell_enabled {
            return Err(PraxisError::Sandbox(
                "shell execution is disabled by policy".into(),
            ));
        }

        let provider = self.provider.clone();

        // Build the ExecRequest from the praxis CommandRequest.
        let exec = ExecRequest {
            command: std::iter::once(request.executable.clone())
                .chain(request.args.iter().cloned())
                .collect(),
            working_dir: Some(request.cwd.display().to_string()),
            env: request.env.iter().cloned().collect(),
            timeout_secs: Some(policy.max_execution_ms.div_ceil(1000)),
            stdin: None,
        };

        // Derive a sandbox name from the cwd leaf for observability.
        let entity_id = request
            .cwd
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("praxis");
        let spec = derive_sandbox_spec(policy, entity_id);

        block_on_sandbox(async move {
            // Create an ephemeral sandbox.
            let handle = provider
                .create(spec)
                .await
                .map_err(|e| PraxisError::CommandFailed(format!("sandbox create failed: {e}")))?;

            // Execute the command.
            let run_result = provider.run(&handle.id, exec).await;

            // Best-effort destroy — don't suppress the run error.
            if let Err(e) = provider.destroy(&handle.id).await {
                warn!(sandbox_id = %handle.id, error = %e, "sandbox destroy failed (non-fatal)");
            }

            let exec_result = run_result
                .map_err(|e| PraxisError::CommandFailed(format!("sandbox run failed: {e}")))?;

            debug!(
                sandbox_id = %handle.id,
                exit_code = exec_result.exit_code,
                duration_ms = exec_result.duration_ms,
                "sandbox exec completed"
            );

            let stdout = truncate(&exec_result.stdout, policy.max_stdout_bytes);
            let stderr = truncate(&exec_result.stderr, policy.max_stderr_bytes);

            Ok(CommandResult {
                exit_code: exec_result.exit_code,
                stdout,
                stderr,
            })
        })
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Run `future` to completion using the current tokio runtime when available,
/// otherwise spin up a temporary runtime.
fn block_on_sandbox<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Inside a tokio runtime — use block_in_place to yield the thread.
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Err(_) => {
            // No runtime — build a temporary one.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build temporary tokio runtime")
                .block_on(future)
        }
    }
}

/// Decode bytes as lossy UTF-8 and truncate to `max_bytes`.
fn truncate(bytes: &[u8], max_bytes: usize) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > max_bytes {
        s[..max_bytes].to_owned()
    } else {
        s.into_owned()
    }
}

// ── SandboxSessionLifecycle ───────────────────────────────────────────────────

/// Session lifecycle hooks for a [`SandboxService`]-backed session.
///
/// Wire `on_pause` and `on_end` into your session's pause/end handlers so the
/// provider-level sandbox is snapshotted or destroyed at the right time.
///
/// ```ignore
/// let lifecycle = SandboxSessionLifecycle::new(service, agent_id, session_id);
/// lifecycle.on_pause().await;  // e.g. human-in-the-loop wait
/// lifecycle.on_end().await;    // session teardown
/// ```
pub struct SandboxSessionLifecycle {
    service: Arc<SandboxService>,
    agent_id: String,
    session_id: String,
}

impl SandboxSessionLifecycle {
    pub fn new(
        service: Arc<SandboxService>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            service,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }

    /// Snapshot the session sandbox — call on session pause.
    ///
    /// Errors are logged and swallowed; callers must not fail on snapshot.
    pub async fn on_pause(&self) {
        if let Err(e) = self
            .service
            .snapshot_session(&self.agent_id, &self.session_id)
            .await
        {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                error = %e,
                "snapshot on pause failed (non-fatal)"
            );
        }
    }

    /// Destroy the session sandbox — call on session end.
    ///
    /// Errors are logged and swallowed; callers must not fail on destroy.
    pub async fn on_end(&self) {
        if let Err(e) = self
            .service
            .destroy_session(&self.agent_id, &self.session_id)
            .await
        {
            warn!(
                agent_id = %self.agent_id,
                session_id = %self.session_id,
                error = %e,
                "destroy on end failed (non-fatal)"
            );
        }
    }
}

// ── SandboxServiceRunner ──────────────────────────────────────────────────────

/// Session-scoped [`CommandRunner`] that routes through [`SandboxService`].
///
/// Unlike [`SandboxCommandRunner`] (ephemeral sandbox per call), this runner
/// maintains a **persistent sandbox** for the agent session — files written in
/// one call are visible in the next.  [`SandboxService`] handles
/// create-or-resume transparently.
///
/// Construct one runner per session (not shared across sessions):
///
/// ```ignore
/// let runner = SandboxServiceRunner::new(
///     Arc::clone(&service),
///     agent_id.clone(),
///     session_id.clone(),
/// );
/// let bash_tool = BashTool::new(sandbox_policy, Box::new(runner));
/// ```
pub struct SandboxServiceRunner {
    service: Arc<SandboxService>,
    agent_id: String,
    session_id: String,
}

impl SandboxServiceRunner {
    pub fn new(
        service: Arc<SandboxService>,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            service,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }
}

impl CommandRunner for SandboxServiceRunner {
    fn run(&self, policy: &SandboxPolicy, request: &CommandRequest) -> PraxisResult<CommandResult> {
        if !policy.shell_enabled {
            return Err(PraxisError::Sandbox(
                "shell execution is disabled by policy".into(),
            ));
        }

        let service = Arc::clone(&self.service);
        let agent_id = self.agent_id.clone();
        let session_id = self.session_id.clone();
        let max_stdout = policy.max_stdout_bytes;
        let max_stderr = policy.max_stderr_bytes;

        let exec = ExecRequest {
            command: std::iter::once(request.executable.clone())
                .chain(request.args.iter().cloned())
                .collect(),
            working_dir: Some(request.cwd.display().to_string()),
            env: request.env.iter().cloned().collect(),
            timeout_secs: Some(policy.max_execution_ms.div_ceil(1000)),
            stdin: None,
        };

        block_on_sandbox(async move {
            let result = service
                .run(&agent_id, &session_id, exec)
                .await
                .map_err(|e| PraxisError::CommandFailed(format!("sandbox service run: {e}")))?;

            debug!(
                agent_id = %agent_id,
                session_id = %session_id,
                exit_code = result.exit_code,
                duration_ms = result.duration_ms,
                "service-routed sandbox exec completed"
            );

            Ok(CommandResult {
                exit_code: result.exit_code,
                stdout: truncate(&result.stdout, max_stdout),
                stderr: truncate(&result.stderr, max_stderr),
            })
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use praxis_core::sandbox::SandboxPolicy;
    use std::collections::BTreeSet;

    fn test_policy(dir: &std::path::Path) -> SandboxPolicy {
        SandboxPolicy {
            workspace_root: dir.to_path_buf(),
            shell_enabled: true,
            network: aios_protocol::sandbox::NetworkPolicy::Disabled,
            allowed_env: BTreeSet::new(),
            max_execution_ms: 10_000,
            max_stdout_bytes: 65_536,
            max_stderr_bytes: 65_536,
        }
    }

    #[test]
    fn shell_disabled_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.shell_enabled = false;

        // BubblewrapProvider::from_env() falls back to plain subprocess when bwrap is absent.
        let runner = SandboxCommandRunner::new(Arc::new(BubblewrapProvider::from_env()));
        let req = CommandRequest {
            executable: "echo".into(),
            args: vec!["hello".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };

        let err = runner.run(&policy, &req).unwrap_err();
        assert!(err.to_string().contains("disabled by policy"));
    }

    #[test]
    fn echo_via_bubblewrap_provider() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let runner = SandboxCommandRunner::new(Arc::new(BubblewrapProvider::from_env()));
        let req = CommandRequest {
            executable: "sh".into(),
            args: vec!["-c".into(), "echo sandbox-works".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };

        let result = runner.run(&policy, &req).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("sandbox-works"));
    }

    #[test]
    fn nonzero_exit_propagated() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let runner = SandboxCommandRunner::new(Arc::new(BubblewrapProvider::from_env()));
        let req = CommandRequest {
            executable: "sh".into(),
            args: vec!["-c".into(), "exit 42".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };

        let result = runner.run(&policy, &req).unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn derive_spec_sets_correct_fields() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());
        let spec = derive_sandbox_spec(&policy, "test-run-123");

        assert_eq!(spec.name, "praxis-test-run-123");
        assert!(matches!(
            spec.persistence,
            arcan_sandbox::PersistencePolicy::Ephemeral
        ));
        // timeout_secs = ceil(10_000 / 1000) = 10
        assert_eq!(spec.resources.timeout_secs, 10);
    }

    #[test]
    fn bubblewrap_provider_name() {
        // BubblewrapProvider::from_env falls back to plain subprocess when bwrap is absent.
        let provider = BubblewrapProvider::from_env();
        assert_eq!(provider.name(), "bubblewrap");
    }

    // ── SandboxServiceRunner tests ──────────────────────────────────────────

    fn make_service() -> Arc<SandboxService> {
        use arcan_sandbox::{
            NoopSink, SandboxEventSink, SandboxProvider, SandboxRegistry, SandboxService,
            SandboxServicePolicy,
        };
        let mut registry = SandboxRegistry::new("bubblewrap");
        registry.register(Arc::new(BubblewrapProvider::from_env()) as Arc<dyn SandboxProvider>);
        Arc::new(SandboxService::new(
            registry,
            Arc::new(NoopSink) as Arc<dyn SandboxEventSink>,
            SandboxServicePolicy::free(),
        ))
    }

    #[test]
    fn service_runner_shell_disabled_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.shell_enabled = false;

        let service = make_service();
        let runner = SandboxServiceRunner::new(service, "agent-1", "session-A");
        let req = CommandRequest {
            executable: "echo".into(),
            args: vec!["hello".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };

        let err = runner.run(&policy, &req).unwrap_err();
        assert!(err.to_string().contains("disabled by policy"));
    }

    #[test]
    fn service_runner_echo_works() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let service = make_service();
        let runner = SandboxServiceRunner::new(service, "agent-1", "session-B");
        let req = CommandRequest {
            executable: "sh".into(),
            args: vec!["-c".into(), "echo service-routed".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };

        let result = runner.run(&policy, &req).unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("service-routed"));
    }

    #[test]
    fn service_runner_session_reuse_preserves_files() {
        // Write a file in the first call; read it in the second.
        // With a persistent sandbox (vs ephemeral), the file persists.
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let service = make_service();
        let runner = SandboxServiceRunner::new(Arc::clone(&service), "agent-2", "session-C");

        let write_req = CommandRequest {
            executable: "sh".into(),
            args: vec![
                "-c".into(),
                format!("echo marker > {}/session-file.txt", dir.path().display()),
            ],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };
        let r1 = runner.run(&policy, &write_req).unwrap();
        assert_eq!(r1.exit_code, 0);

        // Second call on the same runner (same session_id) reads the file.
        let read_req = CommandRequest {
            executable: "sh".into(),
            args: vec![
                "-c".into(),
                format!("cat {}/session-file.txt", dir.path().display()),
            ],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };
        let r2 = runner.run(&policy, &read_req).unwrap();
        assert_eq!(r2.exit_code, 0);
        assert!(r2.stdout.contains("marker"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_lifecycle_on_end_destroys_session() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let service = make_service();
        let runner = SandboxServiceRunner::new(Arc::clone(&service), "agent-3", "session-D");

        // Prime the session with one exec.
        let req = CommandRequest {
            executable: "sh".into(),
            args: vec!["-c".into(), "true".into()],
            cwd: dir.path().to_path_buf(),
            env: vec![],
        };
        runner.run(&policy, &req).unwrap();
        assert_eq!(service.session_count(), 1);

        // on_end should destroy it.
        let lifecycle = SandboxSessionLifecycle::new(Arc::clone(&service), "agent-3", "session-D");
        lifecycle.on_end().await;
        assert_eq!(service.session_count(), 0);
    }
}
