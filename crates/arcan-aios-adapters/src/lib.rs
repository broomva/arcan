pub mod approval;
pub mod autonomic;
pub mod capability_map;
pub mod embedded_autonomic;
pub mod gating_middleware;
#[cfg(feature = "haima")]
pub mod haima_middleware;
pub mod policy;
pub mod provider;
pub mod sandbox;
pub mod tools;

pub use approval::ArcanApprovalAdapter;
pub use autonomic::{AutonomicPolicyAdapter, EconomicGateHandle, GatingProfileHandle};
pub use capability_map::tools_allowed_by_policy;
pub use embedded_autonomic::EmbeddedAutonomicController;
pub use gating_middleware::{AutonomicGatingMiddleware, AutonomicGatingState};
#[cfg(feature = "haima")]
pub use haima_middleware::HaimaPaymentMiddleware;
pub use policy::ArcanPolicyAdapter;
pub use provider::{ArcanProviderAdapter, StreamingSenderHandle};
pub use sandbox::SandboxEnforcer;

// Re-export for convenience (the canonical type lives in arcan-core).
pub use arcan_core::runtime::SwappableProviderHandle;
pub use tools::ArcanHarnessAdapter;
