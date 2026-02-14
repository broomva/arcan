use arcan_core::protocol::{AgentEvent, ToolCall, ToolResultSummary};
use lago_core::event::SpanStatus;
use lago_core::{BranchId, EventEnvelope, EventId, EventPayload, SeqNo, SessionId};
use std::collections::HashMap;

/// Convert an Arcan `AgentEvent` into a Lago `EventEnvelope`.
pub fn arcan_to_lago(
    session_id: &SessionId,
    branch_id: &BranchId,
    seq: SeqNo,
    run_id: &str,
    event: &AgentEvent,
    arcan_event_id: &str,
) -> EventEnvelope {
    let mut metadata = HashMap::new();
    metadata.insert("arcan_event_id".to_string(), arcan_event_id.to_string());

    let payload = match event {
        AgentEvent::TextDelta {
            delta, iteration, ..
        } => EventPayload::MessageDelta {
            role: "assistant".to_string(),
            delta: delta.clone(),
            index: *iteration,
        },

        AgentEvent::ToolCallRequested { call, .. } => EventPayload::ToolInvoke {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments: call.input.clone(),
            category: None,
        },

        AgentEvent::ToolCallCompleted { result, .. } => EventPayload::ToolResult {
            call_id: result.call_id.clone(),
            tool_name: result.tool_name.clone(),
            result: result.output.clone(),
            duration_ms: 0,
            status: SpanStatus::Ok,
        },

        AgentEvent::ToolCallFailed {
            call_id,
            tool_name,
            error,
            ..
        } => EventPayload::ToolResult {
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            result: serde_json::json!({ "error": error }),
            duration_ms: 0,
            status: SpanStatus::Error,
        },

        AgentEvent::RunFinished {
            final_answer,
            reason,
            total_iterations,
            ..
        } => {
            // If there's a final answer, emit it as a Message.
            // The run metadata goes into the metadata map.
            metadata.insert("run_stop_reason".to_string(), format!("{reason:?}"));
            metadata.insert("total_iterations".to_string(), total_iterations.to_string());

            if let Some(answer) = final_answer {
                EventPayload::Message {
                    role: "assistant".to_string(),
                    content: answer.clone(),
                    model: None,
                    token_usage: None,
                }
            } else {
                EventPayload::Custom {
                    event_type: "run_finished".to_string(),
                    data: serde_json::to_value(event).unwrap_or_default(),
                }
            }
        }

        // Everything else maps to Custom with the full AgentEvent serialized.
        _ => EventPayload::Custom {
            event_type: event_type_name(event).to_string(),
            data: serde_json::to_value(event).unwrap_or_default(),
        },
    };

    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: Some(run_id.to_string().into()),
        seq,
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload,
        metadata,
    }
}

/// Convert a Lago `EventEnvelope` back into an Arcan `AgentEvent`.
///
/// Returns `None` for events that don't map to an `AgentEvent` (e.g. user messages,
/// file events not originated by the agent loop).
pub fn lago_to_arcan(envelope: &EventEnvelope) -> Option<AgentEvent> {
    let run_id = envelope
        .run_id
        .as_ref()
        .map(|r| r.to_string())
        .unwrap_or_default();
    let session_id = envelope.session_id.to_string();

    match &envelope.payload {
        EventPayload::MessageDelta { delta, index, .. } => Some(AgentEvent::TextDelta {
            run_id,
            session_id,
            iteration: *index,
            delta: delta.clone(),
        }),

        EventPayload::ToolInvoke {
            call_id,
            tool_name,
            arguments,
            ..
        } => Some(AgentEvent::ToolCallRequested {
            run_id,
            session_id,
            iteration: 0, // Not preserved in Lago payload; default
            call: ToolCall {
                call_id: call_id.clone(),
                tool_name: tool_name.clone(),
                input: arguments.clone(),
            },
        }),

        EventPayload::ToolResult {
            call_id,
            tool_name,
            result,
            status,
            ..
        } => match status {
            SpanStatus::Ok => Some(AgentEvent::ToolCallCompleted {
                run_id,
                session_id,
                iteration: 0,
                result: ToolResultSummary {
                    call_id: call_id.clone(),
                    tool_name: tool_name.clone(),
                    output: result.clone(),
                },
            }),
            _ => Some(AgentEvent::ToolCallFailed {
                run_id,
                session_id,
                iteration: 0,
                call_id: call_id.clone(),
                tool_name: tool_name.clone(),
                error: result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error")
                    .to_string(),
            }),
        },

        EventPayload::Message { role, content, .. } if role == "assistant" => {
            // Reconstruct as a TextDelta for replay context
            Some(AgentEvent::TextDelta {
                run_id,
                session_id,
                iteration: 0,
                delta: content.clone(),
            })
        }

        EventPayload::Custom { event_type, data } => {
            // Try to deserialize the original AgentEvent from the Custom data.
            match event_type.as_str() {
                "run_started" | "iteration_started" | "model_output" | "state_patched"
                | "run_errored" | "run_finished" => {
                    serde_json::from_value::<AgentEvent>(data.clone()).ok()
                }
                _ => None,
            }
        }

        _ => None,
    }
}

