use aios_protocol::{
    ApprovalDecision as ProtocolApprovalDecision, EventKind as ProtocolEventKind,
    EventRecord as ProtocolEventRecord, RiskLevel as ProtocolRiskLevel,
    SpanStatus as ProtocolSpanStatus,
};
use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
use futures::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Error as EventSourceError, Event, EventSource};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::sync::mpsc;

/// Configuration for the daemon connection
pub struct NetworkConfig {
    pub base_url: String,
    pub session_id: String,
}

pub struct NetworkClient {
    client: Client,
    config: NetworkConfig,
}

fn parse_protocol_record(data: &str) -> Option<AgentEvent> {
    let record: ProtocolEventRecord = serde_json::from_str(data).ok()?;
    agent_event_from_protocol_record(&record)
}

fn parse_canonical_event(event_name: &str, data: &str, session_id: &str) -> Option<AgentEvent> {
    let payload: Value = serde_json::from_str(data).ok()?;
    let run_id = "stream".to_string();
    let session_id = session_id.to_string();

    match event_name {
        "assistant.text.delta" => {
            let delta = payload.get("delta")?.as_str()?.to_string();
            Some(AgentEvent::TextDelta {
                run_id,
                session_id,
                iteration: 0,
                delta,
            })
        }
        "assistant.message.committed" => {
            let content = payload.get("content")?.as_str()?.to_string();
            Some(AgentEvent::RunFinished {
                run_id,
                session_id,
                reason: RunStopReason::Completed,
                total_iterations: 0,
                final_answer: Some(content),
            })
        }
        "tool.started" => {
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let arguments = payload
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            Some(AgentEvent::ToolCallRequested {
                run_id,
                session_id,
                iteration: 0,
                call: ToolCall {
                    call_id,
                    tool_name,
                    input: arguments,
                },
            })
        }
        "tool.completed" => {
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let status = payload
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();
            if status == "ok" {
                Some(AgentEvent::ToolCallCompleted {
                    run_id,
                    session_id,
                    iteration: 0,
                    result: ToolResultSummary {
                        call_id,
                        tool_name,
                        output: payload.get("result").cloned().unwrap_or(Value::Null),
                    },
                })
            } else {
                let error = payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("tool call failed")
                    .to_string();
                Some(AgentEvent::ToolCallFailed {
                    run_id,
                    session_id,
                    iteration: 0,
                    call_id,
                    tool_name,
                    error,
                })
            }
        }
        "intent.evaluated" => {
            if !payload
                .get("requires_approval")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                return None;
            }
            let approval_id = payload.get("approval_id")?.as_str()?.to_string();
            let call_id = payload.get("intent_id")?.as_str()?.to_string();
            let tool_name = payload.get("tool_name")?.as_str()?.to_string();
            let arguments = payload
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let risk = payload
                .get("risk")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            Some(AgentEvent::ApprovalRequested {
                run_id,
                session_id,
                approval_id,
                call_id,
                tool_name,
                arguments,
                risk,
            })
        }
        "intent.approved" | "intent.rejected" => {
            let approval_id = payload.get("approval_id")?.as_str()?.to_string();
            let decision = payload
                .get("decision")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    if event_name == "intent.approved" {
                        "approved"
                    } else {
                        "denied"
                    }
                })
                .to_string();
            let reason = payload
                .get("reason")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(AgentEvent::ApprovalResolved {
                run_id,
                session_id,
                approval_id,
                decision,
                reason,
            })
        }
        _ => None,
    }
}

fn risk_level_to_string(level: ProtocolRiskLevel) -> &'static str {
    match level {
        ProtocolRiskLevel::Low => "low",
        ProtocolRiskLevel::Medium => "medium",
        ProtocolRiskLevel::High => "high",
        ProtocolRiskLevel::Critical => "critical",
    }
}

fn approval_decision_to_string(decision: ProtocolApprovalDecision) -> &'static str {
    match decision {
        ProtocolApprovalDecision::Approved => "approved",
        ProtocolApprovalDecision::Denied => "denied",
        ProtocolApprovalDecision::Timeout => "timeout",
    }
}

