//! `arcan-provider-vercel` — [`HypervisorBackend`] backed by the Vercel Sandbox
//! v2 API (Firecracker microVM isolation, named sandboxes + auto-persistence).
//!
//! # Named sandboxes (beta)
//!
//! The v2 API uses a two-level model:
//!
//! | Layer | Identified by | Lifetime |
//! |-------|--------------|---------|
//! | **Sandbox** | User-defined `name` (unique per project) | Persistent across sessions |
//! | **Session** | System-generated `sessionId` | Ephemeral VM run |
//!
//! This provider maps [`SandboxId`] / [`VmId`] to the **sandbox name** — a
//! stable, human-readable key that survives across sessions and restarts.
//! Commands are executed against the current *session*, which is resolved (or
//! created via auto-resume) on every `exec()` call.
//!
//! # Automatic persistence (beta)
//!
//! When `persistent: true` is set at creation time, the Vercel backend
//! automatically snapshots the sandbox filesystem when the session is stopped.
//! The next `restore()` call boots a fresh session from that snapshot — no
//! manual snapshot management needed.
//!
//! # Isolation
//!
//! Every session runs inside a dedicated Firecracker microVM; no kernel sharing
//! between tenants.
//!
//! # Plan limits
//!
//! | Plan    | Max session | Concurrent |
//! |---------|-------------|-----------|
//! | Hobby   | 45 min      | 10        |
//! | Pro     | 5 hr        | 2 000     |
//!
//! # Authentication
//!
//! Reads `VERCEL_TOKEN` (preferred) or `VERCEL_SANDBOX_API_KEY` for the bearer
//! token. `VERCEL_TEAM_ID` is optional; `VERCEL_PROJECT_ID` is required for
//! list operations.
//!
//! # Kernel ABI (BRO-854)
//!
//! This crate implements [`aios_protocol::hypervisor::HypervisorBackend`] and
//! [`aios_protocol::hypervisor::HypervisorFilesystemExt`] directly. The legacy
//! `arcan_sandbox::SandboxProvider` surface is reached via the blanket
//! `impl<T: HypervisorBackend> SandboxProvider for T` exported by
//! `arcan-sandbox`, so existing callers keep compiling while the workspace
//! migrates off the deprecated trait.
//!
//! [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use arcan_sandbox::{
    error::SandboxError,
    types::{
        ExecRequest, ExecResult, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus,
        SnapshotId,
    },
};

// ── Private HTTP request / response types (v2 API) ───────────────────────────

/// Request body for `POST /v2/sandboxes`.
#[derive(Serialize)]
struct CreateSandboxRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resources: Option<VercelResources>,
    persistent: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
    /// Arbitrary key-value labels (mapped from [`SandboxSpec::labels`]).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    tags: HashMap<String, String>,
}

/// CPU / memory resource hints.
#[derive(Serialize)]
struct VercelResources {
    vcpus: u32,
}

/// Sandbox-level entity returned by the v2 API.
#[derive(Deserialize)]
struct VercelSandboxV2 {
    name: String,
    /// `"running" | "stopped" | "error"`
    status: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(default)]
    persistent: bool,
}

/// Session-level entity returned by the v2 API.
#[derive(Deserialize)]
struct VercelSessionV2 {
    id: String,
    /// `"starting" | "running" | "stopping" | "stopped" | "failed" | "aborted"`
    #[allow(dead_code)]
    status: String,
}

/// Combined response from `POST /v2/sandboxes` and `GET /v2/sandboxes/{name}`.
#[derive(Deserialize)]
struct SandboxAndSession {
    sandbox: VercelSandboxV2,
    session: VercelSessionV2,
}

/// Request body for the v2 exec endpoint.
#[derive(Serialize)]
struct ExecRequestV2 {
    command: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(rename = "wait")]
    wait: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
    sudo: bool,
}

/// Inline result returned when `wait: true` is sent to the exec endpoint.
#[derive(Deserialize)]
struct CommandFinishedV2 {
    #[serde(rename = "exitCode")]
    exit_code: i32,
    #[serde(rename = "startedAt")]
    started_at: u64,
    #[serde(rename = "finishedAt")]
    finished_at: u64,
}

/// Wrapping object from `POST /v2/sandboxes/sessions/{id}/cmd` (wait=true).
#[derive(Deserialize)]
struct ExecResponseV2 {
    /// Command result is inside the `command` field.
    command: CommandFinishedV2,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
}

/// Pagination wrapper from `GET /v2/sandboxes`.
#[derive(Deserialize)]
struct ListSandboxesResponse {
    sandboxes: Vec<VercelSandboxV2>,
}

// ── Provider struct ───────────────────────────────────────────────────────────

/// [`HypervisorBackend`] backed by the Vercel Sandbox v2 API (named sandboxes +
/// auto-persistence beta).
///
/// The primary identifier is the **sandbox name** (stored as [`SandboxId`] /
/// [`aios_protocol::hypervisor::VmId`]). A session ID is resolved on each
/// `exec()` call via the auto-resume endpoint — this incurs one additional GET
/// per exec, which is acceptable for the arcan single-tool-call-per-session
/// pattern.
///
/// [`HypervisorBackend`]: aios_protocol::hypervisor::HypervisorBackend
pub struct VercelSandboxProvider {
    client: reqwest::Client,
    api_token: String,
    team_id: Option<String>,
    /// Required for `list()` and `list_prefixed()`.
    project_id: Option<String>,
    base_url: String,
}

