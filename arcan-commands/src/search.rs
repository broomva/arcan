//! `/search` slash command — keyword search across workspace memory files.

use crate::{Command, CommandContext, CommandResult};

pub struct SearchCommand;

impl Command for SearchCommand {
    fn name(&self) -> &str {
        "search"
    }

    fn aliases(&self) -> &[&str] {
        &["find"]
    }

    fn description(&self) -> &str {
        "Search memory files for keywords"
    }

    fn execute(&self, args: &str, ctx: &mut CommandContext) -> CommandResult {
        let query = args.trim();
        if query.is_empty() {
            return CommandResult::Output(
                "Usage: /search <query>\n  Search memory files for keyword matches.".to_string(),
            );
        }

        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();

        let memory_dir = &ctx.memory_dir;
        if !memory_dir.exists() {
            return CommandResult::Output("No memory directory found.".to_string());
        }

        let entries = match std::fs::read_dir(memory_dir) {
            Ok(entries) => entries,
            Err(e) => return CommandResult::Error(format!("Failed to read memory dir: {e}")),
        };

        let mut results: Vec<SearchResult> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();

            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };

            let content_lower = content.to_lowercase();
            let hits: usize = keywords
                .iter()
                .filter(|kw| content_lower.contains(**kw))
                .count();

            if hits == 0 {
                continue;
            }

            // Find the best matching line and extract context around it
            let excerpt = extract_excerpt(&content, &keywords);

            results.push(SearchResult {
                file: name,
                hits,
                excerpt,
            });
        }

        if results.is_empty() {
            return CommandResult::Output(format!("No matches for \"{query}\"."));
        }

        // Sort by hit count descending, then file name
        results.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.file.cmp(&b.file)));

        let mut output = format!(
            "Search results for \"{}\" ({} files):\n",
            query,
            results.len()
        );
        for r in &results {
            output.push_str(&format!(
                "\n  {} ({}/{} keywords)\n    {}\n",
                r.file,
                r.hits,
                keywords.len(),
                r.excerpt
            ));
        }
        CommandResult::Output(output.trim_end().to_string())
    }
}

struct SearchResult {
    file: String,
    hits: usize,
    excerpt: String,
}

/// Extract a short excerpt around the first keyword match.
fn extract_excerpt(content: &str, keywords: &[&str]) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let content_lower = content.to_lowercase();

    // Find the first line containing any keyword
    let match_idx = lines.iter().position(|line| {
        let ll = line.to_lowercase();
        keywords.iter().any(|kw| ll.contains(*kw))
    });

    match match_idx {
        Some(idx) => {
            // Show ±1 lines around the match
            let start = idx.saturating_sub(1);
            let end = (idx + 2).min(lines.len());
            let excerpt: Vec<&str> = lines[start..end].to_vec();
            let joined = excerpt.join(" | ");
            if joined.len() > 200 {
                format!("{}...", &joined[..200])
            } else {
                joined
            }
        }
        None => {
            // Keyword matched somewhere (maybe across line boundaries)
            let pos = keywords
                .iter()
                .find_map(|kw| content_lower.find(*kw))
                .unwrap_or(0);
            let start = pos.saturating_sub(50);
            let end = (pos + 150).min(content.len());
            let snippet = &content[start..end];
            format!("...{}...", snippet.replace('\n', " | "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn search_empty_query_shows_usage() {
        let cmd = SearchCommand;
        let mut ctx = CommandContext::default();
        match cmd.execute("", &mut ctx) {
            CommandResult::Output(text) => assert!(text.contains("Usage:")),
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn search_no_dir() {
        let cmd = SearchCommand;
        let mut ctx = CommandContext {
            memory_dir: PathBuf::from("/nonexistent/path/memory"),
            ..Default::default()
        };
        match cmd.execute("test", &mut ctx) {
            CommandResult::Output(text) => assert!(text.contains("No memory directory")),
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn search_no_matches() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-search-test-nomatch-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "nothing relevant here").unwrap();

        let cmd = SearchCommand;
        let mut ctx = CommandContext {
            memory_dir: dir.clone(),
            ..Default::default()
        };
        match cmd.execute("xyzzyx", &mut ctx) {
            CommandResult::Output(text) => assert!(text.contains("No matches")),
            other => panic!("expected Output, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_finds_matching_file() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-search-test-match-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("e2e-result.md"),
            "---\ntitle: e2e test\n---\nThe E2E test passed successfully.",
        )
        .unwrap();
        std::fs::write(dir.join("unrelated.md"), "something else entirely").unwrap();

        let cmd = SearchCommand;
        let mut ctx = CommandContext {
            memory_dir: dir.clone(),
            ..Default::default()
        };
        match cmd.execute("e2e test", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("e2e-result"));
                assert!(text.contains("2/2 keywords"));
                // unrelated should not appear
                assert!(!text.contains("unrelated"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn search_case_insensitive() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-search-test-case-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.md"), "The Arcan Shell is working").unwrap();

        let cmd = SearchCommand;
        let mut ctx = CommandContext {
            memory_dir: dir.clone(),
            ..Default::default()
        };
        match cmd.execute("arcan shell", &mut ctx) {
            CommandResult::Output(text) => {
                assert!(text.contains("note"));
                assert!(text.contains("2/2 keywords"));
            }
            other => panic!("expected Output, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extract_excerpt_finds_context() {
        let content = "line one\nline two\nthe keyword here\nline four\nline five";
        let excerpt = extract_excerpt(content, &["keyword"]);
        assert!(excerpt.contains("keyword"));
        assert!(excerpt.contains("line two")); // context line before
    }
}
