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

        // Nous safety scores line
        let safety_line = if ctx.nous_scores.is_empty() {
            "  Safety:   (no evaluations yet)".to_string()
        } else {
            let scores_str: Vec<String> = ctx
                .nous_scores
                .iter()
                .map(|(name, val)| format!("{name}: {val:.2}"))
                .collect();
            format!("  Safety:   {}", scores_str.join(", "))
        };

        // Economic / budget line
        let economic_line = match (&ctx.economic_mode, ctx.budget_usd) {
            (Some(mode), Some(budget)) => {
                format!(
                    "  Economic: {mode} (${:.4} / ${:.2} budget)",
                    ctx.session_cost_usd, budget
                )
            }
            (Some(mode), None) => {
                format!("  Economic: {mode} (${:.4} spent)", ctx.session_cost_usd)
            }
            (None, Some(budget)) => {
                format!(
                    "  Economic: ${:.4} / ${:.2} budget",
                    ctx.session_cost_usd, budget
                )
            }
            (None, None) => {
                format!(
                    "  Economic: ${:.4} spent (no budget set)",
                    ctx.session_cost_usd
                )
            }
        };

        // Workspace journal line
        let workspace_line = match &ctx.workspace_journal_status {
            Some(status) => format!("  Workspace: {status}"),
            None => "  Workspace: (not configured)".to_string(),
        };

        let output = format!(
            "Session status:\n\
             \n  Provider: {}\
             \n  Model:    {}\
             \n  Tools:    {}\
             \n  Hooks:    {}\
             \n  Skills:   {}\
             \n  Turns:    {}\
             \n  Tokens:   {} (in: {}, out: {})\
             \n  Cost:     ${:.4}\
             \n{safety_line}\
             \n{economic_line}\
             \n{workspace_line}",
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
                assert!(text.contains("Safety:"));
                assert!(text.contains("Economic:"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn status_shows_nous_scores() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![
                ("safety_compliance".to_string(), 0.95),
                ("tool_correctness".to_string(), 1.0),
            ],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("safety_compliance: 0.95"));
                assert!(text.contains("tool_correctness: 1.00"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn status_shows_economic_mode_with_budget() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            session_cost_usd: 0.0741,
            budget_usd: Some(5.0),
            economic_mode: Some("Sovereign".to_string()),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Sovereign"));
                assert!(text.contains("$0.0741"));
                assert!(text.contains("$5.00 budget"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn status_shows_budget_without_autonomic() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            session_cost_usd: 1.25,
            budget_usd: Some(10.0),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("$1.25"));
                assert!(text.contains("$10.00 budget"));
                assert!(!text.contains("Sovereign"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
