use arcan_core::protocol::{AgentEvent, ChatMessage, Role};
use arcan_core::state::AppState;
use lago_core::Projection;
use lago_core::error::LagoResult;
use lago_core::event::EventEnvelope;

use crate::event_map;

/// A lago [`Projection`] that replays arcan events to rebuild
/// `AppState` and `Vec<ChatMessage>` (conversation history).
///
/// This replaces the ad-hoc replay logic in `loop.rs` with a
/// reusable, testable projection.
pub struct AppStateProjection {
    state: AppState,
    messages: Vec<ChatMessage>,
}

impl AppStateProjection {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            messages: Vec::new(),
        }
    }

    /// Return the reconstructed `AppState`.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Return the reconstructed conversation history.
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Consume the projection and return the state + messages.
    pub fn into_parts(self) -> (AppState, Vec<ChatMessage>) {
        (self.state, self.messages)
    }
}

impl Default for AppStateProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl Projection for AppStateProjection {
    fn on_event(&mut self, envelope: &EventEnvelope) -> LagoResult<()> {
        let Some(agent_event) = event_map::lago_to_arcan(envelope) else {
            return Ok(());
        };

        match agent_event {
            AgentEvent::StatePatched { patch, .. } => {
                let _ = self.state.apply_patch(&patch);
            }
            AgentEvent::TextDelta { delta, .. } => {
                // Aggregate deltas into the last assistant message.
                if let Some(last) = self.messages.last_mut() {
                    if last.role == Role::Assistant {
                        last.content.push_str(&delta);
                    } else {
                        self.messages.push(ChatMessage::assistant(delta));
                    }
                } else {
                    self.messages.push(ChatMessage::assistant(delta));
                }
            }
            AgentEvent::ToolCallCompleted { result, .. } => {
                let output_str =
                    serde_json::to_string(&result.output).unwrap_or_else(|_| "{}".to_string());
                self.messages
                    .push(ChatMessage::tool_result(&result.call_id, output_str));
            }
            _ => {}
        }

        Ok(())
    }

    fn name(&self) -> &str {
        "arcan::app_state"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{StatePatch, StatePatchFormat, StatePatchSource, ToolResultSummary};
    use lago_core::BranchId;
    use lago_core::SessionId;

    fn make_envelope(event: &AgentEvent) -> EventEnvelope {
        let session_id = SessionId::new();
        let branch_id = BranchId::new();
        crate::event_map::arcan_to_lago(&session_id, &branch_id, 1, "r1", event, "test-id")
    }

    #[test]
    fn text_deltas_aggregate_into_assistant_message() {
        let mut proj = AppStateProjection::new();

        let e1 = make_envelope(&AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "Hello ".into(),
        });
        let e2 = make_envelope(&AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "world!".into(),
        });

        proj.on_event(&e1).unwrap();
        proj.on_event(&e2).unwrap();

        assert_eq!(proj.messages().len(), 1);
        assert_eq!(proj.messages()[0].content, "Hello world!");
        assert_eq!(proj.messages()[0].role, Role::Assistant);
    }

    #[test]
    fn tool_result_adds_tool_message() {
        let mut proj = AppStateProjection::new();

        let e = make_envelope(&AgentEvent::ToolCallCompleted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            result: ToolResultSummary {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                output: serde_json::json!({"content": "file data"}),
            },
        });

        proj.on_event(&e).unwrap();

        assert_eq!(proj.messages().len(), 1);
        assert_eq!(proj.messages()[0].role, Role::Tool);
        assert!(proj.messages()[0].content.contains("file data"));
    }

    #[test]
    fn state_patch_applies_to_app_state() {
        let mut proj = AppStateProjection::new();

        let e = make_envelope(&AgentEvent::StatePatched {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            patch: StatePatch {
                format: StatePatchFormat::MergePatch,
                patch: serde_json::json!({"cwd": "/home/user"}),
                source: StatePatchSource::Tool,
            },
            revision: 1,
        });

        proj.on_event(&e).unwrap();

        assert_eq!(proj.state().cwd(), Some("/home/user".to_string()));
    }

    #[test]
    fn into_parts_returns_owned_data() {
        let mut proj = AppStateProjection::new();

        let e = make_envelope(&AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "test".into(),
        });
        proj.on_event(&e).unwrap();

        let (state, messages) = proj.into_parts();
        assert_eq!(messages.len(), 1);
        assert_eq!(state.revision, 0);
    }

    #[test]
    fn mixed_events_produce_correct_history() {
        let mut proj = AppStateProjection::new();

        // Assistant text
        proj.on_event(&make_envelope(&AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "Let me read that file.".into(),
        }))
        .unwrap();

        // Tool result
        proj.on_event(&make_envelope(&AgentEvent::ToolCallCompleted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            result: ToolResultSummary {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                output: serde_json::json!({"content": "hello"}),
            },
        }))
        .unwrap();

        // More assistant text (should be a NEW message after tool result)
        proj.on_event(&make_envelope(&AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 2,
            delta: "Done.".into(),
        }))
        .unwrap();

        assert_eq!(proj.messages().len(), 3);
        assert_eq!(proj.messages()[0].role, Role::Assistant);
        assert_eq!(proj.messages()[0].content, "Let me read that file.");
        assert_eq!(proj.messages()[1].role, Role::Tool);
        assert_eq!(proj.messages()[2].role, Role::Assistant);
        assert_eq!(proj.messages()[2].content, "Done.");
    }

    #[test]
    fn projection_name() {
        let proj = AppStateProjection::new();
        assert_eq!(proj.name(), "arcan::app_state");
    }
}
