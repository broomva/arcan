//! Error type for the arcan-prosopon bridge.

use thiserror::Error;

/// Errors raised while running the Arcan → Prosopon bridge.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BridgeError {
    /// The upstream Arcan event broadcast closed.
    #[error("arcan event stream closed")]
    UpstreamClosed,

    /// Publishing to the Prosopon fanout failed.
    #[error("prosopon fanout send failed: {0}")]
    Fanout(#[from] prosopon_daemon::FanoutError),

    /// Envelope encoding failed (should be unreachable — JSON serialisation of known types).
    #[error("envelope encoding failed: {0}")]
    Encoding(#[from] prosopon_protocol::ProtocolError),
}
