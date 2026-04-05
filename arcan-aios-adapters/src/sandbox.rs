//! Per-session sandbox directory lifecycle for Arcan (BRO-215).
//!
//! Restricted tiers (anonymous, free, default) receive a dedicated
//! session-scoped workspace under `{data_dir}/sessions/{session_id}/`.
//! Pro and enterprise sessions retain access to the full workspace root,
//! with no additional path restriction beyond the existing `FsPolicy`.
//!
//! # Tier mapping
//!
//! | Tier        | Sandbox root                                |
//! |-------------|---------------------------------------------|
//! | anonymous   | `{data_dir}/sessions/{session_id}/`         |
//! | free        | `{data_dir}/sessions/{session_id}/`         |
//! | default     | `{data_dir}/sessions/{session_id}/`         |
//! | pro         | `None` (full workspace root)                |
//! | enterprise  | `None` (full workspace root)                |
//!
//! The underlying `FsPolicy` (in praxis-core) prevents directory traversal
//! by enforcing `canonicalize() + starts_with(workspace_root)`. This module
//! adds per-session directory isolation on top of that enforcement.

use std::path::PathBuf;

use aios_protocol::PolicySet;

use crate::capability_map::tools_allowed_by_policy;

/// Per-session sandbox directory lifecycle manager.
///
/// # Usage
///
/// ```no_run
/// # use arcan_aios_adapters::SandboxEnforcer;
/// # use aios_protocol::PolicySet;
/// # let data_dir = std::path::PathBuf::from("/tmp/arcan-data");
/// # let session_id = "sess-abc123";
/// # let policy = PolicySet::anonymous();
/// let enforcer = SandboxEnforcer::new(&data_dir);
/// if let Some(sandbox) = enforcer.prepare(session_id, &policy).unwrap() {
///     // inject sandbox path into the LLM system prompt
///     println!("Session workspace: {}", sandbox.display());
/// }
/// ```
pub struct SandboxEnforcer {
    data_dir: PathBuf,
}

impl SandboxEnforcer {
    /// Create a new enforcer rooted at `data_dir`.
    ///
    /// `data_dir` should be within the server's workspace root so that the
    /// existing `FsPolicy` grants access to sandbox directories without
    /// requiring a separate whitelist.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Prepare a sandbox directory for the given session and policy tier.
    ///
    /// Returns `None` for pro/enterprise sessions (no sandbox, full workspace
    /// access). Returns `Some(path)` for restricted tiers, creating the
    /// directory if it does not exist.
    ///
    /// Session IDs are sanitised before use as directory names: any character
    /// other than alphanumeric, `-`, or `_` is replaced with `_`.
    pub fn prepare(
        &self,
        session_id: &str,
        policy: &PolicySet,
    ) -> std::io::Result<Option<PathBuf>> {
        // Reuse BRO-214 tier detection: `None` → wildcard / unrestricted tier.
        if tools_allowed_by_policy(policy).is_none() {
            return Ok(None);
        }
        let sandbox = self.sandbox_path(session_id);
        std::fs::create_dir_all(&sandbox)?;
        Ok(Some(sandbox))
    }

    /// Remove the sandbox directory for `session_id`.
    ///
    /// Intended for anonymous / ephemeral session cleanup after a run.
    /// Silently succeeds if the directory does not exist.
    pub fn cleanup(&self, session_id: &str) -> std::io::Result<()> {
        let path = self.sandbox_path(session_id);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        Ok(())
    }

    /// Compute the expected sandbox path for a session without creating it.
    pub fn sandbox_path(&self, session_id: &str) -> PathBuf {
        self.data_dir.join("sessions").join(sanitize_id(session_id))
    }
}

/// Sanitise an arbitrary session ID string for use as a directory component.
///
/// Any character that is not alphanumeric, `-`, or `_` is replaced with `_`.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro_policy_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let result = enforcer.prepare("sess-123", &PolicySet::pro()).unwrap();
        assert!(result.is_none(), "pro tier must not create a sandbox");
    }

    #[test]
    fn enterprise_policy_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let result = enforcer
            .prepare("sess-456", &PolicySet::enterprise())
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn anonymous_policy_creates_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let sandbox = enforcer
            .prepare("anon-session-1", &PolicySet::anonymous())
            .unwrap()
            .expect("anonymous must have a sandbox");
        assert!(sandbox.exists(), "sandbox directory must be created");
        assert!(sandbox.starts_with(dir.path()));
    }

    #[test]
    fn free_policy_creates_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let sandbox = enforcer
            .prepare("free-user-session", &PolicySet::free())
            .unwrap()
            .expect("free tier must have a sandbox");
        assert!(sandbox.exists());
    }

    #[test]
    fn default_policy_creates_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let sandbox = enforcer
            .prepare("default-session", &PolicySet::default())
            .unwrap()
            .expect("default tier must have a sandbox");
        assert!(sandbox.exists());
    }

    #[test]
    fn cleanup_removes_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let sandbox = enforcer
            .prepare("cleanup-test", &PolicySet::anonymous())
            .unwrap()
            .unwrap();
        assert!(sandbox.exists());
        enforcer.cleanup("cleanup-test").unwrap();
        assert!(!sandbox.exists());
    }

    #[test]
    fn cleanup_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        // Cleanup of non-existent sandbox must not error.
        enforcer.cleanup("nonexistent-session").unwrap();
    }

    #[test]
    fn sanitize_id_preserves_valid_chars() {
        assert_eq!(sanitize_id("abc-123_def"), "abc-123_def");
    }

    #[test]
    fn sanitize_id_replaces_slashes() {
        assert_eq!(sanitize_id("abc/def"), "abc_def");
    }

    #[test]
    fn sanitize_id_replaces_colons() {
        assert_eq!(sanitize_id("abc:def:ghi"), "abc_def_ghi");
    }

    #[test]
    fn sanitize_id_replaces_spaces() {
        assert_eq!(sanitize_id("a b c"), "a_b_c");
    }

    #[test]
    fn prepare_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let enforcer = SandboxEnforcer::new(dir.path());
        let p1 = enforcer
            .prepare("session", &PolicySet::anonymous())
            .unwrap();
        let p2 = enforcer
            .prepare("session", &PolicySet::anonymous())
            .unwrap();
        assert_eq!(p1, p2, "repeated prepare must return the same path");
    }
}