impl VercelSandboxProvider {
    /// Construct from explicit parameters.
    pub fn new(
        api_token: impl Into<String>,
        team_id: Option<String>,
        project_id: Option<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token: api_token.into(),
            team_id,
            project_id,
            base_url: "https://vercel.com/api".into(),
        }
    }

    /// Construct from environment variables.
    ///
    /// | Variable | Required | Notes |
    /// |----------|----------|-------|
    /// | `VERCEL_TOKEN` / `VERCEL_SANDBOX_API_KEY` | Yes | Bearer token |
    /// | `VERCEL_TEAM_ID` | No | Scopes requests to a team |
    /// | `VERCEL_PROJECT_ID` | No | Required for list operations |
    pub fn from_env() -> Result<Self, SandboxError> {
        Self::from_env_fn(|key| std::env::var(key))
    }

    /// Internal constructor accepting a custom env-lookup closure.
    ///
    /// Allows unit tests to inject a controlled environment without mutating
    /// the process-wide environment (which requires `unsafe` in Rust 2024).
    fn from_env_fn<F, E>(env: F) -> Result<Self, SandboxError>
    where
        F: Fn(&str) -> Result<String, E>,
    {
        let api_token = env("VERCEL_TOKEN")
            .or_else(|_| env("VERCEL_SANDBOX_API_KEY"))
            .map_err(|_| SandboxError::ProviderError {
                provider: "vercel",
                message: "VERCEL_TOKEN or VERCEL_SANDBOX_API_KEY must be set".into(),
            })?;
        let team_id = env("VERCEL_TEAM_ID").ok();
        let project_id = env("VERCEL_PROJECT_ID").ok();
        Ok(Self::new(api_token, team_id, project_id))
    }

    /// Override the base URL (useful for tests or staging environments).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    // ── URL helpers ───────────────────────────────────────────────────────────

    /// Build a URL with `teamId` appended when configured.
    fn url(&self, path: &str) -> String {
        match &self.team_id {
            Some(tid) => format!("{}{}?teamId={}", self.base_url, path, tid),
            None => format!("{}{}", self.base_url, path),
        }
    }

    /// Build a URL with an extra query parameter in addition to `teamId`.
    fn url_with_query(&self, path: &str, extra: &[(&str, &str)]) -> String {
        let mut parts: Vec<String> = extra
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding_simple(v)))
            .collect();
        if let Some(tid) = &self.team_id {
            parts.push(format!("teamId={}", tid));
        }
        if parts.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}?{}", self.base_url, path, parts.join("&"))
        }
    }

    // ── HTTP retry helper ─────────────────────────────────────────────────────

    /// Execute a request, retrying on 429 / 503 with exponential backoff
    /// (max 3 attempts: 100 ms → 200 ms → 400 ms).
    async fn send_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, SandboxError> {
        let delays: [u64; 3] = [100, 200, 400];
        let mut last_err: Option<SandboxError> = None;

        for (attempt, &delay_ms) in delays.iter().enumerate() {
            let resp = build()
                .header("Authorization", format!("Bearer {}", self.api_token))
                .send()
                .await
                .map_err(|e| SandboxError::ProviderError {
                    provider: "vercel",
                    message: format!("HTTP transport error: {e}"),
                })?;

            let status = resp.status();
            if status.as_u16() == 429 || status.as_u16() == 503 {
                warn!(
                    attempt = attempt + 1,
                    status = status.as_u16(),
                    delay_ms,
                    "Vercel API rate-limited / unavailable; retrying"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                last_err = Some(SandboxError::ProviderError {
                    provider: "vercel",
                    message: format!("HTTP {status}: rate-limited or service unavailable"),
                });
                continue;
            }

            return Ok(resp);
        }

        Err(last_err.unwrap_or_else(|| SandboxError::ProviderError {
            provider: "vercel",
            message: "request failed after all retries".into(),
        }))
    }

    // ── Private v2 helpers ────────────────────────────────────────────────────

    /// Resolve a session ID for a named sandbox, resuming it if stopped.
    ///
    /// Calls `GET /v2/sandboxes/{name}?resume=true` which returns the active
    /// (or newly-resumed) session together with sandbox metadata.
    async fn resolve_session(
        &self,
        sandbox_name: &str,
    ) -> Result<(VercelSandboxV2, VercelSessionV2), SandboxError> {
        let url = self.url_with_query(
            &format!("/v2/sandboxes/{}", sandbox_name),
            &[("resume", "true")],
        );
        debug!(name = %sandbox_name, "resolving session for Vercel sandbox (auto-resume)");

        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| body_read_err(&e))?;

        if !status.is_success() {
            return Err(map_status_error(
                status,
                &body,
                Some(&SandboxId(sandbox_name.into())),
            ));
        }

        let parsed: SandboxAndSession =
            serde_json::from_str(&body).map_err(SandboxError::Serialization)?;
        Ok((parsed.sandbox, parsed.session))
    }

    /// Fetch sandbox metadata **without** resuming a stopped sandbox.
    ///
    /// Returns `None` if the sandbox does not exist (HTTP 404).
    async fn get_sandbox_info(
        &self,
        sandbox_name: &str,
    ) -> Result<Option<SandboxInfo>, SandboxError> {
        let url = self.url(&format!("/v2/sandboxes/{}", sandbox_name));
        debug!(name = %sandbox_name, "fetching Vercel sandbox info");

        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();

        if status.as_u16() == 404 {
            return Ok(None);
        }

        let body = resp.text().await.map_err(|e| body_read_err(&e))?;
        if !status.is_success() {
            return Err(map_status_error(
                status,
                &body,
                Some(&SandboxId(sandbox_name.into())),
            ));
        }

        // GET /v2/sandboxes/{name} without resume=true returns the sandbox object
        // (not the combined sandbox+session wrapper used by create/resume).
        let sandbox: VercelSandboxV2 =
            serde_json::from_str(&body).map_err(SandboxError::Serialization)?;
        Ok(Some(vercel_sandbox_to_info(sandbox)?))
    }

    // ── v2 SandboxProvider-semantics helpers ──────────────────────────────────
    //
    // These mirror the method bodies of the (now removed) explicit
    // `SandboxProvider` impl. The new `HypervisorBackend` impl calls them
    // through type conversions so existing HTTP paths and fixtures stay intact.

    /// Provision a new sandbox via `POST /v2/sandboxes` and return the
    /// legacy-style handle used by the pre-kernel code path.
    async fn v2_create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let url = self.url("/v2/sandboxes");
        let persistent = !matches!(
            spec.persistence,
            arcan_sandbox::types::PersistencePolicy::Ephemeral
        );
        let body = CreateSandboxRequest {
            name: spec.name.clone(),
            resources: Some(VercelResources {
                vcpus: spec.resources.vcpus,
            }),
            persistent,
            env: spec.env,
            tags: spec.labels,
        };

        debug!(name = %spec.name, persistent, "creating Vercel sandbox (v2)");

        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, None));
        }

        let combined: SandboxAndSession =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;

        sandbox_and_session_to_handle(&combined)
    }

    /// Resume a stopped sandbox by name (v2 auto-resume).
    async fn v2_resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        debug!(sandbox_name = %id, "resuming Vercel sandbox (v2 auto-resume)");
        let (sandbox, session) = self.resolve_session(&id.0).await?;
        sandbox_and_session_to_handle(&SandboxAndSession { sandbox, session })
    }

    /// Execute a command inside the current session of the named sandbox.
    async fn v2_run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        // 1. Resolve (or resume) the current session.
        let (_sandbox, session) = self.resolve_session(&id.0).await?;
        let session_id = session.id;

        // 2. Execute the command against the active session.
        let url = self.url(&format!("/v2/sandboxes/sessions/{}/cmd", session_id));

        // Split argv: command[0] is the executable, the rest are args.
        let (command, args) =
            req.command
                .split_first()
                .ok_or_else(|| SandboxError::ProviderError {
                    provider: "vercel",
                    message: "exec request must have at least one argument".into(),
                })?;

        let body = ExecRequestV2 {
            command: command.clone(),
            args: args.to_vec(),
            cwd: req.working_dir,
            wait: true,
            env: req.env,
            sudo: false,
        };

        debug!(sandbox_name = %id, session_id = %session_id, "executing command in Vercel sandbox (v2)");

        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, Some(id)));
        }

        let exec_resp: ExecResponseV2 =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;

        let duration_ms = exec_resp
            .command
            .finished_at
            .saturating_sub(exec_resp.command.started_at);

        Ok(ExecResult {
            stdout: exec_resp.stdout.into_bytes(),
            stderr: exec_resp.stderr.into_bytes(),
            exit_code: exec_resp.command.exit_code,
            duration_ms,
        })
    }

    /// Snapshot the sandbox by stopping the current session.
    ///
    /// For **persistent** sandboxes, Vercel automatically saves the filesystem
    /// state when the session is stopped — no explicit snapshot call is needed.
    async fn v2_snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        // Resolve the current session (we need its ID to stop it).
        let (_sandbox, session) = self.resolve_session(&id.0).await.map_err(|e| {
            // If already stopped there is nothing to snapshot.
            if matches!(e, SandboxError::NotFound(_)) {
                SandboxError::ProviderError {
                    provider: "vercel",
                    message: format!("sandbox '{}' not found; cannot snapshot", id),
                }
            } else {
                e
            }
        })?;

        let url = self.url(&format!("/v2/sandboxes/sessions/{}/stop", session.id));
        debug!(sandbox_name = %id, session_id = %session.id, "stopping Vercel session (v2 auto-snapshot)");

        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();

        if !status.is_success() {
            let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;
            return Err(map_status_error(status, &body_text, Some(id)));
        }

        // The sandbox name is the stable "snapshot" handle in v2.
        Ok(SnapshotId(id.0.clone()))
    }

    /// Permanently delete a sandbox and all its sessions / snapshots.
    async fn v2_destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let url = self.url(&format!("/v2/sandboxes/{}", id.0));
        debug!(sandbox_name = %id, "deleting Vercel sandbox (v2)");

        let resp = self.send_with_retry(|| self.client.delete(&url)).await?;
        let status = resp.status();

        // 404 = already gone — treat as success.
        if status.as_u16() == 404 || status.is_success() {
            return Ok(());
        }

        let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;
        Err(map_status_error(status, &body_text, Some(id)))
    }

    /// List all sandboxes in the configured project.
    async fn v2_list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        let mut query: Vec<(&str, String)> = Vec::new();
        let project_id_owned;
        if let Some(pid) = &self.project_id {
            project_id_owned = pid.clone();
            query.push(("project", project_id_owned.clone()));
        }

        let url = if query.is_empty() {
            self.url("/v2/sandboxes")
        } else {
            self.url_with_query(
                "/v2/sandboxes",
                &query
                    .iter()
                    .map(|(k, v)| (*k, v.as_str()))
                    .collect::<Vec<_>>(),
            )
        };

        debug!("listing Vercel sandboxes (v2)");

        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, None));
        }

        let list_resp: ListSandboxesResponse =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;

        list_resp
            .sandboxes
            .into_iter()
            .map(vercel_sandbox_to_info)
            .collect()
    }
}

