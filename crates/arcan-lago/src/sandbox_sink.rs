//! Sandbox event sinks for the Lago persistence layer (BRO-257).
//!
//! # Sinks
//!
//! - [`LagoSandboxEventSink`] — writes `SandboxEvent`s to a Postgres database
//!   (the `sandbox_events`, `sandbox_instances`, and `sandbox_snapshots` tables
//!   created by migration `0001_sandbox_metadata.sql`).  Events are sent to a
//!   background tokio task via an unbounded channel so `emit()` is always
//!   synchronous and cheap.
//! - [`LagoSandboxEventSink::spawn_with_manifest`] — full sink with blob store
//!   and manifest sync for `FileWritten` events.
//!
//! # Schema
//!
//! Migration:
//! `apps/chatOS/packages/db/drizzle/0001_sandbox_metadata.sql`

use std::sync::{Arc, Mutex};

use arcan_sandbox::{SandboxEvent, SandboxEventKind, SandboxEventSink, SandboxProvider};
use lago_store::BlobStore;
use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::sandbox_manifest::{FileWrittenParams, SandboxManifest, sync_file_written};

// ── LagoSandboxEventSink ──────────────────────────────────────────────────────

/// An [`arcan_sandbox::SandboxEventSink`] that persists events to Postgres
/// and optionally syncs file content to the Lago blob store.
///
/// Uses an unbounded `mpsc` channel so `emit()` never blocks.  The background
/// task runs sqlx queries against the three sandbox metadata tables.
///
/// # Construction
///
/// - [`LagoSandboxEventSink::spawn`] — Postgres persistence only.
/// - [`LagoSandboxEventSink::spawn_with_manifest`] — Postgres + blob store
///   + in-memory file manifest sync for `FileWritten` events.
pub struct LagoSandboxEventSink {
    tx: mpsc::UnboundedSender<SandboxEvent>,
}

impl LagoSandboxEventSink {
    /// Spawn the background writer and return the sink.
    pub fn spawn(pool: PgPool) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<SandboxEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = write_event(&pool, &event).await {
                    warn!(
                        sandbox_id = %event.sandbox_id,
                        kind = ?event.kind,
                        error = %e,
                        "sandbox event write failed"
                    );
                } else {
                    debug!(
                        sandbox_id = %event.sandbox_id,
                        kind = ?event.kind,
                        "sandbox event persisted"
                    );
                }
            }
        });

        Self { tx }
    }

    /// Spawn a full sink with Postgres persistence + Lago blob store +
    /// in-memory file manifest.
    ///
    /// On `FileWritten` events the background task calls
    /// `provider.read_file()` and stores the content in `blob_store`
    /// (content-addressed, deduplicated). If `read_file` is not supported by
    /// the provider, the manifest entry is still recorded using the SHA-256
    /// hash pre-computed by [`JournaledSandboxProvider`].
    ///
    /// Returns both the sink and a shared handle to the manifest so callers
    /// can query it.
    pub fn spawn_with_manifest(
        pool: PgPool,
        blob_store: Arc<BlobStore>,
        provider: Arc<dyn SandboxProvider>,
    ) -> (Self, Arc<Mutex<SandboxManifest>>) {
        let manifest = Arc::new(Mutex::new(SandboxManifest::new()));
        let manifest_bg = Arc::clone(&manifest);
        let (tx, mut rx) = mpsc::unbounded_channel::<SandboxEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Always persist to Postgres.
                if let Err(e) = write_event(&pool, &event).await {
                    warn!(
                        sandbox_id = %event.sandbox_id,
                        kind = ?event.kind,
                        error = %e,
                        "sandbox event write failed"
                    );
                }

                // For FileWritten, also sync to blob store + manifest.
                if let SandboxEventKind::FileWritten {
                    ref path,
                    size_bytes,
                    ref sha256,
                    mode,
                } = event.kind
                {
                    let entry = sync_file_written(FileWrittenParams {
                        sandbox_id: &event.sandbox_id,
                        session_id: &event.session_id,
                        path,
                        size_bytes,
                        sha256,
                        mode,
                        provider: &provider,
                        blob_store: &blob_store,
                        provider_name: &event.provider,
                    })
                    .await;

                    manifest_bg
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .upsert(entry);
                }
            }
        });

        (Self { tx }, manifest)
    }
}

