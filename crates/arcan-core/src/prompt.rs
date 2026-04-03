//! Liquid prompt system — assembles a structured system prompt from multiple sources.
//!
//! Architecture (cacheable vs. dynamic split for prompt cache savings):
//!
//! **Cacheable** (stable across turns — Anthropic auto-caches matching prefixes):
//! 1. Role definition
//! 2. Environment info (OS, shell, date, model)
//! 3. Project instructions (CLAUDE.md, AGENTS.md, docs/, .control/policy.yaml)
//! 4. Guidelines
//!
//! **Dynamic** (changes per turn — appended after cacheable prefix):
//! 5. Git context (branch, status, recent commits)
//! 6. Memory context (MEMORY.md index from `.arcan/memory/*.md`)
//! 7. Workspace context (shared cross-session journal summaries)
//! 8. Skill catalog
//!
//! This module lives in `arcan-core` so both the shell REPL (`arcan` binary)
//! and the daemon HTTP server (`arcand`) can share the same prompt builder.

use std::collections::BTreeMap;
use std::path::Path;

/// Structured system prompt split into cacheable and dynamic sections.
///
/// Anthropic automatically caches the longest matching prefix of the system
/// prompt across turns. By placing stable content first (cacheable) and
/// per-turn content after (dynamic), we get ~75% token savings on cache hits.
#[derive(Debug, Clone)]
pub struct SystemPrompt {
    /// Stable across turns — gets Anthropic prompt cache hits.
    pub cacheable: String,
    /// Changes per turn — always re-sent fresh.
    pub dynamic: String,
}

impl SystemPrompt {
    /// Combine both sections into a single prompt string (backward compatible).
    pub fn combined(&self) -> String {
        if self.dynamic.is_empty() {
            self.cacheable.clone()
        } else {
            format!("{}\n\n---\n\n{}", self.cacheable, self.dynamic)
        }
    }
}

/// Build the complete system prompt from all available context sources.
///
/// Returns a [`SystemPrompt`] with cacheable (stable) and dynamic (per-turn)
/// sections. Use [`SystemPrompt::combined()`] for backward-compatible single string.
pub fn build_system_prompt(
    workspace: &Path,
    provider_name: &str,
    model_name: &str,
    memory_dir: &Path,
    workspace_context: Option<&str>,
    skill_catalog: Option<&str>,
    claude_md_content: Option<&str>,
) -> SystemPrompt {
    // --- CACHEABLE (stable across turns) ---
    let mut cacheable_sections = Vec::new();

    // 1. Role definition
    cacheable_sections.push(build_role_section());

    // 2. Environment info
    cacheable_sections.push(build_environment_section(
        workspace,
        provider_name,
        model_name,
    ));

    // 3. CLAUDE.md / project instructions
    if let Some(instructions) = claude_md_content {
        if !instructions.is_empty() {
            cacheable_sections.push(format!("# Project Instructions\n\n{instructions}"));
        }
    }

    // 4. Guidelines
    cacheable_sections.push(build_guidelines_section());

    let cacheable = cacheable_sections.join("\n\n---\n\n");

    // --- DYNAMIC (changes per turn) ---
    let mut dynamic_sections = Vec::new();

    // 5. Git context
    if let Some(git) = build_git_section(workspace) {
        dynamic_sections.push(git);
    }

    // 6. Memory context (MEMORY.md index)
    if let Some(memory) = build_memory_section(memory_dir) {
        dynamic_sections.push(memory);
    }

    // 7. Workspace context
    if let Some(context) = workspace_context {
        if !context.is_empty() {
            dynamic_sections.push(format!("# Workspace Context\n\n{context}"));
        }
    }

    // 8. Skills catalog
    if let Some(catalog) = skill_catalog {
        if !catalog.is_empty() {
            dynamic_sections.push(format!("# Available Skills\n\n{catalog}"));
        }
    }

    let dynamic = if dynamic_sections.is_empty() {
        String::new()
    } else {
        dynamic_sections.join("\n\n---\n\n")
    };

    SystemPrompt { cacheable, dynamic }
}

