use crate::{Command, CommandContext, CommandResult};

pub struct DiffCommand;

impl Command for DiffCommand {
    fn name(&self) -> &str {
        "diff"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Show uncommitted changes in the workspace (git diff)"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let workspace = &ctx.workspace;
        let output = std::process::Command::new("git")
            .args(["diff", "--stat"])
            .current_dir(workspace)
            .output();

        match output {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);
                if result.status.success() {
                    if stdout.trim().is_empty() {
                        CommandResult::Output("No uncommitted changes.".to_string())
                    } else {
                        CommandResult::Output(stdout.into_owned())
                    }
                } else {
                    CommandResult::Error(format!("git diff failed: {stderr}"))
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
    fn diff_runs_in_temp_dir() {
        let cmd = DiffCommand;
        let tmp = std::env::temp_dir();
        let mut ctx = CommandContext {
            workspace: tmp,
            ..Default::default()
        };
        // In a non-git directory, this should return an error (not panic).
        let result = cmd.execute("", &mut ctx);
        // Either an error or output is fine — just verify it doesn't panic.
        assert!(matches!(
            result,
            CommandResult::Output(_) | CommandResult::Error(_)
        ));
    }
}
