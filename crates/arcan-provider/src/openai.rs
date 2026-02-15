use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ModelTurn, Role, TokenUsage, ToolCall,
    ToolDefinition,
};
use arcan_core::runtime::{Provider, ProviderRequest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Configuration for an OpenAI-compatible provider.
///
/// Works with: OpenAI, Ollama, Together, Groq, LM Studio, vLLM, and any
/// server that implements the OpenAI chat completions API.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// API key (empty string for local servers like Ollama).
    pub api_key: String,
    /// Model name (e.g., "gpt-4o", "llama3.1", "qwen2.5").
    pub model: String,
    /// Maximum tokens for model response.
    pub max_tokens: u32,
    /// Base URL for the API (e.g., "https://api.openai.com", "http://localhost:11434").
    pub base_url: String,
    /// Provider name for display/logging.
    pub provider_name: String,
}

impl OpenAiConfig {
    /// Create config for OpenAI from environment variables.
    pub fn openai_from_env() -> Result<Self, CoreError> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            CoreError::Provider("OPENAI_API_KEY environment variable not set".to_string())
        })?;

        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
        let max_tokens = std::env::var("OPENAI_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096);
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com".to_string());

        Ok(Self {
            api_key,
            model,
            max_tokens,
            base_url,
            provider_name: "openai".to_string(),
        })
    }

    /// Create config for Ollama from environment variables.
    /// Defaults to localhost:11434 with no API key.
    pub fn ollama_from_env() -> Result<Self, CoreError> {
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".to_string());
        let max_tokens = std::env::var("OLLAMA_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096);
        let base_url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());

        Ok(Self {
            api_key: String::new(),
            model,
            max_tokens,
            base_url,
            provider_name: "ollama".to_string(),
        })
    }
}

/// Provider implementation for any OpenAI-compatible chat completions API.
///
/// Supports tool calling via the `tools` parameter with `function` type.
/// Works with OpenAI, Ollama, Together, Groq, LM Studio, vLLM, etc.
pub struct OpenAiCompatibleProvider {
    config: OpenAiConfig,
    client: reqwest::blocking::Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(config: OpenAiConfig) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self { config, client }
    }

    fn build_messages(&self, messages: &[ChatMessage]) -> Vec<ApiMessage> {
        messages
            .iter()
            .map(|msg| {
                let (role, tool_call_id) = match msg.role {
                    Role::System => ("system", None),
                    Role::User => ("user", None),
                    Role::Assistant => ("assistant", None),
                    Role::Tool => ("tool", msg.tool_call_id.clone()),
                };
                ApiMessage {
                    role: role.to_string(),
                    content: Some(msg.content.clone()),
                    tool_call_id,
                    tool_calls: None,
                }
            })
            .collect()
    }

    fn convert_tools(&self, tools: &[ToolDefinition]) -> Vec<ApiTool> {
        tools
            .iter()
            .map(|t| ApiTool {
                r#type: "function".to_string(),
                function: ApiFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.clone(),
                    strict: Some(true),
                },
            })
            .collect()
    }

    fn parse_response(&self, response: ApiResponse) -> Result<ModelTurn, CoreError> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Provider("OpenAI response had no choices".to_string()))?;

        let mut directives = Vec::new();

        // Handle text content
        if let Some(content) = &choice.message.content {
            if !content.is_empty() {
                directives.push(ModelDirective::Text {
                    delta: content.clone(),
                });
            }
        }

        // Handle tool calls
        if let Some(tool_calls) = &choice.message.tool_calls {
            for tc in tool_calls {
                let input: Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_else(|_| json!({}));
                directives.push(ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: tc.id.clone(),
                        tool_name: tc.function.name.clone(),
                        input,
                    },
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => ModelStopReason::EndTurn,
            Some("tool_calls") => ModelStopReason::ToolUse,
            Some("length") => ModelStopReason::MaxTokens,
            Some("content_filter") => ModelStopReason::Safety,
            _ => ModelStopReason::Unknown,
        };

        let usage = response.usage.map(|u| TokenUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        });

        Ok(ModelTurn {
            directives,
            stop_reason,
            usage,
        })
    }

    /// Execute with retry logic for transient errors (429, 5xx).
    fn execute_with_retry(
        &self,
        body: &Value,
        url: &str,
        max_retries: u32,
    ) -> Result<String, CoreError> {
        let mut last_error = None;
        let base_delay = std::time::Duration::from_millis(200);

        for attempt in 0..=max_retries {
            if attempt > 0 {
                let delay = base_delay * 2u32.pow(attempt - 1);
                std::thread::sleep(delay);
            }

            let mut request = self
                .client
                .post(url)
                .header("content-type", "application/json");

            // Only add Authorization header if API key is non-empty
            if !self.config.api_key.is_empty() {
                request =
                    request.header("authorization", format!("Bearer {}", self.config.api_key));
            }

            let response = match request.json(body).send() {
                Ok(resp) => resp,
                Err(e) if e.is_timeout() && attempt < max_retries => {
                    last_error = Some(format!("timeout: {e}"));
                    continue;
                }
                Err(e) => return Err(CoreError::Provider(format!("HTTP request failed: {e}"))),
            };

            let status = response.status();
            let response_text = response
                .text()
                .map_err(|e| CoreError::Provider(format!("failed to read response: {e}")))?;

            // Retry on transient errors
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < max_retries {
                last_error = Some(format!("{status}: {response_text}"));
                continue;
            }

            if !status.is_success() {
                return Err(CoreError::Provider(format!(
                    "{} API returned {status}: {response_text}",
                    self.config.provider_name
                )));
            }

            return Ok(response_text);
        }

        Err(CoreError::Provider(format!(
            "{} API failed after {} retries: {}",
            self.config.provider_name,
            max_retries,
            last_error.unwrap_or_default()
        )))
    }
}

