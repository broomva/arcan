//! Hive coordination over Spaces channels.
//!
//! Uses existing `ChannelType::AgentLog` + `MessageType::AgentEvent` with
//! structured JSON content. Avoids heavyweight WASM module redeployment.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::SpacesBridgeError;
use crate::port::{SpacesMessage, SpacesPort};

/// A parsed hive artifact message from Spaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiveArtifactMessage {
    pub hive_task_id: String,
    pub session_id: String,
    pub score: f32,
    pub generation: u32,
    pub tldr: String,
}

/// Aggregated context for a hive task from Spaces messages.
#[derive(Debug, Clone, Default)]
pub struct HiveContext {
    pub artifacts: Vec<HiveArtifactMessage>,
    pub claims: Vec<String>,
    pub skills: Vec<String>,
}

/// Coordinator for hive multi-agent collaboration over Spaces.
///
/// Wraps a `SpacesPort` and provides hive-specific message conventions:
/// - One channel per hive task
/// - Threads separate generations (thread_id = generation number)
/// - JSON-structured content with `hive_event` discriminator
pub struct HiveSpacesCoordinator {
    spaces: Arc<dyn SpacesPort>,
    server_id: u64,
}

impl HiveSpacesCoordinator {
    pub fn new(spaces: Arc<dyn SpacesPort>, server_id: u64) -> Self {
        Self { spaces, server_id }
    }

    /// Find or identify the hive channel for a task.
    /// Returns the channel_id for the channel named `hive-{task_id}`.
    pub fn find_hive_channel(&self, hive_task_id: &str) -> Result<Option<u64>, SpacesBridgeError> {
        let channels = self.spaces.list_channels(self.server_id)?;
        let target_name = format!("hive-{hive_task_id}");
        Ok(channels
            .iter()
            .find(|c| c.name == target_name)
            .map(|c| c.id))
    }

    /// Share an artifact result to the hive channel.
    pub fn share_artifact(
        &self,
        channel_id: u64,
        generation: u32,
        hive_task_id: &str,
        session_id: &str,
        score: f32,
        tldr: &str,
    ) -> Result<SpacesMessage, SpacesBridgeError> {
        let content = serde_json::json!({
            "hive_event": "artifact_shared",
            "hive_task_id": hive_task_id,
            "session_id": session_id,
            "score": score,
            "generation": generation,
            "tldr": tldr,
        });
        self.spaces.send_message(
            channel_id,
            &content.to_string(),
            Some(generation as u64),
            None,
        )
    }

    /// Read all artifact messages for a given generation.
    pub fn read_generation_artifacts(
        &self,
        channel_id: u64,
        generation: u32,
    ) -> Result<Vec<HiveArtifactMessage>, SpacesBridgeError> {
        let messages = self.spaces.read_messages(channel_id, 200, None)?;
        let mut artifacts = Vec::new();

        for msg in &messages {
            // Filter by thread (generation)
            if msg.thread_id != Some(generation as u64) {
                continue;
            }
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                if parsed.get("hive_event").and_then(|v| v.as_str()) == Some("artifact_shared") {
                    if let Ok(artifact) =
                        serde_json::from_value::<HiveArtifactMessage>(serde_json::json!({
                            "hive_task_id": parsed["hive_task_id"],
                            "session_id": parsed["session_id"],
                            "score": parsed["score"],
                            "generation": parsed["generation"],
                            "tldr": parsed["tldr"],
                        }))
                    {
                        artifacts.push(artifact);
                    }
                }
            }
        }

