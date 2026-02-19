use arcan_core::error::CoreError;
use arcan_core::protocol::{AgentEvent, ModelDirective, ModelStopReason, ModelTurn, RunStopReason};
use arcan_core::runtime::{
    Orchestrator, OrchestratorConfig, Provider, ProviderRequest, ToolRegistry,
};
use arcan_store::session::{InMemorySessionRepository, SessionRepository};
use arcand::r#loop::AgentLoop;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Simple mock provider that echoes user messages.
struct EchoProvider;

impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let last = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();

        Ok(ModelTurn {
            directives: vec![ModelDirective::Text {
                delta: format!("Echo: {last}"),
            }],
            stop_reason: ModelStopReason::EndTurn,
            usage: None,
        })
    }
}

/// Provider that always returns ToolUse then text on second call.
struct ToolThenTextProvider {
    call_count: std::sync::atomic::AtomicU32,
}

impl ToolThenTextProvider {
    fn new() -> Self {
        Self {
            call_count: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

impl Provider for ToolThenTextProvider {
    fn name(&self) -> &str {
        "tool-then-text"
    }

    fn complete(&self, _request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if count == 0 {
            Ok(ModelTurn {
                directives: vec![ModelDirective::ToolCall {
                    call: arcan_core::protocol::ToolCall {
                        call_id: "call-1".to_string(),
                        tool_name: "nonexistent_tool".to_string(),
                        input: serde_json::json!({}),
                    },
                }],
                stop_reason: ModelStopReason::ToolUse,
                usage: None,
            })
        } else {
            Ok(ModelTurn {
                directives: vec![ModelDirective::Text {
                    delta: "Done after tool.".to_string(),
                }],
                stop_reason: ModelStopReason::EndTurn,
                usage: None,
            })
        }
    }
}

fn build_agent_loop(provider: impl Provider + 'static) -> AgentLoop {
    let repo = Arc::new(InMemorySessionRepository::default());
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(provider),
        ToolRegistry::default(),
        Vec::new(),
        OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    AgentLoop::new(repo, orchestrator)
}

async fn collect_events(mut rx: mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    events
}

#[tokio::test]
async fn agent_loop_echo_produces_correct_event_sequence() {
    let agent_loop = build_agent_loop(EchoProvider);
    let (tx, rx) = mpsc::channel(100);

    let output = agent_loop
        .run("session-1", "main", "Hello".to_string(), tx)
        .await;
    assert!(output.is_ok());

    let events = collect_events(rx).await;

    // Verify event sequence: RunStarted → IterationStarted → TextDelta → ModelOutput → RunFinished
    assert!(
        matches!(&events[0], AgentEvent::RunStarted { .. }),
        "First event should be RunStarted, got {:?}",
        events[0]
    );

    let has_text_delta = events
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta { .. }));
    assert!(has_text_delta, "Should have at least one TextDelta event");

    let last = events.last().expect("should have events");
    assert!(
        matches!(
            last,
            AgentEvent::RunFinished {
                reason: RunStopReason::Completed,
                ..
            }
        ),
        "Last event should be RunFinished(Completed), got {:?}",
        last
    );

    // Verify the text content
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains("Echo: Hello"),
        "Response should echo input, got: {text}"
    );
}

#[tokio::test]
async fn agent_loop_persists_events_to_repo() {
    let repo = Arc::new(InMemorySessionRepository::default());
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(EchoProvider),
        ToolRegistry::default(),
        Vec::new(),
        OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    let agent_loop = AgentLoop::new(repo.clone(), orchestrator);
    let (tx, rx) = mpsc::channel(100);

    agent_loop
        .run("persist-test", "main", "Hello".to_string(), tx)
        .await
        .unwrap();

    // Drain receiver so we don't block
    drop(rx);

    // Verify events were persisted
    let records = repo.load_session("persist-test", "main").unwrap();
    assert!(!records.is_empty(), "Session should have persisted events");

    // Verify first persisted event is RunStarted
    assert!(matches!(&records[0].event, AgentEvent::RunStarted { .. }));

    // Verify last persisted event is RunFinished
    let last = &records.last().unwrap().event;
    assert!(matches!(last, AgentEvent::RunFinished { .. }));
}

#[tokio::test]
async fn agent_loop_continues_from_existing_session() {
    let repo = Arc::new(InMemorySessionRepository::default());
    let orchestrator = Arc::new(Orchestrator::new(
        Arc::new(EchoProvider),
        ToolRegistry::default(),
        Vec::new(),
        OrchestratorConfig {
            max_iterations: 10,
            context: None,
            context_compiler: None,
        },
    ));
    let agent_loop = AgentLoop::new(repo.clone(), orchestrator);

    // First run
    let (tx1, rx1) = mpsc::channel(100);
    agent_loop
        .run("multi-turn", "main", "First message".to_string(), tx1)
        .await
        .unwrap();
    drop(rx1);

    // Second run — should load history from first run
    let (tx2, rx2) = mpsc::channel(100);
    agent_loop
        .run("multi-turn", "main", "Second message".to_string(), tx2)
        .await
        .unwrap();

    let events = collect_events(rx2).await;

    // The echo should contain "Second message" (the latest input)
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert!(
        text.contains("Second message"),
        "Should echo the second message, got: {text}"
    );
}

#[tokio::test]
async fn agent_loop_handles_tool_not_found() {
    let agent_loop = build_agent_loop(ToolThenTextProvider::new());
    let (tx, rx) = mpsc::channel(100);

    let output = agent_loop
        .run("tool-test", "main", "trigger tool".to_string(), tx)
        .await;
    assert!(output.is_ok());

    let events = collect_events(rx).await;

    // Should have a ToolCallFailed event since tool doesn't exist
    let has_tool_failed = events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolCallFailed { .. }));
    assert!(has_tool_failed, "Should have ToolCallFailed event");
}

#[tokio::test]
async fn agent_loop_tool_not_found_produces_error_stop() {
    // When the provider requests a tool that doesn't exist,
    // the orchestrator should stop with Error reason.
    let agent_loop = build_agent_loop(ToolThenTextProvider::new());
    let (tx, rx) = mpsc::channel(100);

    agent_loop
        .run("error-stop", "main", "trigger tool".to_string(), tx)
        .await
        .unwrap();

    let events = collect_events(rx).await;

    // Should end with Error stop reason (tool not found is fatal)
    let last = events.last().expect("should have events");
    assert!(
        matches!(
            last,
            AgentEvent::RunFinished {
                reason: RunStopReason::Error,
                ..
            }
        ),
        "Should end with Error, got: {:?}",
        last
    );
}
