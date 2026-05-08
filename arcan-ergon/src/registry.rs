//! Type-erased workflow registry.
//!
//! `ergon::WorkflowExecutor<W>` is generic over `W: Workflow`, which
//! has associated `Input` / `Output` types. The kernel hands us a
//! `name: String` plus a `serde_json::Value`, so we need a layer that
//! erases the generics and operates on `Value` directly. That's
//! [`BoxedWorkflowExecutor`] — a trait object holding a workflow plus
//! the `serde_json::from_value` / `to_value` boundary translation.
//!
//! Workflow authors register their concrete `Workflow` types via
//! [`WorkflowRegistry::register`]; the registry boxes them and stores
//! them keyed by name.

use crate::error::{AdapterError, Result};
use async_trait::async_trait;
use ergon::{StepCtx, Workflow, WorkflowExecutor};
use std::collections::HashMap;
use std::sync::Arc;

/// Type-erased workflow execution surface.
///
/// `BoxedWorkflowExecutor::run_json` is the JSON-in / JSON-out
/// equivalent of [`ergon::WorkflowExecutor::run`]. It deserializes the
/// concrete `Input` from the supplied JSON, dispatches to the boxed
/// executor, then serializes the typed `Output` back to JSON for the
/// kernel.
///
/// Implementers should not need to write this trait by hand —
/// [`WorkflowRegistry::register`] wraps any `Workflow` automatically.
#[async_trait]
pub trait BoxedWorkflowExecutor: Send + Sync {
    /// Stable workflow name (e.g. `"bookkeeping.promotion-judge"`).
    fn name(&self) -> &str;

    /// Run the workflow against the given context with a JSON input,
    /// returning a JSON output.
    ///
    /// Errors from JSON boundary translation are surfaced as
    /// [`AdapterError::InputDeserialize`] / [`AdapterError::OutputSerialize`];
    /// errors from inside the workflow body surface as
    /// [`AdapterError::Workflow`].
    async fn run_json(
        &self,
        ctx: &mut StepCtx<'_>,
        input: serde_json::Value,
    ) -> Result<serde_json::Value>;
}

/// Internal monomorphization shim: holds a concrete
/// [`WorkflowExecutor<W>`] and adapts it to [`BoxedWorkflowExecutor`].
struct ConcreteWorkflowEntry<W: Workflow> {
    name: String,
    executor: WorkflowExecutor<W>,
}

#[async_trait]
impl<W> BoxedWorkflowExecutor for ConcreteWorkflowEntry<W>
where
    W: Workflow,
    W::Input: for<'de> serde::Deserialize<'de> + Send,
    W::Output: serde::Serialize + Send,
{
    fn name(&self) -> &str {
        &self.name
    }

    async fn run_json(
        &self,
        ctx: &mut StepCtx<'_>,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let typed_input: W::Input =
            serde_json::from_value(input).map_err(|source| AdapterError::InputDeserialize {
                workflow: self.name.clone(),
                source,
            })?;

        let typed_output = self
            .executor
            .run(ctx, typed_input)
            .await
            .map_err(|source| AdapterError::Workflow {
                workflow: self.name.clone(),
                source,
            })?;

        serde_json::to_value(&typed_output).map_err(|source| AdapterError::OutputSerialize {
            workflow: self.name.clone(),
            source,
        })
    }
}

/// String-keyed registry of boxed workflows.
///
/// Workflow authors register concrete `Workflow` impls; the
/// dispatcher looks up the boxed executor by `TickKind::Workflow.name`.
///
/// Construction is `Default::default()`. Registration consumes the
/// registry (builder-style); the assembled [`Arc<WorkflowRegistry>`]
/// is then handed to [`crate::ErgonWorkflowDispatcher::new`].
#[derive(Default)]
pub struct WorkflowRegistry {
    entries: HashMap<String, Arc<dyn BoxedWorkflowExecutor>>,
}

impl WorkflowRegistry {
    /// Construct an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a concrete [`Workflow`] under its `name()`.
    ///
    /// The workflow is wrapped in a [`WorkflowExecutor`] internally so
    /// callers don't need to construct one themselves.
    ///
    /// Returns the same registry for chaining (`reg.register(a).register(b)`).
    /// Panics on duplicate names — registries are typically built once
    /// at adapter construction time, and silent overwrites would mask
    /// configuration bugs.
    pub fn register<W>(mut self, workflow: Arc<W>) -> Self
    where
        W: Workflow + 'static,
        W::Input: for<'de> serde::Deserialize<'de> + Send,
        W::Output: serde::Serialize + Send,
    {
        let name = workflow.name().to_owned();
        if self.entries.contains_key(&name) {
            panic!("workflow `{name}` already registered");
        }
        let entry: Arc<dyn BoxedWorkflowExecutor> = Arc::new(ConcreteWorkflowEntry::<W> {
            name: name.clone(),
            executor: WorkflowExecutor::new(workflow),
        });
        self.entries.insert(name, entry);
        self
    }

    /// Lookup a registered workflow by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn BoxedWorkflowExecutor>> {
        self.entries.get(name).cloned()
    }

    /// All registered names. Used by [`AdapterError::UnknownWorkflow`]
    /// to surface registered names on lookup miss.
    pub fn known_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ergon::{ErgonError, Role};

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
            Ok(format!("echo: {input}"))
        }
    }

    #[test]
    fn registry_records_known_names() {
        let reg = WorkflowRegistry::new().register(Arc::new(EchoWorkflow));
        assert_eq!(reg.known_names(), vec!["echo".to_owned()]);
        assert!(reg.get("echo").is_some());
        assert!(reg.get("missing").is_none());
    }

    #[test]
    #[should_panic(expected = "workflow `echo` already registered")]
    fn duplicate_registration_panics() {
        let _ = WorkflowRegistry::new()
            .register(Arc::new(EchoWorkflow))
            .register(Arc::new(EchoWorkflow));
    }
}
