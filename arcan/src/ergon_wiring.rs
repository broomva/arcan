//! Boot-time wiring of the real ergon auto-hook adapters + durable
//! stream sink into [`arcan_ergon::runner::WorkflowRunInputs`].
//!
//! This module is the host side of the arcan-ergon contract: the
//! library crate stays substrate-free (aios-protocol ports + ergon
//! traits only — see its CLAUDE.md "Don't" list), and the binary —
//! which already holds the lago journal, the Autonomic economic
//! handle, and the Nous registry — constructs the concrete adapters
//! and passes them in via `WorkflowRunInputs` builders. Closes the
//! 2026-05-11 architecture-audit gaps "ergon-life-sinks has zero
//! consumers" and "3 of 4 ergon auto-hook adapters are Noop".

use std::sync::Arc;

use arcan_aios_adapters::EconomicGateHandle;
use arcan_ergon::runner::StreamSinkFactory;
use async_trait::async_trait;
use ergon_life_hooks::BudgetGate;

/// Per-session durable sink factory over the host's lago journal —
/// the same journal the kernel's `EventStorePort`
/// (`LagoAiosEventStoreAdapter`) writes to, so workflow stream events
/// land beside the tick events and are visible to `lago replay`.
/// lago assigns its own sequence numbers on append, so dispatcher-side
/// writes cannot collide with the kernel's per-branch sequence counter
/// (which is why the sink goes through `lago_core::Journal` and NOT
/// through `EventStorePort::append`).
pub fn lago_stream_sink_factory(journal: Arc<dyn lago_core::Journal>) -> StreamSinkFactory {
    Arc::new(move |session_id, branch_id| {
        Arc::new(
            ergon_life_sinks::LagoSink::new(journal.clone(), session_id.clone())
                .with_branch(branch_id.clone()),
        )
    })
}

/// Real [`BudgetGate`] over the Autonomic economic advisory.
///
/// Mirrors the gating semantics the Direct path applies inside
/// `ArcanProviderAdapter` (provider.rs): **Hibernate** denies the
/// inference outright; **Hustle** clamps `max_tokens` to the advisory
/// cap; Sovereign/Conserving (and an absent/unfilled handle) pass
/// through.
///
/// Honest scope (P20 review finding): in arcan serve the
/// `invocation.provider` IS the economically-gated
/// `ArcanProviderAdapter`, which independently re-applies the
/// Hibernate deny and Hustle cap at the port. What this hook adds is
/// (a) the denial becoming hook-visible (`on_pre_inference` outcome +
/// trace) instead of surfacing only as a provider error, and (b)
/// enforcement for hosts whose providers are NOT economically gated.
/// The `max_tokens` clamp mutates the ergon-level `ModelRequest`;
/// today's `ModelProviderAdapter` → `ModelCompletionRequest` bridge
/// does not carry a max-tokens field, so the clamp is advisory at the
/// ergon layer until the port grows one (the port-level cap is what
/// bites in production).
pub struct EconomicBudgetGate {
    handle: EconomicGateHandle,
}

impl EconomicBudgetGate {
    pub fn new(handle: EconomicGateHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl BudgetGate for EconomicBudgetGate {
    async fn allow_inference(
        &self,
        req: &mut ergon::ModelRequest,
    ) -> std::result::Result<(), String> {
        let gates = self.handle.read().await;
        let Some(gates) = gates.as_ref() else {
            // Advisory not yet published — autonomic is advisory by
            // design; absence is never fatal.
            return Ok(());
        };
        use arcan_aios_adapters::autonomic::EconomicMode;
        match gates.economic_mode {
            EconomicMode::Hibernate => {
                tracing::warn!("ergon budget gate: Hibernate mode — denying inference");
                Err("inference blocked: Autonomic Hibernate mode active".to_owned())
            }
            EconomicMode::Hustle => {
                if let Some(cap) = gates.max_tokens_next_turn {
                    let clamped = req.max_tokens.map_or(cap, |m| m.min(cap));
                    tracing::info!(
                        max_tokens = clamped,
                        "ergon budget gate: Hustle mode — capping tokens"
                    );
                    req.max_tokens = Some(clamped);
                }
                Ok(())
            }
            EconomicMode::Sovereign | EconomicMode::Conserving => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use arcan_aios_adapters::autonomic::{EconomicGates, EconomicMode};

    use super::*;

    fn request() -> ergon::ModelRequest {
        let mut req = ergon::ModelRequest::new("test".to_owned(), Vec::new());
        req.max_tokens = Some(4096);
        req
    }

    fn gate_with(mode: EconomicMode, cap: Option<u32>) -> EconomicBudgetGate {
        let handle: EconomicGateHandle = Arc::new(tokio::sync::RwLock::new(Some(EconomicGates {
            economic_mode: mode,
            max_tokens_next_turn: cap,
            preferred_model: None,
            allow_expensive_tools: true,
            allow_replication: false,
        })));
        EconomicBudgetGate::new(handle)
    }

    #[tokio::test]
    async fn empty_handle_allows() {
        let gate = EconomicBudgetGate::new(Arc::new(tokio::sync::RwLock::new(None)));
        let mut req = request();
        assert!(gate.allow_inference(&mut req).await.is_ok());
        assert_eq!(req.max_tokens, Some(4096), "request untouched");
    }

    #[tokio::test]
    async fn hibernate_denies() {
        let gate = gate_with(EconomicMode::Hibernate, None);
        let mut req = request();
        let err = gate.allow_inference(&mut req).await.expect_err("denied");
        assert!(err.contains("Hibernate"));
    }

    #[tokio::test]
    async fn hustle_clamps_max_tokens() {
        let gate = gate_with(EconomicMode::Hustle, Some(512));
        let mut req = request();
        assert!(gate.allow_inference(&mut req).await.is_ok());
        assert_eq!(req.max_tokens, Some(512));
        // A request already below the cap stays put.
        let mut small = request();
        small.max_tokens = Some(128);
        assert!(gate.allow_inference(&mut small).await.is_ok());
        assert_eq!(small.max_tokens, Some(128));
    }

    #[tokio::test]
    async fn sovereign_passes_through() {
        let gate = gate_with(EconomicMode::Sovereign, Some(512));
        let mut req = request();
        assert!(gate.allow_inference(&mut req).await.is_ok());
        assert_eq!(req.max_tokens, Some(4096));
    }
}
