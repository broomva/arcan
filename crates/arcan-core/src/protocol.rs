use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// For tool result messages, the ID of the tool call this responds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub output: Value,
    pub state_patch: Option<StatePatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolResultSummary {
    pub call_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub output: Value,
}

impl From<&ToolResult> for ToolResultSummary {
    fn from(value: &ToolResult) -> Self {
        Self {
            call_id: value.call_id.clone(),
            tool_name: value.tool_name.clone(),
            output: value.output.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatePatchFormat {
    JsonPatch,
    MergePatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatePatchSource {
    Model,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StatePatch {
    pub format: StatePatchFormat,
    #[serde(default)]
    pub patch: Value,
    pub source: StatePatchSource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelStopReason {
    EndTurn,
    ToolUse,
    NeedsUser,
    MaxTokens,
    Safety,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelDirective {
    Text { delta: String },
    ToolCall { call: ToolCall },
    StatePatch { patch: StatePatch },
    FinalAnswer { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ModelTurn {
    pub directives: Vec<ModelDirective>,
    pub stop_reason: ModelStopReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStopReason {
    Completed,
    NeedsUser,
    BlockedByPolicy,
    BudgetExceeded,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "part_type", rename_all = "snake_case")]
pub enum AgentEvent {
    RunStarted {
        run_id: String,
        session_id: String,
        provider: String,
        max_iterations: u32,
    },
    IterationStarted {
        run_id: String,
        session_id: String,
        iteration: u32,
    },
    ModelOutput {
        run_id: String,
        session_id: String,
        iteration: u32,
        stop_reason: ModelStopReason,
        directive_count: usize,
    },
    TextDelta {
        run_id: String,
        session_id: String,
        iteration: u32,
        delta: String,
    },
    ToolCallRequested {
        run_id: String,
        session_id: String,
        iteration: u32,
        call: ToolCall,
    },
    ToolCallCompleted {
        run_id: String,
        session_id: String,
        iteration: u32,
        result: ToolResultSummary,
    },
    ToolCallFailed {
        run_id: String,
        session_id: String,
        iteration: u32,
        call_id: String,
        tool_name: String,
        error: String,
    },
    StatePatched {
        run_id: String,
        session_id: String,
        iteration: u32,
        patch: StatePatch,
        revision: u64,
    },
    RunErrored {
        run_id: String,
        session_id: String,
        error: String,
    },
    RunFinished {
        run_id: String,
        session_id: String,
        reason: RunStopReason,
        total_iterations: u32,
        final_answer: Option<String>,
    },
}

impl AgentEvent {
    pub fn as_sse_data(&self) -> Result<String, serde_json::Error> {
        let payload = serde_json::to_string(self)?;
        Ok(format!("data: {payload}\n\n"))
    }
}
