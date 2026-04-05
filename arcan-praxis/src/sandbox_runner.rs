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
//! | `"vercel"` | [`VercelSandboxProvider`] (Vercel Sandbox HTTP API — requires `VERCEL_TOKEN` (preferred) or `VERCEL_SANDBOX_API_KEY`) |
//! | `"bubblewrap"` / `"bwrap"` / *(unset)* | [`BubblewrapProvider`] (Linux namespaces, falls back to plain subprocess) |

use std::sync::Arc;

use arcan_provider_bubblewrap::BubblewrapProvider;
use arcan_provider_local::LocalSandboxProvider;
use arcan_provider_vercel::VercelSandboxProvider;
use arcan_sandbox::{ExecRequest, SandboxProvider, SandboxSpec};
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
/// | `"vercel"` | [`VercelSandboxProvider`] (Vercel Sandbox HTTP API) — requires `VERCEL_TOKEN` (preferred) or `VERCEL_SANDBOX_API_KEY`; falls back to bwrap if neither is set |
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
        "vercel" => match VercelSandboxProvider::from_env() {
            Ok(p) => {
                debug!("sandbox provider: vercel (Sandbox HTTP API)");
                Arc::new(p)
            }
            Err(e) => {
                warn!(error = %e, "vercel provider unavailable (missing VERCEL_TOKEN / VERCEL_SANDBOX_API_KEY?), falling back to bubblewrap");
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

    #[test]
    fn build_provider_vercel_from_env_returns_err_without_token() {
        // When neither VERCEL_TOKEN nor VERCEL_SANDBOX_API_KEY is present, the
        // Vercel provider construction must fail so build_provider falls back to
        // bubblewrap.  This exercises the Err branch of VercelSandboxProvider::from_env.
        // We rely on the token vars being absent in the test environment; if they happen
        // to be set, skip rather than fail.
        if std::env::var("VERCEL_TOKEN").is_ok() || std::env::var("VERCEL_SANDBOX_API_KEY").is_ok()
        {
            return; // token present — cannot test the fallback path in this environment
        }
        let result = VercelSandboxProvider::from_env();
        assert!(
            result.is_err(),
            "expected Err when no Vercel token is configured"
        );
    }

    #[test]
    fn build_provider_default_is_bubblewrap() {
        // When ARCAN_SANDBOX_PROVIDER is unset, build_provider must return the bubblewrap
        // provider.  We test the default arm by passing an empty/unknown value — calling
        // build_provider() with no ARCAN_SANDBOX_PROVIDER set is safe because the workspace
        // forbids unsafe env mutation, and the default arm is always reachable.
        if std::env::var("ARCAN_SANDBOX_PROVIDER")
            .map(|v| v.to_lowercase())
            .as_deref()
            .unwrap_or("bubblewrap")
            == "bubblewrap"
        {
            let provider = build_provider();
            assert_eq!(provider.name(), "bubblewrap");
        }
        // If ARCAN_SANDBOX_PROVIDER is set to something else, skip to avoid interfering
        // with the test that already set it.
    }
}
