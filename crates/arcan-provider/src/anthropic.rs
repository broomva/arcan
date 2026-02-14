use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ModelTurn, Role, TokenUsage, ToolCall,
    ToolDefinition,
};
use arcan_core::runtime::{Provider, ProviderRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Configuration for the Anthropic provider.
#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub base_url: String,
}

impl AnthropicConfig {
    pub fn from_env() -> Result<Self, CoreError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            CoreError::Provider("ANTHROPIC_API_KEY environment variable not set".to_string())
        })?;

        let model = std::env::var("ANTHROPIC_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());

        let max_tokens = std::env::var("ANTHROPIC_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096);

        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

        Ok(Self {
            api_key,
            model,
            max_tokens,
            base_url,
        })
    }
}

/// Anthropic Messages API provider implementing the `Provider` trait.
pub struct AnthropicProvider {
    config: AnthropicConfig,
    client: reqwest::blocking::Client,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Self {
        let client = reqwest::blocking::Client::new();
        Self { config, client }
    }

    fn build_messages(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<ApiMessage>) {
        let mut system_prompt = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    system_prompt = Some(msg.content.clone());
                }
                Role::User => {
                    api_messages.push(ApiMessage {
                        role: "user".to_string(),
                        content: ApiContent::Text(msg.content.clone()),
                    });
                }
                Role::Assistant => {
                    api_messages.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: ApiContent::Text(msg.content.clone()),
                    });
                }
                Role::Tool => {
                    let tool_use_id = msg
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    api_messages.push(ApiMessage {
                        role: "user".to_string(),
                        content: ApiContent::Blocks(vec![ContentBlock::ToolResult {
                            tool_use_id,
                            content: msg.content.clone(),
                        }]),
                    });
                }
            }
        }

        (system_prompt, api_messages)
    }

    fn convert_tools(&self, tools: &[ToolDefinition]) -> Vec<ApiTool> {
        tools
            .iter()
            .map(|t| ApiTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect()
    }

    fn parse_response(&self, response: ApiResponse) -> Result<ModelTurn, CoreError> {
        let mut directives = Vec::new();

        for block in &response.content {
            match block {
                ResponseBlock::Text { text } => {
                    directives.push(ModelDirective::Text {
                        delta: text.clone(),
                    });
                }
                ResponseBlock::ToolUse { id, name, input } => {
                    directives.push(ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: id.clone(),
                            tool_name: name.clone(),
                            input: input.clone(),
                        },
                    });
                }
            }
        }

        let stop_reason = match response.stop_reason.as_deref() {
            Some("end_turn") => ModelStopReason::EndTurn,
            Some("tool_use") => ModelStopReason::ToolUse,
            Some("max_tokens") => ModelStopReason::MaxTokens,
            _ => ModelStopReason::Unknown,
        };

        let usage = response.usage.map(|u| TokenUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_tokens: u.cache_read_input_tokens,
            cache_creation_tokens: u.cache_creation_input_tokens,
        });

        Ok(ModelTurn {
            directives,
            stop_reason,
            usage,
        })
    }
}

impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.config.model
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let (system_prompt, api_messages) = self.build_messages(&request.messages);
        let api_tools = self.convert_tools(&request.tools);

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": self.config.max_tokens,
            "messages": api_messages,
        });

        if let Some(system) = system_prompt {
            body["system"] = json!(system);
        }

        if !api_tools.is_empty() {
            body["tools"] = serde_json::to_value(&api_tools)
                .map_err(|e| CoreError::Provider(format!("failed to serialize tools: {e}")))?;
        }

        let url = format!("{}/v1/messages", self.config.base_url);

        let response = self
            .client
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| CoreError::Provider(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        let response_text = response
            .text()
            .map_err(|e| CoreError::Provider(format!("failed to read response body: {e}")))?;

        if !status.is_success() {
            return Err(CoreError::Provider(format!(
                "Anthropic API returned {status}: {response_text}"
            )));
        }

        let api_response: ApiResponse = serde_json::from_str(&response_text)
            .map_err(|e| CoreError::Provider(format!("failed to parse response: {e}")))?;

        self.parse_response(api_response)
    }
}

// ─── Anthropic API types ───────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: ApiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ResponseBlock>,
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ResponseBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_messages_with_system_prompt() {
        let config = AnthropicConfig {
            api_key: "test".to_string(),
            model: "test-model".to_string(),
            max_tokens: 1024,
            base_url: "http://localhost".to_string(),
        };
        let provider = AnthropicProvider::new(config);

        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];

        let (system, api_msgs) = provider.build_messages(&messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(api_msgs.len(), 2);
    }

    #[test]
    fn parses_text_response() {
        let config = AnthropicConfig {
            api_key: "test".to_string(),
            model: "test-model".to_string(),
            max_tokens: 1024,
            base_url: "http://localhost".to_string(),
        };
        let provider = AnthropicProvider::new(config);

        let response = ApiResponse {
            content: vec![ResponseBlock::Text {
                text: "Hello!".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: None,
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::EndTurn);
        assert_eq!(turn.directives.len(), 1);
        assert!(matches!(&turn.directives[0], ModelDirective::Text { delta } if delta == "Hello!"));
    }

    #[test]
    fn parses_tool_use_response() {
        let config = AnthropicConfig {
            api_key: "test".to_string(),
            model: "test-model".to_string(),
            max_tokens: 1024,
            base_url: "http://localhost".to_string(),
        };
        let provider = AnthropicProvider::new(config);

        let response = ApiResponse {
            content: vec![
                ResponseBlock::Text {
                    text: "Let me read that file.".to_string(),
                },
                ResponseBlock::ToolUse {
                    id: "toolu_123".to_string(),
                    name: "read_file".to_string(),
                    input: json!({"path": "test.rs"}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: Some(ApiUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::ToolUse);
        assert_eq!(turn.directives.len(), 2);
        assert!(
            matches!(&turn.directives[1], ModelDirective::ToolCall { call } if call.tool_name == "read_file")
        );
        // Verify usage is parsed
        let usage = turn.usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
    }
}