fn event_type_name(event: &AgentEvent) -> &'static str {
    match event {
        AgentEvent::RunStarted { .. } => "run_started",
        AgentEvent::IterationStarted { .. } => "iteration_started",
        AgentEvent::ModelOutput { .. } => "model_output",
        AgentEvent::TextDelta { .. } => "text_delta",
        AgentEvent::ToolCallRequested { .. } => "tool_call_requested",
        AgentEvent::ToolCallCompleted { .. } => "tool_call_completed",
        AgentEvent::ToolCallFailed { .. } => "tool_call_failed",
        AgentEvent::StatePatched { .. } => "state_patched",
        AgentEvent::RunErrored { .. } => "run_errored",
        AgentEvent::RunFinished { .. } => "run_finished",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{RunStopReason, StatePatch, StatePatchFormat, StatePatchSource};

    fn test_session() -> SessionId {
        SessionId::new()
    }

    fn test_branch() -> BranchId {
        BranchId::new()
    }

    #[test]
    fn text_delta_round_trips() {
        let event = AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 3,
            delta: "hello world".into(),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-1");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::TextDelta {
                delta, iteration, ..
            } => {
                assert_eq!(delta, "hello world");
                assert_eq!(iteration, 3);
            }
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn tool_call_requested_round_trips() {
        let event = AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            call: ToolCall {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                input: serde_json::json!({"path": "test.txt"}),
            },
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 2, "r1", &event, "uuid-2");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::ToolCallRequested { call, .. } => {
                assert_eq!(call.call_id, "c1");
                assert_eq!(call.tool_name, "read_file");
            }
            _ => panic!("expected ToolCallRequested"),
        }
    }

    #[test]
    fn tool_call_completed_round_trips() {
        let event = AgentEvent::ToolCallCompleted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            result: ToolResultSummary {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                output: serde_json::json!({"content": "hello"}),
            },
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 3, "r1", &event, "uuid-3");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::ToolCallCompleted { result, .. } => {
                assert_eq!(result.call_id, "c1");
                assert_eq!(result.tool_name, "read_file");
            }
            _ => panic!("expected ToolCallCompleted"),
        }
    }

    #[test]
    fn tool_call_failed_round_trips() {
        let event = AgentEvent::ToolCallFailed {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            call_id: "c1".into(),
            tool_name: "bash".into(),
            error: "command not found".into(),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 4, "r1", &event, "uuid-4");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::ToolCallFailed {
                error, tool_name, ..
            } => {
                assert_eq!(error, "command not found");
                assert_eq!(tool_name, "bash");
            }
            _ => panic!("expected ToolCallFailed"),
        }
    }

    #[test]
    fn run_started_round_trips_via_custom() {
        let event = AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            provider: "anthropic".into(),
            max_iterations: 24,
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-5");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::RunStarted {
                provider,
                max_iterations,
                ..
            } => {
                assert_eq!(provider, "anthropic");
                assert_eq!(max_iterations, 24);
            }
            _ => panic!("expected RunStarted"),
        }
    }

    #[test]
    fn state_patched_round_trips_via_custom() {
        let event = AgentEvent::StatePatched {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 2,
            patch: StatePatch {
                format: StatePatchFormat::MergePatch,
                patch: serde_json::json!({"key": "value"}),
                source: StatePatchSource::Tool,
            },
            revision: 5,
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 5, "r1", &event, "uuid-6");
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::StatePatched { revision, .. } => {
                assert_eq!(revision, 5);
            }
            _ => panic!("expected StatePatched"),
        }
    }

    #[test]
    fn run_finished_with_answer_becomes_message() {
        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 3,
            final_answer: Some("The answer is 42.".into()),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 10, "r1", &event, "uuid-7");

        // Should produce a Message payload (not Custom) when there's a final answer
        match &envelope.payload {
            EventPayload::Message { role, content, .. } => {
                assert_eq!(role, "assistant");
                assert_eq!(content, "The answer is 42.");
            }
            _ => panic!("expected Message payload for RunFinished with answer"),
        }

        // And it round-trips back as a TextDelta (for replay)
        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::TextDelta { delta, .. } => {
                assert_eq!(delta, "The answer is 42.");
            }
            _ => panic!("expected TextDelta from assistant message"),
        }
    }

    #[test]
    fn run_finished_without_answer_becomes_custom() {
        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::NeedsUser,
            total_iterations: 1,
            final_answer: None,
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 10, "r1", &event, "uuid-8");
        match &envelope.payload {
            EventPayload::Custom { event_type, .. } => {
                assert_eq!(event_type, "run_finished");
            }
            _ => panic!("expected Custom payload for RunFinished without answer"),
        }
    }

    #[test]
    fn metadata_includes_arcan_event_id() {
        let event = AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            provider: "mock".into(),
            max_iterations: 10,
        };
        let envelope = arcan_to_lago(
            &test_session(),
            &test_branch(),
            1,
            "r1",
            &event,
            "my-uuid-123",
        );
        assert_eq!(
            envelope.metadata.get("arcan_event_id"),
            Some(&"my-uuid-123".to_string())
        );
    }
}
