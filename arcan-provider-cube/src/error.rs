//! Error surface for the Cube backend.
//!
//! `CubeError` is the crate-internal error type. Every public method on
//! [`crate::CubeProvider`] returns
//! [`aios_protocol::hypervisor::BackendError`]; the conversion at the
//! boundary lives in this module so wire-level details do not leak into
//! the kernel ABI.

use aios_protocol::hypervisor::{BackendError, VmId, VmSnapshotId};
use thiserror::Error;

/// Internal error surface of the Cube backend.
///
/// The variants partition by HTTP status class so the conversion into
/// [`BackendError`] can preserve enough information for the kernel to
/// log + retry intelligently without exposing raw response bodies.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CubeError {
    /// Network-level failure (connect, DNS, TLS, body decode).
    #[error("transport: {0}")]
    Transport(String),

    /// JSON decode of a `2xx` response body failed.
    #[error("decode: {0}")]
    Decode(String),

    /// `400 Bad Request` — the request was malformed.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// `401`/`403` — the bearer token is missing, invalid, or lacks
    /// permission.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// `404 Not Found` — the referenced VM/snapshot does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// `409 Conflict` — concurrent state mutation (e.g. snapshot while
    /// already snapshotting).
    #[error("conflict: {0}")]
    Conflict(String),

    /// `429 Too Many Requests` — Cube rate-limited the call.
    #[error("rate limited (retry after {retry_after_secs:?}s): {message}")]
    RateLimited {
        /// Seconds to wait before retrying, parsed from `Retry-After`.
        retry_after_secs: Option<u64>,
        /// Human-readable rate-limit reason from the Cube control plane.
        message: String,
    },

    /// `5xx` — Cube control-plane error.
    #[error("server error ({status}): {message}")]
    Server {
        /// HTTP status code returned by the Cube control plane.
        status: u16,
        /// Human-readable failure message from the Cube control plane.
        message: String,
    },

    /// The request exceeded its client-side deadline.
    #[error("timeout after {duration_ms} ms")]
    Timeout {
        /// Duration of the request before the timeout fired.
        duration_ms: u64,
    },

    /// The operation is not supported by CubeAPI v1 (e.g. hibernate).
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

impl CubeError {
    /// Convert into [`BackendError`] preserving the failure category.
    ///
    /// `vm_hint` / `snapshot_hint` let the caller pin the correct
    /// `VmNotFound` / `SnapshotNotFound` variant when a 404 occurred —
    /// `BackendError` requires an id, but the HTTP layer does not always
    /// know which one was at fault.
    pub fn into_backend_error(
        self,
        vm_hint: Option<VmId>,
        snapshot_hint: Option<VmSnapshotId>,
    ) -> BackendError {
        match self {
            Self::NotFound(_) => match (vm_hint, snapshot_hint) {
                (Some(vm), _) => BackendError::VmNotFound(vm),
                (None, Some(snap)) => BackendError::SnapshotNotFound(snap),
                (None, None) => BackendError::Internal("404 with no id hint".into()),
            },
            Self::Unsupported(reason) => BackendError::NotSupported {
                backend: "cube",
                reason,
            },
            Self::Timeout { duration_ms } => BackendError::Timeout { duration_ms },
            Self::Transport(msg) | Self::Decode(msg) => BackendError::Transport(msg),
            Self::BadRequest(msg)
            | Self::Unauthorized(msg)
            | Self::Conflict(msg)
            | Self::RateLimited { message: msg, .. } => BackendError::Internal(msg),
            Self::Server { status, message } => {
                BackendError::Internal(format!("cube {status}: {message}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_with_vm_hint_maps_to_vm_not_found() {
        let err = CubeError::NotFound("vm-1 missing".into());
        let mapped = err.into_backend_error(Some(VmId::from("vm-1")), None);
        assert!(matches!(mapped, BackendError::VmNotFound(ref id) if id.0 == "vm-1"));
    }

    #[test]
    fn not_found_with_snapshot_hint_maps_to_snapshot_not_found() {
        let err = CubeError::NotFound("snap-1 missing".into());
        let mapped = err.into_backend_error(None, Some(VmSnapshotId::from("snap-1")));
        assert!(matches!(mapped, BackendError::SnapshotNotFound(ref id) if id.0 == "snap-1"));
    }

    #[test]
    fn unsupported_maps_to_not_supported_with_static_reason() {
        let err = CubeError::Unsupported("hibernate");
        let mapped = err.into_backend_error(None, None);
        assert!(matches!(
            mapped,
            BackendError::NotSupported {
                backend: "cube",
                reason: "hibernate"
            }
        ));
    }

    #[test]
    fn timeout_round_trips_duration() {
        let err = CubeError::Timeout { duration_ms: 1234 };
        let mapped = err.into_backend_error(None, None);
        assert!(matches!(
            mapped,
            BackendError::Timeout { duration_ms: 1234 }
        ));
    }

    #[test]
    fn transport_maps_to_transport() {
        let err = CubeError::Transport("connection refused".into());
        let mapped = err.into_backend_error(None, None);
        assert!(matches!(mapped, BackendError::Transport(_)));
    }

    #[test]
    fn server_maps_to_internal_with_status() {
        let err = CubeError::Server {
            status: 503,
            message: "vm pool exhausted".into(),
        };
        let mapped = err.into_backend_error(None, None);
        match mapped {
            BackendError::Internal(msg) => assert!(msg.contains("503")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}
