//! # arcan-prosopon
//!
//! `Pneuma<L0ToExternal>` for Arcan. Subscribes to the runtime's
//! `EventRecord` broadcast, translates each `EventKind` into a
//! `ProsoponEvent`, and publishes envelopes to a `prosopon-daemon`
//! `EnvelopeFanout` for downstream compositors.
//!
//! See `docs/superpowers/plans/2026-04-23-bro-773-arcan-prosopon.md` for the
//! full design and the translation table.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod error;
pub mod state;
pub mod translator;

pub use bridge::ArcanProsoponBridge;
pub use error::BridgeError;
pub use state::TranslationState;
