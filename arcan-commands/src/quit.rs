use crate::{Command, CommandContext, CommandResult};

pub struct QuitCommand;

impl Command for QuitCommand {
    fn name(&self) -> &str {
        "quit"
    }

    fn aliases(&self) -> &[&str] {
        &["exit", "q"]
    }

    fn description(&self) -> &str {
        "Exit the shell"
    }

    fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::Quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_returns_quit() {
        let cmd = QuitCommand;
        let mut ctx = CommandContext::default();
        assert!(matches!(cmd.execute("", &mut ctx), CommandResult::Quit));
    }
}
