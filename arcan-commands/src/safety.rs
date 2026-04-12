//! `/safety` slash command — show cumulative safety evaluation scores.

use crate::{Command, CommandContext, CommandResult};

pub struct SafetyCommand;

impl Command for SafetyCommand {
    fn name(&self) -> &str {
        "safety"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn description(&self) -> &str {
        "Show cumulative safety evaluation scores for this session"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let safety_scores: Vec<_> = ctx
            .nous_scores
            .iter()
            .filter(|s| s.layer == "safety")
            .collect();

        if safety_scores.is_empty() {
            return CommandResult::Output(
                "Safety: no safety evaluations recorded yet.\n\
                 Safety evaluators run after each tool call."
                    .to_string(),
            );
        }

        let mut lines = vec!["Safety evaluation scores:".to_string()];

        for s in &safety_scores {
            let indicator = match s.label.as_str() {
                "good" => "PASS",
                "warning" => "WARN",
                "critical" => "FAIL",
                _ => "----",
            };
            let explanation = if s.value < 0.5 {
                " — attention needed"
            } else if s.value < 0.8 {
                " — acceptable"
            } else {
                ""
            };
            lines.push(format!(
                "  [{indicator}] {}: {:.2}{explanation}",
                s.name, s.value
            ));
        }

        // Aggregate stats
        let count = safety_scores.len();
        let sum: f64 = safety_scores.iter().map(|s| s.value).sum();
        let avg = sum / count as f64;
        let min = safety_scores
            .iter()
            .map(|s| s.value)
            .fold(f64::INFINITY, f64::min);

        let overall_label = if min < 0.5 {
            "CRITICAL"
        } else if avg < 0.8 {
            "WARNING"
        } else {
            "GOOD"
        };

        lines.push(String::new());
        lines.push(format!(
            "  Overall: {overall_label} (avg: {avg:.2}, min: {min:.2}, evaluators: {count})"
        ));

        // Show all-layer summary if there are non-safety scores too
        let non_safety_count = ctx.nous_scores.len() - count;
        if non_safety_count > 0 {
            lines.push(format!(
                "  Other layers: {non_safety_count} evaluator(s) active (use /status for full view)"
            ));
        }

        CommandResult::Output(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NousScoreDetail;

    #[test]
    fn safety_no_scores() {
        let cmd = SafetyCommand;
        let mut ctx = CommandContext::default();
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("no safety evaluations"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn safety_shows_passing_scores() {
        let cmd = SafetyCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![NousScoreDetail {
                name: "safety_compliance".into(),
                value: 1.0,
                layer: "safety".into(),
                label: "good".into(),
            }],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("[PASS]"));
                assert!(text.contains("safety_compliance"));
                assert!(text.contains("1.00"));
                assert!(text.contains("GOOD"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn safety_shows_critical_scores() {
        let cmd = SafetyCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![NousScoreDetail {
                name: "safety_compliance".into(),
                value: 0.0,
                layer: "safety".into(),
                label: "critical".into(),
            }],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("[FAIL]"));
                assert!(text.contains("CRITICAL"));
                assert!(text.contains("attention needed"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn safety_filters_non_safety_layers() {
        let cmd = SafetyCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![
                NousScoreDetail {
                    name: "safety_compliance".into(),
                    value: 0.9,
                    layer: "safety".into(),
                    label: "good".into(),
                },
                NousScoreDetail {
                    name: "token_efficiency".into(),
                    value: 0.3,
                    layer: "execution".into(),
                    label: "critical".into(),
                },
            ],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("safety_compliance"));
                assert!(!text.contains("token_efficiency"));
                assert!(text.contains("1 evaluator(s) active"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn safety_aggregate_warning() {
        let cmd = SafetyCommand;
        let mut ctx = CommandContext {
            nous_scores: vec![
                NousScoreDetail {
                    name: "safety_a".into(),
                    value: 0.9,
                    layer: "safety".into(),
                    label: "good".into(),
                },
                NousScoreDetail {
                    name: "safety_b".into(),
                    value: 0.6,
                    layer: "safety".into(),
                    label: "warning".into(),
                },
            ],
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("WARNING"));
                assert!(text.contains("avg: 0.75"));
                assert!(text.contains("min: 0.60"));
                assert!(text.contains("evaluators: 2"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }
}
