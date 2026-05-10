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
//!    `ergon-life-hooks` auto-hooks (each wrapped over the BRO-1001
//!    minimum-viable adapter from [`crate::hooks`]).
//! 4. Create a [`ergon::StepCtx`] with a [`ergon::BufferSink`] (the
//!    tick's stream events accumulate there; the kernel surfaces them
//!    afterwards via the standard journal mechanisms).
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
use aios_runtime::{WorkflowTickInvocation, WorkflowTickOutcome};
use ergon::{AgentRegistry, BufferSink, HookRegistry, RecursionContext, StepCtx, ToolDefinition};
use ergon_life_hooks::{AnimaAttestHook, AutonomicBudgetHook, NousScoreHook, PraxisCapabilityHook};
use std::sync::Arc;

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
        }
    }

    /// Builder: attach an agent registry so `spawn_agent` can resolve
    /// sub-agents.
    #[must_use]
    pub fn with_agent_registry(mut self, registry: Arc<dyn AgentRegistry>) -> Self {
        self.agent_registry = Some(registry);
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
    //    them here after the four auto-hooks.
    let cap_resolver = Arc::new(KernelCapabilityResolver::new(
        invocation.policy_gate.clone(),
        invocation.session_id.clone(),
        inputs.tool_capabilities.clone(),
    ));
    let hook_registry = HookRegistry::default()
        .with(PraxisCapabilityHook::new(cap_resolver))
        .with(AutonomicBudgetHook::new(Arc::new(NoopBudgetGate)))
        .with(NousScoreHook::new(Arc::new(NoopResponseScorer)))
        .with(AnimaAttestHook::new(Arc::new(NoopSoulAttester)));

    // 4. Build the StepCtx. We use a BufferSink — durable / OTel /
    //    upstream sinks (LagoSink, VigilSink, LifegwSink) will be
    //    fanned in via a FanoutSink in the follow-up that lands those
    //    impls (BRO-999b). For BRO-1001 we capture events in memory
    //    and account their count in WorkflowTickOutcome.
    let sink: Arc<BufferSink> = Arc::new(BufferSink::new());
    let trace = tracing::Span::current();
    let mut ctx = StepCtx::new(
        invocation.session_id.clone(),
        entry.name(),
        provider,
        tools,
        Arc::new(hook_registry),
        sink.clone() as Arc<dyn ergon::StreamSink>,
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
    let events_emitted = sink.len().await as u64;

    Ok(WorkflowTickOutcome {
        events_emitted,
        output,
        next_mode: None,
    })
}
