//! Lago-backed tracked filesystem: intercepts writes for O(1) event emission.
//!
//! [`LagoTrackedFs`] implements [`FsPort`] by delegating all reads to
//! a [`LocalFs`] and intercepting writes to produce `EventPayload::FileWrite`
//! events via a [`FsTracker`]. Events are sent through an mpsc channel
//! to a background writer that persists them to the Lago journal.

use lago_core::event::EventPayload;
use lago_core::{BranchId, EventEnvelope, EventId, Journal, SessionId};
use lago_fs::FsTracker;
use praxis_core::error::PraxisResult;
use praxis_core::fs_port::{FsDirEntry, FsMetadata, FsPort};
use praxis_core::local_fs::LocalFs;
use praxis_core::workspace::FsPolicy;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Filesystem implementation that tracks writes via Lago's [`FsTracker`].
///
/// All read operations delegate to the underlying [`LocalFs`].
/// Write operations first write to disk, then notify the tracker
/// which produces an `EventPayload` sent through the channel.
///
/// ## Per-session scoping (BRO-1491)
///
/// A single tracker (hence a single shared manifest) serves every session, so
/// manifest keys MUST be session-unique or concurrent sessions overwrite each
/// other's entries. The [`FsPort::scoped`] impl rebases the boundary policy at
/// the per-session workspace root (isolation) while computing manifest keys
/// relative to a shared [`Self::manifest_root`] — the parent of the per-session
/// directories (`{data_dir}`) — so a write to
/// `{data_dir}/sessions/<id>/artifacts/x` is keyed `/sessions/<id>/artifacts/x`.
///
/// Deliberately NOT rooted at `{data_dir}` directly: that would (a) let the
/// boundary policy see across sessions, and (b) drag the redb journal / blob
/// store under the exec-path reconciler's walk. Isolation comes from the
/// per-session *boundary*; uniqueness comes from the shared *key root*.
pub struct LagoTrackedFs {
    local_fs: LocalFs,
    tracker: Arc<FsTracker>,
    tx: mpsc::Sender<EventPayload>,
    /// Base against which manifest keys are computed. Defaults to the local
    /// FS root (boot behavior); scoped instances key relative to
    /// [`Self::session_base`] to stay session-unique.
    manifest_root: PathBuf,
    /// Parent of the per-session workspaces (`{data_dir}`). When set,
    /// [`FsPort::scoped`] keys session writes relative to it. `None` ⇒ scoped
    /// instances key relative to the session root itself (degraded: not
    /// session-unique — only used when no session base was configured).
    session_base: Option<PathBuf>,
}

impl LagoTrackedFs {
    /// Create a new tracked filesystem. Manifest keys are computed relative to
    /// the local FS root — the boot workspace — matching the tracker baseline.
    pub fn new(local_fs: LocalFs, tracker: Arc<FsTracker>, tx: mpsc::Sender<EventPayload>) -> Self {
        let manifest_root = local_fs.workspace_root().to_path_buf();
        Self {
            local_fs,
            tracker,
            tx,
            manifest_root,
            session_base: None,
        }
    }

    /// Declare the parent directory of per-session workspaces (`{data_dir}`).
    ///
    /// When set, [`FsPort::scoped`] keys session writes relative to this base
    /// (`/sessions/<id>/…`) so the shared manifest stays session-unique
    /// (BRO-1491). Without it, scoped instances fall back to keying relative to
    /// the session root, which is not unique across sessions.
    pub fn with_session_base(mut self, base: impl Into<PathBuf>) -> Self {
        self.session_base = Some(base.into());
        self
    }

    /// Compute the manifest key for a just-written path: `/` + the path
    /// (canonicalized) made relative to [`Self::manifest_root`]. Falls back to
    /// the raw path display when the file is outside the manifest root (should
    /// not happen for boundary-checked writes, but keeps a stable key).
    fn manifest_key(&self, path: &Path) -> String {
        // The file exists at this point (write already happened), so `resolve`
        // can canonicalize it.
        let resolved = self.local_fs.resolve(path).ok();
        resolved
            .as_ref()
            .and_then(|abs| self.rel_to_manifest_root(abs))
            .map(|rel| format!("/{}", rel.display()))
            .unwrap_or_else(|| path.display().to_string())
    }

    /// Canonicalize `manifest_root` and strip it from an absolute path.
    fn rel_to_manifest_root(&self, absolute: &Path) -> Option<PathBuf> {
        let root = self.manifest_root.canonicalize().ok()?;
        absolute.strip_prefix(&root).ok().map(Path::to_path_buf)
    }
}

