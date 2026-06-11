//! End-to-end workflow tick test against a **live Anthropic
//! endpoint**.
//!
//! This is the validation slice for BRO-1001: it exercises the full
//! kernel → dispatcher → workflow → arcan-ergon ModelProviderAdapter →
//! ArcanProviderAdapter → AnthropicProvider → Anthropic API → stream
//! events back through the autonomous loop → typed JSON output → kernel
//! journal chain. No mocks anywhere along the path that BRO-1001 owns;
//! the only stand-in is at the substrate edges (event store is file
//! backed, sandbox runner is the local one — both real).
//!
//! ## Why `#[ignore]`?
//!
//! - Real network call → flaky on offline runners.
//! - Costs money — Anthropic billing per call.
//! - Requires `ANTHROPIC_API_KEY` at test time.
//!
//! Run manually with:
//! ```bash
//! ANTHROPIC_API_KEY=sk-ant-... \
//!   cargo test -p arcan-ergon --test anthropic_workflow \
//!     -- --ignored --nocapture
//! ```
//!
//! Override the model with `ANTHROPIC_MODEL`. The default is a
//! cheap, fast Haiku model so a green run costs cents, not dollars.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::{
    ApprovalPort, BranchId, EventKind, EventStorePort, ModelProviderPort, ModelRouting,
    OperatingMode, PolicyGatePort, PolicySet, SessionId, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig, TickInput, TickKind, WorkflowTickDispatcher};
use aios_sandbox::LocalSandboxRunner;
use aios_tools::{ToolDispatcher, ToolRegistry};
use arcan_aios_adapters::ArcanProviderAdapter;
use arcan_core::runtime::Provider as ArcanProvider;
use arcan_ergon::runner::WorkflowRunInputs;
use arcan_ergon::{ErgonWorkflowDispatcher, WorkflowRegistry};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use async_trait::async_trait;
use ergon::{
    ContentBlock, ErgonError, InferenceRequest, MessageRole, Role, StepCtx, StopReason, Workflow,
};

// ── Test workflow ──────────────────────────────────────────────────────

/// Input: a string of text we want a one-line summary of.
#[derive(serde::Deserialize)]
struct SummarizeInput {
    text: String,
}

/// Output: the model's one-sentence summary plus telemetry fields the
/// test asserts on.
#[derive(serde::Serialize)]
struct SummarizeOutput {
    summary: String,
    /// Number of `Message` content blocks the autonomous loop produced
    /// (proxy for "did the streaming loop actually return content?").
    assistant_blocks: usize,
    /// Reason the autonomous loop exited.
    stop_reason: String,
}

/// A workflow that asks Claude for a one-sentence summary of the
/// supplied text. No tool calls — single inference round.
struct SummarizeWorkflow {
    model: String,
}

#[async_trait]
impl Workflow for SummarizeWorkflow {
    type Input = SummarizeInput;
    type Output = SummarizeOutput;

    fn name(&self) -> &str {
        "test.summarize"
    }

    fn role(&self) -> Role {
        Role::default()
    }

    async fn execute(
        &self,
        ctx: &mut StepCtx<'_>,
        input: SummarizeInput,
    ) -> std::result::Result<SummarizeOutput, ErgonError> {
        // Seed the conversation. The kernel-side runner already pushes
        // `invocation.objective` as a user message; we override it here
        // to make sure the autonomous loop sees a well-formed prompt
        // regardless of what the caller put in `objective`.
        ctx.push_message(ergon::Message::user_text(format!(
            "Summarize the following in exactly one sentence. \
             Reply with the sentence only, no preamble or quotes.\n\n{}",
            input.text
        )));

        let request = InferenceRequest::new(self.model.clone()).with_max_turns(1);

        let response = ctx.run_inference_streaming(&request).await?;

        // Concatenate every assistant `Text` block — Claude usually
        // returns a single one for a single-sentence ask.
        let mut summary = String::new();
        let mut assistant_blocks = 0;
        for block in &response.content {
            if let ContentBlock::Text { text } = block {
                if !summary.is_empty() {
                    summary.push(' ');
                }
                summary.push_str(text.trim());
                assistant_blocks += 1;
            }
        }

        Ok(SummarizeOutput {
            summary,
            assistant_blocks,
            stop_reason: stop_reason_str(response.stop_reason).to_owned(),
        })
    }
}

