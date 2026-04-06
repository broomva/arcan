//! `arcan-opsis` — Bridge between Arcan agent runtime and Opsis world state engine.
//!
//! Provides bidirectional flow:
//! - **Arcan → Opsis**: Agent events enter world state as observations/alerts
//! - **Opsis → Arcan**: World state flows back into agent reasoning as context
//!
//! Follows the `arcan-lago` / `arcan-spaces` bridge pattern.

mod client;
pub mod injector;
pub mod observer;
pub mod tools;

pub use client::{InjectResponse, OpsisClient, OpsisClientError, OpsisClientResult};
pub use injector::{InjectorThresholds, WorldStateInjector};
pub use observer::{AmbientFilter, ConsciousnessObserver};
pub use tools::register_opsis_tools;
