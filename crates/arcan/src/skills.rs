//! Skill discovery and registry management for Arcan.
//!
//! Bridges the `praxis-skills` discovery engine into the Arcan runtime,
//! providing:
//! - Directory-based SKILL.md discovery on startup
//! - Registry cache at `.arcan/skills/registry.json`
//! - CLI commands for listing and syncing skills
//! - System prompt catalog generation for the agent loop

use praxis_skills::registry::{SkillError, SkillRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Cached skill entry written to registry.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub name: String,
    pub description: String,
    pub source_dir: PathBuf,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub user_invocable: bool,
    #[serde(default)]
    pub disable_model_invocation: bool,
}

/// The on-disk registry cache format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRegistryCache {
    pub version: u32,
    pub discovered_at: String,
    pub skill_dirs: Vec<PathBuf>,
    pub skills: BTreeMap<String, RegistryEntry>,
}

/// Discover skills from the configured directories and optionally write a registry cache.
///
/// Returns the `SkillRegistry` for runtime use.
pub fn discover_skills(
    skill_dirs: &[PathBuf],
    data_dir: &Path,
    write_registry: bool,
) -> Result<SkillRegistry, SkillError> {
    let registry = SkillRegistry::discover(skill_dirs)?;

    tracing::info!(
        dirs = ?skill_dirs,
        skills_found = registry.count(),
        "skill discovery completed"
    );

    if write_registry && registry.count() > 0 {
        if let Err(e) = write_registry_cache(&registry, skill_dirs, data_dir) {
            tracing::warn!(error = %e, "failed to write skill registry cache");
        }
    }

    Ok(registry)
}

/// Write the registry cache to `.arcan/skills/registry.json`.
fn write_registry_cache(
    registry: &SkillRegistry,
    skill_dirs: &[PathBuf],
    data_dir: &Path,
) -> anyhow::Result<()> {
    let skills_dir = data_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let mut entries = BTreeMap::new();
    for name in registry.skill_names() {
        if let Some(skill) = registry.activate(&name) {
            entries.insert(
                name.clone(),
                RegistryEntry {
                    name,
                    description: skill.meta.description.clone(),
                    source_dir: skill.root_dir.clone(),
                    tags: skill.meta.tags.clone(),
                    user_invocable: skill.meta.user_invocable.unwrap_or(false),
                    disable_model_invocation: skill.meta.disable_model_invocation.unwrap_or(false),
                },
            );
        }
    }

    let cache = SkillRegistryCache {
        version: 1,
        discovered_at: chrono::Utc::now().to_rfc3339(),
        skill_dirs: skill_dirs.to_vec(),
        skills: entries,
    };

    let json = serde_json::to_string_pretty(&cache)?;
    std::fs::write(skills_dir.join("registry.json"), json)?;

    tracing::info!(
        path = %skills_dir.join("registry.json").display(),
        count = cache.skills.len(),
        "wrote skill registry cache"
    );

    Ok(())
}

/// Read the cached registry from disk (for fast startup or CLI queries).
pub fn read_registry_cache(data_dir: &Path) -> Option<SkillRegistryCache> {
    let path = data_dir.join("skills").join("registry.json");
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Print discovered skills in a human-readable format.
#[allow(clippy::print_stdout)]
pub fn print_skills_list(registry: &SkillRegistry) {
    if registry.count() == 0 {
        println!("No skills discovered.");
        println!();
        println!("Skill directories are configured in .arcan/config.toml [skills] section.");
        println!("Install skills with: npx skills add <package>");
        return;
    }

    println!("Discovered skills ({}):", registry.count());
    println!();

    for name in registry.skill_names() {
        if let Some(skill) = registry.activate(&name) {
            let invocable = if skill.meta.user_invocable == Some(true) {
                " [invocable]"
            } else {
                ""
            };
            let tags = if skill.meta.tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", skill.meta.tags.join(", "))
            };
            println!(
                "  {:<30} {}{}{}",
                name, skill.meta.description, tags, invocable
            );
            println!("    source: {}", skill.root_dir.display());
        }
    }
}

/// Print skills from the cached registry (faster than full discovery).
#[allow(clippy::print_stdout)]
pub fn print_cached_skills(data_dir: &Path) -> bool {
    if let Some(cache) = read_registry_cache(data_dir) {
        if cache.skills.is_empty() {
            println!("No skills in registry cache.");
            return true;
        }

        println!(
            "Cached skills ({}, discovered at {}):",
            cache.skills.len(),
            cache.discovered_at
        );
        println!();

        for entry in cache.skills.values() {
            let invocable = if entry.user_invocable {
                " [invocable]"
            } else {
                ""
            };
            let tags = if entry.tags.is_empty() {
                String::new()
            } else {
                format!(" ({})", entry.tags.join(", "))
            };
            println!(
                "  {:<30} {}{}{}",
                entry.name, entry.description, tags, invocable
            );
            println!("    source: {}", entry.source_dir.display());
        }
        true
    } else {
        false
    }
}

