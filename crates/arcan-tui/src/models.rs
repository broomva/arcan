use arcan_core::protocol::AgentEvent;
use chrono::{DateTime, Utc};

/// Pure logical representation of the UI state.
/// This model has zero dependency on ratatui or IO, making it easily testable.
#[derive(Debug, Default)]
pub struct AppState {
    pub session_id: Option<String>,
    pub current_branch: String,
    
    /// The conversation history (messages, tool executions, errors)
    pub blocks: Vec<UiBlock>,
    
    /// Pending text delta buffer (assistant is typing)
    pub streaming_text: Option<String>,
    
    /// User input string
    pub input_buffer: String,
    
    /// True if the UI is blocked waiting for user approval of a policy intervention
    pub pending_approval: Option<ApprovalRequest>,
    
    /// Tracks if we're currently fetching from daemon
    pub is_busy: bool,
}

#[derive(Debug, Clone)]
pub enum UiBlock {
    HumanMessage { text: String, timestamp: DateTime<Utc> },
    AssistantMessage { text: String, timestamp: DateTime<Utc> },
    ToolExecution { 
        call_id: String,
        tool_name: String, 
        arguments: serde_json::Value,
        status: ToolStatus,
        result: Option<serde_json::Value>,
        timestamp: DateTime<Utc>
    },
    SystemAlert { text: String, timestamp: DateTime<Utc> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolStatus {
    Running,
    Success,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub approval_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub risk_level: String,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            session_id: None,
            current_branch: "main".to_string(),
            blocks: Vec::new(),
            streaming_text: None,
            input_buffer: String::new(),
            pending_approval: None,
            is_busy: false,
        }
    }

    /// Process a raw AgentEvent from the daemon and mutate the view state.
    pub fn apply_event(&mut self, event: AgentEvent) {
        let now = Utc::now();
        match event {
            AgentEvent::RunStarted => {
                self.is_busy = true;
                self.streaming_text = None;
            }
            AgentEvent::TextDelta { delta, .. } => {
                if let Some(mut text) = self.streaming_text.take() {
                    text.push_str(&delta);
                    self.streaming_text = Some(text);
                } else {
                    self.streaming_text = Some(delta);
                }
            }
            AgentEvent::ToolCallRequested { call, .. } => {
                // Flush streaming text if it exists (model stopped reasoning, started acting)
                if let Some(text) = self.streaming_text.take() {
                    self.blocks.push(UiBlock::AssistantMessage { text, timestamp: now });
                }

                self.blocks.push(UiBlock::ToolExecution {
                    call_id: call.call_id,
                    tool_name: call.tool_name,
                    arguments: call.input,
                    status: ToolStatus::Running,
                    result: None,
                    timestamp: now,
                });
            }
            AgentEvent::ToolCallCompleted { result, .. } => {
                // Find and update the running tool block
                if let Some(UiBlock::ToolExecution { status, result: block_result, .. }) = 
                    self.blocks.iter_mut().find(|b| {
                        if let UiBlock::ToolExecution { call_id, .. } = b {
                            call_id == &result.call_id
                        } else {
                            false
                        }
                    }) 
                {
                    *status = ToolStatus::Success;
                    *block_result = Some(result.output);
                }
            }
            AgentEvent::ToolCallFailed { call_id, error, .. } => {
                if let Some(UiBlock::ToolExecution { status, .. }) = 
                    self.blocks.iter_mut().find(|b| {
                        if let UiBlock::ToolExecution { call_id: id, .. } = b {
                            id == &call_id
                        } else {
                            false
                        }
                    }) 
                {
                    *status = ToolStatus::Error(error);
                }
            }
            AgentEvent::RunFinished { final_answer, .. } => {
                if let Some(text) = self.streaming_text.take() {
                    self.blocks.push(UiBlock::AssistantMessage { text, timestamp: now });
                } else if let Some(ans) = final_answer {
                    self.blocks.push(UiBlock::AssistantMessage { text: ans, timestamp: now });
                }
                self.is_busy = false;
            }
            AgentEvent::RunErrored { error, .. } => {
                self.blocks.push(UiBlock::SystemAlert { text: format!("Run Error: {}", error), timestamp: now });
                self.is_busy = false;
            }
            AgentEvent::ApprovalRequested { approval_id, call_id, tool_name, arguments, risk, .. } => {
                self.is_busy = false;
                self.pending_approval = Some(ApprovalRequest {
                    approval_id,
                    call_id,
                    tool_name,
                    arguments,
                    risk_level: risk,
                });
            }
            AgentEvent::ApprovalResolved { decision, .. } => {
                self.pending_approval = None;
                self.blocks.push(UiBlock::SystemAlert { 
                    text: format!("Tool execution was {}", decision), 
                    timestamp: now 
                });
                self.is_busy = true; // Loop resumes
            }
            _ => {
                // Ignore other events for pure UI view state
            }
        }
    }
}
