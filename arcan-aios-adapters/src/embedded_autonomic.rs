//! Embedded Autonomic controller — runs in-process within Arcan.
//!
//! Instead of consulting the standalone Autonomic daemon via HTTP,
//! this module embeds the pure `autonomic-controller` (fold + rules)
//! directly in the Arcan process. Events are folded as they flow
//! through Arcan's event handler, producing per-session projections
//! with microsecond-latency gating.

use std::collections::HashMap;
use std::sync::Arc;

use autonomic_controller::{
    BudgetExhaustionRule, ContextPressureRule, ErrorStreakRule, KnowledgeHealthRule,
    KnowledgeRegressionRule, SpendVelocityRule, SurvivalRule, TokenExhaustionRule,
};
use autonomic_core::gating::{AutonomicGatingProfile, HomeostaticState};
use autonomic_core::rules::RuleSet;
use lago_core::event::EventPayload;
use tokio::sync::RwLock;

use crate::autonomic::EconomicGateHandle;

/// Thread-safe projection map: session_id → HomeostaticState.
pub type ProjectionMap = Arc<RwLock<HashMap<String, HomeostaticState>>>;

/// Embedded Autonomic controller running in-process.
///
/// Maintains per-session `HomeostaticState` projections and evaluates
/// the standard 6-rule set to produce gating profiles. All operations
/// are pure in-memory computations — no network, no I/O.
pub struct EmbeddedAutonomicController {
    projections: ProjectionMap,
    rules: Arc<RuleSet>,
    economic_handle: EconomicGateHandle,
}

impl EmbeddedAutonomicController {
    /// Create a new embedded controller with the default rule set.
    pub fn new(economic_handle: EconomicGateHandle) -> Self {
        Self {
            projections: Arc::new(RwLock::new(HashMap::new())),
            rules: Arc::new(default_rules()),
            economic_handle,
        }
    }

    /// Fold an event into the projection for a session.
    ///
    /// Call this from the event handler path whenever Arcan emits an event.
    /// The event is converted to `aios_protocol::EventKind` (= Lago EventPayload)
    /// and folded into the session's `HomeostaticState`.
    pub async fn fold_event(&self, session_id: &str, payload: &EventPayload, seq: u64, ts_ms: u64) {
        let mut projections = self.projections.write().await;
        let state = projections
            .entry(session_id.to_owned())
            .or_insert_with(|| HomeostaticState::for_agent(session_id));
        *state = autonomic_controller::fold(state.clone(), payload, seq, ts_ms);
    }

    /// Evaluate gating for a session.
    ///
    /// Returns `None` if no projection exists yet (no events seen for this session).
    /// The caller should fall through to the inner gate in that case.
    pub async fn evaluate_gating(&self, session_id: &str) -> Option<AutonomicGatingProfile> {
        let projections = self.projections.read().await;
        let state = projections.get(session_id)?;
        let profile = autonomic_controller::evaluate(state, &self.rules);

        // Update the economic handle for the provider layer.
        self.update_economic_handle(&profile).await;

        Some(profile)
    }

    /// Get the current projection for a session (for diagnostics/testing).
    pub async fn get_projection(&self, session_id: &str) -> Option<HomeostaticState> {
        let projections = self.projections.read().await;
        projections.get(session_id).cloned()
    }

    /// Update the shared economic handle from the gating profile.
    async fn update_economic_handle(&self, profile: &AutonomicGatingProfile) {
        use crate::autonomic::{EconomicGates, EconomicMode, ModelTier};

        let gates = EconomicGates {
            economic_mode: match profile.economic.economic_mode {
                autonomic_core::EconomicMode::Sovereign => EconomicMode::Sovereign,
                autonomic_core::EconomicMode::Conserving => EconomicMode::Conserving,
                autonomic_core::EconomicMode::Hustle => EconomicMode::Hustle,
                autonomic_core::EconomicMode::Hibernate => EconomicMode::Hibernate,
            },
            max_tokens_next_turn: profile.economic.max_tokens_next_turn,
            preferred_model: profile.economic.preferred_model.map(|t| match t {
                autonomic_core::ModelTier::Flagship => ModelTier::Flagship,
                autonomic_core::ModelTier::Standard => ModelTier::Standard,
                autonomic_core::ModelTier::Budget => ModelTier::Budget,
            }),
            allow_expensive_tools: profile.economic.allow_expensive_tools,
            allow_replication: profile.economic.allow_replication,
        };

        let mut handle = self.economic_handle.write().await;
        *handle = Some(gates);
    }
}

/// Build the default rule set (same core gates as the standalone Autonomic daemon).
fn default_rules() -> RuleSet {
    let mut rules = RuleSet::new();
    rules.add(Box::new(SurvivalRule));
    rules.add(Box::new(SpendVelocityRule::default()));
    rules.add(Box::new(BudgetExhaustionRule::default()));
    rules.add(Box::new(ContextPressureRule::default()));
    rules.add(Box::new(TokenExhaustionRule::default()));
    rules.add(Box::new(ErrorStreakRule::default()));
    rules.add(Box::new(KnowledgeHealthRule::default()));
    rules.add(Box::new(KnowledgeRegressionRule::default()));
    rules
}

