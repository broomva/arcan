#[cfg(test)]
mod tests {
    use crate::models::{AppState, UiBlock, ToolStatus, ApprovalRequest};
    use arcan_core::protocol::AgentEvent;
    use arcan_core::tool::ToolCallRequest;
    use serde_json::json;
    use std::time::SystemTime;

    #[test]
    fn test_apply_tool_execution_flow() {
        let mut state = AppState::new();

        // Simulate start
        state.apply_event(AgentEvent::RunStarted);
        assert!(state.is_busy);

        // Simulate tool request
        let call_req = ToolCallRequest {
            call_id: "call_123".to_string(),
            tool_name: "fs.read".to_string(),
            input: json!({"path": "foo.txt"}),
            requested_capabilities: vec![],
        };
        state.apply_event(AgentEvent::ToolCallRequested { 
            call: call_req, 
            session_id: "s1".to_string(), 
            run_id: "r1".to_string() 
        });

        assert_eq!(state.blocks.len(), 1);
        match &state.blocks[0] {
            UiBlock::ToolExecution { call_id, status, .. } => {
                assert_eq!(call_id, "call_123");
                assert_eq!(*status, ToolStatus::Running);
            }
            _ => panic!("Expected ToolExecution block"),
        }

        // Simulate tool completion
        let result = arcan_core::tool::ToolCallResult {
            call_id: "call_123".to_string(),
            tool_name: "fs.read".to_string(),
            output: json!("file content"),
            is_error: false,
        };
        state.apply_event(AgentEvent::ToolCallCompleted { 
            result, 
            run_id: "r1".to_string(), 
            session_id: "s1".to_string() 
        });

        match &state.blocks[0] {
            UiBlock::ToolExecution { status, result: block_res, .. } => {
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
        assert_eq!(state.pending_approval.as_ref().unwrap().approval_id, "app_99");

        state.apply_event(AgentEvent::ApprovalResolved {
            approval_id: "app_99".to_string(),
            session_id: "s1".to_string(),
            run_id: "r1".to_string(),
            decision: "Denied".to_string(),
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
}