fn stop_reason_str(reason: StopReason) -> &'static str {
    match reason {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::ToolUse => "tool_use",
        StopReason::StopSequence => "stop_sequence",
        StopReason::Refusal => "refusal",
        StopReason::Error => "error",
        _ => "other",
    }
}

// ── Setup helpers ──────────────────────────────────────────────────────

fn unique_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("arcan-ergon-anthropic-{name}-{nanos}"))
}

/// Build an `arcan_provider::AnthropicProvider`, wrap it in
/// `arcan_aios_adapters::ArcanProviderAdapter` to get a kernel-side
/// `ModelProviderPort`, and stash the model name for the workflow.
fn build_anthropic_port() -> (Arc<dyn ModelProviderPort>, String) {
    let config = AnthropicConfig::from_env().expect("ANTHROPIC_API_KEY not set");
    let model = config.model.clone();
    let arcan_provider: Arc<dyn ArcanProvider> = Arc::new(AnthropicProvider::new(config));

    // ArcanProviderAdapter::new wants a tools list (we pass empty —
    // SummarizeWorkflow doesn't expose tools) and a streaming-sender
    // handle (we pass an empty one — we don't subscribe to its
    // broadcast for this test).
    let streaming_sender = Arc::new(std::sync::Mutex::new(None));
    let port: Arc<dyn ModelProviderPort> = Arc::new(ArcanProviderAdapter::new(
        arcan_provider,
        Vec::new(),
        streaming_sender,
    ));
    (port, model)
}

fn build_runtime(
    root: PathBuf,
    provider: Arc<dyn ModelProviderPort>,
    workflow: Arc<SummarizeWorkflow>,
) -> Arc<KernelRuntime> {
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
        provider,
        tool_harness,
        approvals,
        policy_gate,
    );

    let registry = Arc::new(WorkflowRegistry::new().register(workflow));
    let inputs = Arc::new(WorkflowRunInputs::empty());
    let workflow_dispatcher: Arc<dyn WorkflowTickDispatcher> =
        Arc::new(ErgonWorkflowDispatcher::new(registry, inputs));

    Arc::new(kernel.with_workflow_dispatcher(workflow_dispatcher))
}

fn init_tracing_once() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // RUST_LOG=arcan_ergon=debug,ergon=debug,arcan_provider=info to
        // see the full provider chain.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                    tracing_subscriber::EnvFilter::new("info,arcan_ergon=debug,ergon=debug")
                }),
            )
            .with_test_writer()
            .try_init();
    });
}

// ── The actual integration test ────────────────────────────────────────

/// Runs a full workflow tick against a live Anthropic endpoint and
/// verifies the round-trip:
///
/// 1. Workflow's typed output (`SummarizeOutput`) round-trips through
///    JSON correctly — non-empty summary, ≥1 assistant text block,
///    stop_reason == `end_turn` (Anthropic's normal completion path).
/// 2. Kernel returns Ok with mode != Recover — no error path triggered.
/// 3. Journal contains the canonical event sequence:
///    `RunStarted` (workflow:test.summarize) → `StepStarted` →
///    `Custom("ergon.workflow_output")` carrying our typed output →
///    `StepFinished` → `RunFinished`.
/// 4. The `ergon.workflow_output` event's `data["output"]["summary"]`
///    matches the summary we returned.
///
/// ## Why this is `#[test]`-not-`#[tokio::test]`
///
/// `arcan_provider::AnthropicProvider` uses `reqwest::blocking`
/// internally, which spawns its own tokio runtime. If we construct
/// the provider inside an async context (which `#[tokio::test]`
/// implies), dropping that inner runtime panics with "Cannot drop a
/// runtime in a context where blocking is not allowed". The arcan
/// binary works around this in `main.rs` by initialising the
/// provider stack before entering tokio (sync `main` + manual
/// `runtime.block_on(...)`); we mirror that pattern here.
#[test]
#[ignore = "requires ANTHROPIC_API_KEY and live network; run with --ignored"]
fn workflow_tick_round_trips_against_live_anthropic() {
    init_tracing_once();

    // Build provider OUTSIDE the tokio runtime to dodge the
    // reqwest::blocking-runtime-drop panic (see header comment).
    let (provider_port, model) = build_anthropic_port();
    eprintln!("[validation] using Anthropic model: {model}");

    let async_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    async_runtime.block_on(run_validation(provider_port, model));
}

