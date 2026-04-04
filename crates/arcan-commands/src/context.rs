use crate::{Command, CommandContext, CommandResult};

pub struct ContextCommand;

impl Command for ContextCommand {
    fn name(&self) -> &str {
        "context"
    }

    fn aliases(&self) -> &[&str] {
        &["ctx"]
    }

    fn description(&self) -> &str {
        "Show context window breakdown — what's consuming tokens"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let mut lines = Vec::new();
        lines.push("Context window breakdown:\n".to_string());

        let mut total = 0usize;

        // System prompt sections
        lines.push("  CACHEABLE (stable, cached after turn 1):".to_string());
        let role_tokens = estimate("You are an AI coding assistant powered by Arcan...");
        lines.push(format!(
            "    Role definition:        ~{:>6} tokens",
            role_tokens
        ));
        total += role_tokens;

        let env_tokens = 50; // platform, shell, date, model
        lines.push(format!(
            "    Environment:            ~{:>6} tokens",
            env_tokens
        ));
        total += env_tokens;

        let instructions_tokens = ctx.project_instructions_tokens;
        lines.push(format!(
            "    Project instructions:   ~{:>6} tokens  (CLAUDE.md, AGENTS.md, rules, docs)",
            instructions_tokens
        ));
        total += instructions_tokens;

        let guidelines_tokens = estimate(
            "Read files before editing. Use tools to explore. Be concise. Follow conventions.",
        );
        lines.push(format!(
            "    Guidelines:             ~{:>6} tokens",
            guidelines_tokens
        ));
        total += guidelines_tokens;

        let cacheable_total = role_tokens + env_tokens + instructions_tokens + guidelines_tokens;
        lines.push(format!(
            "    Cacheable subtotal:     ~{:>6} tokens  (75% cheaper after turn 1)",
            cacheable_total
        ));

        lines.push(String::new());
        lines.push("  DYNAMIC (changes per turn, always full price):".to_string());

        let git_tokens = ctx.git_context_tokens;
        lines.push(format!(
            "    Git context:            ~{:>6} tokens  (branch, status, log)",
            git_tokens
        ));
        total += git_tokens;

        let memory_tokens = ctx.memory_index_tokens;
        lines.push(format!(
            "    Memory index:           ~{:>6} tokens  (MEMORY.md)",
            memory_tokens
        ));
        total += memory_tokens;

        let workspace_tokens = ctx.workspace_context_tokens;
        lines.push(format!(
            "    Workspace context:      ~{:>6} tokens  (shared journal summaries)",
            workspace_tokens
        ));
        total += workspace_tokens;

        let skills_tokens = ctx.skills_catalog_tokens;
        lines.push(format!(
            "    Skills catalog:         ~{:>6} tokens  ({} skills)",
            skills_tokens,
            ctx.skill_names.len()
        ));
        total += skills_tokens;

        let dynamic_total = git_tokens + memory_tokens + workspace_tokens + skills_tokens;
        lines.push(format!(
            "    Dynamic subtotal:       ~{:>6} tokens",
            dynamic_total
        ));

        lines.push(String::new());
        lines.push("  CONVERSATION:".to_string());

        let conv_tokens = ctx.session_input_tokens.saturating_sub(total as u64) as usize;
        lines.push(format!(
            "    Messages + tool results: ~{:>6} tokens  ({} messages, {} tool calls)",
            conv_tokens, ctx.message_count, ctx.tool_call_count
        ));
        total += conv_tokens;

        lines.push(String::new());
        lines.push(format!("  TOTAL:                    ~{:>6} tokens", total));

        let window = ctx.context_window.unwrap_or(200_000);
        lines.push(format!(
            "  Window:                    {:>7} tokens",
            format_tokens(window)
        ));
        lines.push(format!(
            "  Utilization:               {:.1}%",
            (total as f64 / window as f64) * 100.0
        ));

        // Autonomic context regulation ruling
        if let Some(ref ruling) = ctx.context_ruling {
            lines.push(format!("  Autonomic ruling:          {ruling}"));
        }

        if skills_tokens > 10_000 {
            lines.push(String::new());
            lines.push(format!(
                "  ⚠ Skills catalog is {} tokens ({} skills). Consider using fewer skills",
                skills_tokens,
                ctx.skill_names.len()
            ));
            lines.push(
                "    or implementing deferred skill loading to reduce context size.".to_string(),
            );
        }

        if instructions_tokens > 20_000 {
            lines.push(String::new());
            lines.push(format!(
                "  ⚠ Project instructions are {} tokens. Large CLAUDE.md or docs/ files.",
                instructions_tokens
            ));
        }

        CommandResult::Output(lines.join("\n"))
    }
}

fn estimate(text: &str) -> usize {
    text.len().div_ceil(4)
}

fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_shows_breakdown() {
        let cmd = ContextCommand;
        let mut ctx = CommandContext {
            project_instructions_tokens: 5000,
            git_context_tokens: 200,
            memory_index_tokens: 100,
            workspace_context_tokens: 300,
            skills_catalog_tokens: 80000,
            session_input_tokens: 100000,
            message_count: 10,
            tool_call_count: 5,
            skill_names: vec!["a".into(); 307],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("CACHEABLE"));
                assert!(text.contains("DYNAMIC"));
                assert!(text.contains("Workspace context"));
                assert!(text.contains("Skills catalog"));
                assert!(text.contains("⚠")); // should warn about 80K skills
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn context_no_warning_for_small_skills() {
        let cmd = ContextCommand;
        let mut ctx = CommandContext {
            skills_catalog_tokens: 500,
            skill_names: vec!["a".into(); 5],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(!text.contains("⚠"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
