//! Adapter-trait implementations for the four `ergon-life-hooks`
//! adapter traits.
//!
//! Each adapter trait declared in `ergon-life-hooks` (`CapabilityResolver`,
//! `BudgetGate`, `ResponseScorer`, `SoulAttester`) gets at least one
//! implementation here. The minimum-viable BRO-1001 ships:
//!
//! 1. **[`KernelCapabilityResolver`]** — real implementation backed by
//!    [`aios_protocol::PolicyGatePort`]. This is the only adapter that
//!    must be functional in v0.1; without it a workflow's tools would
//!    bypass the kernel's capability gating.
//! 2. **[`NoopBudgetGate`]** — accepts every inference call. The real
//!    implementation lives in a follow-up wired against
//!    `autonomic::AutonomicGatingProfile`; for now we ship a permissive
//!    stand-in so the workflow loop can run.
//! 3. **[`NoopResponseScorer`]** — returns an empty JSON object. The
//!    real implementation will route through `nous_core::NousEvaluator`.
//! 4. **[`NoopSoulAttester`]** — returns `Ok(())`. The real
//!    implementation will use `anima_core::AgentSoul` to sign
//!    `SessionStart` / `SessionEnd` events.
//!
//! ## Why noop is acceptable for the non-capability adapters
//!
//! The `NousScoreHook` and `AnimaAttestHook` are documented as
//! observe-only / best-effort: substrate failures (and absences) MUST
//! NOT abort the workflow. So shipping permissive implementations now
//! keeps the workflow tick body functional while the real adapters
//! land in follow-up tickets without changing the public surface.

use crate::error::AdapterError;
use aios_protocol::{Capability, PolicyGatePort, SessionId as KernelSessionId};
use async_trait::async_trait;
use ergon::{ModelRequest, ModelResponse, SessionId as ErgonSessionId};
use ergon_life_hooks::{BudgetGate, CapabilityResolver, ResponseScorer, SoulAttester};
use std::sync::Arc;

/// Maps tool names to required [`aios_protocol::Capability`] tokens.
///
/// Production wiring would ask `praxis_core::ToolRegistry` for the
/// declared capabilities of each tool. For BRO-1001's minimum-viable
/// adapter we accept a hand-supplied map; arcand can pre-compute it
/// from the registered praxis tools and pass it in.
pub type ToolCapabilityMap = std::collections::HashMap<String, Vec<Capability>>;

/// Capability resolver backed by [`PolicyGatePort`].
///
/// Resolution flow per `can_invoke(tool_name, _input)`:
/// 1. Look up the required capabilities for `tool_name` in the
///    [`ToolCapabilityMap`]. If unknown → deny (fail-closed).
/// 2. Call [`PolicyGatePort::evaluate`] with those capabilities.
/// 3. Allow only if [`aios_protocol::PolicyGateDecision::is_allowed_now`]
///    returns true (no `denied`, no `requires_approval`).
///
/// `requires_approval` denials surface as deny here — the BRO-1001
/// minimum doesn't bridge ergon's hooks into the kernel's approval
/// flow. A follow-up will route `requires_approval` through the
/// `ApprovalPort`.
pub struct KernelCapabilityResolver {
    gate: Arc<dyn PolicyGatePort>,
    session_id: KernelSessionId,
    tool_capabilities: ToolCapabilityMap,
}

impl KernelCapabilityResolver {
    /// Construct a resolver bound to one session's policy.
    pub fn new(
        gate: Arc<dyn PolicyGatePort>,
        session_id: KernelSessionId,
        tool_capabilities: ToolCapabilityMap,
    ) -> Self {
        Self {
            gate,
            session_id,
            tool_capabilities,
        }
    }
}

