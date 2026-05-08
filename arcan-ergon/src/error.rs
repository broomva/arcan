//! Adapter-internal error type.
//!
//! `AdapterError` covers the boundary translation failures the adapter
//! itself can hit (unknown workflow, input deserialization, output
//! serialization, port-call failure). Errors that originate inside
//! `ergon::*` are returned as [`AdapterError::Workflow`] preserving
//! the wrapped [`ergon::ErgonError`] for diagnostic fidelity.

use thiserror::Error;

/// Errors produced by the kernel-side ergon adapter.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AdapterError {
    /// `TickKind::Workflow { name }` referenced a workflow that has not
    /// been registered with the [`crate::WorkflowRegistry`]. Surfaces
    /// the supplied name and the known names for fast diagnosis.
    #[error("unknown workflow `{name}` (registered: {known:?})")]
    UnknownWorkflow { name: String, known: Vec<String> },

    /// `serde_json::from_value` failed when deserializing the workflow
    /// input from the [`aios_runtime::TickKind::Workflow::input`] JSON.
    #[error("workflow `{workflow}` input deserialization failed: {source}")]
    InputDeserialize {
        workflow: String,
        #[source]
        source: serde_json::Error,
    },

    /// `serde_json::to_value` failed when serializing the workflow's
    /// typed output back to JSON for the kernel's
    /// [`aios_runtime::WorkflowTickOutcome::output`] field.
    #[error("workflow `{workflow}` output serialization failed: {source}")]
    OutputSerialize {
        workflow: String,
        #[source]
        source: serde_json::Error,
    },

    /// A wrapped error from inside `ergon` (provider failure, hook
    /// denial, max-turns exceeded, etc.).
    #[error("workflow `{workflow}` execution failed: {source}")]
    Workflow {
        workflow: String,
        #[source]
        source: ergon::ErgonError,
    },

    /// A port call (`ModelProviderPort::complete`, `ToolHarnessPort::execute`,
    /// `PolicyGatePort::evaluate`, ...) returned a [`aios_protocol::KernelError`].
    /// The wrapped error preserves the original taxonomy.
    #[error("port `{port}` failed: {message}")]
    Port { port: &'static str, message: String },
}

impl AdapterError {
    /// Construct an [`AdapterError::Port`] from any error type that
    /// implements [`std::fmt::Display`]. Used when adapting
    /// `KernelError` → `AdapterError` at port boundaries.
    pub fn port<E: std::fmt::Display>(port: &'static str, error: E) -> Self {
        Self::Port {
            port,
            message: error.to_string(),
        }
    }
}

/// Convenience alias for adapter-internal results.
pub type Result<T> = std::result::Result<T, AdapterError>;
