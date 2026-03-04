use arcan_core::error::CoreError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpacesBridgeError {
    #[error("spaces connection error: {0}")]
    Connection(String),

    #[error("channel not found: {0}")]
    ChannelNotFound(String),

    #[error("server not found: {0}")]
    ServerNotFound(String),

    #[error("recipient not found: {0}")]
    RecipientNotFound(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("reducer failed: {0}")]
    ReducerFailed(String),

    #[error("HTTP request failed: {0}")]
    Http(String),

    #[error("failed to parse response: {0}")]
    ParseError(String),

    #[error("authentication error: {0}")]
    AuthError(String),
}

impl SpacesBridgeError {
    pub fn into_core_error(self, tool_name: &str) -> CoreError {
        CoreError::ToolExecution {
            tool_name: tool_name.to_string(),
            message: self.to_string(),
        }
    }
}
