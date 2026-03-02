use arcan_core::protocol::AgentEvent;
use crossterm::event::{self, Event, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

/// Unified TUI event type merging terminal, network, and timer events.
pub enum TuiEvent {
    /// A key press from the terminal.
    Key(KeyEvent),
    /// Periodic tick for UI refresh and animation.
    Tick,
    /// An agent event from the daemon SSE stream.
    Network(AgentEvent),
    /// Terminal resize event.
    Resize(u16, u16),
}

/// Spawn background producers that merge terminal input, network events,
/// and periodic ticks into a single `mpsc::Receiver<TuiEvent>`.
///
/// Returns both the receiver and sender, so that callers can inject additional
/// network events (e.g. after a session switch rewires the event stream).
///
/// - Terminal events are read on a dedicated OS thread (crossterm is blocking).
/// - Network events are forwarded from the given receiver.
/// - Ticks are emitted whenever the crossterm poll times out.
pub fn event_pump(
    network_rx: mpsc::Receiver<AgentEvent>,
    tick_rate: Duration,
) -> (mpsc::Receiver<TuiEvent>, mpsc::Sender<TuiEvent>) {
    let (tx, rx) = mpsc::channel(256);

    // Terminal events — must run on a dedicated OS thread (crossterm is blocking)
    let term_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            if event::poll(tick_rate).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if term_tx.blocking_send(TuiEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Event::Resize(w, h)) => {
                        if term_tx.blocking_send(TuiEvent::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            } else {
                // Poll timeout = emit tick
                if term_tx.blocking_send(TuiEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    // Forward network events into the unified channel
    let net_tx = tx.clone();
    tokio::spawn(async move {
        let mut network_rx = network_rx;
        while let Some(agent_event) = network_rx.recv().await {
            if net_tx.send(TuiEvent::Network(agent_event)).await.is_err() {
                break;
            }
        }
    });

    (rx, tx)
}