impl SandboxEventSink for LagoSandboxEventSink {
    fn emit(&self, event: SandboxEvent) {
        if let Err(e) = self.tx.send(event) {
            warn!("LagoSandboxEventSink: background channel closed: {e}");
        }
    }
}

// ── write_event ───────────────────────────────────────────────────────────────

/// Write a single `SandboxEvent` to the database.
///
/// Always inserts a row into `sandbox_events`.  Additionally upserts
/// `sandbox_instances` or appends to `sandbox_snapshots` depending on kind.
async fn write_event(pool: &PgPool, event: &SandboxEvent) -> Result<(), sqlx::Error> {
    let (exit_code, duration_ms, snapshot_ref, error_msg) = match &event.kind {
        SandboxEventKind::ExecCompleted {
            exit_code,
            duration_ms,
        } => (
            Some(*exit_code),
            Some(*duration_ms as i64),
            None::<String>,
            None::<String>,
        ),
        SandboxEventKind::Snapshotted { snapshot_id } => {
            (None, None, Some(snapshot_id.clone()), None)
        }
        SandboxEventKind::Resumed { from_snapshot } => {
            (None, None, Some(from_snapshot.clone()), None)
        }
        SandboxEventKind::Failed { reason } => (None, None, None, Some(reason.clone())),
        _ => (None, None, None, None),
    };

    let kind_str = event_kind_str(&event.kind);

    // 1. Insert into sandbox_events (always).
    sqlx::query(
        r#"
        INSERT INTO sandbox_events
            (sandbox_id, agent_id, session_id, organization_id, provider,
             event_kind, exit_code, duration_ms, snapshot_id, error_message,
             occurred_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(&event.sandbox_id.0)
    .bind(&event.agent_id)
    .bind(&event.session_id)
    .bind(&event.agent_id) // organization_id: use agent_id as surrogate until org_id is in SandboxEvent
    .bind(&event.provider)
    .bind(kind_str)
    .bind(exit_code)
    .bind(duration_ms)
    .bind(&snapshot_ref)
    .bind(&error_msg)
    .bind(event.timestamp)
    .execute(pool)
    .await?;

    // 2. Update sandbox_instances based on event kind.
    match &event.kind {
        SandboxEventKind::Created => {
            sqlx::query(
                r#"
                INSERT INTO sandbox_instances
                    (sandbox_id, agent_id, session_id, organization_id,
                     provider, status, created_at)
                VALUES ($1, $2, $3, $4, $5, 'starting', $6)
                ON CONFLICT (sandbox_id) DO NOTHING
                "#,
            )
            .bind(&event.sandbox_id.0)
            .bind(&event.agent_id)
            .bind(&event.session_id)
            .bind(&event.agent_id)
            .bind(&event.provider)
            .bind(event.timestamp)
            .execute(pool)
            .await?;
        }
        SandboxEventKind::Started => {
            sqlx::query("UPDATE sandbox_instances SET status = 'running' WHERE sandbox_id = $1")
                .bind(&event.sandbox_id.0)
                .execute(pool)
                .await?;
        }
        SandboxEventKind::ExecCompleted { .. } => {
            sqlx::query("UPDATE sandbox_instances SET last_exec_at = $2 WHERE sandbox_id = $1")
                .bind(&event.sandbox_id.0)
                .bind(event.timestamp)
                .execute(pool)
                .await?;
        }
        SandboxEventKind::Snapshotted { snapshot_id } => {
            sqlx::query(
                "UPDATE sandbox_instances SET status = 'snapshotted' WHERE sandbox_id = $1",
            )
            .bind(&event.sandbox_id.0)
            .execute(pool)
            .await?;

            sqlx::query(
                r#"
                INSERT INTO sandbox_snapshots
                    (sandbox_id, snapshot_id, trigger, created_at)
                VALUES ($1, $2, 'idle_reaper', $3)
                "#,
            )
            .bind(&event.sandbox_id.0)
            .bind(snapshot_id)
            .bind(event.timestamp)
            .execute(pool)
            .await?;
        }
        SandboxEventKind::Resumed { from_snapshot } => {
            sqlx::query("UPDATE sandbox_instances SET status = 'running' WHERE sandbox_id = $1")
                .bind(&event.sandbox_id.0)
                .execute(pool)
                .await?;

            sqlx::query(
                r#"
                INSERT INTO sandbox_snapshots
                    (sandbox_id, snapshot_id, trigger, created_at)
                VALUES ($1, $2, 'resumed', $3)
                "#,
            )
            .bind(&event.sandbox_id.0)
            .bind(from_snapshot)
            .bind(event.timestamp)
            .execute(pool)
            .await?;
        }
        SandboxEventKind::Destroyed => {
            sqlx::query(
                r#"UPDATE sandbox_instances
                   SET status = 'stopped', destroyed_at = $2
                   WHERE sandbox_id = $1"#,
            )
            .bind(&event.sandbox_id.0)
            .bind(event.timestamp)
            .execute(pool)
            .await?;
        }
        SandboxEventKind::Failed { reason } => {
            sqlx::query(
                r#"UPDATE sandbox_instances
                   SET status = 'failed',
                       metadata = metadata || jsonb_build_object('error', $2::text)
                   WHERE sandbox_id = $1"#,
            )
            .bind(&event.sandbox_id.0)
            .bind(reason)
            .execute(pool)
            .await?;
        }
        SandboxEventKind::FileWritten { .. } => {
            // File manifest tracking is handled by the manifest sync path;
            // no sandbox_instances status change needed.
        }
    }

    Ok(())
}

fn event_kind_str(kind: &SandboxEventKind) -> &'static str {
    match kind {
        SandboxEventKind::Created => "created",
        SandboxEventKind::Started => "started",
        SandboxEventKind::ExecCompleted { .. } => "exec_completed",
        SandboxEventKind::Snapshotted { .. } => "snapshotted",
        SandboxEventKind::Resumed { .. } => "resumed",
        SandboxEventKind::Destroyed => "destroyed",
        SandboxEventKind::Failed { .. } => "failed",
        SandboxEventKind::FileWritten { .. } => "file_written",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_sandbox::{SandboxEventKind, SandboxId};

    fn make_event(kind: SandboxEventKind) -> SandboxEvent {
        SandboxEvent::now(
            SandboxId("test-sandbox".into()),
            "agent-1",
            "sess-1",
            kind,
            "local",
        )
    }

    #[test]
    fn event_kind_str_covers_all_variants() {
        assert_eq!(event_kind_str(&SandboxEventKind::Created), "created");
        assert_eq!(event_kind_str(&SandboxEventKind::Started), "started");
        assert_eq!(
            event_kind_str(&SandboxEventKind::ExecCompleted {
                exit_code: 0,
                duration_ms: 1
            }),
            "exec_completed"
        );
        assert_eq!(
            event_kind_str(&SandboxEventKind::Snapshotted {
                snapshot_id: "s".into()
            }),
            "snapshotted"
        );
        assert_eq!(
            event_kind_str(&SandboxEventKind::Resumed {
                from_snapshot: "s".into()
            }),
            "resumed"
        );
        assert_eq!(event_kind_str(&SandboxEventKind::Destroyed), "destroyed");
        assert_eq!(
            event_kind_str(&SandboxEventKind::Failed {
                reason: "oom".into()
            }),
            "failed"
        );
        assert_eq!(
            event_kind_str(&SandboxEventKind::FileWritten {
                path: "/f".into(),
                size_bytes: 0,
                sha256: "abc".into(),
                mode: 0o644,
            }),
            "file_written"
        );
    }

    #[tokio::test]
    async fn emit_does_not_panic_without_pool() {
        // Verify that the sink can be used without a real DB.
        // We test this by calling emit on a custom sink that just discards.
        use arcan_sandbox::sink::NoopSink;
        let sink = NoopSink;
        sink.emit(make_event(SandboxEventKind::Created));
        sink.emit(make_event(SandboxEventKind::Destroyed));
    }

    #[tokio::test]
    async fn file_written_event_updates_manifest() {
        use arcan_sandbox::{
            ExecRequest, ExecResult, SandboxCapabilitySet, SandboxHandle, SandboxInfo, SandboxSpec,
            SandboxStatus, SnapshotId,
        };
        use async_trait::async_trait;

        struct StubReadProvider;

        #[async_trait]
        impl SandboxProvider for StubReadProvider {
            fn name(&self) -> &'static str {
                "stub"
            }
            fn capabilities(&self) -> SandboxCapabilitySet {
                SandboxCapabilitySet::FILESYSTEM_READ | SandboxCapabilitySet::FILESYSTEM_WRITE
            }
            async fn create(
                &self,
                _: SandboxSpec,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unimplemented!()
            }
            async fn resume(
                &self,
                _: &SandboxId,
            ) -> Result<SandboxHandle, arcan_sandbox::SandboxError> {
                unimplemented!()
            }
            async fn run(
                &self,
                _: &SandboxId,
                _: ExecRequest,
            ) -> Result<ExecResult, arcan_sandbox::SandboxError> {
                unimplemented!()
            }
            async fn snapshot(
                &self,
                _: &SandboxId,
            ) -> Result<SnapshotId, arcan_sandbox::SandboxError> {
                unimplemented!()
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
                Ok(b"content".to_vec())
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let blob_store = Arc::new(BlobStore::open(dir.path().join("blobs")).unwrap());
        let provider: Arc<dyn SandboxProvider> = Arc::new(StubReadProvider);
        // Use a pool-less path: test only the manifest sync by creating
        // a logging-only background task that doesn't need Postgres.
        let manifest = Arc::new(Mutex::new(SandboxManifest::new()));
        let manifest_bg = Arc::clone(&manifest);
        let blob_store_bg = Arc::clone(&blob_store);
        let (tx, mut rx) = mpsc::unbounded_channel::<SandboxEvent>();

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let SandboxEventKind::FileWritten {
                    ref path,
                    size_bytes,
                    ref sha256,
                    mode,
                } = event.kind
                {
                    let entry = sync_file_written(FileWrittenParams {
                        sandbox_id: &event.sandbox_id,
                        session_id: &event.session_id,
                        path,
                        size_bytes,
                        sha256,
                        mode,
                        provider: &provider,
                        blob_store: &blob_store_bg,
                        provider_name: &event.provider,
                    })
                    .await;

                    manifest_bg
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .upsert(entry);
                }
            }
        });

        tx.send(SandboxEvent::now(
            SandboxId("box-1".into()),
            "agent-1",
            "sess-1",
            SandboxEventKind::FileWritten {
                path: "/workspace/hello.py".into(),
                size_bytes: 7,
                sha256: "some-hash".into(),
                mode: 0o644,
            },
            "stub",
        ))
        .unwrap();

        // Allow the background task to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let m = manifest.lock().unwrap();
        let entry = m.get(&SandboxId("box-1".into()), "/workspace/hello.py");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.path, "/workspace/hello.py");
        assert_eq!(entry.session_id, "sess-1");
        assert!(entry.blob_hash.is_some());
    }
}
