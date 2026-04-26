//! `arcan-provider-cube` — [`HypervisorBackend`] implementation backed by
//! the CubeSandbox HTTP API v1.
//!
//! See `README.md` for backend identity, env vars, and capability set.
//!
//! # Capabilities
//!
//! Cube advertises filesystem (read + write + ext), network egress,
//! persistence (`snapshot` / `restore`), forking, and tag (label)
//! preservation. Hibernation is **not** supported by the upstream Cube
//! API v1 — calls to [`HypervisorBackend::hibernate`] / `resume` fall
//! through to the trait defaults, which return
//! [`BackendError::NotSupported`].
//!
//! # Construction
//!
//! Three constructors are provided:
//!
//! - [`CubeProvider::new`] — explicit base URL + bearer token.
//! - [`CubeProvider::from_env`] — reads `CUBE_API_URL` + `CUBE_API_TOKEN`.
//! - [`CubeProvider::with_session`] — builder applied after construction
//!   to bind every VM the provider creates to a session + agent. The
//!   kernel calls this during backend registration.
//!
//! # Tracing
//!
//! Every public async method is annotated with [`macro@tracing::instrument`]
//! carrying `backend = "cube"` plus the relevant identifier (`vm_id` /
//! `snapshot_id`) so a single grep over Vigil traces yields the full
//! lifecycle of a Cube-backed VM.
//!
//! [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend
//! [`BackendError::NotSupported`]: aios_protocol::hypervisor::BackendError::NotSupported

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod convert;
mod error;
mod types;

use aios_protocol::hypervisor::{
    BackendCapabilitySet, BackendError, ExecRequest, ExecResult, HypervisorBackend, VmHandle,
    VmSnapshotId, VmSpec,
};
use aios_protocol::ids::{AgentId, SessionId};
use async_trait::async_trait;
use reqwest::Method;
use tracing::{debug, instrument};

pub use error::CubeError;

use client::CubeClient;
use convert::{
    create_vm_req_from_spec, exec_req_from, exec_result_from_resp, vm_handle_from_resp,
};
use types::{SnapshotResp, VmResp};

/// Stable backend name used by the kernel for routing + observability.
const BACKEND_NAME: &str = "cube";

/// [`HypervisorBackend`] backed by CubeSandbox v1.
///
/// Construct with [`CubeProvider::new`] (explicit) or
/// [`CubeProvider::from_env`] (reads `CUBE_API_URL` + `CUBE_API_TOKEN`).
/// Every VM created by this provider is attributed to a session + agent;
/// callers thread those through with [`CubeProvider::with_session`].
#[derive(Debug, Clone)]
pub struct CubeProvider {
    client: CubeClient,
    /// Session bound to every VM created by this provider — populated
    /// via [`CubeProvider::with_session`]. Defaults to `"unbound"` so
    /// the provider is constructable in CLI / smoke-test contexts that
    /// do not yet have a session.
    session_id: SessionId,
    /// Agent attribution for every VM created by this provider — same
    /// `"unbound"` default semantics as [`Self::session_id`].
    agent_id: AgentId,
}

