//! Remote blob backend — implements [`lago_store::BlobBackend`] over the Lago
//! HTTP blob API.
//!
//! Used by arcan when `LAGO_URL` is set so that file *content* (not just the
//! event journal) is durable in the remote Lago daemon. Without it, arcan in
//! remote-journal mode would persist `FileWrite` events to `lagod` while the
//! actual bytes stayed on the local container disk — content that vanishes on
//! redeploy. This closes that gap by mirroring [`RemoteLagoJournal`]'s HTTP
//! client against the blob routes.
//!
//! # Endpoints used
//!
//! | Method | Path | Purpose |
//! |--------|------|---------|
//! | `PUT`  | `/v1/blobs/{hash}` | Store a blob (server verifies the hash) |
//! | `GET`  | `/v1/blobs/{hash}` | Retrieve a blob's bytes |
//! | `GET`  | `/v1/blobs/{hash}` (Range: bytes=0-0) | Cheap existence probe |
//!
//! # Sync surface over async transport — and why this is an OS thread, not
//! a `Runtime::block_on` on the caller
//!
//! [`lago_store::BlobBackend`] is synchronous (see its docs for why), but HTTP
//! is async. The naive bridge — own a runtime and call `runtime.block_on()`
//! inline — is a nested-runtime abort waiting to happen: `block_on` from
//! within an ambient runtime panics with "Cannot start a runtime from within
//! a runtime", and under the release profile's `panic = "abort"` that takes
//! the whole process down. Historically the agent's tool harness
//! (`arcan_aios_adapters::ArcanHarnessAdapter::execute`) invoked the sync
//! `Tool::execute` chain *directly on a Tokio worker thread*, so an inline
//! `block_on` inside a blob `put` aborted in production. As of BRO-1483 the
//! harness runs sync `Tool::execute` on the blocking pool (`spawn_blocking`),
//! which defuses that specific path — but a bridge must not *rely* on its
//! caller sitting on a blocking thread.
//!
//! [`RemoteBlobBackend`] instead spawns a **dedicated, long-lived OS thread**
//! that owns the runtime + `reqwest::Client` and services requests off an
//! `mpsc` channel. `block_on` runs only on that thread — which never carries
//! an ambient runtime — so it is panic-free from *any* caller regardless of
//! the harness's threading: a kernel worker executing the sync tool chain, a
//! current-thread CLI runtime, or a plain test thread. Each trait call sends a
//! request and parks the caller on a reply channel; parking a worker on a
//! channel recv is the same tradeoff the local backend already makes blocking
//! the worker on disk I/O — it stalls that worker but never nests a runtime.
//! The worker thread + client are created lazily on first use.

use std::sync::OnceLock;
use std::sync::mpsc;

use lago_core::id::BlobHash;
use lago_core::{LagoError, LagoResult};
use reqwest::StatusCode;

/// Maximum blob size accepted by the remote `PUT /v1/blobs/{hash}` route
/// (mirrors `lago_api::routes::blobs::MAX_BLOB_SIZE`). Enforced client-side so
/// oversized writes fail fast with a clear error instead of buffering the
/// whole body only to be rejected by the server.
const MAX_BLOB_SIZE: usize = 512 * 1024 * 1024;

/// One unit of work for the backend's dedicated worker thread. Each variant
/// carries a one-shot reply sender; the calling trait method blocks on the
/// matching receiver.
enum BlobOp {
    Put(Vec<u8>, mpsc::Sender<LagoResult<BlobHash>>),
    Get(BlobHash, mpsc::Sender<LagoResult<Vec<u8>>>),
    Exists(BlobHash, mpsc::Sender<bool>),
}

/// A [`lago_store::BlobBackend`] that stores blob content in a remote Lago
/// daemon over HTTP.
///
/// Auth mirrors [`RemoteLagoJournal`](crate::RemoteLagoJournal): no bearer
/// token is attached. In the in-container loopback deployment, `lagod` runs
/// with auth disabled and the `/v1/blobs/*` routes sit outside the JWT layer
/// (only `/v1/memory/*` is auth-gated), so unauthenticated PUT/GET succeed.
/// If a future deployment puts these routes behind auth, this client will need
/// the same token plumbing the journal client would.
pub struct RemoteBlobBackend {
    base_url: String,
    /// Lazily-spawned sender to the dedicated worker thread (which owns the
    /// runtime + client). `None` until the first request.
    tx: OnceLock<mpsc::Sender<BlobOp>>,
}

