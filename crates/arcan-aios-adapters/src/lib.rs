pub mod approval;
pub mod policy;
pub mod provider;
pub mod tools;

pub use approval::ArcanApprovalAdapter;
pub use policy::ArcanPolicyAdapter;
pub use provider::{ArcanProviderAdapter, StreamingSenderHandle};

// Re-export for convenience (the canonical type lives in arcan-core).
pub use arcan_core::runtime::SwappableProviderHandle;
pub use tools::ArcanHarnessAdapter;