impl CubeProvider {
    /// Construct from an explicit base URL + bearer token.
    ///
    /// The caller owns the configuration; no I/O happens until the first
    /// trait method is invoked.
    pub fn new(base_url: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            client: CubeClient::new(base_url, bearer_token),
            session_id: SessionId::from_string("unbound"),
            agent_id: AgentId::from_string("unbound"),
        }
    }

    /// Construct from environment variables.
    ///
    /// | Variable | Required |
    /// |---|---|
    /// | `CUBE_API_URL` | yes |
    /// | `CUBE_API_TOKEN` | yes |
    ///
    /// Returns [`CubeError::Unsupported`] when either variable is missing —
    /// the kernel converts this into a `NotSupported` `BackendError` at
    /// the registration boundary, which surfaces in startup logs as
    /// "cube backend skipped: CUBE_API_URL must be set".
    pub fn from_env() -> Result<Self, CubeError> {
        let url = std::env::var("CUBE_API_URL")
            .map_err(|_| CubeError::Unsupported("CUBE_API_URL must be set"))?;
        let token = std::env::var("CUBE_API_TOKEN")
            .map_err(|_| CubeError::Unsupported("CUBE_API_TOKEN must be set"))?;
        Ok(Self::new(url, token))
    }

    /// Bind every VM this provider creates to a session + agent.
    ///
    /// `KernelEngine` calls this during backend registration so the
    /// resulting [`VmHandle`] carries provenance for downstream auditing
    /// + per-session billing.
    #[must_use]
    pub fn with_session(mut self, session: SessionId, agent: AgentId) -> Self {
        self.session_id = session;
        self.agent_id = agent;
        self
    }
}

