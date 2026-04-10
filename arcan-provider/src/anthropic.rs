use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ModelTurn, Role, TokenUsage, ToolCall,
    ToolDefinition,
};
use arcan_core::runtime::{Provider, ProviderRequest, StreamEvent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::credential::{AnthropicApiKeyCredential, Credential};

/// Configuration for the Anthropic provider.
pub struct AnthropicConfig {
    pub credential: Arc<dyn Credential>,
    pub model: String,
    pub max_tokens: u32,
    pub base_url: String,
}

impl std::fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("credential", &self.credential.kind())
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .field("base_url", &self.base_url)
            .finish()
    }
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
            credential: Arc::new(AnthropicApiKeyCredential::new(api_key)),
            model,
            max_tokens,
            base_url,
        })
    }

    /// Create config from resolved CLI settings.
    ///
    /// API key is always read from env (never from config file).
    /// Other settings use the provided overrides, falling back to env vars.
    pub fn from_resolved(
        model_override: Option<&str>,
        base_url_override: Option<&str>,
        max_tokens_override: Option<u32>,
    ) -> Result<Self, CoreError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            CoreError::Provider("ANTHROPIC_API_KEY environment variable not set".to_string())
        })?;

        let model = model_override
            .map(String::from)
            .or_else(|| std::env::var("ANTHROPIC_MODEL").ok())
            .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string());

        let max_tokens = max_tokens_override
            .or_else(|| {
                std::env::var("ANTHROPIC_MAX_TOKENS")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(4096);

        let base_url = base_url_override
            .map(String::from)
            .or_else(|| std::env::var("ANTHROPIC_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());

        Ok(Self {
            credential: Arc::new(crate::credential::AnthropicApiKeyCredential::new(api_key)),
            model,
            max_tokens,
            base_url,
        })
    }
}

/// Check if a key/header value is an OAuth access token (vs. regular API key).
///
/// Handles both raw tokens (`sk-ant-oat01-...`) and pre-formatted Bearer headers
/// (`Bearer sk-ant-oat01-...`) from `OAuthCredential::auth_header()`.
fn is_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat") || key.starts_with("Bearer sk-ant-oat")
}

/// Apply auth headers to a request builder.
///
/// OAuth tokens use `Authorization: Bearer` + the `anthropic-beta: oauth-2025-04-20`
/// header. Regular API keys use `x-api-key`.
fn apply_auth(
    builder: reqwest::blocking::RequestBuilder,
    key: &str,
) -> reqwest::blocking::RequestBuilder {
    if is_oauth_token(key) {
        // If already prefixed with "Bearer ", use as-is; otherwise add prefix.
        let bearer = if key.starts_with("Bearer ") {
            key.to_string()
        } else {
            format!("Bearer {key}")
        };
        builder
            .header("authorization", bearer)
            .header("anthropic-beta", crate::oauth::ANTHROPIC_OAUTH_BETA)
    } else {
        builder.header("x-api-key", key)
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
        let mut system_prompt: Option<String> = None;
        let mut api_messages = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => match &mut system_prompt {
                    Some(existing) => {
                        existing.push_str("\n\n");
                        existing.push_str(&msg.content);
                    }
                    None => {
                        system_prompt = Some(msg.content.clone());
                    }
                },
                Role::User => {
                    // Check if content is JSON-encoded content blocks (from shell tool results)
                    if let Ok(blocks) = serde_json::from_str::<Vec<ContentBlock>>(&msg.content)
                        && !blocks.is_empty()
                        && blocks
                            .iter()
                            .all(|b| matches!(b, ContentBlock::ToolResult { .. }))
                    {
                        api_messages.push(ApiMessage {
                            role: "user".to_string(),
                            content: ApiContent::Blocks(blocks),
                        });
                        continue;
                    }
                    api_messages.push(ApiMessage {
                        role: "user".to_string(),
                        content: ApiContent::Text(msg.content.clone()),
                    });
                }
                Role::Assistant => {
                    // Check if content is JSON-encoded content blocks (from shell tool_use)
                    if let Ok(blocks) = serde_json::from_str::<Vec<ContentBlock>>(&msg.content)
                        && !blocks.is_empty()
                        && blocks
                            .iter()
                            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
                    {
                        api_messages.push(ApiMessage {
                            role: "assistant".to_string(),
                            content: ApiContent::Blocks(blocks),
                        });
                        continue;
                    }
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
                            is_error: false,
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

    fn context_window(&self) -> Option<u32> {
        // All current Claude models support 200K context.
        Some(200_000)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn complete_streaming(
        &self,
        request: &ProviderRequest,
        on_delta: &dyn Fn(StreamEvent<'_>),
    ) -> Result<ModelTurn, CoreError> {
        let (system_prompt, api_messages) = self.build_messages(&request.messages);
        let api_tools = self.convert_tools(&request.tools);
        let max_tokens = request.max_tokens.unwrap_or(self.config.max_tokens);

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": max_tokens,
            "messages": api_messages,
            "stream": true,
        });

        if let Some(system) = system_prompt {
            body["system"] = json!(system);
        }
        if !api_tools.is_empty() {
            body["tools"] = serde_json::to_value(&api_tools)
                .map_err(|e| CoreError::Provider(format!("failed to serialize tools: {e}")))?;
        }

        let url = format!("{}/v1/messages", self.config.base_url);
        let api_key = self
            .config
            .credential
            .auth_header()
            .map_err(|e| CoreError::Provider(format!("credential error: {e}")))?;

        let request = self
            .client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
        let response = apply_auth(request, &api_key)
            .json(&body)
            .send()
            .map_err(|e| CoreError::Provider(format!("streaming request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response
                .text()
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(CoreError::Provider(format!(
                "Anthropic streaming API returned {status}: {body_text}"
            )));
        }

        // Parse SSE stream line by line
        use std::io::BufRead;
        let reader = std::io::BufReader::new(response);

        let mut directives = Vec::new();
        let mut tool_inputs: std::collections::BTreeMap<u32, (String, String, String)> =
            std::collections::BTreeMap::new(); // index → (id, name, json_accum)
        let mut stop_reason = ModelStopReason::Unknown;
        let mut usage: Option<TokenUsage> = None;
        let mut current_event_type = String::new();

        for line in reader.lines() {
            let line =
                line.map_err(|e| CoreError::Provider(format!("streaming read error: {e}")))?;

            // SSE format: "event: <type>" then "data: <json>"
            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event_type = event_type.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            let v: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match current_event_type.as_str() {
                "message_start" => {
                    if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
                        let input_tokens = u["input_tokens"].as_u64().unwrap_or(0);
                        usage = Some(TokenUsage {
                            input_tokens,
                            output_tokens: 0,
                            cache_read_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
                            cache_creation_tokens: u["cache_creation_input_tokens"]
                                .as_u64()
                                .unwrap_or(0),
                        });
                    }
                }
                "content_block_start" => {
                    let index = v["index"].as_u64().unwrap_or(0) as u32;
                    if let Some(cb) = v.get("content_block")
                        && cb["type"].as_str() == Some("tool_use")
                    {
                        let id = cb["id"].as_str().unwrap_or_default().to_string();
                        let name = cb["name"].as_str().unwrap_or_default().to_string();
                        tool_inputs.insert(index, (id, name, String::new()));
                    }
                }
                "content_block_delta" => {
                    let index = v["index"].as_u64().unwrap_or(0) as u32;
                    if let Some(delta) = v.get("delta") {
                        match delta["type"].as_str() {
                            Some("text_delta") => {
                                if let Some(text) = delta["text"].as_str() {
                                    on_delta(StreamEvent::Text(text));
                                    directives.push(ModelDirective::Text {
                                        delta: text.to_string(),
                                    });
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(thinking) = delta["thinking"].as_str() {
                                    on_delta(StreamEvent::Reasoning(thinking));
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(json_chunk) = delta["partial_json"].as_str()
                                    && let Some(entry) = tool_inputs.get_mut(&index)
                                {
                                    entry.2.push_str(json_chunk);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "content_block_stop" => {
                    let index = v["index"].as_u64().unwrap_or(0) as u32;
                    if let Some((id, name, json_str)) = tool_inputs.remove(&index) {
                        let input: Value = serde_json::from_str(&json_str)
                            .unwrap_or(Value::Object(Default::default()));
                        directives.push(ModelDirective::ToolCall {
                            call: ToolCall {
                                call_id: id,
                                tool_name: name,
                                input,
                            },
                        });
                    }
                }
                "message_delta" => {
                    if let Some(delta) = v.get("delta") {
                        stop_reason = match delta["stop_reason"].as_str() {
                            Some("end_turn") => ModelStopReason::EndTurn,
                            Some("tool_use") => ModelStopReason::ToolUse,
                            Some("max_tokens") => ModelStopReason::MaxTokens,
                            _ => ModelStopReason::Unknown,
                        };
                    }
                    if let Some(u) = v.get("usage")
                        && let Some(ref mut existing) = usage
                    {
                        existing.output_tokens = u["output_tokens"].as_u64().unwrap_or(0);
                    }
                }
                _ => {}
            }
        }

        Ok(ModelTurn {
            directives,
            stop_reason,
            usage,
        })
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let (system_prompt, api_messages) = self.build_messages(&request.messages);
        let api_tools = self.convert_tools(&request.tools);
        let max_tokens = request.max_tokens.unwrap_or(self.config.max_tokens);

        let mut body = json!({
            "model": self.config.model,
            "max_tokens": max_tokens,
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

        let api_key = self
            .config
            .credential
            .auth_header()
            .map_err(|e| CoreError::Provider(format!("credential error: {e}")))?;

        let request = self
            .client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
        let response = apply_auth(request, &api_key)
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
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

    fn test_config() -> AnthropicConfig {
        AnthropicConfig {
            credential: Arc::new(AnthropicApiKeyCredential::new("test".to_string())),
            model: "test-model".to_string(),
            max_tokens: 1024,
            base_url: "http://localhost".to_string(),
        }
    }

    #[test]
    fn builds_messages_with_system_prompt() {
        let provider = AnthropicProvider::new(test_config());

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
    fn concatenates_multiple_system_messages() {
        let provider = AnthropicProvider::new(test_config());

        let messages = vec![
            ChatMessage::system("First system message."),
            ChatMessage::system("Second system message."),
            ChatMessage::user("Hello"),
        ];

        let (system, api_msgs) = provider.build_messages(&messages);
        let system_text = system.expect("should have system prompt");
        assert!(
            system_text.contains("First system message."),
            "missing first system message"
        );
        assert!(
            system_text.contains("Second system message."),
            "missing second system message"
        );
        assert_eq!(api_msgs.len(), 1);
    }

    #[test]
    fn parses_text_response() {
        let provider = AnthropicProvider::new(test_config());

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
        let provider = AnthropicProvider::new(test_config());

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