impl Provider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.config.model
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let api_messages = self.build_messages(&request.messages);
        let api_tools = self.convert_tools(&request.tools);

        let mut body = json!({
            "model": self.config.model,
            "messages": api_messages,
            "max_tokens": self.config.max_tokens,
        });

        if !api_tools.is_empty() {
            body["tools"] = serde_json::to_value(&api_tools)
                .map_err(|e| CoreError::Provider(format!("failed to serialize tools: {e}")))?;
        }

        let url = format!("{}/v1/chat/completions", self.config.base_url);
        let response_text = self.execute_with_retry(&body, &url, 3)?;

        let api_response: ApiResponse = serde_json::from_str(&response_text).map_err(|e| {
            CoreError::Provider(format!(
                "failed to parse {} response: {e}\nBody: {}",
                self.config.provider_name,
                &response_text[..response_text.len().min(500)]
            ))
        })?;

        self.parse_response(api_response)
    }
}

// ─── OpenAI-compatible API types ────────────────────────────────

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ApiToolCallRef>>,
}

#[derive(Debug, Serialize)]
struct ApiTool {
    r#type: String,
    function: ApiFunction,
}

#[derive(Debug, Serialize)]
struct ApiFunction {
    name: String,
    description: String,
    parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiToolCallRef {
    id: String,
    r#type: String,
    function: ApiToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct ApiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    choices: Vec<ApiChoice>,
    #[serde(default)]
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
struct ApiChoice {
    message: ApiChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ApiToolCallRef>>,
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_config() -> OpenAiConfig {
        OpenAiConfig {
            api_key: "test-key".to_string(),
            model: "gpt-4o".to_string(),
            max_tokens: 4096,
            base_url: "http://localhost:8080".to_string(),
            provider_name: "test".to_string(),
        }
    }

    #[test]
    fn builds_messages_correctly() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];

        let api_msgs = provider.build_messages(&messages);
        assert_eq!(api_msgs.len(), 3);
        assert_eq!(api_msgs[0].role, "system");
        assert_eq!(api_msgs[1].role, "user");
        assert_eq!(api_msgs[2].role, "assistant");
    }

    #[test]
    fn builds_tool_messages_with_call_id() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let messages = vec![ChatMessage::tool_result("call-1", "result data")];

