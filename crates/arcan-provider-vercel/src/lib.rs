//! `arcan-provider-vercel` — [`SandboxProvider`] implementation backed by the
//! [Vercel Sandbox API](https://vercel.com/docs/sandbox).
//!
//! # Isolation
//!
//! Every sandbox runs inside a dedicated Firecracker microVM; no kernel sharing
//! between tenants.
//!
//! # Limits (as of beta)
//!
//! | Plan  | Max session | Concurrent |
//! |-------|-------------|-----------|
//! | Hobby | 45 min      | 10        |
//! | Pro   | 5 hr        | 2 000     |
//!
//! # Snapshot semantics
//!
//! Vercel conflates *pause* and *snapshot* into a single `stop` operation:
//! calling [`VercelSandboxProvider::snapshot`] issues
//! `POST /v1/sandboxes/{id}/stop`.  Billing halts and state is preserved.
//! [`VercelSandboxProvider::resume`] issues
//! `POST /v1/sandboxes/{id}/sessions` to restore from the implicit snapshot.
//! The [`SnapshotId`] returned by `snapshot()` is the sandbox ID itself.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use arcan_sandbox::{
    capability::SandboxCapabilitySet,
    error::SandboxError,
    provider::SandboxProvider,
    types::{
        ExecRequest, ExecResult, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec, SandboxStatus,
        SnapshotId,
    },
};

// ── Private HTTP request / response types ────────────────────────────────────

/// Request body for `POST /v1/sandboxes`.
#[derive(Serialize)]
struct CreateSandboxRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resources: Option<VercelResources>,
    #[serde(default)]
    persistent: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
}

/// CPU / memory resource hints sent to the Vercel API.
#[derive(Serialize)]
struct VercelResources {
    /// Number of virtual CPUs.
    cpu: u32,
    /// RAM in megabytes.
    memory: u32,
}

/// Response from `POST /v1/sandboxes` and `GET /v1/sandboxes`.
#[derive(Deserialize)]
struct VercelSandbox {
    id: String,
    name: String,
    /// `"starting" | "running" | "stopped" | "error"`
    status: String,
    #[serde(rename = "createdAt")]
    created_at: String,
}

/// Request body for `POST /v1/sandboxes/{id}/exec`.
#[derive(Serialize)]
struct ExecSandboxRequest {
    command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
}

/// Response from `POST /v1/sandboxes/{id}/exec`.
#[derive(Deserialize)]
struct ExecSandboxResponse {
    stdout: String,
    stderr: String,
    #[serde(rename = "exitCode")]
    exit_code: i32,
    #[serde(rename = "durationMs")]
    duration_ms: u64,
}

/// Wrapper around the list endpoint response.
#[derive(Deserialize)]
struct ListSandboxesResponse {
    sandboxes: Vec<VercelSandbox>,
}

// ── Provider struct ───────────────────────────────────────────────────────────

/// [`SandboxProvider`] implementation backed by the Vercel Sandbox API.
///
/// Isolation: Firecracker microVM (dedicated kernel per sandbox).
///
/// Hobby limits: 45 min max session, 10 concurrent.
/// Pro limits:   5 hr max session, 2,000 concurrent.
pub struct VercelSandboxProvider {
    client: reqwest::Client,
    api_token: String,
    team_id: Option<String>,
    base_url: String,
}

