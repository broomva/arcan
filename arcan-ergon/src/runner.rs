//! `run_workflow_as_tick` — the function the kernel-side dispatcher
//! calls per `TickKind::Workflow` invocation.
//!
//! Composes a fully-built [`ergon::StepCtx`] from a
//! [`aios_runtime::WorkflowTickInvocation`]:
//!
//! 1. Resolve the workflow by name in the [`crate::WorkflowRegistry`].
//! 2. Construct port-backed [`crate::ModelProviderAdapter`] +
//!    [`crate::ToolHarnessAdapter`] for this tick.
//! 3. Build a [`ergon::HookRegistry`] holding the four
//!    `ergon-life-hooks` auto-hooks — capability gating always real
//!    ([`KernelCapabilityResolver`]); budget / scorer / attester use
//!    the host-wired adapters from [`WorkflowRunInputs`] when present
//!    and permissive noops otherwise.
//! 4. Create a [`ergon::StepCtx`] whose sink is a [`ergon::BufferSink`]
//!    (feeds `events_emitted` accounting) fanned out with the host's
//!    durable per-session sink (e.g. `ergon_life_sinks::LagoSink`)
//!    when a [`StreamSinkFactory`] is wired.
//! 5. Drive the workflow through [`ergon::WorkflowExecutor::run`] via
//!    the type-erased registry entry's `run_json`.
//! 6. Return a [`aios_runtime::WorkflowTickOutcome`] with the JSON
//!    output and the count of stream events emitted (folded into the
//!    kernel's `events_emitted` accounting).

use crate::error::{AdapterError, Result};
use crate::hooks::{
    KernelCapabilityResolver, NoopBudgetGate, NoopResponseScorer, NoopSoulAttester,
    ToolCapabilityMap,
};
use crate::provider::ModelProviderAdapter;
use crate::registry::{BoxedWorkflowExecutor, WorkflowRegistry};
use crate::runtime_handle::ModeRuntimeHandle;
use crate::tools::ToolHarnessAdapter;
use aios_protocol::{BranchId, SessionId};
use aios_runtime::{WorkflowTickInvocation, WorkflowTickOutcome};
use ergon::{
    AgentRegistry, BufferSink, FanoutSink, HookRegistry, RecursionContext, StepCtx, StreamSink,
    ToolDefinition,
};
use ergon_life_hooks::{
    AnimaAttestHook, AutonomicBudgetHook, BudgetGate, NousScoreHook, PraxisCapabilityHook,
    ResponseScorer, SoulAttester,
};
use std::sync::Arc;

/// Per-invocation stream-sink constructor. arcand wires this at boot
/// with a closure that builds a substrate-coupled sink (e.g.
/// `ergon_life_sinks::LagoSink`) for the tick's session + branch —
/// this is how workflow stream events reach the durable lago journal
/// without `arcan-ergon` taking a `lago-*` dependency (see the crate
/// CLAUDE.md "Don't" list: substrate sinks plumb in via
/// [`WorkflowRunInputs`], not direct deps).
pub type StreamSinkFactory =
    Arc<dyn Fn(&SessionId, &BranchId) -> Arc<dyn StreamSink> + Send + Sync>;

/// Optional construction inputs for [`run_workflow_as_tick`] that
/// arcand wires per-deployment.
pub struct WorkflowRunInputs {
    /// Tool definitions advertised to the model on this tick. arcand
    /// computes this from the registered praxis tools (or any future
    /// ToolHarnessPort that exposes its registry).
    pub tool_definitions: Vec<ToolDefinition>,
    /// Capability map used by [`KernelCapabilityResolver`]. Maps
    /// each tool's name to its required [`aios_protocol::Capability`]
    /// tokens. Tools missing from the map are denied fail-closed.
    pub tool_capabilities: ToolCapabilityMap,
    /// Optional agent registry for `spawn_agent` tool dispatch
    /// (BRO-1007b). When set, the model can invoke registered
    /// sub-agents from within its autonomous loop. When `None`,
    /// `spawn_agent` calls return a model-visible
    /// `no_registry_configured` error.
    pub agent_registry: Option<Arc<dyn AgentRegistry>>,
    /// Recursion guardrail policy for spawn_agent (BRO-1007b).
    /// Workflow ticks always create a fresh root [`RecursionContext`]
    /// from these limits — there's no per-tick state to carry over,
    /// since each tick is a new bounded computation.
    pub max_recursion_depth: u32,
    /// Cap on total agent invocations within a single workflow tick
    /// (top-level + descendants). Default
    /// [`ergon::DEFAULT_MAX_INVOCATIONS`].
    pub max_invocations: u32,
    /// Optional per-invocation durable stream sink constructor. When
    /// set, the tick's stream events fan out to the constructed sink
    /// in addition to the in-memory buffer (closing the audited
    /// "workflow stream events never reach lago" gap). When `None`
    /// (tests, minimal embedders), events stay buffer-only.
    pub stream_sink_factory: Option<StreamSinkFactory>,
    /// Optional real budget gate consulted `on_pre_inference`. When
    /// `None`, the permissive [`NoopBudgetGate`] stands in.
    pub budget_gate: Option<Arc<dyn BudgetGate>>,
    /// Optional real response scorer fired `on_post_inference` (e.g.
    /// `ergon_nous_adapter::NousAdapter`). When `None`, the
    /// [`NoopResponseScorer`] stands in.
    pub response_scorer: Option<Arc<dyn ResponseScorer>>,
    /// Optional real soul attester fired at workflow start/end (e.g.
    /// `ergon_anima_adapter::AgentAttestationAdapter`). When `None`,
    /// the [`NoopSoulAttester`] stands in.
    pub soul_attester: Option<Arc<dyn SoulAttester>>,
}

