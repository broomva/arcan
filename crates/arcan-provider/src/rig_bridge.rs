use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ModelTurn, Role, ToolCall,
};
use arcan_core::runtime::{Provider, ProviderRequest};
use rig::completion::{CompletionModel, CompletionRequest};
use rig::message::{AssistantContent, Message, ToolResultContent, UserContent};
use rig::OneOrMany;
use tokio::runtime::Handle;

/// A provider adapter that uses `rig-core` as the HTTP layer.
///
/// This bridges rig's `CompletionModel` to Arcan's `Provider` trait,
/// enabling any rig-supported model (Anthropic, OpenAI, etc.) to be
/// used as an Arcan provider.
pub struct RigProvider<M: CompletionModel> {
    model: M,
    model_name: String,
    runtime: Handle,
}

impl<M: CompletionModel> RigProvider<M> {
    pub fn new(model: M, model_name: String, runtime: Handle) -> Self {
        Self {
            model,
            model_name,
            runtime,
        }
    }
}

impl<M: CompletionModel + Send + Sync> Provider for RigProvider<M>
where
    M::Response: Send,
{
    fn name(&self) -> &str {
        &self.model_name
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        // 1. Separate system prompt from chat messages
        let (system_prompt, all_messages) = build_rig_messages(&request.messages)?;

        // 2. Build rig tool definitions
        let rig_tools: Vec<rig::completion::ToolDefinition> = request
            .tools
            .iter()
            .map(|t| rig::completion::ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            })
            .collect();

        // 3. Build CompletionRequest — prompt is the last message in chat_history
        let chat_history = OneOrMany::many(all_messages)
            .map_err(|_| CoreError::Provider("no messages provided".to_string()))?;

        let rig_request = CompletionRequest {
            preamble: system_prompt,
            chat_history,
            documents: vec![],
            tools: rig_tools,
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
        };

        // 4. Execute via rig (blocking bridge from sync to async)
        let model = &self.model;
        let response = self
            .runtime
            .block_on(async { model.completion(rig_request).await })
            .map_err(|e| CoreError::Provider(format!("rig completion failed: {}", e)))?;

        // 5. Convert rig response to ModelTurn
        parse_rig_response(response)
    }
}

/// Separate Arcan messages into rig's system prompt and chat history.
/// The last user message is included in the history as the prompt (rig convention).
fn build_rig_messages(
    messages: &[ChatMessage],
) -> Result<(Option<String>, Vec<Message>), CoreError> {
    let mut system_prompt = None;
    let mut history: Vec<Message> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                system_prompt = Some(msg.content.clone());
            }
            Role::User => {
                history.push(Message::User {
                    content: OneOrMany::one(UserContent::text(&msg.content)),
                });
            }
            Role::Assistant => {
                history.push(Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::text(&msg.content)),
                });
            }
            Role::Tool => {
                let tool_call_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                history.push(Message::User {
                    content: OneOrMany::one(UserContent::tool_result(
                        tool_call_id,
                        OneOrMany::one(ToolResultContent::text(&msg.content)),
                    )),
                });
            }
        }
    }

    // Ensure at least one message exists (rig requires OneOrMany)
    if history.is_empty() {
        history.push(Message::User {
            content: OneOrMany::one(UserContent::text("Continue.")),
        });
    }

    Ok((system_prompt, history))
}

/// Parse a rig `CompletionResponse` into Arcan's `ModelTurn`.
/// The response `choice` is `OneOrMany<AssistantContent>` which may contain
/// text, tool calls, reasoning, or images.
fn parse_rig_response<T>(
    response: rig::completion::CompletionResponse<T>,
) -> Result<ModelTurn, CoreError> {
    let mut directives = Vec::new();
    let mut stop_reason = ModelStopReason::EndTurn;

    for content in response.choice.into_iter() {
        match content {
            AssistantContent::Text(text) => {
                directives.push(ModelDirective::Text { delta: text.text });
            }
            AssistantContent::ToolCall(tc) => {
                stop_reason = ModelStopReason::ToolUse;
                directives.push(ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: tc.id.clone(),
                        tool_name: tc.function.name.clone(),
                        input: tc.function.arguments.clone(),
                    },
                });
            }
            _ => {
                // Reasoning, Image, etc. — not mapped to Arcan directives yet
            }
        }
    }

    Ok(ModelTurn {
        directives,
        stop_reason,
        usage: None, // TODO: map rig usage when rig exposes it in CompletionResponse
    })
}

