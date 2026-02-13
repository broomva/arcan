use crate::protocol::{StatePatch, StatePatchFormat};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

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
