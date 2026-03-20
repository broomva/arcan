use std::sync::Arc;
use std::time::Instant;

use aios_protocol::{
    EventKind, EventRecord, KernelError, ModelCompletion, ModelCompletionRequest, ModelDirective,
    ModelProviderPort, ModelStopReason, TokenUsage, ToolCall,
};
use arcan_core::protocol::{
    ChatMessage, ModelDirective as ArcanDirective, ModelStopReason as ArcanStopReason,
};
use arcan_core::runtime::{Provider, ProviderRequest, SwappableProviderHandle};
use arcan_core::state::AppState;
use async_trait::async_trait;
use tokio::sync::broadcast;
use tracing::Instrument;

use crate::autonomic::{EconomicGateHandle, EconomicMode};

/// Shared handle for the runtime broadcast sender.
/// Starts as `None` (before the runtime is created) and is filled in
/// once `KernelRuntime::event_sender()` is available.
pub type StreamingSenderHandle = Arc<std::sync::Mutex<Option<broadcast::Sender<EventRecord>>>>;

#[derive(Clone)]
pub struct ArcanProviderAdapter {
    handle: SwappableProviderHandle,
    tools: Vec<arcan_core::protocol::ToolDefinition>,
    streaming_sender: StreamingSenderHandle,
    economic_handle: Option<EconomicGateHandle>,
    /// System prompt to prepend to every provider call (skill catalog, persona, etc.).
    system_prompt: Option<Arc<String>>,
}

