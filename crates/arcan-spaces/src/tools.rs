use crate::port::SpacesPort;
use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::Tool;
use arcan_core::runtime::ToolContext;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_CONTENT_LENGTH: usize = 4000;

fn validate_content(content: &str, tool_name: &str) -> Result<(), CoreError> {
    if content.is_empty() {
        return Err(CoreError::ToolExecution {
            tool_name: tool_name.to_string(),
            message: "content must not be empty".to_string(),
        });
    }
    if content.len() > MAX_CONTENT_LENGTH {
        return Err(CoreError::ToolExecution {
            tool_name: tool_name.to_string(),
            message: format!(
                "content exceeds maximum length of {MAX_CONTENT_LENGTH} characters ({} given)",
                content.len()
            ),
        });
    }
    Ok(())
}

fn get_required_str<'a>(
    input: &'a Value,
    key: &str,
    tool_name: &str,
) -> Result<&'a str, CoreError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::ToolExecution {
            tool_name: tool_name.to_string(),
            message: format!("missing required parameter: {key}"),
        })
}

fn get_required_u64(input: &Value, key: &str, tool_name: &str) -> Result<u64, CoreError> {
    input
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| CoreError::ToolExecution {
            tool_name: tool_name.to_string(),
            message: format!("missing required parameter: {key}"),
        })
}

// ── SpacesSendMessageTool ──

pub struct SpacesSendMessageTool {
    port: Arc<dyn SpacesPort>,
}

impl SpacesSendMessageTool {
    pub fn new(port: Arc<dyn SpacesPort>) -> Self {
        Self { port }
    }
}

