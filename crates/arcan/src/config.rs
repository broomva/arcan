//! Persistent CLI configuration with layered resolution.
//!
//! Resolution order: hardcoded defaults → global config → project-local config → env vars → CLI flags.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level TOML configuration file structure.
///
/// All fields use `Option<T>` to allow partial configs and clean merging.
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

/// Fully resolved configuration with concrete values (no Options).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedConfig {
    pub provider: String,
    pub model: Option<String>,
    pub port: u16,
    pub max_iterations: u32,
    pub approval_timeout: u64,
    pub provider_config: Option<ProviderConfig>,
}

impl ArcanConfig {
    /// Merge `other` on top of `self`. Non-None values in `other` win.
    pub fn merge(&mut self, other: &ArcanConfig) {
        if other.defaults.provider.is_some() {
            self.defaults.provider.clone_from(&other.defaults.provider);
        }
        if other.defaults.model.is_some() {
            self.defaults.model.clone_from(&other.defaults.model);
        }
        if other.defaults.port.is_some() {
            self.defaults.port = other.defaults.port;
        }
        if other.agent.max_iterations.is_some() {
            self.agent.max_iterations = other.agent.max_iterations;
        }
        if other.agent.approval_timeout.is_some() {
            self.agent.approval_timeout = other.agent.approval_timeout;
        }
        for (name, pc) in &other.providers {
            let entry = self.providers.entry(name.clone()).or_default();
            if pc.model.is_some() {
                entry.model.clone_from(&pc.model);
            }
            if pc.base_url.is_some() {
                entry.base_url.clone_from(&pc.base_url);
            }
            if pc.max_tokens.is_some() {
                entry.max_tokens = pc.max_tokens;
            }
            if pc.enable_streaming.is_some() {
                entry.enable_streaming = pc.enable_streaming;
            }
        }
    }

