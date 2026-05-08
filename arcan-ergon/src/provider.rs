//! `ergon::Provider` over `aios_protocol::ModelProviderPort`.
//!
//! Translates ergon's structured request (typed `Vec<Message>` plus
//! system prompt and tool list) into the kernel's flatter
//! `ModelCompletionRequest` (objective + system prompt +
//! conversation history), invokes the registered port, and converts
//! the returned `ModelCompletion` back into ergon's `ModelResponse` —
//! emitting `StreamEvent`s onto the sink as we walk the directives.
//!
//! ## Why we synthesize stream events from directives
//!
//! The kernel's `ModelProviderPort::complete` is not yet a streaming
//! contract — it returns a complete `ModelCompletion` in one shot. We
//! still need to feed the workflow's [`ergon::StreamSink`] (which
//! drives observability, lago persistence, lifegw delivery, etc.). So
//! we synthesize the canonical event sequence from the directives:
//!
//! ```text
//! SessionStart → for each directive { TextStart/ToolUseStart →
//! TextDelta/ToolUseInputDelta → TextEnd/ToolUseEnd } → Usage → Done
//! ```
//!
//! Once the port itself becomes streaming (Spec E / BRO-1019), this
//! adapter will stream natively without buffering.

use crate::error::AdapterError;
use aios_protocol::{
    ConversationTurn, ModelCompletionRequest, ModelDirective, ModelProviderPort, ModelStopReason,
};
use async_trait::async_trait;
use ergon::{
    ContentBlock, ErgonError, Message, MessageRole, ModelRequest, ModelResponse, Provider,
    Result as ErgonResult, SessionId, StopReason, StreamEvent, StreamSink, ToolCall, Usage,
};
use std::sync::Arc;

/// Adapter wrapping a [`ModelProviderPort`] as an [`ergon::Provider`].
///
/// Carries enough kernel-side context to populate
/// [`ModelCompletionRequest`] correctly per turn. The
/// [`crate::run_workflow_as_tick`] runner constructs one of these per
/// tick, holding the live `SessionId` / `BranchId` / `RunId` from the
/// invocation.
pub struct ModelProviderAdapter {
    port: Arc<dyn ModelProviderPort>,
    session_id: aios_protocol::SessionId,
    branch_id: aios_protocol::BranchId,
    run_id: aios_protocol::RunId,
    provider_name: String,
}

impl ModelProviderAdapter {
    /// Construct from a port and per-tick identifiers.
    ///
    /// `provider_name` is reported back via [`ergon::Provider::name`]
    /// and embedded in [`StreamEvent::SessionStart`]. Pass the kernel
    /// runtime's configured provider label (typically `"canonical"`
    /// or the provider crate's name).
    pub fn new(
        port: Arc<dyn ModelProviderPort>,
        session_id: aios_protocol::SessionId,
        branch_id: aios_protocol::BranchId,
        run_id: aios_protocol::RunId,
        provider_name: impl Into<String>,
    ) -> Self {
        Self {
            port,
            session_id,
            branch_id,
            run_id,
            provider_name: provider_name.into(),
        }
    }
}

#[async_trait]
impl Provider for ModelProviderAdapter {
    fn name(&self) -> &str {
        &self.provider_name
    }

    async fn stream(
        &self,
        req: ModelRequest,
        sink: Arc<dyn StreamSink>,
    ) -> ErgonResult<ModelResponse> {
        // 1. Emit SessionStart so durable sinks see the boundary.
        sink.emit(StreamEvent::SessionStart {
            session_id: ergon_session_id(&self.session_id),
            model: req.model.clone(),
            provider: self.provider_name.clone(),
        })
        .await?;

        // 2. Translate ergon::ModelRequest → ModelCompletionRequest.
        let port_request = build_completion_request(
            &req,
            self.session_id.clone(),
            self.branch_id.clone(),
            self.run_id.clone(),
        );

        // 3. Invoke the port.
        let completion = self.port.complete(port_request).await.map_err(|err| {
            ErgonError::Provider(AdapterError::port("ModelProviderPort", err).to_string())
        })?;

        // 4. Replay directives onto the sink as canonical StreamEvents,
        //    while assembling the ergon ContentBlocks for the returned
        //    ModelResponse. The two views must agree.
        //
        //    Per-directive `id`s must be unique across the stream so
        //    sinks (lago durable replay, vigil tracing, lifegw SSE)
        //    can pair Start/Delta/End events. We synthesize them from
        //    a directive-local counter; the upstream `index` (when
        //    present on TextDelta) is folded in for stable ordering.
        let mut content: Vec<ContentBlock> = Vec::new();
        let mut message_idx: u32 = 0;
        for directive in &completion.directives {
            match directive {
                ModelDirective::TextDelta { delta, index } => {
                    let id = format!("text-{}", index.unwrap_or(0));
                    sink.emit(StreamEvent::TextStart { id: id.clone() }).await?;
                    sink.emit(StreamEvent::TextDelta {
                        id: id.clone(),
                        delta: delta.clone(),
                    })
                    .await?;
                    sink.emit(StreamEvent::TextEnd { id }).await?;
                    content.push(ContentBlock::text(delta));
                }
                ModelDirective::Message {
                    role: _,
                    content: text,
                } => {
                    let id = format!("message-{message_idx}");
                    message_idx += 1;
                    sink.emit(StreamEvent::TextStart { id: id.clone() }).await?;
                    sink.emit(StreamEvent::TextDelta {
                        id: id.clone(),
                        delta: text.clone(),
                    })
                    .await?;
                    sink.emit(StreamEvent::TextEnd { id }).await?;
                    content.push(ContentBlock::text(text));
                }
                ModelDirective::ToolCall { call } => {
                    sink.emit(StreamEvent::ToolUseStart {
                        id: call.call_id.clone(),
                        name: call.tool_name.clone(),
                    })
                    .await?;
                    let partial_args = serde_json::to_string(&call.input).unwrap_or_default();
                    sink.emit(StreamEvent::ToolUseInputDelta {
                        id: call.call_id.clone(),
                        partial_args,
                    })
                    .await?;
                    sink.emit(StreamEvent::ToolUseEnd {
                        id: call.call_id.clone(),
                        ok: true,
                        denied: false,
                        error: None,
                    })
                    .await?;
                    content.push(ContentBlock::ToolUse {
                        id: call.call_id.clone(),
                        name: call.tool_name.clone(),
                        input: call.input.clone(),
                    });
                }
            }
        }

        // 5. Usage event (best-effort — the kernel's TokenUsage shape
        //    differs slightly from ergon's; we map the fields we have).
        let usage = match completion.usage {
            Some(t) => {
                let mut u = Usage::default();
                u.input_tokens = t.prompt_tokens;
                u.output_tokens = t.completion_tokens;
                u
            }
            None => Usage::default(),
        };
        sink.emit(StreamEvent::Usage {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached_input: usage.cached_input_tokens,
            reasoning: usage.reasoning_tokens,
        })
        .await?;

        // 6. Map stop reason and emit Done.
        let stop_reason = map_stop_reason(&completion.stop_reason);
        sink.emit(StreamEvent::Done { stop_reason }).await?;

        Ok(ModelResponse::new(content, stop_reason).with_usage(usage))
    }
}

