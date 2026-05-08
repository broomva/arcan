//! `ergon::RuntimeHandle` implementation over a captured kernel mode.
//!
//! The kernel passes the current [`aios_protocol::OperatingMode`] into
//! the dispatcher per tick. We snapshot it into a tiny struct and hand
//! it to ergon as `Arc<dyn RuntimeHandle>`. The mode is the only
//! kernel-side fact ergon's v0.1 surface exposes (per the spec
//! deviation notes in `crates/ergon/ergon/CLAUDE.md`).
//!
//! Future expansion (e.g. `aios_caps()`, `edit_hashline()`) lands as a
//! deliberate boundary widening — adding methods to
//! [`ergon::RuntimeHandle`] AND updating this adapter.

use aios_protocol::mode::OperatingMode;
use ergon::RuntimeHandle;

/// Thin `RuntimeHandle` that returns a captured operating mode.
///
/// One instance per tick — constructed by
/// [`crate::run_workflow_as_tick`] from
/// [`aios_runtime::WorkflowTickInvocation::mode`].
#[derive(Debug, Clone, Copy)]
pub struct ModeRuntimeHandle {
    mode: OperatingMode,
}

impl ModeRuntimeHandle {
    /// Construct from a snapshotted operating mode.
    pub fn new(mode: OperatingMode) -> Self {
        Self { mode }
    }
}

impl RuntimeHandle for ModeRuntimeHandle {
    fn operating_mode(&self) -> OperatingMode {
        self.mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn returns_captured_mode() {
        let h = ModeRuntimeHandle::new(OperatingMode::Execute);
        assert_eq!(h.operating_mode(), OperatingMode::Execute);
    }

    #[test]
    fn handles_all_modes() {
        for mode in [
            OperatingMode::Explore,
            OperatingMode::Execute,
            OperatingMode::Verify,
            OperatingMode::Recover,
            OperatingMode::AskHuman,
            OperatingMode::Sleep,
        ] {
            let h: Arc<dyn RuntimeHandle> = Arc::new(ModeRuntimeHandle::new(mode));
            assert_eq!(h.operating_mode(), mode);
        }
    }
}
