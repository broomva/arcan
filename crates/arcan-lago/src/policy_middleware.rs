use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolResult};
use arcan_core::runtime::{Middleware, ToolContext};
use lago_core::event::PolicyDecisionKind;
use lago_core::event::RiskLevel;
use lago_core::PolicyContext;
use lago_policy::engine::PolicyEngine;
use std::collections::HashMap;

/// Arcan [`Middleware`] backed by lago's [`PolicyEngine`].
///
/// On every tool call, builds a [`PolicyContext`], evaluates the policy rules,
/// and returns `Err(CoreError::Middleware)` when the decision is `Deny`.
/// `RequireApproval` is currently treated as `Deny` (no interactive approval yet).
pub struct LagoPolicyMiddleware {
    engine: PolicyEngine,
    /// Cached annotations per tool name, used to derive risk levels.
    tool_annotations: HashMap<String, ToolAnnotations>,
}

impl LagoPolicyMiddleware {
    pub fn new(engine: PolicyEngine, tool_annotations: HashMap<String, ToolAnnotations>) -> Self {
        Self {
            engine,
            tool_annotations,
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
                let rule = decision.rule_id.unwrap_or_else(|| "unknown".to_string());
                Err(CoreError::Middleware(format!(
                    "tool '{}' requires approval (rule: {})",
                    call.tool_name, rule
                )))
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
    fn require_approval_treated_as_error() {
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
        assert!(err.contains("requires approval"), "got: {err}");
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
}
