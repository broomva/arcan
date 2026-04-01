//! Slash command system for the Arcan agent runtime.
//!
//! Provides a [`CommandRegistry`] that dispatches `/`-prefixed user input to
//! built-in commands (`/help`, `/clear`, `/cost`, `/quit`, `/diff`).

mod clear;
mod cost;
mod diff;
mod help;
mod quit;

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Result of executing a slash command.
#[derive(Debug)]
pub enum CommandResult {
    /// Text output to display to the user.
    Output(String),
    /// Clear the conversation history and start a new session.
    ClearSession,
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
        registry.register(Box::new(cost::CostCommand));
        registry.register(Box::new(quit::QuitCommand));
        registry.register(Box::new(diff::DiffCommand));
        registry.rebuild_help_text();
        registry
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
        assert!(text.contains("/cost"));
        assert!(text.contains("/quit"));
        assert!(text.contains("/diff"));
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
}
