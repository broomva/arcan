use crate::{Command, CommandContext, CommandResult};

pub struct ClearCommand;

impl Command for ClearCommand {
    fn name(&self) -> &str {
        "clear"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Clear conversation history and start fresh"
    }

    fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::ClearSession
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_returns_clear_session() {
        let cmd = ClearCommand;
        let mut ctx = CommandContext::default();
        assert!(matches!(cmd.execute("", &mut ctx), CommandResult::ClearSession));
    }
}
