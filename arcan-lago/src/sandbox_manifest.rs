//! Lago-backed sandbox filesystem manifest.
//!
//! Tracks every file written into a sandbox as a content-addressed entry in
//! the Lago [`BlobStore`], with metadata stored in an in-memory
//! [`SandboxManifest`].
//!
//! # Architecture
//!
//! ```text
//! JournaledSandboxProvider
//!     │ FileWritten event (path, size, sha256, mode)
//!     ▼
//! LagoSandboxEventSink (background task)
//!     │ provider.read_file(path)   → raw bytes
//!     │ BlobStore.put(bytes)       → blob_hash
//!     │ SandboxManifest.upsert()   → in-memory index
//!     ▼
//! Lago blob store (content-addressed, provider-independent)
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use arcan_sandbox::{SandboxId, SandboxProvider};
use chrono::{DateTime, Utc};
use lago_core::BlobHash;
use lago_store::BlobStore;
use tracing::{debug, warn};
use uuid::Uuid;

// ── FileManifestEntry ─────────────────────────────────────────────────────────

/// A single file-write record in the Lago sandbox manifest.
#[derive(Debug, Clone)]
pub struct FileManifestEntry {
    /// Stable record identifier.
    pub id: Uuid,
    /// Sandbox the file belongs to.
    pub sandbox_id: SandboxId,
    /// Agent session during which the file was written.
    pub session_id: String,
    /// Absolute path inside the sandbox.
    pub path: String,
    /// Content size in bytes.
    pub size_bytes: u64,
    /// SHA-256 hex hash of the file content at write time.
    ///
    /// If a Lago blob was successfully stored, this equals `blob_hash.as_str()`.
    /// If the provider's `read_file` was unavailable, this is still the hash
    /// computed by `JournaledSandboxProvider` — the blob may not be in the store.
    pub sha256: String,
    /// Content-addressed key in the Lago [`BlobStore`], when available.
    ///
    /// `None` when `read_file` was not supported or failed.
    pub blob_hash: Option<BlobHash>,
    /// Unix permission bits.
    pub mode: u32,
    /// Wall-clock time when the file was written.
    pub written_at: DateTime<Utc>,
    /// Soft-delete flag.
    pub deleted: bool,
    /// Name of the provider that held the file at write time.
    pub provider_at_write: String,
}

// ── SandboxManifest ───────────────────────────────────────────────────────────

/// In-memory index of all files written across all sandboxes in this process.
///
/// Keyed by `(sandbox_id_string, path)` so lookups are O(1). The manifest is
/// eventually consistent — entries are inserted from the background event task.
///
/// For persistence across process restarts, the manifest should be rebuilt from
/// the Lago journal on startup (not yet implemented — BRO-258 follow-up).
#[derive(Debug, Default)]
pub struct SandboxManifest {
    /// Primary index: (sandbox_id, path) → latest entry for that file.
    entries: HashMap<(String, String), FileManifestEntry>,
}

impl SandboxManifest {
    /// Create a new, empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the manifest entry for `(sandbox_id, path)`.
    pub fn upsert(&mut self, entry: FileManifestEntry) {
        let key = (entry.sandbox_id.0.clone(), entry.path.clone());
        self.entries.insert(key, entry);
    }

    /// Look up the latest entry for a file in a given sandbox.
    pub fn get(&self, sandbox_id: &SandboxId, path: &str) -> Option<&FileManifestEntry> {
        self.entries.get(&(sandbox_id.0.clone(), path.to_owned()))
    }

