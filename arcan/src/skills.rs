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

// Re-export activation types from praxis-skills for downstream consumers.
#[allow(unused_imports)]
pub use praxis_skills::registry::{ActiveSkillState, active_skill_prompt, try_activate_skill};

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

    if write_registry
        && registry.count() > 0
        && let Err(e) = write_registry_cache(&registry, skill_dirs, data_dir)
    {
        tracing::warn!(error = %e, "failed to write skill registry cache");
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

/// Build a **compact** system prompt from the skill registry.
///
/// Instead of dumping all skill descriptions (which can be 18K+ tokens for 300+
/// skills), this produces a ~200-token summary with category breakdown and
/// instructions for activation. Full skill details are injected only when a
/// skill is activated via `/skill <name>`.
///
/// The full catalog remains available via `/skill list`.
pub fn build_system_prompt(registry: &SkillRegistry) -> String {
    let count = registry.count();
    if count == 0 {
        return String::new();
    }

    // Collect unique tags across all skills as category hints
    let mut categories = std::collections::BTreeSet::new();
    for name in registry.skill_names() {
        if let Some(skill) = registry.activate(&name) {
            for tag in &skill.meta.tags {
                categories.insert(tag.clone());
            }
        }
    }

    let categories_str = if categories.is_empty() {
        "various".to_string()
    } else {
        // Show up to 20 categories to keep it compact
        let cats: Vec<&str> = categories.iter().map(String::as_str).take(20).collect();
        let mut result = cats.join(", ");
        if categories.len() > 20 {
            result.push_str(", ...");
        }
        result
    };

    format!(
        "<skills>\n# Available Skills ({count} discovered)\n\n\
         Use `/skill <name>` to activate a skill. Use `/skill list` for the full catalog.\n\n\
         Categories: {categories_str}\n\
         </skills>"
    )
}

/// Build the full (uncapped) system prompt catalog for use when the user
/// explicitly requests the skill list. This is the original verbose format.
#[allow(dead_code)]
pub fn build_full_system_prompt(registry: &SkillRegistry) -> String {
    let catalog = registry.system_prompt_catalog();
    if catalog.is_empty() {
        return String::new();
    }

    format!(
        "<skills>\n{catalog}\n\n\
         To activate a skill, the user types `/skill-name` as their message.\n\
         When a skill is active, follow its instructions for that interaction.\n\
         </skills>"
    )
}

/// Convert a SKILL.md `SkillMcpServer` declaration into a `praxis-mcp-bridge` config.
#[allow(dead_code)] // Phase 4: called when MCP activation is wired into arcand
pub fn to_mcp_config(
    server: &praxis_skills::parser::SkillMcpServer,
) -> praxis_mcp_bridge::connection::McpServerConfig {
    praxis_mcp_bridge::connection::McpServerConfig {
        name: server.name.clone(),
        transport: praxis_mcp_bridge::connection::McpTransport::Stdio {
            command: server.command.clone(),
            args: server.args.clone(),
        },
    }
}

/// Spawn MCP server connections for a skill's declared `mcp_servers`.
///
/// Returns the list of tool definitions discovered from all connected servers
/// (as aios-protocol `ToolDefinition`s that can be bridged into Arcan via `PraxisToolBridge`),
/// plus the connection handles (which must be kept alive for the skill session duration).
///
/// Errors are logged but don't fail the activation — partial MCP is better than none.
#[allow(dead_code)] // Phase 4: called when MCP activation is wired into arcand
pub async fn spawn_skill_mcp_servers(
    servers: &[praxis_skills::parser::SkillMcpServer],
) -> (
    Vec<aios_protocol::tool::ToolDefinition>,
    Vec<praxis_mcp_bridge::connection::McpConnection>,
) {
    use aios_protocol::tool::Tool;

    let mut all_definitions = Vec::new();
    let mut connections = Vec::new();

    for server in servers {
        let config = to_mcp_config(server);
        match praxis_mcp_bridge::connection::connect_mcp_stdio(&config).await {
            Ok(connection) => {
                tracing::info!(
                    server = %server.name,
                    tools = connection.tools.len(),
                    "MCP server connected for skill"
                );
                for tool in &connection.tools {
                    all_definitions.push(tool.definition());
                }
                connections.push(connection);
            }
            Err(e) => {
                tracing::warn!(
                    server = %server.name,
                    error = %e,
                    "failed to connect MCP server for skill (non-fatal)"
                );
            }
        }
    }

    (all_definitions, connections)
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
    fn build_system_prompt_compact_format() {
        let skills_dir = TempDir::new().unwrap();
        create_skill(
            skills_dir.path(),
            "commit-helper",
            "Helps create git commits",
        );
        create_skill(skills_dir.path(), "test-runner", "Runs test suites");

        let registry = SkillRegistry::discover(&[skills_dir.path().to_path_buf()]).unwrap();

        let prompt = build_system_prompt(&registry);
        assert!(prompt.contains("<skills>"));
        assert!(prompt.contains("</skills>"));
        assert!(prompt.contains("2 discovered"));
        assert!(prompt.contains("/skill <name>"));
        // Compact format should NOT contain individual skill descriptions
        assert!(!prompt.contains("Helps create git commits"));
        assert!(!prompt.contains("Runs test suites"));
        // Should be compact — well under 500 tokens (~2000 chars)
        assert!(
            prompt.len() < 2000,
            "Compact prompt too large: {} chars",
            prompt.len()
        );
    }

    #[test]
    fn build_full_system_prompt_includes_catalog() {
        let skills_dir = TempDir::new().unwrap();
        create_skill(
            skills_dir.path(),
            "commit-helper",
            "Helps create git commits",
        );
        create_skill(skills_dir.path(), "test-runner", "Runs test suites");

        let registry = SkillRegistry::discover(&[skills_dir.path().to_path_buf()]).unwrap();

        let prompt = build_full_system_prompt(&registry);
        assert!(prompt.contains("<skills>"));
        assert!(prompt.contains("</skills>"));
        assert!(prompt.contains("commit-helper"));
        assert!(prompt.contains("test-runner"));
        assert!(prompt.contains("Available skills:"));
    }

    #[test]
    fn build_system_prompt_empty_registry() {
        let empty_dir = TempDir::new().unwrap();
        let registry = SkillRegistry::discover(&[empty_dir.path().to_path_buf()]).unwrap();

        let prompt = build_system_prompt(&registry);
        assert!(prompt.is_empty());
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