impl FsPort for LagoTrackedFs {
    fn workspace_root(&self) -> &Path {
        self.local_fs.workspace_root()
    }

    fn resolve(&self, path: &Path) -> PraxisResult<PathBuf> {
        self.local_fs.resolve(path)
    }

    fn resolve_for_write(&self, path: &Path) -> PraxisResult<PathBuf> {
        self.local_fs.resolve_for_write(path)
    }

    fn read_to_string(&self, path: &Path) -> PraxisResult<String> {
        self.local_fs.read_to_string(path)
    }

    fn read_bytes(&self, path: &Path) -> PraxisResult<Vec<u8>> {
        self.local_fs.read_bytes(path)
    }

    fn write(&self, path: &Path, content: &[u8]) -> PraxisResult<()> {
        // 1. Write to disk via LocalFs
        self.local_fs.write(path, content)?;

        // 2. Compute the session-unique manifest key (relative to manifest_root)
        let rel_path = self.manifest_key(path);

        // 3. Track the write (stores blob, updates manifest, returns event)
        match self.tracker.track_write(&rel_path, content, None) {
            Ok(payload) => {
                // 4. Send event — non-blocking, log warning if channel is full
                if let Err(e) = self.tx.try_send(payload) {
                    tracing::warn!(
                        path = %rel_path,
                        "LagoTrackedFs: event channel full or closed, write event dropped: {e}"
                    );
                }
            }
            Err(e) => {
                // track_write stores the content in the blob backend before
                // building the event. A failure here means the content was NOT
                // durably stored — with a remote backend this is a real
                // durability loss (network error, or the server rejecting the
                // content), distinct from the benign full-channel event drop
                // above. Surface it at error level so it is not lost in noise.
                // The disk write already succeeded, so the tool still reports
                // success; only lago's record of the content is incomplete.
                tracing::error!(
                    path = %rel_path,
                    error = %e,
                    "LagoTrackedFs: content NOT tracked in lago (blob store/manifest \
                     write failed) — file is on local disk but its content may not be \
                     durable"
                );
            }
        }

        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.local_fs.exists(path)
    }

    fn metadata(&self, path: &Path) -> PraxisResult<FsMetadata> {
        self.local_fs.metadata(path)
    }

    fn read_dir(&self, path: &Path) -> PraxisResult<Vec<FsDirEntry>> {
        self.local_fs.read_dir(path)
    }

    fn create_dir_all(&self, path: &Path) -> PraxisResult<()> {
        self.local_fs.create_dir_all(path)
    }

    fn relative(&self, absolute_path: &Path) -> Option<PathBuf> {
        self.local_fs.relative(absolute_path)
    }

    fn scoped(&self, root: &Path) -> Option<Arc<dyn FsPort>> {
        // Boundary policy rebased at the per-session root → isolation.
        let scoped_local = LocalFs::new(FsPolicy::new(root));
        // Keep manifest keys session-unique by keying relative to the shared
        // session base (`{data_dir}` → `/sessions/<id>/…`). If no base was
        // configured, fall back to the session root (degraded: `/artifacts/…`,
        // not unique across sessions — surfaced via with_session_base at boot).
        let manifest_root = self
            .session_base
            .clone()
            .unwrap_or_else(|| root.to_path_buf());
        Some(Arc::new(LagoTrackedFs {
            local_fs: scoped_local,
            tracker: self.tracker.clone(),
            tx: self.tx.clone(),
            manifest_root,
            session_base: self.session_base.clone(),
        }))
    }
}