    /// Return all entries for a given sandbox, sorted by path.
    pub fn list_sandbox(&self, sandbox_id: &SandboxId) -> Vec<&FileManifestEntry> {
        let sid = sandbox_id.0.as_str();
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .filter(|((s, _), _)| s == sid)
            .map(|(_, v)| v)
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// Total number of manifest entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the manifest has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── sync_file_written ─────────────────────────────────────────────────────────

/// Parameters for [`sync_file_written`].
pub struct FileWrittenParams<'a> {
    pub sandbox_id: &'a SandboxId,
    pub session_id: &'a str,
    pub path: &'a str,
    pub size_bytes: u64,
    pub sha256: &'a str,
    pub mode: u32,
    pub provider: &'a Arc<dyn SandboxProvider>,
    pub blob_store: &'a Arc<BlobStore>,
    pub provider_name: &'a str,
}

/// Persist a single `FileWritten` event to the Lago blob store and return a
/// [`FileManifestEntry`].
///
/// Attempts to call `provider.read_file()` to fetch the current content.
/// If the provider does not support `read_file` (returns `NotSupported`) the
/// entry is still recorded with `blob_hash: None` — the manifest remains
/// eventually consistent using the SHA-256 already computed by
/// [`JournaledSandboxProvider`].
pub async fn sync_file_written(p: FileWrittenParams<'_>) -> FileManifestEntry {
    let blob_hash = match p.provider.read_file(p.sandbox_id, p.path).await {
        Ok(content) => match p.blob_store.put(&content) {
            Ok(hash) => {
                debug!(
                    sandbox_id = %p.sandbox_id, path = p.path, ?hash,
                    "file synced to Lago blob store"
                );
                Some(hash)
            }
            Err(e) => {
                warn!(sandbox_id = %p.sandbox_id, path = p.path, error = %e, "blob store put failed");
                None
            }
        },
        Err(e) => {
            // Provider may not support read_file (NotSupported is expected for
            // bubblewrap/local providers). Record manifest entry without blob.
            debug!(
                sandbox_id = %p.sandbox_id, path = p.path, error = %e,
                "read_file unavailable, manifest entry recorded without blob"
            );
            None
        }
    };

    FileManifestEntry {
        id: Uuid::new_v4(),
        sandbox_id: p.sandbox_id.clone(),
        session_id: p.session_id.to_owned(),
        path: p.path.to_owned(),
        size_bytes: p.size_bytes,
        sha256: p.sha256.to_owned(),
        blob_hash,
        mode: p.mode,
        written_at: Utc::now(),
        deleted: false,
        provider_at_write: p.provider_name.to_owned(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(sandbox_id: &str, path: &str) -> FileManifestEntry {
        FileManifestEntry {
            id: Uuid::new_v4(),
            sandbox_id: SandboxId(sandbox_id.to_owned()),
            session_id: "sess-1".to_owned(),
            path: path.to_owned(),
            size_bytes: 100,
            sha256: "abc123".to_owned(),
            blob_hash: None,
            mode: 0o644,
            written_at: Utc::now(),
            deleted: false,
            provider_at_write: "stub".to_owned(),
        }
    }

    #[test]
    fn upsert_and_get() {
        let mut m = SandboxManifest::new();
        let entry = make_entry("box-1", "/workspace/main.py");
        m.upsert(entry);
        let got = m.get(&SandboxId("box-1".into()), "/workspace/main.py");
        assert!(got.is_some());
        assert_eq!(got.unwrap().size_bytes, 100);
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut m = SandboxManifest::new();
        let e1 = make_entry("box-1", "/file.txt");
        let mut e2 = make_entry("box-1", "/file.txt");
        e2.size_bytes = 999;
        m.upsert(e1);
        m.upsert(e2);
        assert_eq!(
            m.get(&SandboxId("box-1".into()), "/file.txt")
                .unwrap()
                .size_bytes,
            999
        );
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn list_sandbox_filters_by_id() {
        let mut m = SandboxManifest::new();
        m.upsert(make_entry("box-A", "/a.py"));
        m.upsert(make_entry("box-A", "/b.py"));
        m.upsert(make_entry("box-B", "/c.py"));

        let a_entries = m.list_sandbox(&SandboxId("box-A".into()));
        assert_eq!(a_entries.len(), 2);
        let b_entries = m.list_sandbox(&SandboxId("box-B".into()));
        assert_eq!(b_entries.len(), 1);
    }

    #[test]
    fn list_sandbox_sorted_by_path() {
        let mut m = SandboxManifest::new();
        m.upsert(make_entry("box-1", "/z.py"));
        m.upsert(make_entry("box-1", "/a.py"));
        m.upsert(make_entry("box-1", "/m.py"));
        let entries = m.list_sandbox(&SandboxId("box-1".into()));
        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["/a.py", "/m.py", "/z.py"]);
    }

    #[tokio::test]
    async fn sync_file_written_records_without_blob_when_read_file_unsupported() {
        use arcan_sandbox::{
            ExecRequest, ExecResult, SandboxCapabilitySet, SandboxHandle, SandboxId, SandboxInfo,
            SandboxSpec, SnapshotId,
        };
        use async_trait::async_trait;

        struct NoReadProvider;

        #[async_trait]
        impl SandboxProvider for NoReadProvider {
            fn name(&self) -> &'static str {
                "stub-no-read"
            }
            fn capabilities(&self) -> SandboxCapabilitySet {
                SandboxCapabilitySet::FILESYSTEM_READ
            }
            async fn create(
                &self,
                _: SandboxSpec,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn resume(
                &self,
                _: &SandboxId,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn run(
                &self,
                _: &SandboxId,
                _: ExecRequest,
            ) -> Result<ExecResult, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn snapshot(
                &self,
                _: &SandboxId,
            ) -> Result<SnapshotId, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn destroy(&self, _: &SandboxId) -> Result<(), arcan_sandbox::SandboxError> {
                Ok(())
            }
            async fn list(&self) -> Result<Vec<SandboxInfo>, arcan_sandbox::SandboxError> {
                Ok(vec![])
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let blob_store = Arc::new(BlobStore::open(dir.path().join("blobs")).unwrap());
        let provider: Arc<dyn SandboxProvider> = Arc::new(NoReadProvider);

        let entry = sync_file_written(FileWrittenParams {
            sandbox_id: &SandboxId("box-1".into()),
            session_id: "sess-1",
            path: "/workspace/main.py",
            size_bytes: 14,
            sha256: "abc123def456",
            mode: 0o644,
            provider: &provider,
            blob_store: &blob_store,
            provider_name: "stub-no-read",
        })
        .await;

        assert_eq!(entry.path, "/workspace/main.py");
        assert_eq!(entry.size_bytes, 14);
        assert_eq!(entry.sha256, "abc123def456");
        assert!(entry.blob_hash.is_none()); // read_file not supported
        assert_eq!(entry.provider_at_write, "stub-no-read");
    }

    #[tokio::test]
    async fn sync_file_written_stores_blob_when_read_file_succeeds() {
        use arcan_sandbox::{
            ExecRequest, ExecResult, SandboxCapabilitySet, SandboxHandle, SandboxId, SandboxInfo,
            SandboxSpec, SnapshotId,
        };
        use async_trait::async_trait;

        struct ReadProvider;

        #[async_trait]
        impl SandboxProvider for ReadProvider {
            fn name(&self) -> &'static str {
                "stub-read"
            }
            fn capabilities(&self) -> SandboxCapabilitySet {
                SandboxCapabilitySet::FILESYSTEM_READ | SandboxCapabilitySet::FILESYSTEM_WRITE
            }
            async fn create(
                &self,
                _: SandboxSpec,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn resume(
                &self,
                _: &SandboxId,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn run(
                &self,
                _: &SandboxId,
                _: ExecRequest,
            ) -> Result<ExecResult, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn snapshot(
                &self,
                _: &SandboxId,
            ) -> Result<SnapshotId, arcan_sandbox::SandboxError> {
                unreachable!("not called in test")
            }
            async fn destroy(&self, _: &SandboxId) -> Result<(), arcan_sandbox::SandboxError> {
                Ok(())
            }
            async fn list(&self) -> Result<Vec<SandboxInfo>, arcan_sandbox::SandboxError> {
                Ok(vec![])
            }
            async fn read_file(
                &self,
                _: &SandboxId,
                _: &str,
            ) -> Result<Vec<u8>, arcan_sandbox::SandboxError> {
                Ok(b"print('hello')".to_vec())
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let blob_store = Arc::new(BlobStore::open(dir.path().join("blobs")).unwrap());
        let provider: Arc<dyn SandboxProvider> = Arc::new(ReadProvider);

        let entry = sync_file_written(FileWrittenParams {
            sandbox_id: &SandboxId("box-1".into()),
            session_id: "sess-1",
            path: "/workspace/main.py",
            size_bytes: 14,
            sha256: "irrelevant-sha",
            mode: 0o644,
            provider: &provider,
            blob_store: &blob_store,
            provider_name: "stub-read",
        })
        .await;

        assert!(entry.blob_hash.is_some());
        // Blob should be retrievable from the store
        let content = blob_store.get(entry.blob_hash.as_ref().unwrap()).unwrap();
        assert_eq!(content, b"print('hello')");
    }
}
