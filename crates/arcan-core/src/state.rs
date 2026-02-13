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
