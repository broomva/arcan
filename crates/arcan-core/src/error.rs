use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("tool not found: {tool_name}")]
    ToolNotFound { tool_name: String },
    #[error("tool execution failed ({tool_name}): {message}")]
    ToolExecution { tool_name: String, message: String },
    #[error("middleware rejected request: {0}")]
    Middleware(String),
    #[error("state patch failed: {0}")]
    State(String),
    #[error("auth error: {0}")]
    Auth(String),
}
