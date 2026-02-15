use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use wait_timeout::ChildExt;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Disabled,
    AllowAll,
    AllowList(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub workspace_root: PathBuf,
    pub shell_enabled: bool,
    pub network: NetworkPolicy,
    pub allowed_env: BTreeSet<String>,
    pub max_execution_ms: u64,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub max_processes: u16,
    pub max_memory_mb: u32,
}

impl SandboxPolicy {
    pub fn locked_down(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            shell_enabled: false,
            network: NetworkPolicy::Disabled,
            allowed_env: BTreeSet::new(),
            max_execution_ms: 30_000,
            max_stdout_bytes: 512 * 1024,
            max_stderr_bytes: 512 * 1024,
            max_processes: 16,
            max_memory_mb: 512,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CommandRequest {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        policy: &SandboxPolicy,
        request: &CommandRequest,
    ) -> Result<CommandResult, SandboxError>;
}

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("shell execution is disabled by policy")]
    ShellDisabled,
    #[error("command violates sandbox policy: {0}")]
    PolicyViolation(String),
    #[error("command runner failed: {0}")]
    Runner(String),
    #[error("command timed out after {0}ms")]
    Timeout(u64),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct LocalCommandRunner;

impl LocalCommandRunner {
    /// Validate that a cwd path is within the workspace root.
    fn validate_cwd(
        policy: &SandboxPolicy,
        cwd: &std::path::Path,
    ) -> Result<PathBuf, SandboxError> {
        let resolved = if cwd.is_absolute() {
            cwd.to_path_buf()
        } else {
            policy.workspace_root.join(cwd)
        };

        // Canonicalize both to resolve symlinks
        let canonical = resolved.canonicalize().map_err(SandboxError::Io)?;
        let root = policy
            .workspace_root
            .canonicalize()
            .map_err(SandboxError::Io)?;

        if canonical.starts_with(&root) {
            Ok(canonical)
        } else {
            Err(SandboxError::PolicyViolation(format!(
                "cwd '{}' escapes workspace root '{}'",
                canonical.display(),
                root.display()
            )))
        }
    }

    /// Truncate a byte vector to the given limit, appending a marker if truncated.
    fn truncate_output(raw: &[u8], max_bytes: usize) -> String {
        if raw.len() <= max_bytes {
            String::from_utf8_lossy(raw).to_string()
        } else {
            let truncated = String::from_utf8_lossy(&raw[..max_bytes]).to_string();
            format!(
                "{}\n\n... [truncated: {} bytes total, showing first {}]",
                truncated,
                raw.len(),
                max_bytes
            )
        }
    }
}

impl CommandRunner for LocalCommandRunner {
    fn run(
        &self,
        policy: &SandboxPolicy,
        request: &CommandRequest,
    ) -> Result<CommandResult, SandboxError> {
        if !policy.shell_enabled {
            return Err(SandboxError::ShellDisabled);
        }

        // 1. Prepare command
        let mut cmd = std::process::Command::new(&request.executable);
        cmd.args(&request.args);

        // 2. Validate and set cwd (enforces workspace boundary)
        if let Some(cwd) = &request.cwd {
            let validated_cwd = Self::validate_cwd(policy, cwd)?;
            cmd.current_dir(validated_cwd);
        } else {
            cmd.current_dir(&policy.workspace_root);
        }

        // 3. Environment: clear everything, then allow only explicitly permitted vars.
        //    An empty allowed_env set means NO request env vars are passed through.
        cmd.env_clear();
        for (k, v) in &request.env {
            if policy.allowed_env.contains(k) {
                cmd.env(k, v);
            }
        }
        // Always provide basic env for shell functionality
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("TERM", "xterm-256color");

        // 4. Spawn child process (not .output() — we need timeout control)
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(SandboxError::Io)?;

        // 5. Enforce execution timeout
        let timeout = Duration::from_millis(policy.max_execution_ms);
        match child.wait_timeout(timeout) {
            Ok(Some(status)) => {
                // Process exited within timeout — read output
                let stdout_raw = child
                    .stdout
                    .take()
                    .map(|mut r| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                        buf
                    })
                    .unwrap_or_default();
                let stderr_raw = child
                    .stderr
                    .take()
                    .map(|mut r| {
                        let mut buf = Vec::new();
                        std::io::Read::read_to_end(&mut r, &mut buf).unwrap_or(0);
                        buf
                    })
                    .unwrap_or_default();

                // 6. Enforce output size limits
                let stdout = Self::truncate_output(&stdout_raw, policy.max_stdout_bytes);
                let stderr = Self::truncate_output(&stderr_raw, policy.max_stderr_bytes);

                Ok(CommandResult {
                    exit_code: status.code().unwrap_or(-1),
                    stdout,
                    stderr,
                })
            }
            Ok(None) => {
                // Timeout expired — kill the process
                let _ = child.kill();
                let _ = child.wait(); // reap zombie
                Err(SandboxError::Timeout(policy.max_execution_ms))
            }
            Err(e) => Err(SandboxError::Io(e)),
        }
    }
}

