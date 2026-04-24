//! HTTP API DTOs for arcand.
//!
//! Mirrors the session-tier surface defined by `aios-protocol::session`.

#![forbid(unsafe_code)]

pub use aios_protocol::session::{
    CreateSessionRequest, SessionFilter, SessionManifest, TickInput, TickOutput,
};

pub use aios_protocol::identity::{Belief, BeliefFilter, SoulUpdate};
pub use aios_protocol::memory::SoulProfile;

/// Server-specific wrapper around SSE event records — arcand's stream frame shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamFrame {
    pub event: aios_protocol::event::EventRecord,
}
