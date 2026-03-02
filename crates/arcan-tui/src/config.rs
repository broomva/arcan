//! Minimal config persistence for the TUI.
//!
//! Reads and writes `~/.config/arcan/config.toml` using the same TOML schema
//! as the CLI binary crate, so both share the same config file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level TOML configuration (mirrors the binary crate's `ArcanConfig`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ArcanConfig {
    pub defaults: DefaultsConfig,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub providers: HashMap<String, ProviderConfig>,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub max_tokens: Option<u32>,
    pub enable_streaming: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    pub max_iterations: Option<u32>,
    pub approval_timeout: Option<u64>,
}

impl ArcanConfig {
    /// Set a config key using dotted notation.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "provider" | "defaults.provider" => {
                self.defaults.provider = Some(value.to_owned());
            }
            "model" | "defaults.model" => {
                self.defaults.model = Some(value.to_owned());
            }
            _ => return Err(format!("unknown config key: {key}")),
        }
        Ok(())
    }
}

/// Return the path to `~/.config/arcan/config.toml`, if determinable.
pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("arcan").join("config.toml"))
}

/// Load the global config file. Returns default config if the file is missing or invalid.
pub fn load_global_config() -> ArcanConfig {
    global_config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save config to the global config file.
pub fn save_global_config(config: &ArcanConfig) -> Result<(), String> {
    let path = global_config_path().ok_or("could not determine config directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create config directory: {e}"))?;
    }
    let content =
        toml::to_string_pretty(config).map_err(|e| format!("failed to serialize config: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("failed to write config: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_key_provider() {
        let mut config = ArcanConfig::default();
        config.set_key("provider", "openai").unwrap();
        assert_eq!(config.defaults.provider.as_deref(), Some("openai"));
    }

    #[test]
    fn set_key_unknown_errors() {
        let mut config = ArcanConfig::default();
        assert!(config.set_key("unknown", "value").is_err());
    }

    #[test]
    fn roundtrip_serialization() {
        let mut config = ArcanConfig::default();
        config.defaults.provider = Some("openai".into());
        config.defaults.model = Some("gpt-4o".into());

        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: ArcanConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.defaults.provider.as_deref(), Some("openai"));
        assert_eq!(parsed.defaults.model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn global_config_path_exists() {
        // Should return Some on most platforms
        let path = global_config_path();
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.ends_with("arcan/config.toml") || p.ends_with("arcan\\config.toml"));
    }
}
