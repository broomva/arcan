use aios_protocol::{ApprovalId, ToolRunId};
use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
use lago_core::event::{ApprovalDecision, RiskLevel, SpanStatus};
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
        } => EventPayload::TextDelta {
            delta: delta.clone(),
            index: Some(*iteration),
        },

        AgentEvent::ToolCallRequested { call, .. } => EventPayload::ToolCallRequested {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments: call.input.clone(),
            category: None,
        },

        AgentEvent::ToolCallCompleted { result, .. } => EventPayload::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some(result.call_id.clone()),
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
        } => EventPayload::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some(call_id.clone()),
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
                reason: run_stop_reason_to_str(*reason).to_string(),
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

        AgentEvent::IterationStarted { iteration, .. } => {
            EventPayload::StepStarted { index: *iteration }
        }

        AgentEvent::ModelOutput {
            iteration,
            stop_reason,
            directive_count,
            ..
        } => EventPayload::StepFinished {
            index: *iteration,
            stop_reason: model_stop_reason_to_str(*stop_reason).to_string(),
            directive_count: *directive_count,
        },

        AgentEvent::StatePatched {
            iteration,
            patch,
            revision,
            ..
        } => EventPayload::StatePatched {
            index: Some(*iteration),
            patch: serde_json::to_value(patch).unwrap_or_default(),
            revision: *revision,
        },

        AgentEvent::ContextCompacted {
            iteration,
            dropped_count,
            tokens_before,
            tokens_after,
            ..
        } => EventPayload::Custom {
            event_type: "context_compacted".to_string(),
            data: serde_json::json!({
                "iteration": iteration,
                "dropped_count": dropped_count,
                "tokens_before": tokens_before,
                "tokens_after": tokens_after,
            }),
        },

        AgentEvent::ApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            risk,
            ..
        } => EventPayload::ApprovalRequested {
            approval_id: ApprovalId::from_string(approval_id.clone()),
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            risk: parse_risk_level(risk),
        },

        AgentEvent::ApprovalResolved {
            approval_id,
            decision,
            reason,
            ..
        } => EventPayload::ApprovalResolved {
            approval_id: ApprovalId::from_string(approval_id.clone()),
            decision: parse_approval_decision(decision),
            reason: reason.clone(),
        },

        AgentEvent::RunErrored { error, .. } => EventPayload::ErrorRaised {
            message: error.clone(),
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
        schema_version: 1,
    }
}