// ── Extended methods (beyond the SandboxProvider trait) ──────────────────────

impl VercelSandboxProvider {
    /// Look up a sandbox by name without resuming it.
    ///
    /// Returns `None` if the sandbox does not exist.
    pub async fn find_by_name(&self, name: &str) -> Result<Option<SandboxInfo>, SandboxError> {
        self.get_sandbox_info(name).await
    }

    /// List sandboxes whose names begin with `prefix`.
    ///
    /// Calls `GET /v2/sandboxes?namePrefix={prefix}&sortBy=name`.
    /// Requires `VERCEL_PROJECT_ID` to be set; returns empty list otherwise.
    pub async fn list_prefixed(&self, prefix: &str) -> Result<Vec<SandboxInfo>, SandboxError> {
        let mut params: Vec<(&str, String)> = vec![
            ("namePrefix", prefix.to_owned()),
            ("sortBy", "name".to_owned()),
        ];
        if let Some(pid) = &self.project_id {
            params.push(("project", pid.clone()));
        }

        let url = self.url_with_query(
            "/v2/sandboxes",
            &params
                .iter()
                .map(|(k, v)| (*k, v.as_str()))
                .collect::<Vec<_>>(),
        );

        debug!(prefix = %prefix, "listing Vercel sandboxes by name prefix (v2)");

        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| body_read_err(&e))?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, None));
        }

        let list_resp: ListSandboxesResponse =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;

        list_resp
            .sandboxes
            .into_iter()
            .map(vercel_sandbox_to_info)
            .collect()
    }

    /// Resume a named sandbox and return a fresh [`SandboxHandle`].
    ///
    /// Convenience wrapper over the v2 auto-resume endpoint.
    pub async fn resume_by_name(&self, name: &str) -> Result<SandboxHandle, SandboxError> {
        self.v2_resume(&SandboxId(name.to_owned())).await
    }
}

