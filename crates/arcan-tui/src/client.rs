use aios_protocol::{
    ApprovalDecision as ProtocolApprovalDecision, EventKind as ProtocolEventKind,
    EventRecord as ProtocolEventRecord, RiskLevel as ProtocolRiskLevel,
    SpanStatus as ProtocolSpanStatus,
};
use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;

// ── Typed response structs ──────────────────────────────────────────────────

/// Summary of a session returned by the daemon's `/sessions` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    #[serde(default = "default_owner")]
    pub owner: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

fn default_owner() -> String {
    "unknown".to_string()
}

/// Top-level response from the daemon's `/sessions/{id}/state` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentStateResponse {
    #[serde(default)]
    pub session_id: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub state: AgentStateFields,
    #[serde(default)]
    pub version: u64,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_mode() -> String {
    "Unknown".to_string()
}

/// Nested state vector fields.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentStateFields {
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub uncertainty: f64,
    #[serde(default = "default_risk")]
    pub risk_level: String,
    #[serde(default)]
    pub error_streak: u64,
    #[serde(default)]
    pub context_pressure: f64,
    #[serde(default)]
    pub side_effect_pressure: f64,
    #[serde(default)]
    pub human_dependency: f64,
    #[serde(default)]
    pub budget: BudgetFields,
}

fn default_risk() -> String {
    "Low".to_string()
}

/// Budget counters within the agent state.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BudgetFields {
    #[serde(default)]
    pub tokens_remaining: u64,
    #[serde(default)]
    pub time_remaining_ms: u64,
    #[serde(default)]
    pub cost_remaining_usd: f64,
    #[serde(default)]
    pub tool_calls_remaining: u64,
    #[serde(default)]
    pub error_budget_remaining: u64,
}

/// Provider info returned by the daemon's `GET /provider` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInfo {
    pub provider: String,
    #[serde(default)]
    pub available: Vec<String>,
}

// ── AgentClientPort trait ───────────────────────────────────────────────────

/// Transport-agnostic port for communicating with the agent runtime.
///
/// Implementations may use HTTP/SSE (daemon mode) or direct function calls
/// (in-process mode). The TUI's `App` only sees this trait.
#[async_trait]
pub trait AgentClientPort: Send + Sync + 'static {
    /// Submit a user message to start or continue a run.
    async fn submit_run(&self, message: &str, branch: Option<&str>) -> anyhow::Result<()>;

    /// Submit an approval decision for a pending tool call.
    async fn submit_approval(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<()>;

    /// List all sessions known to the runtime.
    async fn list_sessions(&self) -> anyhow::Result<Vec<SessionSummary>>;

    /// Get the current agent state snapshot for a session.
    async fn get_session_state(&self, branch: Option<&str>) -> anyhow::Result<AgentStateResponse>;

    /// Get the currently selected model identifier.
    async fn get_model(&self) -> anyhow::Result<String>;

    /// Get the current provider and list of available providers.
    async fn get_provider_info(&self) -> anyhow::Result<ProviderInfo>;

    /// Switch the active provider/model. Returns the new active model string.
    async fn set_model(&self, provider: &str, model: Option<&str>) -> anyhow::Result<String>;

    /// Subscribe to the agent event stream. Returns a receiver for parsed events.
    /// The implementation spawns background tasks as needed.
    fn subscribe_events(&self) -> mpsc::Receiver<AgentEvent>;

    /// The current session ID.
    fn session_id(&self) -> String;

    /// Query the daemon version from the `/health` endpoint.
    async fn get_daemon_version(&self) -> anyhow::Result<String>;

    /// The base URL of the connected daemon (e.g. `http://localhost:3000`).
    fn base_url(&self) -> String;

    /// Switch to a different session. Returns a new event receiver wired to
    /// the new session's event stream.
    async fn switch_session(&self, new_id: &str) -> anyhow::Result<mpsc::Receiver<AgentEvent>>;
}

// ── Shared event conversion ─────────────────────────────────────────────────

/// Convert a canonical `ProtocolEventRecord` into the TUI's `AgentEvent`.
///
/// This is transport-independent and shared between HTTP and future in-process
/// implementations.
pub fn agent_event_from_protocol_record(record: &ProtocolEventRecord) -> Option<AgentEvent> {
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
        ProtocolEventKind::AssistantTextDelta { delta, index } => Some(AgentEvent::TextDelta {
            run_id,
            session_id,
            iteration: index.unwrap_or(0),
            delta: delta.clone(),
        }),
        // Persisted TextDelta is a duplicate of the ephemeral AssistantTextDelta
        // that was already broadcast during streaming. Ignoring it prevents the
        // TUI from accumulating the same content twice in streaming_text.
        ProtocolEventKind::TextDelta { .. } => None,
        // Message / AssistantMessageCommitted are redundant when RunFinished
        // already carries final_answer. Converting both to RunFinished caused
        // duplicate assistant messages in the TUI.
        ProtocolEventKind::AssistantMessageCommitted { .. } | ProtocolEventKind::Message { .. } => {
            None
        }
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
            usage: None,
        }),
        ProtocolEventKind::RunErrored { error } => Some(AgentEvent::RunErrored {
            run_id,
            session_id,
            error: error.clone(),
        }),
        _ => None,
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

pub(crate) fn risk_level_to_string(level: ProtocolRiskLevel) -> &'static str {
    match level {
        ProtocolRiskLevel::Low => "low",
        ProtocolRiskLevel::Medium => "medium",
        ProtocolRiskLevel::High => "high",
        ProtocolRiskLevel::Critical => "critical",
    }
}

pub(crate) fn approval_decision_to_string(decision: ProtocolApprovalDecision) -> &'static str {
    match decision {
        ProtocolApprovalDecision::Approved => "approved",
        ProtocolApprovalDecision::Denied => "denied",
        ProtocolApprovalDecision::Timeout => "timeout",
    }
}

pub(crate) fn run_stop_reason_from_string(reason: &str) -> RunStopReason {
    match reason {
        "completed" => RunStopReason::Completed,
        "needs_user" => RunStopReason::NeedsUser,
        "blocked_by_policy" => RunStopReason::BlockedByPolicy,
        "budget_exceeded" => RunStopReason::BudgetExceeded,
        "cancelled" => RunStopReason::Cancelled,
        _ => RunStopReason::Error,
    }
}
