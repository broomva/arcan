//! Core protocol types for the provider-agnostic sandbox abstraction.

use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::capability::SandboxCapabilitySet;

// ── Identity ─────────────────────────────────────────────────────────────────

/// Opaque, globally unique identifier for a sandbox instance.
///
/// The format is provider-dependent; treat as an opaque string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SandboxId(pub String);

impl fmt::Display for SandboxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SandboxId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SandboxId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Opaque identifier for a filesystem snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(pub String);

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Resource limits ───────────────────────────────────────────────────────────

/// Compute resource limits for a sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxResources {
    /// Number of virtual CPUs.
    pub vcpus: u32,
    /// RAM limit in megabytes.
    pub memory_mb: u32,
    /// Disk quota in megabytes.
    pub disk_mb: u32,
    /// Maximum wall-clock seconds for a single `exec` call.
    pub timeout_secs: u64,
}

impl Default for SandboxResources {
    fn default() -> Self {
        Self { vcpus: 1, memory_mb: 512, disk_mb: 2048, timeout_secs: 60 }
    }
}

// ── Persistence policy ────────────────────────────────────────────────────────

/// Controls how a sandbox's filesystem is retained across sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersistencePolicy {
    /// Sandbox is destroyed when the session ends; no filesystem retention.
    Ephemeral,
    /// Filesystem is automatically snapshotted after `idle_timeout_secs` of
    /// inactivity and restored on next `resume()`.
    Persistent {
        /// Seconds of idle time before the provider auto-snapshots.
        ///
        /// E2B pause has ~4 s latency per GiB RAM, so set this to at least 60.
        idle_timeout_secs: u64,
    },
    /// Snapshot only when the caller explicitly invokes `snapshot()`.
    ManualSnapshot,
}

impl Default for PersistencePolicy {
    fn default() -> Self {
        Self::Ephemeral
    }
}

// ── Specification ─────────────────────────────────────────────────────────────

/// Full specification used to create a new sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxSpec {
    /// Human-readable name; used for dedup and resume lookup.
    pub name: String,
    /// Container / VM image reference.
    ///
    /// Interpretation is provider-specific:
    /// - Local/Bubblewrap: OCI image tag (e.g., `"ubuntu:22.04"`)
    /// - E2B: template ID
    /// - Vercel: ignored (image selection is implicit)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Compute resource limits.
    #[serde(default)]
    pub resources: SandboxResources,
    /// Environment variables injected at sandbox startup.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Filesystem persistence strategy.
    #[serde(default)]
    pub persistence: PersistencePolicy,
    /// Capability subset granted to this sandbox.
    #[serde(default)]
    pub capabilities: SandboxCapabilitySet,
    /// Arbitrary key-value labels for routing, billing, and audit.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl SandboxSpec {
    /// Create a minimal ephemeral spec with just a name.
    pub fn ephemeral(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: None,
            resources: SandboxResources::default(),
            env: HashMap::new(),
            persistence: PersistencePolicy::Ephemeral,
            capabilities: SandboxCapabilitySet::default(),
            labels: HashMap::new(),
        }
    }
}

// ── Handle & Status ───────────────────────────────────────────────────────────

/// Current lifecycle state of a sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SandboxStatus {
    /// Sandbox is being provisioned.
    Starting,
    /// Sandbox is ready to accept `exec` calls.
    Running,
    /// Sandbox has been snapshotted and is not currently consuming resources.
    Snapshotted,
    /// Sandbox is in the process of being destroyed.
    Stopping,
    /// Sandbox has been cleanly stopped.
    Stopped,
    /// Sandbox encountered an unrecoverable error.
    Failed {
        /// Description of the failure.
        reason: String,
    },
}

/// A live reference to a sandbox instance returned by `create()` or `resume()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxHandle {
    /// Stable identifier for this sandbox.
    pub id: SandboxId,
    /// Human-readable name from the spec.
    pub name: String,
    /// Current lifecycle state.
    pub status: SandboxStatus,
    /// Wall clock time when the sandbox was first created.
    pub created_at: DateTime<Utc>,
    /// Name of the provider that owns this sandbox.
    pub provider: String,
    /// Provider-specific opaque metadata (serialized JSON).
    pub metadata: serde_json::Value,
}

