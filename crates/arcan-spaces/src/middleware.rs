use crate::port::{SpacesMessageType, SpacesPort};
use arcan_core::error::CoreError;
use arcan_core::protocol::ToolResult;
use arcan_core::runtime::{Middleware, RunOutput, ToolContext};
use std::sync::Arc;

/// Middleware that logs agent tool activity and run summaries to a Spaces channel.
///
/// Posts `AgentEvent`-typed messages to the configured `agent_log_channel_id`.
/// Errors from the port are swallowed (logged via `tracing::warn!`) — observability
/// must never block the agent loop.
pub struct SpacesActivityMiddleware {
    port: Arc<dyn SpacesPort>,
    agent_log_channel_id: u64,
}

impl SpacesActivityMiddleware {
    pub fn new(port: Arc<dyn SpacesPort>, agent_log_channel_id: u64) -> Self {
        Self {
            port,
            agent_log_channel_id,
        }
    }
}

impl Middleware for SpacesActivityMiddleware {
    fn post_tool_call(&self, _context: &ToolContext, result: &ToolResult) -> Result<(), CoreError> {
        let status = if result.is_error {
            "failed"
        } else {
            "completed"
        };
        let content = format!("[agent] Tool '{}' {status}", result.tool_name);

        if let Err(e) = self
            .port
            .send_message(self.agent_log_channel_id, &content, None, None)
        {
            tracing::warn!(
                error = %e,
                channel_id = self.agent_log_channel_id,
                "SpacesActivityMiddleware: failed to log tool call"
            );
        }

        Ok(())
    }

    fn on_run_finished(&self, output: &RunOutput) -> Result<(), CoreError> {
        let content = format!(
            "[agent] Run '{}' finished: {:?} (iterations: {})",
            output.run_id,
            output.reason,
            output
                .events
                .iter()
                .filter(|e| {
                    matches!(e, arcan_core::protocol::AgentEvent::IterationStarted { .. })
                })
                .count()
        );

        if let Err(e) = self
            .port
            .send_message(self.agent_log_channel_id, &content, None, None)
        {
            tracing::warn!(
                error = %e,
                channel_id = self.agent_log_channel_id,
                "SpacesActivityMiddleware: failed to log run finished"
            );
        }

        Ok(())
    }
}

// Suppress unused import warning — SpacesMessageType is used conceptually for the
// AgentEvent message type but the mock doesn't enforce it in middleware.
const _: () = {
    let _ = SpacesMessageType::AgentEvent;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockSpacesClient;
    use arcan_core::protocol::{AgentEvent, RunStopReason, TokenUsage};
    use arcan_core::state::AppState;

    fn make_middleware() -> (Arc<MockSpacesClient>, SpacesActivityMiddleware) {
        let mock = Arc::new(MockSpacesClient::default_hub());
        let port: Arc<dyn SpacesPort> = mock.clone();
        let mw = SpacesActivityMiddleware::new(port, 2); // channel 2 = agent-logs
        (mock, mw)
    }

    #[test]
    fn logs_tool_activity() {
        let (mock, mw) = make_middleware();
        let ctx = ToolContext {
            run_id: "run-1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
        };
        let result = ToolResult {
            call_id: "c1".to_string(),
            tool_name: "read_file".to_string(),
            output: serde_json::json!({}),
            content: None,
            is_error: false,
            state_patch: None,
        };

        mw.post_tool_call(&ctx, &result).unwrap();
        let messages = mock.sent_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("read_file"));
        assert!(messages[0].content.contains("completed"));
    }

    #[test]
    fn logs_run_finished() {
        let (mock, mw) = make_middleware();
        let output = RunOutput {
            run_id: "run-42".to_string(),
            session_id: "s1".to_string(),
            branch_id: "main".to_string(),
            events: vec![AgentEvent::IterationStarted {
                run_id: "run-42".to_string(),
                session_id: "s1".to_string(),
                iteration: 1,
            }],
            messages: vec![],
            state: AppState::default(),
            reason: RunStopReason::Completed,
            final_answer: None,
            total_usage: TokenUsage::default(),
        };

        mw.on_run_finished(&output).unwrap();
        let messages = mock.sent_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("run-42"));
        assert!(messages[0].content.contains("Completed"));
    }

    #[test]
    fn swallows_port_errors() {
        let (mock, mw) = make_middleware();
        *mock.force_error.lock().unwrap() = Some("network down".to_string());

        let ctx = ToolContext {
            run_id: "run-1".to_string(),
            session_id: "s1".to_string(),
            iteration: 1,
        };
        let result = ToolResult {
            call_id: "c1".to_string(),
            tool_name: "bash".to_string(),
            output: serde_json::json!({}),
            content: None,
            is_error: false,
            state_patch: None,
        };

        // Should return Ok even though the port fails
        let outcome = mw.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());

        let output = RunOutput {
            run_id: "run-1".to_string(),
            session_id: "s1".to_string(),
            branch_id: "main".to_string(),
            events: vec![],
            messages: vec![],
            state: AppState::default(),
            reason: RunStopReason::Error,
            final_answer: None,
            total_usage: TokenUsage::default(),
        };
        let outcome = mw.on_run_finished(&output);
        assert!(outcome.is_ok());
    }
}
