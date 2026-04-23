//! Bridge — spawns the subscriber task that drains arcan events into the
//! prosopon fanout.

use crate::{BridgeError, TranslationState, translator::translate};
use aios_protocol::EventRecord;
use prosopon_daemon::EnvelopeFanout;
use prosopon_sdk::Session;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// Drains an `aios_protocol::EventRecord` broadcast into a Prosopon fanout.
///
/// Produced envelopes share a single Prosopon `SessionId` (minted by `Session::new`)
/// regardless of the upstream `EventRecord.session_id` — compositors can multiplex
/// Prosopon sessions separately if needed by upgrading this to a per-arcan-session
/// `HashMap<SessionId, Session>` in a future revision.
pub struct ArcanProsoponBridge {
    fanout: EnvelopeFanout,
    session: Session,
    state: TranslationState,
}

impl ArcanProsoponBridge {
    /// Create a bridge that publishes to the supplied fanout.
    pub fn new(fanout: EnvelopeFanout) -> Self {
        Self {
            fanout,
            session: Session::new(),
            state: TranslationState::new(),
        }
    }

    /// Spawn the drain loop on the current tokio runtime. The loop exits when
    /// the upstream broadcast closes.
    pub fn spawn(mut self, mut events: broadcast::Receiver<EventRecord>) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(record) => {
                        if let Err(err) = self.drain_one(&record) {
                            warn!(error = %err, "arcan-prosopon: translation/publish failed");
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(lagged = n, "arcan-prosopon: dropped events due to lag");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("arcan-prosopon: upstream closed, exiting bridge");
                        return;
                    }
                }
            }
        })
    }

    fn drain_one(&mut self, record: &EventRecord) -> Result<(), BridgeError> {
        for event in translate(&mut self.state, &record.kind) {
            let envelope = self.session.envelope(event);
            self.fanout.send(envelope)?;
        }
        Ok(())
    }
}
