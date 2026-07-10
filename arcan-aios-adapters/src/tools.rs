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
}

impl ArcanHarnessAdapter {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            observers: Vec::new(),
        }
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

        let arcan_call = ToolCall {
            call_id: request.call.call_id.clone(),
            tool_name: request.call.tool_name.clone(),
            input: request.call.input.clone(),
        };
        let context = ToolContext {
            run_id: format!("run-{}", request.call.call_id),
            session_id: request.session_id.as_str().to_owned(),
            iteration: 1,
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
}
