use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use aios_protocol::{
    KernelError, ToolExecutionReport, ToolExecutionRequest, ToolHarnessPort, ToolOutcome, ToolRunId,
};
use arcan_core::protocol::ToolCall;
use arcan_core::runtime::{ToolContext, ToolRegistry};
use async_trait::async_trait;
use tracing::Instrument;

/// Structured context captured when a run completes.
///
/// This gives observers a stable, typed seam for async post-run work
/// without forcing each observer to reconstruct the session history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunCompletionContext {
    pub objective: Option<String>,
    pub final_answer: Option<String>,
    pub assistant_messages: Option<String>,
    pub tool_calls_summary: Option<String>,
    pub tool_call_count: Option<u32>,
    pub tool_error_count: Option<u32>,
    pub knowledge_context: Option<String>,
    pub knowledge_query: Option<String>,
    pub knowledge_retrieved_count: Option<u32>,
    pub knowledge_top_relevance: Option<f64>,
}

#[async_trait]
pub trait ToolHarnessObserver: Send + Sync {
    async fn post_execute(&self, session_id: String, tool_name: String, is_error: bool);

    /// Called after an agent run completes. Default: no-op.
    ///
    /// Receives the session context so observers can run async evaluations
    /// (e.g. LLM-as-judge) without blocking the HTTP response.
    async fn on_run_finished(&self, _session_id: String, _context: RunCompletionContext) {}
}

#[derive(Clone)]
pub struct ArcanHarnessAdapter {
    registry: ToolRegistry,
    observers: Vec<Arc<dyn ToolHarnessObserver>>,
    /// `{data_dir}/sessions`: the only directory whose children may become a
    /// per-session tool boundary. `None` turns per-session scoping off.
    sessions_dir: Option<PathBuf>,
}

impl ArcanHarnessAdapter {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            observers: Vec::new(),
            sessions_dir: None,
        }
    }

    /// Enable per-session tool scoping (BRO-1491) for sessions whose
    /// workspaces live under `sessions_dir` (the kernel's `{root}/sessions`).
    ///
    /// Without it, the kernel's per-session root is ignored and tools use
    /// their construction-time workspace, which was the behavior before
    /// BRO-1491.
    pub fn with_sessions_dir(mut self, sessions_dir: impl Into<PathBuf>) -> Self {
        self.sessions_dir = Some(sessions_dir.into());
        self
    }

    /// The per-session root to hand to tools, or `None` for no scoping.
    ///
    /// The root is a filesystem boundary, so it is accepted only when it
    /// canonicalizes to exactly `sessions_dir/<request.session_id>`. Any other
    /// root is refused with `CapabilityDenied` instead of being passed on:
    /// one reached through `..`, one resolved through a symlink out of the
    /// tree, one naming another session's workspace, or one whose session id
    /// fails the grammar. Falling back to the boot workspace would hide the
    /// fault, so a bad root is refused rather than downgraded.
    fn verified_workspace_root(
        &self,
        request: &ToolExecutionRequest,
    ) -> Result<Option<String>, KernelError> {
        if request.workspace_root.is_empty() {
            return Ok(None);
        }
        let Some(sessions_dir) = self.sessions_dir.as_deref() else {
            return Ok(None);
        };
        let denied = |reason: String| {
            KernelError::CapabilityDenied(format!(
                "session workspace for {:?} rejected: {reason}",
                request.session_id.as_str()
            ))
        };
        let verified = aios_protocol::session_path::verify_session_root(
            sessions_dir,
            request.session_id.as_str(),
            Path::new(&request.workspace_root),
        )
        .map_err(|error| denied(error.to_string()))?;
        verified
            .into_os_string()
            .into_string()
            .map(Some)
            .map_err(|raw| denied(format!("{} is not valid UTF-8", Path::new(&raw).display())))
    }

    pub fn with_observer(mut self, observer: Arc<dyn ToolHarnessObserver>) -> Self {
        self.observers.push(observer);
        self
    }

    /// Return a reference to the registered observers.
    ///
    /// Used by the canonical router to call `on_run_finished` after a run completes.
    pub fn observers(&self) -> &[Arc<dyn ToolHarnessObserver>] {
        &self.observers
    }
}

