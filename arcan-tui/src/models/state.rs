use super::scroll::ScrollState;
use super::ui_block::{ApprovalRequest, ToolStatus, UiBlock};
use crate::focus::FocusTarget;
use crate::widgets::spinner::Spinner;
use arcan_core::protocol::AgentEvent;
use chrono::{DateTime, Utc};
use serde_json::Value;

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

    /// Last known provider name (from RunStarted events).
    pub provider: Option<String>,

    /// Context pressure percentage (0–100), updated after each run.
    pub context_pressure_pct: f64,

    /// Autonomic ruling label (e.g., "Breathe", "Compress"), updated after each run.
    pub autonomic_ruling: Option<String>,

    /// Session cost remaining in USD, updated after each run.
    pub cost_remaining: Option<f64>,

    /// Animated spinner for the thinking indicator.
    pub spinner: Spinner,
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
            provider: None,
            context_pressure_pct: 0.0,
            autonomic_ruling: None,
            cost_remaining: None,
            spinner: Spinner::new(),
        }
    }

    /// Reset UI state for switching to a new session.
    ///
    /// Clears conversation blocks, streaming text, pending approval, scroll,
    /// error flash, and sets the new session ID. Preserves focus and connection status.
    pub fn reset_for_session_switch(&mut self, new_session_id: String) {
        self.session_id = Some(new_session_id);
        self.current_branch = "main".to_string();
        self.blocks.clear();
        self.streaming_text = None;
        self.input_buffer.clear();
        self.pending_approval = None;
        self.is_busy = false;
        self.scroll = super::scroll::ScrollState::new();
        self.last_error = None;
        self.provider = None;
        self.context_pressure_pct = 0.0;
        self.autonomic_ruling = None;
        self.cost_remaining = None;
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
        if let Some(ref flash) = self.last_error
            && Utc::now() - flash.timestamp > ttl
        {
            self.last_error = None;
        }
    }

    /// Process a raw AgentEvent from the daemon and mutate the view state.
    pub fn apply_event(&mut self, event: AgentEvent) {
        let now = Utc::now();
        match event {
            AgentEvent::RunStarted { provider, .. } => {
                self.is_busy = true;
                self.streaming_text = None;
                self.provider = Some(provider);
                self.spinner.new_verb();
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
                    self.push_assistant_or_error(text, now);
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
                    self.push_assistant_or_error(text, now);
                } else if let Some(ans) = final_answer {
                    self.push_assistant_or_error(ans, now);
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

    /// Push text as either an `AssistantMessage` or a `SystemAlert` if the text
    /// looks like a JSON API error response (OpenAI, Anthropic, etc.).
    fn push_assistant_or_error(&mut self, text: String, timestamp: DateTime<Utc>) {
        if self.last_assistant_message_matches(&text) {
            return; // deduplicate
        }

        if let Some(error_msg) = extract_api_error(&text) {
            let provider = self.provider.as_deref().unwrap_or("provider");
            self.blocks.push(UiBlock::SystemAlert {
                text: format!("API Error ({provider}): {error_msg}"),
                timestamp,
            });
        } else {
            self.blocks
                .push(UiBlock::AssistantMessage { text, timestamp });
        }
    }
}

/// Try to extract a human-readable error message from a JSON API error response.
///
/// Recognizes common patterns from OpenAI, Anthropic, and generic `{"error": ...}` responses.
fn extract_api_error(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('{') {
        return None;
    }

    let value: Value = serde_json::from_str(trimmed).ok()?;
    let obj = value.as_object()?;

    // Pattern 1: {"error": {"message": "...", "type": "...", "code": "..."}}
    // Used by OpenAI, many OpenAI-compatible APIs
    if let Some(error) = obj.get("error") {
        if let Some(msg) = error.get("message").and_then(Value::as_str) {
            let error_type = error.get("type").and_then(Value::as_str).unwrap_or("error");
            return Some(format!("{error_type}: {msg}"));
        }
        // {"error": "string message"}
        if let Some(msg) = error.as_str() {
            return Some(msg.to_string());
        }
    }

    // Pattern 2: {"type": "error", "error": {"type": "...", "message": "..."}}
    // Used by Anthropic
    if obj.get("type").and_then(Value::as_str) == Some("error")
        && let Some(error) = obj.get("error")
        && let Some(msg) = error.get("message").and_then(Value::as_str)
    {
        let error_type = error.get("type").and_then(Value::as_str).unwrap_or("error");
        return Some(format!("{error_type}: {msg}"));
    }

    // Pattern 3: {"message": "...", "status": 4xx/5xx}
    if let Some(msg) = obj.get("message").and_then(Value::as_str)
        && obj.get("status").and_then(Value::as_u64).unwrap_or(0) >= 400
    {
        return Some(msg.to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_error_sets_message() {
        let mut state = AppState::new();
        assert!(state.last_error.is_none());

        state.flash_error("something went wrong");
        assert!(state.last_error.is_some());
        assert_eq!(
            state.last_error.as_ref().unwrap().message,
            "something went wrong"
        );
    }

    #[test]
    fn clear_expired_errors_removes_old_flash() {
        let mut state = AppState::new();
        // Set an error flash with a timestamp in the past
        state.last_error = Some(ErrorFlash {
            message: "old error".to_string(),
            timestamp: Utc::now() - chrono::Duration::seconds(10),
        });

        // TTL of 5 seconds → should clear the 10s-old error
        state.clear_expired_errors(chrono::Duration::seconds(5));
        assert!(state.last_error.is_none());
    }

    #[test]
    fn clear_expired_errors_keeps_fresh_flash() {
        let mut state = AppState::new();
        state.flash_error("fresh error");

        // TTL of 5 seconds → just-created error should survive
        state.clear_expired_errors(chrono::Duration::seconds(5));
        assert!(state.last_error.is_some());
    }

    #[test]
    fn new_state_has_sensible_defaults() {
        let state = AppState::new();
        assert_eq!(state.current_branch, "main");
        assert!(state.blocks.is_empty());
        assert!(!state.is_busy);
        assert_eq!(state.connection_status, ConnectionStatus::Connecting);
        assert_eq!(state.focus, FocusTarget::InputBar);
    }

    #[test]
    fn reset_for_session_switch_clears_state() {
        let mut state = AppState::new();
        // Populate with some data
        state.session_id = Some("old-session".to_string());
        state.current_branch = "feature".to_string();
        state.blocks.push(UiBlock::SystemAlert {
            text: "hello".to_string(),
            timestamp: Utc::now(),
        });
        state.streaming_text = Some("partial...".to_string());
        state.is_busy = true;
        state.pending_approval = Some(ApprovalRequest {
            approval_id: "ap-1".to_string(),
            call_id: "c-1".to_string(),
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({}),
            risk_level: "high".to_string(),
        });
        state.flash_error("some error");

        // Reset
        state.reset_for_session_switch("new-session".to_string());

        assert_eq!(state.session_id, Some("new-session".to_string()));
        assert_eq!(state.current_branch, "main");
        assert!(state.blocks.is_empty());
        assert!(state.streaming_text.is_none());
        assert!(!state.is_busy);
        assert!(state.pending_approval.is_none());
        assert!(state.last_error.is_none());
    }
}