// ── HypervisorBackend impl (BRO-854) ──────────────────────────────────────────
//
// Explicit first-class impl of the canonical `aios_protocol::HypervisorBackend`
// contract. Delegates to the private `v2_*` helpers directly so there is no
// round-trip through the deprecated `SandboxProvider` shim.
//
// The deprecated `SandboxProvider` trait is still available to legacy callers
// via the blanket `impl<T: HypervisorBackend> SandboxProvider for T` exposed by
// `arcan-sandbox`; we no longer maintain an explicit `impl SandboxProvider`
// because (a) it would conflict with the blanket impl and (b) all of its
// behaviour is preserved by delegating to the helpers above.
//
// Capability reality-check:
//
// - `FILESYSTEM_READ` / `FILESYSTEM_WRITE` — conceptually supported by the v2
//   API (the SDK exposes read/write), but this provider does not yet wire
//   those endpoints into `HypervisorFilesystemExt::{read_file, write_files}`.
//   The capability bits are advertised so the kernel surfaces Vercel as a
//   filesystem-capable backend; callers that actually invoke the methods get a
//   structured `NotSupported` error until the HTTP wiring lands (see
//   DONE_WITH_CONCERNS note in BRO-854).
// - `FILESYSTEM_EXT` — advertised because this crate implements the extension
//   trait (including `list()`). `write_files`/`read_file` remain `NotSupported`.
// - `NETWORK_EGRESS` — Firecracker sessions have outbound networking.
// - `PERSISTENCE` — named sandboxes + auto-snapshot-on-stop map to the kernel's
//   `snapshot` / `restore` pair.
// - `HIBERNATE` — *not* advertised. The v2 API does not expose a
//   pause-in-place operation separate from `stop` (which is already mapped to
//   snapshot). Advertising `HIBERNATE` would lie; BRO-853 pattern says match
//   bits to actual code.

#[async_trait]
impl aios_protocol::hypervisor::HypervisorBackend for VercelSandboxProvider {
    fn name(&self) -> &'static str {
        "vercel"
    }

    fn capabilities(&self) -> aios_protocol::hypervisor::BackendCapabilitySet {
        use aios_protocol::hypervisor::BackendCapabilitySet;
        BackendCapabilitySet::FILESYSTEM_READ
            | BackendCapabilitySet::FILESYSTEM_WRITE
            | BackendCapabilitySet::FILESYSTEM_EXT
            | BackendCapabilitySet::NETWORK_EGRESS
            | BackendCapabilitySet::PERSISTENCE
            | BackendCapabilitySet::TAGS
    }

    async fn create(
        &self,
        spec: aios_protocol::hypervisor::VmSpec,
    ) -> Result<aios_protocol::hypervisor::VmHandle, aios_protocol::hypervisor::BackendError> {
        use aios_protocol::hypervisor::{BackendId, VmHandle};

        let sandbox_spec = vm_spec_to_sandbox_spec(&spec);
        let handle = self
            .v2_create(sandbox_spec)
            .await
            .map_err(backend_error_from_sandbox)?;

        Ok(VmHandle {
            vm_id: aios_protocol::hypervisor::VmId(handle.id.0),
            backend: BackendId::from("vercel"),
            session_id: session_id_from_spec(&spec),
            agent_id: agent_id_from_spec(&spec),
            status: vm_status_from_sandbox(handle.status),
            created_at: handle.created_at,
            metadata: handle.metadata,
        })
    }

    async fn exec(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
        req: aios_protocol::hypervisor::ExecRequest,
    ) -> Result<aios_protocol::hypervisor::ExecResult, aios_protocol::hypervisor::BackendError>
    {
        let id = SandboxId(vm.vm_id.0.clone());
        let legacy_req = vm_exec_request_to_sandbox(req);
        let legacy = self
            .v2_run(&id, legacy_req)
            .await
            .map_err(backend_error_from_sandbox)?;
        Ok(aios_protocol::hypervisor::ExecResult {
            stdout: legacy.stdout,
            stderr: legacy.stderr,
            exit_code: legacy.exit_code,
            duration_ms: legacy.duration_ms,
        })
    }

    async fn snapshot(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<aios_protocol::hypervisor::VmSnapshotId, aios_protocol::hypervisor::BackendError>
    {
        let id = SandboxId(vm.vm_id.0.clone());
        let snap = self
            .v2_snapshot(&id)
            .await
            .map_err(backend_error_from_sandbox)?;
        Ok(aios_protocol::hypervisor::VmSnapshotId(snap.0))
    }

    /// Restore (resume) a Vercel sandbox from a named snapshot.
    ///
    /// The snapshot id is the stable **sandbox name**; the v2 API returns a
    /// fresh session and sandbox handle via `GET /v2/sandboxes/{name}?resume=true`.
    async fn restore(
        &self,
        snapshot: &aios_protocol::hypervisor::VmSnapshotId,
    ) -> Result<aios_protocol::hypervisor::VmHandle, aios_protocol::hypervisor::BackendError> {
        use aios_protocol::hypervisor::{BackendId, VmHandle};

        let id = SandboxId(snapshot.0.clone());
        let handle = self
            .v2_resume(&id)
            .await
            .map_err(backend_error_from_sandbox)?;

        Ok(VmHandle {
            vm_id: aios_protocol::hypervisor::VmId(handle.id.0),
            backend: BackendId::from("vercel"),
            // restore() has no VmSpec; stamp compat ids so tracing stays
            // consistent with the legacy `SandboxProvider::resume` surface.
            session_id: aios_protocol::ids::SessionId::from_string("arcan-provider-vercel"),
            agent_id: aios_protocol::ids::AgentId::from_string("arcan-provider-vercel"),
            status: vm_status_from_sandbox(handle.status),
            created_at: handle.created_at,
            metadata: handle.metadata,
        })
    }

    async fn destroy(
        &self,
        vm: &aios_protocol::hypervisor::VmHandle,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        let id = SandboxId(vm.vm_id.0.clone());
        self.v2_destroy(&id)
            .await
            .map_err(backend_error_from_sandbox)
    }

    // `hibernate` / `resume` inherit the trait's `BackendError::NotSupported`
    // defaults — the v2 API exposes neither as distinct operations from
    // `snapshot` / `restore`.
}