/// Lightweight summary used in `SandboxProvider::list()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    /// Stable identifier.
    pub id: SandboxId,
    /// Human-readable name.
    pub name: String,
    /// Current lifecycle state.
    pub status: SandboxStatus,
    /// Wall clock time when the sandbox was created.
    pub created_at: DateTime<Utc>,
}

// ── Exec ──────────────────────────────────────────────────────────────────────

/// A command to execute inside a running sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecRequest {
    /// Full `argv`: `command[0]` is the executable, the rest are arguments.
    pub command: Vec<String>,
    /// Working directory inside the sandbox. `None` uses the provider default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Additional environment variables that override the spec env.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Per-request timeout override. `None` falls back to `SandboxResources::timeout_secs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// Optional bytes written to the process's stdin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<Vec<u8>>,
}

impl ExecRequest {
    /// Convenience constructor for a simple shell command with no overrides.
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            command: vec!["/bin/sh".into(), "-c".into(), command.into()],
            working_dir: None,
            env: HashMap::new(),
            timeout_secs: None,
            stdin: None,
        }
    }
}

/// The outcome of an `exec` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    /// Raw bytes written to stdout.
    pub stdout: Vec<u8>,
    /// Raw bytes written to stderr.
    pub stderr: Vec<u8>,
    /// Process exit code.
    pub exit_code: i32,
    /// Wall-clock milliseconds from process start to exit.
    pub duration_ms: u64,
}

impl ExecResult {
    /// `stdout` decoded as lossy UTF-8.
    pub fn stdout_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    /// `stderr` decoded as lossy UTF-8.
    pub fn stderr_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }

    /// Returns `true` if the process exited with code 0.
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_id_display() {
        assert_eq!(SandboxId("abc".into()).to_string(), "abc");
    }

    #[test]
    fn spec_ephemeral_defaults() {
        let spec = SandboxSpec::ephemeral("test");
        assert_eq!(spec.name, "test");
        assert!(spec.image.is_none());
        assert_eq!(spec.persistence, PersistencePolicy::Ephemeral);
        assert_eq!(spec.capabilities, SandboxCapabilitySet::FILESYSTEM_READ);
    }

    #[test]
    fn resources_default() {
        let r = SandboxResources::default();
        assert_eq!(r.vcpus, 1);
        assert_eq!(r.memory_mb, 512);
        assert_eq!(r.disk_mb, 2048);
        assert_eq!(r.timeout_secs, 60);
    }

    #[test]
    fn persistence_policy_serde_roundtrip() {
        for policy in [
            PersistencePolicy::Ephemeral,
            PersistencePolicy::Persistent { idle_timeout_secs: 120 },
            PersistencePolicy::ManualSnapshot,
        ] {
            let json = serde_json::to_string(&policy).unwrap();
            let back: PersistencePolicy = serde_json::from_str(&json).unwrap();
            assert_eq!(policy, back);
        }
    }

    #[test]
    fn sandbox_status_serde_roundtrip() {
        for status in [
            SandboxStatus::Starting,
            SandboxStatus::Running,
            SandboxStatus::Snapshotted,
            SandboxStatus::Stopping,
            SandboxStatus::Stopped,
            SandboxStatus::Failed { reason: "oom".into() },
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: SandboxStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }

    #[test]
    fn exec_request_shell_helper() {
        let req = ExecRequest::shell("echo hello");
        assert_eq!(req.command[0], "/bin/sh");
        assert_eq!(req.command[2], "echo hello");
        assert!(req.working_dir.is_none());
        assert!(req.stdin.is_none());
    }

    #[test]
    fn exec_result_success_and_str_helpers() {
        let result = ExecResult {
            stdout: b"hello\n".to_vec(),
            stderr: vec![],
            exit_code: 0,
            duration_ms: 42,
        };
        assert!(result.success());
        assert_eq!(result.stdout_str(), "hello\n");

        let failure =
            ExecResult { stdout: vec![], stderr: b"oops".to_vec(), exit_code: 1, duration_ms: 1 };
        assert!(!failure.success());
        assert_eq!(failure.stderr_str(), "oops");
    }

    #[test]
    fn sandbox_id_serde_roundtrip() {
        let id = SandboxId("my-sandbox-id".into());
        let json = serde_json::to_string(&id).unwrap();
        let back: SandboxId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        // transparent — should serialize as plain string
        assert_eq!(json, "\"my-sandbox-id\"");
    }
}
