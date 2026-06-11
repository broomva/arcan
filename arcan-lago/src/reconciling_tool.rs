//! Exec-path manifest reconciliation: tracks file changes made by shell tools.
//!
//! [`ReconcilingTool`] wraps any [`Tool`] (in practice the shell/exec tool)
//! and, after a successful invocation, reconciles the workspace against the
//! tracked [`FsTracker`] manifest. Every created/modified/deleted path is
//! turned into the same `FileWrite`/`FileDelete` event the [`FsPort`] write
//! path emits, and sent through the *same* mpsc channel drained by
//! [`run_event_writer`](crate::tracked_fs::run_event_writer).
//!
//! This closes the gap where shell commands write files directly to the
//! workspace — bypassing the [`LagoTrackedFs`](crate::tracked_fs::LagoTrackedFs)
//! write interception — leaving the session's lago history blind to anything a
//! shell command created, modified, or deleted.
//!
//! ## Best-effort, bounded, transparent
//!
//! Reconciliation is best-effort observability layered *on top of* the tool.
//! It never changes exec semantics:
//! - the inner tool's result (and any error) is returned verbatim;
//! - reconciliation runs after the inner tool returns an `Ok` result whose
//!   `is_error` flag is unset. In practice this means it reconciles after
//!   *every* shell command — the canonical `BashTool` reports a non-zero exit
//!   as a normal `Ok { is_error: false }` result (the command ran; its exit
//!   code is data, not a tool failure), so a failed-but-executed command still
//!   has its workspace side effects tracked, which is correct. The `is_error`
//!   guard is defensive cover for a hypothetical inner tool that *does* set the
//!   flag to signal an aborted exec; only a hard `Err` (the tool itself failed
//!   to run) skips reconciliation.
//! - a reconciliation failure (diff/blob-put error, or a full/closed channel)
//!   is logged and swallowed — the exec already happened.
//!
//! The walk is bounded via [`SnapshotLimits`] (file-count + per-file-size caps,
//! pruning `.git`/journal/blob dirs) so a runaway command cannot make tracking
//! unbounded.

use std::path::PathBuf;
use std::sync::Arc;

use aios_protocol::tool::{Tool, ToolCall, ToolContext, ToolDefinition, ToolError, ToolResult};
use lago_core::event::EventPayload;
use lago_fs::{FsTracker, SnapshotLimits};
use tokio::sync::mpsc;

/// Decorator that reconciles workspace changes after the inner tool runs.
///
/// Generic over `T: Tool` so it imposes no dependency on the concrete tool
/// (the shell tool lives in `praxis-tools`, which this crate does not depend
/// on). Wrap the inner tool *before* bridging it into Arcan's registry:
///
/// ```ignore
/// let bash = ReconcilingTool::new(BashTool::new(policy, runner), tracker, tx, workspace_root);
/// registry.register(PraxisToolBridge::new(bash));
/// ```
pub struct ReconcilingTool<T> {
    inner: T,
    tracker: Arc<FsTracker>,
    tx: mpsc::Sender<EventPayload>,
    workspace_root: PathBuf,
    limits: SnapshotLimits,
}

