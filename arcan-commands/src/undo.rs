//! `/undo` slash command — show the most recent commit's diff (informational).

use crate::{Command, CommandContext, CommandResult};

pub struct UndoCommand;

impl Command for UndoCommand {
    fn name(&self) -> &str {
        "undo"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Show the last commit's changes (informational)"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let workspace = &ctx.workspace;

        let output = std::process::Command::new("git")
            .args(["diff", "HEAD~1", "--stat"])
            .current_dir(workspace)
            .output();

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);
                if result.status.success() {
                    if stdout.trim().is_empty() {
                        CommandResult::Output("No changes in the last commit.".to_string())
                    } else {
                        CommandResult::Output(format!(
                            "Last commit changes:\n{}",
                            stdout.trim_end()
                        ))
                    }
                } else {
                    CommandResult::Error(format!("git diff HEAD~1 failed: {stderr}"))
                }
            }
            Err(e) => CommandResult::Error(format!("failed to run git: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_runs_without_panic() {
        let cmd = UndoCommand;
        let mut ctx = CommandContext {
            workspace: std::env::temp_dir(),
            ..Default::default()
        };
        let result = cmd.execute("", &mut ctx);
        assert!(matches!(
            result,
            CommandResult::Output(_) | CommandResult::Error(_)
        ));
    }
}
