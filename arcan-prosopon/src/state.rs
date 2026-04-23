//! Per-session translator state (stream registry, iteration counter).

use prosopon_core::StreamId;
use std::collections::HashMap;

/// Mutable state maintained across a single arcan session's event stream.
#[derive(Debug, Default)]
pub struct TranslationState {
    /// Active streaming intent id per iteration, for folding `*TextDelta` events.
    pub streams_by_iteration: HashMap<u32, StreamId>,
    /// Current iteration number, if an assistant turn is in progress.
    pub current_iteration: Option<u32>,
}

impl TranslationState {
    pub fn new() -> Self {
        Self::default()
    }
}
