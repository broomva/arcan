use crate::protocol::AgentEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ─── AI SDK v6 UI Message Stream Protocol ───────────────────────
//
// Spec: https://sdk.vercel.ai/docs/ai-sdk-ui/stream-protocol
// Header: x-vercel-ai-ui-message-stream: v1
// Transport: SSE, data: {json}\n\n
// Termination: data: [DONE]

/// Vercel AI SDK v6 "UI Message Stream Protocol" part.
///
/// Each variant maps to a v6 stream part type. Custom Arcan extensions
/// use the `data-*` namespace per spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UiStreamPart {
    // ── Control ──
    Start {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    Finish {},
    StartStep {},
    FinishStep {},
    Abort {
        reason: String,
    },

    // ── Text ──
    TextStart {
        id: String,
    },
    TextDelta {
        id: String,
        delta: String,
    },
    TextEnd {
        id: String,
    },

    // ── Reasoning ──
    ReasoningStart {
        id: String,
    },
    ReasoningDelta {
        id: String,
        delta: String,
    },
    ReasoningEnd {
        id: String,
    },

    // ── Tool ──
    ToolInputStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
    },
    ToolInputDelta {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "inputTextDelta")]
        input_text_delta: String,
    },
    ToolInputAvailable {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: Value,
    },
    ToolOutputAvailable {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        output: Value,
    },

    // ── Error ──
    Error {
        #[serde(rename = "errorText")]
        error_text: String,
    },

    // ── Arcan Extensions (data-* namespace) ──
    #[serde(rename = "data-state-patch")]
    DataStatePatch {
        data: Value,
    },
    #[serde(rename = "data-approval-request")]
    DataApprovalRequest {
        data: Value,
    },
}

/// Convert an `AgentEvent` into zero or more `UiStreamPart`s (v6 protocol).
///
/// This is a stateless mapping. Text boundary tracking (TextStart/TextEnd)
/// is handled by the SSE bridge in server.rs, which wraps consecutive
/// TextDelta events with boundary signals.
pub fn to_ui_stream_parts(event: &AgentEvent) -> Vec<UiStreamPart> {
    match event {
        AgentEvent::RunStarted { run_id, .. } => {
            vec![UiStreamPart::Start {
                message_id: run_id.clone(),
            }]
        }

        AgentEvent::IterationStarted { .. } => {
            vec![UiStreamPart::StartStep {}]
        }

        AgentEvent::ModelOutput { .. } => {
            vec![UiStreamPart::FinishStep {}]
        }

        AgentEvent::TextDelta { run_id, delta, .. } => {
            vec![UiStreamPart::TextDelta {
                id: format!("{run_id}-text"),
                delta: delta.clone(),
            }]
        }

        AgentEvent::ToolCallRequested { call, .. } => {
            let args_json = serde_json::to_string(&call.input).unwrap_or_default();
            vec![
                UiStreamPart::ToolInputStart {
                    tool_call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                },
                UiStreamPart::ToolInputDelta {
                    tool_call_id: call.call_id.clone(),
                    input_text_delta: args_json,
                },
                UiStreamPart::ToolInputAvailable {
                    tool_call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: call.input.clone(),
                },
            ]
        }

        AgentEvent::ToolCallCompleted { result, .. } => {
            vec![UiStreamPart::ToolOutputAvailable {
                tool_call_id: result.call_id.clone(),
                output: result.output.clone(),
            }]
        }

        AgentEvent::ToolCallFailed { call_id, error, .. } => {
            vec![UiStreamPart::ToolOutputAvailable {
                tool_call_id: call_id.clone(),
                output: serde_json::json!({ "error": error }),
            }]
        }

        AgentEvent::StatePatched {
            patch, revision, ..
        } => {
            vec![UiStreamPart::DataStatePatch {
                data: serde_json::json!({
                    "patch": patch.patch,
                    "revision": revision,
                }),
            }]
        }

        AgentEvent::RunErrored { error, .. } => {
            vec![UiStreamPart::Error {
                error_text: error.clone(),
            }]
        }

        AgentEvent::RunFinished {
            run_id,
            final_answer,
            ..
        } => {
            let mut parts = Vec::new();
            if let Some(answer) = final_answer {
                if !answer.is_empty() {
                    let text_id = format!("{run_id}-text");
                    parts.push(UiStreamPart::TextDelta {
                        id: text_id.clone(),
                        delta: answer.clone(),
                    });
                }
            }
            parts.push(UiStreamPart::Finish {});
            parts
        }

        AgentEvent::ApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            risk,
            ..
        } => {
            vec![UiStreamPart::DataApprovalRequest {
                data: serde_json::json!({
                    "approvalId": approval_id,
                    "toolCallId": call_id,
                    "toolName": tool_name,
                    "arguments": arguments,
                    "risk": risk,
                }),
            }]
        }

        // Events with no UI representation
        AgentEvent::ContextCompacted { .. } | AgentEvent::ApprovalResolved { .. } => {
            vec![]
        }
    }
}

