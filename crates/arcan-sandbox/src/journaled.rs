//! `JournaledSandboxProvider` — decorator that emits [`SandboxEvent`]s for every
//! provider call.
//!
//! Wraps any [`SandboxProvider`] and intercepts `create`, `resume`, `run`,
//! `snapshot`, `destroy`, and `write_files`.  Each call emits a corresponding
//! [`SandboxEventKind`] to the attached [`SandboxEventSink`] after the
//! underlying provider succeeds.
//!
//! Errors from the inner provider are propagated as-is; no event is emitted on
//! failure (the sink receives only successful transitions).
//!
//! # Usage
//!
//! ```rust,ignore
//! use arcan_sandbox::{JournaledSandboxProvider, NoopSink, SandboxProvider};
//!
//! let journaled = JournaledSandboxProvider::new(inner_provider, NoopSink, "agent-1", "sess-1");
//! let handle = journaled.create(spec).await?;
//! ```

use sha2::{Digest, Sha256};

use crate::error::SandboxError;
use crate::event::{SandboxEvent, SandboxEventKind};
use crate::provider::SandboxProvider;
use crate::sink::SandboxEventSink;
use crate::types::{
    ExecRequest, ExecResult, FileWrite, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec,
    SnapshotId,
};

// ── JournaledSandboxProvider ──────────────────────────────────────────────────

/// Decorator that wraps a [`SandboxProvider`] and emits lifecycle events to a
/// [`SandboxEventSink`] after each successful operation.
pub struct JournaledSandboxProvider<P, S> {
    inner: P,
    sink: S,
    agent_id: String,
    session_id: String,
}

impl<P, S> JournaledSandboxProvider<P, S>
where
    P: SandboxProvider,
    S: SandboxEventSink,
{
    /// Wrap `inner` with event journaling.
    ///
    /// - `agent_id` — stable agent identifier (attached to every event).
    /// - `session_id` — current session identifier (attached to every event).
    pub fn new(
        inner: P,
        sink: S,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            inner,
            sink,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
        }
    }

    /// Emit a single event to the attached sink.
    fn emit(&self, sandbox_id: SandboxId, kind: SandboxEventKind) {
        self.sink.emit(SandboxEvent::now(
            sandbox_id,
            &self.agent_id,
            &self.session_id,
            kind,
            self.inner.name(),
        ));
    }
}

