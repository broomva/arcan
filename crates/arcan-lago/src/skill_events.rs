//! Skill lifecycle events for the Lago event journal.
//!
//! Tracks skill discovery, activation, and deactivation as `EventKind::Custom`
//! events with `"skill."` prefix, following the same pattern used by
//! Autonomic (`"autonomic."`) and Haima (`"finance."`).

use lago_core::{BranchId, EventEnvelope, EventId, EventPayload, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

/// Event types emitted by the skill system.
pub mod event_types {
    /// Emitted on startup when skills are discovered from directories.
    pub const SKILL_DISCOVERED: &str = "skill.discovered";
    /// Emitted when a user activates a skill via `/skill-name`.
    pub const SKILL_ACTIVATED: &str = "skill.activated";
    /// Emitted when a skill session ends (deactivation).
    pub const SKILL_DEACTIVATED: &str = "skill.deactivated";
    /// Emitted when an MCP server is connected for a skill.
    pub const SKILL_MCP_CONNECTED: &str = "skill.mcp_connected";
    /// Emitted when an MCP server is disconnected for a skill.
    pub const SKILL_MCP_DISCONNECTED: &str = "skill.mcp_disconnected";
}

/// Data for a `skill.discovered` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDiscoveredData {
    pub count: usize,
    pub dirs: Vec<String>,
}

/// Data for a `skill.activated` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillActivatedData {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<String>>,
}

/// Data for a `skill.deactivated` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDeactivatedData {
    pub name: String,
    pub duration_ms: u64,
    pub reason: String,
}

/// Data for `skill.mcp_connected` / `skill.mcp_disconnected` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMcpData {
    pub skill_name: String,
    pub server_name: String,
    pub tools_count: usize,
}

/// Build a Lago `EventEnvelope` for a skill event.
///
/// Uses `EventPayload::Custom` with a `"skill."` namespace prefix.
fn build_skill_event(
    session_id: &SessionId,
    branch_id: &BranchId,
    event_type: &str,
    data: serde_json::Value,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: None,
        seq: 0, // Auto-assigned by journal
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload: EventPayload::Custom {
            event_type: event_type.to_string(),
            data,
        },
        metadata: HashMap::new(),
        schema_version: 1,
    }
}

/// Emit a `skill.discovered` event to the journal.
pub async fn emit_skill_discovered(
    journal: &dyn lago_core::Journal,
    session_id: &SessionId,
    branch_id: &BranchId,
    count: usize,
    dirs: &[std::path::PathBuf],
) -> Result<(), lago_core::LagoError> {
    let data = SkillDiscoveredData {
        count,
        dirs: dirs.iter().map(|d| d.display().to_string()).collect(),
    };
    let event = build_skill_event(
        session_id,
        branch_id,
        event_types::SKILL_DISCOVERED,
        json!(data),
    );
    journal.append(event).await?;
    Ok(())
}

/// Emit a `skill.activated` event to the journal.
pub async fn emit_skill_activated(
    journal: &dyn lago_core::Journal,
    session_id: &SessionId,
    branch_id: &BranchId,
    name: &str,
    tags: &[String],
    allowed_tools: Option<&[String]>,
    mcp_server_names: Option<&[String]>,
) -> Result<(), lago_core::LagoError> {
    let data = SkillActivatedData {
        name: name.to_string(),
        tags: tags.to_vec(),
        allowed_tools: allowed_tools.map(<[String]>::to_vec),
        mcp_servers: mcp_server_names.map(<[String]>::to_vec),
    };
    let event = build_skill_event(
        session_id,
        branch_id,
        event_types::SKILL_ACTIVATED,
        json!(data),
    );
    journal.append(event).await?;
    Ok(())
}

/// Emit a `skill.deactivated` event to the journal.
pub async fn emit_skill_deactivated(
    journal: &dyn lago_core::Journal,
    session_id: &SessionId,
    branch_id: &BranchId,
    name: &str,
    duration_ms: u64,
    reason: &str,
) -> Result<(), lago_core::LagoError> {
    let data = SkillDeactivatedData {
        name: name.to_string(),
        duration_ms,
        reason: reason.to_string(),
    };
    let event = build_skill_event(
        session_id,
        branch_id,
        event_types::SKILL_DEACTIVATED,
        json!(data),
    );
    journal.append(event).await?;
    Ok(())
}

/// Emit a `skill.mcp_connected` event to the journal.
pub async fn emit_skill_mcp_connected(
    journal: &dyn lago_core::Journal,
    session_id: &SessionId,
    branch_id: &BranchId,
    skill_name: &str,
    server_name: &str,
    tools_count: usize,
) -> Result<(), lago_core::LagoError> {
    let data = SkillMcpData {
        skill_name: skill_name.to_string(),
        server_name: server_name.to_string(),
        tools_count,
    };
    let event = build_skill_event(
        session_id,
        branch_id,
        event_types::SKILL_MCP_CONNECTED,
        json!(data),
    );
    journal.append(event).await?;
    Ok(())
}

/// Projection that tracks skill activation analytics by folding events.
///
/// Follows the same pattern as `MemoryProjection` — stateless fold over events.
#[derive(Debug, Default, Clone)]
pub struct SkillProjection {
    /// Total activations per skill name.
    pub activations: HashMap<String, u64>,
    /// Last activation timestamp per skill.
    pub last_activated: HashMap<String, u64>,
    /// Total MCP connections established.
    pub mcp_connections: u64,
}

