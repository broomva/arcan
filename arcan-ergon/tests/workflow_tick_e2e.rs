//! End-to-end workflow tick test.
//!
//! Builds a real `aios_runtime::KernelRuntime` over file-backed
//! event storage, registers an `ergon::Workflow`, installs the
//! `arcan_ergon::ErgonWorkflowDispatcher`, and submits a
//! `TickKind::Workflow` tick via the kernel's public API. Verifies:
//!
//! 1. The tick succeeds end-to-end (no kernel error).
//! 2. The kernel's emitted event count is greater than the direct-tick
//!    baseline (RunStarted + ergon.workflow_output + Commit phase
//!    transitions, plus whatever the workflow body did).
//! 3. The kernel emits an `ergon.workflow_output` `Custom` event
//!    carrying the workflow's typed JSON output.
//! 4. A `TickKind::Direct` tick still works on the same runtime.
//! 5. A `TickKind::Workflow` referencing an unknown workflow yields a
//!    clear error and does NOT panic.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::{
    ApprovalPort, BranchId, EventStorePort, KernelResult, ModelCompletion, ModelCompletionRequest,
    ModelDirective, ModelProviderPort, ModelRouting, ModelStopReason, PolicyGatePort, PolicySet,
    SessionId, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig, TickInput, TickKind, WorkflowTickDispatcher};
use aios_sandbox::LocalSandboxRunner;
use aios_tools::{ToolDispatcher, ToolRegistry};
use arcan_ergon::runner::WorkflowRunInputs;
use arcan_ergon::{ErgonWorkflowDispatcher, WorkflowRegistry};
use async_trait::async_trait;
use ergon::{ErgonError, Role, StepCtx, Workflow};

/// Tiny fake provider — every workflow that calls
/// `run_inference_streaming` would funnel here. Our test workflow
/// short-circuits before that, but we still wire something usable.
#[derive(Debug, Default)]
struct EchoProvider;

#[async_trait]
impl ModelProviderPort for EchoProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        Ok(ModelCompletion {
            provider: "echo".to_owned(),
            model: "echo-1".to_owned(),
            llm_call_record: None,
            directives: vec![ModelDirective::Message {
                role: "assistant".to_owned(),
                content: format!("echo: {}", request.objective),
            }],
            stop_reason: ModelStopReason::Completed,
            usage: None,
            final_answer: Some(request.objective),
        })
    }
}

/// Test workflow: takes a `Greeting { name }`, returns
/// `Reply { message }`. No model or tool calls — keeps the test
/// hermetic and fast.
#[derive(serde::Deserialize)]
struct Greeting {
    name: String,
}

#[derive(serde::Serialize)]
struct Reply {
    message: String,
}

struct GreeterWorkflow;

#[async_trait]
impl Workflow for GreeterWorkflow {
    type Input = Greeting;
    type Output = Reply;

    fn name(&self) -> &str {
        "test.greeter"
    }

    fn role(&self) -> Role {
        Role::default()
    }

    async fn execute(
        &self,
        _ctx: &mut StepCtx<'_>,
        input: Greeting,
    ) -> std::result::Result<Reply, ErgonError> {
        Ok(Reply {
            message: format!("hello, {}", input.name),
        })
    }
}

fn unique_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("arcan-ergon-{name}-{nanos}"))
}

fn build_runtime_with_dispatcher(root: PathBuf) -> Arc<KernelRuntime> {
    let event_store_backend = Arc::new(FileEventStore::new(root.join("kernel")));
    let journal = Arc::new(EventJournal::new(
        event_store_backend,
        EventStreamHub::new(1024),
    ));
    let event_store: Arc<dyn EventStorePort> = journal;

    let policy_engine = Arc::new(SessionPolicyEngine::new(PolicySet::default()));
    let policy_gate: Arc<dyn PolicyGatePort> = policy_engine.clone();
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ApprovalQueue::default());

    let tool_registry = Arc::new(ToolRegistry::with_core_tools());
    let sandbox = Arc::new(LocalSandboxRunner::new(vec!["echo".to_owned()]));
    let dispatcher = Arc::new(ToolDispatcher::new(tool_registry, policy_engine, sandbox));
    let tool_harness: Arc<dyn ToolHarnessPort> = dispatcher;

    let kernel = KernelRuntime::new(
        RuntimeConfig::new(root),
        event_store,
        Arc::new(EchoProvider),
        tool_harness,
        approvals,
        policy_gate,
    );

    let registry = Arc::new(WorkflowRegistry::new().register(Arc::new(GreeterWorkflow)));
    let inputs = Arc::new(WorkflowRunInputs::empty());
    let workflow_dispatcher: Arc<dyn WorkflowTickDispatcher> =
        Arc::new(ErgonWorkflowDispatcher::new(registry, inputs));

    Arc::new(kernel.with_workflow_dispatcher(workflow_dispatcher))
}