impl RemoteBlobBackend {
    /// Create a backend pointing at `base_url`
    /// (e.g. `http://lagod.railway.internal:3001`).
    ///
    /// Neither the worker thread nor the HTTP client is created here — both are
    /// deferred to the first request so construction is cheap and never touches
    /// a reactor.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            tx: OnceLock::new(),
        }
    }

    /// Lazily spawn (once) the dedicated worker thread and return a sender to
    /// it. The thread owns a current-thread runtime + `reqwest::Client` and
    /// loops over the request channel until all senders drop.
    fn sender(&self) -> &mpsc::Sender<BlobOp> {
        self.tx.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<BlobOp>();
            let base_url = self.base_url.clone();
            // If the OS refuses the thread we have no way to serve requests;
            // surface it loudly. In practice thread spawn does not fail.
            std::thread::Builder::new()
                .name("lago-remote-blob".to_owned())
                .spawn(move || worker_loop(base_url, rx))
                .expect("spawn lago-remote-blob worker thread");
            tx
        })
    }

    /// Send an op to the worker and block on its reply. A dropped reply channel
    /// (worker thread died, e.g. its runtime failed to build) maps to a clear
    /// store error rather than a panic.
    fn send_recv<T>(
        &self,
        make: impl FnOnce(mpsc::Sender<T>) -> BlobOp,
        on_dead: impl FnOnce() -> T,
    ) -> T {
        let (reply_tx, reply_rx) = mpsc::channel::<T>();
        if self.sender().send(make(reply_tx)).is_err() {
            return on_dead();
        }
        reply_rx.recv().unwrap_or_else(|_| on_dead())
    }
}

impl lago_store::BlobBackend for RemoteBlobBackend {
    fn put(&self, data: &[u8]) -> LagoResult<BlobHash> {
        if data.len() > MAX_BLOB_SIZE {
            return Err(LagoError::Store(format!(
                "blob too large: {} bytes (max {MAX_BLOB_SIZE})",
                data.len()
            )));
        }
        let bytes = data.to_vec();
        self.send_recv(
            |reply| BlobOp::Put(bytes, reply),
            || {
                Err(LagoError::Store(
                    "remote blob worker unavailable".to_owned(),
                ))
            },
        )
    }

    fn get(&self, hash: &BlobHash) -> LagoResult<Vec<u8>> {
        let hash = hash.clone();
        self.send_recv(
            |reply| BlobOp::Get(hash, reply),
            || {
                Err(LagoError::Store(
                    "remote blob worker unavailable".to_owned(),
                ))
            },
        )
    }

    fn exists(&self, hash: &BlobHash) -> bool {
        let hash = hash.clone();
        // A dead worker can't confirm existence — report absent (consistent
        // with treating any transport failure as "does not exist").
        self.send_recv(|reply| BlobOp::Exists(hash, reply), || false)
    }
}

/// The dedicated worker thread's body. Builds the runtime + client ONCE on a
/// thread that carries no ambient Tokio context (so `block_on` is always
/// legal), then services requests until the channel closes.
fn worker_loop(base_url: String, rx: mpsc::Receiver<BlobOp>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            // Drop `rx`: pending and future sends fail, and `send_recv` maps
            // the dropped reply channel to a clear error for every caller.
            tracing::error!(error = %e, "remote blob worker: runtime init failed; backend disabled");
            return;
        }
    };
    // Bounded timeouts are load-bearing here: every blob op funnels through
    // this single worker, and each caller is a Tokio worker thread parked on
    // the reply channel for the full round-trip. Without a timeout, a hung
    // lagod would park callers — and transitively stall file writes across all
    // sessions — unboundedly. Fail fast instead.
    let client = runtime.block_on(async {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    });

    while let Ok(op) = rx.recv() {
        match op {
            BlobOp::Put(data, reply) => {
                let hash = lago_store::hash_bytes(&data);
                let url = blob_url(&base_url, &hash);
                let result = runtime.block_on(do_put(&client, &url, data)).map(|()| hash);
                let _ = reply.send(result);
            }
            BlobOp::Get(hash, reply) => {
                let url = blob_url(&base_url, &hash);
                let result = runtime.block_on(do_get(&client, &url, &hash));
                let _ = reply.send(result);
            }
            BlobOp::Exists(hash, reply) => {
                let url = blob_url(&base_url, &hash);
                let result = runtime.block_on(do_exists(&client, &url));
                let _ = reply.send(result);
            }
        }
    }
}