impl WorkflowRunInputs {
    /// Construct minimal inputs (no tools advertised, empty capability
    /// map, no agent registry) — useful for workflows that only call
    /// the model.
    pub fn empty() -> Self {
        Self {
            tool_definitions: Vec::new(),
            tool_capabilities: ToolCapabilityMap::new(),
            agent_registry: None,
            max_recursion_depth: ergon::DEFAULT_MAX_RECURSION_DEPTH,
            max_invocations: ergon::DEFAULT_MAX_INVOCATIONS,
            stream_sink_factory: None,
            budget_gate: None,
            response_scorer: None,
            soul_attester: None,
        }
    }

    /// Builder: attach an agent registry so `spawn_agent` can resolve
    /// sub-agents.
    #[must_use]
    pub fn with_agent_registry(mut self, registry: Arc<dyn AgentRegistry>) -> Self {
        self.agent_registry = Some(registry);
        self
    }

    /// Builder: attach a per-invocation durable stream-sink factory
    /// (e.g. one constructing `ergon_life_sinks::LagoSink` over the
    /// host's lago journal).
    #[must_use]
    pub fn with_stream_sink_factory(mut self, factory: StreamSinkFactory) -> Self {
        self.stream_sink_factory = Some(factory);
        self
    }

    /// Builder: attach a real budget gate (replaces [`NoopBudgetGate`]).
    #[must_use]
    pub fn with_budget_gate(mut self, gate: Arc<dyn BudgetGate>) -> Self {
        self.budget_gate = Some(gate);
        self
    }

    /// Builder: attach a real response scorer (replaces
    /// [`NoopResponseScorer`]).
    #[must_use]
    pub fn with_response_scorer(mut self, scorer: Arc<dyn ResponseScorer>) -> Self {
        self.response_scorer = Some(scorer);
        self
    }

    /// Builder: attach a real soul attester (replaces
    /// [`NoopSoulAttester`]).
    #[must_use]
    pub fn with_soul_attester(mut self, attester: Arc<dyn SoulAttester>) -> Self {
        self.soul_attester = Some(attester);
        self
    }

    /// Builder: cap recursion depth for spawn_agent (default 8).
    #[must_use]
    pub fn with_max_recursion_depth(mut self, depth: u32) -> Self {
        self.max_recursion_depth = depth;
        self
    }

    /// Builder: cap total agent invocations per workflow tick.
    #[must_use]
    pub fn with_max_invocations(mut self, n: u32) -> Self {
        self.max_invocations = n;
        self
    }
}