/// The role identity block — defines what the agent is and how it should behave.
pub fn build_role_section() -> String {
    "# System\n\n\
     You are an AI coding assistant powered by Arcan, the Life Agent OS runtime. \
     You help users with software engineering tasks by reading files, editing code, \
     running commands, and searching codebases. Be concise and direct. \
     Read files before editing them. Use tools to explore rather than guessing. \
     Follow existing code style and conventions."
        .to_string()
}

/// Platform, runtime, and temporal context.
pub fn build_environment_section(workspace: &Path, provider: &str, model: &str) -> String {
    let cwd = workspace.display();
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let date = chrono::Local::now().format("%Y-%m-%d");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());

    format!(
        "# Environment\n\n\
         - Working directory: {cwd}\n\
         - Platform: {platform} ({arch})\n\
         - Shell: {shell}\n\
         - Date: {date}\n\
         - Provider: {provider}\n\
         - Model: {model}"
    )
}

/// Git branch, working-tree status, and recent commits.
///
/// Returns `None` if the workspace is not inside a git repository.
pub fn build_git_section(workspace: &Path) -> Option<String> {
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !branch.status.success() {
        return None;
    }
    let branch_name = String::from_utf8_lossy(&branch.stdout).trim().to_string();

    let status = std::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(workspace)
        .output()
        .ok()?;
    let status_text = String::from_utf8_lossy(&status.stdout).trim().to_string();
    let status_display = if status_text.is_empty() {
        "Clean".to_string()
    } else if status_text.len() > 500 {
        format!("{}...(truncated)", &status_text[..500])
    } else {
        status_text
    };

    let log = std::process::Command::new("git")
        .args(["log", "--oneline", "-5"])
        .current_dir(workspace)
        .output()
        .ok()?;
    let log_text = String::from_utf8_lossy(&log.stdout).trim().to_string();

    Some(format!(
        "# Git Context\n\n\
         - Branch: {branch_name}\n\
         - Status:\n```\n{status_display}\n```\n\
         - Recent commits:\n```\n{log_text}\n```"
    ))
}

/// Load project instructions from the workspace hierarchy.
///
/// Searches for instructions in multiple locations (all optional, concatenated):
///
/// **Base rules** (project-level, not tied to any specific agent framework):
/// 1. `<workspace>/CLAUDE.md` — Claude Code conventions
/// 2. `<workspace>/AGENTS.md` — Agent operational rules and boundaries
/// 3. `<workspace>/.claude/CLAUDE.md` — Additional Claude-specific instructions
/// 4. `<workspace>/.claude/rules/*.md` — Granular rule files (sorted)
///
/// **Life framework context** (if running inside a Life Agent OS workspace):
/// 5. `<workspace>/../CLAUDE.md` — Parent workspace instructions (e.g., `core/life/CLAUDE.md`)
/// 6. `<workspace>/docs/STATUS.md` — Current implementation status
/// 7. `<workspace>/docs/ARCHITECTURE.md` — System architecture
/// 8. `<workspace>/docs/ROADMAP.md` — Development roadmap
///
/// **Control metalayer** (if present):
/// 9. `<workspace>/.control/policy.yaml` — Enforceable policy constraints
///
/// Returns the concatenated content, or `None` if nothing was found.
pub fn load_project_instructions(workspace: &Path) -> Option<String> {
    let mut contents = Vec::new();

    // --- Base rules ---

    // CLAUDE.md (Claude Code conventions — widely adopted standard)
    load_file_if_exists(workspace, "CLAUDE.md", &mut contents);

    // AGENTS.md (agent operational rules — framework-agnostic)
    load_file_if_exists(workspace, "AGENTS.md", &mut contents);

    // .claude/CLAUDE.md (additional instructions)
    load_file_if_exists(workspace, ".claude/CLAUDE.md", &mut contents);

    // .claude/rules/*.md (granular rules, sorted for deterministic ordering)
    load_rules_dir(workspace, ".claude/rules", &mut contents);

    // --- Life framework context (if present) ---

    // Parent CLAUDE.md (e.g., core/life/CLAUDE.md when running in core/life/arcan/)
    if let Some(parent) = workspace.parent() {
        let parent_claude = parent.join("CLAUDE.md");
        if parent_claude.exists() && parent_claude != workspace.join("CLAUDE.md") {
            if let Ok(content) = std::fs::read_to_string(&parent_claude) {
                if !content.trim().is_empty() {
                    contents.push(format!(
                        "<!-- from {} -->\n{}",
                        parent_claude.display(),
                        content
                    ));
                }
            }
        }
    }

    // docs/ context files — lightweight summaries that inform the agent
    // about project status without requiring tool calls
    for doc_file in &["docs/STATUS.md", "docs/ARCHITECTURE.md", "docs/ROADMAP.md"] {
        let path = workspace.join(doc_file);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    // Truncate large docs to first 2000 chars to save tokens
                    let truncated = if trimmed.len() > 2000 {
                        format!(
                            "{}\n\n... (truncated, {} total chars — use read_file for full content)",
                            &trimmed[..2000],
                            trimmed.len()
                        )
                    } else {
                        trimmed.to_string()
                    };
                    contents.push(format!("<!-- from {doc_file} -->\n{truncated}"));
                }
            }
        }
    }

    // --- Control metalayer ---

    // .control/policy.yaml — machine-readable policy constraints
    let policy_path = workspace.join(".control/policy.yaml");
    if policy_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&policy_path) {
            if !content.trim().is_empty() {
                contents.push(format!(
                    "<!-- Control policy (.control/policy.yaml) -->\n```yaml\n{}\n```",
                    content.trim()
                ));
            }
        }
    }

    if contents.is_empty() {
        None
    } else {
        Some(contents.join("\n\n"))
    }
}

