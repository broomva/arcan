//! Advisory integration with the Autonomic homeostasis controller.
//!
//! [`AutonomicPolicyAdapter`] decorates an inner [`PolicyGatePort`] by consulting
//! Autonomic's `/gating/{session_id}` HTTP endpoint before delegating. If Autonomic
//! is unreachable or returns an error, the adapter fails open and delegates entirely
//! to the inner gate (advisory semantics).
//!
//! Response types are duplicated locally to avoid coupling `arcan-aios-adapters`
//! to `autonomic-core` at the crate level.

use std::sync::Arc;

use aios_protocol::{
    Capability, GatingProfile, KernelError, PolicyGateDecision, PolicyGatePort, PolicySet,
    SessionId,
};
use async_trait::async_trait;
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Public handle for economic gates (shared with provider layer in Phase 2)
// ---------------------------------------------------------------------------

/// Shared handle exposing the latest economic gates from Autonomic.
///
/// The provider layer can read this to adjust model selection and token budgets.
pub type EconomicGateHandle = Arc<tokio::sync::RwLock<Option<EconomicGates>>>;

// ---------------------------------------------------------------------------
// Local response types (mirrors autonomic-core, avoids crate coupling)
// ---------------------------------------------------------------------------

/// HTTP response from `GET /gating/{session_id}`.
#[derive(Debug, Deserialize)]
struct GatingResponse {
    #[allow(dead_code)]
    session_id: String,
    profile: LocalGatingProfile,
    #[allow(dead_code)]
    last_event_seq: u64,
    #[allow(dead_code)]
    last_event_ms: u64,
}

/// Mirrors `autonomic_core::gating::AutonomicGatingProfile`.
#[derive(Debug, Clone, Deserialize)]
struct LocalGatingProfile {
    /// Canonical operational gates (from aios-protocol).
    operational: GatingProfile,
    /// Economic regulation gates.
    economic: EconomicGates,
    #[serde(default)]
    #[allow(dead_code)]
    rationale: Vec<String>,
}

/// Economic regulation gates from Autonomic.
///
/// Published so the provider layer can read model tier and token caps.
#[derive(Debug, Clone, Deserialize)]
pub struct EconomicGates {
    /// Current economic operating mode.
    pub economic_mode: EconomicMode,
    /// Maximum tokens allowed for the next turn (advisory).
    pub max_tokens_next_turn: Option<u32>,
    /// Preferred model tier for cost control.
    pub preferred_model: Option<ModelTier>,
    /// Whether expensive tools (e.g., web search, code execution) are allowed.
    pub allow_expensive_tools: bool,
    /// Whether agent replication is allowed.
    pub allow_replication: bool,
}

/// Economic operating mode, determined by balance-to-burn ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicMode {
    Sovereign,
    Conserving,
    Hustle,
    Hibernate,
}

/// LLM model tier for cost-aware selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Flagship,
    Standard,
    Budget,
}

// ---------------------------------------------------------------------------
// AutonomicPolicyAdapter
// ---------------------------------------------------------------------------

/// Decorator that optionally consults Autonomic before delegating to an inner
/// [`PolicyGatePort`].
///
/// Advisory semantics: if Autonomic is unreachable, the adapter falls through
/// to the inner gate. When Autonomic responds, the most-restrictive union of
/// Autonomic's operational gates and the inner decision wins.
pub struct AutonomicPolicyAdapter {
    inner: Arc<dyn PolicyGatePort>,
    client: reqwest::Client,
    base_url: String,
    economic_handle: EconomicGateHandle,
}

impl AutonomicPolicyAdapter {
    /// Create a new adapter wrapping `inner`, consulting `base_url` for gating.
    pub fn new(inner: Arc<dyn PolicyGatePort>, base_url: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("failed to build reqwest client");

        Self {
            inner,
            client,
            base_url,
            economic_handle: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Returns a cloneable handle to the latest economic gates.
    pub fn economic_handle(&self) -> EconomicGateHandle {
        self.economic_handle.clone()
    }
}

#[async_trait]
impl PolicyGatePort for AutonomicPolicyAdapter {
    async fn evaluate(
        &self,
        session_id: SessionId,
        requested: Vec<Capability>,
    ) -> Result<PolicyGateDecision, KernelError> {
        // 1. Get the inner (base) decision first.
        let inner_decision = self.inner.evaluate(session_id.clone(), requested).await?;

        // 2. Consult Autonomic (advisory — errors fall through).
        let Some(gating) = self.fetch_gating(&session_id).await else {
            return Ok(inner_decision);
        };

        // 3. Store economic gates for provider layer.
        {
            let mut handle = self.economic_handle.write().await;
            *handle = Some(gating.economic);
        }

        // 4. Merge operational gates: most restrictive wins.
        Ok(merge_decision(inner_decision, &gating.operational))
    }

    async fn set_policy(
        &self,
        session_id: SessionId,
        policy: PolicySet,
    ) -> Result<(), KernelError> {
        // Autonomic doesn't own policy writes — delegate directly.
        self.inner.set_policy(session_id, policy).await
    }
}

impl AutonomicPolicyAdapter {
    /// Fetch gating profile from Autonomic. Returns `None` on any failure.
    async fn fetch_gating(&self, session_id: &SessionId) -> Option<LocalGatingProfile> {
        let url = format!("{}/gating/{}", self.base_url, session_id);

        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, %url, "Autonomic unreachable, falling through");
                return None;
            }
        };

