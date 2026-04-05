use crate::protocol::{StatePatch, StatePatchFormat};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Well-known keys in `AppState.data`. JSON Patch still operates on the raw `Value`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(default)]
pub struct WellKnownState {
    /// Current working directory for the session.
    pub cwd: Option<String>,
    /// Active file paths the agent is aware of.
    pub open_files: Option<Vec<String>>,
    /// Session-level metadata.
    pub session_meta: Option<SessionMeta>,
    /// Tool execution budget tracking.
    pub budget: Option<BudgetState>,
    /// Currently loaded skill names.
    pub active_skills: Option<Vec<String>>,
    /// Connected MCP server info.
    pub mcp_servers: Option<Vec<McpServerInfo>>,
}

/// Session-level metadata stored in agent state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SessionMeta {
    pub session_name: Option<String>,
    pub user_id: Option<String>,
    pub created_at: Option<String>,
}

/// Tool execution budget tracking.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct BudgetState {
    pub total_tokens_used: u64,
    pub max_tokens_budget: Option<u64>,
    pub tool_calls_count: u32,
    pub max_tool_calls: Option<u32>,
}

/// Info about a connected MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct McpServerInfo {
    pub name: String,
    /// Transport type: "stdio" or "http".
    pub transport: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct AppState {
    pub revision: u64,
    #[serde(default)]
    pub data: Value,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            revision: 0,
            data: Value::Object(Default::default()),
        }
    }
}

impl AppState {
    pub fn new(data: Value) -> Self {
        Self { revision: 0, data }
    }

    /// Parse well-known keys from the raw JSON data.
    pub fn well_known(&self) -> Result<WellKnownState, serde_json::Error> {
        serde_json::from_value(self.data.clone())
    }

