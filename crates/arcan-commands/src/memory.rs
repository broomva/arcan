//! `/memory` slash command — list memory files with previews.

use crate::{Command, CommandContext, CommandResult};

pub struct MemoryCommand;

impl Command for MemoryCommand {
    fn name(&self) -> &str {
        "memory"
    }

    fn aliases(&self) -> &[&str] {
        &["mem"]
    }

    fn description(&self) -> &str {
        "List memory files with previews"
    }

    fn execute(&self, _args: &str, ctx: &mut CommandContext) -> CommandResult {
        let memory_dir = &ctx.memory_dir;

        if !memory_dir.exists() {
            return CommandResult::Output("No memory directory found.".to_string());
        }

        let entries = match std::fs::read_dir(memory_dir) {
            Ok(entries) => entries,
            Err(e) => return CommandResult::Error(format!("Failed to read memory dir: {e}")),
        };

        let mut files: Vec<(String, String)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                let preview = match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let first_line = content.lines().next().unwrap_or("(empty)");
                        if first_line.len() > 80 {
                            format!("{}...", &first_line[..80])
                        } else {
                            first_line.to_string()
                        }
                    }
                    Err(_) => "(unreadable)".to_string(),
                };
                files.push((name, preview));
            }
        }

        if files.is_empty() {
            return CommandResult::Output("No memory files found.".to_string());
        }

        files.sort_by(|a, b| a.0.cmp(&b.0));

        let mut output = format!("Memory files ({}):\n", files.len());
        for (name, preview) in &files {
            output.push_str(&format!("  {name}: {preview}\n"));
        }
        CommandResult::Output(output.trim_end().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn memory_no_dir() {
        let cmd = MemoryCommand;
        let mut ctx = CommandContext {
            memory_dir: PathBuf::from("/nonexistent/path/memory"),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => assert!(text.contains("No memory directory")),
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn memory_empty_dir() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-memory-test-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let cmd = MemoryCommand;
        let mut ctx = CommandContext {
            memory_dir: dir.clone(),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => assert!(text.contains("No memory files")),
            other => panic!("expected Output, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn memory_lists_files_with_previews() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-memory-test-list-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("session_summary.md"),
            "# Session Summary\nKey fact",
        )
        .unwrap();
        std::fs::write(dir.join("global.md"), "# Global Memory\nSome note").unwrap();

        let cmd = MemoryCommand;
        let mut ctx = CommandContext {
            memory_dir: dir.clone(),
            ..Default::default()
        };
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("Memory files (2)"));
                assert!(text.contains("session_summary"));
                assert!(text.contains("# Session Summary"));
                assert!(text.contains("global"));
                assert!(text.contains("# Global Memory"));
            }
            other => panic!("expected Output, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}