impl VercelSandboxProvider {
    /// Construct from explicit parameters.
    ///
    /// `api_token` is the Vercel bearer token; `team_id` is optional and
    /// appended as `?teamId=…` to every request when present.
    pub fn new(api_token: impl Into<String>, team_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_token: api_token.into(),
            team_id,
            base_url: "https://api.vercel.com".into(),
        }
    }

    /// Construct from environment variables.
    ///
    /// Reads `VERCEL_TOKEN` (preferred) or `VERCEL_SANDBOX_API_KEY` for the
    /// bearer token.  Reads `VERCEL_TEAM_ID` for the optional team scope.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxError::ProviderError`] if neither token variable is set.
    pub fn from_env() -> Result<Self, SandboxError> {
        Self::from_env_fn(|key| std::env::var(key))
    }

    /// Internal constructor that accepts a custom env-lookup function.
    ///
    /// This indirection allows unit tests to inject a controlled environment
    /// without mutating the process-wide environment (which requires `unsafe`
    /// in Rust edition 2024).
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
        Ok(Self::new(api_token, team_id))
    }

    /// Override the base URL (useful for tests or staging environments).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Append `?teamId=…` when a team ID is configured.
    fn url(&self, path: &str) -> String {
        match &self.team_id {
            Some(tid) => format!("{}{}?teamId={}", self.base_url, path, tid),
            None => format!("{}{}", self.base_url, path),
        }
    }

    /// Execute an HTTP request, retrying on 429 and 503 with exponential backoff
    /// (max 3 attempts: 100 ms / 200 ms / 400 ms).
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
}

// ── Status / error helpers ────────────────────────────────────────────────────

