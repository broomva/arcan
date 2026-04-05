//! `/skill` slash command — list discovered skills or query by name.

use crate::{Command, CommandContext, CommandResult};

pub struct SkillCommand;

impl Command for SkillCommand {
    fn name(&self) -> &str {
        "skill"
    }

    fn aliases(&self) -> &[&str] {
        &["skills"]
    }

    fn description(&self) -> &str {
        "List discovered skills or query by name"
    }

    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let query = args.trim();

        if ctx.skill_names.is_empty() {
            return CommandResult::Output("No skills discovered.".to_string());
        }

        if query.is_empty() {
            let mut output = format!("Discovered skills ({}):\n", ctx.skill_names.len());
            for name in &ctx.skill_names {
                output.push_str(&format!("  /{name}\n"));
            }
            return CommandResult::Output(output.trim_end().to_string());
        }

        // Search for a skill matching the query
        let matches: Vec<&String> = ctx
            .skill_names
            .iter()
            .filter(|n| n.contains(query))
            .collect();

        if matches.is_empty() {
            CommandResult::Output(format!("No skill matching \"{query}\" found."))
        } else {
            let mut output = format!("Skills matching \"{query}\" ({}):\n", matches.len());
            for name in matches {
                output.push_str(&format!("  /{name}\n"));
            }
            CommandResult::Output(output.trim_end().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_lists_all_when_no_args() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec![
                "commit-helper".to_string(),
                "test-runner".to_string(),
                "deploy".to_string(),
            ],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Discovered skills (3)"));
                assert!(text.contains("/commit-helper"));
                assert!(text.contains("/test-runner"));
                assert!(text.contains("/deploy"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_filters_by_query() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec![
                "commit-helper".to_string(),
                "test-runner".to_string(),
                "deploy".to_string(),
            ],
            ..Default::default()
        };
        match cmd.execute("commit", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("matching \"commit\" (1)"));
                assert!(text.contains("/commit-helper"));
                assert!(!text.contains("/test-runner"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_empty_registry() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext::default();
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("No skills discovered"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_no_match() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["deploy".to_string()],
            ..Default::default()
        };
        match cmd.execute("nonexistent", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("No skill matching"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
