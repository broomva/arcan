use std::sync::Arc;
use std::time::Instant;

use aios_protocol::{
    EventKind, EventRecord, KernelError, ModelCompletion, ModelCompletionRequest, ModelDirective,
    ModelProviderPort, ModelStopReason, TokenUsage, ToolCall,
};

use crate::capability_map::capabilities_for_tool;
use arcan_core::protocol::{
    ChatMessage, ModelDirective as ArcanDirective, ModelStopReason as ArcanStopReason,
    ProviderCircuitState,
};
use arcan_core::runtime::{Provider, ProviderRequest, StreamEvent, SwappableProviderHandle};
use arcan_core::state::AppState;
use async_trait::async_trait;
use life_vigil::{CircuitState, JsonlWriter, LlmCallRecord, LlmRequestEnvelope};
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
    /// Shared GenAI metrics instruments (created once, reused across calls).
    genai_metrics: Arc<life_vigil::GenAiMetrics>,
    /// Optional local dual-write sink for LLM call envelopes.
    jsonl_writer: Option<JsonlWriter>,
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
            genai_metrics: Arc::new(life_vigil::GenAiMetrics::new("arcan")),
            jsonl_writer: JsonlWriter::from_env(),
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
            genai_metrics: Arc::new(life_vigil::GenAiMetrics::new("arcan")),
            jsonl_writer: JsonlWriter::from_env(),
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

    /// Override the JSONL writer used for LLM call envelope dual-writes.
    ///
    /// This exists primarily for tests and for embedders that want to provide a
    /// sink without relying on process-wide environment variables.
    pub fn with_jsonl_writer(mut self, writer: Option<JsonlWriter>) -> Self {
        self.jsonl_writer = writer;
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

fn infer_provider_system(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("claude") || lower.contains("anthropic") {
        "anthropic"
    } else if lower.starts_with("gpt-")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.contains("openai")
    {
        "openai"
    } else if lower.contains("ollama") || lower.contains("llama") {
        "ollama"
    } else if lower.contains("mock") {
        "mock"
    } else {
        "unknown"
    }
}

fn infer_model_tier(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("opus") || lower.starts_with("o3") {
        "frontier"
    } else if lower.contains("sonnet") || lower.contains("gpt-4o") {
        "balanced"
    } else if lower.contains("haiku") || lower.contains("mini") {
        "economy"
    } else if lower.contains("llama") || lower.contains("ollama") {
        "local"
    } else {
        "unknown"
    }
}

fn economic_mode_label(mode: EconomicMode) -> &'static str {
    match mode {
        EconomicMode::Sovereign => "sovereign",
        EconomicMode::Conserving => "conserving",
        EconomicMode::Hustle => "hustle",
        EconomicMode::Hibernate => "hibernate",
    }
}

fn to_vigil_circuit_state(state: ProviderCircuitState) -> CircuitState {
    match state {
        ProviderCircuitState::Closed => CircuitState::Closed,
        ProviderCircuitState::Open => CircuitState::Open,
        ProviderCircuitState::HalfOpen => CircuitState::HalfOpen,
    }
}

fn llm_call_record(
    envelope: LlmRequestEnvelope,
    response: Option<life_vigil::LlmResponseEconomics>,
    error: Option<String>,
) -> LlmCallRecord {
    LlmCallRecord {
        timestamp: life_vigil::jsonl::now_iso8601(),
        envelope,
        response,
        trace_id: None,
        span_id: None,
        error,
    }
}

