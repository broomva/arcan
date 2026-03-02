use super::scroll::ScrollState;
use super::ui_block::{ApprovalRequest, ToolStatus, UiBlock};
use crate::focus::FocusTarget;
use arcan_core::protocol::AgentEvent;
use chrono::{DateTime, Utc};

/// Connection status for the daemon.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    #[default]
    Connecting,
}

/// Error flash message with timestamp for TTL-based expiry.
#[derive(Debug, Clone)]
pub struct ErrorFlash {
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

/// Pure logical representation of the UI state.
/// This model has zero dependency on ratatui or IO, making it easily testable.
#[derive(Debug)]
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

    /// Scroll state for the chat log
    pub scroll: ScrollState,

    /// Which widget currently has keyboard focus
    pub focus: FocusTarget,

    /// Connection status to the daemon
    pub connection_status: ConnectionStatus,

    /// Transient error shown in the status bar
    pub last_error: Option<ErrorFlash>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
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
            scroll: ScrollState::new(),
            focus: FocusTarget::InputBar,
            connection_status: ConnectionStatus::Connecting,
            last_error: None,
        }
    }

    /// Set an error flash that will be displayed in the status bar.
    pub fn flash_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(ErrorFlash {
            message: message.into(),
            timestamp: Utc::now(),
        });
    }

    /// Clear expired error flashes (older than `ttl`).
    pub fn clear_expired_errors(&mut self, ttl: chrono::Duration) {
        if let Some(ref flash) = self.last_error {
            if Utc::now() - flash.timestamp > ttl {
                self.last_error = None;
            }
        }
    }

    /// Process a raw AgentEvent from the daemon and mutate the view state.
    pub fn apply_event(&mut self, event: AgentEvent) {
        let now = Utc::now();
        match event {
            AgentEvent::RunStarted { .. } => {
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
                    self.blocks.push(UiBlock::AssistantMessage {
                        text,
                        timestamp: now,
                    });
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
                if let Some(UiBlock::ToolExecution {
                    status,
                    result: block_result,
                    ..
                }) = self.blocks.iter_mut().find(|b| {
                    if let UiBlock::ToolExecution { call_id, .. } = b {
                        call_id == &result.call_id
                    } else {
                        false
                    }
                }) {
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
                    if !self.last_assistant_message_matches(&text) {
                        self.blocks.push(UiBlock::AssistantMessage {
                            text,
                            timestamp: now,
                        });
                    }
                } else if let Some(ans) = final_answer {
                    if !self.last_assistant_message_matches(&ans) {
                        self.blocks.push(UiBlock::AssistantMessage {
                            text: ans,
                            timestamp: now,
                        });
                    }
                }
                self.is_busy = false;
            }
            AgentEvent::RunErrored { error, .. } => {
                self.blocks.push(UiBlock::SystemAlert {
                    text: format!("Run Error: {}", error),
                    timestamp: now,
                });
                self.is_busy = false;
            }
            AgentEvent::ApprovalRequested {
                approval_id,
                call_id,
                tool_name,
                arguments,
                risk,
                ..
            } => {
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
                    timestamp: now,
                });
                self.is_busy = true; // Loop resumes
            }
            _ => {
                // Ignore other events for pure UI view state
            }
        }
    }

    fn last_assistant_message_matches(&self, text: &str) -> bool {
        self.blocks
            .last()
            .and_then(|block| match block {
                UiBlock::AssistantMessage { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .is_some_and(|last| last == text)
    }
}
