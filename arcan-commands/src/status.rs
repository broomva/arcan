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

        // Nous evaluation scores grouped by layer
        let eval_line = if ctx.nous_scores.is_empty() {
            "  Eval:     (no evaluations yet)".to_string()
        } else {
            let mut by_layer: std::collections::BTreeMap<&str, Vec<String>> =
                std::collections::BTreeMap::new();
            for s in &ctx.nous_scores {
                let entry = by_layer.entry(s.layer.as_str()).or_default();
                let indicator = match s.label.as_str() {
                    "good" => "✓",
                    "warning" => "⚠",
                    "critical" => "✗",
                    _ => "·",
                };
                entry.push(format!("{}{} {:.2}", indicator, s.name, s.value));
            }
            let mut lines = vec!["  Eval:".to_string()];
            for (layer, scores) in &by_layer {
                lines.push(format!("    {layer}: {}", scores.join(", ")));
            }
            lines.join("\n")
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

        // Identity line (BRO-370)
        let identity_line = match (&ctx.identity_tier, &ctx.identity_subject) {
            (Some(tier), Some(subject)) => format!("  Identity: {tier} ({subject})"),
            (Some(tier), None) => format!("  Identity: {tier}"),
            _ => "  Identity: anonymous local agent".to_string(),
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
             \n{eval_line}\
             \n{economic_line}\
             \n{identity_line}\
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
    use crate::NousScoreDetail;

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
                assert!(text.contains("Eval:"));
                assert!(text.contains("Economic:"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn status_shows_nous_scores_grouped_by_layer() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![
                NousScoreDetail {
                    name: "safety_compliance".into(),
                    value: 0.95,
                    layer: "safety".into(),
                    label: "good".into(),
                },
                NousScoreDetail {
                    name: "tool_correctness".into(),
                    value: 1.0,
                    layer: "action".into(),
                    label: "good".into(),
                },
                NousScoreDetail {
                    name: "token_efficiency".into(),
                    value: 0.45,
                    layer: "execution".into(),
                    label: "critical".into(),
                },
            ],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("safety_compliance"));
                assert!(text.contains("0.95"));
                assert!(text.contains("tool_correctness"));
                assert!(text.contains("safety:"));
                assert!(text.contains("action:"));
                assert!(text.contains("execution:"));
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

    #[test]
    fn status_shows_identity_with_subject() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext {
            identity_tier: Some("pro".to_string()),
            identity_subject: Some("user@example.com".to_string()),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Identity: pro (user@example.com)"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn status_shows_anonymous_identity() {
        let cmd = StatusCommand;
        let mut ctx = CommandContext::default();
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Identity: anonymous local agent"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
