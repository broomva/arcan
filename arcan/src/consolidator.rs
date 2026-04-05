//! Lightweight memory consolidation engine (BRO-421).
//!
//! Runs on session end (graceful quit or EOF) and on `/consolidate`.
//! Scans episodic memories for patterns, decays old unused memories,
//! and prunes below-threshold entries.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Run the full consolidation pipeline on the memory directory.
#[allow(clippy::print_stderr)]
pub fn consolidate(memory_dir: &Path) {
    if !memory_dir.exists() {
        return;
    }
    decay_unused_memories(memory_dir);
    extract_patterns(memory_dir);
    prune_low_importance(memory_dir);
}

// ---------------------------------------------------------------------------
// Frontmatter helpers
// ---------------------------------------------------------------------------

/// Parsed frontmatter fields relevant to consolidation.
#[derive(Debug, Default)]
struct FrontmatterFields {
    importance: Option<f64>,
    access_count: Option<u64>,
    kind: Option<String>,
}

/// Parse YAML frontmatter from a markdown file's content.
///
/// Returns the parsed fields and the byte offset where the body starts.
fn parse_frontmatter(content: &str) -> (FrontmatterFields, usize) {
    let mut fields = FrontmatterFields::default();

    if !content.starts_with("---") {
        return (fields, 0);
    }

    // Find closing `---`
    let rest = &content[3..];
    let Some(end) = rest.find("\n---") else {
        return (fields, 0);
    };

    let fm_block = &rest[..end];
    let body_start = 3 + end + 4; // skip opening `---` + fm + `\n---`

    for line in fm_block.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("importance:") {
            fields.importance = val.trim().parse().ok();
        } else if let Some(val) = line.strip_prefix("access_count:") {
            fields.access_count = val.trim().parse().ok();
        } else if let Some(val) = line.strip_prefix("kind:") {
            fields.kind = Some(val.trim().to_string());
        }
    }

    (fields, body_start)
}

