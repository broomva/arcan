//! Error types for sandbox operations.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display() {
        let err = SandboxError::NotFound(SandboxId("abc-123".into()));
        assert_eq!(err.to_string(), "sandbox not found: abc-123");
    }

    #[test]
    fn not_supported_display() {
        let err = SandboxError::NotSupported { provider: "local", reason: "persistence" };
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
}