/// Convert a Lago `EventEnvelope` back into an Arcan `AgentEvent`.
pub fn lago_to_arcan(envelope: &EventEnvelope) -> Option<AgentEvent> {
    let run_id = envelope
        .run_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let session_id = envelope.session_id.to_string();

    match &envelope.payload {
        EventPayload::TextDelta { delta, index } => Some(AgentEvent::TextDelta {
            run_id,
            session_id,
            iteration: index.unwrap_or(0),
            delta: delta.clone(),
        }),

        EventPayload::ToolCallRequested {
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

        EventPayload::ToolCallCompleted {
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
                    call_id: call_id.clone().unwrap_or_default(),
                    tool_name: tool_name.clone(),
                    output: result.clone(),
                },
            }),
            _ => Some(AgentEvent::ToolCallFailed {
                run_id,
                session_id,
                iteration: 0,
                call_id: call_id.clone().unwrap_or_default(),
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
            usage: None, // Token usage not preserved in Lago StepFinished payload
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
                iteration: index.unwrap_or(0),
                patch: p,
                revision: *revision,
            }),

        EventPayload::ApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            risk,
        } => Some(AgentEvent::ApprovalRequested {
            run_id,
            session_id,
            approval_id: approval_id.to_string(),
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            risk: risk_level_to_str(*risk).to_string(),
        }),

        EventPayload::ApprovalResolved {
            approval_id,
            decision,
            reason,
        } => Some(AgentEvent::ApprovalResolved {
            run_id,
            session_id,
            approval_id: approval_id.to_string(),
            decision: approval_decision_to_str(*decision).to_string(),
            reason: reason.clone(),
        }),

        EventPayload::ErrorRaised { message } => Some(AgentEvent::RunErrored {
            run_id,
            session_id,
            error: message.clone(),
        }),

        // Context compaction events stored as Custom
        EventPayload::Custom {
            event_type, data, ..
        } if event_type == "context_compacted" => Some(AgentEvent::ContextCompacted {
            run_id,
            session_id,
            iteration: data
                .get("iteration")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as u32,
            dropped_count: data
                .get("dropped_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
            tokens_before: data
                .get("tokens_before")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
            tokens_after: data
                .get("tokens_after")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
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

fn run_stop_reason_to_str(reason: RunStopReason) -> &'static str {
    match reason {
        RunStopReason::Completed => "Completed",
        RunStopReason::NeedsUser => "NeedsUser",
        RunStopReason::BlockedByPolicy => "BlockedByPolicy",
        RunStopReason::BudgetExceeded => "BudgetExceeded",
        RunStopReason::Cancelled => "Cancelled",
        RunStopReason::Error => "Error",
    }
}

fn parse_run_stop_reason(s: &str) -> RunStopReason {
    match s {
        "Completed" => RunStopReason::Completed,
        "NeedsUser" => RunStopReason::NeedsUser,
        "BlockedByPolicy" => RunStopReason::BlockedByPolicy,
        "BudgetExceeded" => RunStopReason::BudgetExceeded,
        "Cancelled" => RunStopReason::Cancelled,
        "Error" => RunStopReason::Error,
        _ => RunStopReason::Completed, // Default fallback
    }
}

fn model_stop_reason_to_str(reason: arcan_core::protocol::ModelStopReason) -> &'static str {
    use arcan_core::protocol::ModelStopReason;
    match reason {
        ModelStopReason::EndTurn => "EndTurn",
        ModelStopReason::ToolUse => "ToolUse",
        ModelStopReason::NeedsUser => "NeedsUser",
        ModelStopReason::MaxTokens => "MaxTokens",
        ModelStopReason::Safety => "Safety",
        ModelStopReason::Unknown => "Unknown",
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

fn parse_risk_level(s: &str) -> RiskLevel {
    match s {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        _ => RiskLevel::Low,
    }
}

fn risk_level_to_str(r: RiskLevel) -> &'static str {
    match r {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

fn parse_approval_decision(s: &str) -> ApprovalDecision {
    match s {
        "approved" => ApprovalDecision::Approved,
        "denied" => ApprovalDecision::Denied,
        "timeout" => ApprovalDecision::Timeout,
        _ => ApprovalDecision::Denied,
    }
}

fn approval_decision_to_str(d: ApprovalDecision) -> &'static str {
    match d {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Denied => "denied",
        ApprovalDecision::Timeout => "timeout",
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
            AgentEvent::RunFinished {
                reason,
                final_answer,
                ..
            } => {
                assert_eq!(reason, RunStopReason::Completed);
                assert_eq!(final_answer.as_deref(), Some("42"));
            }
            _ => panic!("expected RunFinished"),
        }
    }

    #[test]
    fn approval_requested_round_trips() {
        let event = AgentEvent::ApprovalRequested {
            run_id: "r1".into(),
            session_id: "s1".into(),
            approval_id: "appr-1".into(),
            call_id: "c1".into(),
            tool_name: "bash".into(),
            arguments: serde_json::json!({"command": "rm -rf /"}),
            risk: "high".into(),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-a1");

        if let EventPayload::ApprovalRequested {
            tool_name, risk, ..
        } = &envelope.payload
        {
            assert_eq!(tool_name, "bash");
            assert_eq!(*risk, lago_core::event::RiskLevel::High);
        } else {
            panic!("expected ApprovalRequested payload");
        }

        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::ApprovalRequested {
                approval_id,
                tool_name,
                risk,
                ..
            } => {
                assert_eq!(approval_id, "appr-1");
                assert_eq!(tool_name, "bash");
                assert_eq!(risk, "high");
            }
            _ => panic!("expected ApprovalRequested"),
        }
    }

    #[test]
    fn approval_resolved_round_trips() {
        let event = AgentEvent::ApprovalResolved {
            run_id: "r1".into(),
            session_id: "s1".into(),
            approval_id: "appr-2".into(),
            decision: "denied".into(),
            reason: Some("too dangerous".into()),
        };
        let envelope = arcan_to_lago(&test_session(), &test_branch(), 1, "r1", &event, "uuid-a2");

        if let EventPayload::ApprovalResolved {
            decision, reason, ..
        } = &envelope.payload
        {
            assert_eq!(*decision, lago_core::event::ApprovalDecision::Denied);
            assert_eq!(reason.as_deref(), Some("too dangerous"));
        } else {
            panic!("expected ApprovalResolved payload");
        }

        let back = lago_to_arcan(&envelope).expect("should map back");
        match back {
            AgentEvent::ApprovalResolved {
                approval_id,
                decision,
                reason,
                ..
            } => {
                assert_eq!(approval_id, "appr-2");
                assert_eq!(decision, "denied");
                assert_eq!(reason.as_deref(), Some("too dangerous"));
            }
            _ => panic!("expected ApprovalResolved"),
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