#[async_trait]
impl ToolHarnessPort for ArcanHarnessAdapter {
    async fn execute(
        &self,
        request: ToolExecutionRequest,
    ) -> Result<ToolExecutionReport, KernelError> {
        let tool = self
            .registry
            .get(&request.call.tool_name)
            .ok_or_else(|| KernelError::ToolNotFound(request.call.tool_name.clone()))?;
        let workspace_root = self.verified_workspace_root(&request)?;

        let arcan_call = ToolCall {
            call_id: request.call.call_id.clone(),
            tool_name: request.call.tool_name.clone(),
            input: request.call.input.clone(),
        };
        let context = ToolContext {
            run_id: format!("run-{}", request.call.call_id),
            session_id: request.session_id.as_str().to_owned(),
            iteration: 1,
            // BRO-1491: thread the kernel's per-session workspace root
            // (`manifest.workspace_root`) into tool execution so filesystem
            // tools scope to `{data_dir}/sessions/<id>/` instead of the shared
            // boot workspace. Only a root that passed the containment check
            // above gets here.
            workspace_root,
        };

        let tool_span =
            life_vigil::spans::tool_span(&request.call.tool_name, &request.call.call_id);
        let tool_start = Instant::now();

        // `Tool::execute` is a *synchronous* interface that may block: file I/O,
        // BM25 indexing, or (for cross-session tools like `knowledge_search`) an
        // inner `tokio::runtime::Handle::block_on`. Running it directly on the
        // async worker thread makes any such nested `block_on` panic with
        // "Cannot block the current thread from within a runtime"; under the
        // release profile's `panic = "abort"` that aborts the whole arcand
        // process — BRO-1483, where one anonymous `knowledge_search` call took
        // down the runtime (chat 502 until restart). Execute on the blocking
        // pool instead, where blocking and nested `block_on` are legal, so a
        // tool fault stays a structured error the kernel records as
        // `ToolCallFailed` rather than a process crash.
        let exec_call = arcan_call.clone();
        let exec_context = context;
        let exec_span = tool_span.clone();
        let result = tokio::task::spawn_blocking(move || {
            exec_span.in_scope(|| tool.execute(&exec_call, &exec_context))
        })
        .await
        .map_err(|join_error| {
            KernelError::Runtime(format!(
                "tool '{}' execution task failed: {join_error}",
                arcan_call.tool_name
            ))
        })?
        .map_err(|error| KernelError::Runtime(error.to_string()))?;
        let tool_duration = tool_start.elapsed();
        let exit_status = if result.is_error { 1 } else { 0 };
        let status_str;
        let outcome = if result.is_error {
            status_str = "error";
            life_vigil::spans::record_tool_status(&tool_span, status_str);
            ToolOutcome::Failure {
                error: result
                    .output
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| "tool execution failed".to_owned()),
            }
        } else {
            status_str = "ok";
            life_vigil::spans::record_tool_status(&tool_span, status_str);
            ToolOutcome::Success {
                output: result.output,
            }
        };

        // Record GenAI tool execution metric.
        let genai_metrics = life_vigil::metrics::GenAiMetrics::new("arcan");
        genai_metrics.record_tool_execution(&arcan_call.tool_name, status_str);

        for observer in &self.observers {
            observer
                .as_ref()
                .post_execute(
                    request.session_id.as_str().to_owned(),
                    arcan_call.tool_name.clone(),
                    result.is_error,
                )
                .instrument(tool_span.clone())
                .await;
        }

        Ok(ToolExecutionReport {
            tool_run_id: ToolRunId::default(),
            call_id: arcan_call.call_id,
            tool_name: arcan_call.tool_name,
            exit_status,
            duration_ms: tool_duration.as_millis() as u64,
            outcome,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::SessionId;
    use arcan_core::error::CoreError;
    use arcan_core::protocol::{ToolDefinition, ToolResult};
    use arcan_core::runtime::Tool;

    /// Sync tool that drives an async op via `Handle::block_on` inside its
    /// synchronous `execute` — exactly the `knowledge_search` shape that
    /// crashed arcand in BRO-1483. On an async worker thread this panics;
    /// on the blocking pool it succeeds.
    struct BlockOnTool;

    impl Tool for BlockOnTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "block_on_tool".to_string(),
                description: "test tool that block_on's an async op in sync execute".to_string(),
                input_schema: serde_json::json!({ "type": "object", "properties": {} }),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: vec![],
                timeout_secs: None,
            }
        }

        fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
            let handle = tokio::runtime::Handle::current();
            let value = handle.block_on(async { 42u64 });
            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: serde_json::json!({ "value": value }),
                content: None,
                is_error: false,
                state_patch: None,
            })
        }
    }

    /// Regression for BRO-1483: a synchronous tool that nests `block_on` must
    /// not panic (and abort the process under `panic = "abort"`) when executed
    /// through the harness on an async runtime. Running on the blocking pool
    /// keeps the nested `block_on` legal.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_tool_does_not_panic_the_worker() {
        let mut registry = ToolRegistry::default();
        registry.register(BlockOnTool);
        let adapter = ArcanHarnessAdapter::new(registry);

        let request = ToolExecutionRequest {
            session_id: SessionId::from_string("sess-bro-1483"),
            workspace_root: "/tmp".to_string(),
            call: aios_protocol::ToolCall::new("block_on_tool", serde_json::json!({}), vec![]),
        };

        let report = adapter
            .execute(request)
            .await
            .expect("harness execute should return a report, not panic");

        assert_eq!(report.tool_name, "block_on_tool");
        assert_eq!(report.exit_status, 0);
        match report.outcome {
            ToolOutcome::Success { output } => {
                assert_eq!(output.get("value").and_then(|v| v.as_u64()), Some(42));
            }
            other => panic!("expected success outcome, got {other:?}"),
        }
    }

    // ── Per-session root containment (BRO-1491) ─────────────────────────

    /// Tool that reports the workspace root it was handed, and counts calls,
    /// so a test can prove a rejected root never reached a tool.
    struct RootProbe {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Tool for RootProbe {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "root_probe".to_string(),
                description: "echoes ctx.workspace_root".to_string(),
                input_schema: serde_json::json!({ "type": "object", "properties": {} }),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: vec![],
                timeout_secs: None,
            }
        }

        fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, CoreError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: serde_json::json!({ "workspace_root": ctx.workspace_root }),
                content: None,
                is_error: false,
                state_patch: None,
            })
        }
    }

    struct Fixture {
        tmp: tempfile::TempDir,
        sessions: PathBuf,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        adapter: ArcanHarnessAdapter,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let sessions = tmp.path().join("data/sessions");
        std::fs::create_dir_all(sessions.join("sess-a")).unwrap();
        std::fs::create_dir_all(sessions.join("sess-b")).unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut registry = ToolRegistry::default();
        registry.register(RootProbe {
            calls: calls.clone(),
        });
        let adapter = ArcanHarnessAdapter::new(registry).with_sessions_dir(&sessions);
        Fixture {
            tmp,
            sessions,
            calls,
            adapter,
        }
    }

    fn probe(session: &str, root: &Path) -> ToolExecutionRequest {
        ToolExecutionRequest {
            session_id: SessionId::from_string(session),
            workspace_root: root.display().to_string(),
            call: aios_protocol::ToolCall::new("root_probe", serde_json::json!({}), vec![]),
        }
    }

    fn handed_root(report: ToolExecutionReport) -> Option<String> {
        match report.outcome {
            ToolOutcome::Success { output } => output["workspace_root"].as_str().map(str::to_owned),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn own_session_root_reaches_the_tool_canonicalized() {
        let f = fixture();
        let report = f
            .adapter
            .execute(probe("sess-a", &f.sessions.join("sess-a")))
            .await
            .unwrap();
        let expected = f.sessions.join("sess-a").canonicalize().unwrap();
        assert_eq!(handed_root(report), Some(expected.display().to_string()));
    }

    #[tokio::test]
    async fn escaping_roots_are_refused_before_any_tool_runs() {
        let f = fixture();
        let outside = f.tmp.path().join("home");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(f.sessions.join("sess-a/artifacts")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, f.sessions.join("victim")).unwrap();

        let mut cases = vec![
            // Another tenant's workspace, named directly and via `..`.
            ("sess-a", f.sessions.join("sess-b")),
            ("sess-a", f.sessions.join("sess-a/../sess-b")),
            // Wider than any session: the sessions dir, the data dir, above it.
            ("sess-a", f.sessions.join("sess-a/..")),
            ("sess-a", f.sessions.join("sess-a/../..")),
            ("sess-a", f.sessions.join("sess-a/../../..")),
            ("sess-a", outside.clone()),
            // A subdirectory is not the workspace either.
            ("sess-a", f.sessions.join("sess-a/artifacts")),
            // Ids that fail the grammar, even when the root would resolve.
            ("../sess-b", f.sessions.join("sess-b")),
            ("..", f.sessions.clone()),
            ("", f.sessions.clone()),
        ];
        #[cfg(unix)]
        cases.push(("victim", f.sessions.join("victim")));

        for (session, root) in cases {
            let err = f
                .adapter
                .execute(probe(session, &root))
                .await
                .expect_err(&format!("{session:?} @ {} must be refused", root.display()));
            assert!(
                matches!(err, KernelError::CapabilityDenied(_)),
                "{session:?} @ {}: wrong error {err}",
                root.display()
            );
        }
        assert_eq!(
            f.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no refused root may reach a tool"
        );
    }

    #[tokio::test]
    async fn empty_root_means_no_scoping() {
        let f = fixture();
        let report = f
            .adapter
            .execute(probe("sess-a", Path::new("")))
            .await
            .unwrap();
        assert_eq!(handed_root(report), None);
    }

    #[tokio::test]
    async fn without_a_sessions_dir_the_root_is_ignored_not_trusted() {
        let f = fixture();
        let mut registry = ToolRegistry::default();
        registry.register(RootProbe {
            calls: f.calls.clone(),
        });
        let unscoped = ArcanHarnessAdapter::new(registry);
        let report = unscoped
            .execute(probe("sess-a", &f.sessions.join("sess-a/../..")))
            .await
            .unwrap();
        assert_eq!(handed_root(report), None);
    }
}
