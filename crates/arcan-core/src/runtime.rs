use crate::error::CoreError;
use crate::protocol::{
    AgentEvent, ChatMessage, ModelDirective, ModelStopReason, ModelTurn, RunStopReason, TokenUsage,
    ToolCall, ToolDefinition, ToolResult, ToolResultSummary,
};
use crate::state::AppState;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct ProviderRequest {
    pub run_id: String,
    pub session_id: String,
    pub iteration: u32,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    pub state: AppState,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError>;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub run_id: String,
    pub session_id: String,
    pub iteration: u32,
}

pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, CoreError>;
}

pub trait Middleware: Send + Sync {
    fn before_model_call(&self, _request: &ProviderRequest) -> Result<(), CoreError> {
        Ok(())
    }

    fn after_model_call(
        &self,
        _request: &ProviderRequest,
        _response: &ModelTurn,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn pre_tool_call(&self, _context: &ToolContext, _call: &ToolCall) -> Result<(), CoreError> {
        Ok(())
    }

    fn post_tool_call(
        &self,
        _context: &ToolContext,
        _result: &ToolResult,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn on_run_finished(&self, _output: &RunOutput) -> Result<(), CoreError> {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools
            .insert(tool.definition().name.clone(), Arc::new(tool));
    }

    pub fn get(&self, tool_name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(tool_name).cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub max_iterations: u32,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self { max_iterations: 24 }
    }
}

#[derive(Debug, Clone)]
pub struct RunInput {
    pub run_id: String,
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    pub state: AppState,
}

#[derive(Debug, Clone)]
pub struct RunOutput {
    pub run_id: String,
    pub session_id: String,
    pub events: Vec<AgentEvent>,
    pub messages: Vec<ChatMessage>,
    pub state: AppState,
    pub reason: RunStopReason,
    pub final_answer: Option<String>,
    /// Accumulated token usage across all iterations.
    pub total_usage: TokenUsage,
}

pub struct Orchestrator {
    provider: Arc<dyn Provider>,
    tools: ToolRegistry,
    middlewares: Vec<Arc<dyn Middleware>>,
    config: OrchestratorConfig,
}

impl Orchestrator {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: ToolRegistry,
        middlewares: Vec<Arc<dyn Middleware>>,
        config: OrchestratorConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            middlewares,
            config,
        }
    }

    pub fn run(&self, input: RunInput, event_handler: impl FnMut(AgentEvent)) -> RunOutput {
        self.run_cancellable(input, None, event_handler)
    }