#[async_trait]
impl CapabilityResolver for KernelCapabilityResolver {
    async fn can_invoke(
        &self,
        tool_name: &str,
        _input: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        let required = self
            .tool_capabilities
            .get(tool_name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "tool `{tool_name}` has no declared capabilities; \
                     denying fail-closed (register capabilities in the \
                     adapter's ToolCapabilityMap)"
                )
            })?;

        if required.is_empty() {
            // Tool is explicitly capability-free.
            return Ok(());
        }

        let decision = self
            .gate
            .evaluate(self.session_id.clone(), required.clone())
            .await
            .map_err(|err| AdapterError::port("PolicyGatePort", err).to_string())?;

        if decision.is_allowed_now() {
            Ok(())
        } else if !decision.denied.is_empty() {
            Err(format!(
                "denied capabilities: {:?}",
                decision.denied.into_iter().map(|c| c.0).collect::<Vec<_>>()
            ))
        } else {
            Err(format!(
                "requires approval: {:?} (approval flow not wired in BRO-1001)",
                decision
                    .requires_approval
                    .into_iter()
                    .map(|c| c.0)
                    .collect::<Vec<_>>()
            ))
        }
    }
}

/// Permissive [`BudgetGate`] — accepts every inference call.
///
/// Real implementation lives in a follow-up against
/// `autonomic::AutonomicGatingProfile`. This stand-in keeps the
/// workflow loop functional in BRO-1001's minimum slice.
pub struct NoopBudgetGate;

#[async_trait]
impl BudgetGate for NoopBudgetGate {
    async fn allow_inference(&self, _req: &mut ModelRequest) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// Permissive [`ResponseScorer`] — returns an empty score object.
///
/// Real implementation lives in a follow-up against
/// `nous_core::NousEvaluator`.
pub struct NoopResponseScorer;

#[async_trait]
impl ResponseScorer for NoopResponseScorer {
    async fn score(
        &self,
        _response: &ModelResponse,
    ) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({}))
    }
}

/// Permissive [`SoulAttester`] — returns `Ok(())` for both boundaries.
///
/// Real implementation lives in a follow-up against
/// `anima_core::AgentSoul`.
pub struct NoopSoulAttester;

#[async_trait]
impl SoulAttester for NoopSoulAttester {
    async fn sign_session_start(
        &self,
        _session_id: &ErgonSessionId,
        _workflow_name: &str,
    ) -> std::result::Result<(), String> {
        Ok(())
    }

    async fn sign_session_end(
        &self,
        _session_id: &ErgonSessionId,
        _workflow_name: &str,
        _ok: bool,
    ) -> std::result::Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::{KernelResult, PolicyGateDecision, PolicySet};

    struct AlwaysAllow;

    #[async_trait]
    impl PolicyGatePort for AlwaysAllow {
        async fn evaluate(
            &self,
            _session_id: KernelSessionId,
            requested: Vec<Capability>,
        ) -> KernelResult<PolicyGateDecision> {
            Ok(PolicyGateDecision {
                allowed: requested,
                requires_approval: Vec::new(),
                denied: Vec::new(),
            })
        }

        async fn set_policy(
            &self,
            _session_id: KernelSessionId,
            _policy: PolicySet,
        ) -> KernelResult<()> {
            Ok(())
        }
    }

    struct AlwaysRequiresApproval;

    #[async_trait]
    impl PolicyGatePort for AlwaysRequiresApproval {
        async fn evaluate(
            &self,
            _session_id: KernelSessionId,
            requested: Vec<Capability>,
        ) -> KernelResult<PolicyGateDecision> {
            Ok(PolicyGateDecision {
                allowed: Vec::new(),
                requires_approval: requested,
                denied: Vec::new(),
            })
        }

        async fn set_policy(
            &self,
            _session_id: KernelSessionId,
            _policy: PolicySet,
        ) -> KernelResult<()> {
            Ok(())
        }
    }

    struct AlwaysDeny;