/// Backward-compatible alias for `load_project_instructions`.
pub fn load_claude_md(workspace: &Path) -> Option<String> {
    load_project_instructions(workspace)
}

/// Load a single file relative to workspace if it exists and is non-empty.
fn load_file_if_exists(workspace: &Path, relative: &str, contents: &mut Vec<String>) {
    let path = workspace.join(relative);
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                contents.push(content);
            }
        }
    }
}

/// Load all .md files from a rules directory, sorted alphabetically.
fn load_rules_dir(workspace: &Path, relative: &str, contents: &mut Vec<String>) {
    let rules_dir = workspace.join(relative);
    if rules_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&rules_dir) {
            let mut rule_files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                .collect();
            rule_files.sort_by_key(std::fs::DirEntry::path);

            for entry in rule_files {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if !content.trim().is_empty() {
                        contents.push(content);
                    }
                }
            }
        }
    }
}

/// Cross-session memory loaded from the MEMORY.md index.
///
/// Reads the generated `MEMORY.md` index from the memory directory and returns
/// a formatted string for inclusion in the system prompt. Falls back to reading
/// individual `.md` files if the index doesn't exist.
///
/// Returns `None` if the directory doesn't exist or contains no memory files.
pub fn build_memory_section(memory_dir: &Path) -> Option<String> {
    if !memory_dir.exists() {
        return None;
    }

    // Prefer the generated MEMORY.md index
    let index_path = memory_dir.join("MEMORY.md");
    if index_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&index_path) {
            if !content.trim().is_empty() {
                return Some(format!("# Agent Memory\n\n{content}"));
            }
        }
    }

    // Fallback: read individual files (backward compat)
    let entries = std::fs::read_dir(memory_dir).ok()?;
    let mut sections = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") {
            continue;
        }
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                sections.push(format!("## {key}\n{content}"));
            }
        }
    }

    if sections.is_empty() {
        return None;
    }

    sections.sort();
    Some(format!(
        "# Agent Memory (cross-session)\n\n{}",
        sections.join("\n\n")
    ))
}

// ---------------------------------------------------------------------------
// MEMORY.md index generation (BRO-419)
// ---------------------------------------------------------------------------

/// Maximum number of lines allowed in the MEMORY.md index.
const MEMORY_INDEX_MAX_LINES: usize = 200;

/// Maximum number of bytes allowed in the MEMORY.md index.
const MEMORY_INDEX_MAX_BYTES: usize = 25_000;

