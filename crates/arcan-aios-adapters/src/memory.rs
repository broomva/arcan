use std::path::PathBuf;

use aios_memory::WorkspaceMemoryStore;
use aios_protocol::{KernelError, MemoryPort, MemoryQuery, Observation, SessionId, SoulProfile};
use async_trait::async_trait;

#[derive(Clone)]
pub struct ArcanMemoryAdapter {
    inner: WorkspaceMemoryStore,
}

impl ArcanMemoryAdapter {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            inner: WorkspaceMemoryStore::new(root),
        }
    }
}

#[async_trait]
impl MemoryPort for ArcanMemoryAdapter {
    async fn load_soul(&self, session_id: SessionId) -> Result<SoulProfile, KernelError> {
        <WorkspaceMemoryStore as MemoryPort>::load_soul(&self.inner, session_id).await
    }

    async fn save_soul(&self, session_id: SessionId, soul: SoulProfile) -> Result<(), KernelError> {
        <WorkspaceMemoryStore as MemoryPort>::save_soul(&self.inner, session_id, soul).await
    }

    async fn append_observation(
        &self,
        session_id: SessionId,
        observation: Observation,
    ) -> Result<(), KernelError> {
        <WorkspaceMemoryStore as MemoryPort>::append_observation(
            &self.inner,
            session_id,
            observation,
        )
        .await
    }

    async fn query_observations(
        &self,
        session_id: SessionId,
        query: MemoryQuery,
    ) -> Result<Vec<Observation>, KernelError> {
        <WorkspaceMemoryStore as MemoryPort>::query_observations(&self.inner, session_id, query)
            .await
    }
}