fn run_stop_reason_from_string(reason: &str) -> RunStopReason {
    match reason {
        "completed" => RunStopReason::Completed,
        "needs_user" => RunStopReason::NeedsUser,
        "blocked_by_policy" => RunStopReason::BlockedByPolicy,
        "budget_exceeded" => RunStopReason::BudgetExceeded,
        "cancelled" => RunStopReason::Cancelled,
        _ => RunStopReason::Error,
    }
}

fn agent_event_from_protocol_record(record: &ProtocolEventRecord) -> Option<AgentEvent> {
    let run_id = "stream".to_string();
    let session_id = record.session_id.to_string();

    match &record.kind {
        ProtocolEventKind::RunStarted {
            provider,
            max_iterations,
        } => Some(AgentEvent::RunStarted {
            run_id,
            session_id,
            provider: provider.clone(),
            max_iterations: *max_iterations,
        }),
        ProtocolEventKind::StepStarted { index } => Some(AgentEvent::IterationStarted {
            run_id,
            session_id,
            iteration: *index,
        }),
        ProtocolEventKind::StepFinished {
            index,
            stop_reason,
            directive_count,
        } => Some(AgentEvent::ModelOutput {
            run_id,
            session_id,
            iteration: *index,
            stop_reason: match stop_reason.as_str() {
                "end_turn" => arcan_core::protocol::ModelStopReason::EndTurn,
                "tool_use" => arcan_core::protocol::ModelStopReason::ToolUse,
                "needs_user" => arcan_core::protocol::ModelStopReason::NeedsUser,
                "max_tokens" => arcan_core::protocol::ModelStopReason::MaxTokens,
                "safety" => arcan_core::protocol::ModelStopReason::Safety,
                _ => arcan_core::protocol::ModelStopReason::Unknown,
            },
            directive_count: *directive_count,
            usage: None,
        }),
        ProtocolEventKind::AssistantTextDelta { delta, index }
        | ProtocolEventKind::TextDelta { delta, index } => Some(AgentEvent::TextDelta {
            run_id,
            session_id,
            iteration: index.unwrap_or(0),
            delta: delta.clone(),
        }),
        ProtocolEventKind::AssistantMessageCommitted { content, .. }
        | ProtocolEventKind::Message { content, .. } => Some(AgentEvent::RunFinished {
            run_id,
            session_id,
            reason: RunStopReason::Completed,
            total_iterations: 0,
            final_answer: Some(content.clone()),
        }),
        ProtocolEventKind::ToolCallRequested {
            call_id,
            tool_name,
            arguments,
            ..
        } => Some(AgentEvent::ToolCallRequested {
            run_id,
            session_id,
            iteration: 0,
            call: ToolCall {
                call_id: call_id.clone(),
                tool_name: tool_name.clone(),
                input: arguments.clone(),
            },
        }),
        ProtocolEventKind::ToolCallCompleted {
            call_id,
            tool_name,
            result,
            status,
            ..
        } => {
            if *status == ProtocolSpanStatus::Ok {
                Some(AgentEvent::ToolCallCompleted {
                    run_id,
                    session_id,
                    iteration: 0,
                    result: ToolResultSummary {
                        call_id: call_id.clone().unwrap_or_default(),
                        tool_name: tool_name.clone(),
                        output: result.clone(),
                    },
                })
            } else {
                Some(AgentEvent::ToolCallFailed {
                    run_id,
                    session_id,
                    iteration: 0,
                    call_id: call_id.clone().unwrap_or_default(),
                    tool_name: tool_name.clone(),
                    error: result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("tool call failed")
                        .to_string(),
                })
            }
        }
        ProtocolEventKind::ToolCallFailed {
            call_id,
            tool_name,
            error,
        } => Some(AgentEvent::ToolCallFailed {
            run_id,
            session_id,
            iteration: 0,
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            error: error.clone(),
        }),
        ProtocolEventKind::StatePatched {
            index,
            patch,
            revision,
        } => Some(AgentEvent::StatePatched {
            run_id,
            session_id,
            iteration: index.unwrap_or(0),
            patch: arcan_core::protocol::StatePatch {
                format: arcan_core::protocol::StatePatchFormat::MergePatch,
                patch: patch.clone(),
                source: arcan_core::protocol::StatePatchSource::System,
            },
            revision: *revision,
        }),
        ProtocolEventKind::ContextCompacted {
            dropped_count,
            tokens_before,
            tokens_after,
        } => Some(AgentEvent::ContextCompacted {
            run_id,
            session_id,
            iteration: 0,
            dropped_count: *dropped_count,
            tokens_before: *tokens_before,
            tokens_after: *tokens_after,
        }),
        ProtocolEventKind::ApprovalRequested {
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
            risk: risk_level_to_string(*risk).to_string(),
        }),
        ProtocolEventKind::ApprovalResolved {
            approval_id,
            decision,
            reason,
        } => Some(AgentEvent::ApprovalResolved {
            run_id,
            session_id,
            approval_id: approval_id.to_string(),
            decision: approval_decision_to_string(*decision).to_string(),
            reason: reason.clone(),
        }),
        ProtocolEventKind::RunFinished {
            reason,
            total_iterations,
            final_answer,
            ..
        } => Some(AgentEvent::RunFinished {
            run_id,
            session_id,
            reason: run_stop_reason_from_string(reason),
            total_iterations: *total_iterations,
            final_answer: final_answer.clone(),
        }),
        ProtocolEventKind::RunErrored { error } => Some(AgentEvent::RunErrored {
            run_id,
            session_id,
            error: error.clone(),
        }),
        _ => None,
    }
}

