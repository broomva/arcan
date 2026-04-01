//! Slash command system for the Arcan agent runtime.
//!
//! Provides a [`CommandRegistry`] that dispatches `/`-prefixed user input to
//! built-in commands (`/help`, `/clear`, `/cost`, `/quit`, `/diff`).

mod clear;
mod commit;
mod compact;
mod config_cmd;
mod cost;
mod diff;
mod help;
mod history;
mod memory;
mod model;
mod quit;
mod skill;
mod status;
mod undo;

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// Result of executing a slash command.
#[derive(Debug)]
pub enum CommandResult {
    /// Text output to display to the user.
    Output(String),
    /// Clear the conversation history and start a new session.
    ClearSession,
    /// Compact conversation history to reduce token usage.
    CompactRequested,
    /// Exit the REPL.
    Quit,
    /// An error occurred during command execution.
    Error(String),
}

/// Mutable context passed to every command invocation.
#[derive(Debug, Default)]
pub struct CommandContext {
    /// Accumulated cost in USD for this session.
    pub session_cost_usd: f64,
    /// Input tokens consumed this session.
    pub session_input_tokens: u64,
    /// Output tokens consumed this session.
    pub session_output_tokens: u64,
    /// Number of user turns in this session.
    pub session_turns: u32,
    /// Workspace root directory.
    pub workspace: PathBuf,
    /// Pre-rendered help text (set by the registry).
    pub help_text: String,
    /// Tools the user has permanently approved for this session (via "always" response).
    pub session_approved_tools: HashSet<String>,
    /// Permission mode: "default" (prompt), "yes" (auto-approve all), "plan" (deny all writes).
    pub permission_mode: PermissionMode,
    /// Directory for persistent agent memory files (`.arcan/memory/`).
    pub memory_dir: PathBuf,
    /// Provider name (e.g. "anthropic", "openai", "mock").
    pub provider_name: String,
    /// Current model name (e.g. "claude-sonnet-4-20250514").
    pub model_name: String,
    /// Model override requested via `/model` command (applied on next turn).
    pub model_override: Option<String>,
    /// Data directory for persistent storage (`.arcan/`).
    pub data_dir: PathBuf,
    /// Number of messages in the conversation history.
    pub message_count: usize,
    /// Number of tool calls executed this session.
    pub tool_call_count: usize,
    /// Number of registered tools.
    pub tools_count: usize,
    /// Number of registered hooks.
    pub hooks_count: usize,
    /// Names of discovered skills.
    pub skill_names: Vec<String>,
}

/// Permission mode governing tool approval in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionMode {
    /// Prompt the user for non-read-only tools.
    #[default]
    Default,
    /// Auto-approve all tools (--yes flag).
    Yes,
    /// Plan mode: deny all write/destructive tools.
    Plan,
}

/// Well-known read-only tool names that never require permission prompts.
const READ_ONLY_TOOLS: &[&str] = &[
    "glob",
    "grep",
    "file_read",
    "list_dir",
    "read_file",
    "list_directory",
    "memory_read",
    "read_memory",
];

/// Determine whether a tool requires user permission before execution.
///
/// Returns `true` if the tool should be auto-approved (no prompt needed),
/// `false` if the user must be prompted.
pub fn is_tool_auto_approved(
    tool_name: &str,
    permission_mode: PermissionMode,
    session_approved: &HashSet<String>,
    is_read_only_annotation: bool,
) -> bool {
    // --yes mode: everything is auto-approved
    if permission_mode == PermissionMode::Yes {
        return true;
    }

    // Tools with read_only annotation or in the well-known list
    if is_read_only_annotation || READ_ONLY_TOOLS.contains(&tool_name) {
        return true;
    }

    // User previously chose "always" for this tool
    if session_approved.contains(tool_name) {
        return true;
    }

    false
}

/// Prompt the user for permission to execute a tool.
///
/// Returns the user's choice: `'y'` (once), `'n'` (deny), or `'a'` (always).
/// On EOF or invalid input, defaults to `'n'`.
#[allow(clippy::print_stderr)]
pub fn prompt_tool_permission(tool_name: &str) -> char {
    use std::io::Write;

    eprint!("[y/n/a] Allow {tool_name}? ");
    std::io::stderr().flush().ok();

    let mut response = String::new();
    match std::io::stdin().read_line(&mut response) {
        Ok(0) => 'n', // EOF
        Ok(_) => match response.trim().to_lowercase().as_str() {
            "y" | "yes" => 'y',
            "a" | "always" => 'a',
            _ => 'n',
        },
        Err(_) => 'n',
    }
}

/// Trait implemented by each slash command.
pub trait Command: Send + Sync {
    /// Primary name (without the leading `/`).
    fn name(&self) -> &str;

    /// Alternative names that also dispatch to this command.
    fn aliases(&self) -> &[&str];

    /// One-line description shown in `/help`.
    fn description(&self) -> &str;

