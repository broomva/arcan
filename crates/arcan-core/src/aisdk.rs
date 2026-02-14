use crate::protocol::AgentEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Vercel AI SDK v5 "UI Message Stream Protocol" data part.
///
/// Maps Arcan's native `AgentEvent` to AI SDK compatible wire format
/// for consumption by `useChat` / `useCompletion` hooks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AiSdkPart {
    /// Signal that a new message is starting.
    Start { message_id: String },
    /// Incremental text content from the model.
    TextDelta { text_delta: String },
    /// A tool call is beginning.
    ToolCallBegin {
        tool_call_id: String,
        tool_name: String,
    },
    /// Incremental arguments for an in-progress tool call.
    ToolCallDelta {
        tool_call_id: String,
        args_text_delta: String,
    },
    /// A tool has returned a result.
    ToolResult {
        tool_call_id: String,
        tool_name: String,
        result: Value,
    },
    /// The stream is finishing.
    Finish {
        finish_reason: String,
        usage: Option<AiSdkUsage>,
    },
    /// An error occurred.
    Error { error: String },
    /// Arcan extension: state patch event.
    ArcanStatePatch { patch: Value, revision: u64 },
}

/// Token usage statistics compatible with AI SDK.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiSdkUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

/// Convert an `AgentEvent` into zero or more `AiSdkPart`s.
///
/// Some events map 1:1, others produce multiple parts, and some
/// (like `IterationStarted`) are omitted as they have no AI SDK equivalent.
pub fn to_aisdk_parts(event: &AgentEvent) -> Vec<AiSdkPart> {
    match event {
        AgentEvent::RunStarted { run_id, .. } => {
            vec![AiSdkPart::Start {
                message_id: run_id.clone(),
            }]
        }

        AgentEvent::TextDelta { delta, .. } => {
            vec![AiSdkPart::TextDelta {
                text_delta: delta.clone(),
            }]
        }

        AgentEvent::ToolCallRequested { call, .. } => {
            let args_json = serde_json::to_string(&call.input).unwrap_or_default();
            vec![
                AiSdkPart::ToolCallBegin {
                    tool_call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                },
                AiSdkPart::ToolCallDelta {
                    tool_call_id: call.call_id.clone(),
                    args_text_delta: args_json,
                },
            ]
        }

        AgentEvent::ToolCallCompleted { result, .. } => {
            vec![AiSdkPart::ToolResult {
                tool_call_id: result.call_id.clone(),
                tool_name: result.tool_name.clone(),
                result: result.output.clone(),
            }]
        }

        AgentEvent::ToolCallFailed {
            call_id,
            tool_name,
            error,
            ..
        } => {
            vec![AiSdkPart::ToolResult {
                tool_call_id: call_id.clone(),
                tool_name: tool_name.clone(),
                result: serde_json::json!({ "error": error }),
            }]
        }

        AgentEvent::StatePatched {
            patch, revision, ..
        } => {
            vec![AiSdkPart::ArcanStatePatch {
                patch: patch.patch.clone(),
                revision: *revision,
            }]
        }

        AgentEvent::RunErrored { error, .. } => {
            vec![AiSdkPart::Error {
                error: error.clone(),
            }]
        }

        AgentEvent::RunFinished {
            reason,
            final_answer,
            ..
        } => {
            let mut parts = Vec::new();
            // If there's a final answer, emit it as a text delta
            if let Some(answer) = final_answer {
                if !answer.is_empty() {
                    parts.push(AiSdkPart::TextDelta {
                        text_delta: answer.clone(),
                    });
                }
            }
            parts.push(AiSdkPart::Finish {
                finish_reason: format!("{:?}", reason),
                usage: None,
            });
            parts
        }

        // IterationStarted and ModelOutput have no direct AI SDK equivalent
        AgentEvent::IterationStarted { .. } | AgentEvent::ModelOutput { .. } => {
            vec![]
        }
    }
}