/// Convert a Vercel status string to [`SandboxStatus`].
fn map_status(s: &str) -> SandboxStatus {
    match s {
        "starting" => SandboxStatus::Starting,
        "running" => SandboxStatus::Running,
        "stopped" | "snapshotted" => SandboxStatus::Snapshotted,
        "stopping" => SandboxStatus::Stopping,
        "error" => SandboxStatus::Failed {
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

/// Parse a Vercel sandbox into a [`SandboxHandle`].
fn vercel_sandbox_to_handle(s: VercelSandbox) -> Result<SandboxHandle, SandboxError> {
    let created_at: DateTime<Utc> =
        s.created_at
            .parse()
            .map_err(|e| SandboxError::ProviderError {
                provider: "vercel",
                message: format!("invalid createdAt timestamp '{}': {e}", s.created_at),
            })?;
    Ok(SandboxHandle {
        id: SandboxId(s.id),
        name: s.name,
        status: map_status(&s.status),
        created_at,
        provider: "vercel".into(),
        metadata: serde_json::Value::Null,
    })
}

/// Parse a Vercel sandbox into a [`SandboxInfo`].
fn vercel_sandbox_to_info(s: VercelSandbox) -> Result<SandboxInfo, SandboxError> {
    let created_at: DateTime<Utc> =
        s.created_at
            .parse()
            .map_err(|e| SandboxError::ProviderError {
                provider: "vercel",
                message: format!("invalid createdAt timestamp '{}': {e}", s.created_at),
            })?;
    Ok(SandboxInfo {
        id: SandboxId(s.id),
        name: s.name,
        status: map_status(&s.status),
        created_at,
    })
}

// ── SandboxProvider impl ──────────────────────────────────────────────────────

#[async_trait]
impl SandboxProvider for VercelSandboxProvider {
    fn name(&self) -> &'static str {
        "vercel"
    }

    /// Returns the capability set supported by the Vercel sandbox backend.
    ///
    /// Note: `CUSTOM_IMAGE` is not included; the Vercel beta does not expose
    /// image selection to callers.
    fn capabilities(&self) -> SandboxCapabilitySet {
        SandboxCapabilitySet::FILESYSTEM_READ
            | SandboxCapabilitySet::FILESYSTEM_WRITE
            | SandboxCapabilitySet::NETWORK_OUTBOUND
            | SandboxCapabilitySet::PERSISTENCE
    }

    /// Provision a new sandbox via `POST /v1/sandboxes`.
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let url = self.url("/v1/sandboxes");
        let persistent = !matches!(
            spec.persistence,
            arcan_sandbox::types::PersistencePolicy::Ephemeral
        );
        let body = CreateSandboxRequest {
            name: spec.name.clone(),
            resources: Some(VercelResources {
                cpu: spec.resources.vcpus,
                memory: spec.resources.memory_mb,
            }),
            persistent,
            env: spec.env,
        };

        debug!(name = %spec.name, "creating Vercel sandbox");

        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
            provider: "vercel",
            message: format!("failed to read response body: {e}"),
        })?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, None));
        }

        let sandbox: VercelSandbox =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;
        vercel_sandbox_to_handle(sandbox)
    }

    /// Resume a snapshotted sandbox via `POST /v1/sandboxes/{id}/sessions`.
    async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        let url = self.url(&format!("/v1/sandboxes/{}/sessions", id.0));

        debug!(sandbox_id = %id, "resuming Vercel sandbox");

        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
            provider: "vercel",
            message: format!("failed to read response body: {e}"),
        })?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, Some(id)));
        }

        let sandbox: VercelSandbox =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;
        vercel_sandbox_to_handle(sandbox)
    }

    /// Execute a command inside a running sandbox via `POST /v1/sandboxes/{id}/exec`.
    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        let url = self.url(&format!("/v1/sandboxes/{}/exec", id.0));
        let body = ExecSandboxRequest {
            command: req.command,
            cwd: req.working_dir,
            timeout: req.timeout_secs,
            env: req.env,
        };

        debug!(sandbox_id = %id, "executing command in Vercel sandbox");

        let resp = self
            .send_with_retry(|| self.client.post(&url).json(&body))
            .await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
            provider: "vercel",
            message: format!("failed to read response body: {e}"),
        })?;

        if !status.is_success() {
            return Err(map_status_error(status, &body_text, Some(id)));
        }

        let exec_resp: ExecSandboxResponse =
            serde_json::from_str(&body_text).map_err(SandboxError::Serialization)?;

        Ok(ExecResult {
            stdout: exec_resp.stdout.into_bytes(),
            stderr: exec_resp.stderr.into_bytes(),
            exit_code: exec_resp.exit_code,
            duration_ms: exec_resp.duration_ms,
        })
    }

    /// Snapshot the sandbox by stopping it via `POST /v1/sandboxes/{id}/stop`.
    ///
    /// Vercel auto-snapshots on stop; billing halts and state is preserved.
    /// The returned [`SnapshotId`] is the sandbox ID itself (Vercel keeps a
    /// single implicit snapshot per sandbox).
    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let url = self.url(&format!("/v1/sandboxes/{}/stop", id.0));

        debug!(sandbox_id = %id, "snapshotting Vercel sandbox (stop)");

        let resp = self.send_with_retry(|| self.client.post(&url)).await?;
        let status = resp.status();

        if !status.is_success() {
            let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
                provider: "vercel",
                message: format!("failed to read response body: {e}"),
            })?;
            return Err(map_status_error(status, &body_text, Some(id)));
        }

        // Vercel uses the sandbox ID as the implicit snapshot handle.
        Ok(SnapshotId(id.0.clone()))
    }

    /// Permanently destroy a sandbox via `DELETE /v1/sandboxes/{id}`.
    ///
    /// Succeeds even if the sandbox is already stopped or not found.
    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        let url = self.url(&format!("/v1/sandboxes/{}", id.0));

        debug!(sandbox_id = %id, "destroying Vercel sandbox");

        let resp = self.send_with_retry(|| self.client.delete(&url)).await?;
        let status = resp.status();

        // 404 is acceptable — already gone.
        if status.as_u16() == 404 || status.is_success() {
            return Ok(());
        }

        let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
            provider: "vercel",
            message: format!("failed to read response body: {e}"),
        })?;
        Err(map_status_error(status, &body_text, Some(id)))
    }

    /// List all sandboxes visible to this provider via `GET /v1/sandboxes`.
    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        let url = self.url("/v1/sandboxes");

        debug!("listing Vercel sandboxes");

        let resp = self.send_with_retry(|| self.client.get(&url)).await?;
        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| SandboxError::ProviderError {
            provider: "vercel",
            message: format!("failed to read response body: {e}"),
        })?;

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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(unsafe_code)] // env mutation in tests requires unsafe (Rust 2024)
mod tests {
    use super::*;

