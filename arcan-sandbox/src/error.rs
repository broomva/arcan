//! Error types for sandbox operations.

use aios_protocol::hypervisor::BackendError;
use thiserror::Error;

use crate::types::SandboxId;

/// All errors that can occur during sandbox lifecycle operations.
#[derive(Debug, Error)]
pub enum SandboxError {
    /// No sandbox with the given ID exists or is visible to this provider.
    #[error("sandbox not found: {0}")]
    NotFound(SandboxId),

    /// The requested operation is not supported by this provider.
    ///
    /// `reason` names the unsupported feature (e.g., `"persistence"`,
    /// `"custom_image"`).
    #[error("operation not supported by provider '{provider}': {reason}")]
    NotSupported {
        /// Provider that rejected the call.
        provider: &'static str,
        /// Human-readable name of the unsupported feature.
        reason: &'static str,
    },

    /// A provider-specific error that doesn't map to a structured variant.
    #[error("provider '{provider}' error: {message}")]
    ProviderError {
        /// Provider that emitted the error.
        provider: &'static str,
        /// Raw error message from the provider SDK.
        message: String,
    },

    /// The command inside the sandbox did not complete within the allowed time.
    #[error("exec timed out in sandbox {sandbox_id} after {timeout_secs}s")]
    ExecTimeout {
        /// Sandbox in which the timeout occurred.
        sandbox_id: SandboxId,
        /// Configured timeout that was exceeded.
        timeout_secs: u64,
    },

    /// The spec requested a capability the policy does not grant.
    #[error("capability denied: {capability}")]
    CapabilityDenied {
        /// Name of the denied capability bit.
        capability: &'static str,
    },

    /// JSON serialization/deserialization error (e.g., provider metadata).
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

// ── BackendError → SandboxError bridge ────────────────────────────────────────
//
// Added in BRO-852 as part of the `HypervisorBackend` migration (see
// `crates/aios/aios-protocol/src/hypervisor.rs`). The `SandboxProvider` trait
// is now a deprecated compat shim over `HypervisorBackend`; this conversion
// lets the blanket impl forward backend failures to legacy callers without
// lossy stringification where structured variants exist.

impl From<BackendError> for SandboxError {
    fn from(e: BackendError) -> Self {
        match e {
            BackendError::VmNotFound(vm_id) => SandboxError::NotFound(SandboxId(vm_id.to_string())),
            BackendError::SnapshotNotFound(snap_id) => {
                SandboxError::NotFound(SandboxId(snap_id.to_string()))
            }
            BackendError::NotSupported { backend, reason } => SandboxError::NotSupported {
                provider: backend,
                reason,
            },
            // `SandboxError::CapabilityDenied` only carries a `&'static str`
            // today while `BackendError::CapabilityDenied` carries a
            // `BackendCapabilitySet`. Fall back to a provider error so the
            // structured detail survives in the message.
            BackendError::CapabilityDenied(caps) => SandboxError::ProviderError {
                provider: "hypervisor",
                message: format!("capability denied: {caps:?}"),
            },
            // Legacy ExecTimeout requires a sandbox id; the backend error has
            // no such field, so we synthesise a placeholder rather than invent
            // a non-existent id. Callers that need the real id should map
            // themselves (the blanket impl does not have enough context).
            BackendError::Timeout { duration_ms } => SandboxError::ExecTimeout {
                sandbox_id: SandboxId("<unknown>".into()),
                timeout_secs: duration_ms.div_ceil(1_000),
            },
            BackendError::Transport(message) => SandboxError::ProviderError {
                provider: "hypervisor",
                message,
            },
            BackendError::Internal(message) => SandboxError::ProviderError {
                provider: "hypervisor",
                message,
            },
            // `BackendError` is `#[non_exhaustive]`; fall back to a provider
            // error stringified via `Display` if a future variant lands in
            // aios-protocol before this bridge is updated.
            other => SandboxError::ProviderError {
                provider: "hypervisor",
                message: other.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use aios_protocol::hypervisor::{BackendCapabilitySet, VmId, VmSnapshotId};

    use super::*;

    #[test]
    fn not_found_display() {
        let err = SandboxError::NotFound(SandboxId("abc-123".into()));
        assert_eq!(err.to_string(), "sandbox not found: abc-123");
    }

    #[test]
    fn not_supported_display() {
        let err = SandboxError::NotSupported {
            provider: "local",
            reason: "persistence",
        };
        assert!(err.to_string().contains("persistence"));
        assert!(err.to_string().contains("local"));
    }

    #[test]
    fn exec_timeout_display() {
        let err = SandboxError::ExecTimeout {
            sandbox_id: SandboxId("s1".into()),
            timeout_secs: 30,
        };
        assert!(err.to_string().contains("30s"));
    }

    #[test]
    fn capability_denied_display() {
        let err = SandboxError::CapabilityDenied { capability: "GPU" };
        assert!(err.to_string().contains("GPU"));
    }

    // ── BackendError → SandboxError conversions ──────────────────────────────

    #[test]
    fn backend_error_vm_not_found_maps_to_not_found() {
        let e: SandboxError = BackendError::VmNotFound(VmId::from("vm-42")).into();
        match e {
            SandboxError::NotFound(id) => assert_eq!(id.0, "vm-42"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_snapshot_not_found_maps_to_not_found() {
        let e: SandboxError = BackendError::SnapshotNotFound(VmSnapshotId::from("snap-1")).into();
        match e {
            SandboxError::NotFound(id) => assert_eq!(id.0, "snap-1"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_not_supported_preserves_fields() {
        let e: SandboxError = BackendError::NotSupported {
            backend: "local",
            reason: "hibernate",
        }
        .into();
        match e {
            SandboxError::NotSupported { provider, reason } => {
                assert_eq!(provider, "local");
                assert_eq!(reason, "hibernate");
            }
            other => panic!("expected NotSupported, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_capability_denied_maps_to_provider_error() {
        let caps = BackendCapabilitySet::FILESYSTEM_WRITE | BackendCapabilitySet::GPU;
        let e: SandboxError = BackendError::CapabilityDenied(caps).into();
        match e {
            SandboxError::ProviderError { provider, message } => {
                assert_eq!(provider, "hypervisor");
                assert!(message.contains("FILESYSTEM_WRITE") || message.contains("GPU"));
            }
            other => panic!("expected ProviderError, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_timeout_rounds_up_to_secs() {
        let e: SandboxError = BackendError::Timeout { duration_ms: 1_500 }.into();
        match e {
            SandboxError::ExecTimeout { timeout_secs, .. } => assert_eq!(timeout_secs, 2),
            other => panic!("expected ExecTimeout, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_transport_maps_to_provider_error() {
        let e: SandboxError = BackendError::Transport("connection refused".into()).into();
        match e {
            SandboxError::ProviderError { provider, message } => {
                assert_eq!(provider, "hypervisor");
                assert_eq!(message, "connection refused");
            }
            other => panic!("expected ProviderError, got {other:?}"),
        }
    }

    #[test]
    fn backend_error_internal_maps_to_provider_error() {
        let e: SandboxError = BackendError::Internal("bug".into()).into();
        match e {
            SandboxError::ProviderError { provider, message } => {
                assert_eq!(provider, "hypervisor");
                assert_eq!(message, "bug");
            }
            other => panic!("expected ProviderError, got {other:?}"),
        }
    }
}
