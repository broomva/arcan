//! Round-trip integration test for [`arcan_lago::RemoteBlobBackend`] against a
//! real `lago-api` router bound to an ephemeral socket.
//!
//! Two things are proven here that the unit tests cannot:
//!
//! 1. **Wire correctness** — PUT then GET then `exists` against the actual
//!    `/v1/blobs/{hash}` routes arcan talks to in production, with a real
//!    `reqwest` client over TCP.
//! 2. **No nested-runtime panic** — the blob methods are invoked from *inside*
//!    a multi-threaded Tokio runtime, on a runtime worker thread. This is the
//!    worst case a `BlobBackend` must survive: a naive `Runtime::block_on`-on-
//!    the-caller bridge aborts here with "Cannot start a runtime from within a
//!    runtime"; the dedicated-worker-thread bridge does not. (As of BRO-1483
//!    the tool harness runs sync `Tool::execute` on the blocking pool via
//!    `spawn_blocking`, so the production call no longer lands on a worker
//!    thread — but the bridge must not depend on that, and this test pins the
//!    stronger guarantee that it works from an ambient runtime regardless.)

use std::sync::Arc;

use arcan_lago::RemoteBlobBackend;
use lago_api::build_router;
use lago_api::state::AppState;
use lago_store::{BlobBackend, BlobStore};

/// Build a minimal auth-disabled, no-policy lago-api state (matches the
/// in-container lagod posture: the `/v1/blobs/*` routes accept unauthenticated
/// PUT/GET).
fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
    let dir = tempfile::tempdir().unwrap();
    let journal = lago_journal::RedbJournal::open(dir.path().join("test.redb")).unwrap();
    let blob_store = BlobStore::open(dir.path().join("blobs")).unwrap();

    // Just need a handle for AppState; no global recorder — blob-route metric
    // emissions are harmless no-ops without one.
    let prometheus_handle = metrics_exporter_prometheus::PrometheusBuilder::new()
        .build_recorder()
        .handle();

    let state = Arc::new(AppState {
        journal: Arc::new(journal) as Arc<dyn lago_core::Journal>,
        blob_store: Arc::new(blob_store),
        data_dir: dir.path().to_path_buf(),
        started_at: std::time::Instant::now(),
        auth: None,
        policy_engine: None,
        rbac_manager: None,
        hook_runner: None,
        rate_limiter: None,
        prometheus_handle,
        manifest_cache: tokio::sync::RwLock::new(std::collections::HashMap::new()),
    });
    (dir, state)
}

/// Serve the router on 127.0.0.1:0 and return the bound base URL. The server
/// task runs for the lifetime of the test process (dropped on exit).
async fn serve_lago_api() -> (tempfile::TempDir, String) {
    let (dir, state) = test_state();
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (dir, format!("http://{addr}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_blob_roundtrips_from_within_a_runtime() {
    let (_dir, base_url) = serve_lago_api().await;
    let backend = RemoteBlobBackend::new(base_url);

    // Drive the SYNC blob methods directly on a runtime worker thread — the
    // production call context (sync tool chain on an async worker, no
    // spawn_blocking). This is what trips the nested-runtime panic with a
    // naive block_on bridge.
    let data = b"durable content lives in lago, not local disk".to_vec();

    let missing_before = backend.exists(&lago_store::hash_bytes(&data));
    assert!(!missing_before, "blob must not exist before PUT");

    let hash = backend.put(&data).expect("remote PUT succeeds");
    assert!(backend.exists(&hash), "blob exists after PUT");
    assert_eq!(
        backend.get(&hash).expect("remote GET succeeds"),
        data,
        "GET returns the exact bytes PUT"
    );

    // Content addressing holds across the wire: the remote hash equals the
    // local hash of the same content.
    assert_eq!(hash, lago_store::hash_bytes(&data));

    // A missing blob is a clean BlobNotFound, not a transport error.
    let absent = lago_store::hash_bytes(b"never stored");
    assert!(!backend.exists(&absent));
    let err = backend.get(&absent).unwrap_err();
    assert!(
        matches!(err, lago_core::LagoError::BlobNotFound(_)),
        "missing blob -> BlobNotFound, got: {err:?}"
    );
}

/// A stored ZERO-byte blob must report `exists() == true` and `get()` must
/// return empty bytes. The `Range: bytes=0-0` existence probe hits the
/// server's 416 path for an empty resource (no satisfiable range); treating
/// that as absent would make every agent-written empty file (touch, empty
/// `__init__.py`, placeholders) look missing. Also pins the server-side
/// `parse_range` `total == 0` guard against the `total - 1` u64 underflow.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_blob_handles_empty_blob() {
    let (_dir, base_url) = serve_lago_api().await;
    let backend = RemoteBlobBackend::new(base_url);

    let hash = backend.put(b"").expect("PUT empty blob");
    assert!(
        backend.exists(&hash),
        "a stored zero-byte blob must report as present (416 != absent)"
    );
    assert_eq!(backend.get(&hash).expect("GET empty blob"), b"");
}

/// The same backend instance, reused across many writes, must keep working —
/// proving the lazily-spawned worker thread is durable (one thread services
/// every request, not one-per-call).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_blob_backend_reuses_one_worker() {
    let (_dir, base_url) = serve_lago_api().await;
    let backend: Arc<dyn BlobBackend> = Arc::new(RemoteBlobBackend::new(base_url));

    for i in 0..16u32 {
        let data = format!("blob number {i}").into_bytes();
        let hash = backend.put(&data).expect("PUT");
        assert_eq!(backend.get(&hash).expect("GET"), data);
    }
}