/// Run a workflow as a tick body.
///
/// `registry` is the global workflow registry, looked up by
/// `invocation.workflow_name`. `inputs` carries per-tick configuration
/// the registry doesn't itself own (tool definitions, capability map);
/// arcand assembles these once at startup and passes them per tick.
pub async fn run_workflow_as_tick(
    registry: &WorkflowRegistry,
    inputs: &WorkflowRunInputs,
    invocation: WorkflowTickInvocation<'_>,
) -> Result<WorkflowTickOutcome> {
    // 1. Resolve the workflow.
    let entry: Arc<dyn BoxedWorkflowExecutor> =
        registry
            .get(invocation.workflow_name)
            .ok_or_else(|| AdapterError::UnknownWorkflow {
                name: invocation.workflow_name.to_owned(),
                known: registry.known_names(),
            })?;

    // 2. Build the port-backed adapters.
    let provider = Arc::new(ModelProviderAdapter::new(
        invocation.provider.clone(),
        invocation.session_id.clone(),
        invocation.branch_id.clone(),
        invocation.run_id.clone(),
        "kernel",
    ));
    let tools = Arc::new(ToolHarnessAdapter::new(
        invocation.tool_harness.clone(),
        invocation.session_id.clone(),
        invocation.manifest.workspace_root.clone(),
        inputs.tool_definitions.clone(),
    ));
    let runtime: Arc<dyn ergon::RuntimeHandle> = Arc::new(ModeRuntimeHandle::new(invocation.mode));

    // 3. Assemble the auto-hook registry. Order is significant: the
    //    spec mandates auto-hooks fire BEFORE user hooks. User-supplied
    //    hooks aren't in scope for BRO-1001; once they are, append
    //    them here after the four auto-hooks. The budget / scorer /
    //    attester slots use the host-wired real adapters when present
    //    (see [`WorkflowRunInputs`]) and fall back to the permissive
    //    noops otherwise (tests, minimal embedders).
    let cap_resolver = Arc::new(KernelCapabilityResolver::new(
        invocation.policy_gate.clone(),
        invocation.session_id.clone(),
        inputs.tool_capabilities.clone(),
    ));
    let budget_gate: Arc<dyn BudgetGate> = inputs
        .budget_gate
        .clone()
        .unwrap_or_else(|| Arc::new(NoopBudgetGate));
    let response_scorer: Arc<dyn ResponseScorer> = inputs
        .response_scorer
        .clone()
        .unwrap_or_else(|| Arc::new(NoopResponseScorer));
    let soul_attester: Arc<dyn SoulAttester> = inputs
        .soul_attester
        .clone()
        .unwrap_or_else(|| Arc::new(NoopSoulAttester));
    let hook_registry = HookRegistry::default()
        .with(PraxisCapabilityHook::new(cap_resolver))
        .with(AutonomicBudgetHook::new(budget_gate))
        .with(NousScoreHook::new(response_scorer))
        .with(AnimaAttestHook::new(soul_attester));

    // 4. Build the StepCtx sink. The in-memory buffer always runs (it
    //    feeds `WorkflowTickOutcome.events_emitted`); when the host
    //    wired a durable sink factory, fan events out to the
    //    constructed per-session sink as well — this is how workflow
    //    stream events reach the lago journal (`lago replay --tree`
    //    visibility). Durable-sink failures short-circuit the fanout
    //    by design (backpressure + durability-first, see
    //    `ergon::FanoutSink`).
    let buffer: Arc<BufferSink> = Arc::new(BufferSink::new());
    let sink: Arc<dyn StreamSink> = match inputs.stream_sink_factory.as_ref() {
        Some(factory) => Arc::new(FanoutSink::new(vec![
            buffer.clone() as Arc<dyn StreamSink>,
            factory(invocation.session_id, invocation.branch_id),
        ])),
        None => buffer.clone() as Arc<dyn StreamSink>,
    };
    let trace = tracing::Span::current();
    let mut ctx = StepCtx::new(
        invocation.session_id.clone(),
        entry.name(),
        provider,
        tools,
        Arc::new(hook_registry),
        sink,
        runtime,
        trace,
    );

    // Attach the spawn-agent substrate (BRO-1007b). Always create a
    // fresh root recursion context per tick — recursion limits are
    // per-tick, not per-session. If no agent registry is configured,
    // spawn_agent calls fail-closed with a model-visible error.
    let recursion = RecursionContext::root()
        .with_max_depth(inputs.max_recursion_depth)
        .with_max_invocations(inputs.max_invocations);
    ctx = ctx.with_recursion(recursion);
    if let Some(registry) = inputs.agent_registry.as_ref() {
        ctx = ctx.with_agent_registry(Arc::clone(registry));
    }

    // Optional: seed the autonomous loop with the user's objective so
    // workflows whose execute() body calls run_inference_streaming()
    // immediately get a populated history.
    if !invocation.objective.is_empty() {
        ctx.push_message(ergon::Message::user_text(invocation.objective));
    }

    // 5. Run the workflow.
    let output = entry
        .run_json(&mut ctx, invocation.workflow_input.clone())
        .await?;

    // 6. Tally stream events for the kernel's accounting.
    let events_emitted = buffer.len().await as u64;

    Ok(WorkflowTickOutcome {
        events_emitted,
        output,
        next_mode: None,
    })
}