/// Create an Anthropic provider using rig-core.
///
/// Usage:
/// ```rust,ignore
/// let runtime = tokio::runtime::Handle::current();
/// let provider = anthropic_rig_provider("your-api-key", "claude-sonnet-4-5-20250929", runtime)?;
/// ```
pub fn anthropic_rig_provider(
    api_key: &str,
    model_id: &str,
    runtime: Handle,
) -> Result<RigProvider<rig::providers::anthropic::completion::CompletionModel>, CoreError> {
    use rig::client::CompletionClient;
    use rig::providers::anthropic;

    let client = anthropic::Client::new(api_key)
        .map_err(|e| CoreError::Provider(format!("failed to build Anthropic client: {}", e)))?;

    let model = client.completion_model(model_id);
    Ok(RigProvider::new(model, model_id.to_string(), runtime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::ChatMessage;
    use serde_json::json;

    #[test]
    fn build_messages_extracts_system() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
        ];

        let (system, history) = build_rig_messages(&messages).unwrap();
        assert_eq!(system, Some("You are helpful.".to_string()));
        // History contains just the user message (which is also the prompt)
        assert_eq!(history.len(), 1);
        assert!(matches!(&history[0], Message::User { .. }));
    }

    #[test]
    fn build_messages_with_conversation() {
        let messages = vec![
            ChatMessage::system("Be brief."),
            ChatMessage::user("What is 2+2?"),
            ChatMessage::assistant("4"),
            ChatMessage::user("And 3+3?"),
        ];

        let (system, history) = build_rig_messages(&messages).unwrap();
        assert_eq!(system, Some("Be brief.".to_string()));
        // 3 history messages: user "What is 2+2?" + assistant "4" + user "And 3+3?"
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn build_messages_with_tool_result() {
        let messages = vec![
            ChatMessage::user("Read test.rs"),
            ChatMessage {
                role: Role::Tool,
                content: "file contents here".to_string(),
                tool_call_id: Some("call_123".to_string()),
            },
            ChatMessage::user("Now edit it"),
        ];

        let (system, history) = build_rig_messages(&messages).unwrap();
        assert!(system.is_none());
        // 3 messages: user "Read test.rs" + tool result + user "Now edit it"
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn build_messages_empty_adds_default() {
        let messages: Vec<ChatMessage> = vec![];
        let (system, history) = build_rig_messages(&messages).unwrap();
        assert!(system.is_none());
        assert_eq!(history.len(), 1);
        assert!(matches!(&history[0], Message::User { .. }));
    }

    #[test]
    fn parse_text_response() {
        let response = rig::completion::CompletionResponse {
            choice: OneOrMany::one(AssistantContent::text("Hello!")),
            usage: rig::completion::Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cached_input_tokens: 0,
            },
            raw_response: json!({}),
        };

        let turn = parse_rig_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::EndTurn);
        assert_eq!(turn.directives.len(), 1);
        assert!(matches!(&turn.directives[0], ModelDirective::Text { delta } if delta == "Hello!"));
    }

    #[test]
    fn parse_tool_call_response() {
        let response = rig::completion::CompletionResponse {
            choice: OneOrMany::one(AssistantContent::tool_call(
                "call_123",
                "read_file",
                json!({"path": "test.rs"}),
            )),
            usage: rig::completion::Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cached_input_tokens: 0,
            },
            raw_response: json!({}),
        };

        let turn = parse_rig_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::ToolUse);
        assert_eq!(turn.directives.len(), 1);
        assert!(
            matches!(&turn.directives[0], ModelDirective::ToolCall { call } if call.tool_name == "read_file")
        );
    }
}