#[async_trait]
impl HypervisorBackend for CubeProvider {
    fn name(&self) -> &'static str {
        BACKEND_NAME
    }

    fn capabilities(&self) -> BackendCapabilitySet {
        BackendCapabilitySet::FILESYSTEM_READ
            | BackendCapabilitySet::FILESYSTEM_WRITE
            | BackendCapabilitySet::FILESYSTEM_EXT
            | BackendCapabilitySet::NETWORK_EGRESS
            | BackendCapabilitySet::PERSISTENCE
            | BackendCapabilitySet::FORK
            | BackendCapabilitySet::TAGS
    }

    #[instrument(
        skip(self, spec),
        fields(
            backend = BACKEND_NAME,
            session_id = %self.session_id,
            agent_id = %self.agent_id,
        ),
    )]
    async fn create(&self, spec: VmSpec) -> Result<VmHandle, BackendError> {
        let body = create_vm_req_from_spec(&spec);
        debug!(
            vcpus = body.vcpus,
            memory_mb = body.memory_mb,
            disk_mb = body.disk_mb,
            "cube.create",
        );
        let resp: VmResp = self
            .client
            .request(Method::POST, "/api/v1/vms", &body)
            .await
            .map_err(|e| e.into_backend_error(None, None))?;
        Ok(vm_handle_from_resp(
            resp,
            BACKEND_NAME,
            &self.session_id,
            &self.agent_id,
        ))
    }

    #[instrument(
        skip(self, vm, req),
        fields(
            backend = BACKEND_NAME,
            vm_id = %vm.vm_id,
            session_id = %self.session_id,
        ),
    )]
    async fn exec(&self, vm: &VmHandle, req: ExecRequest) -> Result<ExecResult, BackendError> {
        let body = exec_req_from(&req);
        let path = format!("/api/v1/vms/{}/exec", vm.vm_id);
        let resp = self
            .client
            .request(Method::POST, &path, &body)
            .await
            .map_err(|e| e.into_backend_error(Some(vm.vm_id.clone()), None))?;
        exec_result_from_resp(resp).map_err(|e| e.into_backend_error(Some(vm.vm_id.clone()), None))
    }

    #[instrument(
        skip(self, vm),
        fields(
            backend = BACKEND_NAME,
            vm_id = %vm.vm_id,
            session_id = %self.session_id,
        ),
    )]
    async fn snapshot(&self, vm: &VmHandle) -> Result<VmSnapshotId, BackendError> {
        let path = format!("/api/v1/vms/{}/snapshot", vm.vm_id);
        let resp: SnapshotResp = self
            .client
            .request(Method::POST, &path, &serde_json::json!({}))
            .await
            .map_err(|e| e.into_backend_error(Some(vm.vm_id.clone()), None))?;
        Ok(VmSnapshotId::from(resp.id))
    }

    #[instrument(
        skip(self, snapshot),
        fields(
            backend = BACKEND_NAME,
            snapshot_id = %snapshot,
            session_id = %self.session_id,
        ),
    )]
    async fn restore(&self, snapshot: &VmSnapshotId) -> Result<VmHandle, BackendError> {
        let path = format!("/api/v1/snapshots/{snapshot}/restore");
        let resp: VmResp = self
            .client
            .request(Method::POST, &path, &serde_json::json!({}))
            .await
            .map_err(|e| e.into_backend_error(None, Some(snapshot.clone())))?;
        Ok(vm_handle_from_resp(
            resp,
            BACKEND_NAME,
            &self.session_id,
            &self.agent_id,
        ))
    }

    #[instrument(
        skip(self, vm),
        fields(
            backend = BACKEND_NAME,
            vm_id = %vm.vm_id,
            session_id = %self.session_id,
        ),
    )]
    async fn destroy(&self, vm: &VmHandle) -> Result<(), BackendError> {
        let path = format!("/api/v1/vms/{}", vm.vm_id);
        match self
            .client
            .request_no_body::<serde_json::Value>(Method::DELETE, &path)
            .await
        {
            Ok(_) => Ok(()),
            // Destroy MUST be idempotent — the trait contract says it
            // succeeds even when the VM is already gone.
            Err(CubeError::NotFound(_)) => Ok(()),
            Err(e) => Err(e.into_backend_error(Some(vm.vm_id.clone()), None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the capability bit-set down so silent drift in either
    /// direction (gaining or losing a bit) requires updating this test.
    /// The set is tied to the wire contract — capability bits drive
    /// kernel routing, so a regression here would silently break
    /// `BackendSelector::Auto` matches across the whole workspace.
    #[test]
    fn capabilities_are_the_documented_set() {
        let provider = CubeProvider::new("http://127.0.0.1:0", "test-token");
        let caps = provider.capabilities();
        let expected = BackendCapabilitySet::FILESYSTEM_READ
            | BackendCapabilitySet::FILESYSTEM_WRITE
            | BackendCapabilitySet::FILESYSTEM_EXT
            | BackendCapabilitySet::NETWORK_EGRESS
            | BackendCapabilitySet::PERSISTENCE
            | BackendCapabilitySet::FORK
            | BackendCapabilitySet::TAGS;
        assert_eq!(
            caps, expected,
            "Cube capability set drifted; update lib.rs + README + ticket together",
        );
        // And spell out the negative space — Cube must NOT advertise
        // these even by accident, because the trait routes to default
        // `NotSupported` impls for them.
        assert!(!caps.contains(BackendCapabilitySet::HIBERNATE));
        assert!(!caps.contains(BackendCapabilitySet::NETWORK_INGRESS));
        assert!(!caps.contains(BackendCapabilitySet::CUSTOM_IMAGE));
        assert!(!caps.contains(BackendCapabilitySet::GPU));
    }

    #[test]
    fn name_is_stable_string() {
        let provider = CubeProvider::new("http://127.0.0.1:0", "test-token");
        assert_eq!(provider.name(), "cube");
        assert_eq!(provider.name(), BACKEND_NAME);
    }

    #[test]
    fn with_session_threads_attribution_through_provider() {
        let provider = CubeProvider::new("http://127.0.0.1:0", "test-token").with_session(
            SessionId::from_string("sess-42"),
            AgentId::from_string("agent-7"),
        );
        assert_eq!(provider.session_id.as_str(), "sess-42");
        assert_eq!(provider.agent_id.as_str(), "agent-7");
    }

    /// Compile-time assertion that `CubeProvider` implements
    /// [`HypervisorBackend`] — if the trait signature changes upstream,
    /// this stops compiling first.
    #[allow(dead_code)]
    fn _assert_impls_hypervisor_backend() {
        fn _accept<T: HypervisorBackend>() {}
        _accept::<CubeProvider>();
    }

    /// Compile-time assertion that `CubeProvider` is dyn-compatible.
    #[allow(dead_code)]
    fn _assert_dyn_safe(_: &dyn HypervisorBackend) {}
}
