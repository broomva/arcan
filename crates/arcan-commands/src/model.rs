//! `/model` slash command — show or switch the current model.

use crate::{Command, CommandContext, CommandResult};

pub struct ModelCommand;

impl Command for ModelCommand {
    fn name(&self) -> &str {
        "model"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Show current model or switch to a new one"
    }

    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let requested = args.trim();
        if requested.is_empty() {
            let override_info = match &ctx.model_override {
                Some(m) => format!(" (override pending: {m})"),
                None => String::new(),
            };
            return CommandResult::Output(format!(
                "Current model: {}{}",
                ctx.model_name, override_info,
            ));
        }

        ctx.model_override = Some(requested.to_string());
        CommandResult::Output(format!(
            "Model override set to \"{requested}\". Will take effect on the next provider call."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_shows_current_when_no_args() {
        let cmd = ModelCommand;
        let mut ctx = CommandContext {
            model_name: "claude-sonnet-4-20250514".to_string(),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("claude-sonnet-4-20250514"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn model_sets_override() {
        let cmd = ModelCommand;
        let mut ctx = CommandContext {
            model_name: "claude-sonnet-4-20250514".to_string(),
            ..Default::default()
        };
        match cmd.execute("gpt-4o", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("gpt-4o"));
                assert!(text.contains("override"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
        assert_eq!(ctx.model_override.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn model_shows_pending_override() {
        let cmd = ModelCommand;
        let mut ctx = CommandContext {
            model_name: "claude-sonnet-4-20250514".to_string(),
            model_override: Some("gpt-4o".to_string()),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("claude-sonnet-4-20250514"));
                assert!(text.contains("override pending: gpt-4o"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