impl ArcanProviderAdapter {
    /// Create from a plain provider Arc (backward compatible).
    /// Wraps it in a SwappableProviderHandle internally.
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Vec<arcan_core::protocol::ToolDefinition>,
        streaming_sender: StreamingSenderHandle,
    ) -> Self {
        Self {
            handle: Arc::new(std::sync::RwLock::new(provider)),
            tools,
            streaming_sender,
            economic_handle: None,
            system_prompt: None,
        }
    }

    /// Create from a pre-built swappable handle (for live provider switching).
    pub fn from_handle(
        handle: SwappableProviderHandle,
        tools: Vec<arcan_core::protocol::ToolDefinition>,
        streaming_sender: StreamingSenderHandle,
    ) -> Self {
        Self {
            handle,
            tools,
            streaming_sender,
            economic_handle: None,
            system_prompt: None,
        }
    }

    /// Attach an economic gate handle for advisory token capping.
    ///
    /// When set, the provider will consult economic gates before each model call:
    /// - **Hibernate**: Block the call entirely (return error).
    /// - **Hustle**: Cap `max_tokens` to `gates.max_tokens_next_turn`.
    /// - **Conserving**: Advisory log, cap tokens if set.
    /// - **Sovereign**: No restrictions.
    pub fn with_economic_handle(mut self, handle: EconomicGateHandle) -> Self {
        self.economic_handle = Some(handle);
        self
    }

    /// Set the system prompt (skill catalog, persona, context compiler output).
    ///
    /// When set, a system message is prepended to every provider call.
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        if !prompt.is_empty() {
            self.system_prompt = Some(Arc::new(prompt));
        }
        self
    }

    /// Filter tool definitions by an allowed_tools whitelist.
    ///
    /// Returns either the filtered set (if whitelist is provided) or the full set.
    /// Warns on tool names in the whitelist that don't match any registered tool.
    fn filter_tools(
        &self,
        allowed_tools: Option<&[String]>,
    ) -> Vec<arcan_core::protocol::ToolDefinition> {
        match allowed_tools {
            Some(allowed) => {
                let filtered: Vec<_> = self
                    .tools
                    .iter()
                    .filter(|t| allowed.iter().any(|a| a == &t.name))
                    .cloned()
                    .collect();

                // Warn on whitelist entries that don't match any tool.
                for name in allowed {
                    if !self.tools.iter().any(|t| &t.name == name) {
                        tracing::warn!(
                            tool = %name,
                            "skill allowed_tools references unknown tool"
                        );
                    }
                }

                tracing::debug!(
                    total = self.tools.len(),
                    filtered = filtered.len(),
                    "tool filtering applied by active skill"
                );
                filtered
            }
            None => self.tools.clone(),
        }
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
        // Consult economic gates (advisory — if handle is absent, proceed normally).
        if let Some(ref handle) = self.economic_handle {
            let gates = handle.read().await;
            if let Some(ref gates) = *gates {
                match gates.economic_mode {
                    EconomicMode::Hibernate => {
                        tracing::warn!(
                            session = %request.session_id,
                            "Autonomic: Hibernate mode — blocking model call"
                        );
                        return Err(KernelError::Runtime(
                            "model call blocked: Autonomic Hibernate mode active".to_owned(),
                        ));
                    }
                    EconomicMode::Hustle => {
                        if let Some(max) = gates.max_tokens_next_turn {
                            tracing::info!(
                                session = %request.session_id,
                                max_tokens = max,
                                "Autonomic: Hustle mode — capping tokens"
                            );
                        }
                    }
                    EconomicMode::Conserving => {
                        tracing::debug!(
                            session = %request.session_id,
                            "Autonomic: Conserving mode"
                        );
                    }
                    EconomicMode::Sovereign => {}
                }
            }
        }

        // Build messages: system prompt(s) first, then user objective.
        let mut messages = Vec::new();

        // Adapter-level system prompt (skill catalog from startup).
        if let Some(ref prompt) = self.system_prompt {
            messages.push(ChatMessage::system(prompt.as_str()));
        }

        // Per-request system prompt (active skill body, context compiler output).
        if let Some(ref prompt) = request.system_prompt {
            messages.push(ChatMessage::system(prompt.as_str()));
        }

        messages.push(ChatMessage::user(request.objective));

        // Apply tool filtering from active skill's allowed_tools whitelist.
        let tools = self.filter_tools(request.allowed_tools.as_deref());

        let provider_request = ProviderRequest {
            run_id: request.run_id.as_str().to_owned(),
            session_id: request.session_id.as_str().to_owned(),
            iteration: request.step_index + 1,
            messages,
            tools,
            state: AppState::default(),
        };

        let provider = self
            .handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let provider_name = provider.name().to_owned();
        let use_streaming = provider.supports_streaming();
        let sender = self
            .streaming_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let session_id = request.session_id.clone();
        let branch_id = request.branch_id.clone();

        // Create a GenAI chat span for this provider call.
        let chat_span = vigil::spans::chat_span(&provider_name, &provider_name, None, None);

        // Measure wall-clock duration of the provider call for GenAI metrics.
        let call_start = Instant::now();

        // The Arcan Provider trait is synchronous and may use reqwest::blocking,
        // which panics if called directly on a tokio worker thread.
        // Wrap in spawn_blocking to run on a dedicated thread.
        let turn = tokio::task::spawn_blocking(move || {
            if use_streaming {
                if let Some(sender) = sender {
                    let sess = session_id;
                    let branch = branch_id;
                    return provider.complete_streaming(&provider_request, &|text| {
                        let event = EventRecord::new(
                            sess.clone(),
                            branch.clone(),
                            0, // sequence 0 = ephemeral, not persisted
                            EventKind::AssistantTextDelta {
                                delta: text.to_owned(),
                                index: None,
                            },
                        );
                        let _ = sender.send(event);
                    });
                }
            }
            provider.complete(&provider_request)
        })
        .instrument(chat_span.clone())
        .await
        .map_err(|join_error: tokio::task::JoinError| {
            KernelError::Runtime(format!("provider task panicked: {join_error}"))
        })?
        .map_err(|error| KernelError::Runtime(error.to_string()))?;

        // Record stop reason on the chat span.
        let stop_reason = to_stop_reason(turn.stop_reason);
        let reason_str = match &stop_reason {
            ModelStopReason::Completed => "stop",
            ModelStopReason::ToolCall => "tool_calls",
            ModelStopReason::MaxIterations => "max_tokens",
            ModelStopReason::Cancelled => "cancelled",
            ModelStopReason::Error => "error",
            ModelStopReason::Other(s) => s.as_str(),
        };
        vigil::spans::record_finish_reason(&chat_span, reason_str);

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

        // Record token usage on the chat span.
        if let Some(ref usage) = usage {
            vigil::spans::record_token_usage(&chat_span, usage);
        }

        // Record GenAI metrics (token usage + operation duration).
        let genai_metrics = vigil::metrics::GenAiMetrics::new("arcan");
        let call_duration = call_start.elapsed();
        genai_metrics.record_operation_duration(&provider_name, "chat", call_duration);
        if let Some(ref usage) = usage {
            genai_metrics.record_token_usage(
                &provider_name,
                "chat",
                usage.prompt_tokens as u64,
                usage.completion_tokens as u64,
            );
        }

        Ok(ModelCompletion {
            provider: provider_name.clone(),
            model: provider_name,
            directives,
            stop_reason,
            usage,
            final_answer,
        })
    }
}