/// Rewrite the `importance` value in the frontmatter of `content`.
///
/// If the file has a frontmatter block with an `importance:` key, the value
/// is replaced in-place. If there is no frontmatter, one is prepended.
fn set_importance(content: &str, new_importance: f64) -> String {
    let val_str = format!("{new_importance:.2}");

    if let Some(rest) = content.strip_prefix("---") {
        // Try to replace existing importance line
        let has_importance = content.lines().any(|l| l.trim().starts_with("importance:"));
        if has_importance {
            let rebuilt: Vec<String> = content
                .lines()
                .map(|l| {
                    if l.trim().starts_with("importance:") {
                        format!("importance: {val_str}")
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            return rebuilt.join("\n");
        }
        // Frontmatter exists but no importance key — insert before closing ---
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            let after = &rest[end + 4..];
            return format!("---{fm}\nimportance: {val_str}\n---{after}");
        }
    }

    // No frontmatter — prepend one
    format!("---\nimportance: {val_str}\n---\n{content}")
}

// ---------------------------------------------------------------------------
// Consolidation passes
// ---------------------------------------------------------------------------

/// Reduce importance of memories not accessed recently.
///
/// For each `.md` file in the memory directory: if the file has not been
/// modified in the last 7 days and its importance is above 0.3, reduce
/// importance by 0.1.
#[allow(clippy::print_stderr)]
fn decay_unused_memories(memory_dir: &Path) {
    let Ok(entries) = fs::read_dir(memory_dir) else {
        return;
    };

    let seven_days_ago = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(7 * 24 * 3600))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut decayed = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        // Skip the index file itself
        if path.file_name().is_some_and(|n| n == "MEMORY.md") {
            continue;
        }

        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let mtime = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        if mtime > seven_days_ago {
            continue; // Recently modified — skip
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        let (fields, _) = parse_frontmatter(&content);
        let importance = fields.importance.unwrap_or(0.5);

        if importance > 0.3 {
            let new_importance = (importance - 0.1).max(0.0);
            let updated = set_importance(&content, new_importance);
            if fs::write(&path, updated).is_ok() {
                decayed += 1;
            }
        }
    }

    if decayed > 0 {
        eprintln!("[consolidate] Decayed {decayed} memories");
    }
}

/// Look for repeated patterns across episodic memories.
///
/// Reads all `session_summary*.md` files and looks for keywords that
/// appear in 3+ summaries. Creates a semantic memory for each pattern.
#[allow(clippy::print_stderr)]
fn extract_patterns(memory_dir: &Path) {
    let Ok(entries) = fs::read_dir(memory_dir) else {
        return;
    };

    let mut summaries = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with("session_summary")
            && name.ends_with(".md")
            && let Ok(content) = fs::read_to_string(&path)
        {
            summaries.push(content);
        }
    }

    if summaries.len() < 3 {
        return; // Not enough data for pattern detection
    }

    // Count word frequency across summaries (only meaningful words)
    let mut word_counts: HashMap<String, u32> = HashMap::new();
    let stop_words = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "above", "below", "between", "this", "that", "these", "those",
        "it", "its", "and", "or", "but", "not", "no", "if", "then", "else", "when", "up", "out",
        "so", "than", "too", "very", "just", "about", "all", "each",
    ];

    for summary in &summaries {
        // Track words seen in THIS summary to count per-document frequency
        let mut seen_in_doc = std::collections::HashSet::new();
        for word in summary.split_whitespace() {
            let clean: String = word
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .collect();
            if clean.len() >= 4
                && !stop_words.contains(&clean.as_str())
                && seen_in_doc.insert(clean.clone())
            {
                *word_counts.entry(clean).or_default() += 1;
            }
        }
    }

    // Find words appearing in 3+ summaries
    let patterns: Vec<(&String, &u32)> = word_counts
        .iter()
        .filter(|(_, count)| **count >= 3)
        .collect();

    if patterns.is_empty() {
        return;
    }

    let mut extracted = 0u32;
    for (keyword, count) in &patterns {
        let pattern_file = memory_dir.join(format!("pattern_{keyword}.md"));
        if pattern_file.exists() {
            continue; // Already extracted
        }
        let content = format!(
            "---\ntype: semantic\nkind: consolidate\nimportance: 0.4\naccess_count: 0\n---\n\
             # Pattern: {keyword}\n\n\
             Appears in {count} session summaries. This is a recurring theme in agent sessions.\n"
        );
        if fs::write(&pattern_file, content).is_ok() {
            extracted += 1;
        }
    }

    if extracted > 0 {
        eprintln!("[consolidate] Extracted {extracted} patterns");
    }
}

