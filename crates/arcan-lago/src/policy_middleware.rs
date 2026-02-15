use crate::approval_gate::{ApprovalGate, ApprovalOutcome};
use arcan_core::error::CoreError;
use arcan_core::protocol::{AgentEvent, ToolAnnotations, ToolCall, ToolResult};
use arcan_core::runtime::{Middleware, ToolContext};
use lago_core::PolicyContext;
use lago_core::event::{ApprovalDecision, PolicyDecisionKind, RiskLevel};
use lago_policy::engine::PolicyEngine;
use std::collections::HashMap;
use std::sync::Arc;

/// Arcan [`Middleware`] backed by lago's [`PolicyEngine`].
///
/// On every tool call, builds a [`PolicyContext`], evaluates the policy rules,
/// and returns `Err(CoreError::Middleware)` when the decision is `Deny`.
/// When `RequireApproval` is returned and a gate is configured, the middleware
/// blocks until the approval is resolved (approved, denied, or timed out).
/// Without a gate, `RequireApproval` falls back to `Deny`.
pub struct LagoPolicyMiddleware {
    engine: PolicyEngine,
    /// Cached annotations per tool name, used to derive risk levels.
    tool_annotations: HashMap<String, ToolAnnotations>,
    /// Optional approval gate for interactive approval flow.
    gate: Option<Arc<ApprovalGate>>,
}

impl LagoPolicyMiddleware {
    pub fn new(engine: PolicyEngine, tool_annotations: HashMap<String, ToolAnnotations>) -> Self {
        Self {
            engine,
            tool_annotations,
            gate: None,
        }
    }

    pub fn with_gate(
        engine: PolicyEngine,
        tool_annotations: HashMap<String, ToolAnnotations>,
        gate: Arc<ApprovalGate>,
    ) -> Self {
        Self {
            engine,
            tool_annotations,
            gate: Some(gate),
        }
    }

    /// Derive a lago `RiskLevel` from arcan `ToolAnnotations`.
    fn risk_level(&self, tool_name: &str) -> RiskLevel {
        match self.tool_annotations.get(tool_name) {
            Some(ann) if ann.requires_confirmation => RiskLevel::High,
            Some(ann) if ann.destructive => RiskLevel::Medium,
            Some(ann) if ann.read_only => RiskLevel::Low,
            _ => RiskLevel::Low,
        }
    }

    fn risk_level_str(&self, tool_name: &str) -> &'static str {
        match self.risk_level(tool_name) {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        }
    }

    fn build_context(&self, ctx: &ToolContext, call: &ToolCall) -> PolicyContext {
        PolicyContext {
            tool_name: call.tool_name.clone(),
            arguments: call.input.clone(),
            category: None,
            risk: Some(self.risk_level(&call.tool_name)),
            session_id: ctx.session_id.clone(),
            role: None,
            sandbox_tier: None,
        }
    }
}