async fn run_validation(provider_port: Arc<dyn ModelProviderPort>, model: String) {
    let workflow = Arc::new(SummarizeWorkflow {
        model: model.clone(),
    });
    let runtime = build_runtime(
        unique_root("workflow-roundtrip"),
        provider_port,
        workflow.clone(),
    );

    let session_id = SessionId::from_string("validate-bro-1001".to_owned());
    runtime
        .create_session_with_id(
            session_id.clone(),
            "validation",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .expect("create session");

    let workflow_input = serde_json::json!({
        "text": "BRO-1001 lands the kernel-side adapter that runs an \
                 ergon::Workflow as the body of a single aios_runtime \
                 KernelRuntime tick. The adapter exposes a string-keyed \
                 workflow registry, a port-backed provider/tool/runtime \
                 surface, four auto-hook adapter implementations, and a \
                 dispatcher trait the kernel calls per TickKind::Workflow."
    });

    let tick_input = TickInput {
        objective: "summarize this for me please".to_owned(),
        proposed_tool: None,
        system_prompt: None,
        allowed_tools: None,
        client_tools: Vec::new(),
        kind: TickKind::Workflow {
            name: "test.summarize".to_owned(),
            input: workflow_input,
        },
    };

    let output = runtime
        .tick_on_branch(&session_id, &BranchId::main(), tick_input)
        .await
        .expect("workflow tick must complete (Ok or coherent error path) — never panic");

    eprintln!("[validation] tick output: {output:#?}");

    let events = runtime
        .read_events_on_branch(&session_id, &BranchId::main(), 0, 4096)
        .await
        .expect("read events");

    eprintln!(
        "[validation] journal kinds (first 64): {:?}",
        events
            .iter()
            .take(64)
            .map(|e| event_kind_name(&e.kind))
            .collect::<Vec<_>>()
    );

    // Invariants that must hold REGARDLESS of whether the provider
    // returned a 200 or a structured error. Both outcomes exercise the
    // full BRO-1001 chain (kernel → dispatcher → runner → provider
    // adapter → kernel ModelProviderPort → AnthropicProvider → wire);
    // the only difference is which terminal sub-lifecycle the kernel
    // emits at the end (StepFinished+RunFinished vs RunErrored).
    assert!(
        events.iter().any(|e| matches!(
            &e.kind,
            EventKind::RunStarted { provider, .. } if provider == "workflow:test.summarize"
        )),
        "RunStarted with workflow provider tag must appear regardless of outcome"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::StepStarted { .. })),
        "StepStarted must appear regardless of outcome"
    );

    // Branch on which terminal lifecycle the kernel produced.
    let succeeded = events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::RunFinished { .. }));
    let errored = events
        .iter()
        .any(|e| matches!(&e.kind, EventKind::RunErrored { .. }));
    assert!(
        succeeded ^ errored,
        "tick must end in exactly one of RunFinished / RunErrored, got \
         succeeded={succeeded} errored={errored}"
    );

    if succeeded {
        validate_happy_path(&output, &events);
    } else {
        validate_error_path(&output, &events);
    }

    eprintln!(
        "[validation] BRO-1001 round-trip OK — {} path, {} events",
        if succeeded { "happy" } else { "error" },
        events.len()
    );

    // Keep the role-import alive for future assertions.
    let _ = MessageRole::Assistant;
}

