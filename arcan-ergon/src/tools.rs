//! `ergon::ToolRegistry` over `ToolHarnessPort` + `PolicyGatePort`.
//!
//! The adapter exposes a static set of [`ergon::ToolDefinition`]s (the
//! registry is *the* source of truth in v0.1 — a future revision will
//! grow `ToolHarnessPort` to advertise definitions natively, at which
//! point the adapter will defer to that). Tool dispatch goes through
//! the kernel's [`aios_protocol::ToolHarnessPort::execute`].
//!
//! Capability gating is handled by the dedicated
//! [`crate::hooks::KernelCapabilityResolver`] hook, which talks to the
//! [`aios_protocol::PolicyGatePort`]. We deliberately do NOT call
//! `policy.evaluate(...)` inside `invoke` — the spec's hook ordering
//! says capability gating fires `on_pre_tool_use`, and pushing it
//! down into the registry would double-fire it.

use crate::error::AdapterError;
use aios_protocol::{ToolExecutionRequest, ToolHarnessPort, ToolOutcome};
use async_trait::async_trait;
use ergon::{
    ErgonError, Result as ErgonResult, ToolCall, ToolDefinition, ToolRegistry, ToolResult,
};
use std::sync::Arc;

/// Adapter that runs tool calls through the kernel's
/// [`ToolHarnessPort`].
///
/// One instance per tick (constructed by [`crate::run_workflow_as_tick`]).
/// The adapter holds the live `SessionId` and `workspace_root` so it
/// can build [`ToolExecutionRequest`]s without leaking those fields
/// through `ergon::ToolRegistry::invoke`.
pub struct ToolHarnessAdapter {
    port: Arc<dyn ToolHarnessPort>,
    session_id: aios_protocol::SessionId,
    workspace_root: String,
    definitions: Vec<ToolDefinition>,
}

impl ToolHarnessAdapter {
    /// Construct from a port plus per-tick context.
    ///
    /// `definitions` is the static set of tools advertised to the
    /// model. Pass an empty `Vec` if the workflow doesn't expose any
    /// tools on this turn (the autonomous loop will then disallow
    /// tool calls).
    pub fn new(
        port: Arc<dyn ToolHarnessPort>,
        session_id: aios_protocol::SessionId,
        workspace_root: impl Into<String>,
        definitions: Vec<ToolDefinition>,
    ) -> Self {
        Self {
            port,
            session_id,
            workspace_root: workspace_root.into(),
            definitions,
        }
    }
}

#[async_trait]
impl ToolRegistry for ToolHarnessAdapter {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.definitions.clone()
    }

    async fn invoke(&self, call: ToolCall) -> ErgonResult<ToolResult> {
        let port_call = aios_protocol::tool::ToolCall {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            input: call.input.clone(),
            requested_capabilities: Vec::new(),
        };
        let request = ToolExecutionRequest {
            session_id: self.session_id.clone(),
            workspace_root: self.workspace_root.clone(),
            call: port_call,
        };

        let report = self.port.execute(request).await.map_err(|err| {
            ErgonError::Tool(AdapterError::port("ToolHarnessPort", err).to_string())
        })?;

        let result = match report.outcome {
            ToolOutcome::Success { output } => ToolResult::success(call.id.clone(), output),
            ToolOutcome::Failure { error } => {
                ToolResult::model_error(call.id.clone(), serde_json::json!({ "error": error }))
            }
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::tool::ToolCall as ProtoToolCall;
    use aios_protocol::{BranchId, KernelError, KernelResult, SessionId, ToolExecutionReport};
    use ergon::ToolCall as ErgonToolCall;

    struct EchoHarness;

    #[async_trait]
    impl ToolHarnessPort for EchoHarness {
        async fn execute(
            &self,
            request: ToolExecutionRequest,
        ) -> KernelResult<ToolExecutionReport> {
            let _: &SessionId = &request.session_id;
            let _: &ProtoToolCall = &request.call;
            Ok(ToolExecutionReport {
                tool_run_id: aios_protocol::ToolRunId::default(),
                call_id: request.call.call_id.clone(),
                tool_name: request.call.tool_name.clone(),
                exit_status: 0,
                duration_ms: 1,
                outcome: ToolOutcome::Success {
                    output: serde_json::json!({"echoed": request.call.input}),
                },
            })
        }
    }

    struct FailingHarness;

    #[async_trait]
    impl ToolHarnessPort for FailingHarness {
        async fn execute(
            &self,
            _request: ToolExecutionRequest,
        ) -> KernelResult<ToolExecutionReport> {
            Err(KernelError::Runtime("harness offline".into()))
        }
    }

    fn def(name: &str) -> ToolDefinition {
        ToolDefinition::new(name, name, serde_json::json!({"type": "object"}))
    }

    #[tokio::test]
    async fn definitions_round_trip() {
        let adapter = ToolHarnessAdapter::new(
            Arc::new(EchoHarness),
            SessionId::default(),
            "/tmp/work",
            vec![def("alpha"), def("beta")],
        );
        let names: Vec<_> = adapter.definitions().into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["alpha".to_owned(), "beta".to_owned()]);
        let _ = BranchId::main();
    }

    #[tokio::test]
    async fn successful_invoke_returns_success() {
        let adapter = ToolHarnessAdapter::new(
            Arc::new(EchoHarness),
            SessionId::default(),
            "/tmp/work",
            Vec::new(),
        );
        let call = ErgonToolCall::new("echo", "calls", serde_json::json!({"k": 1}));
        let result = adapter.invoke(call).await.expect("invoke ok");
        assert!(!result.is_error);
        assert_eq!(result.output["echoed"], serde_json::json!({"k": 1}));
    }

    #[tokio::test]
    async fn port_failure_surfaces_as_tool_error() {
        let adapter = ToolHarnessAdapter::new(
            Arc::new(FailingHarness),
            SessionId::default(),
            "/tmp/work",
            Vec::new(),
        );
        let call = ErgonToolCall::new("doomed", "calls", serde_json::json!({}));
        let err = adapter.invoke(call).await.expect_err("port should fail");
        assert!(matches!(err, ErgonError::Tool(_)));
    }
}
