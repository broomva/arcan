use std::sync::Arc;

use aios_protocol::{
    KernelError, ModelCompletion, ModelCompletionRequest, ModelDirective, ModelProviderPort,
    ModelStopReason, TokenUsage, ToolCall,
};
use arcan_core::protocol::{
    ChatMessage, ModelDirective as ArcanDirective, ModelStopReason as ArcanStopReason,
};
use arcan_core::runtime::{Provider, ProviderRequest};
use arcan_core::state::AppState;
use async_trait::async_trait;

#[derive(Clone)]
pub struct ArcanProviderAdapter {
    provider: Arc<dyn Provider>,
    tools: Vec<arcan_core::protocol::ToolDefinition>,
}

impl ArcanProviderAdapter {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<arcan_core::protocol::ToolDefinition>,
    ) -> Self {
        Self { provider, tools }
    }
}

fn to_stop_reason(stop_reason: ArcanStopReason) -> ModelStopReason {
    match stop_reason {
        ArcanStopReason::EndTurn => ModelStopReason::Completed,
        ArcanStopReason::ToolUse => ModelStopReason::ToolCall,
        ArcanStopReason::NeedsUser => ModelStopReason::Cancelled,
        ArcanStopReason::MaxTokens => ModelStopReason::MaxIterations,
        ArcanStopReason::Safety => ModelStopReason::Error,
        ArcanStopReason::Unknown => ModelStopReason::Other("unknown".to_owned()),
    }
}

#[async_trait]
impl ModelProviderPort for ArcanProviderAdapter {
    async fn complete(
        &self,
        request: ModelCompletionRequest,
    ) -> Result<ModelCompletion, KernelError> {
        let provider_request = ProviderRequest {
            run_id: request.run_id.as_str().to_owned(),
            session_id: request.session_id.as_str().to_owned(),
            iteration: request.step_index + 1,
            messages: vec![ChatMessage::user(request.objective)],
            tools: self.tools.clone(),
            state: AppState::default(),
        };

        let turn = self
            .provider
            .complete(&provider_request)
            .map_err(|error| KernelError::Runtime(error.to_string()))?;

        let mut directives = Vec::new();
        let mut final_answer = None;
        for directive in turn.directives {
            match directive {
                ArcanDirective::Text { delta } => {
                    directives.push(ModelDirective::TextDelta { delta, index: None });
                }
                ArcanDirective::ToolCall { call } => {
                    directives.push(ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: call.call_id,
                            tool_name: call.tool_name,
                            input: call.input,
                            requested_capabilities: Vec::new(),
                        },
                    });
                }
                ArcanDirective::StatePatch { patch } => {
                    directives.push(ModelDirective::Message {
                        role: "system".to_owned(),
                        content: serde_json::to_string(&patch.patch)
                            .unwrap_or_else(|_| "{}".to_owned()),
                    });
                }
                ArcanDirective::FinalAnswer { text } => {
                    final_answer = Some(text.clone());
                    directives.push(ModelDirective::Message {
                        role: "assistant".to_owned(),
                        content: text,
                    });
                }
            }
        }

        let usage = turn.usage.map(|usage| TokenUsage {
            prompt_tokens: usage.input_tokens as u32,
            completion_tokens: usage.output_tokens as u32,
            total_tokens: usage.total() as u32,
        });

        Ok(ModelCompletion {
            provider: self.provider.name().to_owned(),
            model: self.provider.name().to_owned(),
            directives,
            stop_reason: to_stop_reason(turn.stop_reason),
            usage,
            final_answer,
        })
    }
}