use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use serde_json::json;

pub struct BashTool {
    policy: SandboxPolicy,
    runner: Box<dyn CommandRunner>,
}

impl BashTool {
    pub fn new(policy: SandboxPolicy, runner: Box<dyn CommandRunner>) -> Self {
        Self { policy, runner }
    }
}

impl Tool for BashTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "bash".to_string(),
            description: "Executes a bash command in the sandbox.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command line to execute" },
                    "cwd": { "type": "string", "description": "Working directory (optional)" }
                },
                "required": ["command"]
            }),
            title: Some("Bash Command".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                open_world: true,
                requires_confirmation: true,
                ..Default::default()
            }),
            category: Some("shell".to_string()),
            tags: vec!["shell".to_string(), "exec".to_string()],
            timeout_secs: Some(60),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let command_line = call
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "bash".to_string(),
                message: "Missing 'command' argument".to_string(),
            })?;

        let cwd = call
            .input
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);

        // Naive parsing of command line - in reality we should use words/shlex
        // For "bash" tool, we usually run ["/bin/bash", "-c", command_line]
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), command_line.to_string()],
            cwd,
            env: BTreeMap::new(),
        };

        let result =
            self.runner
                .run(&self.policy, &request)
                .map_err(|e| CoreError::ToolExecution {
                    tool_name: "bash".to_string(),
                    message: e.to_string(),
                })?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "exit_code": result.exit_code,
                "stdout": result.stdout,
                "stderr": result.stderr
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy(dir: &std::path::Path) -> SandboxPolicy {
        SandboxPolicy {
            workspace_root: dir.to_path_buf(),
            shell_enabled: true,
            network: NetworkPolicy::Disabled,
            allowed_env: BTreeSet::new(),
            max_execution_ms: 5_000,
            max_stdout_bytes: 1024,
            max_stderr_bytes: 1024,
            max_processes: 16,
            max_memory_mb: 512,
        }
    }

    // --- Env var filtering ---

    #[test]
    fn empty_allowed_env_denies_all_request_vars() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());
        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "echo $SECRET".to_string()],
            cwd: None,
            env: BTreeMap::from([("SECRET".to_string(), "leaked".to_string())]),
        };

        let result = runner.run(&policy, &request).unwrap();
        // SECRET should NOT be passed through — output should be empty (just newline)
        assert_eq!(result.stdout.trim(), "");
    }

    #[test]
    fn allowed_env_permits_listed_vars() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.allowed_env.insert("MY_VAR".to_string());

        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "echo $MY_VAR".to_string()],
            cwd: None,
            env: BTreeMap::from([("MY_VAR".to_string(), "hello".to_string())]),
        };

        let result = runner.run(&policy, &request).unwrap();
        assert_eq!(result.stdout.trim(), "hello");
    }

    #[test]
    fn allowed_env_filters_unlisted_vars() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.allowed_env.insert("GOOD".to_string());

        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "echo $BAD".to_string()],
            cwd: None,
            env: BTreeMap::from([
                ("GOOD".to_string(), "ok".to_string()),
                ("BAD".to_string(), "leaked".to_string()),
            ]),
        };

        let result = runner.run(&policy, &request).unwrap();
        assert_eq!(result.stdout.trim(), "");
    }

    // --- Cwd validation ---

    #[test]
    fn cwd_within_workspace_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();

        let policy = test_policy(dir.path());
        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "pwd".to_string()],
            cwd: Some(subdir.clone()),
            env: BTreeMap::new(),
        };

        let result = runner.run(&policy, &request).unwrap();
        let canonical_sub = subdir.canonicalize().unwrap();
        assert_eq!(result.stdout.trim(), canonical_sub.display().to_string());
    }

    #[test]
    fn cwd_outside_workspace_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());
        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "pwd".to_string()],
            cwd: Some(PathBuf::from("/tmp")),
            env: BTreeMap::new(),
        };

        let err = runner.run(&policy, &request).unwrap_err();
        assert!(
            matches!(err, SandboxError::PolicyViolation(_)),
            "expected PolicyViolation, got: {err}"
        );
    }

    #[test]
    fn cwd_relative_resolved_against_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("child");
        std::fs::create_dir(&subdir).unwrap();

        let policy = test_policy(dir.path());
        let runner = LocalCommandRunner;

        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "pwd".to_string()],
            cwd: Some(PathBuf::from("child")),
            env: BTreeMap::new(),
        };

        let result = runner.run(&policy, &request).unwrap();
        let canonical_sub = subdir.canonicalize().unwrap();
        assert_eq!(result.stdout.trim(), canonical_sub.display().to_string());
    }

    // --- Shell disabled ---

    #[test]
    fn shell_disabled_rejects_execution() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.shell_enabled = false;

        let runner = LocalCommandRunner;
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "echo hi".to_string()],
            cwd: None,
            env: BTreeMap::new(),
        };

        let err = runner.run(&policy, &request).unwrap_err();
        assert!(matches!(err, SandboxError::ShellDisabled));
    }

    // --- Timeout ---

    #[test]
    fn timeout_kills_long_running_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.max_execution_ms = 500; // 500ms timeout

        let runner = LocalCommandRunner;
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            // Use a busy-wait loop instead of `sleep` since the sandbox strips PATH
            // and `sleep` may not be found as an external command.
            args: vec!["-c".to_string(), "while true; do :; done".to_string()],
            cwd: None,
            env: BTreeMap::new(),
        };

        let start = std::time::Instant::now();
        let err = runner.run(&policy, &request).unwrap_err();
        let elapsed = start.elapsed();

        assert!(
            matches!(err, SandboxError::Timeout(500)),
            "expected Timeout, got: {err}"
        );
        // Should have returned in roughly 500ms, not 30s
        assert!(
            elapsed < Duration::from_secs(5),
            "took too long: {elapsed:?}"
        );
    }

    // --- Output truncation ---

    #[test]
    fn stdout_truncated_at_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.max_stdout_bytes = 50;

        let runner = LocalCommandRunner;
        // Generate ~200 bytes of output
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec![
                "-c".to_string(),
                "python3 -c \"print('A' * 200)\"".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
        };

        let result = runner.run(&policy, &request).unwrap();
        assert!(
            result.stdout.contains("[truncated:"),
            "output should be truncated: {}",
            &result.stdout[..100.min(result.stdout.len())]
        );
    }

    #[test]
    fn stderr_truncated_at_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = test_policy(dir.path());
        policy.max_stderr_bytes = 50;

        let runner = LocalCommandRunner;
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec![
                "-c".to_string(),
                "python3 -c \"import sys; sys.stderr.write('E' * 200)\" 2>&1 1>/dev/null; python3 -c \"import sys; sys.stderr.write('E' * 200)\"".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
        };

        let result = runner.run(&policy, &request).unwrap();
        assert!(
            result.stderr.contains("[truncated:"),
            "stderr should be truncated: {}",
            &result.stderr[..100.min(result.stderr.len())]
        );
    }

    #[test]
    fn small_output_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());

        let runner = LocalCommandRunner;
        let request = CommandRequest {
            executable: "/bin/bash".to_string(),
            args: vec!["-c".to_string(), "echo hello".to_string()],
            cwd: None,
            env: BTreeMap::new(),
        };

        let result = runner.run(&policy, &request).unwrap();
        assert_eq!(result.stdout.trim(), "hello");
        assert!(!result.stdout.contains("[truncated:"));
    }

    // --- Truncate helper unit test ---

    #[test]
    fn truncate_output_within_limit() {
        let data = b"hello world";
        let result = LocalCommandRunner::truncate_output(data, 100);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn truncate_output_exceeds_limit() {
        let data = b"hello world, this is a longer string";
        let result = LocalCommandRunner::truncate_output(data, 11);
        assert!(result.starts_with("hello world"));
        assert!(result.contains("[truncated:"));
        assert!(result.contains("showing first 11"));
    }
}