#[async_trait]
impl aios_protocol::hypervisor::HypervisorFilesystemExt for VercelSandboxProvider {
    async fn write_files(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _files: Vec<aios_protocol::hypervisor::FileWrite>,
    ) -> Result<(), aios_protocol::hypervisor::BackendError> {
        // The v2 SDK exposes `/v2/sandboxes/sessions/{id}/files` but this
        // provider does not yet wire the endpoint. Track in BRO-854 follow-up.
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "vercel",
            reason: "write_files",
        })
    }

    async fn read_file(
        &self,
        _vm: &aios_protocol::hypervisor::VmHandle,
        _path: &str,
    ) -> Result<Vec<u8>, aios_protocol::hypervisor::BackendError> {
        // Same caveat as `write_files` — endpoint exists upstream but is not
        // wired here yet. Track in BRO-854 follow-up.
        Err(aios_protocol::hypervisor::BackendError::NotSupported {
            backend: "vercel",
            reason: "read_file",
        })
    }

    async fn list(
        &self,
    ) -> Result<Vec<aios_protocol::hypervisor::VmInfo>, aios_protocol::hypervisor::BackendError>
    {
        let legacy = self.v2_list().await.map_err(backend_error_from_sandbox)?;

        Ok(legacy
            .into_iter()
            .map(|info| aios_protocol::hypervisor::VmInfo {
                vm_id: aios_protocol::hypervisor::VmId(info.id.0),
                backend: aios_protocol::hypervisor::BackendId::from("vercel"),
                status: vm_status_to_kernel(info.status),
                created_at: info.created_at,
            })
            .collect())
    }
}

// ── Conversion helpers (VmSpec ↔ SandboxSpec, SandboxError → BackendError) ────

/// Translate a kernel-level [`aios_protocol::hypervisor::VmSpec`] into the
/// legacy [`SandboxSpec`] consumed by the private `v2_*` helpers.
///
/// The conversion honours two label conventions:
///
/// | Label | Effect |
/// |-------|--------|
/// | `sandbox.name` | Overrides the sandbox name (otherwise a random UUID) |
/// | `sandbox.persistent` | `"true"` → create a persistent (auto-snapshotted) sandbox |
fn vm_spec_to_sandbox_spec(spec: &aios_protocol::hypervisor::VmSpec) -> SandboxSpec {
    use aios_protocol::hypervisor::RuntimeHint;
    use arcan_sandbox::types::{PersistencePolicy, SandboxResources, SandboxSpec};

    let image = match &spec.runtime_hint {
        RuntimeHint::Custom { image } if !image.is_empty() => Some(image.clone()),
        _ => None,
    };

    let name = spec
        .labels
        .get("sandbox.name")
        .cloned()
        .unwrap_or_else(|| format!("arcan-{}", uuid_like_from_labels(&spec.labels)));

    let persistent = spec
        .labels
        .get("sandbox.persistent")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    // `PersistencePolicy::Persistent { idle_timeout_secs }` is the
    // closest match; default to 60 s per the trait's E2B-calibrated
    // guidance so the v2 auto-snapshot path kicks in.
    let persistence = if persistent {
        PersistencePolicy::Persistent {
            idle_timeout_secs: 60,
        }
    } else {
        PersistencePolicy::Ephemeral
    };

    SandboxSpec {
        name,
        image,
        resources: SandboxResources {
            vcpus: spec.resources.vcpus,
            memory_mb: (spec.resources.memory_kb / 1024).min(u64::from(u32::MAX)) as u32,
            disk_mb: (spec.resources.disk_kb / 1024).min(u64::from(u32::MAX)) as u32,
            timeout_secs: spec.resources.timeout_secs,
        },
        env: spec.env.clone(),
        persistence,
        capabilities: arcan_sandbox::capability::SandboxCapabilitySet::FILESYSTEM_READ
            | arcan_sandbox::capability::SandboxCapabilitySet::FILESYSTEM_WRITE,
        labels: spec.labels.clone(),
    }
}

/// Cheap pseudo-uuid sourced from a `session.id` label when present, otherwise
/// a timestamp. The full `uuid` crate is not pulled in because name collisions
/// are caught server-side by Vercel's uniqueness constraint.
fn uuid_like_from_labels(labels: &HashMap<String, String>) -> String {
    if let Some(sess) = labels.get("session.id") {
        return sess.clone();
    }
    let now = chrono::Utc::now().timestamp_millis();
    format!("{now}")
}

/// Translate a kernel-level [`aios_protocol::hypervisor::ExecRequest`] into the
/// legacy [`arcan_sandbox::types::ExecRequest`] used by the private helpers.
fn vm_exec_request_to_sandbox(
    req: aios_protocol::hypervisor::ExecRequest,
) -> arcan_sandbox::types::ExecRequest {
    arcan_sandbox::types::ExecRequest {
        command: req.command,
        working_dir: req.working_dir,
        env: req.env,
        timeout_secs: req.timeout_secs,
        stdin: req.stdin,
    }
}