/// Serialize an `AiSdkPart` to the SSE wire format (newline-delimited JSON).
pub fn aisdk_part_to_sse(part: &AiSdkPart) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string(part)?;
    Ok(format!("data: {json}\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ModelStopReason, RunStopReason, StatePatch, StatePatchFormat, StatePatchSource, ToolCall,
        ToolResultSummary,
    };
    use serde_json::json;

    #[test]
    fn run_started_maps_to_start() {
        let event = AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            provider: "anthropic".to_string(),
            max_iterations: 10,
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::Start {
                message_id: "r1".to_string()
            }
        );
    }

    #[test]
    fn text_delta_maps_directly() {
        let event = AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            delta: "Hello ".to_string(),
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::TextDelta {
                text_delta: "Hello ".to_string()
            }
        );
    }

    #[test]
    fn tool_call_requested_produces_begin_and_delta() {
        let event = AgentEvent::ToolCallRequested {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            call: ToolCall {
                call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
                input: json!({"path": "/tmp/test.rs"}),
            },
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts[0],
            AiSdkPart::ToolCallBegin {
                tool_call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
            }
        );
        match &parts[1] {
            AiSdkPart::ToolCallDelta {
                tool_call_id,
                args_text_delta,
            } => {
                assert_eq!(tool_call_id, "c1");
                assert!(args_text_delta.contains("path"));
            }
            other => panic!("Expected ToolCallDelta, got {:?}", other),
        }
    }

    #[test]
    fn tool_call_completed_maps_to_result() {
        let event = AgentEvent::ToolCallCompleted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            result: ToolResultSummary {
                call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
                output: json!({"content": "file contents here"}),
            },
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::ToolResult {
                tool_call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
                result: json!({"content": "file contents here"}),
            }
        );
    }

    #[test]
    fn tool_call_failed_maps_to_error_result() {
        let event = AgentEvent::ToolCallFailed {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            call_id: "c1".to_string(),
            tool_name: "bash".to_string(),
            error: "command not found".to_string(),
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::ToolResult {
                tool_call_id: "c1".to_string(),
                tool_name: "bash".to_string(),
                result: json!({"error": "command not found"}),
            }
        );
    }

    #[test]
    fn state_patched_maps_to_extension() {
        let event = AgentEvent::StatePatched {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            patch: StatePatch {
                format: StatePatchFormat::MergePatch,
                patch: json!({"cwd": "/new"}),
                source: StatePatchSource::System,
            },
            revision: 5,
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::ArcanStatePatch {
                patch: json!({"cwd": "/new"}),
                revision: 5,
            }
        );
    }

    #[test]
    fn run_errored_maps_to_error() {
        let event = AgentEvent::RunErrored {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            error: "provider timeout".to_string(),
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            AiSdkPart::Error {
                error: "provider timeout".to_string()
            }
        );
    }

    #[test]
    fn run_finished_maps_to_finish() {
        let event = AgentEvent::RunFinished {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            reason: RunStopReason::Completed,
            total_iterations: 3,
            final_answer: None,
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            AiSdkPart::Finish {
                finish_reason,
                usage,
            } => {
                assert!(finish_reason.contains("Completed"));
                assert!(usage.is_none());
            }
            other => panic!("Expected Finish, got {:?}", other),
        }
    }

    #[test]
    fn run_finished_with_final_answer_emits_text_then_finish() {
        let event = AgentEvent::RunFinished {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("Done!".to_string()),
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts[0],
            AiSdkPart::TextDelta {
                text_delta: "Done!".to_string()
            }
        );
        assert!(matches!(parts[1], AiSdkPart::Finish { .. }));
    }

    #[test]
    fn iteration_started_and_model_output_produce_empty() {
        let event1 = AgentEvent::IterationStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
        };
        let event2 = AgentEvent::ModelOutput {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            stop_reason: ModelStopReason::EndTurn,
            directive_count: 0,
            usage: None,
        };
        assert!(to_aisdk_parts(&event1).is_empty());
        assert!(to_aisdk_parts(&event2).is_empty());
    }

    #[test]
    fn sse_wire_format() {
        let part = AiSdkPart::TextDelta {
            text_delta: "hello".to_string(),
        };
        let sse = aisdk_part_to_sse(&part).unwrap();
        assert!(sse.starts_with("data: "));
        assert!(sse.ends_with("\n\n"));
        assert!(sse.contains("text-delta"));
        assert!(sse.contains("hello"));
    }

    #[test]
    fn round_trip_serialization() {
        let parts = vec![
            AiSdkPart::Start {
                message_id: "m1".to_string(),
            },
            AiSdkPart::TextDelta {
                text_delta: "hi".to_string(),
            },
            AiSdkPart::Finish {
                finish_reason: "end_turn".to_string(),
                usage: Some(AiSdkUsage {
                    prompt_tokens: Some(100),
                    completion_tokens: Some(50),
                }),
            },
        ];

        for part in &parts {
            let json = serde_json::to_string(part).unwrap();
            let decoded: AiSdkPart = serde_json::from_str(&json).unwrap();
            assert_eq!(*part, decoded);
        }
    }
}