impl Tool for SpacesSendMessageTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spaces_send_message".to_string(),
            description: "Send a message to a Spaces channel".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "channel_id": { "type": "integer", "description": "Target channel ID" },
                    "content": { "type": "string", "description": "Message content (max 4000 chars)", "maxLength": MAX_CONTENT_LENGTH },
                    "thread_id": { "type": "integer", "description": "Optional thread ID to reply in" },
                    "reply_to_id": { "type": "integer", "description": "Optional message ID to reply to" }
                },
                "required": ["channel_id", "content"]
            }),
            title: None,
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true,
                requires_confirmation: false,
            }),
            category: Some("spaces".to_string()),
            tags: vec!["spaces".to_string(), "messaging".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let channel_id = get_required_u64(&call.input, "channel_id", &call.tool_name)?;
        let content = get_required_str(&call.input, "content", &call.tool_name)?;
        validate_content(content, &call.tool_name)?;

        let thread_id = call.input.get("thread_id").and_then(Value::as_u64);
        let reply_to_id = call.input.get("reply_to_id").and_then(Value::as_u64);

        let msg = self
            .port
            .send_message(channel_id, content, thread_id, reply_to_id)
            .map_err(|e| e.into_core_error(&call.tool_name))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: serde_json::to_value(&msg).unwrap_or(json!({"status": "sent"})),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── SpacesListChannelsTool ──

pub struct SpacesListChannelsTool {
    port: Arc<dyn SpacesPort>,
}

impl SpacesListChannelsTool {
    pub fn new(port: Arc<dyn SpacesPort>) -> Self {
        Self { port }
    }
}

impl Tool for SpacesListChannelsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spaces_list_channels".to_string(),
            description: "List channels in a Spaces server".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "server_id": { "type": "integer", "description": "Server ID (default: 1)", "default": 1 }
                }
            }),
            title: None,
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: true,
                requires_confirmation: false,
            }),
            category: Some("spaces".to_string()),
            tags: vec!["spaces".to_string(), "messaging".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let server_id = call
            .input
            .get("server_id")
            .and_then(Value::as_u64)
            .unwrap_or(1);

        let channels = self
            .port
            .list_channels(server_id)
            .map_err(|e| e.into_core_error(&call.tool_name))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: serde_json::to_value(&channels).unwrap_or(json!([])),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── SpacesReadMessagesTool ──

pub struct SpacesReadMessagesTool {
    port: Arc<dyn SpacesPort>,
}

impl SpacesReadMessagesTool {
    pub fn new(port: Arc<dyn SpacesPort>) -> Self {
        Self { port }
    }
}

impl Tool for SpacesReadMessagesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spaces_read_messages".to_string(),
            description: "Read messages from a Spaces channel".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "channel_id": { "type": "integer", "description": "Channel to read from" },
                    "limit": { "type": "integer", "description": "Max messages to return (default 50, max 200)", "default": 50, "maximum": 200 },
                    "before_id": { "type": "integer", "description": "Only return messages before this ID (pagination)" }
                },
                "required": ["channel_id"]
            }),
            title: None,
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: true,
                requires_confirmation: false,
            }),
            category: Some("spaces".to_string()),
            tags: vec!["spaces".to_string(), "messaging".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let channel_id = get_required_u64(&call.input, "channel_id", &call.tool_name)?;
        let limit = call
            .input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .min(200) as u32;
        let before_id = call.input.get("before_id").and_then(Value::as_u64);

        let messages = self
            .port
            .read_messages(channel_id, limit, before_id)
            .map_err(|e| e.into_core_error(&call.tool_name))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: serde_json::to_value(&messages).unwrap_or(json!([])),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── SpacesSendDmTool ──

pub struct SpacesSendDmTool {
    port: Arc<dyn SpacesPort>,
}

impl SpacesSendDmTool {
    pub fn new(port: Arc<dyn SpacesPort>) -> Self {
        Self { port }
    }
}

impl Tool for SpacesSendDmTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spaces_send_dm".to_string(),
            description: "Send a direct message to another agent or user in Spaces".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "recipient": { "type": "string", "description": "Recipient identity (hex string)" },
                    "content": { "type": "string", "description": "Message content (max 4000 chars)", "maxLength": MAX_CONTENT_LENGTH }
                },
                "required": ["recipient", "content"]
            }),
            title: None,
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true,
                requires_confirmation: false,
            }),
            category: Some("spaces".to_string()),
            tags: vec!["spaces".to_string(), "messaging".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let recipient = get_required_str(&call.input, "recipient", &call.tool_name)?;
        if recipient.is_empty() {
            return Err(CoreError::ToolExecution {
                tool_name: call.tool_name.clone(),
                message: "recipient must not be empty".to_string(),
            });
        }

        let content = get_required_str(&call.input, "content", &call.tool_name)?;
        validate_content(content, &call.tool_name)?;

        let dm = self
            .port
            .send_dm(recipient, content)
            .map_err(|e| e.into_core_error(&call.tool_name))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: serde_json::to_value(&dm).unwrap_or(json!({"status": "sent"})),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockSpacesClient;

    fn mock_port() -> Arc<dyn SpacesPort> {
        Arc::new(MockSpacesClient::default_hub())
    }

    fn test_ctx() -> ToolContext {
        ToolContext {
            run_id: "run-1".to_string(),
            session_id: "session-1".to_string(),
            iteration: 1,
        }
    }

    // ── send_message tests ──

    #[test]
    fn send_message_success() {
        let port = mock_port();
        let tool = SpacesSendMessageTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_send_message".to_string(),
            input: json!({ "channel_id": 1, "content": "hello world" }),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["content"], "hello world");
    }

    #[test]
    fn send_message_empty_content_fails() {
        let port = mock_port();
        let tool = SpacesSendMessageTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_send_message".to_string(),
            input: json!({ "channel_id": 1, "content": "" }),
        };
        let result = tool.execute(&call, &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("content must not be empty"), "got: {err}");
    }

    #[test]
    fn send_message_content_too_long_fails() {
        let port = mock_port();
        let tool = SpacesSendMessageTool::new(port);
        let long_content = "x".repeat(MAX_CONTENT_LENGTH + 1);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_send_message".to_string(),
            input: json!({ "channel_id": 1, "content": long_content }),
        };
        let result = tool.execute(&call, &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum length"), "got: {err}");
    }

    // ── list_channels tests ──

    #[test]
    fn list_channels_returns_seeded() {
        let port = mock_port();
        let tool = SpacesListChannelsTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_list_channels".to_string(),
            input: json!({ "server_id": 1 }),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        let channels = result.output.as_array().unwrap();
        assert_eq!(channels.len(), 3);
        assert_eq!(channels[0]["name"], "general");
    }

    #[test]
    fn list_channels_default_server_id() {
        let port = mock_port();
        let tool = SpacesListChannelsTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_list_channels".to_string(),
            input: json!({}),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        let channels = result.output.as_array().unwrap();
        assert_eq!(channels.len(), 3);
    }

    // ── read_messages tests ──

    #[test]
    fn read_messages_empty() {
        let port = mock_port();
        let tool = SpacesReadMessagesTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_read_messages".to_string(),
            input: json!({ "channel_id": 1 }),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        let messages = result.output.as_array().unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn read_messages_with_limit() {
        let mock = Arc::new(MockSpacesClient::default_hub());
        // Send 3 messages
        mock.send_message(1, "msg1", None, None).unwrap();
        mock.send_message(1, "msg2", None, None).unwrap();
        mock.send_message(1, "msg3", None, None).unwrap();

        let port: Arc<dyn SpacesPort> = mock;
        let tool = SpacesReadMessagesTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_read_messages".to_string(),
            input: json!({ "channel_id": 1, "limit": 2 }),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        let messages = result.output.as_array().unwrap();
        assert_eq!(messages.len(), 2);
    }

    // ── send_dm tests ──

    #[test]
    fn send_dm_success() {
        let port = mock_port();
        let tool = SpacesSendDmTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_send_dm".to_string(),
            input: json!({ "recipient": "abcdef01", "content": "hi there" }),
        };
        let result = tool.execute(&call, &test_ctx()).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.output["content"], "hi there");
        assert_eq!(result.output["recipient"], "abcdef01");
    }

    #[test]
    fn send_dm_empty_recipient_fails() {
        let port = mock_port();
        let tool = SpacesSendDmTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_send_dm".to_string(),
            input: json!({ "recipient": "", "content": "hello" }),
        };
        let result = tool.execute(&call, &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("recipient must not be empty"), "got: {err}");
    }

    // ── definition tests ──

    #[test]
    fn all_definitions_have_correct_name_and_category() {
        let port = mock_port();

        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(SpacesSendMessageTool::new(port.clone())),
            Box::new(SpacesListChannelsTool::new(port.clone())),
            Box::new(SpacesReadMessagesTool::new(port.clone())),
            Box::new(SpacesSendDmTool::new(port)),
        ];

        let expected_names = [
            "spaces_send_message",
            "spaces_list_channels",
            "spaces_read_messages",
            "spaces_send_dm",
        ];

        for (tool, expected_name) in tools.iter().zip(expected_names.iter()) {
            let def = tool.definition();
            assert_eq!(def.name, *expected_name);
            assert_eq!(def.category.as_deref(), Some("spaces"));
            assert!(!def.description.is_empty());
            assert!(def.input_schema.is_object());
        }
    }

    #[test]
    fn port_error_surfaces_as_core_error() {
        let mock = Arc::new(MockSpacesClient::default_hub());
        *mock.force_error.lock().unwrap() = Some("connection lost".to_string());

        let port: Arc<dyn SpacesPort> = mock;
        let tool = SpacesListChannelsTool::new(port);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "spaces_list_channels".to_string(),
            input: json!({}),
        };
        let result = tool.execute(&call, &test_ctx());
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            CoreError::ToolExecution { tool_name, message } => {
                assert_eq!(tool_name, "spaces_list_channels");
                assert!(message.contains("connection lost"), "got: {message}");
            }
            other => panic!("expected ToolExecution, got: {other:?}"),
        }
    }
}