    /// Run the orchestrator loop with an optional cancellation flag.
    ///
    /// If `cancel` is provided and set to `true` during execution,
    /// the loop will stop at the next iteration boundary.
    pub fn run_cancellable(
        &self,
        input: RunInput,
        cancel: Option<&Arc<AtomicBool>>,
        mut event_handler: impl FnMut(AgentEvent),
    ) -> RunOutput {
        let mut events = Vec::new();
        let mut messages = input.messages;
        let mut state = input.state;
        let mut final_answer: Option<String> = None;
        let mut stop_reason = RunStopReason::BudgetExceeded;
        let mut total_iterations = 0;
        let mut total_usage = TokenUsage::default();

        let start_event = AgentEvent::RunStarted {
            run_id: input.run_id.clone(),
            session_id: input.session_id.clone(),
            provider: self.provider.name().to_string(),
            max_iterations: self.config.max_iterations,
        };
        event_handler(start_event.clone());
        events.push(start_event);

        for iteration in 1..=self.config.max_iterations {
            // Check cancellation at each iteration boundary
            if let Some(flag) = cancel {
                if flag.load(Ordering::Relaxed) {
                    stop_reason = RunStopReason::Cancelled;
                    let err_event = AgentEvent::RunErrored {
                        run_id: input.run_id.clone(),
                        session_id: input.session_id.clone(),
                        error: "run cancelled".to_string(),
                    };
                    event_handler(err_event.clone());
                    events.push(err_event);
                    break;
                }
            }

            total_iterations = iteration;
            let iter_event = AgentEvent::IterationStarted {
                run_id: input.run_id.clone(),
                session_id: input.session_id.clone(),
                iteration,
            };
            event_handler(iter_event.clone());
            events.push(iter_event);

            let provider_request = ProviderRequest {
                run_id: input.run_id.clone(),
                session_id: input.session_id.clone(),
                iteration,
                messages: messages.clone(),
                tools: self.tools.definitions(),
                state: state.clone(),
            };

            if let Err(err) = self.run_before_model(&provider_request) {
                stop_reason = RunStopReason::BlockedByPolicy;
                let err_event = AgentEvent::RunErrored {
                    run_id: input.run_id.clone(),
                    session_id: input.session_id.clone(),
                    error: err.to_string(),
                };
                event_handler(err_event.clone());
                events.push(err_event);
                break;
            }

            let model_turn = match self.provider.complete(&provider_request) {
                Ok(turn) => turn,
                Err(err) => {
                    stop_reason = RunStopReason::Error;
                    let err_event = AgentEvent::RunErrored {
                        run_id: input.run_id.clone(),
                        session_id: input.session_id.clone(),
                        error: err.to_string(),
                    };
                    event_handler(err_event.clone());
                    events.push(err_event);
                    break;
                }
            };

            if let Err(err) = self.run_after_model(&provider_request, &model_turn) {
                stop_reason = RunStopReason::BlockedByPolicy;
                let err_event = AgentEvent::RunErrored {
                    run_id: input.run_id.clone(),
                    session_id: input.session_id.clone(),
                    error: err.to_string(),
                };
                event_handler(err_event.clone());
                events.push(err_event);
                break;
            }

            // Accumulate token usage if reported
            if let Some(ref usage) = model_turn.usage {
                total_usage.accumulate(usage);
            }

            let output_event = AgentEvent::ModelOutput {
                run_id: input.run_id.clone(),
                session_id: input.session_id.clone(),
                iteration,
                stop_reason: model_turn.stop_reason,
                directive_count: model_turn.directives.len(),
                usage: model_turn.usage,
            };
            event_handler(output_event.clone());
            events.push(output_event);

            let mut requested_tool = false;

            for directive in model_turn.directives {
                match directive {
                    ModelDirective::Text { delta } => {
                        let delta_event = AgentEvent::TextDelta {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            iteration,
                            delta: delta.clone(),
                        };
                        event_handler(delta_event.clone());
                        events.push(delta_event);
                        messages.push(ChatMessage::assistant(delta));
                    }
                    ModelDirective::ToolCall { call } => {
                        requested_tool = true;
                        let tc_event = AgentEvent::ToolCallRequested {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            iteration,
                            call: call.clone(),
                        };
                        event_handler(tc_event.clone());
                        events.push(tc_event);

                        let context = ToolContext {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            iteration,
                        };

                        if let Err(err) = self.run_pre_tool(&context, &call) {
                            stop_reason = RunStopReason::BlockedByPolicy;
                            let err_event = AgentEvent::ToolCallFailed {
                                run_id: input.run_id.clone(),
                                session_id: input.session_id.clone(),
                                iteration,
                                call_id: call.call_id.clone(),
                                tool_name: call.tool_name.clone(),
                                error: err.to_string(),
                            };
                            event_handler(err_event.clone());
                            events.push(err_event);
                            break;
                        }

                        let Some(tool) = self.tools.get(&call.tool_name) else {
                            stop_reason = RunStopReason::Error;
                            let err_event = AgentEvent::ToolCallFailed {
                                run_id: input.run_id.clone(),
                                session_id: input.session_id.clone(),
                                iteration,
                                call_id: call.call_id.clone(),
                                tool_name: call.tool_name.clone(),
                                error: format!(
                                    "{}",
                                    CoreError::ToolNotFound {
                                        tool_name: call.tool_name.clone(),
                                    }
                                ),
                            };
                            event_handler(err_event.clone());
                            events.push(err_event);
                            break;
                        };

                        match tool.execute(&call, &context) {
                            Ok(result) => {
                                if let Some(patch) = &result.state_patch {
                                    match state.apply_patch(patch) {
                                        Ok(()) => {
                                            let patch_event = AgentEvent::StatePatched {
                                                run_id: input.run_id.clone(),
                                                session_id: input.session_id.clone(),
                                                iteration,
                                                patch: patch.clone(),
                                                revision: state.revision,
                                            };
                                            event_handler(patch_event.clone());
                                            events.push(patch_event);
                                        }
                                        Err(err) => {
                                            stop_reason = RunStopReason::Error;
                                            let err_event = AgentEvent::ToolCallFailed {
                                                run_id: input.run_id.clone(),
                                                session_id: input.session_id.clone(),
                                                iteration,
                                                call_id: call.call_id.clone(),
                                                tool_name: call.tool_name.clone(),
                                                error: err.to_string(),
                                            };
                                            event_handler(err_event.clone());
                                            events.push(err_event);
                                            break;
                                        }
                                    }
                                }

                                if let Err(err) = self.run_post_tool(&context, &result) {
                                    stop_reason = RunStopReason::BlockedByPolicy;
                                    let err_event = AgentEvent::ToolCallFailed {
                                        run_id: input.run_id.clone(),
                                        session_id: input.session_id.clone(),
                                        iteration,
                                        call_id: call.call_id.clone(),
                                        tool_name: call.tool_name.clone(),
                                        error: err.to_string(),
                                    };
                                    event_handler(err_event.clone());
                                    events.push(err_event);
                                    break;
                                }

                                let completed_event = AgentEvent::ToolCallCompleted {
                                    run_id: input.run_id.clone(),
                                    session_id: input.session_id.clone(),
                                    iteration,
                                    result: ToolResultSummary::from(&result),
                                };
                                event_handler(completed_event.clone());
                                events.push(completed_event);

                                messages.push(ChatMessage::tool_result(
                                    &result.call_id,
                                    serde_json::to_string(&result.output)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                ));
                            }
                            Err(err) => {
                                stop_reason = RunStopReason::Error;
                                let err_event = AgentEvent::ToolCallFailed {
                                    run_id: input.run_id.clone(),
                                    session_id: input.session_id.clone(),
                                    iteration,
                                    call_id: call.call_id.clone(),
                                    tool_name: call.tool_name.clone(),
                                    error: err.to_string(),
                                };
                                event_handler(err_event.clone());
                                events.push(err_event);
                                break;
                            }
                        }
                    }
                    ModelDirective::StatePatch { patch } => match state.apply_patch(&patch) {
                        Ok(()) => {
                            let patch_event = AgentEvent::StatePatched {
                                run_id: input.run_id.clone(),
                                session_id: input.session_id.clone(),
                                iteration,
                                patch: patch.clone(),
                                revision: state.revision,
                            };
                            event_handler(patch_event.clone());
                            events.push(patch_event);
                        }
                        Err(err) => {
                            stop_reason = RunStopReason::Error;
                            let err_event = AgentEvent::RunErrored {
                                run_id: input.run_id.clone(),
                                session_id: input.session_id.clone(),
                                error: err.to_string(),
                            };
                            event_handler(err_event.clone());
                            events.push(err_event);
                            break;
                        }
                    },
                    ModelDirective::FinalAnswer { text } => {
                        final_answer = Some(text.clone());
                        let delta_event = AgentEvent::TextDelta {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            iteration,
                            delta: text.clone(),
                        };
                        event_handler(delta_event.clone());
                        events.push(delta_event);
                        messages.push(ChatMessage::assistant(text));
                    }
                }
            }

            if matches!(
                stop_reason,
                RunStopReason::Error | RunStopReason::BlockedByPolicy | RunStopReason::Cancelled
            ) {
                break;
            }

            match model_turn.stop_reason {
                ModelStopReason::EndTurn => {
                    stop_reason = RunStopReason::Completed;
                    break;
                }
                ModelStopReason::NeedsUser => {
                    stop_reason = RunStopReason::NeedsUser;
                    break;
                }
                ModelStopReason::Safety => {
                    stop_reason = RunStopReason::BlockedByPolicy;
                    break;
                }
                ModelStopReason::ToolUse => {
                    if !requested_tool {
                        stop_reason = RunStopReason::Error;
                        let err_event = AgentEvent::RunErrored {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            error: "model requested tool_use stop reason without tool call"
                                .to_string(),
                        };
                        event_handler(err_event.clone());
                        events.push(err_event);
                        break;
                    }
                }
                ModelStopReason::MaxTokens | ModelStopReason::Unknown => {
                    if !requested_tool {
                        stop_reason = RunStopReason::Error;
                        let err_event = AgentEvent::RunErrored {
                            run_id: input.run_id.clone(),
                            session_id: input.session_id.clone(),
                            error: "model returned non-terminal stop reason without tool call"
                                .to_string(),
                        };
                        event_handler(err_event.clone());
                        events.push(err_event);
                        break;
                    }
                }
            }
        }

        if total_iterations == self.config.max_iterations
            && stop_reason == RunStopReason::BudgetExceeded
        {
            let err_event = AgentEvent::RunErrored {
                run_id: input.run_id.clone(),
                session_id: input.session_id.clone(),
                error: "max iteration budget exceeded".to_string(),
            };
            event_handler(err_event.clone());
            events.push(err_event);
        }

        let finished_event = AgentEvent::RunFinished {
            run_id: input.run_id.clone(),
            session_id: input.session_id.clone(),
            reason: stop_reason,
            total_iterations,
            final_answer: final_answer.clone(),
        };
        event_handler(finished_event.clone());
        events.push(finished_event);

        let output = RunOutput {
            run_id: input.run_id,
            session_id: input.session_id,
            events,
            messages,
            state,
            reason: stop_reason,
            final_answer,
            total_usage,
        };

        let _ = self
            .middlewares
            .iter()
            .try_for_each(|middleware| middleware.on_run_finished(&output));

        output
    }

