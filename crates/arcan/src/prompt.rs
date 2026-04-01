//! Liquid prompt system — assembles a single, structured system prompt from multiple sources.
//!
//! Mirrors the 6-layer architecture used by Claude Code:
//! 1. Role definition
//! 2. Environment info (OS, shell, date, model)
//! 3. Git context (branch, status, recent commits)
//! 4. Project instructions (CLAUDE.md, AGENTS.md, docs/, .control/policy.yaml)
//! 5. Memory context (cross-session `.arcan/memory/*.md`)
//! 6. Skill catalog
//! 7. Guidelines

use std::path::Path;

/// Build the complete system prompt from all available context sources.
///
/// Each non-empty section is separated by a horizontal rule for readability.
/// The result is a single string suitable for a single `ChatMessage::system()`.
pub fn build_system_prompt(
    workspace: &Path,
    provider_name: &str,
    model_name: &str,
    memory_dir: &Path,
    skill_catalog: Option<&str>,
    claude_md_content: Option<&str>,
) -> String {
    let mut sections = Vec::new();

    // 1. Role definition
    sections.push(build_role_section());

    // 2. Environment info
    sections.push(build_environment_section(
        workspace,
        provider_name,
        model_name,
    ));

    // 3. Git context
    if let Some(git) = build_git_section(workspace) {
        sections.push(git);
    }

    // 4. CLAUDE.md / project instructions
    if let Some(instructions) = claude_md_content {
        if !instructions.is_empty() {
            sections.push(format!("# Project Instructions\n\n{instructions}"));
        }
    }

    // 5. Memory context
    if let Some(memory) = build_memory_section(memory_dir) {
        sections.push(memory);
    }

    // 6. Skills catalog
    if let Some(catalog) = skill_catalog {
        if !catalog.is_empty() {
            sections.push(format!("# Available Skills\n\n{catalog}"));
        }
    }

    // 7. Guidelines
    sections.push(build_guidelines_section());

    sections.join("\n\n---\n\n")
}

/// The role identity block — defines what the agent is and how it should behave.
fn build_role_section() -> String {
    "# System\n\n\
     You are an AI coding assistant powered by Arcan, the Life Agent OS runtime. \
     You help users with software engineering tasks by reading files, editing code, \
     running commands, and searching codebases. Be concise and direct. \
     Read files before editing them. Use tools to explore rather than guessing. \
     Follow existing code style and conventions."
        .to_string()
}

/// Platform, runtime, and temporal context.
fn build_environment_section(workspace: &Path, provider: &str, model: &str) -> String {
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
fn build_git_section(workspace: &Path) -> Option<String> {
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

/// Cross-session memory from `.arcan/memory/*.md` files.
///
/// Reads all markdown files from the memory directory and returns a formatted
/// string. Returns `None` if the directory doesn't exist or contains no files.
fn build_memory_section(memory_dir: &Path) -> Option<String> {
    if !memory_dir.exists() {
        return None;
    }

    let entries = std::fs::read_dir(memory_dir).ok()?;
    let mut sections = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
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

/// Behavioral guidelines that bound how the agent operates.
fn build_guidelines_section() -> String {
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

        let prompt = build_system_prompt(
            workspace,
            "anthropic",
            "claude-sonnet-4-5-20250929",
            &memory_dir,
            Some("- skill_a: Does A\n- skill_b: Does B"),
            Some("# My Project\n\nBuild fast."),
        );

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

        let prompt = build_system_prompt(workspace, "mock", "mock-model", &memory_dir, None, None);

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
}
