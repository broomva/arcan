use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
use lago_core::event::{SpanStatus, TokenUsage};
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
            // Map RunFinished to native Lago RunFinished
            EventPayload::RunFinished {
                reason: format!("{reason:?}"), // TODO: map enum to string/enum?
                total_iterations: *total_iterations,
                final_answer: final_answer.clone(),
                usage: None,
            }
        }

        AgentEvent::RunStarted {
            provider,
            max_iterations,
            ..
        } => EventPayload::RunStarted {
            provider: provider.clone(),
            max_iterations: *max_iterations,
        },

        AgentEvent::IterationStarted { iteration, .. } => EventPayload::StepStarted { index: *iteration },

        AgentEvent::ModelOutput {
            iteration,
            stop_reason,
            directive_count,
            ..
        } => EventPayload::StepFinished {
            index: *iteration,
            stop_reason: format!("{stop_reason:?}"),
            directive_count: *directive_count,
        },

        AgentEvent::StatePatched {
            iteration,
            patch,
            revision,
            ..
        } => EventPayload::StatePatched {
            index: *iteration,
            patch: serde_json::to_value(patch).unwrap_or_default(),
            revision: *revision,
        },

        AgentEvent::RunErrored { error, .. } => EventPayload::Error {
            error: error.clone(),
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

        EventPayload::RunStarted {
            provider,
            max_iterations,
        } => Some(AgentEvent::RunStarted {
            run_id,
            session_id,
            provider: provider.clone(),
            max_iterations: *max_iterations,
        }),

        EventPayload::RunFinished {
            reason,
            total_iterations,
            final_answer,
            ..
        } => Some(AgentEvent::RunFinished {
            run_id,
            session_id,
            reason: parse_run_stop_reason(reason),
            total_iterations: *total_iterations,
            final_answer: final_answer.clone(),
        }),

        EventPayload::StepStarted { index } => Some(AgentEvent::IterationStarted {
            run_id,
            session_id,
            iteration: *index,
        }),

        EventPayload::StepFinished {
            index,
            stop_reason,
            directive_count,
        } => Some(AgentEvent::ModelOutput {
            run_id,
            session_id,
            iteration: *index,
            stop_reason: parse_model_stop_reason(stop_reason),
            directive_count: *directive_count,
        }),

        EventPayload::StatePatched {
            index,
            patch,
            revision,
        } => serde_json::from_value(patch.clone())
            .ok()
            .map(|p| AgentEvent::StatePatched {
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                iteration: *index,
                patch: p,
                revision: *revision,
            }),

        EventPayload::Error { error } => Some(AgentEvent::RunErrored {
            run_id,
            session_id,
            error: error.clone(),
        }),

        // Backward compatibility / Replay logic:
        // Assistant Message -> TextDelta
        EventPayload::Message { role, content, .. } if role == "assistant" => {
            Some(AgentEvent::TextDelta {
                run_id,
                session_id,
                iteration: 0,
                delta: content.clone(),
            })
        }

        _ => None,
    }
}

fn parse_run_stop_reason(s: &str) -> RunStopReason {
    match s {
        "Completed" => RunStopReason::Completed,
        "NeedsUser" => RunStopReason::NeedsUser,
        "BlockedByPolicy" => RunStopReason::BlockedByPolicy,
        "BudgetExceeded" => RunStopReason::BudgetExceeded,
        "Error" => RunStopReason::Error,
        _ => RunStopReason::Completed, // Default fallback
    }
}

fn parse_model_stop_reason(s: &str) -> arcan_core::protocol::ModelStopReason {
    use arcan_core::protocol::ModelStopReason;
    match s {
        "EndTurn" => ModelStopReason::EndTurn,
        "ToolUse" => ModelStopReason::ToolUse,
        "NeedsUser" => ModelStopReason::NeedsUser,
        "MaxTokens" => ModelStopReason::MaxTokens,
        "Safety" => ModelStopReason::Safety,
        "Unknown" => ModelStopReason::Unknown,
        _ => ModelStopReason::Unknown,
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
    fn run_started_round_trips() {
        let event = AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "s1".into(),
            provider: "anthropic".into(),
            max_iterations: 24,
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-5");
        
        // Assert native payload
        if let EventPayload::RunStarted { provider, .. } = &envelope.payload {
            assert_eq!(provider, "anthropic");
        } else {
            panic!("expected RunStarted payload");
        }

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
    fn run_finished_round_trips() {
        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 10,
            final_answer: Some("42".into()),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-6");

        if let EventPayload::RunFinished { reason, .. } = &envelope.payload {
             assert_eq!(reason, "Completed");
        } else {
             panic!("expected RunFinished payload");
        }

        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
             AgentEvent::RunFinished { reason, final_answer, .. } => {
                 assert_eq!(reason, RunStopReason::Completed);
                 assert_eq!(final_answer.as_deref(), Some("42"));
             }
             _ => panic!("expected RunFinished"),
        }
    }

    #[test]
    fn state_patched_round_trips() {
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
         let envelope = arcan_to_lago(&test_session(), &test_branch(), 5, "r1", &event, "uuid-7");
         
         if let EventPayload::StatePatched { revision, .. } = &envelope.payload {
             assert_eq!(*revision, 5);
         } else {
             panic!("expected StatePatched payload");
         }

         let back = lago_to_arcan(&envelope).expect("should map back");
         match back {
             AgentEvent::StatePatched { revision, .. } => {
                 assert_eq!(revision, 5);
             }
             _ => panic!("expected StatePatched"),
         }
    }
}
