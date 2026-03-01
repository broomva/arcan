use aios_protocol::{
    KernelError, ToolExecutionReport, ToolExecutionRequest, ToolHarnessPort, ToolOutcome, ToolRunId,
};
use arcan_core::protocol::ToolCall;
use arcan_core::runtime::{ToolContext, ToolRegistry};
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait ToolHarnessObserver: Send + Sync {
    async fn post_execute(&self, session_id: String, tool_name: String);
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

        let result = tool
            .execute(&arcan_call, &context)
            .map_err(|error| KernelError::Runtime(error.to_string()))?;
        let exit_status = if result.is_error { 1 } else { 0 };
        let outcome = if result.is_error {
            ToolOutcome::Failure {
                error: result
                    .output
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| "tool execution failed".to_owned()),
            }
        } else {
            ToolOutcome::Success {
                output: result.output,
            }
        };

        for observer in &self.observers {
            observer
                .as_ref()
                .post_execute(
                    request.session_id.as_str().to_owned(),
                    arcan_call.tool_name.clone(),
                )
                .await;
        }

        Ok(ToolExecutionReport {
            tool_run_id: ToolRunId::default(),
            call_id: arcan_call.call_id,
            tool_name: arcan_call.tool_name,
            exit_status,
            duration_ms: 0,
            outcome,
        })
    }
}