        let api_msgs = provider.build_messages(&messages);
        assert_eq!(api_msgs.len(), 1);
        assert_eq!(api_msgs[0].role, "tool");
        assert_eq!(api_msgs[0].tool_call_id, Some("call-1".to_string()));
    }

    #[test]
    fn converts_tools_to_api_format() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            title: None,
            output_schema: None,
            annotations: None,
            category: None,
            tags: vec![],
            timeout_secs: None,
        }];

        let api_tools = provider.convert_tools(&tools);
        assert_eq!(api_tools.len(), 1);
        assert_eq!(api_tools[0].r#type, "function");
        assert_eq!(api_tools[0].function.name, "read_file");
        assert_eq!(api_tools[0].function.strict, Some(true));
    }

    #[test]
    fn parses_text_response() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let response = ApiResponse {
            choices: vec![ApiChoice {
                message: ApiChoiceMessage {
                    content: Some("Hello!".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: Some(ApiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            }),
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::EndTurn);
        assert_eq!(turn.directives.len(), 1);
        assert!(matches!(&turn.directives[0], ModelDirective::Text { delta } if delta == "Hello!"));
        let usage = turn.usage.unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 5);
    }

    #[test]
    fn parses_tool_call_response() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let response = ApiResponse {
            choices: vec![ApiChoice {
                message: ApiChoiceMessage {
                    content: Some("Let me read that.".to_string()),
                    tool_calls: Some(vec![ApiToolCallRef {
                        id: "call_abc123".to_string(),
                        r#type: "function".to_string(),
                        function: ApiToolCallFunction {
                            name: "read_file".to_string(),
                            arguments: r#"{"path":"test.rs"}"#.to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: None,
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::ToolUse);
        assert_eq!(turn.directives.len(), 2); // text + tool call
        assert!(matches!(&turn.directives[0], ModelDirective::Text { .. }));
        assert!(
            matches!(&turn.directives[1], ModelDirective::ToolCall { call } if call.tool_name == "read_file")
        );
    }

    #[test]
    fn parses_empty_choices_returns_error() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let response = ApiResponse {
            choices: vec![],
            usage: None,
        };

        let result = provider.parse_response(response);
        assert!(result.is_err());
    }

    #[test]
    fn parses_max_tokens_stop_reason() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let response = ApiResponse {
            choices: vec![ApiChoice {
                message: ApiChoiceMessage {
                    content: Some("truncated...".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("length".to_string()),
            }],
            usage: None,
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::MaxTokens);
    }

    #[test]
    fn parses_content_filter_stop_reason() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        let response = ApiResponse {
            choices: vec![ApiChoice {
                message: ApiChoiceMessage {
                    content: None,
                    tool_calls: None,
                },
                finish_reason: Some("content_filter".to_string()),
            }],
            usage: None,
        };

        let turn = provider.parse_response(response).unwrap();
        assert_eq!(turn.stop_reason, ModelStopReason::Safety);
    }

    #[test]
    fn ollama_config_defaults() {
        // This just tests the config structure, not actual env vars
        let config = OpenAiConfig {
            api_key: String::new(),
            model: "llama3.2".to_string(),
            max_tokens: 4096,
            base_url: "http://localhost:11434".to_string(),
            provider_name: "ollama".to_string(),
        };
        assert!(config.api_key.is_empty());
        assert_eq!(config.base_url, "http://localhost:11434");
    }

    #[test]
    fn provider_name_returns_model() {
        let provider = OpenAiCompatibleProvider::new(test_config());
        assert_eq!(provider.name(), "gpt-4o");
    }

    #[test]
    fn api_message_serialization() {
        let msg = ApiMessage {
            role: "user".to_string(),
            content: Some("hello".to_string()),
            tool_call_id: None,
            tool_calls: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"content\":\"hello\""));
        // tool_call_id should be skipped
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn api_response_deserialization() {
        let json = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello!"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 12,
                "total_tokens": 21
            }
        }"#;
        let response: ApiResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.choices.len(), 1);
        assert_eq!(
            response.choices[0].message.content,
            Some("Hello!".to_string())
        );
        assert_eq!(response.usage.unwrap().prompt_tokens, 9);
    }

    #[test]
    fn tool_call_response_deserialization() {
        let json = r#"{
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"Paris\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 50, "completion_tokens": 20}
        }"#;
        let response: ApiResponse = serde_json::from_str(json).unwrap();
        let tc = response.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].function.name, "get_weather");
        assert!(tc[0].function.arguments.contains("Paris"));
    }
}