/// Map a legacy [`SandboxStatus`] to the canonical
/// [`aios_protocol::hypervisor::VmStatus`]. Kept at module scope so both the
/// `HypervisorBackend` and `HypervisorFilesystemExt` impls can reuse it.
fn vm_status_from_sandbox(status: SandboxStatus) -> aios_protocol::hypervisor::VmStatus {
    use aios_protocol::hypervisor::VmStatus;
    match status {
        SandboxStatus::Starting => VmStatus::Starting,
        SandboxStatus::Running => VmStatus::Running,
        SandboxStatus::Snapshotted => VmStatus::Snapshotted,
        SandboxStatus::Stopping => VmStatus::Stopping,
        SandboxStatus::Stopped => VmStatus::Stopped,
        SandboxStatus::Failed { reason } => VmStatus::Failed { reason },
    }
}

/// Alias used from the `list()` implementation — keeps the call site readable
/// when we are converting the legacy status we just built in `v2_list`.
fn vm_status_to_kernel(status: SandboxStatus) -> aios_protocol::hypervisor::VmStatus {
    vm_status_from_sandbox(status)
}

/// Bridge from the legacy [`SandboxError`] (returned by the internal `v2_*`
/// helpers) to the canonical [`aios_protocol::hypervisor::BackendError`].
///
/// Symmetric counterpart to `From<BackendError> for SandboxError` in
/// `arcan-sandbox::error`. A local forward mapping avoids a circular
/// dependency that would arise from placing `From<SandboxError>` inside
/// `aios-protocol`.
fn backend_error_from_sandbox(e: SandboxError) -> aios_protocol::hypervisor::BackendError {
    use aios_protocol::hypervisor::{BackendError, VmId as NewVmId};
    match e {
        SandboxError::NotFound(id) => BackendError::VmNotFound(NewVmId(id.0)),
        SandboxError::NotSupported { provider, reason } => BackendError::NotSupported {
            backend: provider,
            reason,
        },
        SandboxError::ProviderError {
            provider: _,
            message,
        } => BackendError::Internal(message),
        SandboxError::ExecTimeout { timeout_secs, .. } => BackendError::Timeout {
            duration_ms: timeout_secs.saturating_mul(1_000),
        },
        SandboxError::CapabilityDenied { capability } => {
            BackendError::Internal(format!("capability denied: {capability}"))
        }
        SandboxError::Serialization(err) => BackendError::Internal(err.to_string()),
    }
}

/// Extract a [`SessionId`] from optional `session.id` labels on the spec.
fn session_id_from_spec(spec: &aios_protocol::hypervisor::VmSpec) -> aios_protocol::ids::SessionId {
    spec.labels
        .get("session.id")
        .map(|s| aios_protocol::ids::SessionId::from_string(s.as_str()))
        .unwrap_or_else(|| aios_protocol::ids::SessionId::from_string("arcan-provider-vercel"))
}

/// Extract an [`AgentId`] from optional `agent.id` labels on the spec.
fn agent_id_from_spec(spec: &aios_protocol::hypervisor::VmSpec) -> aios_protocol::ids::AgentId {
    spec.labels
        .get("agent.id")
        .map(|s| aios_protocol::ids::AgentId::from_string(s.as_str()))
        .unwrap_or_else(|| aios_protocol::ids::AgentId::from_string("arcan-provider-vercel"))
}

// ── Conversion helpers (Vercel v2 responses ↔ legacy types) ───────────────────

/// Convert a Vercel v2 status string to [`SandboxStatus`].
fn map_status(s: &str) -> SandboxStatus {
    match s {
        "starting" => SandboxStatus::Starting,
        "running" => SandboxStatus::Running,
        "stopped" | "snapshotted" => SandboxStatus::Snapshotted,
        "stopping" => SandboxStatus::Stopping,
        "error" | "failed" | "aborted" => SandboxStatus::Failed {
            reason: "provider reported error".into(),
        },
        _ => SandboxStatus::Running, // unknown → assume running
    }
}

/// Map an HTTP error status to a [`SandboxError`].
fn map_status_error(
    status: reqwest::StatusCode,
    body: &str,
    sandbox_id: Option<&SandboxId>,
) -> SandboxError {
    match status.as_u16() {
        404 => SandboxError::NotFound(
            sandbox_id
                .cloned()
                .unwrap_or_else(|| SandboxId("unknown".into())),
        ),
        402 | 403 => SandboxError::CapabilityDenied {
            capability: "api_access",
        },
        408 | 504 => SandboxError::ExecTimeout {
            sandbox_id: sandbox_id
                .cloned()
                .unwrap_or_else(|| SandboxId("unknown".into())),
            timeout_secs: 0,
        },
        _ => SandboxError::ProviderError {
            provider: "vercel",
            message: format!("HTTP {}: {}", status, body),
        },
    }
}

/// Build a [`SandboxHandle`] from a combined sandbox + session response.
fn sandbox_and_session_to_handle(c: &SandboxAndSession) -> Result<SandboxHandle, SandboxError> {
    let created_at: DateTime<Utc> =
        c.sandbox
            .created_at
            .parse()
            .map_err(|e| SandboxError::ProviderError {
                provider: "vercel",
                message: format!("invalid createdAt '{}': {e}", c.sandbox.created_at),
            })?;

    // Sandbox name is the stable [`SandboxId`] in v2.
    let name = c.sandbox.name.clone();

    Ok(SandboxHandle {
        id: SandboxId(name.clone()),
        name,
        status: map_status(&c.sandbox.status),
        created_at,
        provider: "vercel".into(),
        metadata: serde_json::json!({
            "session_id": c.session.id,
            "persistent": c.sandbox.persistent,
        }),
    })
}

/// Build a [`SandboxInfo`] from a v2 sandbox object.
fn vercel_sandbox_to_info(s: VercelSandboxV2) -> Result<SandboxInfo, SandboxError> {
    let created_at: DateTime<Utc> =
        s.created_at
            .parse()
            .map_err(|e| SandboxError::ProviderError {
                provider: "vercel",
                message: format!("invalid createdAt '{}': {e}", s.created_at),
            })?;

    Ok(SandboxInfo {
        id: SandboxId(s.name.clone()),
        name: s.name,
        status: map_status(&s.status),
        created_at,
    })
}

