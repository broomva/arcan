//! Configuration for the Praxis tool integration.
//!
//! [`PraxisConfig`] captures the workspace root, sandbox constraints,
//! and optional memory directory for constructing Praxis tools.

use aios_protocol::sandbox::NetworkPolicy;
use praxis_core::sandbox::SandboxPolicy;
use praxis_core::workspace::FsPolicy;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Configuration for wiring Praxis tools into Arcan.
///
/// Captures workspace root (for FsPolicy) and sandbox constraints
/// (for SandboxPolicy / BashTool). All fields have sensible defaults.
#[derive(Debug, Clone)]
pub struct PraxisConfig {
    /// Root directory for workspace boundary enforcement.
    pub workspace_root: PathBuf,
    /// Whether shell (bash) tool execution is enabled.
    pub shell_enabled: bool,
    /// Network access policy for sandboxed commands.
    pub network: NetworkPolicy,
    /// Environment variables allowed through the sandbox.
    pub allowed_env: BTreeSet<String>,
    /// Maximum command execution time in milliseconds.
    pub max_execution_ms: u64,
    /// Maximum stdout size in bytes.
    pub max_stdout_bytes: usize,
    /// Maximum stderr size in bytes.
    pub max_stderr_bytes: usize,
    /// Optional directory for agent memory files.
    /// When `None`, memory tools are not registered.
    pub memory_dir: Option<PathBuf>,
}

impl PraxisConfig {
    /// Create a config with the given workspace root and sensible defaults.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            shell_enabled: true,
            network: NetworkPolicy::Disabled,
            allowed_env: BTreeSet::from([
                "PATH".to_string(),
                "HOME".to_string(),
                "LANG".to_string(),
                "TERM".to_string(),
            ]),
            max_execution_ms: 120_000,
            max_stdout_bytes: 512 * 1024,
            max_stderr_bytes: 512 * 1024,
            memory_dir: None,
        }
    }

    /// Set a memory directory for agent memory tools.
    pub fn with_memory_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.memory_dir = Some(dir.into());
        self
    }

    /// Disable shell execution.
    pub fn with_shell_disabled(mut self) -> Self {
        self.shell_enabled = false;
        self
    }

    /// Set the network policy.
    pub fn with_network(mut self, policy: NetworkPolicy) -> Self {
        self.network = policy;
        self
    }

    /// Set the maximum command execution time in milliseconds.
    pub fn with_max_execution_ms(mut self, ms: u64) -> Self {
        self.max_execution_ms = ms;
        self
    }

    /// Build an [`FsPolicy`] from this config.
    pub fn fs_policy(&self) -> FsPolicy {
        FsPolicy::new(&self.workspace_root)
    }

    /// Build a [`SandboxPolicy`] from this config.
    pub fn sandbox_policy(&self) -> SandboxPolicy {
        SandboxPolicy {
            workspace_root: self.workspace_root.clone(),
            shell_enabled: self.shell_enabled,
            network: self.network.clone(),
            allowed_env: self.allowed_env.clone(),
            max_execution_ms: self.max_execution_ms,
            max_stdout_bytes: self.max_stdout_bytes,
            max_stderr_bytes: self.max_stderr_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn config_defaults_are_sensible() {
        let config = PraxisConfig::new("/tmp/workspace");
        assert!(config.shell_enabled);
        assert!(config.memory_dir.is_none());
        assert_eq!(config.max_execution_ms, 120_000);
        assert!(config.allowed_env.contains("PATH"));
        assert!(config.allowed_env.contains("HOME"));
    }

    #[test]
    fn config_builder_methods() {
        let config = PraxisConfig::new("/tmp/workspace")
            .with_memory_dir("/tmp/memory")
            .with_shell_disabled()
            .with_max_execution_ms(30_000);

        assert!(!config.shell_enabled);
        assert_eq!(config.memory_dir.as_deref(), Some(Path::new("/tmp/memory")));
        assert_eq!(config.max_execution_ms, 30_000);
    }

    #[test]
    fn fs_policy_from_config() {
        let config = PraxisConfig::new("/tmp/workspace");
        let policy = config.fs_policy();
        assert_eq!(policy.workspace_root(), Path::new("/tmp/workspace"));
    }

    #[test]
    fn sandbox_policy_from_config() {
        let config = PraxisConfig::new("/tmp/workspace")
            .with_shell_disabled()
            .with_max_execution_ms(5000);

        let policy = config.sandbox_policy();
        assert!(!policy.shell_enabled);
        assert_eq!(policy.max_execution_ms, 5000);
        assert_eq!(policy.workspace_root, PathBuf::from("/tmp/workspace"));
    }
}
