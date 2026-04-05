//! `/history` slash command — show conversation message stats.

use crate::{Command, CommandContext, CommandResult};

pub struct HistoryCommand;

impl Command for HistoryCommand {
    fn name(&self) -> &str {
        "history"
    }

    fn aliases(&self) -> &[&str] {
        &["messages"]
    }

    fn description(&self) -> &str {
        "Show message count, turns, tool calls, and token usage"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let total_tokens = ctx.session_input_tokens + ctx.session_output_tokens;
        let output = format!(
            "Conversation history:\n\
             \n  Messages:   {}\
             \n  Turns:      {}\
             \n  Tool calls: {}\
             \n  Tokens:     {} (in: {}, out: {})",
            ctx.message_count,
            ctx.session_turns,
            ctx.tool_call_count,
            total_tokens,
            ctx.session_input_tokens,
            ctx.session_output_tokens,
        );
        CommandResult::Output(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_shows_counts() {
        let cmd = HistoryCommand;
        let mut ctx = CommandContext {
            message_count: 12,
            session_turns: 4,
            tool_call_count: 7,
            session_input_tokens: 2000,
            session_output_tokens: 1000,
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Messages:   12"));
                assert!(text.contains("Turns:      4"));
                assert!(text.contains("Tool calls: 7"));
                assert!(text.contains("Tokens:     3000"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