fn write_llm_call_record(writer: Option<&JsonlWriter>, record: &LlmCallRecord) {
    if let Some(writer) = writer {
        writer.write_best_effort(record);
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
        let provider = self
            .handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let model_name = provider.name().to_owned();
        let provider_system = infer_provider_system(&model_name).to_owned();

        let mut envelope = LlmRequestEnvelope::new(
            request.session_id.as_str(),
            request.run_id.as_str(),
            "arcan",
            request.step_index,
            provider_system.clone(),
            model_name.clone(),
        );
        envelope.branch_id = Some(request.branch_id.to_string());
        envelope.provider_requested = provider_system.clone();
        envelope.provider_selected = provider_system.clone();
        envelope.model_tier = Some(infer_model_tier(&model_name).to_owned());
        envelope.routing_decision = Some("direct_provider_handle".to_owned());
        envelope.allowed_tools = request.allowed_tools.clone();
        let mut max_tokens = None;

        // Consult economic gates (advisory — if handle is absent, proceed normally).
        if let Some(ref handle) = self.economic_handle {
            let gates = handle.read().await;
            if let Some(ref gates) = *gates {
                envelope.policy_mode = Some(economic_mode_label(gates.economic_mode).to_owned());
                match gates.economic_mode {
                    EconomicMode::Hibernate => {
                        tracing::warn!(
                            session = %request.session_id,
                            "Autonomic: Hibernate mode — blocking model call"
                        );
                        envelope.policy_decision = Some("blocked_hibernate".to_owned());
                        let record = llm_call_record(
                            envelope,
                            None,
                            Some("model call blocked: Autonomic Hibernate mode active".to_owned()),
                        );
                        write_llm_call_record(self.jsonl_writer.as_ref(), &record);
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
                            max_tokens = Some(max);
                            envelope.max_tokens = Some(max);
                        }
                        envelope.policy_decision = Some("allowed_with_token_cap".to_owned());
                    }
                    EconomicMode::Conserving => {
                        tracing::debug!(
                            session = %request.session_id,
                            "Autonomic: Conserving mode"
                        );
                        envelope.policy_decision = Some("allowed_conserving".to_owned());
                    }
                    EconomicMode::Sovereign => {
                        envelope.policy_decision = Some("allowed".to_owned());
                    }
                }
            }
        }

        // Check if prompt/completion content capture is enabled (privacy-sensitive).
        let capture_content = std::env::var("VIGIL_CAPTURE_CONTENT")
            .map(|v| matches!(v.as_str(), "true" | "1" | "yes"))
            .unwrap_or(false);

        // Snapshot objective before it's moved into messages (needed for prompt event).
        let objective_snapshot = if capture_content && !request.objective.is_empty() {
            Some(request.objective.clone())
        } else {
            None
        };

        // Build messages: system prompt(s), conversation history, then current objective.
        let mut messages = Vec::new();

        // Adapter-level system prompt (skill catalog from startup).
        if let Some(ref prompt) = self.system_prompt {
            messages.push(ChatMessage::system(prompt.as_str()));
        }

        // Per-request system prompt (active skill body, context compiler output).
        if let Some(ref prompt) = request.system_prompt {
            messages.push(ChatMessage::system(prompt.as_str()));
        }

        // Conversation history from prior turns (built by tick_on_branch from Lago).
        for turn in &request.conversation_history {
            match turn.role.as_str() {
                "user" => messages.push(ChatMessage::user(&turn.content)),
                "assistant" => messages.push(ChatMessage::assistant(&turn.content)),
                _ => messages.push(ChatMessage::system(&turn.content)),
            }
        }

        // Current user objective (the new message for this turn).
        if !request.objective.is_empty() {
            messages.push(ChatMessage::user(request.objective));
        }

        // Apply tool filtering from active skill's allowed_tools whitelist.
        let tools = self.filter_tools(request.allowed_tools.as_deref());

        let provider_request = ProviderRequest {
            run_id: request.run_id.as_str().to_owned(),
            session_id: request.session_id.as_str().to_owned(),
            iteration: request.step_index + 1,
            messages,
            tools,
            max_tokens,
            state: AppState::default(),
        };

        let use_streaming = provider.supports_streaming();
        let sender = self
            .streaming_sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let session_id = request.session_id.clone();
        let branch_id = request.branch_id.clone();

        // Create a GenAI chat span for this provider call (with session.id for thread grouping).
        let chat_span = life_vigil::spans::chat_span(
            &model_name,
            &provider_system,
            None,
            None,
            session_id.as_str(),
        );
        envelope.record_on_span(&chat_span);

        // Record prompt content as span event (input capture for LangSmith/Langfuse).
        if let Some(ref objective) = objective_snapshot {
            let _enter = chat_span.enter();
            life_vigil::spans::record_prompt_content(objective);
        }

        // Measure wall-clock duration of the provider call for GenAI metrics.
        let call_start = Instant::now();

        // The Arcan Provider trait is synchronous and may use reqwest::blocking,
        // which panics if called directly on a tokio worker thread.
        // Wrap in spawn_blocking to run on a dedicated thread.
        let turn_result = tokio::task::spawn_blocking(move || {
            if use_streaming && let Some(sender) = sender {
                let sess = session_id;
                let branch = branch_id;
                return provider.complete_streaming(&provider_request, &|delta| {
                    if let StreamEvent::Text(text) = delta {
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
                    }
                });
            }
            provider.complete(&provider_request)
        })
        .instrument(chat_span.clone())
        .await;

        let turn = match turn_result {
            Err(join_error) => {
                let call_duration = call_start.elapsed();
                let error = format!("provider task panicked: {join_error}");
                let mut error_envelope = envelope.clone();
                error_envelope.latency_ms =
                    Some(call_duration.as_millis().min(u128::from(u64::MAX)) as u64);
                error_envelope
                    .policy_decision
                    .get_or_insert_with(|| "error".to_owned());
                error_envelope.record_on_span(&chat_span);
                life_vigil::spans::record_finish_reason(&chat_span, "error");
                life_vigil::spans::record_reliability(&chat_span, 0, false, "closed");
                self.genai_metrics.record_operation_duration(
                    &provider_system,
                    &model_name,
                    "chat",
                    "error",
                    call_duration,
                );
                self.genai_metrics.record_llm_request(
                    &provider_system,
                    &model_name,
                    "chat",
                    "error",
                );
                let record = llm_call_record(error_envelope, None, Some(error.clone()));
                write_llm_call_record(self.jsonl_writer.as_ref(), &record);
                return Err(KernelError::Runtime(error));
            }
            Ok(Err(error)) => {
                let call_duration = call_start.elapsed();
                let error = error.to_string();
                let mut error_envelope = envelope.clone();
                error_envelope.latency_ms =
                    Some(call_duration.as_millis().min(u128::from(u64::MAX)) as u64);
                error_envelope
                    .policy_decision
                    .get_or_insert_with(|| "error".to_owned());
                error_envelope.record_on_span(&chat_span);
                life_vigil::spans::record_finish_reason(&chat_span, "error");
                life_vigil::spans::record_reliability(&chat_span, 0, false, "closed");
                self.genai_metrics.record_operation_duration(
                    &provider_system,
                    &model_name,
                    "chat",
                    "error",
                    call_duration,
                );
                self.genai_metrics.record_llm_request(
                    &provider_system,
                    &model_name,
                    "chat",
                    "error",
                );
                let record = llm_call_record(error_envelope, None, Some(error.clone()));
                write_llm_call_record(self.jsonl_writer.as_ref(), &record);
                return Err(KernelError::Runtime(error));
            }
            Ok(Ok(turn)) => turn,
        };

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
        let provider_telemetry = turn.telemetry.clone().unwrap_or_default();
        let circuit_state = provider_telemetry.circuit_state.as_str();
        let finish_reason = provider_telemetry
            .finish_reason
            .clone()
            .unwrap_or_else(|| reason_str.to_owned());
        life_vigil::spans::record_finish_reason(&chat_span, reason_str);

        // Record completion content as span event (output capture for LangSmith/Langfuse).
        if capture_content {
            let completion_text: String = turn
                .directives
                .iter()
                .filter_map(|d| match d {
                    ArcanDirective::Text { delta } => Some(delta.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            if !completion_text.is_empty() {
                let _enter = chat_span.enter();
                life_vigil::spans::record_completion_content(&completion_text);
            }
        }

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
                            requested_capabilities: capabilities_for_tool(
                                &call.tool_name,
                                &call.input,
                            ),
                            call_id: call.call_id,
                            tool_name: call.tool_name,
                            input: call.input,
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

        let call_duration = call_start.elapsed();
        let response_economics = turn.usage.as_ref().map(|usage| {
            life_vigil::pricing::build_response_economics(
                &model_name,
                usage.input_tokens as u32,
                usage.output_tokens as u32,
                usage.cache_read_tokens as u32,
                usage.cache_creation_tokens as u32,
                call_duration,
            )
        });

        let usage = turn.usage.map(|usage| TokenUsage {
            prompt_tokens: usage.input_tokens as u32,
            completion_tokens: usage.output_tokens as u32,
            total_tokens: usage.total() as u32,
        });

        // Record token usage on the chat span (attributes + event for OTel bridge reliability).
        if let Some(ref usage) = usage {
            life_vigil::spans::record_token_usage(&chat_span, usage);
            // Also emit as span event — events propagate more reliably through
            // tracing-opentelemetry → LangSmith than record() on Empty fields.
            let _enter = chat_span.enter();
            life_vigil::spans::record_usage_event(
                usage.prompt_tokens,
                usage.completion_tokens,
                &model_name,
                reason_str,
            );
        }
        if let Some(ref response_economics) = response_economics {
            response_economics.record_on_span(&chat_span);
        }
        life_vigil::spans::record_reliability(
            &chat_span,
            provider_telemetry.retry_count,
            provider_telemetry.fallback_triggered,
            circuit_state,
        );

        // Record GenAI metrics (token usage + operation duration) on shared instruments.
        self.genai_metrics.record_operation_duration(
            &provider_system,
            &model_name,
            "chat",
            "success",
            call_duration,
        );
        self.genai_metrics
            .record_llm_request(&provider_system, &model_name, "chat", "success");
        if let Some(ref usage) = usage {
            self.genai_metrics.record_token_usage(
                &provider_system,
                &model_name,
                "chat",
                usage.prompt_tokens as u64,
                usage.completion_tokens as u64,
            );
        }
        if let Some(cost_usd) = response_economics
            .as_ref()
            .and_then(|economics| economics.total_cost_usd)
        {
            self.genai_metrics.record_estimated_cost_usd(
                &provider_system,
                &model_name,
                "chat",
                "direct_provider_handle",
                cost_usd,
            );
        }

        let mut completed_envelope = envelope;
        completed_envelope.latency_ms =
            Some(call_duration.as_millis().min(u128::from(u64::MAX)) as u64);
        completed_envelope.retry_count = provider_telemetry.retry_count;
        completed_envelope.fallback_triggered = provider_telemetry.fallback_triggered;
        completed_envelope.fallback_reason = provider_telemetry.fallback_reason;
        completed_envelope.circuit_state = to_vigil_circuit_state(provider_telemetry.circuit_state);
        completed_envelope.time_to_first_token_ms = provider_telemetry.time_to_first_token_ms;
        completed_envelope.finish_reason = Some(finish_reason);
        completed_envelope.tokens_in = usage.as_ref().map(|usage| usage.prompt_tokens);
        completed_envelope.tokens_out = usage.as_ref().map(|usage| usage.completion_tokens);
        completed_envelope.cost_source = response_economics
            .as_ref()
            .map(|economics| economics.cost_source);
        completed_envelope.estimated_cost_usd = response_economics
            .as_ref()
            .and_then(|economics| economics.total_cost_usd);
        completed_envelope.estimated_total_cost_usd = completed_envelope.estimated_cost_usd;
        completed_envelope.record_on_span(&chat_span);
        let call_record = llm_call_record(completed_envelope, response_economics, None);
        write_llm_call_record(self.jsonl_writer.as_ref(), &call_record);
        let llm_call_record = serde_json::to_value(&call_record).ok();

        Ok(ModelCompletion {
            provider: provider_system,
            model: model_name,
            llm_call_record,
            directives,
            stop_reason,
            usage,
            final_answer,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::{BranchId, RunId, SessionId};
    use arcan_core::error::CoreError;
    use arcan_core::protocol::{ModelTurn, ProviderTelemetry, TokenUsage as ArcanTokenUsage};

    struct ScriptedProvider;

    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "claude-sonnet-4-20250514"
        }

        fn complete(&self, _request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
            Ok(ModelTurn {
                directives: vec![ArcanDirective::FinalAnswer {
                    text: "done".to_owned(),
                }],
                stop_reason: ArcanStopReason::EndTurn,
                usage: Some(ArcanTokenUsage {
                    input_tokens: 1_000,
                    output_tokens: 250,
                    cache_read_tokens: 0,
                    cache_creation_tokens: 0,
                }),
                telemetry: Some(ProviderTelemetry {
                    retry_count: 2,
                    time_to_first_token_ms: Some(42),
                    finish_reason: Some("stop".to_owned()),
                    ..Default::default()
                }),
            })
        }
    }

    fn streaming_sender() -> StreamingSenderHandle {
        Arc::new(std::sync::Mutex::new(None))
    }

    #[tokio::test]
    async fn complete_writes_llm_envelope_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let jsonl_path = dir.path().join("llm-calls.jsonl");
        let adapter =
            ArcanProviderAdapter::new(Arc::new(ScriptedProvider), Vec::new(), streaming_sender())
                .with_jsonl_writer(Some(JsonlWriter::new(&jsonl_path)));

        let completion = adapter
            .complete(ModelCompletionRequest {
                session_id: SessionId::from("sess-1"),
                branch_id: BranchId::main(),
                run_id: RunId::from("run-1"),
                step_index: 2,
                objective: "answer".to_owned(),
                proposed_tool: None,
                system_prompt: None,
                allowed_tools: Some(vec!["read_file".to_owned()]),
                conversation_history: Vec::new(),
            })
            .await
            .unwrap();

        assert_eq!(completion.final_answer.as_deref(), Some("done"));
        assert!(completion.llm_call_record.is_some());

        let content = std::fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);

        let record: LlmCallRecord = serde_json::from_str(lines[0]).unwrap();
        assert!(
            record
                .envelope
                .request_id
                .starts_with("sess-1:run-1:2:anthropic:claude-sonnet-4-20250514:")
        );
        assert_eq!(completion.provider, "anthropic");
        assert_eq!(record.envelope.provider_requested, "anthropic");
        assert_eq!(record.envelope.provider_selected, "anthropic");
        assert_eq!(record.envelope.model, "claude-sonnet-4-20250514");
        assert_eq!(record.envelope.model_tier.as_deref(), Some("balanced"));
        assert_eq!(
            record.envelope.allowed_tools,
            Some(vec!["read_file".to_owned()])
        );
        assert!(record.envelope.latency_ms.is_some());
        assert_eq!(record.envelope.tokens_in, Some(1_000));
        assert_eq!(record.envelope.tokens_out, Some(250));
        assert!(record.envelope.estimated_cost_usd.is_some());
        assert_eq!(record.envelope.retry_count, 2);
        assert!(!record.envelope.fallback_triggered);
        assert_eq!(record.envelope.circuit_state, CircuitState::Closed);
        assert_eq!(record.envelope.time_to_first_token_ms, Some(42));
        assert_eq!(record.envelope.finish_reason.as_deref(), Some("stop"));

        let response = record.response.unwrap();
        assert_eq!(response.input_tokens, 1_000);
        assert_eq!(response.output_tokens, 250);
        assert!(response.total_cost_usd.is_some());
        assert!(record.error.is_none());
    }
}
