//! # arcan-ergon — kernel-side adapter that runs `ergon::Workflow` as a tick body
//!
//! This crate is the **substrate side** of the ergon harness. Where
//! `ergon` (and its `ergon-life-hooks` sibling) is vendor-neutral and
//! depends only on `aios-protocol`, this crate translates between
//! ergon's traits and the live kernel ports + Life-substrate types.
//!
//! ## What it exports
//!
//! | Module | Role |
//! |---|---|
//! | [`error`]          | `AdapterError` + `Result` alias for adapter-internal errors |
//! | [`registry`]       | [`registry::WorkflowRegistry`] — string → boxed `WorkflowExecutor` |
//! | [`runtime_handle`] | Implementation of `ergon::RuntimeHandle` over kernel state |
//! | [`provider`]       | `ergon::Provider` over `aios_protocol::ModelProviderPort` |
//! | [`tools`]          | `ergon::ToolRegistry` over `ToolHarnessPort` + `PolicyGatePort` |
//! | [`hooks`]          | The four `ergon-life-hooks` adapter-trait implementations |
//! | [`runner`]         | [`runner::run_workflow_as_tick`] — the actual workflow body executor |
//! | [`dispatcher`]     | [`dispatcher::ErgonWorkflowDispatcher`] — `WorkflowTickDispatcher` impl wired into the kernel |
//!
//! ## Position in the harness stack
//!
//! ```text
//! L5 — Session orchestration (arcand::ConsciousnessActor)
//! L4 — Tick engine (aios_runtime::KernelRuntime)
//! L3.5 — Tick body — direct OR ergon::Workflow      ← THIS CRATE supplies the workflow shape
//! L3 — Port traits (aios-runtime / aios-protocol)
//! L2 — Substrate adapters (incl. arcan-ergon — THIS CRATE)
//! L1 — Substrate primitives (lago, praxis, anima, ...)
//! L0 — Kernel contract (aios-protocol)
//! ```
//!
//! Wiring: arcand constructs `KernelRuntime` and an
//! [`ErgonWorkflowDispatcher`] holding the [`WorkflowRegistry`], then
//! installs the dispatcher via
//! [`aios_runtime::KernelRuntime::with_workflow_dispatcher`]. Every
//! subsequent tick whose `kind == TickKind::Workflow` is routed to the
//! dispatcher; the kernel never sees ergon's traits directly.
//!
//! ## Spec & tracker
//!
//! - Spec: `core/life/docs/superpowers/specs/2026-05-08-bro-1001-ergon-tick-body.md`
//! - Linear: [BRO-1001](https://linear.app/broomva/issue/BRO-1001)

#![doc(html_no_source)]

pub mod dispatcher;
pub mod error;
pub mod hooks;
pub mod provider;
pub mod registry;
pub mod runner;
pub mod runtime_handle;
pub mod tools;

pub use dispatcher::ErgonWorkflowDispatcher;
pub use error::{AdapterError, Result};
pub use hooks::{KernelCapabilityResolver, NoopBudgetGate, NoopResponseScorer, NoopSoulAttester};
pub use provider::ModelProviderAdapter;
pub use registry::{BoxedWorkflowExecutor, WorkflowRegistry};
pub use runner::{WorkflowRunInputs, run_workflow_as_tick};
pub use runtime_handle::ModeRuntimeHandle;
pub use tools::ToolHarnessAdapter;