        if !resp.status().is_success() {
            tracing::warn!(
                status = %resp.status(),
                %url,
                "Autonomic returned non-success, falling through"
            );
            return None;
        }

        match resp.json::<GatingResponse>().await {
            Ok(gr) => Some(gr.profile),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse Autonomic gating response");
                None
            }
        }
    }
}

/// Merge an inner [`PolicyGateDecision`] with Autonomic's operational gates.
///
/// The most restrictive outcome wins: if Autonomic disallows side effects,
/// all capabilities the inner gate allowed are moved to denied.
fn merge_decision(
    mut decision: PolicyGateDecision,
    operational: &GatingProfile,
) -> PolicyGateDecision {
    if !operational.allow_side_effects {
        // Nuclear: deny everything the inner allowed.
        decision.denied.append(&mut decision.allowed);
        decision.denied.append(&mut decision.requires_approval);
        return decision;
    }

    // Selectively deny based on specific gates.
    let mut newly_denied = Vec::new();
    decision.allowed.retain(|cap| {
        let s = cap.as_str();
        if !operational.allow_shell && s.starts_with("exec:cmd:") {
            newly_denied.push(cap.clone());
            return false;
        }
        if !operational.allow_network && s.starts_with("net:egress:") {
            newly_denied.push(cap.clone());
            return false;
        }
        true
    });
    decision.denied.extend(newly_denied);

    decision
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Minimal inner gate that always allows everything.
    struct AlwaysAllowGate;

    #[async_trait]
    impl PolicyGatePort for AlwaysAllowGate {
        async fn evaluate(
            &self,
            _session_id: SessionId,
            requested: Vec<Capability>,
        ) -> Result<PolicyGateDecision, KernelError> {
            Ok(PolicyGateDecision {
                allowed: requested,
                requires_approval: vec![],
                denied: vec![],
            })
        }
    }

    /// Inner gate that tracks set_policy calls.
    struct TrackingGate {
        called: Arc<tokio::sync::Mutex<bool>>,
    }

    #[async_trait]
    impl PolicyGatePort for TrackingGate {
        async fn evaluate(
            &self,
            _session_id: SessionId,
            requested: Vec<Capability>,
        ) -> Result<PolicyGateDecision, KernelError> {
            Ok(PolicyGateDecision {
                allowed: requested,
                requires_approval: vec![],
                denied: vec![],
            })
        }

        async fn set_policy(
            &self,
            _session_id: SessionId,
            _policy: PolicySet,
        ) -> Result<(), KernelError> {
            *self.called.lock().await = true;
            Ok(())
        }
    }

    fn permissive_gating_json() -> serde_json::Value {
        serde_json::json!({
            "session_id": "test-session",
            "profile": {
                "operational": {
                    "allow_side_effects": true,
                    "require_approval_for_risk": "high",
                    "max_tool_calls_per_tick": 10,
                    "max_file_mutations_per_tick": 5,
                    "allow_network": true,
                    "allow_shell": true
                },
                "economic": {
                    "economic_mode": "sovereign",
                    "max_tokens_next_turn": null,
                    "preferred_model": null,
                    "allow_expensive_tools": true,
                    "allow_replication": true
                },
                "rationale": []
            },
            "last_event_seq": 0,
            "last_event_ms": 0
        })
    }

    fn restrictive_gating_json() -> serde_json::Value {
        serde_json::json!({
            "session_id": "test-session",
            "profile": {
                "operational": {
                    "allow_side_effects": false,
                    "require_approval_for_risk": "low",
                    "max_tool_calls_per_tick": 2,
                    "max_file_mutations_per_tick": 0,
                    "allow_network": false,
                    "allow_shell": false
                },
                "economic": {
                    "economic_mode": "hibernate",
                    "max_tokens_next_turn": 100,
                    "preferred_model": "budget",
                    "allow_expensive_tools": false,
                    "allow_replication": false
                },
                "rationale": ["balance depleted"]
            },
            "last_event_seq": 5,
            "last_event_ms": 1700000000000u64
        })
    }

    fn conserving_gating_json() -> serde_json::Value {
        serde_json::json!({
            "session_id": "test-session",
            "profile": {
                "operational": {
                    "allow_side_effects": true,
                    "require_approval_for_risk": "medium",
                    "max_tool_calls_per_tick": 5,
                    "max_file_mutations_per_tick": 2,
                    "allow_network": true,
                    "allow_shell": false
                },
                "economic": {
                    "economic_mode": "conserving",
                    "max_tokens_next_turn": 2000,
                    "preferred_model": "standard",
                    "allow_expensive_tools": true,
                    "allow_replication": false
                },
                "rationale": ["approaching burn limit"]
            },
            "last_event_seq": 3,
            "last_event_ms": 1700000000000u64
        })
    }

    #[tokio::test]
    async fn passthrough_when_autonomic_allows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gating/test-session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(permissive_gating_json()))
            .mount(&server)
            .await;

        let inner: Arc<dyn PolicyGatePort> = Arc::new(AlwaysAllowGate);
        let adapter = AutonomicPolicyAdapter::new(inner, server.uri());

        let caps = vec![
            Capability::fs_write("/workspace/foo.rs"),
            Capability::exec("cargo"),
        ];
        let decision = adapter
            .evaluate(SessionId::from_string("test-session"), caps.clone())
            .await
            .unwrap();

        assert_eq!(decision.allowed.len(), 2);
        assert!(decision.denied.is_empty());
        assert!(decision.requires_approval.is_empty());
    }

    #[tokio::test]
    async fn deny_overrides_inner_allow() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gating/test-session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(restrictive_gating_json()))
            .mount(&server)
            .await;

        let inner: Arc<dyn PolicyGatePort> = Arc::new(AlwaysAllowGate);
        let adapter = AutonomicPolicyAdapter::new(inner, server.uri());

        let caps = vec![
            Capability::fs_write("/workspace/foo.rs"),
            Capability::exec("cargo"),
        ];
        let decision = adapter
            .evaluate(SessionId::from_string("test-session"), caps)
            .await
            .unwrap();

        // allow_side_effects: false → everything denied
        assert!(decision.allowed.is_empty());
        assert_eq!(decision.denied.len(), 2);
    }

    #[tokio::test]
    async fn unreachable_falls_through() {
        // No mock server → connection refused
        let inner: Arc<dyn PolicyGatePort> = Arc::new(AlwaysAllowGate);
        let adapter = AutonomicPolicyAdapter::new(inner, "http://127.0.0.1:1".to_string());

        let caps = vec![Capability::exec("cargo")];
        let decision = adapter
            .evaluate(SessionId::from_string("test-session"), caps)
            .await
            .unwrap();

        // Falls through to inner → allowed
        assert_eq!(decision.allowed.len(), 1);
        assert!(decision.denied.is_empty());
    }

    #[tokio::test]
    async fn timeout_falls_through() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gating/test-session"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(restrictive_gating_json())
                    .set_delay(std::time::Duration::from_secs(5)),
            )
            .mount(&server)
            .await;

        let inner: Arc<dyn PolicyGatePort> = Arc::new(AlwaysAllowGate);
        let adapter = AutonomicPolicyAdapter::new(inner, server.uri());

        let caps = vec![Capability::exec("cargo")];
        let decision = adapter
            .evaluate(SessionId::from_string("test-session"), caps)
            .await
            .unwrap();

        // Timeout → falls through to inner → allowed
        assert_eq!(decision.allowed.len(), 1);
        assert!(decision.denied.is_empty());
    }

    #[tokio::test]
    async fn economic_gates_stored_in_handle() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gating/test-session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(conserving_gating_json()))
            .mount(&server)
            .await;

        let inner: Arc<dyn PolicyGatePort> = Arc::new(AlwaysAllowGate);
        let adapter = AutonomicPolicyAdapter::new(inner, server.uri());
        let handle = adapter.economic_handle();

        // Before evaluate: handle is empty
        assert!(handle.read().await.is_none());

        let caps = vec![Capability::fs_read("/workspace/foo.rs")];
        let _decision = adapter
            .evaluate(SessionId::from_string("test-session"), caps)
            .await
            .unwrap();

        // After evaluate: handle contains economic gates
        let gates = handle.read().await;
        let gates = gates.as_ref().expect("economic gates should be stored");
        assert_eq!(gates.economic_mode, EconomicMode::Conserving);
        assert_eq!(gates.max_tokens_next_turn, Some(2000));
        assert_eq!(gates.preferred_model, Some(ModelTier::Standard));
        assert!(!gates.allow_replication);
    }

    #[tokio::test]
    async fn set_policy_delegates_to_inner() {
        let called = Arc::new(tokio::sync::Mutex::new(false));
        let inner: Arc<dyn PolicyGatePort> = Arc::new(TrackingGate {
            called: called.clone(),
        });
        let adapter = AutonomicPolicyAdapter::new(inner, "http://127.0.0.1:1".to_string());

        adapter
            .set_policy(SessionId::from_string("test-session"), PolicySet::default())
            .await
            .unwrap();

        assert!(*called.lock().await, "set_policy should delegate to inner");
    }
}
