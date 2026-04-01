use crate::{Command, CommandContext, CommandResult};

pub struct CostCommand;

impl Command for CostCommand {
    fn name(&self) -> &str {
        "cost"
    }

    fn aliases(&self) -> &[&str] {
        &["usage", "tokens"]
    }

    fn description(&self) -> &str {
        "Show session cost and token usage"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let output = format!(
            "Session usage:\n  Turns:  {}\n  Tokens: {} (input: {}, output: {})\n  Cost:   ${:.4}",
            ctx.session_turns,
            ctx.session_input_tokens + ctx.session_output_tokens,
            ctx.session_input_tokens,
            ctx.session_output_tokens,
            ctx.session_cost_usd,
        );
        CommandResult::Output(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_shows_usage() {
        let cmd = CostCommand;
        let mut ctx = CommandContext::default();
        ctx.session_turns = 5;
        ctx.session_input_tokens = 1000;
        ctx.session_output_tokens = 500;
        ctx.session_cost_usd = 0.0123;
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Turns:  5"));
                assert!(text.contains("Tokens: 1500"));
                assert!(text.contains("$0.0123"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