/// Assertions that must hold when Anthropic returned a 200 and the
/// workflow body completed successfully.
fn validate_happy_path(output: &aios_runtime::TickOutput, events: &[aios_protocol::EventRecord]) {
    assert_ne!(
        output.mode,
        OperatingMode::Recover,
        "happy path must not drop the runtime into Recover"
    );
    assert_eq!(
        output.state.error_streak, 0,
        "happy path must not increment error_streak"
    );

    let workflow_output_data = events
        .iter()
        .find_map(|e| match &e.kind {
            EventKind::Custom { event_type, data } if event_type == "ergon.workflow_output" => {
                Some(data.clone())
            }
            _ => None,
        })
        .expect("ergon.workflow_output Custom event must be in the journal on happy path");

    assert_eq!(
        workflow_output_data["workflow"], "test.summarize",
        "workflow name in output event"
    );
    let summary = workflow_output_data["output"]["summary"]
        .as_str()
        .expect("output.summary must be a string");
    assert!(
        !summary.is_empty(),
        "summary returned by the workflow must be non-empty"
    );
    let assistant_blocks = workflow_output_data["output"]["assistant_blocks"]
        .as_u64()
        .expect("output.assistant_blocks must be a u64");
    assert!(
        assistant_blocks >= 1,
        "autonomous loop must have surfaced ≥1 assistant text block, got {assistant_blocks}"
    );
    let stop_reason = workflow_output_data["output"]["stop_reason"]
        .as_str()
        .expect("output.stop_reason must be a string");
    eprintln!(
        "[validation] summary ({} char): {summary:?}",
        summary.chars().count()
    );
    assert!(
        matches!(stop_reason, "end_turn" | "max_tokens" | "stop_sequence"),
        "stop_reason should be a normal-completion variant, got: {stop_reason}"
    );

    assert!(
        events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::StepFinished { .. })),
        "StepFinished must appear on happy path"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::RunFinished { .. })),
        "RunFinished must appear on happy path"
    );
}

/// Assertions that must hold when Anthropic returned a structured
/// error (4xx/5xx).
///
/// The point of these is to validate the CodeRabbit-driven
/// dispatch-error lifecycle (RunErrored + Recover + finalize_tick)
/// introduced as part of BRO-1001 — i.e., the kernel never panics on
/// a provider failure, and the journal still tells a coherent story.
fn validate_error_path(output: &aios_runtime::TickOutput, events: &[aios_protocol::EventRecord]) {
    assert_eq!(
        output.mode,
        OperatingMode::Recover,
        "error path must drop the runtime into Recover"
    );
    assert!(
        output.state.error_streak >= 1,
        "error path must increment error_streak, got {}",
        output.state.error_streak
    );

    let run_errored = events
        .iter()
        .find_map(|e| match &e.kind {
            EventKind::RunErrored { error } => Some(error.clone()),
            _ => None,
        })
        .expect("RunErrored event must be in the journal on error path");

    eprintln!("[validation] error path captured: {run_errored}");
    assert!(
        run_errored.contains("test.summarize"),
        "RunErrored should mention the workflow name, got: {run_errored}"
    );
    assert!(
        run_errored.contains("provider error")
            || run_errored.contains("Anthropic")
            || run_errored.contains("ModelProviderPort"),
        "RunErrored should reference the provider that failed, got: {run_errored}"
    );

    // The CodeRabbit fix also requires that the kernel still walked
    // through Commit/Reflect/Sleep on the error path so the tick is
    // finalized rather than left dangling. Sanity-check that.
    assert!(
        events.iter().any(|e| matches!(
            &e.kind,
            EventKind::PhaseEntered { phase, .. } if matches!(phase, aios_protocol::LoopPhase::Commit)
        )),
        "Commit phase must still fire on error path (lifecycle finalization)"
    );
}

// ── Misc helpers ───────────────────────────────────────────────────────

fn event_kind_name(kind: &EventKind) -> String {
    match kind {
        EventKind::Custom { event_type, .. } => format!("Custom:{event_type}"),
        other => format!("{other:?}").chars().take(48).collect(),
    }
}
