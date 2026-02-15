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

/// MCP-compatible behavioral annotations for tools.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
pub struct ToolAnnotations {
    /// Tool does not modify its environment.
    #[serde(default)]
    pub read_only: bool,
    /// Tool may perform destructive updates.
    #[serde(default)]
    pub destructive: bool,
    /// Repeated calls with same args produce same result.
    #[serde(default)]
    pub idempotent: bool,
    /// Tool interacts with external entities (network, APIs).
    #[serde(default)]
    pub open_world: bool,
    /// Tool requires user confirmation before execution.
    #[serde(default)]
    pub requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,

    // ── MCP-aligned fields (all optional, backward-compatible) ──
    /// Human-readable display name (MCP: title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// JSON Schema for structured output (MCP: outputSchema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Behavioral hints (MCP: annotations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,

    // ── Arcan extensions ──
    /// Tool category for grouping ("filesystem", "code", "shell", "mcp").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Tags for filtering and matching (skills.sh compatible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Maximum execution timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
}

/// Typed content block in a tool result (MCP-compatible).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text { text: String },
    Image { data: String, mime_type: String },
    Json { value: Value },
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
    /// MCP-style typed content blocks (optional, alongside output for compat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ToolContent>>,
    /// Whether this result represents an error (MCP: isError).
    #[serde(default)]
    pub is_error: bool,
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
    /// Token usage for this turn (if reported by the provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct TokenUsage {
    /// Tokens in the input/prompt.
    #[serde(default)]
    pub input_tokens: u64,
    /// Tokens in the output/completion.
    #[serde(default)]
    pub output_tokens: u64,
    /// Tokens from cache reads (Anthropic-specific).
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Tokens written to cache (Anthropic-specific).
    #[serde(default)]
    pub cache_creation_tokens: u64,
}

impl TokenUsage {
    /// Accumulate another usage into this one.
    pub fn accumulate(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
    }

    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStopReason {
    Completed,
    NeedsUser,
    BlockedByPolicy,
    BudgetExceeded,
    Cancelled,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
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
    ContextCompacted {
        run_id: String,
        session_id: String,
        iteration: u32,
        dropped_count: usize,
        tokens_before: usize,
        tokens_after: usize,
    },
    ApprovalRequested {
        run_id: String,
        session_id: String,
        approval_id: String,
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        /// Risk level as string ("low"/"medium"/"high"/"critical") to keep arcan-core lago-free.
        risk: String,
    },
    ApprovalResolved {
        run_id: String,
        session_id: String,
        approval_id: String,
        /// Decision as string ("approved"/"denied"/"timeout").
        decision: String,
        reason: Option<String>,
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
