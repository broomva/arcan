use std::sync::Arc;

use aios_policy::SessionPolicyEngine;
use aios_protocol::{
    Capability, KernelError, PolicyGateDecision, PolicyGatePort, PolicySet, SessionId,
};
use async_trait::async_trait;

#[derive(Clone)]
pub struct ArcanPolicyAdapter {
    inner: Arc<SessionPolicyEngine>,
}

impl ArcanPolicyAdapter {
    pub fn new(default_policy: PolicySet) -> Self {
        Self {
            inner: Arc::new(SessionPolicyEngine::new(default_policy)),
        }
    }

    pub fn engine(&self) -> Arc<SessionPolicyEngine> {
        self.inner.clone()
    }
}

#[async_trait]
impl PolicyGatePort for ArcanPolicyAdapter {
    async fn evaluate(
        &self,
        session_id: SessionId,
        requested: Vec<Capability>,
    ) -> Result<PolicyGateDecision, KernelError> {
        <SessionPolicyEngine as PolicyGatePort>::evaluate(&*self.inner, session_id, requested).await
    }

    async fn set_policy(
        &self,
        session_id: SessionId,
        policy: PolicySet,
    ) -> Result<(), KernelError> {
        <SessionPolicyEngine as PolicyGatePort>::set_policy(&*self.inner, session_id, policy).await
    }
}