/// Background event writer: consumes event payloads from the channel
/// and appends them to the Lago journal as `EventEnvelope`s.
pub async fn run_event_writer(
    mut rx: mpsc::Receiver<EventPayload>,
    journal: Arc<dyn Journal>,
    session_id: SessionId,
    branch_id: BranchId,
) {
    while let Some(payload) = rx.recv().await {
        let envelope = EventEnvelope {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            branch_id: branch_id.clone(),
            run_id: None,
            seq: 0,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload,
            metadata: std::collections::HashMap::new(),
            schema_version: 1,
        };

        if let Err(e) = journal.append(envelope).await {
            tracing::warn!(%e, "LagoTrackedFs event writer: failed to append event");
        }
    }

    tracing::debug!("LagoTrackedFs event writer: channel closed, shutting down");
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::event::EventPayload;
    use lago_fs::Manifest;
    use lago_store::{BlobStore, LocalBlobBackend};
    use praxis_core::workspace::FsPolicy;

    fn setup() -> (
        tempfile::TempDir,
        Arc<FsTracker>,
        mpsc::Sender<EventPayload>,
        mpsc::Receiver<EventPayload>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = Arc::new(BlobStore::open(tmp.path().join("blobs")).unwrap());
        let tracker = Arc::new(FsTracker::new(
            Manifest::new(),
            Arc::new(LocalBlobBackend::new(blob_store)),
        ));
        let (tx, rx) = mpsc::channel(100);
        (tmp, tracker, tx, rx)
    }

    fn make_tracked_fs(
        tmp: &tempfile::TempDir,
        tracker: Arc<FsTracker>,
        tx: mpsc::Sender<EventPayload>,
    ) -> LagoTrackedFs {
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let local_fs = LocalFs::new(FsPolicy::new(&ws));
        LagoTrackedFs::new(local_fs, tracker, tx)
    }

    #[test]
    fn write_sends_notification() {
        let (tmp, tracker, tx, mut rx) = setup();
        let fs = make_tracked_fs(&tmp, tracker, tx);
        let ws = tmp.path().join("ws");

        let file = ws.join("test.txt");
        fs.write(&file, b"hello").unwrap();

        // Should have received exactly one event
        let payload = rx.try_recv().unwrap();
        match payload {
            EventPayload::FileWrite {
                path, size_bytes, ..
            } => {
                assert!(path.contains("test.txt"));
                assert_eq!(size_bytes, 5);
            }
            _ => panic!("expected FileWrite"),
        }
    }

    #[test]
    fn reads_are_not_tracked() {
        let (tmp, tracker, tx, mut rx) = setup();
        let fs = make_tracked_fs(&tmp, tracker, tx);
        let ws = tmp.path().join("ws");

        std::fs::write(ws.join("read_me.txt"), "data").unwrap();
        let _content = fs.read_to_string(&ws.join("read_me.txt")).unwrap();

        // No events should be sent for reads
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn channel_full_does_not_block_write() {
        let (tmp, tracker, _, _rx_dropped) = setup();
        // Create a channel with capacity 1, fill it, then try to write
        let (tx, _rx) = mpsc::channel(1);
        let fs = make_tracked_fs(&tmp, tracker, tx.clone());
        let ws = tmp.path().join("ws");

        // Fill the channel
        let _ = tx.try_send(EventPayload::FileDelete {
            path: "/filler".into(),
        });

        // This write should succeed even though the channel is full
        let file = ws.join("overflow.txt");
        fs.write(&file, b"still works").unwrap();

        // File should exist on disk
        assert!(file.exists());
    }

    #[test]
    fn tracker_manifest_updated_on_write() {
        let (tmp, tracker, tx, _rx) = setup();
        let fs = make_tracked_fs(&tmp, tracker.clone(), tx);
        let ws = tmp.path().join("ws");

        fs.write(&ws.join("tracked.txt"), b"content").unwrap();

        let manifest = tracker.manifest();
        assert!(
            manifest
                .entries()
                .values()
                .any(|e| e.path.contains("tracked.txt"))
        );
    }

    #[test]
    fn multiple_writes_produce_multiple_events() {
        let (tmp, tracker, tx, mut rx) = setup();
        let fs = make_tracked_fs(&tmp, tracker, tx);
        let ws = tmp.path().join("ws");

        fs.write(&ws.join("a.txt"), b"aaa").unwrap();
        fs.write(&ws.join("b.txt"), b"bbb").unwrap();
        fs.write(&ws.join("c.txt"), b"ccc").unwrap();

        let mut count = 0;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    // ── Per-session scoping (BRO-1491) ──────────────────────────────────

    /// Build a boot tracked FS whose session base is `data_dir` (parent of the
    /// per-session workspaces), mirroring the `arcan serve` wiring.
    fn boot_tracked_fs(
        data_dir: &std::path::Path,
        tracker: Arc<FsTracker>,
        tx: mpsc::Sender<EventPayload>,
    ) -> LagoTrackedFs {
        // Boot workspace is the data dir itself here; production points it at
        // the --workspace dir, but the session base is what scoping uses.
        let local_fs = LocalFs::new(FsPolicy::new(data_dir));
        LagoTrackedFs::new(local_fs, tracker, tx).with_session_base(data_dir)
    }

    #[test]
    fn scoped_sessions_get_session_unique_manifest_keys() {
        let (tmp, tracker, tx, mut rx) = setup();
        let data_dir = tmp.path().join("data");
        let sess_a = data_dir.join("sessions/a");
        let sess_b = data_dir.join("sessions/b");
        // The kernel's initialize_workspace creates these before dispatch.
        std::fs::create_dir_all(sess_a.join("artifacts")).unwrap();
        std::fs::create_dir_all(sess_b.join("artifacts")).unwrap();

        let boot = boot_tracked_fs(&data_dir, tracker.clone(), tx);
        let fs_a = boot.scoped(&sess_a).unwrap();
        let fs_b = boot.scoped(&sess_b).unwrap();

        fs_a.write(Path::new("artifacts/receipt.txt"), b"from A")
            .unwrap();
        fs_b.write(Path::new("artifacts/receipt.txt"), b"from B")
            .unwrap();

        // (1) Each landed in its own session workspace on disk.
        assert_eq!(
            std::fs::read_to_string(sess_a.join("artifacts/receipt.txt")).unwrap(),
            "from A"
        );
        assert_eq!(
            std::fs::read_to_string(sess_b.join("artifacts/receipt.txt")).unwrap(),
            "from B"
        );

        // (2) The shared manifest carries two DISTINCT, session-unique keys.
        let manifest = tracker.manifest();
        assert!(manifest.exists("/sessions/a/artifacts/receipt.txt"));
        assert!(manifest.exists("/sessions/b/artifacts/receipt.txt"));

        // (3) Two distinct FileWrite events with session-unique paths.
        let mut paths = Vec::new();
        while let Ok(EventPayload::FileWrite { path, .. }) = rx.try_recv() {
            paths.push(path);
        }
        assert!(paths.contains(&"/sessions/a/artifacts/receipt.txt".to_string()));
        assert!(paths.contains(&"/sessions/b/artifacts/receipt.txt".to_string()));
    }

    #[test]
    fn scoped_session_cannot_read_another_session() {
        let (tmp, tracker, tx, _rx) = setup();
        let data_dir = tmp.path().join("data");
        let sess_a = data_dir.join("sessions/a");
        let sess_b = data_dir.join("sessions/b");
        std::fs::create_dir_all(&sess_a).unwrap();
        std::fs::create_dir_all(&sess_b).unwrap();
        std::fs::write(sess_b.join("secret.txt"), "B's secret").unwrap();

        let boot = boot_tracked_fs(&data_dir, tracker, tx);
        let fs_a = boot.scoped(&sess_a).unwrap();

        // A traversal out of session A into session B is rejected.
        let err = fs_a
            .read_to_string(Path::new("../b/secret.txt"))
            .unwrap_err();
        assert!(matches!(
            err,
            praxis_core::error::PraxisError::PathOutsideWorkspace { .. }
        ));
    }

    #[test]
    fn boot_write_keys_are_unchanged_without_scoping() {
        // Backward-compat: the un-scoped boot FS keys relative to its own root,
        // exactly as before manifest_root existed.
        let (tmp, tracker, tx, mut rx) = setup();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let fs = LagoTrackedFs::new(LocalFs::new(FsPolicy::new(&ws)), tracker.clone(), tx);

        fs.write(&ws.join("top.txt"), b"x").unwrap();

        assert!(tracker.manifest().exists("/top.txt"));
        assert!(matches!(
            rx.try_recv().unwrap(),
            EventPayload::FileWrite { path, .. } if path == "/top.txt"
        ));
    }

    #[test]
    fn read_operations_delegate_to_local_fs() {
        let (tmp, tracker, tx, _rx) = setup();
        let fs = make_tracked_fs(&tmp, tracker, tx);
        let ws = tmp.path().join("ws");

        std::fs::write(ws.join("hello.txt"), "world").unwrap();

        let content = fs.read_to_string(&ws.join("hello.txt")).unwrap();
        assert_eq!(content, "world");

        let bytes = fs.read_bytes(&ws.join("hello.txt")).unwrap();
        assert_eq!(bytes, b"world");

        assert!(fs.exists(&ws.join("hello.txt")));

        let meta = fs.metadata(&ws.join("hello.txt")).unwrap();
        assert!(meta.is_file);
        assert_eq!(meta.size_bytes, 5);
    }
}