/// Generate a `MEMORY.md` index from all `.md` files in the memory directory.
///
/// Groups entries by the `type` field in YAML frontmatter (defaults to "general").
/// Each entry is a markdown link with a description extracted from the first
/// non-frontmatter, non-heading content line.
///
/// The output is capped at 200 lines / 25KB.
pub fn generate_memory_index(memory_dir: &Path) -> String {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let Ok(entries) = std::fs::read_dir(memory_dir) else {
        return String::from("# Memory Index\n");
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") {
            continue;
        }

        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let content = std::fs::read_to_string(&path).unwrap_or_default();

        let mem_type = extract_frontmatter_type(&content).unwrap_or_else(|| "general".to_string());

        let description = extract_first_content_line(&content);

        sections
            .entry(mem_type)
            .or_default()
            .push(format!("- [{}]({}.md) — {}", key, key, description));
    }

    let mut index = String::from("# Memory Index\n\n");
    for (section, entries) in &sections {
        index.push_str(&format!("## {}\n", capitalize(section)));
        for entry in entries {
            index.push_str(entry);
            index.push('\n');
        }
        index.push('\n');
    }

    // Cap at 200 lines
    let lines: Vec<&str> = index.lines().collect();
    if lines.len() > MEMORY_INDEX_MAX_LINES {
        index = lines[..MEMORY_INDEX_MAX_LINES].join("\n");
        index.push_str("\n\n... (truncated, showing first 200 entries)\n");
    }

    // Cap at 25KB
    if index.len() > MEMORY_INDEX_MAX_BYTES {
        index.truncate(MEMORY_INDEX_MAX_BYTES);
        index.push_str("\n\n... (truncated at 25KB)\n");
    }

    index
}

/// Write the generated MEMORY.md index to disk.
///
/// Creates the memory directory if it doesn't exist.
pub fn write_memory_index(memory_dir: &Path) {
    let _ = std::fs::create_dir_all(memory_dir);
    let index = generate_memory_index(memory_dir);
    let index_path = memory_dir.join("MEMORY.md");
    let _ = std::fs::write(&index_path, &index);
}

