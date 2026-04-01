//! User-configurable hook system for the Arcan agent runtime.
//!
//! Hooks are shell commands that fire on specific agent lifecycle events.
//! They enable users to extend Arcan with custom automation — conversation
//! logging, safety gates, webhook notifications, and more — without modifying
//! the runtime itself.
//!
//! Modeled after Claude Code's 20-event hook system, providing feature parity
//! for the Arcan runtime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read as _;
use std::process::Command;
use std::time::Duration;
use wait_timeout::ChildExt;

/// Events that can trigger hooks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Fires when a new session begins.
    SessionStart,
    /// Fires when a session ends (REPL exit, daemon shutdown).
    SessionEnd,
    /// Fires before a tool is executed. Blocking hooks can deny the operation.
    PreToolUse,
    /// Fires after a tool executes successfully.
    PostToolUse,
    /// Fires after a tool execution fails.
    PostToolUseFailure,
    /// Fires at the start of an agent loop run.
    RunStart,
    /// Fires when an agent loop run completes.
    RunEnd,
    /// Fires before context compaction.
    PreCompact,
    /// Fires after context compaction.
    PostCompact,
    /// Fires when the user submits a prompt.
    UserPromptSubmit,
    /// Fires when configuration changes.
    ConfigChange,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionStart => write!(f, "session_start"),
            Self::SessionEnd => write!(f, "session_end"),
            Self::PreToolUse => write!(f, "pre_tool_use"),
            Self::PostToolUse => write!(f, "post_tool_use"),
            Self::PostToolUseFailure => write!(f, "post_tool_use_failure"),
            Self::RunStart => write!(f, "run_start"),
            Self::RunEnd => write!(f, "run_end"),
            Self::PreCompact => write!(f, "pre_compact"),
            Self::PostCompact => write!(f, "post_compact"),
            Self::UserPromptSubmit => write!(f, "user_prompt_submit"),
            Self::ConfigChange => write!(f, "config_change"),
        }
    }
}

/// A configured hook — a shell command to run on a specific event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// The event that triggers this hook.
    pub event: HookEvent,
    /// Optional matcher — only fire for specific tools (e.g., "bash", "file_edit").
    /// Only relevant for tool-related events (PreToolUse, PostToolUse, PostToolUseFailure).
    pub matcher: Option<String>,
    /// Shell command to execute. Supports `{tool_name}`, `{session_id}`, `{workspace}` placeholders.
    pub command: String,
    /// Timeout in seconds (default 10).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
    /// If true, a non-zero exit code blocks the operation (for Pre* events).
    #[serde(default)]
    pub blocking: bool,
}

fn default_timeout() -> u32 {
    10
}

/// Context passed to hook execution.
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    /// Current session identifier.
    pub session_id: String,
    /// Name of the tool being invoked (for tool-related events).
    pub tool_name: Option<String>,
    /// Input passed to the tool (for tool-related events).
    pub tool_input: Option<serde_json::Value>,
    /// Workspace root path.
    pub workspace: String,
}

/// Result of a single hook execution.
#[derive(Debug)]
pub struct HookResult {
    /// Process exit code (or -1 if the process could not be started).
    pub exit_code: i32,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Whether the hook was killed due to timeout.
    pub timed_out: bool,
}

/// Error returned when a blocking hook denies an operation.
#[derive(Debug, thiserror::Error)]
#[error("hook blocked operation: {message}")]
pub struct HookDenied {
    /// The stderr or summary from the blocking hook.
    pub message: String,
}

/// Registry that holds configured hooks and fires them.
#[derive(Debug, Clone)]
pub struct HookRegistry {
    hooks: Vec<HookConfig>,
}

impl HookRegistry {
    /// Create an empty registry with no hooks.
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Create a registry from a list of hook configurations.
    pub fn from_configs(configs: Vec<HookConfig>) -> Self {
        Self { hooks: configs }
    }

