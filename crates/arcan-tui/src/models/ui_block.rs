use chrono::{DateTime, Utc};

/// A single rendered block in the conversation view.
#[derive(Debug, Clone)]
pub enum UiBlock {
    HumanMessage {
        text: String,
        timestamp: DateTime<Utc>,
    },
    AssistantMessage {
        text: String,
        timestamp: DateTime<Utc>,
    },
    ToolExecution {
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        status: ToolStatus,
        result: Option<serde_json::Value>,
        timestamp: DateTime<Utc>,
    },
    SystemAlert {
        text: String,
        timestamp: DateTime<Utc>,
    },
}

/// Execution status for a tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    Running,
    Success,
    Error(String),
}

/// A pending approval request awaiting user decision.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub risk_level: String,
}