/// Build the kernel-side request from ergon's structured form.
fn build_completion_request(
    req: &ModelRequest,
    session_id: aios_protocol::SessionId,
    branch_id: aios_protocol::BranchId,
    run_id: aios_protocol::RunId,
) -> ModelCompletionRequest {
    // Conversation history = every message except the trailing user
    // turn (which becomes the `objective`). If the last message isn't
    // a user message, we treat the whole history as context and pass
    // an empty objective.
    let mut history_iter = req.messages.iter().peekable();
    let mut history: Vec<ConversationTurn> = Vec::new();
    let mut objective = String::new();

    let messages: Vec<&Message> = history_iter.by_ref().collect();
    if let Some(last) = messages.last() {
        if last.role == MessageRole::User {
            // Pull all but the last into history.
            for msg in &messages[..messages.len() - 1] {
                if let Some(turn) = message_to_turn(msg) {
                    history.push(turn);
                }
            }
            objective = flatten_text(last);
        } else {
            for msg in &messages {
                if let Some(turn) = message_to_turn(msg) {
                    history.push(turn);
                }
            }
        }
    }

    ModelCompletionRequest {
        session_id,
        branch_id,
        run_id,
        step_index: 0,
        objective,
        proposed_tool: None,
        system_prompt: req.system.clone(),
        allowed_tools: if req.tools.is_empty() {
            None
        } else {
            Some(req.tools.iter().map(|t| t.name.clone()).collect())
        },
        conversation_history: history,
    }
}

/// Flatten ergon `Message` content blocks into the flat-string shape
/// the kernel's `ConversationTurn` expects. Tool-use / tool-result
/// blocks are rendered as compact JSON for now — once the port grows
/// a structured-content channel (Spec E), this becomes a 1:1 mapping.
fn message_to_turn(msg: &Message) -> Option<ConversationTurn> {
    let role = match msg.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
        // `MessageRole` is `#[non_exhaustive]`; future variants land
        // as `user` until ergon decides on canonical mapping.
        _ => "user",
    }
    .to_owned();
    let content = flatten_text(msg);
    if content.is_empty() {
        return None;
    }
    Some(ConversationTurn { role, content })
}

fn flatten_text(msg: &Message) -> String {
    let mut out = String::new();
    for block in &msg.content {
        match block {
            ContentBlock::Text { text } => out.push_str(text),
            ContentBlock::Reasoning { text, .. } => out.push_str(text),
            ContentBlock::ToolUse { name, input, .. } => {
                let payload = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_owned());
                out.push_str(&format!("[tool_use {name}({payload})]"));
            }
            ContentBlock::ToolResult {
                call_id,
                output,
                is_error,
            } => {
                let prefix = if *is_error {
                    "tool_error"
                } else {
                    "tool_result"
                };
                let payload = serde_json::to_string(output).unwrap_or_else(|_| "{}".to_owned());
                out.push_str(&format!("[{prefix} {call_id} {payload}]"));
            }
            // `ContentBlock` is `#[non_exhaustive]` — future variants
            // (citations, etc.) flatten to nothing for now; provider
            // still sees them via the structured directives path.
            _ => {}
        }
    }
    out
}

fn map_stop_reason(reason: &ModelStopReason) -> StopReason {
    match reason {
        ModelStopReason::Completed => StopReason::EndTurn,
        ModelStopReason::ToolCall => StopReason::ToolUse,
        ModelStopReason::MaxIterations => StopReason::MaxTokens,
        ModelStopReason::Cancelled => StopReason::Error,
        ModelStopReason::Error => StopReason::Error,
        ModelStopReason::Other(_) => StopReason::Error,
    }
}

/// `aios_protocol::SessionId` and `ergon::SessionId` are the same
/// underlying re-export, but inference often complains — keep this
/// helper as the single source of truth for the conversion.
fn ergon_session_id(id: &aios_protocol::SessionId) -> SessionId {
    id.clone()
}

// Suppress unused-import warning when ToolCall isn't directly used in
// public signatures — kept available for future Streaming impl.
#[allow(dead_code)]
fn _tool_call_typecheck(c: ToolCall) -> ToolCall {
    c
}