impl SkillProjection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold an event into the projection state.
    pub fn fold(&mut self, event: &EventEnvelope) {
        if let EventPayload::Custom {
            ref event_type,
            ref data,
        } = event.payload
        {
            match event_type.as_str() {
                event_types::SKILL_ACTIVATED => {
                    if let Some(name) = data.get("name").and_then(|v| v.as_str()) {
                        *self.activations.entry(name.to_string()).or_default() += 1;
                        self.last_activated
                            .insert(name.to_string(), event.timestamp);
                    }
                }
                event_types::SKILL_MCP_CONNECTED => {
                    self.mcp_connections += 1;
                }
                _ => {}
            }
        }
    }

    /// Most activated skill (name, count).
    pub fn most_activated(&self) -> Option<(&str, u64)> {
        self.activations
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(name, count)| (name.as_str(), *count))
    }
}

/// Ingest a SKILL.md file into Lago's blob store as a knowledge graph node.
///
/// This makes skills searchable via `/v1/memory/search` and traversable
/// via wikilink graph traversal. Each skill becomes a `Note` in the
/// knowledge index, with its frontmatter (tags, description) available
/// for scored search.
///
/// The content is stored in the blob store (SHA-256 + zstd), and the
/// blob hash is returned for manifest construction.
pub fn ingest_skill_to_blob_store(
    blob_store: &lago_store::BlobStore,
    _skill_name: &str,
    skill_content: &str,
) -> Result<lago_core::BlobHash, lago_core::LagoError> {
    blob_store.put(skill_content.as_bytes())
}

/// Build manifest entries for all discovered skills.
///
/// Each skill becomes a manifest entry pointing to its blob hash,
/// making it discoverable by the knowledge index's `build()` method.
pub fn skills_to_manifest_entries(
    blob_store: &lago_store::BlobStore,
    skills: &[(String, String)], // (name, full_content)
) -> Vec<(String, lago_core::BlobHash)> {
    let mut entries = Vec::new();
    for (name, content) in skills {
        match ingest_skill_to_blob_store(blob_store, name, content) {
            Ok(blob_hash) => {
                let path = format!("skills/{}/SKILL.md", name);
                entries.push((path, blob_hash));
                tracing::debug!(skill = %name, "ingested skill into blob store");
            }
            Err(e) => {
                tracing::warn!(
                    skill = %name,
                    error = %e,
                    "failed to ingest skill into blob store"
                );
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_custom_event(event_type: &str, data: serde_json::Value) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: SessionId::from_string("test"),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq: 1,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::Custom {
                event_type: event_type.to_string(),
                data,
            },
            metadata: HashMap::new(),
            schema_version: 1,
        }
    }

    #[test]
    fn skill_projection_tracks_activations() {
        let mut proj = SkillProjection::new();

        proj.fold(&make_custom_event(
            "skill.activated",
            json!({ "name": "commit-helper", "tags": ["git"] }),
        ));
        proj.fold(&make_custom_event(
            "skill.activated",
            json!({ "name": "commit-helper", "tags": ["git"] }),
        ));
        proj.fold(&make_custom_event(
            "skill.activated",
            json!({ "name": "test-runner", "tags": ["testing"] }),
        ));

        assert_eq!(proj.activations["commit-helper"], 2);
        assert_eq!(proj.activations["test-runner"], 1);
        assert_eq!(proj.most_activated(), Some(("commit-helper", 2)));
    }

    #[test]
    fn skill_projection_tracks_mcp() {
        let mut proj = SkillProjection::new();

        proj.fold(&make_custom_event(
            "skill.mcp_connected",
            json!({ "skill_name": "db-admin", "server_name": "postgres", "tools_count": 3 }),
        ));

        assert_eq!(proj.mcp_connections, 1);
    }

    #[test]
    fn skill_projection_ignores_unrelated_events() {
        let mut proj = SkillProjection::new();

        proj.fold(&make_custom_event(
            "autonomic.mode_changed",
            json!({ "mode": "Sovereign" }),
        ));

        assert!(proj.activations.is_empty());
        assert_eq!(proj.mcp_connections, 0);
    }

    #[test]
    fn skill_event_data_serialization() {
        let data = SkillActivatedData {
            name: "test".to_string(),
            tags: vec!["a".to_string()],
            allowed_tools: Some(vec!["read_file".to_string()]),
            mcp_servers: None,
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["name"], "test");
        assert_eq!(json["tags"], json!(["a"]));
        assert_eq!(json["allowed_tools"], json!(["read_file"]));
        assert!(json.get("mcp_servers").is_none());
    }

    #[test]
    fn build_skill_event_structure() {
        let event = build_skill_event(
            &SessionId::from_string("sess-1"),
            &BranchId::from_string("main"),
            event_types::SKILL_ACTIVATED,
            json!({ "name": "test" }),
        );

        assert_eq!(event.session_id.as_str(), "sess-1");
        assert_eq!(event.branch_id.as_str(), "main");
        assert!(event.schema_version == 1);
        if let EventPayload::Custom {
            event_type, data, ..
        } = &event.payload
        {
            assert_eq!(event_type, "skill.activated");
            assert_eq!(data["name"], "test");
        } else {
            panic!("expected Custom payload");
        }
    }
}