    fn run_before_model(&self, request: &ProviderRequest) -> Result<(), CoreError> {
        self.middlewares
            .iter()
            .try_for_each(|middleware| middleware.before_model_call(request))
    }

    fn run_after_model(
        &self,
        request: &ProviderRequest,
        response: &ModelTurn,
    ) -> Result<(), CoreError> {
        self.middlewares
            .iter()
            .try_for_each(|middleware| middleware.after_model_call(request, response))
    }

    fn run_pre_tool(&self, context: &ToolContext, call: &ToolCall) -> Result<(), CoreError> {
        self.middlewares
            .iter()
            .try_for_each(|middleware| middleware.pre_tool_call(context, call))
    }

    fn run_post_tool(&self, context: &ToolContext, result: &ToolResult) -> Result<(), CoreError> {
        self.middlewares
            .iter()
            .try_for_each(|middleware| middleware.post_tool_call(context, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ModelDirective, ModelStopReason, ModelTurn, StatePatch, StatePatchFormat, StatePatchSource,
    };
    use serde_json::json;
    use std::sync::Mutex;

    struct ScriptedProvider {
        turns: Vec<ModelTurn>,
        cursor: Mutex<usize>,
    }

    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        fn complete(&self, _request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
            let mut cursor = self
                .cursor
                .lock()
                .map_err(|_| CoreError::Provider("scripted provider lock poisoned".to_string()))?;
            let idx = *cursor;
            let Some(turn) = self.turns.get(idx) else {
                return Err(CoreError::Provider("no scripted turn left".to_string()));
            };
            *cursor += 1;
            Ok(turn.clone())
        }
    }

    struct EchoTool;

    impl Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".to_string(),
                description: "Echoes the provided value".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "value": { "type": "string" } },
                    "required": ["value"]
                }),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: Vec::new(),
                timeout_secs: None,
            }
        }

        fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
            let value = call
                .input
                .get("value")
                .cloned()
                .unwrap_or_else(|| json!(null));
            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: json!({ "echo": value.clone() }),
                content: None,
                is_error: false,
                state_patch: Some(StatePatch {
                    format: StatePatchFormat::MergePatch,
                    patch: json!({ "last_echo": value }),
                    source: StatePatchSource::Tool,
                }),
            })
        }
    }

    #[test]
    fn orchestrator_runs_tool_then_finishes() {
        let provider = ScriptedProvider {
            turns: vec![
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "call-1".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({ "value": "hello" }),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: None,
                },
                ModelTurn {
                    directives: vec![ModelDirective::FinalAnswer {
                        text: "done".to_string(),
                    }],
                    stop_reason: ModelStopReason::EndTurn,
                    usage: None,
                },
            ],
            cursor: Mutex::new(0),
        };

        let mut tools = ToolRegistry::default();
        tools.register(EchoTool);

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            tools,
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "session-1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::Completed);
        assert_eq!(output.final_answer.as_deref(), Some("done"));
        assert_eq!(output.state.revision, 1);
        assert_eq!(output.state.data["last_echo"], "hello");

        assert!(
            output
                .events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolCallCompleted { .. }))
        );
        assert!(output.events.iter().any(|event| matches!(
            event,
            AgentEvent::RunFinished {
                reason: RunStopReason::Completed,
                ..
            }
        )));
    }

    #[test]
    fn provider_error_stops_run() {
        struct FailProvider;
        impl Provider for FailProvider {
            fn name(&self) -> &str {
                "fail"
            }
            fn complete(&self, _request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
                Err(CoreError::Provider("connection refused".to_string()))
            }
        }

        let orchestrator = Orchestrator::new(
            Arc::new(FailProvider),
            ToolRegistry::default(),
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::Error);
        assert!(
            output
                .events
                .iter()
                .any(|e| matches!(e, AgentEvent::RunErrored { .. }))
        );
    }

    #[test]
    fn tool_not_found_stops_run() {
        let provider = ScriptedProvider {
            turns: vec![ModelTurn {
                directives: vec![ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: "c1".to_string(),
                        tool_name: "nonexistent".to_string(),
                        input: json!({}),
                    },
                }],
                stop_reason: ModelStopReason::ToolUse,
                usage: None,
            }],
            cursor: Mutex::new(0),
        };

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            ToolRegistry::default(),
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::Error);
        assert!(
            output
                .events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallFailed { .. }))
        );
    }

    #[test]
    fn middleware_blocks_model_call() {
        struct BlockMiddleware;
        impl Middleware for BlockMiddleware {
            fn before_model_call(&self, _request: &ProviderRequest) -> Result<(), CoreError> {
                Err(CoreError::Middleware("blocked by policy".to_string()))
            }
        }

        let provider = ScriptedProvider {
            turns: vec![ModelTurn {
                directives: vec![ModelDirective::Text {
                    delta: "hi".to_string(),
                }],
                stop_reason: ModelStopReason::EndTurn,
                usage: None,
            }],
            cursor: Mutex::new(0),
        };

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            ToolRegistry::default(),
            vec![Arc::new(BlockMiddleware)],
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::BlockedByPolicy);
    }

    #[test]
    fn budget_exceeded_when_iterations_exhausted() {
        // Provider always returns ToolUse but no tool call directives → continues loop
        // Actually, we need it to keep looping. Use a tool that works, but provider
        // always asks for more.
        let provider = ScriptedProvider {
            turns: vec![
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "c1".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({"value": "1"}),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: None,
                },
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "c2".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({"value": "2"}),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: None,
                },
                // Only 2 turns, but max_iterations = 2, so it exhausts budget
                // 3rd iteration will fail because no more scripted turns
            ],
            cursor: Mutex::new(0),
        };

        let mut tools = ToolRegistry::default();
        tools.register(EchoTool);

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            tools,
            Vec::new(),
            OrchestratorConfig { max_iterations: 2 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::BudgetExceeded);
    }

    #[test]
    fn text_only_response_completes() {
        let provider = ScriptedProvider {
            turns: vec![ModelTurn {
                directives: vec![ModelDirective::Text {
                    delta: "Hello, world!".to_string(),
                }],
                stop_reason: ModelStopReason::EndTurn,
                usage: None,
            }],
            cursor: Mutex::new(0),
        };

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            ToolRegistry::default(),
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("hi")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::Completed);
        assert!(output.messages.iter().any(|m| m.content == "Hello, world!"));
    }

    #[test]
    fn event_handler_receives_all_events() {
        let provider = ScriptedProvider {
            turns: vec![ModelTurn {
                directives: vec![ModelDirective::FinalAnswer {
                    text: "done".to_string(),
                }],
                stop_reason: ModelStopReason::EndTurn,
                usage: None,
            }],
            cursor: Mutex::new(0),
        };

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            ToolRegistry::default(),
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let received = Arc::new(Mutex::new(Vec::new()));
        let received_clone = received.clone();

        orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            move |event| {
                received_clone.lock().unwrap().push(event);
            },
        );

        let events = received.lock().unwrap();
        assert!(events.len() >= 4); // RunStarted, IterationStarted, ModelOutput, TextDelta, RunFinished
        assert!(matches!(events[0], AgentEvent::RunStarted { .. }));
        assert!(matches!(
            events.last().unwrap(),
            AgentEvent::RunFinished { .. }
        ));
    }

    #[test]
    fn tool_result_includes_call_id() {
        let provider = ScriptedProvider {
            turns: vec![
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "my-call-id".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({"value": "test"}),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: None,
                },
                ModelTurn {
                    directives: vec![ModelDirective::FinalAnswer {
                        text: "ok".to_string(),
                    }],
                    stop_reason: ModelStopReason::EndTurn,
                    usage: None,
                },
            ],
            cursor: Mutex::new(0),
        };

        let mut tools = ToolRegistry::default();
        tools.register(EchoTool);

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            tools,
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        // Verify tool result message has the correct call_id
        let tool_msg = output
            .messages
            .iter()
            .find(|m| m.role == crate::protocol::Role::Tool)
            .expect("should have tool message");
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("my-call-id"));
    }

    #[test]
    fn cancellation_stops_run() {
        let provider = ScriptedProvider {
            turns: vec![
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "c1".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({"value": "1"}),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: None,
                },
                ModelTurn {
                    directives: vec![ModelDirective::FinalAnswer {
                        text: "should not reach".to_string(),
                    }],
                    stop_reason: ModelStopReason::EndTurn,
                    usage: None,
                },
            ],
            cursor: Mutex::new(0),
        };

        let mut tools = ToolRegistry::default();
        tools.register(EchoTool);

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            tools,
            Vec::new(),
            OrchestratorConfig { max_iterations: 10 },
        );

        // Set cancellation flag before the second iteration
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();
        let call_count = Arc::new(Mutex::new(0u32));
        let call_count_clone = call_count.clone();

        let output = orchestrator.run_cancellable(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            Some(&cancel_clone),
            move |event| {
                // Cancel after first iteration completes
                if matches!(event, AgentEvent::ToolCallCompleted { .. }) {
                    let mut count = call_count_clone.lock().unwrap();
                    *count += 1;
                    if *count >= 1 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            },
        );

        assert_eq!(output.reason, RunStopReason::Cancelled);
        // Should not have a final answer since we cancelled
        assert!(output.final_answer.is_none());
    }

    #[test]
    fn token_usage_accumulated() {
        let provider = ScriptedProvider {
            turns: vec![
                ModelTurn {
                    directives: vec![ModelDirective::ToolCall {
                        call: ToolCall {
                            call_id: "c1".to_string(),
                            tool_name: "echo".to_string(),
                            input: json!({"value": "hi"}),
                        },
                    }],
                    stop_reason: ModelStopReason::ToolUse,
                    usage: Some(TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    }),
                },
                ModelTurn {
                    directives: vec![ModelDirective::FinalAnswer {
                        text: "done".to_string(),
                    }],
                    stop_reason: ModelStopReason::EndTurn,
                    usage: Some(TokenUsage {
                        input_tokens: 200,
                        output_tokens: 30,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    }),
                },
            ],
            cursor: Mutex::new(0),
        };

        let mut tools = ToolRegistry::default();
        tools.register(EchoTool);

        let orchestrator = Orchestrator::new(
            Arc::new(provider),
            tools,
            Vec::new(),
            OrchestratorConfig { max_iterations: 4 },
        );

        let output = orchestrator.run(
            RunInput {
                run_id: "run-1".to_string(),
                session_id: "s1".to_string(),
                messages: vec![ChatMessage::user("test")],
                state: AppState::default(),
            },
            |_| {},
        );

        assert_eq!(output.reason, RunStopReason::Completed);
        assert_eq!(output.total_usage.input_tokens, 300);
        assert_eq!(output.total_usage.output_tokens, 80);
        assert_eq!(output.total_usage.total(), 380);
    }
}
