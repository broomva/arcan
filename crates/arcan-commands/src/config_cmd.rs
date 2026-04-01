//! `/config` slash command — show current configuration.

use crate::{Command, CommandContext, CommandResult, PermissionMode};

pub struct ConfigCommand;

impl Command for ConfigCommand {
    fn name(&self) -> &str {
        "config"
    }

    fn aliases(&self) -> &[&str] {
        &["settings"]
    }

    fn description(&self) -> &str {
        "Show current configuration"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let mode_str = match ctx.permission_mode {
            PermissionMode::Default => "default (prompt)",
            PermissionMode::Yes => "yes (auto-approve)",
            PermissionMode::Plan => "plan (read-only)",
        };

        let output = format!(
            "Configuration:\n\
             \n  Provider:    {}\
             \n  Model:       {}\
             \n  Workspace:   {}\
             \n  Data dir:    {}\
             \n  Memory dir:  {}\
             \n  Permissions: {}",
            ctx.provider_name,
            ctx.model_name,
            ctx.workspace.display(),
            ctx.data_dir.display(),
            ctx.memory_dir.display(),
            mode_str,
        );
        CommandResult::Output(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_shows_provider_and_workspace() {
        let cmd = ConfigCommand;
        let mut ctx = CommandContext {
            provider_name: "anthropic".to_string(),
            model_name: "claude-sonnet-4-20250514".to_string(),
            workspace: PathBuf::from("/home/user/project"),
            data_dir: PathBuf::from("/home/user/project/.arcan"),
            memory_dir: PathBuf::from("/home/user/project/.arcan/memory"),
            permission_mode: PermissionMode::Default,
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("anthropic"));
                assert!(text.contains("claude-sonnet-4-20250514"));
                assert!(text.contains("/home/user/project"));
                assert!(text.contains("default (prompt)"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn config_shows_yes_mode() {
        let cmd = ConfigCommand;
        let mut ctx = CommandContext {
            permission_mode: PermissionMode::Yes,
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("yes (auto-approve)"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