/// Serialize a `UiStreamPart` to the SSE wire format.
pub fn ui_stream_part_to_sse(part: &UiStreamPart) -> Result<String, serde_json::Error> {
    let json = serde_json::to_string(part)?;
    Ok(format!("data: {json}\n\n"))
}

// ─── Deprecated v5 aliases (will be removed) ────────────────────

/// Deprecated: use `UiStreamPart` instead.
pub type AiSdkPart = UiStreamPart;

/// Deprecated: use `to_ui_stream_parts` instead.
pub fn to_aisdk_parts(event: &AgentEvent) -> Vec<UiStreamPart> {
    to_ui_stream_parts(event)
}

/// Deprecated: use `ui_stream_part_to_sse` instead.
pub fn aisdk_part_to_sse(part: &UiStreamPart) -> Result<String, serde_json::Error> {
    ui_stream_part_to_sse(part)
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::Start {
                message_id: "r1".to_string()
            }
        );
    }

    #[test]
    fn iteration_started_maps_to_start_step() {
        let event = AgentEvent::IterationStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
        };
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], UiStreamPart::StartStep {});
    }

    #[test]
    fn model_output_maps_to_finish_step() {
        let event = AgentEvent::ModelOutput {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            stop_reason: ModelStopReason::EndTurn,
            directive_count: 0,
            usage: None,
        };
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], UiStreamPart::FinishStep {});
    }

    #[test]
    fn text_delta_includes_id() {
        let event = AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            delta: "Hello ".to_string(),
        };
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::TextDelta {
                id: "r1-text".to_string(),
                delta: "Hello ".to_string(),
            }
        );
    }

    #[test]
    fn tool_call_produces_input_start_delta_available() {
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 3);

        assert_eq!(
            parts[0],
            UiStreamPart::ToolInputStart {
                tool_call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
            }
        );
        match &parts[1] {
            UiStreamPart::ToolInputDelta {
                tool_call_id,
                input_text_delta,
            } => {
                assert_eq!(tool_call_id, "c1");
                assert!(input_text_delta.contains("path"));
            }
            other => panic!("Expected ToolInputDelta, got {:?}", other),
        }
        assert_eq!(
            parts[2],
            UiStreamPart::ToolInputAvailable {
                tool_call_id: "c1".to_string(),
                tool_name: "read_file".to_string(),
                input: json!({"path": "/tmp/test.rs"}),
            }
        );
    }

    #[test]
    fn tool_completed_maps_to_output_available() {
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::ToolOutputAvailable {
                tool_call_id: "c1".to_string(),
                output: json!({"content": "file contents here"}),
            }
        );
    }

    #[test]
    fn tool_failed_maps_to_output_with_error() {
        let event = AgentEvent::ToolCallFailed {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            call_id: "c1".to_string(),
            tool_name: "bash".to_string(),
            error: "command not found".to_string(),
        };
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::ToolOutputAvailable {
                tool_call_id: "c1".to_string(),
                output: json!({"error": "command not found"}),
            }
        );
    }

    #[test]
    fn state_patched_maps_to_data_extension() {
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::DataStatePatch {
                data: json!({"patch": {"cwd": "/new"}, "revision": 5}),
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0],
            UiStreamPart::Error {
                error_text: "provider timeout".to_string()
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0], UiStreamPart::Finish {});
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
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts[0],
            UiStreamPart::TextDelta {
                id: "r1-text".to_string(),
                delta: "Done!".to_string(),
            }
        );
        assert_eq!(parts[1], UiStreamPart::Finish {});
    }

    #[test]
    fn context_compacted_produces_empty() {
        let event = AgentEvent::ContextCompacted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            dropped_count: 5,
            tokens_before: 1000,
            tokens_after: 500,
        };
        assert!(to_ui_stream_parts(&event).is_empty());
    }

    #[test]
    fn approval_requested_maps_to_data_approval() {
        let event = AgentEvent::ApprovalRequested {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            approval_id: "appr-1".to_string(),
            call_id: "c1".to_string(),
            tool_name: "bash".to_string(),
            arguments: json!({"command": "rm -rf /"}),
            risk: "high".to_string(),
        };
        let parts = to_ui_stream_parts(&event);
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            UiStreamPart::DataApprovalRequest { data } => {
                assert_eq!(data["approvalId"], "appr-1");
                assert_eq!(data["toolCallId"], "c1");
                assert_eq!(data["toolName"], "bash");
                assert_eq!(data["risk"], "high");
            }
            other => panic!("Expected DataApprovalRequest, got {:?}", other),
        }
    }

    #[test]
    fn v6_wire_format_serialization() {
        // Verify exact JSON shapes match v6 spec
        let start = UiStreamPart::Start {
            message_id: "m1".to_string(),
        };
        let json = serde_json::to_string(&start).unwrap();
        assert!(json.contains(r#""type":"start""#));
        assert!(json.contains(r#""messageId":"m1""#));

        let text = UiStreamPart::TextDelta {
            id: "t1".to_string(),
            delta: "hi".to_string(),
        };
        let json = serde_json::to_string(&text).unwrap();
        assert!(json.contains(r#""type":"text-delta""#));
        assert!(json.contains(r#""delta":"hi""#));

        let tool = UiStreamPart::ToolInputStart {
            tool_call_id: "c1".to_string(),
            tool_name: "bash".to_string(),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains(r#""type":"tool-input-start""#));
        assert!(json.contains(r#""toolCallId":"c1""#));
        assert!(json.contains(r#""toolName":"bash""#));

        let error = UiStreamPart::Error {
            error_text: "boom".to_string(),
        };
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains(r#""type":"error""#));
        assert!(json.contains(r#""errorText":"boom""#));

        let ext = UiStreamPart::DataStatePatch {
            data: json!({"patch": {}}),
        };
        let json = serde_json::to_string(&ext).unwrap();
        assert!(json.contains(r#""type":"data-state-patch""#));
    }

    #[test]
    fn sse_wire_format() {
        let part = UiStreamPart::TextDelta {
            id: "t1".to_string(),
            delta: "hello".to_string(),
        };
        let sse = ui_stream_part_to_sse(&part).unwrap();
        assert!(sse.starts_with("data: "));
        assert!(sse.ends_with("\n\n"));
        assert!(sse.contains("text-delta"));
        assert!(sse.contains("hello"));
    }

    #[test]
    fn round_trip_serialization() {
        let parts = vec![
            UiStreamPart::Start {
                message_id: "m1".to_string(),
            },
            UiStreamPart::TextDelta {
                id: "t1".to_string(),
                delta: "hi".to_string(),
            },
            UiStreamPart::Finish {},
            UiStreamPart::ToolInputAvailable {
                tool_call_id: "c1".to_string(),
                tool_name: "bash".to_string(),
                input: json!({"cmd": "ls"}),
            },
            UiStreamPart::Error {
                error_text: "oops".to_string(),
            },
        ];

        for part in &parts {
            let json = serde_json::to_string(part).unwrap();
            let decoded: UiStreamPart = serde_json::from_str(&json).unwrap();
            assert_eq!(*part, decoded);
        }
    }

    // ── Deprecated alias tests ──

    #[test]
    fn deprecated_to_aisdk_parts_still_works() {
        let event = AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
            delta: "test".to_string(),
        };
        let parts = to_aisdk_parts(&event);
        assert_eq!(parts.len(), 1);
    }
}