    /// Helper: build a provider that talks to the mock server.
    fn provider_for(server: &mockito::Server) -> VercelSandboxProvider {
        VercelSandboxProvider::new("test-token", None).with_base_url(server.url())
    }

    /// Canonical ISO-8601 timestamp used across fixtures.
    const CREATED_AT: &str = "2026-01-01T00:00:00Z";

    /// JSON fixture for a single VercelSandbox in "running" state.
    fn running_sandbox_json(id: &str, name: &str) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","status":"running","createdAt":"{CREATED_AT}"}}"#)
    }

    // ── Static property tests (no network) ───────────────────────────────────

    #[test]
    fn name_is_vercel() {
        let p = VercelSandboxProvider::new("tok", None);
        assert_eq!(p.name(), "vercel");
    }

    #[test]
    fn capabilities_include_network_outbound() {
        let p = VercelSandboxProvider::new("tok", None);
        assert!(
            p.capabilities()
                .contains(SandboxCapabilitySet::NETWORK_OUTBOUND)
        );
    }

    #[test]
    fn from_env_returns_err_without_token() {
        // Inject a controlled env-lookup that never returns a value, avoiding
        // any mutation of the process environment (which requires `unsafe`
        // in Rust edition 2024 and is not permitted by workspace lints).
        let result =
            VercelSandboxProvider::from_env_fn(|_key: &str| -> Result<String, &'static str> {
                Err("not set")
            });
        assert!(result.is_err(), "expected Err when no token env var is set");
    }

    // ── HTTP-level tests (mockito) ────────────────────────────────────────────

    #[tokio::test]
    async fn create_maps_spec_to_request() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/sandboxes")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(running_sandbox_json("sbx-abc", "test-sandbox"))
            .create_async()
            .await;

        let provider = provider_for(&server);
        let spec = SandboxSpec::ephemeral("test-sandbox");
        let handle = provider.create(spec).await.expect("create should succeed");

        assert_eq!(handle.id.0, "sbx-abc");
        assert_eq!(handle.name, "test-sandbox");
        assert_eq!(handle.status, SandboxStatus::Running);
        assert_eq!(handle.provider, "vercel");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn run_maps_exec_request() {
        let sandbox_id = "sbx-xyz";
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", format!("/v1/sandboxes/{sandbox_id}/exec").as_str())
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"stdout":"hello\n","stderr":"","exitCode":0,"durationMs":42}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        let id = SandboxId(sandbox_id.into());
        let req = ExecRequest::shell("echo hello");
        let result = provider.run(&id, req).await.expect("run should succeed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration_ms, 42);
        // stdout is the raw JSON string value (with literal \n escape)
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn error_404_maps_to_not_found() {
        let sandbox_id = "sbx-missing";
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/sandboxes")
            .with_status(404)
            .with_body(r#"{"error":"not found"}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        let err = provider.list().await.expect_err("should return error");
        assert!(
            matches!(err, SandboxError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
        // suppress unused variable warning
        let _ = sandbox_id;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn error_429_retries() {
        let mut server = mockito::Server::new_async().await;

        // First call → 429, second call → 200
        let mock_429 = server
            .mock("GET", "/v1/sandboxes")
            .with_status(429)
            .with_body("")
            .create_async()
            .await;

        let mock_200 = server
            .mock("GET", "/v1/sandboxes")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"sandboxes":[]}"#)
            .create_async()
            .await;

        let provider = provider_for(&server);
        let result = provider.list().await;
        assert!(result.is_ok(), "should succeed after retry; got {result:?}");
        assert_eq!(result.unwrap().len(), 0);

        mock_429.assert_async().await;
        mock_200.assert_async().await;
    }
}
