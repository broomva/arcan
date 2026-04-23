//! End-to-end: canned EventRecord stream → fanout subscriber sees translated envelopes.

use aios_protocol::{BranchId, EventKind, EventRecord, SessionId};
use arcan_prosopon::ArcanProsoponBridge;
use prosopon_daemon::EnvelopeFanout;
use tokio::sync::broadcast;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_forwards_run_started_as_three_signal_envelopes() {
    let (event_tx, event_rx) = broadcast::channel::<EventRecord>(16);
    let fanout = EnvelopeFanout::new();
    let mut subscriber = fanout.subscribe();

    let bridge = ArcanProsoponBridge::new(fanout);
    let handle = bridge.spawn(event_rx);

    event_tx
        .send(EventRecord::new(
            SessionId::default(),
            BranchId::main(),
            1u64,
            EventKind::RunStarted {
                provider: "anthropic".into(),
                max_iterations: 5,
            },
        ))
        .expect("send ok");

    for _ in 0..3 {
        let env = tokio::time::timeout(std::time::Duration::from_secs(2), subscriber.recv())
            .await
            .expect("timeout waiting for envelope")
            .expect("recv ok");
        assert!(matches!(
            env.event,
            prosopon_core::ProsoponEvent::SignalChanged { .. }
        ));
    }

    drop(event_tx);
    let _ = handle.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_exits_when_upstream_closes() {
    let (event_tx, event_rx) = broadcast::channel::<EventRecord>(4);
    let fanout = EnvelopeFanout::new();

    let bridge = ArcanProsoponBridge::new(fanout);
    let handle = bridge.spawn(event_rx);

    drop(event_tx);
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "bridge should exit promptly when upstream closes"
    );
}
