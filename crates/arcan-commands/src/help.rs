use crate::{Command, CommandContext, CommandResult};

pub struct HelpCommand;

impl Command for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }

    fn aliases(&self) -> &[&str] {
        &["h", "?"]
    }

    fn description(&self) -> &str {
        "Show available slash commands"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        CommandResult::Output(ctx.help_text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_returns_help_text() {
        let cmd = HelpCommand;
        let mut ctx = CommandContext::default();
        ctx.help_text = "test help".to_string();
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => assert_eq!(text, "test help"),
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
