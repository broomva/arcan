use arcan_core::protocol::AgentEvent;
use lago_api::sse::format::{SseFormat, SseFrame};
use lago_core::{BranchId, SessionId};

use crate::event_map;

/// Converts arcan `AgentEvent`s into SSE frames using any lago `SseFormat`.
///
/// This bridges arcan's event stream with lago's multi-format SSE system,
/// supporting OpenAI, Anthropic, Vercel AI SDK, and native Lago wire formats.
pub struct SseBridge {
    format: Box<dyn SseFormat>,
    session_id: SessionId,
    branch_id: BranchId,
    seq: u64,
}

impl SseBridge {
    pub fn new(format: Box<dyn SseFormat>, session_id: &str, branch_id: &str) -> Self {
        Self {
            format,
            session_id: SessionId::from(session_id.to_string()),
            branch_id: BranchId::from(branch_id.to_string()),
            seq: 0,
        }
    }

    /// Convert an `AgentEvent` into zero or more SSE frames.
    ///
    /// Events that the selected format doesn't handle are silently dropped
    /// (empty vec returned).
    pub fn format_event(&mut self, event: &AgentEvent, run_id: &str) -> Vec<SseFrame> {
        self.seq += 1;
        let event_id = uuid::Uuid::new_v4().to_string();

        let envelope = event_map::arcan_to_lago(
            &self.session_id,
            &self.branch_id,
            self.seq,
            run_id,
            event,
            &event_id,
        );

        self.format.format(&envelope)
    }

    /// Return the "done" frame for the selected format, if any.
    pub fn done_frame(&self) -> Option<SseFrame> {
        self.format.done_frame()
    }

    /// Return extra HTTP headers required by the selected format.
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        self.format.extra_headers()
    }

    /// The name of the active format.
    pub fn format_name(&self) -> &str {
        self.format.name()
    }
}

/// Select a lago SSE format by name.
///
/// Supported names: `"openai"`, `"anthropic"`, `"vercel"`, `"lago"`.
/// Returns `None` for unknown format names.
pub fn select_format(name: &str) -> Option<Box<dyn SseFormat>> {
    match name {
        "openai" => Some(Box::new(lago_api::sse::openai::OpenAiFormat)),
        "anthropic" => Some(Box::new(lago_api::sse::anthropic::AnthropicFormat)),
        "vercel" => Some(Box::new(lago_api::sse::vercel::VercelFormat)),
        "lago" => Some(Box::new(lago_api::sse::lago::LagoFormat)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{RunStopReason, ToolCall};

    #[test]
    fn text_delta_produces_openai_frame() {
        let mut bridge = SseBridge::new(select_format("openai").unwrap(), "test-session", "main");

        let event = AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "Hello!".into(),
        };

        let frames = bridge.format_event(&event, "r1");
        assert_eq!(frames.len(), 1);

        let data: serde_json::Value = serde_json::from_str(&frames[0].data).unwrap();
        assert_eq!(data["object"], "chat.completion.chunk");
        assert_eq!(data["choices"][0]["delta"]["content"], "Hello!");
    }

    #[test]
    fn run_finished_with_answer_produces_openai_frame() {
        let mut bridge = SseBridge::new(select_format("openai").unwrap(), "test-session", "main");

        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("The answer is 42.".into()),
        };

        let frames = bridge.format_event(&event, "r1");
        // RunFinished with answer becomes Message, OpenAI format handles it
        assert!(!frames.is_empty());
    }

    #[test]
    fn tool_events_filtered_in_openai_format() {
        let mut bridge = SseBridge::new(select_format("openai").unwrap(), "test-session", "main");

        let event = AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            call: ToolCall {
                call_id: "c1".into(),
                tool_name: "bash".into(),
                input: serde_json::json!({}),
            },
        };

        // OpenAI format only emits message/delta frames
        let frames = bridge.format_event(&event, "r1");
        // ToolInvoke events are not message events, so OpenAI filters them
        assert!(frames.is_empty());
    }

    #[test]
    fn lago_format_emits_all_events() {
        let mut bridge = SseBridge::new(select_format("lago").unwrap(), "test-session", "main");

        let event = AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            call: ToolCall {
                call_id: "c1".into(),
                tool_name: "bash".into(),
                input: serde_json::json!({"cmd": "ls"}),
            },
        };

        let frames = bridge.format_event(&event, "r1");
        // Lago native format should emit the event
        assert!(!frames.is_empty());
    }

    #[test]
    fn done_frame_for_openai() {
        let bridge = SseBridge::new(select_format("openai").unwrap(), "test-session", "main");
        let done = bridge.done_frame().unwrap();
        assert_eq!(done.data, "[DONE]");
    }

    #[test]
    fn select_format_unknown_returns_none() {
        assert!(select_format("unknown").is_none());
    }

    #[test]
    fn select_format_all_known() {
        for name in &["openai", "anthropic", "vercel", "lago"] {
            assert!(select_format(name).is_some(), "format {name} not found");
        }
    }

    #[test]
    fn sequence_numbers_increment() {
        let mut bridge = SseBridge::new(select_format("lago").unwrap(), "test-session", "main");

        let event = AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            delta: "a".into(),
        };

        bridge.format_event(&event, "r1");
        bridge.format_event(&event, "r1");

        assert_eq!(bridge.seq, 2);
    }
}