impl<T> ReconcilingTool<T> {
    /// Wrap `inner`, reconciling against `tracker` (and emitting on `tx`) after
    /// each successful invocation. Uses the default [`SnapshotLimits`].
    pub fn new(
        inner: T,
        tracker: Arc<FsTracker>,
        tx: mpsc::Sender<EventPayload>,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            inner,
            tracker,
            tx,
            workspace_root,
            limits: SnapshotLimits::default(),
        }
    }

    /// Override the snapshot walk bounds (file-count + per-file-size caps).
    pub fn with_limits(mut self, limits: SnapshotLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Reconcile the workspace and emit the resulting payloads. Best-effort:
    /// every failure path is logged and swallowed so the caller's tool result
    /// is never affected.
    fn reconcile_and_emit(&self) {
        match self
            .tracker
            .reconcile_bounded(&self.workspace_root, self.limits)
        {
            Ok(payloads) => {
                for payload in payloads {
                    // Non-blocking, mirroring LagoTrackedFs::write: a full or
                    // closed channel drops the event with a warning rather than
                    // stalling the (already-completed) tool call.
                    if let Err(e) = self.tx.try_send(payload) {
                        tracing::warn!(
                            "ReconcilingTool: event channel full or closed, exec-path event dropped: {e}"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    workspace = %self.workspace_root.display(),
                    "ReconcilingTool: workspace reconciliation failed (tracking skipped): {e}"
                );
            }
        }
    }
}

impl<T: Tool> Tool for ReconcilingTool<T> {
    fn definition(&self) -> ToolDefinition {
        self.inner.definition()
    }

    fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        // Run the inner tool first. Its result is authoritative and returned
        // verbatim regardless of what reconciliation does.
        let result = self.inner.execute(call, ctx)?;

        // Reconcile after the tool returns an Ok result with `is_error` unset.
        // The canonical BashTool always reports `is_error: false` (a non-zero
        // exit is normal data, not a tool failure), so this reconciles after
        // every executed command — including ones that exited non-zero, whose
        // workspace side effects we still want tracked. The guard only suppresses
        // reconciliation for a hypothetical inner tool that sets `is_error` to
        // signal an aborted exec; a hard `Err` already returned above.
        if !result.is_error {
            self.reconcile_and_emit();
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_fs::Manifest;
    use lago_store::{BlobStore, LocalBlobBackend};
    use serde_json::json;
    use std::path::Path;

    /// A fake "shell" tool that performs a filesystem side effect (mutating the
    /// workspace directly, like a real shell command) then returns a result.
    struct SideEffectTool {
        /// Closure run on `execute`, given the workspace root.
        effect: Box<dyn Fn(&Path) + Send + Sync>,
        is_error: bool,
        workspace_root: PathBuf,
    }

    impl Tool for SideEffectTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "fake_shell".into(),
                description: "test".into(),
                input_schema: json!({"type": "object"}),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: vec![],
                timeout_secs: None,
            }
        }

        fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
            (self.effect)(&self.workspace_root);
            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: json!({"ok": true}),
                content: None,
                is_error: self.is_error,
                usage: None,
            })
        }
    }

    /// A tool whose own execution fails — reconciliation must NOT run.
    struct FailingTool;

    impl Tool for FailingTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "failing".into(),
                description: "always fails".into(),
                input_schema: json!({"type": "object"}),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: vec![],
                timeout_secs: None,
            }
        }

        fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
            Err(ToolError::ExecutionFailed {
                tool_name: call.tool_name.clone(),
                message: "boom".into(),
            })
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        PathBuf,
        Arc<FsTracker>,
        mpsc::Sender<EventPayload>,
        mpsc::Receiver<EventPayload>,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let blob_store: Arc<dyn lago_store::BlobBackend> = Arc::new(LocalBlobBackend::new(
            Arc::new(BlobStore::open(tmp.path().join("blobs")).unwrap()),
        ));
        let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store));
        let (tx, rx) = mpsc::channel(100);
        (tmp, ws, tracker, tx, rx)
    }

    fn call() -> ToolCall {
        ToolCall {
            call_id: "c1".into(),
            tool_name: "fake_shell".into(),
            input: json!({}),
            requested_capabilities: vec![],
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 0,
            ..Default::default()
        }
    }

    fn drain(rx: &mut mpsc::Receiver<EventPayload>) -> Vec<EventPayload> {
        let mut out = Vec::new();
        while let Ok(p) = rx.try_recv() {
            out.push(p);
        }
        out
    }

    #[test]
    fn created_file_emits_file_write_and_updates_manifest() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();
        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::write(root.join("created.txt"), b"new content").unwrap();
            }),
            is_error: false,
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone());

        let result = tool.execute(&call(), &ctx()).unwrap();
        assert!(!result.is_error);

        // (a) manifest contains the new file
        assert!(tracker.manifest().exists("/created.txt"));
        // (c) a FileWrite event was emitted on the shared channel
        let events = drain(&mut rx);
        assert!(events.iter().any(|p| matches!(
            p,
            EventPayload::FileWrite { path, size_bytes, .. }
            if path == "/created.txt" && *size_bytes == 11
        )));
    }

    #[test]
    fn modified_file_emits_file_write() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();

        // Seed: write an initial file and reconcile once so the tracker's
        // manifest knows the prior state.
        std::fs::write(ws.join("mod.txt"), b"original").unwrap();
        tracker
            .reconcile_bounded(&ws, SnapshotLimits::default())
            .unwrap();

        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::write(
                    root.join("mod.txt"),
                    b"a much longer replacement body than before",
                )
                .unwrap();
            }),
            is_error: false,
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone());

        tool.execute(&call(), &ctx()).unwrap();

        let events = drain(&mut rx);
        assert!(events.iter().any(|p| matches!(
            p,
            EventPayload::FileWrite { path, .. } if path == "/mod.txt"
        )));
    }

    #[test]
    fn deleted_file_emits_file_delete() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();

        std::fs::write(ws.join("doomed.txt"), b"bye").unwrap();
        tracker
            .reconcile_bounded(&ws, SnapshotLimits::default())
            .unwrap();
        assert!(tracker.manifest().exists("/doomed.txt"));

        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::remove_file(root.join("doomed.txt")).unwrap();
            }),
            is_error: false,
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone());

        tool.execute(&call(), &ctx()).unwrap();

        assert!(!tracker.manifest().exists("/doomed.txt"));
        let events = drain(&mut rx);
        assert!(events.iter().any(|p| matches!(
            p,
            EventPayload::FileDelete { path } if path == "/doomed.txt"
        )));
    }

    #[test]
    fn inner_failure_skips_reconciliation() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();
        // Create a file on disk BEFORE the failing tool runs — if reconciliation
        // erroneously fired, it would emit an event for it.
        std::fs::write(ws.join("present.txt"), b"x").unwrap();

        let tool = ReconcilingTool::new(FailingTool, tracker.clone(), tx, ws.clone());
        let err = tool.execute(&call(), &ctx()).unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed { .. }));

        // No reconciliation → no events, manifest untouched.
        assert!(drain(&mut rx).is_empty());
        assert!(!tracker.manifest().exists("/present.txt"));
    }

    #[test]
    fn is_error_result_skips_reconciliation() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();
        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::write(root.join("partial.txt"), b"side effect").unwrap();
            }),
            is_error: true, // command signalled failure
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone());

        let result = tool.execute(&call(), &ctx()).unwrap();
        assert!(result.is_error);
        // is_error → reconciliation skipped even though a file was written.
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn tracking_failure_does_not_affect_tool_result() {
        let (_tmp, ws, tracker, tx, rx) = setup();
        // Drop the receiver so the channel is closed: every try_send fails.
        drop(rx);

        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::write(root.join("ok.txt"), b"content").unwrap();
            }),
            is_error: false,
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone());

        // Tool still succeeds; the closed channel is swallowed.
        let result = tool.execute(&call(), &ctx()).unwrap();
        assert!(!result.is_error);
        // The manifest is still updated (reconcile ran; only the emit failed).
        assert!(tracker.manifest().exists("/ok.txt"));
    }

    #[test]
    fn definition_delegates_to_inner() {
        let (_tmp, ws, tracker, tx, _rx) = setup();
        let inner = SideEffectTool {
            effect: Box::new(|_| {}),
            is_error: false,
            workspace_root: ws.clone(),
        };
        let tool = ReconcilingTool::new(inner, tracker, tx, ws);
        assert_eq!(tool.definition().name, "fake_shell");
    }

    #[test]
    fn oversized_file_not_tracked_under_bound() {
        let (_tmp, ws, tracker, tx, mut rx) = setup();
        let ws_for_effect = ws.clone();
        let inner = SideEffectTool {
            effect: Box::new(move |root| {
                std::fs::write(root.join("small.txt"), b"ok").unwrap();
                std::fs::write(root.join("big.bin"), vec![0u8; 8192]).unwrap();
            }),
            is_error: false,
            workspace_root: ws_for_effect,
        };
        let tool = ReconcilingTool::new(inner, tracker.clone(), tx, ws.clone()).with_limits(
            SnapshotLimits {
                max_files: 10_000,
                max_file_bytes: 1024,
            },
        );

        tool.execute(&call(), &ctx()).unwrap();

        // Bounded walk: small tracked, oversized skipped; tool still succeeded.
        assert!(tracker.manifest().exists("/small.txt"));
        assert!(!tracker.manifest().exists("/big.bin"));
        let events = drain(&mut rx);
        assert!(events.iter().any(|p| matches!(
            p,
            EventPayload::FileWrite { path, .. } if path == "/small.txt"
        )));
        assert!(!events.iter().any(|p| matches!(
            p,
            EventPayload::FileWrite { path, .. } if path == "/big.bin"
        )));
    }
}