impl Middleware for LagoPolicyMiddleware {
    fn pre_tool_call(&self, context: &ToolContext, call: &ToolCall) -> Result<(), CoreError> {
        let policy_ctx = self.build_context(context, call);
        let decision = self.engine.evaluate(&policy_ctx);

        tracing::debug!(
            tool = %call.tool_name,
            decision = ?decision.decision,
            rule_id = ?decision.rule_id,
            "Policy evaluated"
        );

        match decision.decision {
            PolicyDecisionKind::Allow => Ok(()),
            PolicyDecisionKind::Deny => {
                let reason = decision
                    .explanation
                    .unwrap_or_else(|| "denied by policy".to_string());
                Err(CoreError::Middleware(format!(
                    "tool '{}' blocked: {}",
                    call.tool_name, reason
                )))
            }
            PolicyDecisionKind::RequireApproval => {
                let Some(gate) = &self.gate else {
                    // No gate configured: fall back to deny (backward compat)
                    let rule = decision.rule_id.unwrap_or_else(|| "unknown".to_string());
                    return Err(CoreError::Middleware(format!(
                        "tool '{}' requires approval (rule: {}) but no approval gate configured",
                        call.tool_name, rule
                    )));
                };

                let approval_id = lago_core::ApprovalId::new().to_string();
                let risk = self.risk_level_str(&call.tool_name);

                // Emit ApprovalRequested event
                gate.emit_event(AgentEvent::ApprovalRequested {
                    run_id: context.run_id.clone(),
                    session_id: context.session_id.clone(),
                    approval_id: approval_id.clone(),
                    call_id: call.call_id.clone(),
                    tool_name: call.tool_name.clone(),
                    arguments: call.input.clone(),
                    risk: risk.to_string(),
                });

                // Register and block on oneshot
                let rx = gate.request_approval(&approval_id);
                let outcome = tokio::runtime::Handle::current().block_on(async {
                    rx.await.unwrap_or(ApprovalOutcome {
                        decision: ApprovalDecision::Timeout,
                        reason: None,
                    })
                });

                // Emit ApprovalResolved event
                let decision_str = match outcome.decision {
                    ApprovalDecision::Approved => "approved",
                    ApprovalDecision::Denied => "denied",
                    ApprovalDecision::Timeout => "timeout",
                };
                gate.emit_event(AgentEvent::ApprovalResolved {
                    run_id: context.run_id.clone(),
                    session_id: context.session_id.clone(),
                    approval_id,
                    decision: decision_str.to_string(),
                    reason: outcome.reason.clone(),
                });

                match outcome.decision {
                    ApprovalDecision::Approved => Ok(()),
                    _ => Err(CoreError::Middleware(format!(
                        "tool '{}' approval {}: {}",
                        call.tool_name,
                        decision_str,
                        outcome.reason.unwrap_or_default()
                    ))),
                }
            }
        }
    }

    fn post_tool_call(
        &self,
        _context: &ToolContext,
        _result: &ToolResult,
    ) -> Result<(), CoreError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_policy::rule::{MatchCondition, Rule};

