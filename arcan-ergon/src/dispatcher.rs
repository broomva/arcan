//! [`aios_runtime::WorkflowTickDispatcher`] implementation that calls
//! [`crate::run_workflow_as_tick`].
//!
//! This is the trait object the host runtime (arcand) installs on the
//! [`aios_runtime::KernelRuntime`] via
//! [`aios_runtime::KernelRuntime::with_workflow_dispatcher`]. It owns
//! the [`crate::WorkflowRegistry`] plus the per-deployment
//! [`crate::runner::WorkflowRunInputs`] (tool definitions + capability
//! map) and routes every `TickKind::Workflow` invocation to the
//! workflow-as-tick runner.

use crate::error::AdapterError;
use crate::registry::WorkflowRegistry;
use crate::runner::{WorkflowRunInputs, run_workflow_as_tick};
use aios_runtime::{WorkflowTickDispatcher, WorkflowTickInvocation, WorkflowTickOutcome};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Workflow dispatcher backed by a [`WorkflowRegistry`].
///
/// Construction requires the registry plus a [`WorkflowRunInputs`]
/// instance (tool definitions + capability map). Both are immutable
/// after the dispatcher is installed; if you need to add workflows or
/// change tool capabilities at runtime, hot-swap the dispatcher with
/// a new [`KernelRuntime::with_workflow_dispatcher`] call.
pub struct ErgonWorkflowDispatcher {
    registry: Arc<WorkflowRegistry>,
    inputs: Arc<WorkflowRunInputs>,
}

impl ErgonWorkflowDispatcher {
    /// Construct a dispatcher.
    pub fn new(registry: Arc<WorkflowRegistry>, inputs: Arc<WorkflowRunInputs>) -> Self {
        Self { registry, inputs }
    }
}

#[async_trait]
impl WorkflowTickDispatcher for ErgonWorkflowDispatcher {
    async fn dispatch(
        &self,
        invocation: WorkflowTickInvocation<'_>,
    ) -> Result<WorkflowTickOutcome> {
        run_workflow_as_tick(&self.registry, &self.inputs, invocation)
            .await
            .map_err(adapter_error_to_anyhow)
    }
}