#[tokio::test]
async fn workflow_tick_runs_end_to_end() {
    let runtime = build_runtime_with_dispatcher(unique_root("workflow-e2e"));
    let session_id = SessionId::from_string("workflow-e2e".to_owned());
    runtime
        .create_session_with_id(
            session_id.clone(),
            "tester",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .expect("create session");

    let workflow_input = serde_json::json!({"name": "world"});
    let tick_input = TickInput {
        objective: "say hi".to_owned(),
        proposed_tool: None,
        system_prompt: None,
        allowed_tools: None,
        client_tools: Vec::new(),
        kind: TickKind::Workflow {
            name: "test.greeter".to_owned(),
            input: workflow_input,
        },
    };

    let output = runtime
        .tick_on_branch(&session_id, &BranchId::main(), tick_input)
        .await
        .expect("workflow tick succeeds");

    // The workflow tick body emits at least:
    //   Perceive + Deliberate + DeliberationProposed + StateEstimated +
    //   RunStarted (workflow:test.greeter) + ergon.workflow_output +
    //   Commit + Reflect + Sleep
    // i.e. ≥ 8 events. We assert ≥ 6 to allow the kernel to evolve
    // its phase emissions without churning this test.
    assert!(
        output.events_emitted >= 6,
        "expected ≥6 events for workflow tick, got {}",
        output.events_emitted
    );
}

#[tokio::test]
async fn workflow_tick_emits_output_event() {
    use aios_protocol::EventKind;

    let runtime = build_runtime_with_dispatcher(unique_root("workflow-output"));
    let session_id = SessionId::from_string("workflow-output".to_owned());
    runtime
        .create_session_with_id(
            session_id.clone(),
            "tester",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .expect("create session");

    runtime
        .tick_on_branch(
            &session_id,
            &BranchId::main(),
            TickInput {
                objective: "greet".to_owned(),
                proposed_tool: None,
                system_prompt: None,
                allowed_tools: None,
                client_tools: Vec::new(),
                kind: TickKind::Workflow {
                    name: "test.greeter".to_owned(),
                    input: serde_json::json!({"name": "ergon"}),
                },
            },
        )
        .await
        .expect("tick ok");

    // Read all events on this branch and look for the workflow_output
    // Custom event the kernel emits after dispatch returns.
    let events = runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1024)
        .await
        .expect("read events");

    let workflow_output_event = events.iter().find(|e| {
        matches!(
            &e.kind,
            EventKind::Custom { event_type, .. } if event_type == "ergon.workflow_output"
        )
    });
    assert!(
        workflow_output_event.is_some(),
        "expected ergon.workflow_output event in journal; saw kinds: {:?}",
        events
            .iter()
            .map(|e| event_kind_name(&e.kind))
            .collect::<Vec<_>>()
    );

    // Hard-unwrap rather than if-let so a regression that drops the
    // expected event or changes its shape fails the test loudly
    // instead of silently passing on a no-op assertion.
    let event = workflow_output_event.expect("workflow_output event present");
    let EventKind::Custom { data, .. } = &event.kind else {
        panic!(
            "expected EventKind::Custom for ergon.workflow_output, got {:?}",
            event.kind
        );
    };
    assert_eq!(data["workflow"], "test.greeter");
    assert_eq!(data["output"]["message"], "hello, ergon");
}

#[tokio::test]
async fn direct_tick_still_works_after_dispatcher_registration() {
    let runtime = build_runtime_with_dispatcher(unique_root("direct-baseline"));
    let session_id = SessionId::from_string("direct-baseline".to_owned());
    runtime
        .create_session_with_id(
            session_id.clone(),
            "tester",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .expect("create session");

    // Direct tick — kind defaults to TickKind::Direct.
    let tick_input = TickInput {
        objective: "hello".to_owned(),
        proposed_tool: None,
        system_prompt: None,
        allowed_tools: None,
        client_tools: Vec::new(),
        kind: TickKind::Direct,
    };

    let output = runtime
        .tick_on_branch(&session_id, &BranchId::main(), tick_input)
        .await
        .expect("direct tick still works");
    assert!(output.events_emitted > 0);
}

#[tokio::test]
async fn unknown_workflow_routes_to_run_errored_and_recover_mode() {
    use aios_protocol::{EventKind, OperatingMode};

    let runtime = build_runtime_with_dispatcher(unique_root("unknown-workflow"));
    let session_id = SessionId::from_string("unknown-workflow".to_owned());
    runtime
        .create_session_with_id(
            session_id.clone(),
            "tester",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .expect("create session");

    // Per BRO-1001 + CodeRabbit feedback: dispatch failures (including
    // unknown workflow names) are folded into the kernel's standard
    // terminal lifecycle rather than bubbling out as Err. The tick
    // returns Ok(TickOutput) with mode=Recover, an RunErrored event
    // mentioning the workflow name, and the same Commit/Reflect/Sleep
    // finalize trail every other tick produces.
    let output = runtime
        .tick_on_branch(
            &session_id,
            &BranchId::main(),
            TickInput {
                objective: "ghost".to_owned(),
                proposed_tool: None,
                system_prompt: None,
                allowed_tools: None,
                client_tools: Vec::new(),
                kind: TickKind::Workflow {
                    name: "no.such.workflow".to_owned(),
                    input: serde_json::Value::Null,
                },
            },
        )
        .await
        .expect("tick still completes (errors fold into journal, not bubble out)");

    assert_eq!(
        output.mode,
        OperatingMode::Recover,
        "dispatch error must drop the runtime into Recover"
    );
    assert!(
        output.state.error_streak >= 1,
        "dispatch error must increment error_streak, got {}",
        output.state.error_streak
    );

    let events = runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 1024)
        .await
        .expect("read events");
    let run_errored = events.iter().find_map(|e| match &e.kind {
        EventKind::RunErrored { error } => Some(error.clone()),
        _ => None,
    });
    let error_message = run_errored.expect("RunErrored event present after dispatch failure");
    assert!(
        error_message.contains("no.such.workflow"),
        "RunErrored.error should mention the missing workflow name, got: {error_message}"
    );
}

fn event_kind_name(kind: &aios_protocol::EventKind) -> String {
    use aios_protocol::EventKind::*;
    match kind {
        Custom { event_type, .. } => format!("Custom:{event_type}"),
        other => format!("{other:?}").chars().take(40).collect(),
    }
}

/// Harness Phase-2 gap closure: a fully-wired `WorkflowRunInputs`
/// (stream-sink factory + real budget gate / scorer / attester) drives
/// every adapter during a workflow tick that runs one inference turn.
mod fully_wired {
    use std::sync::atomic::{AtomicU32, Ordering};

    use ergon::{InferenceRequest, Message, ModelRequest, ModelResponse, StreamEvent, StreamSink};
    use ergon_life_hooks::{BudgetGate, ResponseScorer, SoulAttester};
    use tokio::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<StreamEvent>>,
    }

    #[async_trait]
    impl StreamSink for RecordingSink {
        async fn emit(&self, event: StreamEvent) -> std::result::Result<(), ErgonError> {
            self.events.lock().await.push(event);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingGate {
        calls: AtomicU32,
    }

    #[async_trait]
    impl BudgetGate for RecordingGate {
        async fn allow_inference(
            &self,
            _req: &mut ModelRequest,
        ) -> std::result::Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingScorer {
        calls: AtomicU32,
    }

    #[async_trait]
    impl ResponseScorer for RecordingScorer {
        async fn score(
            &self,
            _response: &ModelResponse,
        ) -> std::result::Result<serde_json::Value, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::json!({"recorded": true}))
        }
    }

    #[derive(Default)]
    struct RecordingAttester {
        starts: AtomicU32,
        ends: AtomicU32,
    }

    #[async_trait]
    impl SoulAttester for RecordingAttester {
        async fn sign_session_start(
            &self,
            _session_id: &ergon::SessionId,
            _workflow_name: &str,
        ) -> std::result::Result<(), String> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn sign_session_end(
            &self,
            _session_id: &ergon::SessionId,
            _workflow_name: &str,
            _ok: bool,
        ) -> std::result::Result<(), String> {
            self.ends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Workflow that runs exactly one inference turn so the
    /// pre/post-inference hooks (budget gate + scorer) fire.
    struct InferringWorkflow;

    #[async_trait]
    impl Workflow for InferringWorkflow {
        type Input = Greeting;
        type Output = Reply;

        fn name(&self) -> &str {
            "test.inferring"
        }

        fn role(&self) -> Role {
            Role::default()
        }

        async fn execute(
            &self,
            ctx: &mut StepCtx<'_>,
            input: Greeting,
        ) -> std::result::Result<Reply, ErgonError> {
            ctx.push_message(Message::user_text(format!("greet {}", input.name)));
            let request = InferenceRequest::new("echo-1".to_owned()).with_max_turns(1);
            let response = ctx.run_inference_streaming(&request).await?;
            Ok(Reply {
                message: format!("{} blocks", response.content.len()),
            })
        }
    }

    #[tokio::test]
    async fn fully_wired_inputs_drive_sink_and_real_hooks() {
        let root = unique_root("fully-wired");
        let event_store_backend = Arc::new(FileEventStore::new(root.join("kernel")));
        let journal = Arc::new(EventJournal::new(
            event_store_backend,
            EventStreamHub::new(1024),
        ));
        let event_store: Arc<dyn EventStorePort> = journal;
        let policy_engine = Arc::new(SessionPolicyEngine::new(PolicySet::default()));
        let policy_gate: Arc<dyn PolicyGatePort> = policy_engine.clone();
        let approvals: Arc<dyn ApprovalPort> = Arc::new(ApprovalQueue::default());
        let tool_registry = Arc::new(ToolRegistry::with_core_tools());
        let sandbox = Arc::new(LocalSandboxRunner::new(vec!["echo".to_owned()]));
        let dispatcher = Arc::new(ToolDispatcher::new(tool_registry, policy_engine, sandbox));
        let tool_harness: Arc<dyn ToolHarnessPort> = dispatcher;
        let kernel = KernelRuntime::new(
            RuntimeConfig::new(root),
            event_store,
            Arc::new(EchoProvider),
            tool_harness,
            approvals,
            policy_gate,
        );

        let sink = Arc::new(RecordingSink::default());
        let factory_seen: Arc<std::sync::Mutex<Option<String>>> =
            Arc::new(std::sync::Mutex::new(None));
        let gate = Arc::new(RecordingGate::default());
        let scorer = Arc::new(RecordingScorer::default());
        let attester = Arc::new(RecordingAttester::default());

        let sink_for_factory = Arc::clone(&sink);
        let seen_for_factory = Arc::clone(&factory_seen);
        let inputs = WorkflowRunInputs::empty()
            .with_stream_sink_factory(Arc::new(move |session_id, _branch_id| {
                *seen_for_factory.lock().expect("factory lock") =
                    Some(session_id.as_str().to_owned());
                Arc::clone(&sink_for_factory) as Arc<dyn StreamSink>
            }))
            .with_budget_gate(gate.clone())
            .with_response_scorer(scorer.clone())
            .with_soul_attester(attester.clone());

        let registry = Arc::new(WorkflowRegistry::new().register(Arc::new(InferringWorkflow)));
        let workflow_dispatcher: Arc<dyn WorkflowTickDispatcher> =
            Arc::new(ErgonWorkflowDispatcher::new(registry, Arc::new(inputs)));
        let runtime = Arc::new(kernel.with_workflow_dispatcher(workflow_dispatcher));

        let session_id = SessionId::from_string("fully-wired".to_owned());
        runtime
            .create_session_with_id(
                session_id.clone(),
                "tester",
                PolicySet::default(),
                ModelRouting::default(),
            )
            .await
            .expect("create session");

        runtime
            .tick_on_branch(
                &session_id,
                &BranchId::main(),
                TickInput {
                    objective: "greet".to_owned(),
                    proposed_tool: None,
                    system_prompt: None,
                    allowed_tools: None,
                    client_tools: Vec::new(),
                    kind: TickKind::Workflow {
                        name: "test.inferring".to_owned(),
                        input: serde_json::json!({"name": "wired"}),
                    },
                },
            )
            .await
            .expect("workflow tick succeeds");

        assert_eq!(
            factory_seen.lock().expect("factory lock").as_deref(),
            Some("fully-wired"),
            "sink factory receives the invocation's session id"
        );
        assert!(
            !sink.events.lock().await.is_empty(),
            "stream events reach the host-wired durable sink"
        );
        assert!(
            gate.calls.load(Ordering::SeqCst) >= 1,
            "budget gate consulted on_pre_inference"
        );
        assert!(
            scorer.calls.load(Ordering::SeqCst) >= 1,
            "scorer fired on_post_inference"
        );
        assert_eq!(
            attester.starts.load(Ordering::SeqCst),
            1,
            "session start attested"
        );
        assert_eq!(
            attester.ends.load(Ordering::SeqCst),
            1,
            "session end attested"
        );
    }
}
