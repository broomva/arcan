//! Bridge — spawns the subscriber task that drains arcan events into the
//! prosopon fanout.

use crate::TranslationState;
use prosopon_daemon::EnvelopeFanout;
use prosopon_sdk::Session;

/// Drains an `aios_protocol::EventRecord` broadcast into a Prosopon fanout.
pub struct ArcanProsoponBridge {
    _fanout: EnvelopeFanout,
    _session: Session,
    _state: TranslationState,
}

impl ArcanProsoponBridge {
    pub fn new(fanout: EnvelopeFanout) -> Self {
        Self {
            _fanout: fanout,
            _session: Session::new(),
            _state: TranslationState::new(),
        }
    }
}
