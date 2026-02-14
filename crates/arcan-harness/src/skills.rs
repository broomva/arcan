use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Parsed SKILL.md frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    /// Arbitrary key-value metadata.
    #[serde(default, flatten)]
    pub metadata: BTreeMap<String, serde_yaml::Value>,
}

/// A loaded skill with its full content.
#[derive(Debug, Clone)]
pub struct LoadedSkill {
    pub meta: SkillMetadata,
    /// Markdown instructions (body after frontmatter).
    pub body: String,
    /// Skill directory (for relative file refs).
    pub root_dir: PathBuf,
}

/// Skill registry: scans directories, caches metadata, loads on demand.
pub struct SkillRegistry {
    skills: BTreeMap<String, LoadedSkill>,
}

impl SkillRegistry {
    /// Scan directories for SKILL.md files. Returns a registry of discovered skills.
    pub fn discover(dirs: &[PathBuf]) -> Result<Self, SkillError> {
        let mut skills = BTreeMap::new();

        for dir in dirs {
            if !dir.exists() {
                continue;
            }

            for entry in walkdir::WalkDir::new(dir)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case("SKILL.md")
                })
            {
                let path = entry.path();
                let content = std::fs::read_to_string(path).map_err(|e| SkillError::Io {
                    path: path.to_path_buf(),
                    source: e,
                })?;

                match parse_skill_md(&content) {
                    Ok((meta, body)) => {
                        let root_dir = path
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .to_path_buf();

                        let name = meta.name.clone();
                        skills.insert(
                            name,
                            LoadedSkill {
                                meta,
                                body,
                                root_dir,
                            },
                        );
                    }
                    Err(e) => {
                        // Log but don't fail — skip malformed skills
                        tracing::warn!(
                            path = %path.display(),
                            error = %e,
                            "Skipping malformed skill"
                        );
                    }
                }
            }
        }

        Ok(Self { skills })
    }

    /// Number of discovered skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Get skill metadata for system prompt injection.
    /// Returns a compact listing of all available skills (~100 tokens per skill).
    pub fn system_prompt_catalog(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut lines = vec!["Available skills:".to_string()];
        for skill in self.skills.values() {
            let invocable = if skill.meta.user_invocable == Some(true) {
                " [user-invocable]"
            } else {
                ""
            };
            lines.push(format!(
                "- {}: {}{}",
                skill.meta.name, skill.meta.description, invocable
            ));
        }
        lines.join("\n")
    }

    /// Load full skill content when activated.
    pub fn activate(&self, name: &str) -> Option<&LoadedSkill> {
        self.skills.get(name)
    }

    /// Get all skill names.
    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    /// Get allowed tools for an active skill (if restricted).
    pub fn allowed_tools(&self, name: &str) -> Option<&[String]> {
        self.skills
            .get(name)
            .and_then(|s| s.meta.allowed_tools.as_deref())
    }
}

