//! `/commit` slash command — show git status and staged changes.

use crate::{Command, CommandContext, CommandResult};

pub struct CommitCommand;

impl Command for CommitCommand {
    fn name(&self) -> &str {
        "commit"
    }

    fn aliases(&self) -> &[&str] {
        &["git-status"]
    }

    fn description(&self) -> &str {
        "Show git status and staged changes"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let workspace = &ctx.workspace;

        let status_output = std::process::Command::new("git")
            .args(["status", "--short"])
            .current_dir(workspace)
            .output();

        let diff_output = std::process::Command::new("git")
            .args(["diff", "--cached", "--stat"])
            .current_dir(workspace)
            .output();

        let mut result = String::new();

        match status_output {
            Ok(ref out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if stdout.trim().is_empty() {
                    result.push_str("Working tree clean.\n");
                } else {
                    result.push_str("Changes:\n");
                    result.push_str(&stdout);
                }
            }
            Ok(ref out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return CommandResult::Error(format!("git status failed: {stderr}"));
            }
            Err(e) => return CommandResult::Error(format!("failed to run git: {e}")),
        }

        match diff_output {
            Ok(ref out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                if !stdout.trim().is_empty() {
                    result.push_str("\nStaged:\n");
                    result.push_str(&stdout);
                }
            }
            Ok(_) | Err(_) => {
                // Staged diff is optional — don't fail the command.
            }
        }

        CommandResult::Output(result.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_runs_without_panic() {
        let cmd = CommitCommand;
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
