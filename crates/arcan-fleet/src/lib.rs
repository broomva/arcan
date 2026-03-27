//! Vertical agent fleet for the Life marketplace.
//!
//! Defines three production agent verticals — Coding, Data Processing, and Support —
//! each with a complete profile: system prompt, tool permissions, Haima task contracts,
//! and Spaces marketplace listing data.
//!
//! # Architecture
//!
//! Each vertical is a [`VerticalConfig`] containing everything needed to instantiate
//! and register an agent:
//!
//! - **Persona** — system prompt defining the agent's identity and behavior
//! - **Tools** — which Praxis tools are enabled (e.g., coding gets `edit_file`, data gets `bash`)
//! - **Contract** — Haima outcome-based pricing (price range, success criteria, SLA)
//! - **Listing** — Spaces marketplace registration data (skills, pricing model, capabilities)
//! - **Health thresholds** — Autonomic health reporting configuration
//!
//! # Usage
//!
//! ```rust
//! use arcan_fleet::{AgentVertical, VerticalConfig};
//!
//! let config = VerticalConfig::for_vertical(AgentVertical::Coding);
//! assert_eq!(config.agent_id(), "life-coding-agent-v1");
//! assert!(!config.persona().is_empty());
//! ```

pub mod coding;
pub mod contracts;
pub mod data;
pub mod health;
pub mod listing;
pub mod support;
pub mod vertical;

pub use vertical::{AgentVertical, ToolPermissions, VerticalConfig};

/// Return configs for all three verticals.
pub fn all_verticals() -> Vec<VerticalConfig> {
    vec![
        VerticalConfig::for_vertical(AgentVertical::Coding),
        VerticalConfig::for_vertical(AgentVertical::DataProcessing),
        VerticalConfig::for_vertical(AgentVertical::Support),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_verticals_returns_three() {
        let configs = all_verticals();
        assert_eq!(configs.len(), 3);
    }

    #[test]
    fn each_vertical_has_unique_agent_id() {
        let configs = all_verticals();
        let ids: Vec<&str> = configs.iter().map(|c| c.agent_id()).collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len(), "agent IDs must be unique");
    }

    #[test]
    fn each_vertical_has_persona() {
        for config in all_verticals() {
            assert!(
                !config.persona().is_empty(),
                "{:?} has empty persona",
                config.vertical
            );
        }
    }

    #[test]
    fn each_vertical_has_contract() {
        for config in all_verticals() {
            let contract = config.contract();
            assert!(
                contract.price_floor_micro_credits > 0,
                "{:?} has zero floor price",
                config.vertical
            );
            assert!(
                contract.price_floor_micro_credits <= contract.price_ceiling_micro_credits,
                "{:?} has floor > ceiling",
                config.vertical
            );
        }
    }

    #[test]
    fn each_vertical_has_listing() {
        for config in all_verticals() {
            let listing = config.listing();
            assert!(
                !listing.name.is_empty(),
                "{:?} has empty listing name",
                config.vertical
            );
            assert!(
                !listing.skills.is_empty(),
                "{:?} has no skills",
                config.vertical
            );
        }
    }
}
