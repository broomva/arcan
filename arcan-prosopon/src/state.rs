//! Per-session translator state (stream registry, iteration counter).

use prosopon_core::{NodeId, StreamId};
use std::collections::HashMap;

/// Mutable state maintained across a single arcan session's event stream.
#[derive(Debug)]
pub struct TranslationState {
    /// Active streaming intent id per iteration, for folding `*TextDelta` events.
    pub streams_by_iteration: HashMap<u32, StreamId>,
    /// Monotonic sequence counter per stream, for `StreamChunk::seq`.
    pub stream_seq: HashMap<StreamId, u64>,
    /// Current iteration number, if an assistant turn is in progress.
    pub current_iteration: Option<u32>,
    /// Stable id of the scene root, used as the parent for session-scoped
    /// `NodeAdded` events. Reset on every `SessionCreated`.
    pub scene_root: NodeId,
}

impl TranslationState {
    pub fn new() -> Self {
        Self {
            streams_by_iteration: HashMap::new(),
            stream_seq: HashMap::new(),
            current_iteration: None,
            scene_root: NodeId::from_raw("session-root"),
        }
    }
}

impl Default for TranslationState {
    fn default() -> Self {
        Self::new()
    }
}