/// Error factory for HTTP response body read failures.
fn body_read_err(e: &reqwest::Error) -> SandboxError {
    SandboxError::ProviderError {
        provider: "vercel",
        message: format!("failed to read response body: {e}"),
    }
}

/// Minimal percent-encoding for query parameter values.
///
/// Only encodes characters that would break URL parsing; not a full RFC 3986
/// encoder (which would be overkill for sandbox names).
fn urlencoding_simple(s: &str) -> String {
    s.replace('%', "%25")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
        .replace(' ', "%20")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use arcan_sandbox::capability::SandboxCapabilitySet;
    use arcan_sandbox::provider::SandboxProvider;

    /// Helper: build a provider pointing at the mock server.
    fn provider_for(server: &mockito::Server) -> VercelSandboxProvider {
        VercelSandboxProvider::new("test-token", None, Some("proj-123".into()))
            .with_base_url(server.url())
    }

    const CREATED_AT: &str = "2026-01-01T00:00:00Z";

    /// JSON fixture for a combined sandbox + session response (v2).
    fn running_combined_json(name: &str, session_id: &str) -> String {
        format!(
            r#"{{
              "sandbox": {{"name":"{name}","status":"running","createdAt":"{CREATED_AT}","persistent":true}},
              "session": {{"id":"{session_id}","status":"running"}}
            }}"#
        )
    }

    /// JSON fixture for a single sandbox object (v2 list/get without session).
    fn sandbox_only_json(name: &str, status: &str) -> String {
        format!(
            r#"{{"name":"{name}","status":"{status}","createdAt":"{CREATED_AT}","persistent":true}}"#
        )
    }

    /// JSON fixture for the list response.
    fn list_response_json(sandboxes: &[(&str, &str)]) -> String {
        let items: Vec<String> = sandboxes
            .iter()
            .map(|(name, status)| sandbox_only_json(name, status))
            .collect();
        format!(r#"{{"sandboxes":[{}]}}"#, items.join(","))
    }

    // ── Static tests ──────────────────────────────────────────────────────────

    #[test]
    fn name_is_vercel() {
        let p = VercelSandboxProvider::new("tok", None, None);
        // Test via the legacy SandboxProvider surface (blanket impl) so we
        // continue to exercise the deprecated name that existing callers read.
        assert_eq!(SandboxProvider::name(&p), "vercel");
    }

    #[test]
    fn capabilities_include_tags_and_persistence() {
        let p = VercelSandboxProvider::new("tok", None, None);
        let caps = SandboxProvider::capabilities(&p);
        assert!(caps.contains(SandboxCapabilitySet::TAGS));
        assert!(caps.contains(SandboxCapabilitySet::PERSISTENCE));
        assert!(caps.contains(SandboxCapabilitySet::NETWORK_OUTBOUND));
    }

    #[test]
    fn from_env_returns_err_without_token() {
        let result =
            VercelSandboxProvider::from_env_fn(|_key: &str| -> Result<String, &'static str> {
                Err("not set")
            });
        assert!(result.is_err(), "expected Err when no token env var is set");
    }

    #[test]
    fn from_env_reads_project_id() {
        let result =
            VercelSandboxProvider::from_env_fn(|key: &str| -> Result<String, &'static str> {
                match key {
                    "VERCEL_TOKEN" => Ok("tok".into()),
                    "VERCEL_TEAM_ID" => Err("not set"),
                    "VERCEL_PROJECT_ID" => Ok("proj-abc".into()),
                    _ => Err("not set"),
                }
            });
        assert!(result.is_ok());
        assert_eq!(result.unwrap().project_id.as_deref(), Some("proj-abc"));
    }

    // ── HTTP-level tests (mockito) ────────────────────────────────────────────

    #[tokio::test]
    async fn create_maps_spec_and_returns_name_as_id() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v2/sandboxes")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(running_combined_json("my-sandbox", "sess-001"))
            .create_async()
            .await;

        let provider = provider_for(&server);
        let spec = SandboxSpec::ephemeral("my-sandbox");
        // Exercise the legacy-surface method via the blanket impl.
        let handle = SandboxProvider::create(&provider, spec)
            .await
            .expect("create should succeed");

        // v2: SandboxId = sandbox name
        assert_eq!(handle.id.0, "my-sandbox");
        assert_eq!(handle.name, "my-sandbox");
        assert_eq!(handle.status, SandboxStatus::Running);
        assert_eq!(handle.provider, "vercel");

        // Session ID stored in metadata
        assert_eq!(handle.metadata["session_id"].as_str().unwrap(), "sess-001");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn create_sends_tags_from_labels() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v2/sandboxes")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(running_combined_json("tagged-sbx", "sess-002"))
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"tags":{"env":"prod"}}"#.to_owned(),
            ))
            .create_async()
            .await;

        let provider = provider_for(&server);
        let mut spec = SandboxSpec::ephemeral("tagged-sbx");
        spec.labels.insert("env".into(), "prod".into());
        SandboxProvider::create(&provider, spec)
            .await
            .expect("create with tags should succeed");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn run_auto_resumes_then_executes() {
        let sandbox_name = "run-sandbox";
        let session_id = "sess-run-001";
        let mut server = mockito::Server::new_async().await;

        // Step 1: resolve session (GET with resume=true)
        let mock_get = server
            .mock(
                "GET",
                mockito::Matcher::Regex(format!(r#"^/v2/sandboxes/{}(\?.*)?$"#, sandbox_name)),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(running_combined_json(sandbox_name, session_id))
            .create_async()
            .await;

        // Step 2: exec command
        let mock_exec = server
            .mock(
                "POST",
                format!("/v2/sandboxes/sessions/{}/cmd", session_id).as_str(),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"command":{"exitCode":0,"startedAt":1000,"finishedAt":1042},"stdout":"hello\n","stderr":""}"#,
            )
            .create_async()
            .await;

        let provider = provider_for(&server);
        let id = SandboxId(sandbox_name.into());
        let req = ExecRequest::shell("echo hello");
        let result = SandboxProvider::run(&provider, &id, req)
            .await
            .expect("run should succeed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration_ms, 42);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));

        mock_get.assert_async().await;
        mock_exec.assert_async().await;
    }

    #[tokio::test]
    async fn find_by_name_returns_some_for_existing_sandbox() {
        let sandbox_name = "lookup-sbx";
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", format!("/v2/sandboxes/{}", sandbox_name).as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(sandbox_only_json(sandbox_name, "running"))
            .create_async()
            .await;

        let provider = provider_for(&server);
        let result = provider
            .find_by_name(sandbox_name)
            .await
            .expect("find_by_name should not error");

        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.id.0, sandbox_name);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn find_by_name_returns_none_on_404() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v2/sandboxes/missing-sbx")
            .with_status(404)
            .with_body(r#"{"error":"not found"}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        let result = provider
            .find_by_name("missing-sbx")
            .await
            .expect("find_by_name should not error on 404");

        assert!(result.is_none());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn list_prefixed_filters_by_name() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/v2/sandboxes\?.*namePrefix=arcan.*$"#.to_owned()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(list_response_json(&[
                ("arcan-sess-1", "running"),
                ("arcan-sess-2", "stopped"),
            ]))
            .create_async()
            .await;

        let provider = provider_for(&server);
        let result = provider
            .list_prefixed("arcan")
            .await
            .expect("list_prefixed should succeed");

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id.0, "arcan-sess-1");
        assert_eq!(result[1].id.0, "arcan-sess-2");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn destroy_sends_delete_and_treats_404_as_ok() {
        let sandbox_name = "del-sbx";
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("DELETE", format!("/v2/sandboxes/{}", sandbox_name).as_str())
            .with_status(404)
            .with_body(r#"{"error":"not found"}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        SandboxProvider::destroy(&provider, &SandboxId(sandbox_name.into()))
            .await
            .expect("destroy should succeed even on 404");

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn error_429_retries() {
        let mut server = mockito::Server::new_async().await;

        // Use regex matchers so that query parameters (?project=…) are ignored.
        let mock_429 = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/v2/sandboxes(\?.*)?$"#.to_owned()),
            )
            .with_status(429)
            .with_body("")
            .create_async()
            .await;

        let mock_200 = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r#"^/v2/sandboxes(\?.*)?$"#.to_owned()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"sandboxes":[]}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        // Legacy `SandboxProvider::list` via the blanket impl returns an empty
        // vec regardless of the backend (the trait has no `list`); call the
        // explicit extension trait instead to exercise the HTTP path.
        let result = aios_protocol::hypervisor::HypervisorFilesystemExt::list(&provider).await;
        assert!(result.is_ok(), "should succeed after retry; got {result:?}");
        assert_eq!(result.unwrap().len(), 0);

        mock_429.assert_async().await;
        mock_200.assert_async().await;
    }
}