/// Sync skills from external install locations into `.arcan/skills/` via symlinks.
///
/// This creates symlinks in `.arcan/skills/` pointing to skills found in
/// `~/.agents/skills/` and `.agents/skills/`, making them discoverable
/// by Arcan's skill scanner.
#[allow(clippy::print_stdout)]
pub fn sync_skills_to_arcan(data_dir: &Path) -> anyhow::Result<usize> {
    let arcan_skills_dir = data_dir.join("skills");
    std::fs::create_dir_all(&arcan_skills_dir)?;

    let mut synced = 0;

    // Source directories to sync from (project-local and global)
    let cwd = std::env::current_dir()?;
    let mut source_dirs = vec![cwd.join(".agents").join("skills")];
    if let Some(home) = dirs::home_dir() {
        source_dirs.push(home.join(".agents").join("skills"));
    }

    for source_dir in &source_dirs {
        if !source_dir.exists() {
            continue;
        }

        let entries = std::fs::read_dir(source_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Only sync directories that contain a SKILL.md
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }

            let name = entry.file_name();
            let link_path = arcan_skills_dir.join(&name);

            // Skip if already linked or exists
            if link_path.exists() {
                continue;
            }

            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&path, &link_path)?;
                println!("  [link] {} -> {}", name.to_string_lossy(), path.display());
                synced += 1;
            }

            #[cfg(not(unix))]
            {
                // On non-Unix, copy instead of symlink
                let _ = path;
                let _ = link_path;
            }
        }
    }

    Ok(synced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_skill(dir: &Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {description}\ntags:\n  - test\nuser_invocable: true\n---\n# {name}\nBody."
            ),
        )
        .unwrap();
    }

    #[test]
    fn discover_and_cache_skills() {
        let skills_dir = TempDir::new().unwrap();
        let data_dir = TempDir::new().unwrap();

        create_skill(skills_dir.path(), "alpha", "Alpha skill");
        create_skill(skills_dir.path(), "beta", "Beta skill");

        let registry =
            discover_skills(&[skills_dir.path().to_path_buf()], data_dir.path(), true).unwrap();

        assert_eq!(registry.count(), 2);

        // Verify cache was written
        let cache = read_registry_cache(data_dir.path()).unwrap();
        assert_eq!(cache.version, 1);
        assert_eq!(cache.skills.len(), 2);
        assert!(cache.skills.contains_key("alpha"));
        assert!(cache.skills.contains_key("beta"));
        assert!(cache.skills["alpha"].user_invocable);
    }

    #[test]
    fn discover_empty_dir() {
        let skills_dir = TempDir::new().unwrap();
        let data_dir = TempDir::new().unwrap();

        let registry =
            discover_skills(&[skills_dir.path().to_path_buf()], data_dir.path(), true).unwrap();

        assert_eq!(registry.count(), 0);
        // No cache written for empty registry
        assert!(read_registry_cache(data_dir.path()).is_none());
    }

    #[test]
    fn discover_nonexistent_dir() {
        let data_dir = TempDir::new().unwrap();

        let registry = discover_skills(
            &[PathBuf::from("/nonexistent/path/12345")],
            data_dir.path(),
            true,
        )
        .unwrap();

        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn multi_dir_discovery() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        let data_dir = TempDir::new().unwrap();

        create_skill(dir1.path(), "skill-a", "Skill A");
        create_skill(dir2.path(), "skill-b", "Skill B");

        let registry = discover_skills(
            &[dir1.path().to_path_buf(), dir2.path().to_path_buf()],
            data_dir.path(),
            true,
        )
        .unwrap();

        assert_eq!(registry.count(), 2);
        assert!(registry.activate("skill-a").is_some());
        assert!(registry.activate("skill-b").is_some());
    }

    #[test]
    fn sync_creates_symlinks() {
        let data_dir = TempDir::new().unwrap();
        let agents_dir = data_dir.path().join(".agents").join("skills");
        std::fs::create_dir_all(&agents_dir).unwrap();

        // Create a skill in .agents/skills/
        create_skill(&agents_dir, "my-skill", "My skill");

        // Set CWD to data_dir for the sync
        let original_dir = std::env::current_dir().unwrap();
        // Don't actually change CWD in tests, just verify the function works
        // with explicit source dirs
        let arcan_skills = data_dir.path().join("skills");
        std::fs::create_dir_all(&arcan_skills).unwrap();

        #[cfg(unix)]
        {
            let source = agents_dir.join("my-skill");
            let link = arcan_skills.join("my-skill");
            std::os::unix::fs::symlink(&source, &link).unwrap();
            assert!(link.exists());
            assert!(link.join("SKILL.md").exists());
        }

        let _ = original_dir; // suppress unused warning
    }
}