/// Convert an Arcan `AgentEvent` to a Lago `EventPayload` for folding.
///
/// This is a lightweight conversion — only the fields needed by the
/// Autonomic projection reducer are included.
pub fn agent_event_to_payload(event: &arcan_core::protocol::AgentEvent) -> Option<EventPayload> {
    use arcan_core::protocol::AgentEvent;
    use lago_core::event::SpanStatus;

    match event {
        AgentEvent::RunFinished {
            reason,
            total_iterations,
            final_answer,
            usage,
            ..
        } => {
            let lago_usage = usage.as_ref().map(|u| lago_core::event::TokenUsage {
                prompt_tokens: u.input_tokens as u32,
                completion_tokens: u.output_tokens as u32,
                total_tokens: u.total() as u32,
            });
            Some(EventPayload::RunFinished {
                reason: format!("{reason:?}"),
                total_iterations: *total_iterations,
                final_answer: final_answer.clone(),
                usage: lago_usage,
            })
        }
        AgentEvent::RunErrored { error, .. } => Some(EventPayload::RunErrored {
            error: error.clone(),
        }),
        AgentEvent::ToolCallCompleted { result, .. } => Some(EventPayload::ToolCallCompleted {
            tool_run_id: aios_protocol::ToolRunId::default(),
            call_id: Some(result.call_id.clone()),
            tool_name: result.tool_name.clone(),
            result: result.output.clone(),
            duration_ms: 0,
            status: SpanStatus::Ok,
        }),
        AgentEvent::ToolCallFailed {
            call_id,
            tool_name,
            error,
            ..
        } => Some(EventPayload::ToolCallCompleted {
            tool_run_id: aios_protocol::ToolRunId::default(),
            call_id: Some(call_id.clone()),
            tool_name: tool_name.clone(),
            result: serde_json::json!({ "error": error }),
            duration_ms: 0,
            status: SpanStatus::Error,
        }),
        // Other event types don't affect the homeostatic projection.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{AgentEvent, RunStopReason, TokenUsage, ToolResultSummary};

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test]
    async fn fold_processes_run_finished() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);

        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 3,
            final_answer: Some("done".into()),
            usage: Some(TokenUsage {
                input_tokens: 500,
                output_tokens: 200,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            }),
        };

        if let Some(payload) = agent_event_to_payload(&event) {
            controller.fold_event("s1", &payload, 1, now_ms()).await;
        }

        let state = controller.get_projection("s1").await.unwrap();
        assert_eq!(state.operational.total_successes, 1);
        assert_eq!(state.cognitive.total_tokens_used, 700);
        assert_eq!(state.last_event_seq, 1);
    }

    #[tokio::test]
    async fn fold_processes_errors() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);

        for i in 0..5 {
            let event = AgentEvent::RunErrored {
                run_id: format!("r{i}"),
                session_id: "s1".into(),
                error: "timeout".into(),
            };
            if let Some(payload) = agent_event_to_payload(&event) {
                controller.fold_event("s1", &payload, i + 1, now_ms()).await;
            }
        }

        let state = controller.get_projection("s1").await.unwrap();
        assert_eq!(state.operational.error_streak, 5);
        assert_eq!(state.operational.total_errors, 5);
    }

    #[tokio::test]
    async fn evaluate_returns_none_for_unknown_session() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);
        assert!(controller.evaluate_gating("unknown").await.is_none());
    }

    #[tokio::test]
    async fn evaluate_returns_profile_after_events() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle.clone());

        // Feed some successful events
        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: None,
            usage: None,
        };
        if let Some(payload) = agent_event_to_payload(&event) {
            controller.fold_event("s1", &payload, 1, now_ms()).await;
        }

        let profile = controller.evaluate_gating("s1").await.unwrap();
        // Default state with 1 success should be permissive
        assert!(profile.operational.allow_side_effects);
    }

    #[tokio::test]
    async fn error_streak_triggers_restrictive_gating() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);

        // Feed 8 consecutive errors (error_streak rule fires at >= 5)
        for i in 0..8 {
            let event = AgentEvent::RunErrored {
                run_id: format!("r{i}"),
                session_id: "s1".into(),
                error: "connection refused".into(),
            };
            if let Some(payload) = agent_event_to_payload(&event) {
                controller.fold_event("s1", &payload, i + 1, now_ms()).await;
            }
        }

        let profile = controller.evaluate_gating("s1").await.unwrap();
        // With 8 consecutive errors, the error streak rule should restrict side effects
        assert!(
            !profile.operational.allow_side_effects || !profile.operational.allow_shell,
            "error streak should restrict operations"
        );
    }

    #[tokio::test]
    async fn economic_handle_updated_after_evaluate() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle.clone());

        // Before: handle is empty
        assert!(handle.read().await.is_none());

        // Feed an event and evaluate
        let event = AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "s1".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: None,
            usage: None,
        };
        if let Some(payload) = agent_event_to_payload(&event) {
            controller.fold_event("s1", &payload, 1, now_ms()).await;
        }
        controller.evaluate_gating("s1").await;

        // After: handle is populated
        let gates = handle.read().await;
        assert!(gates.is_some());
    }

    #[tokio::test]
    async fn tool_call_completed_folds_success() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);

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

        if let Some(payload) = agent_event_to_payload(&event) {
            controller.fold_event("s1", &payload, 1, now_ms()).await;
        }

        let state = controller.get_projection("s1").await.unwrap();
        assert_eq!(state.operational.total_successes, 1);
        assert_eq!(state.operational.error_streak, 0);
    }

    #[tokio::test]
    async fn tool_call_failed_folds_error() {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(None));
        let controller = EmbeddedAutonomicController::new(handle);

        let event = AgentEvent::ToolCallFailed {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
            call_id: "c1".into(),
            tool_name: "bash".into(),
            error: "permission denied".into(),
        };

        if let Some(payload) = agent_event_to_payload(&event) {
            controller.fold_event("s1", &payload, 1, now_ms()).await;
        }

        let state = controller.get_projection("s1").await.unwrap();
        assert_eq!(state.operational.error_streak, 1);
        assert_eq!(state.operational.total_errors, 1);
    }
}