fn blob_url(base_url: &str, hash: &BlobHash) -> String {
    format!("{}/v1/blobs/{}", base_url, hash.as_str())
}

async fn do_put(client: &reqwest::Client, url: &str, body: Vec<u8>) -> LagoResult<()> {
    let resp = client
        .put(url)
        .body(body)
        .send()
        .await
        .map_err(|e| LagoError::Store(format!("blob PUT failed: {e}")))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let detail = resp.text().await.unwrap_or_default();
    Err(LagoError::Store(format!(
        "blob PUT {url} -> HTTP {status}: {detail}"
    )))
}

async fn do_get(client: &reqwest::Client, url: &str, hash: &BlobHash) -> LagoResult<Vec<u8>> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| LagoError::Store(format!("blob GET failed: {e}")))?;
    let status = resp.status();
    if status == StatusCode::NOT_FOUND {
        return Err(LagoError::BlobNotFound(hash.as_str().to_string()));
    }
    if !status.is_success() {
        let detail = resp.text().await.unwrap_or_default();
        return Err(LagoError::Store(format!(
            "blob GET {url} -> HTTP {status}: {detail}"
        )));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| LagoError::Store(format!("blob GET body read failed: {e}")))?;
    Ok(bytes.to_vec())
}

async fn do_exists(client: &reqwest::Client, url: &str) -> bool {
    // Probe with a 1-byte Range request so existence costs ~1 byte instead of
    // downloading the whole blob. Status mapping:
    //   * 2xx (206 Partial / 200) → present
    //   * 416 Range Not Satisfiable → present-but-empty: a stored ZERO-byte
    //     blob has no satisfiable `bytes=0-0` range, so the server returns 416.
    //     Agents write empty files routinely (touch, empty __init__.py), so
    //     treating 416 as absent would report stored empty blobs as missing.
    //   * 404 → absent
    //   * anything else / transport error → treat as "does not exist" (the
    //     trait can't surface uncertainty; a re-PUT is a content-addressed
    //     no-op, so a false "absent" is safe, a false "present" is not).
    match client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            status.is_success() || status == StatusCode::RANGE_NOT_SATISFIABLE
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_spawn_worker() {
        // Construction must be cheap and reactor-free — no worker until a
        // request actually arrives.
        let backend = RemoteBlobBackend::new("http://localhost:9999");
        assert_eq!(backend.base_url, "http://localhost:9999");
        assert!(backend.tx.get().is_none());
    }

    #[test]
    fn trailing_slash_is_trimmed() {
        let backend = RemoteBlobBackend::new("http://localhost:9999/");
        assert_eq!(backend.base_url, "http://localhost:9999");
    }

    #[test]
    fn url_is_built_under_v1_blobs() {
        let hash = BlobHash::from_hex("abc123");
        assert_eq!(
            blob_url("http://localhost:9999", &hash),
            "http://localhost:9999/v1/blobs/abc123"
        );
    }

    // Round-trip (PUT/GET/exists) against a live lago-api router lives in
    // tests/remote_blob_roundtrip.rs, which needs a bound socket for reqwest —
    // and critically also asserts the backend works when CALLED FROM WITHIN a
    // multi-threaded Tokio runtime (the production call context), proving the
    // dedicated-worker-thread bridge does not trip the nested-runtime panic.
}
