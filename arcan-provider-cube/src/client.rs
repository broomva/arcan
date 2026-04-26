//! HTTP client wrapping `reqwest::Client` for CubeAPI v1.
//!
//! Owns the bearer token, base URL, request timeout, and the retry
//! policy. Every network method returns [`Result<T, CubeError>`] —
//! callers in `lib.rs` map `CubeError` to `BackendError` at the trait
//! boundary.
//!
//! ## Retry policy
//!
//! `429`, `502`, `503`, `504`, and transport-level failures (DNS,
//! connect, body decode) trigger **at most one** retry with a 100ms
//! backoff. `4xx` other than `429` are not retried. The single-retry
//! cap is intentional — the kernel layer is responsible for
//! backend-level circuit-breaking once Phase 4 lands its
//! `NetworkIsolationPort` controller; the per-request retry exists
//! only to absorb single-flap failures during a docker-compose Cube
//! restart.

#![allow(dead_code)] // populated by the trait impl in Tasks 6–8.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::{Serialize, de::DeserializeOwned};

use crate::error::CubeError;
use crate::types::ApiErrorEnvelope;

/// Number of retry attempts for transient failures (in addition to
/// the initial request).
const MAX_RETRIES: u32 = 1;

/// Backoff between the first attempt and the retry.
const RETRY_BACKOFF: Duration = Duration::from_millis(100);

/// Default per-request timeout.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Thin HTTP client over CubeAPI v1.
///
/// `CubeClient` is `Clone` because `reqwest::Client` is `Clone` and
/// internally pools connections — a clone is essentially free.
#[derive(Debug, Clone)]
pub(crate) struct CubeClient {
    http: reqwest::Client,
    base_url: String,
    bearer_token: String,
}

impl CubeClient {
    pub(crate) fn new(base_url: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("rustls TLS is built into reqwest with rustls-tls feature");
        Self {
            http,
            base_url: base_url.into(),
            bearer_token: bearer_token.into(),
        }
    }

    /// Test-only constructor accepting a pre-built `reqwest::Client`.
    #[cfg(test)]
    pub(crate) fn with_http(
        http: reqwest::Client,
        base_url: impl Into<String>,
        bearer_token: impl Into<String>,
    ) -> Self {
        Self {
            http,
            base_url: base_url.into(),
            bearer_token: bearer_token.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Send a body-less request (GET / DELETE) and decode the JSON response.
    pub(crate) async fn request_no_body<R>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<R, CubeError>
    where
        R: DeserializeOwned,
    {
        self.send::<(), R>(method, path, None).await
    }

    /// Send a JSON-encoded request body and decode the JSON response.
    pub(crate) async fn request<B, R>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<R, CubeError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        self.send(method, path, Some(body)).await
    }

    async fn send<B, R>(&self, method: Method, path: &str, body: Option<&B>) -> Result<R, CubeError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = self.url(path);
        let mut last_err: Option<CubeError> = None;
        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(RETRY_BACKOFF).await;
            }
            let mut req = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&self.bearer_token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let response = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_timeout() => {
                    return Err(CubeError::Timeout {
                        duration_ms: DEFAULT_TIMEOUT.as_millis() as u64,
                    });
                }
                Err(e) => {
                    last_err = Some(CubeError::Transport(e.to_string()));
                    continue; // retry transport errors
                }
            };
            match Self::translate_response::<R>(response).await {
                Ok(value) => return Ok(value),
                Err(err) if Self::is_retryable(&err) => {
                    last_err = Some(err);
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| CubeError::Transport("retry budget exhausted".into())))
    }

    async fn translate_response<R>(response: reqwest::Response) -> Result<R, CubeError>
    where
        R: DeserializeOwned,
    {
        let status = response.status();
        if status.is_success() {
            return response
                .json::<R>()
                .await
                .map_err(|e| CubeError::Decode(e.to_string()));
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let body_text = response.text().await.unwrap_or_default();
        let message = match serde_json::from_str::<ApiErrorEnvelope>(&body_text) {
            Ok(env) => env.error.message,
            Err(_) => body_text,
        };
        Err(match status {
            StatusCode::BAD_REQUEST => CubeError::BadRequest(message),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => CubeError::Unauthorized(message),
            StatusCode::NOT_FOUND => CubeError::NotFound(message),
            StatusCode::CONFLICT => CubeError::Conflict(message),
            StatusCode::TOO_MANY_REQUESTS => CubeError::RateLimited {
                retry_after_secs: retry_after,
                message,
            },
            s if s.is_server_error() => CubeError::Server {
                status: s.as_u16(),
                message,
            },
            other => CubeError::Server {
                status: other.as_u16(),
                message,
            },
        })
    }

    fn is_retryable(err: &CubeError) -> bool {
        matches!(
            err,
            CubeError::Server { status, .. }
                if matches!(*status, 502..=504)
        ) || matches!(err, CubeError::Transport(_))
            || matches!(err, CubeError::RateLimited { .. })
    }
}