    /// Return the number of configured hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    /// Return whether the registry has no hooks.
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Fire all hooks matching the event and context. Returns results in order.
    ///
    /// For each matching hook: expand placeholders, execute the shell command,
    /// and collect the result. Hooks run sequentially (synchronous execution).
    pub fn fire(&self, event: &HookEvent, ctx: &HookContext) -> Vec<HookResult> {
        self.matching_hooks(event, ctx)
            .into_iter()
            .map(|hook| execute_hook(hook, ctx))
            .collect()
    }

    /// Check if any blocking hook denies the operation.
    ///
    /// Fires all matching hooks for the event. If any hook with `blocking = true`
    /// returns a non-zero exit code, returns `Err(HookDenied)` with the stderr.
    pub fn check_blocking(&self, event: &HookEvent, ctx: &HookContext) -> Result<(), HookDenied> {
        for hook in self.matching_hooks(event, ctx) {
            if !hook.blocking {
                continue;
            }
            let result = execute_hook(hook, ctx);
            if result.exit_code != 0 {
                let message = if result.timed_out {
                    format!(
                        "hook timed out after {}s: {}",
                        hook.timeout_secs, hook.command
                    )
                } else if result.stderr.is_empty() {
                    format!(
                        "hook exited with code {}: {}",
                        result.exit_code, hook.command
                    )
                } else {
                    result.stderr.trim().to_string()
                };
                return Err(HookDenied { message });
            }
        }
        Ok(())
    }

    /// Return hooks that match the given event and context.
    fn matching_hooks<'a>(&'a self, event: &HookEvent, ctx: &HookContext) -> Vec<&'a HookConfig> {
        self.hooks
            .iter()
            .filter(|hook| {
                if &hook.event != event {
                    return false;
                }
                // If the hook has a matcher, check it against the tool name
                if let Some(ref matcher) = hook.matcher {
                    if let Some(ref tool_name) = ctx.tool_name {
                        tool_name == matcher
                    } else {
                        // Hook has a matcher but context has no tool name — skip
                        false
                    }
                } else {
                    true
                }
            })
            .collect()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Expand placeholders in a command string.
fn expand_placeholders(command: &str, ctx: &HookContext) -> String {
    let mut expanded = command.to_string();
    expanded = expanded.replace("{session_id}", &ctx.session_id);
    expanded = expanded.replace("{workspace}", &ctx.workspace);
    if let Some(ref tool_name) = ctx.tool_name {
        expanded = expanded.replace("{tool_name}", tool_name);
    } else {
        expanded = expanded.replace("{tool_name}", "");
    }
    if let Some(ref tool_input) = ctx.tool_input {
        expanded = expanded.replace("{tool_input}", &tool_input.to_string());
    } else {
        expanded = expanded.replace("{tool_input}", "");
    }
    expanded
}

