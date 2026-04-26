//! Error surface for the Cube backend. Populated in Task 3.

use thiserror::Error;

/// Errors surfaced by [`crate::CubeProvider`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CubeError {
    /// Placeholder until Task 3 fills in the real variants.
    #[error("not yet implemented: {0}")]
    NotYetImplemented(&'static str),
}