    fn default_annotations() -> HashMap<String, ToolAnnotations> {
        let mut map = HashMap::new();
        map.insert(
            "read_file".to_string(),
            ToolAnnotations {
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: false,
                requires_confirmation: false,
            },
        );
        map.insert(
            "bash".to_string(),
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: false,
                open_world: true,
                requires_confirmation: true,
            },
        );
        map.insert(
            "write_file".to_string(),
            ToolAnnotations {
                read_only: false,
                destructive: true,
                idempotent: false,
                open_world: false,
                requires_confirmation: false,
            },
        );
        map
    }

    fn tool_context() -> ToolContext {
        ToolContext {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
        }
    }

    fn tool_call(name: &str) -> ToolCall {
        ToolCall {
            call_id: "c1".into(),
            tool_name: name.into(),
            input: serde_json::json!({}),
        }
    }

    #[test]
    fn allows_when_no_rules() {
        let engine = PolicyEngine::new();
        let mw = LagoPolicyMiddleware::new(engine, default_annotations());

        let result = mw.pre_tool_call(&tool_context(), &tool_call("read_file"));
        assert!(result.is_ok());
    }

    #[test]
    fn denies_by_tool_name_rule() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "deny-bash".into(),
            name: "Block bash".into(),
            priority: 100,
            condition: MatchCondition::ToolName("bash".into()),
            decision: PolicyDecisionKind::Deny,
            explanation: Some("bash is not allowed".into()),
            required_sandbox: None,
        });
        let mw = LagoPolicyMiddleware::new(engine, default_annotations());

        let result = mw.pre_tool_call(&tool_context(), &tool_call("bash"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("bash is not allowed"), "got: {err}");
    }

    #[test]
    fn allows_non_matching_tool() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "deny-bash".into(),
            name: "Block bash".into(),
            priority: 100,
            condition: MatchCondition::ToolName("bash".into()),
            decision: PolicyDecisionKind::Deny,
            explanation: Some("bash is not allowed".into()),
            required_sandbox: None,
        });
        let mw = LagoPolicyMiddleware::new(engine, default_annotations());

        let result = mw.pre_tool_call(&tool_context(), &tool_call("read_file"));
        assert!(result.is_ok());
    }

    #[test]
    fn no_gate_falls_back_to_deny() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "approve-write".into(),
            name: "Approve writes".into(),
            priority: 100,
            condition: MatchCondition::ToolName("write_file".into()),
            decision: PolicyDecisionKind::RequireApproval,
            explanation: None,
            required_sandbox: None,
        });
        let mw = LagoPolicyMiddleware::new(engine, default_annotations());

        let result = mw.pre_tool_call(&tool_context(), &tool_call("write_file"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("requires approval") && err.contains("no approval gate"),
            "got: {err}"
        );
    }

    #[test]
    fn risk_level_from_annotations() {
        let mw = LagoPolicyMiddleware::new(PolicyEngine::new(), default_annotations());
        assert_eq!(mw.risk_level("read_file"), RiskLevel::Low);
        assert_eq!(mw.risk_level("bash"), RiskLevel::High); // requires_confirmation
        assert_eq!(mw.risk_level("write_file"), RiskLevel::Medium); // destructive
        assert_eq!(mw.risk_level("unknown_tool"), RiskLevel::Low); // default
    }

    #[test]
    fn denies_by_risk_level_rule() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "deny-high-risk".into(),
            name: "Block high risk".into(),
            priority: 50,
            condition: MatchCondition::RiskAtLeast(RiskLevel::High),
            decision: PolicyDecisionKind::Deny,
            explanation: Some("high risk tools are blocked".into()),
            required_sandbox: None,
        });
        let mw = LagoPolicyMiddleware::new(engine, default_annotations());

        // bash has High risk (requires_confirmation)
        let result = mw.pre_tool_call(&tool_context(), &tool_call("bash"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("high risk"), "got: {err}");

        // read_file has Low risk — should be allowed
        let result = mw.pre_tool_call(&tool_context(), &tool_call("read_file"));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn approval_approved_allows_tool() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "approve-write".into(),
            name: "Approve writes".into(),
            priority: 100,
            condition: MatchCondition::ToolName("write_file".into()),
            decision: PolicyDecisionKind::RequireApproval,
            explanation: None,
            required_sandbox: None,
        });
        let gate = Arc::new(ApprovalGate::new(std::time::Duration::from_secs(300)));
        let mw = LagoPolicyMiddleware::with_gate(engine, default_annotations(), gate.clone());

        // Spawn a task that resolves approval after a brief delay
        let gate_clone = gate.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            // Find the pending approval and resolve it
            let ids = gate_clone.pending_ids();
            if let Some(id) = ids.first() {
                gate_clone.resolve(
                    id,
                    ApprovalOutcome {
                        decision: ApprovalDecision::Approved,
                        reason: None,
                    },
                );
            }
        });

        // This blocks until the approval resolves
        let result = tokio::task::spawn_blocking(move || {
            mw.pre_tool_call(&tool_context(), &tool_call("write_file"))
        })
        .await
        .unwrap();
        assert!(result.is_ok(), "approved tool should pass: {:?}", result);
    }

    #[tokio::test]
    async fn approval_denied_blocks_tool() {
        let mut engine = PolicyEngine::new();
        engine.add_rule(Rule {
            id: "approve-write".into(),
            name: "Approve writes".into(),
            priority: 100,
            condition: MatchCondition::ToolName("write_file".into()),
            decision: PolicyDecisionKind::RequireApproval,
            explanation: None,
            required_sandbox: None,
        });
        let gate = Arc::new(ApprovalGate::new(std::time::Duration::from_secs(300)));
        let mw = LagoPolicyMiddleware::with_gate(engine, default_annotations(), gate.clone());

        let gate_clone = gate.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let ids = gate_clone.pending_ids();
            if let Some(id) = ids.first() {
                gate_clone.resolve(
                    id,
                    ApprovalOutcome {
                        decision: ApprovalDecision::Denied,
                        reason: Some("not allowed".into()),
                    },
                );
            }
        });

        let result = tokio::task::spawn_blocking(move || {
            mw.pre_tool_call(&tool_context(), &tool_call("write_file"))
        })
        .await
        .unwrap();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("denied"), "got: {err}");
    }
}
