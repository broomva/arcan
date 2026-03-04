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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Filesystem implementation that tracks writes via Lago's [`FsTracker`].
///
/// All read operations delegate to the underlying [`LocalFs`].
/// Write operations first write to disk, then notify the tracker
/// which produces an `EventPayload` sent through the channel.
pub struct LagoTrackedFs {
    local_fs: LocalFs,
    tracker: Arc<FsTracker>,
    tx: mpsc::Sender<EventPayload>,
}

impl LagoTrackedFs {
    /// Create a new tracked filesystem.
    pub fn new(local_fs: LocalFs, tracker: Arc<FsTracker>, tx: mpsc::Sender<EventPayload>) -> Self {
        Self {
            local_fs,
            tracker,
            tx,
        }
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

        // 2. Compute relative path for the tracker
        let resolved = self.local_fs.resolve(path).ok();
        let rel_path = resolved
            .as_ref()
            .and_then(|p| self.local_fs.relative(p))
            .map(|p| format!("/{}", p.display()))
            .unwrap_or_else(|| path.display().to_string());

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
                tracing::warn!(
                    path = %rel_path,
                    "LagoTrackedFs: tracker.track_write failed: {e}"
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
    use lago_store::BlobStore;
    use praxis_core::workspace::FsPolicy;

    fn setup() -> (
        tempfile::TempDir,
        Arc<FsTracker>,
        mpsc::Sender<EventPayload>,
        mpsc::Receiver<EventPayload>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = Arc::new(BlobStore::open(tmp.path().join("blobs")).unwrap());
        let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store));
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