/// Parse a SKILL.md file into metadata + body.
pub fn parse_skill_md(content: &str) -> Result<(SkillMetadata, String), SkillError> {
    let trimmed = content.trim();

    if !trimmed.starts_with("---") {
        return Err(SkillError::MissingFrontmatter);
    }

    // Find the closing "---" (skip the first one)
    let after_first = &trimmed[3..];
    let closing = after_first
        .find("\n---")
        .ok_or(SkillError::MissingFrontmatter)?;

    let yaml_str = &after_first[..closing].trim();
    let body_start = 3 + closing + 4; // skip "\n---"
    let body = if body_start < trimmed.len() {
        trimmed[body_start..].trim().to_string()
    } else {
        String::new()
    };

    let meta: SkillMetadata =
        serde_yaml::from_str(yaml_str).map_err(|e| SkillError::YamlParse(e.to_string()))?;

    if meta.name.is_empty() {
        return Err(SkillError::MissingField("name".to_string()));
    }
    if meta.description.is_empty() {
        return Err(SkillError::MissingField("description".to_string()));
    }

    Ok((meta, body))
}

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("SKILL.md missing YAML frontmatter (must start and end with ---)")]
    MissingFrontmatter,
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("IO error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_full_skill_md() {
        let content = r#"---
name: commit-helper
description: Helps create well-structured git commits
license: MIT
tags:
  - git
  - workflow
allowed_tools:
  - bash
  - read_file
user_invocable: true
---
# Commit Helper

When the user asks to commit, follow these steps:
1. Run `git status` to see changes
2. Draft a commit message
3. Ask for confirmation
"#;

        let (meta, body) = parse_skill_md(content).unwrap();
        assert_eq!(meta.name, "commit-helper");
        assert_eq!(meta.description, "Helps create well-structured git commits");
        assert_eq!(meta.license, Some("MIT".to_string()));
        assert_eq!(meta.tags, vec!["git", "workflow"]);
        assert_eq!(
            meta.allowed_tools,
            Some(vec!["bash".to_string(), "read_file".to_string()])
        );
        assert_eq!(meta.user_invocable, Some(true));
        assert!(body.contains("# Commit Helper"));
        assert!(body.contains("Run `git status`"));
    }

    #[test]
    fn parse_minimal_skill_md() {
        let content = r#"---
name: simple
description: A simple skill
---
Just do the thing.
"#;
        let (meta, body) = parse_skill_md(content).unwrap();
        assert_eq!(meta.name, "simple");
        assert_eq!(meta.description, "A simple skill");
        assert_eq!(meta.tags, Vec::<String>::new());
        assert_eq!(meta.allowed_tools, None);
        assert_eq!(meta.user_invocable, None);
        assert_eq!(body, "Just do the thing.");
    }

    #[test]
    fn parse_missing_frontmatter_fails() {
        let content = "# No frontmatter\nJust text.";
        assert!(parse_skill_md(content).is_err());
    }

    #[test]
    fn parse_missing_name_fails() {
        let content = r#"---
description: No name field
---
Body."#;
        // serde_yaml will error because name is required
        assert!(parse_skill_md(content).is_err());
    }

    #[test]
    fn parse_empty_name_fails() {
        let content = r#"---
name: ""
description: Empty name
---
Body."#;
        let err = parse_skill_md(content).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn discovery_from_temp_dir() {
        let dir = TempDir::new().unwrap();

        // Create a skill directory
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();

        let skill_content = r#"---
name: test-skill
description: A test skill for unit tests
tags:
  - test
---
# Test Skill
This is the body.
"#;
        std::fs::write(skill_dir.join("SKILL.md"), skill_content).unwrap();

        let registry = SkillRegistry::discover(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(registry.count(), 1);

        let skill = registry.activate("test-skill").unwrap();
        assert_eq!(skill.meta.name, "test-skill");
        assert!(skill.body.contains("# Test Skill"));
        assert_eq!(skill.root_dir, skill_dir);
    }

    #[test]
    fn discovery_skips_malformed_skills() {
        let dir = TempDir::new().unwrap();

        // Create a malformed skill
        let bad_dir = dir.path().join("bad-skill");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("SKILL.md"), "no frontmatter here").unwrap();

        // Create a good skill
        let good_dir = dir.path().join("good-skill");
        std::fs::create_dir_all(&good_dir).unwrap();
        std::fs::write(
            good_dir.join("SKILL.md"),
            "---\nname: good\ndescription: A good skill\n---\nGood body.",
        )
        .unwrap();

        let registry = SkillRegistry::discover(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(registry.count(), 1);
        assert!(registry.activate("good").is_some());
    }

    #[test]
    fn discovery_nonexistent_dir_is_ok() {
        let registry =
            SkillRegistry::discover(&[PathBuf::from("/nonexistent/path/12345")]).unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn system_prompt_catalog_formatting() {
        let dir = TempDir::new().unwrap();

        let skill1_dir = dir.path().join("skill-a");
        std::fs::create_dir_all(&skill1_dir).unwrap();
        std::fs::write(
            skill1_dir.join("SKILL.md"),
            "---\nname: alpha\ndescription: Alpha skill\nuser_invocable: true\n---\nBody A.",
        )
        .unwrap();

        let skill2_dir = dir.path().join("skill-b");
        std::fs::create_dir_all(&skill2_dir).unwrap();
        std::fs::write(
            skill2_dir.join("SKILL.md"),
            "---\nname: beta\ndescription: Beta skill\n---\nBody B.",
        )
        .unwrap();

        let registry = SkillRegistry::discover(&[dir.path().to_path_buf()]).unwrap();
        let catalog = registry.system_prompt_catalog();

        assert!(catalog.contains("Available skills:"));
        assert!(catalog.contains("- alpha: Alpha skill [user-invocable]"));
        assert!(catalog.contains("- beta: Beta skill"));
    }

    #[test]
    fn allowed_tools_filtering() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("restricted");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: restricted\ndescription: Restricted skill\nallowed_tools:\n  - read_file\n  - grep\n---\nBody.",
        )
        .unwrap();

        let registry = SkillRegistry::discover(&[dir.path().to_path_buf()]).unwrap();
        let tools = registry.allowed_tools("restricted").unwrap();
        assert_eq!(tools, &["read_file", "grep"]);
    }

    #[test]
    fn skill_names_returns_all() {
        let dir = TempDir::new().unwrap();

        for name in &["aaa", "bbb", "ccc"] {
            let skill_dir = dir.path().join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!(
                    "---\nname: {}\ndescription: Skill {}\n---\nBody.",
                    name, name
                ),
            )
            .unwrap();
        }

        let registry = SkillRegistry::discover(&[dir.path().to_path_buf()]).unwrap();
        let mut names = registry.skill_names();
        names.sort();
        assert_eq!(names, vec!["aaa", "bbb", "ccc"]);
    }
}
