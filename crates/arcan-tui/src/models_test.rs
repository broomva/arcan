#[cfg(test)]
mod tests {
    use crate::models::{AppState, ToolStatus, UiBlock};
    use arcan_core::protocol::{AgentEvent, ToolCall, ToolResultSummary};
    use serde_json::json;

    #[test]
    fn test_apply_tool_execution_flow() {
        let mut state = AppState::new();

        // Simulate start
        state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            provider: "test".to_string(),
            max_iterations: 10,
        });
        assert!(state.is_busy);

        // Simulate tool request
        let call_req = ToolCall {
            call_id: "call_123".to_string(),
            tool_name: "fs.read".to_string(),
            input: json!({"path": "foo.txt"}),
        };
        state.apply_event(AgentEvent::ToolCallRequested {
            call: call_req,
            iteration: 1,
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
        });

        assert_eq!(state.blocks.len(), 1);
        match &state.blocks[0] {
            UiBlock::ToolExecution {
                call_id, status, ..
            } => {
                assert_eq!(call_id, "call_123");
                assert_eq!(*status, ToolStatus::Running);
            }
            _ => panic!("Expected ToolExecution block"),
        }

        // Simulate tool completion
        let result = ToolResultSummary {
            call_id: "call_123".to_string(),
            tool_name: "fs.read".to_string(),
            output: json!("file content"),
        };
        state.apply_event(AgentEvent::ToolCallCompleted {
            result,
            iteration: 1,
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
        });

        match &state.blocks[0] {
            UiBlock::ToolExecution {
                status,
                result: block_res,
                ..
            } => {
                assert_eq!(*status, ToolStatus::Success);
                assert_eq!(block_res.as_ref().unwrap(), &json!("file content"));
            }
            _ => panic!("Expected ToolExecution block"),
        }
    }

    #[test]
    fn test_state_approval_flow() {
        let mut state = AppState::new();
        state.apply_event(AgentEvent::ApprovalRequested {
            approval_id: "app_99".to_string(),
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
            call_id: "call_99".to_string(),
            tool_name: "shell.exec".to_string(),
            arguments: json!({"command": "rm -rf /"}),
            risk: "High".to_string(),
        });

        assert!(!state.is_busy);
        assert!(state.pending_approval.is_some());
        assert_eq!(
            state.pending_approval.as_ref().unwrap().approval_id,
            "app_99"
        );

        state.apply_event(AgentEvent::ApprovalResolved {
            approval_id: "app_99".to_string(),
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
            decision: "Denied".to_string(),
            reason: None,
        });

        assert!(state.pending_approval.is_none());
        assert_eq!(state.blocks.len(), 1);
        match &state.blocks[0] {
            UiBlock::SystemAlert { text, .. } => {
                assert_eq!(text, "Tool execution was Denied");
            }
            _ => panic!("Expected System Alert"),
        }
    }

    #[test]
    fn test_run_finished_deduplicates_repeated_assistant_answer() {
        let mut state = AppState::new();
        state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            provider: "test".to_string(),
            max_iterations: 10,
        });
        state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            iteration: 0,
            delta: "Echo: hi".to_string(),
        });

        state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            reason: arcan_core::protocol::RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("Echo: hi".to_string()),
        });
        state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            reason: arcan_core::protocol::RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("Echo: hi".to_string()),
        });

        let assistant_count = state
            .blocks
            .iter()
            .filter(|block| matches!(block, UiBlock::AssistantMessage { .. }))
            .count();
        assert_eq!(assistant_count, 1);
    }
}