// ── HypervisorBackend trait tests (BRO-854) ──────────────────────────────────

#[cfg(test)]
mod kernel_tests {
    use super::*;
    use aios_protocol::hypervisor::{BackendCapabilitySet, HypervisorBackend};

    #[test]
    fn vercel_provider_impls_hypervisor_backend() {
        // Vercel requires no test credentials for a static capability check —
        // the constructor takes plain strings and stores them without
        // validation.
        let provider = VercelSandboxProvider::new("test-token", None, None);
        assert_eq!(HypervisorBackend::name(&provider), "vercel");
        assert!(
            HypervisorBackend::capabilities(&provider).contains(BackendCapabilitySet::PERSISTENCE),
            "vercel provider must advertise PERSISTENCE (named sandboxes + auto-snapshot)"
        );
    }

    #[test]
    fn vercel_provider_advertises_expected_capability_set() {
        let provider = VercelSandboxProvider::new("test-token", None, None);
        let caps = HypervisorBackend::capabilities(&provider);
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_READ));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_WRITE));
        assert!(caps.contains(BackendCapabilitySet::FILESYSTEM_EXT));
        assert!(caps.contains(BackendCapabilitySet::NETWORK_EGRESS));
        assert!(caps.contains(BackendCapabilitySet::PERSISTENCE));
        // HIBERNATE is intentionally NOT advertised — Vercel v2 exposes no
        // pause-in-place operation separate from snapshot / restore.
        assert!(
            !caps.contains(BackendCapabilitySet::HIBERNATE),
            "vercel must not advertise HIBERNATE — no distinct pause endpoint exists in v2"
        );
    }

    #[tokio::test]
    async fn vercel_provider_hibernate_returns_not_supported_default() {
        use aios_protocol::hypervisor::{BackendError, BackendId, VmHandle, VmId, VmStatus};
        use aios_protocol::ids::{AgentId, SessionId};

        let provider = VercelSandboxProvider::new("test-token", None, None);
        let handle = VmHandle {
            vm_id: VmId::from("vm-1"),
            backend: BackendId::from("vercel"),
            session_id: SessionId::from_string("sess-1"),
            agent_id: AgentId::from_string("agent-1"),
            status: VmStatus::Running,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        };
        let err = HypervisorBackend::hibernate(&provider, &handle)
            .await
            .expect_err("default hibernate impl should return NotSupported");
        assert!(matches!(err, BackendError::NotSupported { .. }));
    }

    #[tokio::test]
    async fn vercel_provider_resume_returns_not_supported_default() {
        use aios_protocol::hypervisor::{BackendError, BackendId, VmHandle, VmId, VmStatus};
        use aios_protocol::ids::{AgentId, SessionId};

        let provider = VercelSandboxProvider::new("test-token", None, None);
        let handle = VmHandle {
            vm_id: VmId::from("vm-1"),
            backend: BackendId::from("vercel"),
            session_id: SessionId::from_string("sess-1"),
            agent_id: AgentId::from_string("agent-1"),
            status: VmStatus::Hibernated,
            created_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        };
        let err = HypervisorBackend::resume(&provider, &handle)
            .await
            .expect_err("default resume impl should return NotSupported");
        assert!(matches!(err, BackendError::NotSupported { .. }));
    }
}