/// Remove memories below importance threshold.
///
/// Deletes `.md` files where importance < 0.05, access_count == 0, and
/// the file hasn't been modified in 30+ days.
#[allow(clippy::print_stderr)]
fn prune_low_importance(memory_dir: &Path) {
    let Ok(entries) = fs::read_dir(memory_dir) else {
        return;
    };

    let thirty_days_ago = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(30 * 24 * 3600))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut pruned = 0u32;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        if path.file_name().is_some_and(|n| n == "MEMORY.md") {
            continue;
        }

        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let mtime = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

        if mtime > thirty_days_ago {
            continue; // Too recent to prune
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        let (fields, _) = parse_frontmatter(&content);
        let importance = fields.importance.unwrap_or(0.5);
        let access_count = fields.access_count.unwrap_or(1);

        if importance < 0.05 && access_count == 0 {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if fs::remove_file(&path).is_ok() {
                eprintln!("[consolidate] Pruned {file_name}");
                pruned += 1;
            }
        }
    }

    if pruned > 0 {
        eprintln!("[consolidate] Pruned {pruned} total memories");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_with_importance() {
        let content = "---\nimportance: 0.7\naccess_count: 3\nkind: episodic\n---\n# Hello\nBody";
        let (fields, body_start) = parse_frontmatter(content);
        assert!((fields.importance.unwrap() - 0.7).abs() < f64::EPSILON);
        assert_eq!(fields.access_count, Some(3));
        assert_eq!(fields.kind.as_deref(), Some("episodic"));
        assert!(body_start > 0);
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "# No frontmatter\nJust body.";
        let (fields, body_start) = parse_frontmatter(content);
        assert!(fields.importance.is_none());
        assert_eq!(body_start, 0);
    }

    #[test]
    fn test_set_importance_existing() {
        let content = "---\nimportance: 0.7\nkind: episodic\n---\n# Hello";
        let updated = set_importance(content, 0.3);
        assert!(updated.contains("importance: 0.30"));
        assert!(updated.contains("kind: episodic"));
    }

    #[test]
    fn test_set_importance_no_frontmatter() {
        let content = "# Hello\nBody text.";
        let updated = set_importance(content, 0.5);
        assert!(updated.starts_with("---\nimportance: 0.50\n---\n"));
        assert!(updated.contains("# Hello"));
    }

    #[test]
    fn test_decay_reduces_importance() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        fs::create_dir_all(&mem_dir).unwrap();

        // Create a memory file with high importance
        let mem_file = mem_dir.join("old_memory.md");
        fs::write(
            &mem_file,
            "---\nimportance: 0.8\naccess_count: 0\n---\n# Old memory\nSome content.",
        )
        .unwrap();

        // Set mtime to 10 days ago
        let ten_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(10 * 24 * 3600))
            .unwrap();
        filetime::set_file_mtime(
            &mem_file,
            filetime::FileTime::from_system_time(ten_days_ago),
        )
        .unwrap();

        decay_unused_memories(&mem_dir);

        let content = fs::read_to_string(&mem_file).unwrap();
        let (fields, _) = parse_frontmatter(&content);
        let importance = fields.importance.unwrap();
        // Should have been reduced from 0.8 to 0.7
        assert!(
            (importance - 0.7).abs() < 0.01,
            "Expected ~0.7, got {importance}"
        );
    }

    #[test]
    fn test_prune_removes_low_importance() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        fs::create_dir_all(&mem_dir).unwrap();

        // Create a low-importance, zero-access memory
        let mem_file = mem_dir.join("stale.md");
        fs::write(
            &mem_file,
            "---\nimportance: 0.01\naccess_count: 0\n---\n# Stale\nNot useful.",
        )
        .unwrap();

        // Set mtime to 60 days ago
        let sixty_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(60 * 24 * 3600))
            .unwrap();
        filetime::set_file_mtime(
            &mem_file,
            filetime::FileTime::from_system_time(sixty_days_ago),
        )
        .unwrap();

        prune_low_importance(&mem_dir);

        assert!(!mem_file.exists(), "Stale file should have been pruned");
    }

    #[test]
    fn test_prune_keeps_important_memories() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        fs::create_dir_all(&mem_dir).unwrap();

        // Create a memory with decent importance
        let mem_file = mem_dir.join("important.md");
        fs::write(
            &mem_file,
            "---\nimportance: 0.5\naccess_count: 0\n---\n# Important\nKeep this.",
        )
        .unwrap();

        // Even if old
        let sixty_days_ago = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(60 * 24 * 3600))
            .unwrap();
        filetime::set_file_mtime(
            &mem_file,
            filetime::FileTime::from_system_time(sixty_days_ago),
        )
        .unwrap();

        prune_low_importance(&mem_dir);

        assert!(mem_file.exists(), "Important file should be kept");
    }

    #[test]
    fn test_consolidate_on_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        fs::create_dir_all(&mem_dir).unwrap();

        // Should not panic
        consolidate(&mem_dir);
    }

    #[test]
    fn test_consolidate_on_nonexistent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("nonexistent");

        // Should not panic
        consolidate(&mem_dir);
    }

    #[test]
    fn test_extract_patterns_creates_semantic_memories() {
        let dir = tempfile::tempdir().unwrap();
        let mem_dir = dir.path().join("memory");
        fs::create_dir_all(&mem_dir).unwrap();

        // Create 4 session summaries that all mention "refactoring"
        for i in 0..4 {
            fs::write(
                mem_dir.join(format!("session_summary_{i}.md")),
                format!(
                    "# Session Summary\n\n- Completed refactoring of module {i}\n- Fixed tests\n"
                ),
            )
            .unwrap();
        }

        extract_patterns(&mem_dir);

        let pattern_file = mem_dir.join("pattern_refactoring.md");
        assert!(
            pattern_file.exists(),
            "Pattern file for 'refactoring' should exist"
        );
        let content = fs::read_to_string(&pattern_file).unwrap();
        assert!(content.contains("kind: consolidate"));
        assert!(content.contains("type: semantic"));
    }
}