/// Map adapter errors back to `anyhow::Error` for the kernel
/// boundary. Preserves the original error chain via `Box<dyn Error>`
/// so downstream observability sees the structured cause.
fn adapter_error_to_anyhow(err: AdapterError) -> anyhow::Error {
    anyhow::Error::new(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::WorkflowRegistry;
    use crate::runner::WorkflowRunInputs;
    use aios_protocol::ids::ApprovalId;
    use aios_protocol::ports::EventRecordStream;
    use aios_protocol::{
        ApprovalPort, ApprovalRequest, ApprovalResolution, ApprovalTicket, BranchId, Capability,
        EventRecord, EventStorePort, KernelError, KernelResult, ModelCompletion,
        ModelCompletionRequest, ModelDirective, ModelProviderPort, ModelStopReason,
        PolicyGateDecision, PolicyGatePort, PolicySet, RunId, SessionId, SessionManifest,
        ToolExecutionReport, ToolExecutionRequest, ToolHarnessPort,
    };
    use aios_runtime::WorkflowTickInvocation;
    use async_trait::async_trait;
    use ergon::{ErgonError, Role, StepCtx, Workflow};
    use futures::StreamExt;

    struct EchoWorkflow;

    #[async_trait]
    impl Workflow for EchoWorkflow {
        type Input = String;
        type Output = String;

        fn name(&self) -> &str {
            "echo"
        }

        fn role(&self) -> Role {
            Role::default()
        }

        async fn execute(
            &self,
            _ctx: &mut StepCtx<'_>,
            input: String,
        ) -> std::result::Result<String, ErgonError> {
            Ok(format!("echo:{input}"))
        }
    }

    struct StubProvider;
    #[async_trait]
    impl ModelProviderPort for StubProvider {
        async fn complete(
            &self,
            _request: ModelCompletionRequest,
        ) -> KernelResult<ModelCompletion> {
            Ok(ModelCompletion {
                provider: "stub".into(),
                model: "stub-1".into(),
                llm_call_record: None,
                directives: vec![ModelDirective::Message {
                    role: "assistant".into(),
                    content: "stub".into(),
                }],
                stop_reason: ModelStopReason::Completed,
                usage: None,
                final_answer: None,
            })
        }
    }

    struct StubTools;
    #[async_trait]
    impl ToolHarnessPort for StubTools {
        async fn execute(
            &self,
            _request: ToolExecutionRequest,
        ) -> KernelResult<ToolExecutionReport> {
            Err(KernelError::Runtime("not used".into()))
        }
    }

    struct StubGate;
    #[async_trait]
    impl PolicyGatePort for StubGate {
        async fn evaluate(
            &self,
            _session_id: SessionId,
            requested: Vec<Capability>,
        ) -> KernelResult<PolicyGateDecision> {
            Ok(PolicyGateDecision {
                allowed: requested,
                requires_approval: Vec::new(),
                denied: Vec::new(),
            })
        }

        async fn set_policy(&self, _session_id: SessionId, _policy: PolicySet) -> KernelResult<()> {
            Ok(())
        }
    }

    struct StubStore;
    #[async_trait]
    impl EventStorePort for StubStore {
        async fn append(&self, event: EventRecord) -> KernelResult<EventRecord> {
            Ok(event)
        }

        async fn read(
            &self,
            _session_id: SessionId,
            _branch_id: BranchId,
            _from_sequence: u64,
            _limit: usize,
        ) -> KernelResult<Vec<EventRecord>> {
            Ok(Vec::new())
        }

        async fn head(&self, _session_id: SessionId, _branch_id: BranchId) -> KernelResult<u64> {
            Ok(0)
        }

        async fn subscribe(
            &self,
            _session_id: SessionId,
            _branch_id: BranchId,
            _after_sequence: u64,
        ) -> KernelResult<EventRecordStream> {
            // Empty stream.
            let s = futures::stream::empty().boxed();
            Ok(s)
        }
    }

    fn make_invocation<'a>(
        workflow_name: &'a str,
        workflow_input: &'a serde_json::Value,
        session_id: &'a SessionId,
        branch_id: &'a BranchId,
        run_id: &'a RunId,
        manifest: &'a SessionManifest,
        state: &'a aios_protocol::AgentStateVector,
        provider: &'a Arc<dyn ModelProviderPort>,
        tools: &'a Arc<dyn ToolHarnessPort>,
        gate: &'a Arc<dyn PolicyGatePort>,
        store: &'a Arc<dyn EventStorePort>,
    ) -> WorkflowTickInvocation<'a> {
        WorkflowTickInvocation {
            session_id,
            branch_id,
            run_id,
            manifest,
            state,
            mode: aios_protocol::OperatingMode::Execute,
            workflow_name,
            workflow_input,
            objective: "test objective",
            system_prompt: None,
            allowed_tools: None,
            provider,
            tool_harness: tools,
            policy_gate: gate,
            event_store: store,
        }
    }

    #[tokio::test]
    async fn dispatcher_runs_registered_workflow() {
        let registry = Arc::new(WorkflowRegistry::new().register(Arc::new(EchoWorkflow)));
        let inputs = Arc::new(WorkflowRunInputs::empty());
        let dispatcher = ErgonWorkflowDispatcher::new(registry, inputs);

        let provider: Arc<dyn ModelProviderPort> = Arc::new(StubProvider);
        let tools: Arc<dyn ToolHarnessPort> = Arc::new(StubTools);
        let gate: Arc<dyn PolicyGatePort> = Arc::new(StubGate);
        let store: Arc<dyn EventStorePort> = Arc::new(StubStore);

        let session_id = SessionId::default();
        let branch_id = BranchId::main();
        let run_id = RunId::default();
        let manifest = SessionManifest {
            session_id: session_id.clone(),
            owner: "tester".into(),
            created_at: chrono::Utc::now(),
            workspace_root: "/tmp".into(),
            model_routing: aios_protocol::ModelRouting::default(),
            policy: serde_json::Value::Null,
        };
        let state = aios_protocol::AgentStateVector::default();
        let input = serde_json::json!("hi");

        let invocation = make_invocation(
            "echo",
            &input,
            &session_id,
            &branch_id,
            &run_id,
            &manifest,
            &state,
            &provider,
            &tools,
            &gate,
            &store,
        );

        let outcome = dispatcher.dispatch(invocation).await.expect("dispatch ok");
        assert_eq!(outcome.output, serde_json::json!("echo:hi"));
        assert!(outcome.next_mode.is_none());
    }

    #[tokio::test]
    async fn unknown_workflow_returns_error() {
        let registry = Arc::new(WorkflowRegistry::new());
        let inputs = Arc::new(WorkflowRunInputs::empty());
        let dispatcher = ErgonWorkflowDispatcher::new(registry, inputs);

        let provider: Arc<dyn ModelProviderPort> = Arc::new(StubProvider);
        let tools: Arc<dyn ToolHarnessPort> = Arc::new(StubTools);
        let gate: Arc<dyn PolicyGatePort> = Arc::new(StubGate);
        let store: Arc<dyn EventStorePort> = Arc::new(StubStore);

        let session_id = SessionId::default();
        let branch_id = BranchId::main();
        let run_id = RunId::default();
        let manifest = SessionManifest {
            session_id: session_id.clone(),
            owner: "tester".into(),
            created_at: chrono::Utc::now(),
            workspace_root: "/tmp".into(),
            model_routing: aios_protocol::ModelRouting::default(),
            policy: serde_json::Value::Null,
        };
        let state = aios_protocol::AgentStateVector::default();
        let input = serde_json::Value::Null;

        let invocation = make_invocation(
            "ghost",
            &input,
            &session_id,
            &branch_id,
            &run_id,
            &manifest,
            &state,
            &provider,
            &tools,
            &gate,
            &store,
        );

        let err = dispatcher.dispatch(invocation).await.expect_err("missing");
        assert!(format!("{err}").contains("ghost"));
    }
    // Reference suppressors to keep `Approval*` imports alive even if
    // future port shapes drop them — this test module is the canonical
    // place we exercise the full port surface.
    #[allow(dead_code)]
    fn _unused_suppressors() {
        let _: Option<Arc<dyn ApprovalPort>> = None;
        let _: ApprovalRequest = ApprovalRequest {
            session_id: SessionId::default(),
            call_id: String::new(),
            tool_name: String::new(),
            capability: Capability::new("noop"),
            reason: String::new(),
        };
        let _: ApprovalTicket = ApprovalTicket {
            approval_id: ApprovalId::default(),
            session_id: SessionId::default(),
            call_id: String::new(),
            tool_name: String::new(),
            capability: Capability::new("noop"),
            reason: String::new(),
            created_at: chrono::Utc::now(),
        };
        let _: ApprovalResolution = ApprovalResolution {
            approval_id: ApprovalId::default(),
            approved: false,
            actor: String::new(),
            resolved_at: chrono::Utc::now(),
        };
    }
}
