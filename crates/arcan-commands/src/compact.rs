use crate::{Command, CommandContext, CommandResult};

pub struct CompactCommand;

impl Command for CompactCommand {
    fn name(&self) -> &str {
        "compact"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Compact conversation history to reduce token usage"
    }

    fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::CompactRequested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_returns_compact_requested() {
        let cmd = CompactCommand;
        let mut ctx = CommandContext::default();
        assert!(matches!(
            cmd.execute("", &mut ctx),
            CommandResult::CompactRequested
        ));
    }
}
