pub mod approval;
pub mod autonomic;
pub mod embedded_autonomic;
pub mod gating_middleware;
pub mod policy;
pub mod provider;
pub mod tools;

pub use approval::ArcanApprovalAdapter;
pub use autonomic::{AutonomicPolicyAdapter, EconomicGateHandle, GatingProfileHandle};
pub use embedded_autonomic::EmbeddedAutonomicController;
pub use gating_middleware::{AutonomicGatingMiddleware, AutonomicGatingState};
pub use policy::ArcanPolicyAdapter;
pub use provider::{ArcanProviderAdapter, StreamingSenderHandle};

// Re-export for convenience (the canonical type lives in arcan-core).
pub use arcan_core::runtime::SwappableProviderHandle;
pub use tools::ArcanHarnessAdapter;
