use arcan_core::error::CoreError;
use arcan_core::protocol::{ModelDirective, ModelStopReason, ModelTurn};
use arcan_core::runtime::{Provider, ProviderRequest};

pub struct MockProvider;

impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock-provider"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        // Simple echo or fixed response
        // For verification, let's just return a text response.

        let last_msg = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        // If user says "ping", we say "pong".
        // If user says "file", we try to write a file (tool call).

        if last_msg.contains("file") {
            Ok(ModelTurn {
                directives: vec![
                    ModelDirective::Text {
                        delta: "I will write a file.".to_string(),
                    },
                    ModelDirective::ToolCall {
                        call: arcan_core::protocol::ToolCall {
                            call_id: "call_1".to_string(),
                            tool_name: "write_file".to_string(),
                            input: serde_json::json!({
                                "path": "test.txt",
                                "content": "Hello from Mock Provider"
                            }),
                        },
                    },
                ],
                stop_reason: ModelStopReason::ToolUse,
                usage: None,
            })
        } else {
            Ok(ModelTurn {
                directives: vec![ModelDirective::Text {
                    delta: format!("Echo: {}", last_msg),
                }],
                stop_reason: ModelStopReason::EndTurn,
                usage: None,
            })
        }
    }
}
