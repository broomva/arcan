use aios_policy::ApprovalQueue;
use aios_protocol::{
    ApprovalId, ApprovalPort, ApprovalRequest, ApprovalResolution, ApprovalTicket, KernelError,
    SessionId,
};
use async_trait::async_trait;

#[derive(Clone, Default)]
pub struct ArcanApprovalAdapter {
    inner: ApprovalQueue,
}

impl ArcanApprovalAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn queue(&self) -> ApprovalQueue {
        self.inner.clone()
    }
}

#[async_trait]
impl ApprovalPort for ArcanApprovalAdapter {
    async fn enqueue(&self, request: ApprovalRequest) -> Result<ApprovalTicket, KernelError> {
        <ApprovalQueue as ApprovalPort>::enqueue(&self.inner, request).await
    }

    async fn list_pending(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<ApprovalTicket>, KernelError> {
        <ApprovalQueue as ApprovalPort>::list_pending(&self.inner, session_id).await
    }

    async fn resolve(
        &self,
        approval_id: ApprovalId,
        approved: bool,
        actor: String,
    ) -> Result<ApprovalResolution, KernelError> {
        <ApprovalQueue as ApprovalPort>::resolve(&self.inner, approval_id, approved, actor).await
    }
}