    /// Get the current working directory from state.
    pub fn cwd(&self) -> Option<String> {
        self.data
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Get the list of open files from state.
    pub fn open_files(&self) -> Vec<String> {
        self.data
            .get("open_files")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    pub fn apply_patch(&mut self, patch: &StatePatch) -> Result<(), StateError> {
        match patch.format {
            StatePatchFormat::JsonPatch => {
                let parsed_patch: json_patch::Patch = serde_json::from_value(patch.patch.clone())
                    .map_err(StateError::InvalidJsonPatch)?;
                json_patch::patch(&mut self.data, &parsed_patch)
                    .map_err(|e| StateError::PatchApply(e.to_string()))?;
            }
            StatePatchFormat::MergePatch => {
                json_patch::merge(&mut self.data, &patch.patch);
            }
        }

        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("invalid JSON patch payload: {0}")]
    InvalidJsonPatch(serde_json::Error),
    #[error("failed to apply patch: {0}")]
    PatchApply(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::StatePatchSource;
    use serde_json::json;

    #[test]
    fn default_state_is_empty_object() {
        let state = AppState::default();
        assert_eq!(state.revision, 0);
        assert_eq!(state.data, json!({}));
    }

    #[test]
    fn merge_patch_adds_fields() {
        let mut state = AppState::default();
        let patch = StatePatch {
            format: StatePatchFormat::MergePatch,
            patch: json!({"name": "arcan", "version": 1}),
            source: StatePatchSource::System,
        };
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.revision, 1);
        assert_eq!(state.data["name"], "arcan");
        assert_eq!(state.data["version"], 1);
    }

    #[test]
    fn merge_patch_overwrites_fields() {
        let mut state = AppState::new(json!({"count": 0}));
        let patch = StatePatch {
            format: StatePatchFormat::MergePatch,
            patch: json!({"count": 42}),
            source: StatePatchSource::Tool,
        };
        state.apply_patch(&patch).unwrap();
        assert_eq!(state.data["count"], 42);
    }

    #[test]
    fn merge_patch_removes_null_fields() {
        let mut state = AppState::new(json!({"a": 1, "b": 2}));
        let patch = StatePatch {
            format: StatePatchFormat::MergePatch,
            patch: json!({"b": null}),
            source: StatePatchSource::Model,
        };
        state.apply_patch(&patch).unwrap();
        assert_eq!(state.data, json!({"a": 1}));
    }

    #[test]
    fn json_patch_add_operation() {
        let mut state = AppState::default();
        let patch = StatePatch {
            format: StatePatchFormat::JsonPatch,
            patch: json!([{"op": "add", "path": "/foo", "value": "bar"}]),
            source: StatePatchSource::System,
        };
        state.apply_patch(&patch).unwrap();
        assert_eq!(state.data["foo"], "bar");
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn json_patch_replace_operation() {
        let mut state = AppState::new(json!({"x": 10}));
        let patch = StatePatch {
            format: StatePatchFormat::JsonPatch,
            patch: json!([{"op": "replace", "path": "/x", "value": 20}]),
            source: StatePatchSource::System,
        };
        state.apply_patch(&patch).unwrap();
        assert_eq!(state.data["x"], 20);
    }

    #[test]
    fn json_patch_invalid_payload_errors() {
        let mut state = AppState::default();
        let patch = StatePatch {
            format: StatePatchFormat::JsonPatch,
            patch: json!("not an array"),
            source: StatePatchSource::System,
        };
        assert!(state.apply_patch(&patch).is_err());
        assert_eq!(state.revision, 0);
    }

    #[test]
    fn well_known_parses_populated_state() {
        let state = AppState::new(json!({
            "cwd": "/home/user",
            "open_files": ["main.rs", "lib.rs"],
            "budget": {
                "total_tokens_used": 1000,
                "max_tokens_budget": 10000,
                "tool_calls_count": 5,
                "max_tool_calls": 100
            }
        }));
        let wk = state.well_known().unwrap();
        assert_eq!(wk.cwd.as_deref(), Some("/home/user"));
        assert_eq!(
            wk.open_files,
            Some(vec!["main.rs".to_string(), "lib.rs".to_string()])
        );
        assert!(wk.budget.is_some());
        assert_eq!(wk.budget.unwrap().total_tokens_used, 1000);
    }

    #[test]
    fn well_known_defaults_for_empty_state() {
        let state = AppState::default();
        let wk = state.well_known().unwrap();
        assert_eq!(wk.cwd, None);
        assert_eq!(wk.open_files, None);
        assert_eq!(wk.session_meta, None);
        assert_eq!(wk.budget, None);
        assert_eq!(wk.active_skills, None);
        assert_eq!(wk.mcp_servers, None);
    }

    #[test]
    fn cwd_accessor() {
        let state = AppState::new(json!({"cwd": "/tmp"}));
        assert_eq!(state.cwd(), Some("/tmp".to_string()));

        let empty = AppState::default();
        assert_eq!(empty.cwd(), None);
    }

    #[test]
    fn open_files_accessor() {
        let state = AppState::new(json!({"open_files": ["a.rs", "b.rs"]}));
        assert_eq!(
            state.open_files(),
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );

        let empty = AppState::default();
        assert!(empty.open_files().is_empty());
    }

    #[test]
    fn json_patch_works_with_well_known_keys() {
        let mut state = AppState::new(json!({"cwd": "/old"}));
        let patch = StatePatch {
            format: StatePatchFormat::JsonPatch,
            patch: json!([{"op": "replace", "path": "/cwd", "value": "/new"}]),
            source: StatePatchSource::System,
        };
        state.apply_patch(&patch).unwrap();
        assert_eq!(state.cwd(), Some("/new".to_string()));
    }

    #[test]
    fn revision_increments_with_each_patch() {
        let mut state = AppState::default();
        for i in 1..=5 {
            let patch = StatePatch {
                format: StatePatchFormat::MergePatch,
                patch: json!({"step": i}),
                source: StatePatchSource::System,
            };
            state.apply_patch(&patch).unwrap();
            assert_eq!(state.revision, i as u64);
        }
    }
}
