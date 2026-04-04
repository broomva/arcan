use crate::{Command, CommandContext, CommandResult};

pub struct ReasoningCommand;

impl Command for ReasoningCommand {
    fn name(&self) -> &str {
        "reasoning"
    }

    fn aliases(&self) -> &[&str] {
        &["thinking"]
    }

    fn description(&self) -> &str {
        "Toggle display of model reasoning/thinking tokens"
    }

    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        match args.trim() {
            "on" | "show" => ctx.show_reasoning = true,
            "off" | "hide" => ctx.show_reasoning = false,
            "" => ctx.show_reasoning = !ctx.show_reasoning,
            other => {
                return CommandResult::Error(format!(
                    "Unknown argument '{other}'. Usage: /reasoning [on|off]"
                ));
            }
        }
        let state = if ctx.show_reasoning { "on" } else { "off" };
        CommandResult::Output(format!("Reasoning display: {state}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_reasoning() {
        let cmd = ReasoningCommand;
        let mut ctx = CommandContext::default();
        assert!(!ctx.show_reasoning);

        let result = cmd.execute("", &mut ctx);
        assert!(ctx.show_reasoning);
        assert!(matches!(result, CommandResult::Output(ref s) if s.contains("on")));

        let result = cmd.execute("", &mut ctx);
        assert!(!ctx.show_reasoning);
        assert!(matches!(result, CommandResult::Output(ref s) if s.contains("off")));
    }

    #[test]
    fn explicit_on_off() {
        let cmd = ReasoningCommand;
        let mut ctx = CommandContext::default();

        cmd.execute("on", &mut ctx);
        assert!(ctx.show_reasoning);

        cmd.execute("off", &mut ctx);
        assert!(!ctx.show_reasoning);
    }
}
