use crate::{Command, CommandContext, CommandResult};

pub struct ConsolidateCommand;

impl Command for ConsolidateCommand {
    fn name(&self) -> &str {
        "consolidate"
    }

    fn aliases(&self) -> &[&str] {
        &["gc"]
    }

    fn description(&self) -> &str {
        "Run memory consolidation (decay, pattern extraction, pruning)"
    }

    fn execute(&self, _args: &str, _ctx: &mut CommandContext) -> CommandResult {
        CommandResult::ConsolidateRequested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consolidate_returns_consolidate_requested() {
        let cmd = ConsolidateCommand;
        let mut ctx = CommandContext::default();
        assert!(matches!(
            cmd.execute("", &mut ctx),
            CommandResult::ConsolidateRequested
        ));
    }

    #[test]
    fn consolidate_command_registered() {
        let registry = crate::CommandRegistry::with_builtins();
        assert!(
            registry.has_command("consolidate"),
            "/consolidate should be registered"
        );
        assert!(registry.has_command("gc"), "/gc alias should be registered");
    }
}
