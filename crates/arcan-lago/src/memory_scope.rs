use lago_core::MemoryId;
use lago_core::event::MemoryScope;
use serde::{Deserialize, Serialize};

/// Runtime configuration for memory scopes.
#[derive(Debug, Clone)]
pub struct MemoryScopeConfig {
    /// Which scopes are enabled for this agent.
    pub enabled_scopes: Vec<MemoryScope>,
    /// Number of observations before triggering compaction.
    pub observation_threshold: usize,
    /// Token budget per scope for context inclusion.
    pub per_scope_token_budget: usize,
}

impl Default for MemoryScopeConfig {
    fn default() -> Self {
        Self {
            enabled_scopes: vec![MemoryScope::Session, MemoryScope::User],
            observation_threshold: 20,
            per_scope_token_budget: 4_000,
        }
    }
}

/// A committed memory entry with provenance metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub memory_id: MemoryId,
    pub scope: MemoryScope,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    #[serde(default)]
    pub source_event_ids: Vec<String>,
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_reasonable() {
        let config = MemoryScopeConfig::default();
        assert_eq!(config.enabled_scopes.len(), 2);
        assert!(config.enabled_scopes.contains(&MemoryScope::Session));
        assert!(config.enabled_scopes.contains(&MemoryScope::User));
        assert_eq!(config.observation_threshold, 20);
        assert_eq!(config.per_scope_token_budget, 4_000);
    }

    #[test]
    fn memory_entry_serde_roundtrip() {
        let entry = MemoryEntry {
            memory_id: MemoryId::from_string("MEM001"),
            scope: MemoryScope::Session,
            content: "User prefers dark mode".to_string(),
            source_run_id: Some("run-1".to_string()),
            source_event_ids: vec!["evt-1".to_string(), "evt-2".to_string()],
            created_at: 1_700_000_000,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: MemoryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.memory_id.as_str(), "MEM001");
        assert_eq!(back.scope, MemoryScope::Session);
        assert_eq!(back.content, "User prefers dark mode");
        assert_eq!(back.source_run_id.as_deref(), Some("run-1"));
        assert_eq!(back.source_event_ids.len(), 2);
    }

    #[test]
    fn custom_config() {
        let config = MemoryScopeConfig {
            enabled_scopes: vec![MemoryScope::Agent, MemoryScope::Org],
            observation_threshold: 50,
            per_scope_token_budget: 8_000,
        };
        assert_eq!(config.enabled_scopes.len(), 2);
        assert_eq!(config.observation_threshold, 50);
    }
}