/// Extract the `type` value from YAML frontmatter (between `---` markers).
///
/// Returns `None` if no frontmatter or no `type:` field is found.
fn extract_frontmatter_type(content: &str) -> Option<String> {
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let frontmatter = &content[3..3 + end];
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("type:") {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// Extract the first non-empty, non-heading content line after any frontmatter.
///
/// Skips YAML frontmatter (between `---` markers) and markdown headings.
/// Truncates to 120 characters.
fn extract_first_content_line(content: &str) -> String {
    let body = if let Some(after_prefix) = content.strip_prefix("---") {
        after_prefix
            .find("---")
            .map(|i| &after_prefix[i + 3..])
            .unwrap_or(content)
    } else {
        content
    };
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("(no description)")
        .chars()
        .take(120)
        .collect()
}

/// Capitalize the first character of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Behavioral guidelines that bound how the agent operates.
pub fn build_guidelines_section() -> String {
    "# Guidelines\n\n\
     - Read files before editing them\n\
     - Use tools to explore the codebase rather than guessing\n\
     - Be concise and direct in responses\n\
     - Follow existing code style and conventions\n\
     - Prefer editing existing files over creating new ones\n\
     - Do not add features beyond what was asked"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_build_system_prompt_includes_all_sections() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("notes.md"), "Some notes here").unwrap();

        let sp = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet-4-5-20250929",
            &memory_dir,
            Some("- Peer session: explored workspace journal"),
            Some("- skill_a: Does A\n- skill_b: Does B"),
            Some("# My Project\n\nBuild fast."),
        );
        let prompt = sp.combined();

        // All sections should be present
        assert!(prompt.contains("# System"), "missing role section");
        assert!(
            prompt.contains("# Environment"),
            "missing environment section"
        );
        assert!(
            prompt.contains("# Project Instructions"),
            "missing claude.md section"
        );
        assert!(prompt.contains("# Agent Memory"), "missing memory section");
        assert!(
            prompt.contains("# Workspace Context"),
            "missing workspace context section"
        );
        assert!(
            prompt.contains("# Available Skills"),
            "missing skills section"
        );
        assert!(
            prompt.contains("# Guidelines"),
            "missing guidelines section"
        );
        // Section separators
        assert!(prompt.contains("---"), "missing section separators");
    }

    #[test]
    fn test_build_system_prompt_omits_empty_sections() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        // Don't create memory dir — should be omitted

        let sp = build_system_prompt(
            workspace,
            "mock",
            "mock-model",
            &memory_dir,
            None,
            None,
            None,
        );
        let prompt = sp.combined();

        assert!(prompt.contains("# System"));
        assert!(prompt.contains("# Environment"));
        assert!(prompt.contains("# Guidelines"));
        assert!(
            !prompt.contains("# Project Instructions"),
            "should omit empty claude.md"
        );
        assert!(
            !prompt.contains("# Agent Memory"),
            "should omit missing memory"
        );
        assert!(
            !prompt.contains("# Available Skills"),
            "should omit empty skills"
        );
    }

    #[test]
    fn test_load_claude_md_from_workspace() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::write(workspace.join("CLAUDE.md"), "# Instructions\nDo X.").unwrap();

        let result = load_project_instructions(workspace);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Do X."));
    }

    #[test]
    fn test_load_agents_md() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::write(workspace.join("AGENTS.md"), "# Agent Rules\nBe safe.").unwrap();

        let result = load_project_instructions(workspace);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Be safe."));
    }

    #[test]
    fn test_load_both_claude_and_agents_md() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::write(workspace.join("CLAUDE.md"), "Claude rules.").unwrap();
        fs::write(workspace.join("AGENTS.md"), "Agent rules.").unwrap();

        let result = load_project_instructions(workspace).unwrap();
        assert!(result.contains("Claude rules."));
        assert!(result.contains("Agent rules."));
    }

    #[test]
    fn test_load_rules_dir() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let rules_dir = workspace.join(".claude/rules");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(rules_dir.join("code-style.md"), "Use snake_case.").unwrap();
        fs::write(rules_dir.join("testing.md"), "All code needs tests.").unwrap();

        let result = load_project_instructions(workspace);
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("Use snake_case."));
        assert!(content.contains("All code needs tests."));
    }

    #[test]
    fn test_load_docs_context() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let docs_dir = workspace.join("docs");
        fs::create_dir_all(&docs_dir).unwrap();
        fs::write(docs_dir.join("STATUS.md"), "# Status\n100% tests passing").unwrap();
        fs::write(docs_dir.join("ARCHITECTURE.md"), "# Arch\nEvent-sourced.").unwrap();

        let result = load_project_instructions(workspace).unwrap();
        assert!(result.contains("100% tests passing"));
        assert!(result.contains("Event-sourced."));
    }

    #[test]
    fn test_load_control_policy() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let control_dir = workspace.join(".control");
        fs::create_dir_all(&control_dir).unwrap();
        fs::write(
            control_dir.join("policy.yaml"),
            "gates:\n  - name: G1\n    blocking: true",
        )
        .unwrap();

        let result = load_project_instructions(workspace).unwrap();
        assert!(result.contains("gates:"));
        assert!(result.contains("blocking: true"));
    }

    #[test]
    fn test_load_empty_workspace_returns_none() {
        let tmp = TempDir::new().unwrap();
        let result = load_project_instructions(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_git_section_in_repo() {
        // Run in the actual workspace which is a git repo
        let workspace = std::env::current_dir().unwrap();
        let result = build_git_section(&workspace);
        // This test is running inside a git repo (the arcan worktree),
        // so we should get a result.
        if let Some(git_section) = result {
            assert!(git_section.contains("# Git Context"));
            assert!(git_section.contains("Branch:"));
        }
        // If git is not available, the test passes trivially.
    }

    #[test]
    fn test_git_section_non_repo() {
        let tmp = TempDir::new().unwrap();
        let result = build_git_section(tmp.path());
        assert!(result.is_none(), "non-repo dir should return None");
    }

    #[test]
    fn test_environment_section() {
        let tmp = TempDir::new().unwrap();
        let section = build_environment_section(tmp.path(), "anthropic", "claude-sonnet");

        assert!(section.contains("# Environment"));
        assert!(section.contains("Platform:"));
        assert!(section.contains("Provider: anthropic"));
        assert!(section.contains("Model: claude-sonnet"));
        assert!(section.contains("Date:"));
    }

    #[test]
    fn test_memory_section() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("notes.md"), "Remember this.").unwrap();

        let result = build_memory_section(&memory_dir);
        assert!(result.is_some());
        let content = result.unwrap();
        assert!(content.contains("# Agent Memory"));
        assert!(content.contains("Remember this."));
    }

    #[test]
    fn test_memory_section_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        let result = build_memory_section(&memory_dir);
        assert!(result.is_none(), "empty memory dir should return None");
    }

    #[test]
    fn test_memory_section_missing_dir() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("nonexistent");

        let result = build_memory_section(&memory_dir);
        assert!(result.is_none(), "missing memory dir should return None");
    }

    #[test]
    fn test_role_section_content() {
        let role = build_role_section();
        assert!(role.contains("Arcan"));
        assert!(role.contains("Life Agent OS"));
    }

    #[test]
    fn test_guidelines_section_content() {
        let guidelines = build_guidelines_section();
        assert!(guidelines.contains("Read files before editing"));
        assert!(guidelines.contains("Do not add features beyond what was asked"));
    }

    #[test]
    fn test_load_combines_all_sources() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();

        // Create all sources
        fs::write(workspace.join("CLAUDE.md"), "Root instructions.").unwrap();
        fs::write(workspace.join("AGENTS.md"), "Agent boundaries.").unwrap();
        let dot_claude = workspace.join(".claude");
        fs::create_dir_all(&dot_claude).unwrap();
        fs::write(dot_claude.join("CLAUDE.md"), "Dot-claude instructions.").unwrap();
        let rules_dir = dot_claude.join("rules");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(rules_dir.join("style.md"), "Style rules.").unwrap();
        let docs = workspace.join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("STATUS.md"), "All green.").unwrap();
        let control = workspace.join(".control");
        fs::create_dir_all(&control).unwrap();
        fs::write(control.join("policy.yaml"), "version: 1").unwrap();

        let result = load_project_instructions(workspace).unwrap();
        assert!(result.contains("Root instructions."));
        assert!(result.contains("Agent boundaries."));
        assert!(result.contains("Dot-claude instructions."));
        assert!(result.contains("Style rules."));
        assert!(result.contains("All green."));
        assert!(result.contains("version: 1"));
    }

    #[test]
    fn test_backward_compat_load_claude_md() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::write(workspace.join("CLAUDE.md"), "Legacy call.").unwrap();
        let result = load_claude_md(workspace);
        assert!(result.is_some());
        assert!(result.unwrap().contains("Legacy call."));
    }

    /// Verify the prompt module is accessible from arcan-core's public API.
    #[test]
    fn test_prompt_available_from_core() {
        // Key public functions that both shell and daemon need.
        let _ = build_system_prompt
            as fn(
                &Path,
                &str,
                &str,
                &Path,
                Option<&str>,
                Option<&str>,
                Option<&str>,
            ) -> SystemPrompt;
        let _ = build_git_section as fn(&Path) -> Option<String>;
        let _ = load_project_instructions as fn(&Path) -> Option<String>;
        let _ = build_environment_section as fn(&Path, &str, &str) -> String;
        let _ = build_memory_section as fn(&Path) -> Option<String>;
        let _ = build_role_section as fn() -> String;
        let _ = build_guidelines_section as fn() -> String;
        let _ = generate_memory_index as fn(&Path) -> String;
        let _ = write_memory_index as fn(&Path);
    }

    // ── BRO-419: MEMORY.md index tests ──

    #[test]
    fn test_generate_memory_index() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        fs::write(
            memory_dir.join("project_notes.md"),
            "Key architecture decisions for the project.",
        )
        .unwrap();
        fs::write(
            memory_dir.join("user_prefs.md"),
            "---\ntype: user\n---\n# Preferences\nPrefers dark mode.",
        )
        .unwrap();

        let index = generate_memory_index(&memory_dir);

        assert!(index.contains("# Memory Index"), "missing header");
        assert!(
            index.contains("[project_notes]"),
            "missing project_notes entry"
        );
        assert!(index.contains("[user_prefs]"), "missing user_prefs entry");
        // user_prefs should be grouped under "User" section
        assert!(index.contains("## User"), "missing User section header");
        // project_notes has no frontmatter, defaults to "General"
        assert!(
            index.contains("## General"),
            "missing General section header"
        );
        // Description extraction
        assert!(
            index.contains("Key architecture decisions"),
            "missing description from project_notes"
        );
        assert!(
            index.contains("Prefers dark mode"),
            "missing description from user_prefs"
        );
    }

    #[test]
    fn test_memory_index_skips_memory_md() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        fs::write(memory_dir.join("MEMORY.md"), "# Old index").unwrap();
        fs::write(memory_dir.join("real_note.md"), "A real note.").unwrap();

        let index = generate_memory_index(&memory_dir);
        assert!(index.contains("[real_note]"));
        // MEMORY.md should not appear as an entry
        assert!(!index.contains("[MEMORY]"));
    }

    #[test]
    fn test_memory_index_caps_at_200_lines() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        // Create enough files to exceed 200 lines.
        // Each file adds 1 line. With header (2 lines), section header (1 line),
        // and trailing blank (1 line), we need >196 files to exceed 200 lines.
        for i in 0..250 {
            fs::write(
                memory_dir.join(format!("note_{i:03}.md")),
                format!("Content for note {i}."),
            )
            .unwrap();
        }

        let index = generate_memory_index(&memory_dir);
        let line_count = index.lines().count();
        // Should be capped (200 lines + truncation message ~2 more lines)
        assert!(line_count <= 205, "expected <= 205 lines, got {line_count}");
        assert!(
            index.contains("truncated"),
            "should contain truncation notice"
        );
    }

    #[test]
    fn test_memory_index_extracts_frontmatter_type() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        fs::write(
            memory_dir.join("arch_notes.md"),
            "---\ntype: project\ntags: [arch]\n---\n# Architecture\nEvent-sourced design.",
        )
        .unwrap();
        fs::write(
            memory_dir.join("tax_info.md"),
            "---\ntype: user\n---\nColombian tax rules.",
        )
        .unwrap();
        fs::write(
            memory_dir.join("general_stuff.md"),
            "Just some general notes without frontmatter.",
        )
        .unwrap();

        let index = generate_memory_index(&memory_dir);

        assert!(index.contains("## Project"), "missing Project section");
        assert!(index.contains("## User"), "missing User section");
        assert!(index.contains("## General"), "missing General section");
    }

    #[test]
    fn test_write_memory_index_creates_file() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("test.md"), "Test content.").unwrap();

        write_memory_index(&memory_dir);

        let index_path = memory_dir.join("MEMORY.md");
        assert!(index_path.exists(), "MEMORY.md should be created");
        let content = fs::read_to_string(&index_path).unwrap();
        assert!(content.contains("# Memory Index"));
        assert!(content.contains("[test]"));
    }

    #[test]
    fn test_memory_section_prefers_index() {
        let tmp = TempDir::new().unwrap();
        let memory_dir = tmp.path().join("memory");
        fs::create_dir_all(&memory_dir).unwrap();

        fs::write(memory_dir.join("notes.md"), "Individual note.").unwrap();
        // Write a MEMORY.md index
        write_memory_index(&memory_dir);

        let section = build_memory_section(&memory_dir).unwrap();
        // Should use the MEMORY.md index (contains "Memory Index" heading)
        assert!(
            section.contains("Memory Index"),
            "should prefer MEMORY.md index"
        );
    }

    // ── BRO-420: Prompt cache boundary tests ──

    #[test]
    fn test_system_prompt_struct_has_both_sections() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("notes.md"), "Some notes.").unwrap();

        let sp = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            Some("- skill_a: Does A"),
            Some("Build fast."),
        );

        assert!(!sp.cacheable.is_empty(), "cacheable should not be empty");
        assert!(!sp.dynamic.is_empty(), "dynamic should not be empty");
    }

    #[test]
    fn test_cacheable_section_stable() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        fs::write(workspace.join("CLAUDE.md"), "Project rules.").unwrap();
        let memory_dir = workspace.join(".arcan/memory");

        let sp1 = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            None,
            Some("Project rules."),
        );
        let sp2 = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            None,
            Some("Project rules."),
        );

        assert_eq!(
            sp1.cacheable, sp2.cacheable,
            "cacheable section should be identical for same inputs"
        );
    }

    #[test]
    fn test_dynamic_section_changes_with_memory() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        fs::create_dir_all(&memory_dir).unwrap();

        // No memory files
        let sp1 = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            None,
            None,
        );

        // Add a memory file
        fs::write(memory_dir.join("new_note.md"), "New insight.").unwrap();

        let sp2 = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            None,
            None,
        );

        assert_ne!(
            sp1.dynamic, sp2.dynamic,
            "dynamic section should change when memory files are added"
        );
        // Cacheable should remain the same
        assert_eq!(
            sp1.cacheable, sp2.cacheable,
            "cacheable section should not change with memory"
        );
    }

    #[test]
    fn test_cacheable_contains_role_env_guidelines() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join("memory");

        let sp = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            None,
            None,
        );

        assert!(
            sp.cacheable.contains("# System"),
            "cacheable should contain role"
        );
        assert!(
            sp.cacheable.contains("# Environment"),
            "cacheable should contain environment"
        );
        assert!(
            sp.cacheable.contains("# Guidelines"),
            "cacheable should contain guidelines"
        );
    }

    #[test]
    fn test_dynamic_contains_git_memory_skills() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("notes.md"), "Remember.").unwrap();

        let sp = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            Some("- Session abc turn 3: Added memory_similar"),
            Some("- skill_a"),
            None,
        );

        assert!(
            sp.dynamic.contains("# Agent Memory"),
            "dynamic should contain memory"
        );
        assert!(
            sp.dynamic.contains("# Workspace Context"),
            "dynamic should contain workspace context"
        );
        assert!(
            sp.dynamic.contains("# Available Skills"),
            "dynamic should contain skills"
        );
    }

    #[test]
    fn test_backward_compat_combined() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join(".arcan/memory");
        fs::create_dir_all(&memory_dir).unwrap();
        fs::write(memory_dir.join("notes.md"), "Some notes.").unwrap();

        let sp = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet",
            &memory_dir,
            None,
            Some("- skill_a"),
            Some("Project instructions."),
        );
        let combined = sp.combined();

        // Combined should contain content from both sections
        assert!(combined.contains("# System"));
        assert!(combined.contains("# Guidelines"));
        assert!(combined.contains("# Agent Memory"));
        assert!(combined.contains("# Available Skills"));
    }

    #[test]
    fn test_combined_empty_dynamic() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory_dir = workspace.join("nonexistent");

        let sp = build_system_prompt(workspace, "mock", "mock", &memory_dir, None, None, None);

        // With no git, no memory, no skills — dynamic should be empty
        let combined = sp.combined();
        // Combined should just be the cacheable section (no trailing ---)
        assert_eq!(combined, sp.cacheable);
    }

    // ── Helper function tests ──

    #[test]
    fn test_extract_frontmatter_type_valid() {
        let content = "---\ntype: project\ntags: [a, b]\n---\n# Title\nBody.";
        assert_eq!(
            extract_frontmatter_type(content),
            Some("project".to_string())
        );
    }

    #[test]
    fn test_extract_frontmatter_type_missing() {
        let content = "---\ntags: [a]\n---\nNo type field.";
        assert_eq!(extract_frontmatter_type(content), None);
    }

    #[test]
    fn test_extract_frontmatter_type_no_frontmatter() {
        let content = "Just plain text.";
        assert_eq!(extract_frontmatter_type(content), None);
    }

    #[test]
    fn test_extract_first_content_line_with_frontmatter() {
        let content = "---\ntype: user\n---\n# Heading\nFirst real line.";
        assert_eq!(extract_first_content_line(content), "First real line.");
    }

    #[test]
    fn test_extract_first_content_line_no_frontmatter() {
        let content = "# Heading\nContent line.";
        assert_eq!(extract_first_content_line(content), "Content line.");
    }

    #[test]
    fn test_extract_first_content_line_empty() {
        let content = "";
        assert_eq!(extract_first_content_line(content), "(no description)");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("general"), "General");
        assert_eq!(capitalize("user"), "User");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("ALREADY"), "ALREADY");
    }
}