#[async_trait::async_trait]
impl<P, S> SandboxProvider for JournaledSandboxProvider<P, S>
where
    P: SandboxProvider,
    S: SandboxEventSink + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn capabilities(&self) -> crate::capability::SandboxCapabilitySet {
        self.inner.capabilities()
    }

    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
        let handle = self.inner.create(spec).await?;
        self.emit(handle.id.clone(), SandboxEventKind::Created);
        Ok(handle)
    }

    async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
        let handle = self.inner.resume(id).await?;
        // Derive snapshot ID from handle metadata if present; fall back to
        // using the sandbox ID string as a sentinel for providers that don't
        // track snapshot lineage.
        let from_snapshot = handle
            .metadata
            .get("snapshot_id")
            .and_then(|v| v.as_str())
            .unwrap_or(id.0.as_str())
            .to_owned();
        self.emit(
            handle.id.clone(),
            SandboxEventKind::Resumed { from_snapshot },
        );
        Ok(handle)
    }

    async fn run(&self, id: &SandboxId, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        let result = self.inner.run(id, req).await?;
        self.emit(
            id.clone(),
            SandboxEventKind::ExecCompleted {
                exit_code: result.exit_code,
                duration_ms: result.duration_ms,
            },
        );
        Ok(result)
    }

    async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
        let snap = self.inner.snapshot(id).await?;
        self.emit(
            id.clone(),
            SandboxEventKind::Snapshotted {
                snapshot_id: snap.0.clone(),
            },
        );
        Ok(snap)
    }

    async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
        self.inner.destroy(id).await?;
        self.emit(id.clone(), SandboxEventKind::Destroyed);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
        // List is a read operation — no lifecycle event emitted.
        self.inner.list().await
    }

    async fn write_files(&self, id: &SandboxId, files: Vec<FileWrite>) -> Result<(), SandboxError> {
        // Pre-compute hashes before moving `files` into the inner provider.
        let hashed: Vec<(String, u64, String, u32)> = files
            .iter()
            .map(|f| {
                let hash = hex::encode(Sha256::digest(&f.content));
                let size = f.content.len() as u64;
                (f.path.clone(), size, hash, f.mode)
            })
            .collect();

        self.inner.write_files(id, files).await?;

        for (path, size_bytes, sha256, mode) in hashed {
            self.emit(
                id.clone(),
                SandboxEventKind::FileWritten {
                    path,
                    size_bytes,
                    sha256,
                    mode,
                },
            );
        }
        Ok(())
    }

    async fn read_file(&self, id: &SandboxId, path: &str) -> Result<Vec<u8>, SandboxError> {
        // Read is not a mutating lifecycle event — delegate only.
        self.inner.read_file(id, path).await
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::capability::SandboxCapabilitySet;
    use crate::error::SandboxError;
    use crate::event::{SandboxEvent, SandboxEventKind};
    use crate::sink::SandboxEventSink;
    use crate::types::{
        ExecResult, PersistencePolicy, SandboxHandle, SandboxId, SandboxInfo, SandboxSpec,
        SandboxStatus, SnapshotId,
    };

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_handle(id: &str) -> SandboxHandle {
        SandboxHandle {
            id: SandboxId(id.to_owned()),
            name: id.to_owned(),
            status: SandboxStatus::Running,
            created_at: chrono::Utc::now(),
            provider: "stub".to_owned(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Minimal stub provider that records calls and returns pre-canned responses.
    struct StubProvider;

    #[async_trait]
    impl SandboxProvider for StubProvider {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn capabilities(&self) -> SandboxCapabilitySet {
            SandboxCapabilitySet::FILESYSTEM_READ | SandboxCapabilitySet::FILESYSTEM_WRITE
        }

        async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
            Ok(make_handle(&spec.name))
        }

        async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
            Ok(make_handle(&id.0))
        }

        async fn run(
            &self,
            _id: &SandboxId,
            _req: ExecRequest,
        ) -> Result<ExecResult, SandboxError> {
            Ok(ExecResult {
                stdout: b"ok\n".to_vec(),
                stderr: vec![],
                exit_code: 0,
                duration_ms: 42,
            })
        }

        async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
            Ok(SnapshotId(format!("snap-{}", id.0)))
        }

        async fn destroy(&self, _id: &SandboxId) -> Result<(), SandboxError> {
            Ok(())
        }

        async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
            Ok(vec![])
        }

        async fn write_files(
            &self,
            _id: &SandboxId,
            _files: Vec<FileWrite>,
        ) -> Result<(), SandboxError> {
            Ok(())
        }
    }

    /// Collecting sink — accumulates events for assertions.
    #[derive(Clone)]
    struct CollectSink(Arc<Mutex<Vec<SandboxEvent>>>);

    impl CollectSink {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(vec![])))
        }

        fn events(&self) -> Vec<SandboxEvent> {
            self.0.lock().unwrap().clone()
        }
    }

    impl SandboxEventSink for CollectSink {
        fn emit(&self, event: SandboxEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn make_journaled() -> (
        JournaledSandboxProvider<StubProvider, CollectSink>,
        CollectSink,
    ) {
        let sink = CollectSink::new();
        let provider =
            JournaledSandboxProvider::new(StubProvider, sink.clone(), "agent-1", "sess-1");
        (provider, sink)
    }

    // ── Tests ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_emits_created_event() {
        let (p, sink) = make_journaled();
        let spec = SandboxSpec {
            name: "my-box".into(),
            image: None,
            resources: Default::default(),
            env: Default::default(),
            persistence: PersistencePolicy::Ephemeral,
            capabilities: SandboxCapabilitySet::FILESYSTEM_READ,
            labels: Default::default(),
        };
        p.create(spec).await.unwrap();
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SandboxEventKind::Created);
        assert_eq!(events[0].sandbox_id, SandboxId("my-box".into()));
    }

    #[tokio::test]
    async fn resume_emits_resumed_event() {
        let (p, sink) = make_journaled();
        p.resume(&SandboxId("box-1".into())).await.unwrap();
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].kind, SandboxEventKind::Resumed { .. }));
    }

    #[tokio::test]
    async fn run_emits_exec_completed_event() {
        let (p, sink) = make_journaled();
        p.run(&SandboxId("box-1".into()), ExecRequest::shell("echo hi"))
            .await
            .unwrap();
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].kind,
            SandboxEventKind::ExecCompleted {
                exit_code: 0,
                duration_ms: 42,
            }
        );
    }

    #[tokio::test]
    async fn snapshot_emits_snapshotted_event() {
        let (p, sink) = make_journaled();
        let snap = p.snapshot(&SandboxId("box-1".into())).await.unwrap();
        assert_eq!(snap.0, "snap-box-1");
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].kind,
            SandboxEventKind::Snapshotted {
                snapshot_id: "snap-box-1".into(),
            }
        );
    }

    #[tokio::test]
    async fn destroy_emits_destroyed_event() {
        let (p, sink) = make_journaled();
        p.destroy(&SandboxId("box-1".into())).await.unwrap();
        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, SandboxEventKind::Destroyed);
    }

    #[tokio::test]
    async fn write_files_emits_file_written_events() {
        let (p, sink) = make_journaled();
        let files = vec![
            FileWrite {
                path: "/workspace/main.py".into(),
                content: b"print('hello')".to_vec(),
                mode: 0o644,
            },
            FileWrite {
                path: "/workspace/run.sh".into(),
                content: b"#!/bin/sh\npython main.py".to_vec(),
                mode: 0o755,
            },
        ];
        p.write_files(&SandboxId("box-1".into()), files)
            .await
            .unwrap();

        let events = sink.events();
        assert_eq!(events.len(), 2);

        // First file
        let SandboxEventKind::FileWritten {
            path,
            size_bytes,
            sha256,
            mode,
        } = &events[0].kind
        else {
            panic!("expected FileWritten, got {:?}", events[0].kind);
        };
        assert_eq!(path, "/workspace/main.py");
        assert_eq!(*size_bytes, b"print('hello')".len() as u64);
        assert_eq!(*mode, 0o644);
        // SHA-256 is 64 hex chars
        assert_eq!(sha256.len(), 64);

        // Second file
        let SandboxEventKind::FileWritten { path, mode, .. } = &events[1].kind else {
            panic!("expected FileWritten, got {:?}", events[1].kind);
        };
        assert_eq!(path, "/workspace/run.sh");
        assert_eq!(*mode, 0o755);
    }

    #[tokio::test]
    async fn list_does_not_emit_events() {
        let (p, sink) = make_journaled();
        p.list().await.unwrap();
        assert!(sink.events().is_empty());
    }
}
