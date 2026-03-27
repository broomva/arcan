//! Tool registration: builds and registers Praxis tools into Arcan's registry.
//!
//! The [`register_praxis_tools`] function is the primary entry point.
//! It constructs all canonical Praxis tools from a [`PraxisConfig`],
//! wraps each in a [`PraxisToolBridge`], and registers them into an
//! Arcan [`ToolRegistry`].

use crate::config::PraxisConfig;
use crate::sandbox_runner::{SandboxCommandRunner, SandboxServiceRunner, SandboxSessionLifecycle};
use arcan_core::runtime::ToolRegistry;
use arcan_harness::bridge::PraxisToolBridge;
use arcan_sandbox::SandboxService;
use praxis_core::local_fs::LocalFs;
use praxis_core::sandbox::LocalCommandRunner;
use praxis_tools::edit::EditFileTool;
use praxis_tools::fs::{GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use praxis_tools::memory::{ReadMemoryTool, WriteMemoryTool};
use praxis_tools::shell::BashTool;
use std::sync::Arc;
use tracing::info;

/// Register all Praxis canonical tools into an Arcan [`ToolRegistry`].
///
/// Tools registered:
/// - `read_file` — Read files with hashline tags
/// - `write_file` — Write files within workspace boundary
/// - `edit_file` — Hashline (Blake3) content-addressed editing
/// - `list_dir` — List directory contents
/// - `glob` — Pattern-based file search
/// - `grep` — Regex search within file contents
/// - `bash` — Shell command execution (sandbox-constrained)
/// - `read_memory` / `write_memory` — Agent memory (if memory_dir configured)
///
/// Returns the number of tools registered.
pub fn register_praxis_tools(config: &PraxisConfig, registry: &mut ToolRegistry) -> usize {
    let fs_policy = config.fs_policy();
    let fs: Arc<dyn praxis_core::FsPort> = Arc::new(LocalFs::new(fs_policy));

    let mut count = 0;

    // Filesystem tools
    registry.register(PraxisToolBridge::new(ReadFileTool::new(fs.clone())));
    count += 1;

    registry.register(PraxisToolBridge::new(WriteFileTool::new(fs.clone())));
    count += 1;

    registry.register(PraxisToolBridge::new(ListDirTool::new(fs.clone())));
    count += 1;

    registry.register(PraxisToolBridge::new(GlobTool::new(fs.clone())));
    count += 1;

    registry.register(PraxisToolBridge::new(GrepTool::new(fs.clone())));
    count += 1;

    // Editing tool (hashline / Blake3)
    registry.register(PraxisToolBridge::new(EditFileTool::new(fs)));
    count += 1;

    // Shell tool (sandbox-constrained).
    // When ARCAN_SANDBOX_PROVIDER is set, delegate to a SandboxProvider-backed
    // runner (bubblewrap, local, vercel, …).  Otherwise fall back to the
    // in-process LocalCommandRunner for zero-regression behaviour.
    let sandbox_policy = config.sandbox_policy();
    let runner: Box<dyn praxis_core::sandbox::CommandRunner> =
        if std::env::var("ARCAN_SANDBOX_PROVIDER").is_ok() {
            Box::new(SandboxCommandRunner::from_env())
        } else {
            Box::new(LocalCommandRunner::new())
        };
    registry.register(PraxisToolBridge::new(BashTool::new(sandbox_policy, runner)));
    count += 1;

    // Memory tools (optional — only if memory_dir is configured)
    if let Some(ref memory_dir) = config.memory_dir {
        registry.register(PraxisToolBridge::new(ReadMemoryTool::new(
            memory_dir.clone(),
        )));
        count += 1;

        registry.register(PraxisToolBridge::new(WriteMemoryTool::new(
            memory_dir.clone(),
        )));
        count += 1;
    }

    info!(
        tools_registered = count,
        workspace = %config.workspace_root.display(),
        memory = config.memory_dir.is_some(),
        shell = config.shell_enabled,
        "praxis tools registered in arcan registry"
    );

    count
}

/// Register all Praxis tools using a **session-scoped** [`SandboxService`].
///
/// Unlike [`register_praxis_tools`] (ephemeral sandbox per bash call), the
/// `bash` tool registered here routes through [`SandboxServiceRunner`], which
/// maintains a **persistent sandbox** for the agent session so files written in
/// one call are visible in the next.
///
/// Returns `(count, lifecycle)` — wire `lifecycle.on_pause()` and
/// `lifecycle.on_end()` into your session's pause/end handlers.
pub fn register_praxis_tools_for_session(
    config: &PraxisConfig,
    service: Arc<SandboxService>,
    agent_id: impl Into<String>,
    session_id: impl Into<String>,
    registry: &mut ToolRegistry,
) -> (usize, SandboxSessionLifecycle) {
    let agent_id = agent_id.into();
    let session_id = session_id.into();

    let fs_policy = config.fs_policy();
    let fs: Arc<dyn praxis_core::FsPort> = Arc::new(LocalFs::new(fs_policy));

    let mut count = 0;

    registry.register(PraxisToolBridge::new(ReadFileTool::new(fs.clone())));
    count += 1;
    registry.register(PraxisToolBridge::new(WriteFileTool::new(fs.clone())));
    count += 1;
    registry.register(PraxisToolBridge::new(ListDirTool::new(fs.clone())));
    count += 1;
    registry.register(PraxisToolBridge::new(GlobTool::new(fs.clone())));
    count += 1;
    registry.register(PraxisToolBridge::new(GrepTool::new(fs.clone())));
    count += 1;
    registry.register(PraxisToolBridge::new(EditFileTool::new(fs)));
    count += 1;

    // Shell tool — session-scoped sandbox via SandboxService.
    let sandbox_policy = config.sandbox_policy();
    let runner =
        SandboxServiceRunner::new(Arc::clone(&service), agent_id.clone(), session_id.clone());
    registry.register(PraxisToolBridge::new(BashTool::new(
        sandbox_policy,
        Box::new(runner),
    )));
    count += 1;

    if let Some(ref memory_dir) = config.memory_dir {
        registry.register(PraxisToolBridge::new(ReadMemoryTool::new(
            memory_dir.clone(),
        )));
        count += 1;
        registry.register(PraxisToolBridge::new(WriteMemoryTool::new(
            memory_dir.clone(),
        )));
        count += 1;
    }

    let lifecycle = SandboxSessionLifecycle::new(service, agent_id, session_id);

    info!(
        tools_registered = count,
        workspace = %config.workspace_root.display(),
        "praxis tools registered (SandboxService-backed session)"
    );

    (count, lifecycle)
}

/// Return the list of tool names that [`register_praxis_tools`] will register.
///
/// Useful for documentation and validation without constructing real tools.
pub fn praxis_tool_names(include_memory: bool) -> Vec<&'static str> {
    let mut names = vec![
        "read_file",
        "write_file",
        "list_dir",
        "glob",
        "grep",
        "edit_file",
        "bash",
    ];
    if include_memory {
        names.push("read_memory");
        names.push("write_memory");
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PraxisConfig;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_config(dir: &TempDir) -> PraxisConfig {
        PraxisConfig::new(dir.path())
    }

    fn make_config_with_memory(dir: &TempDir) -> PraxisConfig {
        PraxisConfig::new(dir.path()).with_memory_dir(dir.path().join("memory"))
    }

    #[test]
    fn registers_core_tools() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();

        let count = register_praxis_tools(&config, &mut registry);

        // 7 tools without memory (read_file, write_file, list_dir, glob, grep, edit_file, bash)
        assert_eq!(count, 7);

        // Verify each tool is accessible
        assert!(registry.get("read_file").is_some());
        assert!(registry.get("write_file").is_some());
        assert!(registry.get("list_dir").is_some());
        assert!(registry.get("glob").is_some());
        assert!(registry.get("grep").is_some());
        assert!(registry.get("edit_file").is_some());
        assert!(registry.get("bash").is_some());

        // Memory tools not registered
        assert!(registry.get("read_memory").is_none());
        assert!(registry.get("write_memory").is_none());
    }

    #[test]
    fn registers_memory_tools_when_configured() {
        let dir = TempDir::new().unwrap();
        let config = make_config_with_memory(&dir);
        let mut registry = ToolRegistry::default();

        let count = register_praxis_tools(&config, &mut registry);

        // 9 tools with memory
        assert_eq!(count, 9);

        assert!(registry.get("read_memory").is_some());
        assert!(registry.get("write_memory").is_some());
    }

    #[test]
    fn tool_definitions_are_valid() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let defs = registry.definitions();
        for def in &defs {
            assert!(!def.name.is_empty(), "tool name should not be empty");
            assert!(
                !def.description.is_empty(),
                "tool description should not be empty"
            );
            assert!(
                def.input_schema.is_object(),
                "input_schema should be a JSON object for {}",
                def.name
            );
        }
    }

    #[test]
    fn praxis_tool_names_without_memory() {
        let names = praxis_tool_names(false);
        assert_eq!(names.len(), 7);
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"bash"));
        assert!(!names.contains(&"read_memory"));
    }

    #[test]
    fn praxis_tool_names_with_memory() {
        let names = praxis_tool_names(true);
        assert_eq!(names.len(), 9);
        assert!(names.contains(&"read_memory"));
        assert!(names.contains(&"write_memory"));
    }

    #[test]
    fn read_file_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();

        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("read_file").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-1".into(),
            tool_name: "read_file".into(),
            input: json!({"path": "hello.txt"}),
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        let content = result.output["content"].as_str().unwrap();
        assert!(content.contains("world"));
    }

    #[test]
    fn write_file_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("write_file").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-2".into(),
            tool_name: "write_file".into(),
            input: json!({"path": "output.txt", "content": "hello from bridge"}),
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["success"], true);

        // Verify file was actually written
        let written = std::fs::read_to_string(dir.path().join("output.txt")).unwrap();
        assert_eq!(written, "hello from bridge");
    }

    #[test]
    fn bash_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("bash").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-3".into(),
            tool_name: "bash".into(),
            input: json!({"command": "echo bridge-test"}),
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["exit_code"], 0);
        assert!(
            result.output["stdout"]
                .as_str()
                .unwrap()
                .contains("bridge-test")
        );
    }

    #[test]
    fn glob_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn test() {}").unwrap();
        std::fs::write(dir.path().join("c.txt"), "text file").unwrap();

        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("glob").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-4".into(),
            tool_name: "glob".into(),
            input: json!({"pattern": "*.rs"}),
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["count"], 2);
    }

    #[test]
    fn grep_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("code.rs"),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .unwrap();

        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("grep").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-5".into(),
            tool_name: "grep".into(),
            input: json!({"pattern": "println"}),
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["count"], 1);
    }

    #[test]
    fn workspace_boundary_enforced_through_bridge() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("read_file").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-6".into(),
            tool_name: "read_file".into(),
            input: json!({"path": "/etc/passwd"}),
        };

        // Should fail — path is outside workspace
        let result = tool.execute(&call, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn shell_disabled_rejects_bash() {
        let dir = TempDir::new().unwrap();
        let config = make_config(&dir).with_shell_disabled();
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let tool = registry.get("bash").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-7".into(),
            tool_name: "bash".into(),
            input: json!({"command": "echo should-not-run"}),
        };

        // Should fail — shell is disabled
        let result = tool.execute(&call, &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn memory_tools_through_bridge_work() {
        let dir = TempDir::new().unwrap();
        let config = make_config_with_memory(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };

        // Write memory
        let write_tool = registry.get("write_memory").unwrap();
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-8".into(),
            tool_name: "write_memory".into(),
            input: json!({"key": "bridge-test", "content": "# Memory\nBridge works!"}),
        };
        let result = write_tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["success"], true);

        // Read memory
        let read_tool = registry.get("read_memory").unwrap();
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-9".into(),
            tool_name: "read_memory".into(),
            input: json!({"key": "bridge-test"}),
        };
        let result = read_tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["exists"], true);
        assert!(
            result.output["content"]
                .as_str()
                .unwrap()
                .contains("Bridge works!")
        );
    }

    #[test]
    fn edit_file_through_bridge_works() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("edit-me.txt"), "line1\nline2\nline3").unwrap();

        let config = make_config(&dir);
        let mut registry = ToolRegistry::default();
        register_praxis_tools(&config, &mut registry);

        // First, read the file to get hash tags
        let read_tool = registry.get("read_file").unwrap();
        let ctx = arcan_core::runtime::ToolContext {
            run_id: "test-run".into(),
            session_id: "test-session".into(),
            iteration: 1,
        };
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-10".into(),
            tool_name: "read_file".into(),
            input: json!({"path": "edit-me.txt"}),
        };
        let result = read_tool.execute(&call, &ctx).unwrap();
        let content = result.output["content"].as_str().unwrap();

        // Extract the hash tag for line 2 from the formatted output
        // Format is: "   2 <hash> | line2"
        let line2_tag = content
            .lines()
            .find(|l| l.contains("line2"))
            .unwrap()
            .split_whitespace()
            .nth(1) // the hash tag
            .unwrap();

        // Now edit using the hash tag
        let edit_tool = registry.get("edit_file").unwrap();
        let call = arcan_core::protocol::ToolCall {
            call_id: "call-11".into(),
            tool_name: "edit_file".into(),
            input: json!({
                "path": "edit-me.txt",
                "ops": [
                    { "op": "replace_line", "tag": line2_tag, "new_text": "LINE_TWO_REPLACED" }
                ]
            }),
        };
        let result = edit_tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["success"], true);

        // Verify the file was actually edited
        let final_content = std::fs::read_to_string(dir.path().join("edit-me.txt")).unwrap();
        assert!(final_content.contains("LINE_TWO_REPLACED"));
        assert!(!final_content.contains("line2"));
    }
}