    /// Execute the command with the given arguments and mutable context.
    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult;
}

/// Registry of slash commands with dispatch by name or alias.
pub struct CommandRegistry {
    /// Canonical name -> command implementation.
    commands: BTreeMap<String, Box<dyn Command>>,
    /// Alias -> canonical name.
    aliases: BTreeMap<String, String>,
    /// Cached help text.
    help_text: String,
}

impl CommandRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
            aliases: BTreeMap::new(),
            help_text: String::new(),
        }
    }

    /// Create a registry with all built-in commands pre-registered.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(help::HelpCommand));
        registry.register(Box::new(clear::ClearCommand));
        registry.register(Box::new(compact::CompactCommand));
        registry.register(Box::new(cost::CostCommand));
        registry.register(Box::new(quit::QuitCommand));
        registry.register(Box::new(diff::DiffCommand));
        registry.register(Box::new(memory::MemoryCommand));
        registry.register(Box::new(status::StatusCommand));
        registry.register(Box::new(model::ModelCommand));
        registry.register(Box::new(commit::CommitCommand));
        registry.register(Box::new(config_cmd::ConfigCommand));
        registry.register(Box::new(undo::UndoCommand));
        registry.register(Box::new(history::HistoryCommand));
        registry.register(Box::new(skill::SkillCommand));
        registry.rebuild_help_text();
        registry
    }

    /// Check if a name (or alias) is registered as a built-in command.
    pub fn has_command(&self, name: &str) -> bool {
        let name = name.strip_prefix('/').unwrap_or(name);
        self.commands.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Register a command. Overwrites any existing command with the same name.
    pub fn register(&mut self, cmd: Box<dyn Command>) {
        let name = cmd.name().to_string();
        for alias in cmd.aliases() {
            self.aliases.insert((*alias).to_string(), name.clone());
        }
        self.commands.insert(name, cmd);
        self.rebuild_help_text();
    }

    /// Dispatch a `/`-prefixed input string. Returns `None` if the name is unknown.
    pub fn execute(&self, input: &str, ctx: &mut CommandContext) -> Option<CommandResult> {
        let input = input.strip_prefix('/').unwrap_or(input);
        let (name, args) = match input.split_once(char::is_whitespace) {
            Some((n, a)) => (n, a.trim()),
            None => (input.trim(), ""),
        };

        // Inject help text so /help can display it.
        ctx.help_text.clone_from(&self.help_text);

        let canonical = self.aliases.get(name).map(String::as_str).unwrap_or(name);

        self.commands
            .get(canonical)
            .map(|cmd| cmd.execute(args, ctx))
    }

    /// Get the rendered help text for all registered commands.
    pub fn help_text(&self) -> &str {
        &self.help_text
    }

    fn rebuild_help_text(&mut self) {
        let mut lines = vec!["Available commands:".to_string()];
        for cmd in self.commands.values() {
            let aliases = cmd.aliases();
            let alias_str = if aliases.is_empty() {
                String::new()
            } else {
                let formatted: Vec<String> = aliases.iter().map(|a| format!("/{a}")).collect();
                format!(" ({})", formatted.join(", "))
            };
            lines.push(format!(
                "  /{}{} — {}",
                cmd.name(),
                alias_str,
                cmd.description()
            ));
        }
        self.help_text = lines.join("\n");
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_dispatches_by_name() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();
        let result = registry.execute("/help", &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), CommandResult::Output(_)));
    }

    #[test]
    fn registry_dispatches_by_alias() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();

        // /q is an alias for /quit
        let result = registry.execute("/q", &mut ctx);
        assert!(matches!(result.unwrap(), CommandResult::Quit));

        // /exit is an alias for /quit
        let result = registry.execute("/exit", &mut ctx);
        assert!(matches!(result.unwrap(), CommandResult::Quit));
    }

    #[test]
    fn registry_returns_none_for_unknown() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();
        assert!(registry.execute("/nonexistent", &mut ctx).is_none());
    }

    #[test]
    fn help_text_lists_all_commands() {
        let registry = CommandRegistry::with_builtins();
        let text = registry.help_text();
        assert!(text.contains("/help"));
        assert!(text.contains("/clear"));
        assert!(text.contains("/compact"));
        assert!(text.contains("/cost"));
        assert!(text.contains("/quit"));
        assert!(text.contains("/diff"));
        assert!(text.contains("/status"));
        assert!(text.contains("/model"));
        assert!(text.contains("/commit"));
        assert!(text.contains("/config"));
        assert!(text.contains("/undo"));
        assert!(text.contains("/history"));
        assert!(text.contains("/skill"));
    }

    #[test]
    fn new_command_aliases_dispatch() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();

        // /info -> /status
        let result = registry.execute("/info", &mut ctx);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), CommandResult::Output(_)));

        // /git-status -> /commit
        let result = registry.execute("/git-status", &mut ctx);
        assert!(result.is_some());

        // /settings -> /config
        let result = registry.execute("/settings", &mut ctx);
        assert!(result.is_some());

        // /messages -> /history
        let result = registry.execute("/messages", &mut ctx);
        assert!(result.is_some());

        // /skills -> /skill
        let result = registry.execute("/skills", &mut ctx);
        assert!(result.is_some());
    }

    #[test]
    fn has_command_checks_names_and_aliases() {
        let registry = CommandRegistry::with_builtins();
        assert!(registry.has_command("help"));
        assert!(registry.has_command("/help"));
        assert!(registry.has_command("status"));
        assert!(registry.has_command("/info"));
        assert!(registry.has_command("skill"));
        assert!(registry.has_command("/skills"));
        assert!(!registry.has_command("nonexistent"));
    }

    #[test]
    fn command_context_new_fields_default() {
        let ctx = CommandContext::default();
        assert!(ctx.provider_name.is_empty());
        assert!(ctx.model_name.is_empty());
        assert!(ctx.model_override.is_none());
        assert_eq!(ctx.message_count, 0);
        assert_eq!(ctx.tool_call_count, 0);
        assert_eq!(ctx.tools_count, 0);
        assert_eq!(ctx.hooks_count, 0);
        assert!(ctx.skill_names.is_empty());
    }

    #[test]
    fn cost_alias_usage() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext {
            session_turns: 3,
            session_input_tokens: 100,
            session_output_tokens: 50,
            ..Default::default()
        };
        let result = registry.execute("/usage", &mut ctx);
        assert!(result.is_some());
        match result.unwrap() {
            CommandResult::Output(text) => {
                assert!(text.contains("Turns:  3"));
                assert!(text.contains("Tokens: 150"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn slash_prefix_is_optional() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();
        // Without leading /
        let result = registry.execute("help", &mut ctx);
        assert!(matches!(result.unwrap(), CommandResult::Output(_)));
    }

    #[test]
    fn args_are_passed_through() {
        let registry = CommandRegistry::with_builtins();
        let mut ctx = CommandContext::default();
        // /help with trailing args — should still work
        let result = registry.execute("/help some args", &mut ctx);
        assert!(matches!(result.unwrap(), CommandResult::Output(_)));
    }

    // ── Permission logic tests ──

    #[test]
    fn read_only_tools_auto_approved() {
        let empty = HashSet::new();
        assert!(is_tool_auto_approved(
            "glob",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "grep",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "file_read",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "list_dir",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "read_file",
            PermissionMode::Default,
            &empty,
            false
        ));
    }

    #[test]
    fn read_only_annotation_auto_approved() {
        let empty = HashSet::new();
        // Even an unknown tool with read_only annotation should be auto-approved
        assert!(is_tool_auto_approved(
            "custom_reader",
            PermissionMode::Default,
            &empty,
            true
        ));
    }

    #[test]
    fn yes_mode_auto_approves_all() {
        let empty = HashSet::new();
        assert!(is_tool_auto_approved(
            "bash",
            PermissionMode::Yes,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "write_file",
            PermissionMode::Yes,
            &empty,
            false
        ));
        assert!(is_tool_auto_approved(
            "edit_file",
            PermissionMode::Yes,
            &empty,
            false
        ));
    }

    #[test]
    fn session_memory_works_after_always() {
        let mut approved = HashSet::new();
        // bash is not auto-approved by default
        assert!(!is_tool_auto_approved(
            "bash",
            PermissionMode::Default,
            &approved,
            false
        ));

        // After adding to session_approved, it should be auto-approved
        approved.insert("bash".to_string());
        assert!(is_tool_auto_approved(
            "bash",
            PermissionMode::Default,
            &approved,
            false
        ));
    }

    #[test]
    fn non_read_only_tools_require_permission() {
        let empty = HashSet::new();
        assert!(!is_tool_auto_approved(
            "bash",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(!is_tool_auto_approved(
            "write_file",
            PermissionMode::Default,
            &empty,
            false
        ));
        assert!(!is_tool_auto_approved(
            "edit_file",
            PermissionMode::Default,
            &empty,
            false
        ));
    }

    #[test]
    fn plan_mode_still_requires_permission_for_writes() {
        let empty = HashSet::new();
        // Plan mode does NOT auto-approve write tools
        assert!(!is_tool_auto_approved(
            "bash",
            PermissionMode::Plan,
            &empty,
            false
        ));
        // But read-only tools are still auto-approved
        assert!(is_tool_auto_approved(
            "glob",
            PermissionMode::Plan,
            &empty,
            false
        ));
    }

    #[test]
    fn permission_mode_default_trait() {
        assert_eq!(PermissionMode::default(), PermissionMode::Default);
    }

    #[test]
    fn command_context_default_has_empty_approved_tools() {
        let ctx = CommandContext::default();
        assert!(ctx.session_approved_tools.is_empty());
        assert_eq!(ctx.permission_mode, PermissionMode::Default);
    }
}
