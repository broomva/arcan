//! `/skill` command — list and activate skills discovered at startup.

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
        "List discovered skills or show skill details"
    }

    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let args = args.trim();

        if args.is_empty() || args == "list" {
            // List all discovered skills
            if ctx.skill_names.is_empty() {
                return CommandResult::Output(
                    "No skills discovered.\n\
                     Place SKILL.md files in .claude/skills/, .agents/skills/, or ~/.claude/skills/."
                        .to_string(),
                );
            }

            let mut lines = vec![format!("Discovered skills ({}):", ctx.skill_names.len())];
            for name in &ctx.skill_names {
                lines.push(format!("  /{name}"));
            }
            lines.push(String::new());
            lines.push("Activate a skill by typing /<skill-name> as your message.".to_string());
            CommandResult::Output(lines.join("\n"))
        } else {
            // Show info for a specific skill name
            let query = args.strip_prefix('/').unwrap_or(args);
            if ctx.skill_names.iter().any(|n| n == query) {
                CommandResult::Output(format!(
                    "Skill '{query}' is available. Type /{query} to activate it."
                ))
            } else {
                CommandResult::Error(format!(
                    "Unknown skill '{query}'. Type /skill to list available skills."
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_list_empty() {
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
    fn skill_list_with_skills() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["commit-helper".to_string(), "test-runner".to_string()],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Discovered skills (2)"));
                assert!(text.contains("/commit-helper"));
                assert!(text.contains("/test-runner"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_list_subcommand() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["alpha".to_string()],
            ..Default::default()
        };
        match cmd.execute("list", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("/alpha"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_query_found() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["my-skill".to_string()],
            ..Default::default()
        };
        match cmd.execute("my-skill", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("available"));
                assert!(text.contains("/my-skill"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_query_with_slash_prefix() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["my-skill".to_string()],
            ..Default::default()
        };
        match cmd.execute("/my-skill", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("available"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn skill_query_not_found() {
        let cmd = SkillCommand;
        let mut ctx = CommandContext {
            skill_names: vec!["alpha".to_string()],
            ..Default::default()
        };
        match cmd.execute("nonexistent", &mut ctx) {
            CommandResult::Error(text) => {
                assert!(text.contains("Unknown skill"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