        Ok(artifacts)
    }

    /// Announce the selection winner for a generation.
    pub fn announce_selection(
        &self,
        channel_id: u64,
        generation: u32,
        hive_task_id: &str,
        winning_session_id: &str,
        winning_score: f32,
    ) -> Result<SpacesMessage, SpacesBridgeError> {
        let content = serde_json::json!({
            "hive_event": "selection_made",
            "hive_task_id": hive_task_id,
            "winning_session_id": winning_session_id,
            "winning_score": winning_score,
            "generation": generation,
        });
        self.spaces.send_message(
            channel_id,
            &content.to_string(),
            Some(generation as u64),
            None,
        )
    }

    /// Read all hive context from a channel (artifacts, claims, skills).
    pub fn read_hive_context(&self, channel_id: u64) -> Result<HiveContext, SpacesBridgeError> {
        let messages = self.spaces.read_messages(channel_id, 200, None)?;
        let mut ctx = HiveContext::default();

        for msg in &messages {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg.content) {
                match parsed.get("hive_event").and_then(|v| v.as_str()) {
                    Some("artifact_shared") => {
                        if let Ok(artifact) =
                            serde_json::from_value::<HiveArtifactMessage>(serde_json::json!({
                                "hive_task_id": parsed["hive_task_id"],
                                "session_id": parsed["session_id"],
                                "score": parsed["score"],
                                "generation": parsed["generation"],
                                "tldr": parsed["tldr"],
                            }))
                        {
                            ctx.artifacts.push(artifact);
                        }
                    }
                    Some("claim") => {
                        if let Some(desc) = parsed.get("description").and_then(|v| v.as_str()) {
                            ctx.claims.push(desc.to_string());
                        }
                    }
                    Some("skill") => {
                        if let Some(name) = parsed.get("name").and_then(|v| v.as_str()) {
                            ctx.skills.push(name.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockSpacesClient;

    fn setup() -> (Arc<MockSpacesClient>, HiveSpacesCoordinator) {
        let mock = Arc::new(MockSpacesClient::default_hub());
        let coord = HiveSpacesCoordinator::new(mock.clone(), 1);
        (mock, coord)
    }

    #[test]
    fn find_hive_channel_not_found() {
        let (_, coord) = setup();
        let result = coord.find_hive_channel("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn share_artifact_sends_message() {
        let (mock, coord) = setup();
        // Use "general" channel (id=1) as our hive channel for testing
        let msg = coord
            .share_artifact(1, 1, "HIVE001", "SESS-A", 0.87, "rewrote parser")
            .unwrap();

        assert_eq!(msg.channel_id, 1);
        assert_eq!(msg.thread_id, Some(1));

        let sent = mock.sent_messages();
        assert_eq!(sent.len(), 1);

        let parsed: serde_json::Value = serde_json::from_str(&sent[0].content).unwrap();
        assert_eq!(parsed["hive_event"], "artifact_shared");
        assert!((parsed["score"].as_f64().unwrap() - 0.87).abs() < 0.001);
    }

    #[test]
    fn announce_selection_sends_message() {
        let (mock, coord) = setup();
        let msg = coord
            .announce_selection(1, 2, "HIVE001", "SESS-B", 0.92)
            .unwrap();

        assert_eq!(msg.thread_id, Some(2));

        let sent = mock.sent_messages();
        let parsed: serde_json::Value = serde_json::from_str(&sent[0].content).unwrap();
        assert_eq!(parsed["hive_event"], "selection_made");
        assert_eq!(parsed["winning_session_id"], "SESS-B");
    }

    #[test]
    fn read_hive_context_parses_messages() {
        let (mock, coord) = setup();

        // Send some hive messages
        let artifact_json = serde_json::json!({
            "hive_event": "artifact_shared",
            "hive_task_id": "H1",
            "session_id": "S1",
            "score": 0.8,
            "generation": 1,
            "tldr": "first attempt"
        });
        mock.send_message(1, &artifact_json.to_string(), Some(1), None)
            .unwrap();

        let claim_json = serde_json::json!({
            "hive_event": "claim",
            "description": "trying approach X"
        });
        mock.send_message(1, &claim_json.to_string(), None, None)
            .unwrap();

        let ctx = coord.read_hive_context(1).unwrap();
        assert_eq!(ctx.artifacts.len(), 1);
        assert_eq!(ctx.artifacts[0].session_id, "S1");
        assert_eq!(ctx.claims.len(), 1);
        assert_eq!(ctx.claims[0], "trying approach X");
    }

    #[test]
    fn read_generation_artifacts_filters_by_thread() {
        let (mock, coord) = setup();

        // Gen 1 artifact
        let gen1 = serde_json::json!({
            "hive_event": "artifact_shared",
            "hive_task_id": "H1",
            "session_id": "S1",
            "score": 0.7,
            "generation": 1,
            "tldr": "gen1"
        });
        mock.send_message(1, &gen1.to_string(), Some(1), None)
            .unwrap();

        // Gen 2 artifact
        let gen2 = serde_json::json!({
            "hive_event": "artifact_shared",
            "hive_task_id": "H1",
            "session_id": "S2",
            "score": 0.9,
            "generation": 2,
            "tldr": "gen2"
        });
        mock.send_message(1, &gen2.to_string(), Some(2), None)
            .unwrap();

        let gen1_artifacts = coord.read_generation_artifacts(1, 1).unwrap();
        assert_eq!(gen1_artifacts.len(), 1);
        assert_eq!(gen1_artifacts[0].session_id, "S1");

        let gen2_artifacts = coord.read_generation_artifacts(1, 2).unwrap();
        assert_eq!(gen2_artifacts.len(), 1);
        assert_eq!(gen2_artifacts[0].session_id, "S2");
    }
}