/// Execute a single hook command and return the result.
fn execute_hook(hook: &HookConfig, ctx: &HookContext) -> HookResult {
    let command = expand_placeholders(&hook.command, ctx);
    let timeout = Duration::from_secs(u64::from(hook.timeout_secs));

    // Set up environment variables for the hook
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert("ARCAN_SESSION_ID".to_string(), ctx.session_id.clone());
    env.insert("ARCAN_WORKSPACE".to_string(), ctx.workspace.clone());
    env.insert("ARCAN_HOOK_EVENT".to_string(), hook.event.to_string());
    if let Some(ref tool_name) = ctx.tool_name {
        env.insert("ARCAN_TOOL_NAME".to_string(), tool_name.clone());
    }
    if let Some(ref tool_input) = ctx.tool_input {
        env.insert("ARCAN_TOOL_INPUT".to_string(), tool_input.to_string());
    }

    let child_result = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .envs(&env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match child_result {
        Ok(c) => c,
        Err(e) => {
            return HookResult {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("failed to spawn hook: {e}"),
                timed_out: false,
            };
        }
    };

    // Wait with timeout
    match child.wait_timeout(timeout) {
        Ok(Some(status)) => {
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(ref mut out) = child.stdout {
                let _ = out.read_to_string(&mut stdout);
            }
            if let Some(ref mut err) = child.stderr {
                let _ = err.read_to_string(&mut stderr);
            }
            HookResult {
                exit_code: status.code().unwrap_or(-1),
                stdout,
                stderr,
                timed_out: false,
            }
        }
        Ok(None) => {
            // Timed out — kill the child
            let _ = child.kill();
            let _ = child.wait();
            let mut stdout = String::new();
            let mut stderr = String::new();
            if let Some(ref mut out) = child.stdout {
                let _ = out.read_to_string(&mut stdout);
            }
            if let Some(ref mut err) = child.stderr {
                let _ = err.read_to_string(&mut stderr);
            }
            HookResult {
                exit_code: -1,
                stdout,
                stderr,
                timed_out: true,
            }
        }
        Err(e) => HookResult {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("failed to wait on hook: {e}"),
            timed_out: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> HookContext {
        HookContext {
            session_id: "test-session-42".to_string(),
            tool_name: Some("bash".to_string()),
            tool_input: Some(serde_json::json!({"command": "ls"})),
            workspace: "/tmp/test-workspace".to_string(),
        }
    }

    #[test]
    fn test_fire_matching_hooks() {
        let registry = HookRegistry::from_configs(vec![
            HookConfig {
                event: HookEvent::SessionStart,
                matcher: None,
                command: "echo session_start".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
            HookConfig {
                event: HookEvent::RunEnd,
                matcher: None,
                command: "echo run_end".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
            HookConfig {
                event: HookEvent::SessionStart,
                matcher: None,
                command: "echo session_start_2".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
        ]);

        let ctx = make_ctx();

        // SessionStart should fire 2 hooks
        let results = registry.fire(&HookEvent::SessionStart, &ctx);
        assert_eq!(results.len(), 2);
        assert!(results[0].stdout.contains("session_start"));
        assert!(results[1].stdout.contains("session_start_2"));

        // RunEnd should fire 1 hook
        let results = registry.fire(&HookEvent::RunEnd, &ctx);
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("run_end"));

        // PreToolUse should fire 0 hooks
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_matcher_filters() {
        let registry = HookRegistry::from_configs(vec![
            HookConfig {
                event: HookEvent::PreToolUse,
                matcher: Some("bash".to_string()),
                command: "echo matched_bash".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
            HookConfig {
                event: HookEvent::PreToolUse,
                matcher: Some("file_edit".to_string()),
                command: "echo matched_file_edit".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
            HookConfig {
                event: HookEvent::PreToolUse,
                matcher: None,
                command: "echo matched_all".to_string(),
                timeout_secs: 5,
                blocking: false,
            },
        ]);

        // With tool_name = "bash", should match "bash" matcher + no-matcher
        let ctx = HookContext {
            tool_name: Some("bash".to_string()),
            ..make_ctx()
        };
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 2);
        assert!(results[0].stdout.contains("matched_bash"));
        assert!(results[1].stdout.contains("matched_all"));

        // With tool_name = "file_edit", should match "file_edit" matcher + no-matcher
        let ctx = HookContext {
            tool_name: Some("file_edit".to_string()),
            ..make_ctx()
        };
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 2);
        assert!(results[0].stdout.contains("matched_file_edit"));
        assert!(results[1].stdout.contains("matched_all"));

        // With no tool_name, should only match no-matcher
        let ctx = HookContext {
            tool_name: None,
            ..make_ctx()
        };
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 1);
        assert!(results[0].stdout.contains("matched_all"));
    }

    #[test]
    fn test_blocking_hook_denies() {
        let registry = HookRegistry::from_configs(vec![
            HookConfig {
                event: HookEvent::PreToolUse,
                matcher: None,
                command: "echo 'allowed' && exit 0".to_string(),
                timeout_secs: 5,
                blocking: true,
            },
            HookConfig {
                event: HookEvent::PreToolUse,
                matcher: None,
                command: "echo 'denied' >&2 && exit 1".to_string(),
                timeout_secs: 5,
                blocking: true,
            },
        ]);

        let ctx = make_ctx();
        let result = registry.check_blocking(&HookEvent::PreToolUse, &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("denied"),
            "expected 'denied' in error: {}",
            err.message
        );
    }

    #[test]
    fn test_blocking_hook_allows() {
        let registry = HookRegistry::from_configs(vec![HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "exit 0".to_string(),
            timeout_secs: 5,
            blocking: true,
        }]);

        let ctx = make_ctx();
        let result = registry.check_blocking(&HookEvent::PreToolUse, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_non_blocking_hooks_ignored_in_check() {
        // Non-blocking hooks with non-zero exit should NOT cause denial
        let registry = HookRegistry::from_configs(vec![HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "exit 1".to_string(),
            timeout_secs: 5,
            blocking: false, // not blocking
        }]);

        let ctx = make_ctx();
        let result = registry.check_blocking(&HookEvent::PreToolUse, &ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_timeout_handling() {
        let registry = HookRegistry::from_configs(vec![HookConfig {
            event: HookEvent::RunEnd,
            matcher: None,
            command: "sleep 30".to_string(),
            timeout_secs: 1, // 1 second timeout
            blocking: false,
        }]);

        let ctx = make_ctx();
        let results = registry.fire(&HookEvent::RunEnd, &ctx);
        assert_eq!(results.len(), 1);
        assert!(results[0].timed_out);
        assert_eq!(results[0].exit_code, -1);
    }

    #[test]
    fn test_placeholder_expansion() {
        let registry = HookRegistry::from_configs(vec![HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command: "echo 'tool={tool_name} session={session_id} ws={workspace}'".to_string(),
            timeout_secs: 5,
            blocking: false,
        }]);

        let ctx = make_ctx();
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].exit_code, 0);
        let stdout = &results[0].stdout;
        assert!(
            stdout.contains("tool=bash"),
            "expected tool=bash in stdout: {stdout}"
        );
        assert!(
            stdout.contains("session=test-session-42"),
            "expected session=test-session-42 in stdout: {stdout}"
        );
        assert!(
            stdout.contains("ws=/tmp/test-workspace"),
            "expected ws=/tmp/test-workspace in stdout: {stdout}"
        );
    }

    #[test]
    fn test_environment_variables_set() {
        let registry = HookRegistry::from_configs(vec![HookConfig {
            event: HookEvent::PreToolUse,
            matcher: None,
            command:
                "echo \"$ARCAN_SESSION_ID|$ARCAN_WORKSPACE|$ARCAN_TOOL_NAME|$ARCAN_HOOK_EVENT\""
                    .to_string(),
            timeout_secs: 5,
            blocking: false,
        }]);

        let ctx = make_ctx();
        let results = registry.fire(&HookEvent::PreToolUse, &ctx);
        assert_eq!(results.len(), 1);
        let stdout = results[0].stdout.trim();
        assert_eq!(
            stdout,
            "test-session-42|/tmp/test-workspace|bash|pre_tool_use"
        );
    }

    #[test]
    fn test_empty_registry() {
        let registry = HookRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        let ctx = make_ctx();
        let results = registry.fire(&HookEvent::SessionStart, &ctx);
        assert!(results.is_empty());

        // check_blocking on empty registry should always succeed
        assert!(
            registry
                .check_blocking(&HookEvent::PreToolUse, &ctx)
                .is_ok()
        );
    }

    #[test]
    fn test_hook_event_serde_roundtrip() {
        let events = vec![
            HookEvent::SessionStart,
            HookEvent::SessionEnd,
            HookEvent::PreToolUse,
            HookEvent::PostToolUse,
            HookEvent::PostToolUseFailure,
            HookEvent::RunStart,
            HookEvent::RunEnd,
            HookEvent::PreCompact,
            HookEvent::PostCompact,
            HookEvent::UserPromptSubmit,
            HookEvent::ConfigChange,
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let deserialized: HookEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(event, deserialized);
        }
    }

    #[test]
    fn test_hook_config_serde_defaults() {
        let json = r#"{"event":"run_end","command":"echo done"}"#;
        let config: HookConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.event, HookEvent::RunEnd);
        assert_eq!(config.command, "echo done");
        assert_eq!(config.timeout_secs, 10); // default
        assert!(!config.blocking); // default
        assert!(config.matcher.is_none());
    }
}