    #[async_trait]
    impl PolicyGatePort for AlwaysDeny {
        async fn evaluate(
            &self,
            _session_id: KernelSessionId,
            requested: Vec<Capability>,
        ) -> KernelResult<PolicyGateDecision> {
            Ok(PolicyGateDecision {
                allowed: Vec::new(),
                requires_approval: Vec::new(),
                denied: requested,
            })
        }

        async fn set_policy(
            &self,
            _session_id: KernelSessionId,
            _policy: PolicySet,
        ) -> KernelResult<()> {
            Ok(())
        }
    }

    fn map(name: &str, caps: Vec<Capability>) -> ToolCapabilityMap {
        let mut m = ToolCapabilityMap::new();
        m.insert(name.to_owned(), caps);
        m
    }

    #[tokio::test]
    async fn capability_free_tool_is_allowed() {
        let r = KernelCapabilityResolver::new(
            Arc::new(AlwaysDeny),
            KernelSessionId::default(),
            map("free", Vec::new()),
        );
        assert!(r.can_invoke("free", &serde_json::Value::Null).await.is_ok());
    }

    #[tokio::test]
    async fn unknown_tool_fails_closed() {
        let r = KernelCapabilityResolver::new(
            Arc::new(AlwaysAllow),
            KernelSessionId::default(),
            ToolCapabilityMap::new(),
        );
        let err = r
            .can_invoke("ghost", &serde_json::Value::Null)
            .await
            .expect_err("unknown tool denied");
        assert!(err.contains("ghost"));
    }

    #[tokio::test]
    async fn allowed_tool_passes_gate() {
        let r = KernelCapabilityResolver::new(
            Arc::new(AlwaysAllow),
            KernelSessionId::default(),
            map("read", vec![Capability::fs_read("/**")]),
        );
        assert!(r.can_invoke("read", &serde_json::Value::Null).await.is_ok());
    }

    #[tokio::test]
    async fn approval_required_tool_fails_until_flow_lands() {
        // BRO-1001 minimum doesn't bridge ergon's hooks into the
        // kernel's approval flow, so `requires_approval` is treated
        // as fail-closed at the hook boundary. A follow-up will route
        // this through the ApprovalPort.
        let r = KernelCapabilityResolver::new(
            Arc::new(AlwaysRequiresApproval),
            KernelSessionId::default(),
            map("dangerous", vec![Capability::exec("rm")]),
        );
        let err = r
            .can_invoke("dangerous", &serde_json::Value::Null)
            .await
            .expect_err("requires_approval must fail-close in BRO-1001");
        assert!(
            err.contains("requires approval"),
            "error should mention approval requirement, got: {err}"
        );
    }

    #[tokio::test]
    async fn denied_tool_fails() {
        let r = KernelCapabilityResolver::new(
            Arc::new(AlwaysDeny),
            KernelSessionId::default(),
            map("write", vec![Capability::fs_write("/**")]),
        );
        let err = r
            .can_invoke("write", &serde_json::Value::Null)
            .await
            .expect_err("denied tool fails");
        assert!(err.contains("denied"));
    }

    #[tokio::test]
    async fn noop_budget_gate_allows() {
        let mut req = ModelRequest::new("m", Vec::new());
        assert!(NoopBudgetGate.allow_inference(&mut req).await.is_ok());
    }

    #[tokio::test]
    async fn noop_response_scorer_returns_empty_object() {
        let resp = ModelResponse::new(Vec::new(), ergon::StopReason::EndTurn);
        let score = NoopResponseScorer.score(&resp).await.expect("score ok");
        assert!(score.is_object());
        assert!(score.as_object().unwrap().is_empty());
    }

    #[tokio::test]
    async fn noop_soul_attester_signs_both_boundaries() {
        let s = ErgonSessionId::default();
        assert!(NoopSoulAttester.sign_session_start(&s, "wf").await.is_ok());
        assert!(
            NoopSoulAttester
                .sign_session_end(&s, "wf", true)
                .await
                .is_ok()
        );
    }
}