    /// Set a key using dotted notation. Shortcut keys:
    /// `provider` → `defaults.provider`, `model` → `defaults.model`, `port` → `defaults.port`.
    pub fn set_key(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "provider" | "defaults.provider" => {
                self.defaults.provider = Some(value.to_owned());
            }
            "model" | "defaults.model" => {
                self.defaults.model = Some(value.to_owned());
            }
            "port" | "defaults.port" => {
                let port: u16 = value
                    .parse()
                    .map_err(|e| format!("invalid port value: {e}"))?;
                self.defaults.port = Some(port);
            }
            "agent.max_iterations" | "max_iterations" => {
                let v: u32 = value
                    .parse()
                    .map_err(|e| format!("invalid max_iterations: {e}"))?;
                self.agent.max_iterations = Some(v);
            }
            "agent.approval_timeout" | "approval_timeout" => {
                let v: u64 = value
                    .parse()
                    .map_err(|e| format!("invalid approval_timeout: {e}"))?;
                self.agent.approval_timeout = Some(v);
            }
            _ if key.starts_with("providers.") => {
                // e.g. providers.ollama.base_url
                let rest = &key["providers.".len()..];
                let parts: Vec<&str> = rest.splitn(2, '.').collect();
                if parts.len() != 2 {
                    return Err(format!(
                        "invalid provider key: {key} (expected providers.<name>.<field>)"
                    ));
                }
                let provider_name = parts[0];
                let field = parts[1];
                let entry = self.providers.entry(provider_name.to_owned()).or_default();
                match field {
                    "model" => entry.model = Some(value.to_owned()),
                    "base_url" => entry.base_url = Some(value.to_owned()),
                    "max_tokens" => {
                        let v: u32 = value
                            .parse()
                            .map_err(|e| format!("invalid max_tokens: {e}"))?;
                        entry.max_tokens = Some(v);
                    }
                    "enable_streaming" => {
                        let v: bool = value
                            .parse()
                            .map_err(|e| format!("invalid enable_streaming: {e}"))?;
                        entry.enable_streaming = Some(v);
                    }
                    _ => return Err(format!("unknown provider field: {field}")),
                }
            }
            _ => return Err(format!("unknown config key: {key}")),
        }
        Ok(())
    }

    /// Get a value by key. Returns None if unset.
    pub fn get_key(&self, key: &str) -> Option<String> {
        match key {
            "provider" | "defaults.provider" => self.defaults.provider.clone(),
            "model" | "defaults.model" => self.defaults.model.clone(),
            "port" | "defaults.port" => self.defaults.port.map(|p| p.to_string()),
            "agent.max_iterations" | "max_iterations" => {
                self.agent.max_iterations.map(|v| v.to_string())
            }
            "agent.approval_timeout" | "approval_timeout" => {
                self.agent.approval_timeout.map(|v| v.to_string())
            }
            _ if key.starts_with("providers.") => {
                let rest = &key["providers.".len()..];
                let parts: Vec<&str> = rest.splitn(2, '.').collect();
                if parts.len() != 2 {
                    return None;
                }
                let pc = self.providers.get(parts[0])?;
                match parts[1] {
                    "model" => pc.model.clone(),
                    "base_url" => pc.base_url.clone(),
                    "max_tokens" => pc.max_tokens.map(|v| v.to_string()),
                    "enable_streaming" => pc.enable_streaming.map(|v| v.to_string()),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// Global config path: `~/.config/arcan/config.toml`
pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("arcan").join("config.toml"))
}

/// Project-local config path: `<data_dir>/config.toml`
pub fn local_config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.toml")
}

/// Load and merge config from global + local files.
pub fn load_config(data_dir: &Path) -> ArcanConfig {
    let mut config = ArcanConfig::default();

    // Layer 1: global config
    if let Some(global_path) = global_config_path() {
        if let Some(global) = load_config_file(&global_path) {
            config.merge(&global);
        }
    }

    // Layer 2: project-local config
    let local_path = local_config_path(data_dir);
    if let Some(local) = load_config_file(&local_path) {
        config.merge(&local);
    }

    config
}

/// Load a single TOML config file. Returns None if file doesn't exist or is invalid.
fn load_config_file(path: &Path) -> Option<ArcanConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// Save config to the project-local config file.
pub fn save_config(data_dir: &Path, config: &ArcanConfig) -> anyhow::Result<()> {
    let path = local_config_path(data_dir);
    std::fs::create_dir_all(data_dir)?;
    let content = toml::to_string_pretty(config)
        .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Resolve the final config by applying env vars and CLI overrides on top.
pub fn resolve(
    config: &ArcanConfig,
    cli_provider: Option<&str>,
    cli_model: Option<&str>,
    cli_port: Option<u16>,
    cli_max_iterations: Option<u32>,
    cli_approval_timeout: Option<u64>,
) -> ResolvedConfig {
    // Provider: CLI > env > config > ""
    let provider = cli_provider
        .map(String::from)
        .or_else(|| std::env::var("ARCAN_PROVIDER").ok())
        .or_else(|| config.defaults.provider.clone())
        .unwrap_or_default();

    // Model: CLI > env > provider-specific config > defaults config > None
    let model = cli_model
        .map(String::from)
        .or_else(|| std::env::var("ARCAN_MODEL").ok())
        .or_else(|| {
            config
                .providers
                .get(&provider)
                .and_then(|pc| pc.model.clone())
        })
        .or_else(|| config.defaults.model.clone());

    // Port: CLI > env > config > 3000
    let port = cli_port
        .or_else(|| {
            std::env::var("ARCAN_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .or(config.defaults.port)
        .unwrap_or(3000);

    // Max iterations: CLI > config > 10
    let max_iterations = cli_max_iterations
        .or(config.agent.max_iterations)
        .unwrap_or(10);

    // Approval timeout: CLI > config > 300
    let approval_timeout = cli_approval_timeout
        .or(config.agent.approval_timeout)
        .unwrap_or(300);

    // Provider-specific config section
    let provider_config = config.providers.get(&provider).cloned();

    ResolvedConfig {
        provider,
        model,
        port,
        max_iterations,
        approval_timeout,
        provider_config,
    }
}

/// Generate default config TOML content.
pub fn default_config_content() -> String {
    r#"# Arcan CLI Configuration
# Precedence: defaults < config file < env vars < CLI flags

[defaults]
# provider = "anthropic"  # anthropic, openai, ollama, mock
# model = "claude-sonnet-4-5-20250929"
# port = 3000

[agent]
# max_iterations = 10
# approval_timeout = 300

# [providers.anthropic]
# model = "claude-sonnet-4-5-20250929"
# max_tokens = 4096

# [providers.ollama]
# model = "llama3.2"
# base_url = "http://localhost:11434"
# max_tokens = 4096
# enable_streaming = true

# [providers.openai]
# model = "gpt-4o"
# max_tokens = 4096
"#
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_overrides_non_none() {
        let mut base = ArcanConfig::default();
        base.defaults.provider = Some("mock".into());
        base.defaults.port = Some(3000);

        let overlay = ArcanConfig {
            defaults: DefaultsConfig {
                provider: Some("ollama".into()),
                model: Some("llama3.2".into()),
                port: None,
            },
            ..Default::default()
        };

        base.merge(&overlay);
        assert_eq!(base.defaults.provider.as_deref(), Some("ollama"));
        assert_eq!(base.defaults.model.as_deref(), Some("llama3.2"));
        assert_eq!(base.defaults.port, Some(3000)); // preserved
    }

    #[test]
    fn set_and_get_shortcut_keys() {
        let mut config = ArcanConfig::default();
        config.set_key("provider", "ollama").unwrap();
        config.set_key("model", "gpt-oss:20b").unwrap();
        config.set_key("port", "3001").unwrap();

        assert_eq!(config.get_key("provider").as_deref(), Some("ollama"));
        assert_eq!(config.get_key("model").as_deref(), Some("gpt-oss:20b"));
        assert_eq!(config.get_key("port").as_deref(), Some("3001"));
    }

    #[test]
    fn set_and_get_dotted_provider_keys() {
        let mut config = ArcanConfig::default();
        config
            .set_key("providers.ollama.base_url", "http://localhost:11434")
            .unwrap();
        config
            .set_key("providers.ollama.max_tokens", "8192")
            .unwrap();
        config
            .set_key("providers.ollama.enable_streaming", "true")
            .unwrap();

        assert_eq!(
            config.get_key("providers.ollama.base_url").as_deref(),
            Some("http://localhost:11434")
        );
        assert_eq!(
            config.get_key("providers.ollama.max_tokens").as_deref(),
            Some("8192")
        );
        assert_eq!(
            config
                .get_key("providers.ollama.enable_streaming")
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn set_key_rejects_invalid() {
        let mut config = ArcanConfig::default();
        assert!(config.set_key("port", "not-a-number").is_err());
        assert!(config.set_key("unknown_key", "value").is_err());
        assert!(config.set_key("providers.ollama.unknown", "v").is_err());
    }

    #[test]
    fn get_key_returns_none_for_unset() {
        let config = ArcanConfig::default();
        assert!(config.get_key("provider").is_none());
        assert!(config.get_key("providers.ollama.model").is_none());
    }

    #[test]
    fn resolve_defaults() {
        let config = ArcanConfig::default();
        let resolved = resolve(&config, None, None, None, None, None);
        assert_eq!(resolved.provider, "");
        assert!(resolved.model.is_none());
        assert_eq!(resolved.port, 3000);
        assert_eq!(resolved.max_iterations, 10);
        assert_eq!(resolved.approval_timeout, 300);
    }

    #[test]
    fn resolve_cli_overrides_config() {
        let mut config = ArcanConfig::default();
        config.defaults.provider = Some("ollama".into());
        config.defaults.model = Some("llama3.2".into());
        config.defaults.port = Some(3001);

        let resolved = resolve(
            &config,
            Some("anthropic"),
            Some("claude-3"),
            Some(4000),
            None,
            None,
        );
        assert_eq!(resolved.provider, "anthropic");
        assert_eq!(resolved.model.as_deref(), Some("claude-3"));
        assert_eq!(resolved.port, 4000);
    }

    #[test]
    fn resolve_uses_provider_specific_model() {
        let mut config = ArcanConfig::default();
        config.defaults.provider = Some("ollama".into());
        let mut pc = ProviderConfig::default();
        pc.model = Some("special-model".into());
        config.providers.insert("ollama".into(), pc);

        let resolved = resolve(&config, None, None, None, None, None);
        assert_eq!(resolved.model.as_deref(), Some("special-model"));
    }

    #[test]
    fn roundtrip_toml_serialization() {
        let mut config = ArcanConfig::default();
        config.defaults.provider = Some("ollama".into());
        config.defaults.port = Some(3001);
        let mut pc = ProviderConfig::default();
        pc.model = Some("llama3.2".into());
        config.providers.insert("ollama".into(), pc);

        let toml_str = toml::to_string_pretty(&config).expect("serialize");
        let parsed: ArcanConfig = toml::from_str(&toml_str).expect("parse");
        assert_eq!(parsed.defaults.provider.as_deref(), Some("ollama"));
        assert_eq!(parsed.defaults.port, Some(3001));
        assert_eq!(
            parsed
                .providers
                .get("ollama")
                .and_then(|p| p.model.as_deref()),
            Some("llama3.2")
        );
    }

    #[test]
    fn save_and_load_config_file() {
        let dir = std::env::temp_dir().join(format!(
            "arcan-config-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let mut config = ArcanConfig::default();
        config.set_key("provider", "ollama").unwrap();
        config.set_key("model", "test-model").unwrap();
        save_config(&dir, &config).unwrap();

        let loaded = load_config(&dir);
        assert_eq!(loaded.defaults.provider.as_deref(), Some("ollama"));
        assert_eq!(loaded.defaults.model.as_deref(), Some("test-model"));

        let _ = std::fs::remove_dir_all(dir);
    }
}
