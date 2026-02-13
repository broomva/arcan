use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use thiserror::Error;

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
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct LocalCommandRunner;

impl CommandRunner for LocalCommandRunner {
    fn run(
        &self,
        policy: &SandboxPolicy,
        request: &CommandRequest,
    ) -> Result<CommandResult, SandboxError> {
        if !policy.shell_enabled {
            return Err(SandboxError::ShellDisabled);
        }

        // 1. Validate Env
        if let NetworkPolicy::Disabled = policy.network {
            // In a real sandbox we'd block network, here we just trust the runner for now or use unshare
        }

        // 2. Prepare Command
        let mut cmd = std::process::Command::new(&request.executable);
        cmd.args(&request.args);

        if let Some(cwd) = &request.cwd {
            // Resolve cwd against workspace root? Or just allow it if it is within workspace?
            // For now assuming the request.cwd is absolute or relative to workspace.
            // Real implementation would use FsPolicy to resolve.
            cmd.current_dir(cwd);
        } else {
            cmd.current_dir(&policy.workspace_root);
        }

        cmd.env_clear();
        for (k, v) in &request.env {
            if policy.allowed_env.contains(k) || policy.allowed_env.is_empty() {
                // simplistic check
                cmd.env(k, v);
            }
        }

        // Always pass some basic env
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("TERM", "xterm-256color");

        // 3. Execute
        // note: timeouts and memory limits require OS specific logic (wait4, rlimit) not implemented here for brevity
        let output = cmd.output().map_err(SandboxError::Io)?;

        Ok(CommandResult {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
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
            state_patch: None,
        })
    }
}
