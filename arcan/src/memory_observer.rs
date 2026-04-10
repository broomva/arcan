//! Memory extraction observer — extracts key facts from agent responses
//! and writes them to `.arcan/memory/session_summary.md`.
//!
//! This runs as a `ToolHarnessObserver` in the daemon's post-run pipeline,
//! moving memory extraction from the shell's dedicated writer thread into
//! the daemon's observer pattern.

use arcan_aios_adapters::tools::{RunCompletionContext, ToolHarnessObserver};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// Observer that extracts key facts from agent responses and writes them
/// to the memory directory.
pub struct MemoryExtractionObserver {
    memory_dir: PathBuf,
}

impl MemoryExtractionObserver {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }
}

#[async_trait]
impl ToolHarnessObserver for MemoryExtractionObserver {
    async fn post_execute(&self, _session_id: String, _tool_name: String, _is_error: bool) {
        // No per-tool action needed for memory extraction.
    }

    async fn on_run_finished(&self, session_id: String, context: RunCompletionContext) {
        let RunCompletionContext {
            objective,
            final_answer,
            assistant_messages,
            ..
        } = context;

        // Combine all available text for extraction.
        let text = match (&final_answer, &assistant_messages) {
            (Some(fa), Some(am)) => format!("{fa}\n{am}"),
            (Some(fa), None) => fa.clone(),
            (None, Some(am)) => am.clone(),
            (None, None) => return,
        };

        let facts = extract_facts(&text);
        if facts.is_empty() {
            return;
        }

        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
        let objective_line = objective
            .as_deref()
            .map(|o| format!("**Objective**: {o}\n\n"))
            .unwrap_or_default();
        let summary = format!(
            "# Session Summary\n\n**Updated**: {timestamp}\n**Session**: {session_id}\n{objective_line}{}\n",
            facts.join("\n")
        );

        if let Err(e) = write_memory(&self.memory_dir, &summary) {
            tracing::warn!(error = %e, "Failed to write memory extraction");
        } else {
            tracing::debug!(
                facts = facts.len(),
                session = %session_id,
                "Memory extraction completed"
            );
            // Update the memory index file.
            crate::prompt::write_memory_index(&self.memory_dir);
        }
    }
}

/// Write the summary to the memory directory.
fn write_memory(memory_dir: &Path, summary: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(memory_dir)?;
    let path = memory_dir.join("session_summary.md");
    std::fs::write(path, summary)
}

/// Extract key facts from text using heuristic signal detection.
///
/// This is the same logic as `shell.rs::extract_and_save_memories` but
/// operates on a single text blob rather than a message array.
fn extract_facts(text: &str) -> Vec<String> {
    const MAX_LINES: usize = 50;
    const MAX_CHARS: usize = 8_000;

    let text = if text.len() > MAX_CHARS {
        &text[..MAX_CHARS]
    } else {
        text
    };

    let mut facts = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() < 10 {
            continue;
        }

        if is_memory_signal(trimmed) {
            facts.push(format!("- {trimmed}"));
        }

        if facts.len() >= MAX_LINES {
            break;
        }
    }

    facts
}

/// Determine whether a line looks like a key fact worth remembering.
fn is_memory_signal(line: &str) -> bool {
    let lower = line.to_lowercase();

    // Blocklist: tool descriptions and filler
    let noise = [
        "read_file",
        "write_file",
        "edit_file",
        "bash:",
        "glob:",
        "grep:",
        "list_dir",
        "memory_query",
        "memory_propose",
        "memory_commit",
        "let me know if you",
        "feel free to ask",
        "i can help with",
        "here's how you can",
        "would you like to",
        "shall i proceed",
        "i hope this helps",
        "is there anything else",
    ];
    let stripped = lower.replace("**", "").replace('`', "");
    if noise.iter().any(|t| stripped.contains(t)) {
        return false;
    }

    // Bullet points are often summaries
    if lower.starts_with("- ") || lower.starts_with("* ") {
        return true;
    }

    // Headings
    if lower.starts_with("## ") || lower.starts_with("### ") {
        return true;
    }

    // Signal words
    let signals = [
        "decision:",
        "decided",
        "chose",
        "created",
        "implemented",
        "fixed",
        "error:",
        "warning:",
        "bug:",
        "todo:",
        "note:",
        "important:",
        "key finding",
        "conclusion",
        "summary",
        "architecture",
        "pattern:",
        "learned",
        "discovered",
        "the issue was",
        "root cause",
        "solution:",
        "workaround:",
    ];
    if signals.iter().any(|w| lower.contains(w)) {
        return true;
    }

    // File paths
    if line.contains('/')
        && (lower.contains(".rs")
            || lower.contains(".toml")
            || lower.contains(".ts")
            || lower.contains(".md"))
    {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bullet_points() {
        let text = "Here are the findings:\n- Fixed the bug in auth.rs\n- Decided to use OAuth2\nSome filler text.";
        let facts = extract_facts(text);
        assert_eq!(facts.len(), 2);
        assert!(facts[0].contains("Fixed the bug"));
        assert!(facts[1].contains("Decided to use OAuth2"));
    }

    #[test]
    fn filters_tool_noise() {
        let text = "- read_file: reads a file\n- Fixed the auth bug\n- bash: runs commands";
        let facts = extract_facts(text);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("Fixed the auth bug"));
    }

    #[test]
    fn filters_filler() {
        let text = "- Let me know if you need anything else\n- The root cause was a race condition";
        let facts = extract_facts(text);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("root cause"));
    }

    #[test]
    fn extracts_signal_words() {
        let text = "The solution: use async_trait instead of BoxFuture.\nSome random text here.";
        let facts = extract_facts(text);
        assert_eq!(facts.len(), 1);
        assert!(facts[0].contains("solution"));
    }

    #[test]
    fn empty_text_returns_empty() {
        assert!(extract_facts("").is_empty());
        assert!(extract_facts("short").is_empty());
    }
}
