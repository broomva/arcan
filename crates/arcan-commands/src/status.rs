//! `/status` slash command — show session and runtime information.

use crate::{Command, CommandContext, CommandResult};

pub struct StatusCommand;

impl Command for StatusCommand {
    fn name(&self) -> &str {
        "status"
    }

    fn aliases(&self) -> &[&str] {
        &["info"]
    }

    fn description(&self) -> &str {
        "Show provider, model, tools, hooks, session stats"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let total_tokens = ctx.session_input_tokens + ctx.session_output_tokens;
        let output = format!(
            "Session status:\n\
             \n  Provider: {}\
             \n  Model:    {}\
             \n  Tools:    {}\
             \n  Hooks:    {}\
             \n  Skills:   {}\
             \n  Turns:    {}\
             \n  Tokens:   {} (in: {}, out: {})\
             \n  Cost:     ${:.4}",
            ctx.provider_name,
            ctx.model_name,
            ctx.tools_count,
            ctx.hooks_count,
            ctx.skill_names.len(),
            ctx.session_turns,
            total_tokens,
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
    fn status_shows_provider_and_model() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            provider_name: "anthropic".to_string(),
            model_name: "claude-sonnet-4-20250514".to_string(),
            tools_count: 10,
            hooks_count: 3,
            skill_names: vec!["alpha".to_string(), "beta".to_string()],
            session_turns: 5,
            session_input_tokens: 1000,
            session_output_tokens: 500,
            session_cost_usd: 0.05,
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("anthropic"));
                assert!(text.contains("claude-sonnet-4-20250514"));
                assert!(text.contains("Tools:    10"));
                assert!(text.contains("Hooks:    3"));
                assert!(text.contains("Skills:   2"));
                assert!(text.contains("Turns:    5"));
                assert!(text.contains("1500"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