fn parse_vercel_v6_part(data: &str) -> Option<AgentEvent> {
    let value: Value = serde_json::from_str(data).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("data-aios-event") {
        return None;
    }
    let record_value = value.get("data")?.clone();
    let record: ProtocolEventRecord = serde_json::from_value(record_value).ok()?;
    agent_event_from_protocol_record(&record)
}

impl NetworkClient {
    pub fn new(config: NetworkConfig) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            config,
        }
    }

    /// Submits a message to the agent's run endpoint
    pub async fn submit_run(&self, message: &str, branch: Option<&str>) -> anyhow::Result<()> {
        let url = format!(
            "{}/sessions/{}/runs",
            self.config.base_url, self.config.session_id
        );

        let body = json!({
            "objective": message,
            "branch": branch,
        });

        let res = self.client.post(&url).json(&body).send().await?;

        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit run: {}", error_text);
        }

        Ok(())
    }

    /// Submits an approval decision
    pub async fn submit_approval(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<()> {
        let url = format!(
            "{}/sessions/{}/approvals/{}",
            self.config.base_url, self.config.session_id, approval_id
        );
        let approved = matches!(
            decision.to_ascii_lowercase().as_str(),
            "approved" | "approve" | "yes" | "y" | "true"
        );
        let actor = reason.unwrap_or("tui");

        let body = json!({
            "approved": approved,
            "actor": actor,
        });

        let res = self.client.post(&url).json(&body).send().await?;

        if !res.status().is_success() {
            let error_text = res.text().await?;
            anyhow::bail!("Failed to submit approval: {}", error_text);
        }

        Ok(())
    }

    /// Fetches the currently selected model from the daemon.
    pub async fn get_model(&self) -> anyhow::Result<String> {
        anyhow::bail!("Model inspection is not exposed in the canonical session API")
    }

    /// Switches the active provider/model in the daemon.
    pub async fn set_model(&self, provider: &str, model: Option<&str>) -> anyhow::Result<String> {
        let requested = match model {
            Some(model) => format!("{provider}:{model}"),
            None => provider.to_string(),
        };
        anyhow::bail!("Model switching ({requested}) is not exposed in the canonical session API")
    }

    /// Continuously listens to the SSE stream and pushes parsed events to the channel
    pub async fn listen_events(&self, sender: mpsc::Sender<AgentEvent>) -> anyhow::Result<()> {
        let url = format!(
            "{}/sessions/{}/events/stream?branch=main&cursor=0&format=vercel_ai_sdk_v6",
            self.config.base_url, self.config.session_id
        );

        let mut es = EventSource::get(url);

        while let Some(event) = es.next().await {
            match event {
                Ok(Event::Open) => {
                    tracing::info!("SSE Connection Opened");
                }
                Ok(Event::Message(message)) => {
                    let event_name = message.event.as_str();
                    let data = message.data.trim();
                    if event_name == "done" || data == "[DONE]" || data == "{\"type\": \"done\"}" {
                        continue;
                    }

                    if let Some(agent_event) =
                        parse_protocol_record(data).or_else(|| parse_vercel_v6_part(data))
                    {
                        if sender.send(agent_event).await.is_err() {
                            break;
                        }
                    } else if let Some(agent_event) =
                        parse_canonical_event(event_name, data, &self.config.session_id)
                    {
                        if sender.send(agent_event).await.is_err() {
                            break;
                        }
                    } else {
                        tracing::debug!("Ignored SSE event '{}': {}", event_name, data);
                    }
                }
                Err(EventSourceError::StreamEnded) => {
                    tracing::debug!("SSE stream ended; waiting for reconnect");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => {
                    tracing::warn!("SSE stream error: {}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_canonical_event, parse_vercel_v6_part};
    use aios_protocol::{
        BranchId as ProtocolBranchId, EventKind as ProtocolEventKind,
        EventRecord as ProtocolEventRecord, SessionId as ProtocolSessionId,
    };
    use arcan_core::protocol::AgentEvent;
    use serde_json::json;

    #[test]
    fn parses_assistant_delta_event() {
        let event = parse_canonical_event("assistant.text.delta", r#"{"delta":"hello"}"#, "sess-1")
            .expect("event");

        match event {
            AgentEvent::TextDelta {
                session_id, delta, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(delta, "hello");
            }
            _ => panic!("expected TextDelta"),
        }
    }

    #[test]
    fn parses_tool_started_event() {
        let event = parse_canonical_event(
            "tool.started",
            r#"{"intent_id":"call-1","tool_name":"fs.read","arguments":{"path":"foo.txt"}}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ToolCallRequested { call, .. } => {
                assert_eq!(call.call_id, "call-1");
                assert_eq!(call.tool_name, "fs.read");
                assert_eq!(call.input, json!({"path":"foo.txt"}));
            }
            _ => panic!("expected ToolCallRequested"),
        }
    }

    #[test]
    fn parses_tool_completed_error_event() {
        let event = parse_canonical_event(
            "tool.completed",
            r#"{"intent_id":"call-2","tool_name":"shell.exec","status":"error","error":"denied"}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ToolCallFailed {
                call_id,
                tool_name,
                error,
                ..
            } => {
                assert_eq!(call_id, "call-2");
                assert_eq!(tool_name, "shell.exec");
                assert_eq!(error, "denied");
            }
            _ => panic!("expected ToolCallFailed"),
        }
    }

    #[test]
    fn parses_approval_requested_event() {
        let event = parse_canonical_event(
            "intent.evaluated",
            r#"{"intent_id":"call-3","requires_approval":true,"approval_id":"ap-1","tool_name":"shell.exec","arguments":{"cmd":"rm -rf /"},"risk":"high"}"#,
            "sess-1",
        )
        .expect("event");

        match event {
            AgentEvent::ApprovalRequested {
                approval_id,
                call_id,
                risk,
                ..
            } => {
                assert_eq!(approval_id, "ap-1");
                assert_eq!(call_id, "call-3");
                assert_eq!(risk, "high");
            }
            _ => panic!("expected ApprovalRequested"),
        }
    }

    #[test]
    fn parses_vercel_v6_data_aios_event_part() {
        let record = ProtocolEventRecord::new(
            ProtocolSessionId::from_string("sess-1"),
            ProtocolBranchId::main(),
            7,
            ProtocolEventKind::AssistantTextDelta {
                delta: "hello v6".to_string(),
                index: Some(1),
            },
        );
        let payload = serde_json::json!({
            "type": "data-aios-event",
            "id": "7",
            "data": record,
            "transient": false
        });

        let event = parse_vercel_v6_part(&payload.to_string()).expect("event");
        match event {
            AgentEvent::TextDelta {
                session_id, delta, ..
            } => {
                assert_eq!(session_id, "sess-1");
                assert_eq!(delta, "hello v6");
            }
            _ => panic!("expected TextDelta"),
        }
    }
}
