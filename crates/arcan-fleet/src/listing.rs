//! Spaces marketplace listing data for each agent vertical.
//!
//! These structs map directly to the `AgentListing` and `AgentSkill` tables
//! in the Spaces SpacetimeDB module. The bootstrap script converts them into
//! `register_listing` / `add_skill` reducer calls.

use crate::vertical::AgentVertical;
use serde::{Deserialize, Serialize};

/// Marketplace listing data for a vertical.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceListing {
    /// Unique agent identifier (matches `AgentListing.agent_id`).
    pub agent_id: String,
    /// Human-readable name.
    pub name: String,
    /// Description shown in the marketplace.
    pub description: String,
    /// Service endpoint URL (where A2A tasks are sent).
    pub url: String,
    /// Agent version string.
    pub version: String,
    /// Provider organization name.
    pub provider_name: String,
    /// Provider URL.
    pub provider_url: String,
    /// Documentation URL.
    pub documentation_url: String,
    /// Pricing model: "pay_per_use", "free", "subscription", "custom".
    pub pricing_model: String,
    /// Price per request in microdollars (for pay_per_use).
    pub price_per_request_micro: i64,
    /// Currency code.
    pub currency: String,
    /// Listing tier: "free", "premium", "featured".
    pub tier: String,
    /// Input MIME types (comma-separated).
    pub input_modes: String,
    /// Output MIME types (comma-separated).
    pub output_modes: String,
    /// Whether the agent supports streaming responses.
    pub supports_streaming: bool,
    /// Skills offered by this agent.
    pub skills: Vec<AgentSkillData>,
}

/// A single skill offered by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkillData {
    /// Unique skill ID within this agent.
    pub skill_id: String,
    /// Human-readable skill name.
    pub name: String,
    /// Skill description.
    pub description: String,
    /// Comma-separated tags for discovery.
    pub tags: String,
    /// Pipe-separated example use cases.
    pub examples: String,
    /// Input MIME types for this skill.
    pub input_modes: String,
    /// Output MIME types for this skill.
    pub output_modes: String,
}

/// Build the marketplace listing for a vertical.
pub fn listing_for(vertical: AgentVertical) -> MarketplaceListing {
    match vertical {
        AgentVertical::Coding => coding_listing(),
        AgentVertical::DataProcessing => data_listing(),
        AgentVertical::Support => support_listing(),
    }
}

fn coding_listing() -> MarketplaceListing {
    MarketplaceListing {
        agent_id: "life-coding-agent-v1".into(),
        name: "Life Coding Agent".into(),
        description: "Expert code review, bug fixing, refactoring, and test writing. \
            Supports Rust, TypeScript, Python, Go, and more. Outcome-priced per task."
            .into(),
        url: "https://fleet.broomva.tech/agents/coding".into(),
        version: "1.0.0".into(),
        provider_name: "Broomva Tech".into(),
        provider_url: "https://broomva.tech".into(),
        documentation_url: "https://docs.broomva.tech/docs/fleet/coding".into(),
        pricing_model: "pay_per_use".into(),
        price_per_request_micro: 2_000_000, // $2.00 base
        currency: "USD".into(),
        tier: "premium".into(),
        input_modes: "text/plain,application/json".into(),
        output_modes: "text/plain,application/json,text/x-diff".into(),
        supports_streaming: true,
        skills: vec![
            AgentSkillData {
                skill_id: "code-review".into(),
                name: "Code Review".into(),
                description: "Analyze PRs for correctness, style, security, and performance"
                    .into(),
                tags: "code,review,pr,quality,security".into(),
                examples: "Review this PR for security issues|Check code style compliance|Find performance bottlenecks".into(),
                input_modes: "text/plain,text/x-diff".into(),
                output_modes: "text/plain,application/json".into(),
            },
            AgentSkillData {
                skill_id: "bug-fix".into(),
                name: "Bug Fix".into(),
                description: "Diagnose root causes and implement targeted fixes".into(),
                tags: "bug,fix,debug,diagnose,patch".into(),
                examples: "Fix this null pointer exception|Debug why tests fail on CI|Patch this security vulnerability".into(),
                input_modes: "text/plain,application/json".into(),
                output_modes: "text/plain,text/x-diff".into(),
            },
            AgentSkillData {
                skill_id: "refactor".into(),
                name: "Refactoring".into(),
                description: "Restructure code for clarity, maintainability, and performance"
                    .into(),
                tags: "refactor,clean,restructure,optimize".into(),
                examples: "Extract this into a reusable function|Reduce complexity of this module|Optimize this hot path".into(),
                input_modes: "text/plain".into(),
                output_modes: "text/plain,text/x-diff".into(),
            },
            AgentSkillData {
                skill_id: "test-writing".into(),
                name: "Test Writing".into(),
                description: "Generate comprehensive unit, integration, and property tests".into(),
                tags: "test,unit,integration,coverage,tdd".into(),
                examples: "Write tests for this function|Increase coverage for this module|Add property-based tests".into(),
                input_modes: "text/plain".into(),
                output_modes: "text/plain".into(),
            },
        ],
    }
}

fn data_listing() -> MarketplaceListing {
    MarketplaceListing {
        agent_id: "life-data-agent-v1".into(),
        name: "Life Data Processing Agent".into(),
        description: "ETL pipelines, data cleaning, transformation, and report generation. \
            Handles CSV, JSON, SQL, Parquet, and streaming data. Outcome-priced per pipeline run."
            .into(),
        url: "https://fleet.broomva.tech/agents/data".into(),
        version: "1.0.0".into(),
        provider_name: "Broomva Tech".into(),
        provider_url: "https://broomva.tech".into(),
        documentation_url: "https://docs.broomva.tech/docs/fleet/data".into(),
        pricing_model: "pay_per_use".into(),
        price_per_request_micro: 5_000_000, // $5.00 base
        currency: "USD".into(),
        tier: "premium".into(),
        input_modes: "text/csv,application/json,application/x-ndjson,text/plain".into(),
        output_modes: "text/csv,application/json,application/x-ndjson,text/markdown".into(),
        supports_streaming: true,
        skills: vec![
            AgentSkillData {
                skill_id: "etl-pipeline".into(),
                name: "ETL Pipeline".into(),
                description: "Extract, transform, and load data between systems".into(),
                tags: "etl,pipeline,extract,transform,load,data".into(),
                examples: "Run this ETL pipeline|Extract data from API and load to CSV|Transform JSON to normalized tables".into(),
                input_modes: "application/json,text/csv".into(),
                output_modes: "application/json,text/csv".into(),
            },
            AgentSkillData {
                skill_id: "data-cleaning".into(),
                name: "Data Cleaning".into(),
                description: "Detect and fix anomalies, missing values, and format issues".into(),
                tags: "clean,validate,anomaly,missing,format,quality".into(),
                examples: "Clean this CSV of duplicates|Fix date format inconsistencies|Fill missing values".into(),
                input_modes: "text/csv,application/json".into(),
                output_modes: "text/csv,application/json".into(),
            },
            AgentSkillData {
                skill_id: "report-generation".into(),
                name: "Report Generation".into(),
                description: "Produce structured reports with summaries and visualizations".into(),
                tags: "report,summary,aggregate,statistics,markdown".into(),
                examples: "Generate a monthly sales report|Summarize this dataset|Create a data quality report".into(),
                input_modes: "text/csv,application/json".into(),
                output_modes: "text/markdown,application/json".into(),
            },
        ],
    }
}

fn support_listing() -> MarketplaceListing {
    MarketplaceListing {
        agent_id: "life-support-agent-v1".into(),
        name: "Life Support Agent".into(),
        description: "Customer support for ticket triage, FAQ response, and escalation routing. \
            Knowledge-base-powered with sentiment analysis. Outcome-priced per ticket resolved."
            .into(),
        url: "https://fleet.broomva.tech/agents/support".into(),
        version: "1.0.0".into(),
        provider_name: "Broomva Tech".into(),
        provider_url: "https://broomva.tech".into(),
        documentation_url: "https://docs.broomva.tech/docs/fleet/support".into(),
        pricing_model: "pay_per_use".into(),
        price_per_request_micro: 500_000, // $0.50 base
        currency: "USD".into(),
        tier: "premium".into(),
        input_modes: "text/plain,application/json".into(),
        output_modes: "text/plain,application/json".into(),
        supports_streaming: true,
        skills: vec![
            AgentSkillData {
                skill_id: "ticket-triage".into(),
                name: "Ticket Triage".into(),
                description: "Classify tickets by urgency, category, and routing destination"
                    .into(),
                tags: "triage,classify,priority,routing,ticket".into(),
                examples: "Triage this support ticket|Classify urgency level|Route to appropriate team".into(),
                input_modes: "text/plain,application/json".into(),
                output_modes: "application/json".into(),
            },
            AgentSkillData {
                skill_id: "faq-response".into(),
                name: "FAQ Response".into(),
                description: "Answer common questions from the knowledge base".into(),
                tags: "faq,answer,knowledge,help,question".into(),
                examples: "Answer this customer question|Find relevant documentation|Explain this feature".into(),
                input_modes: "text/plain".into(),
                output_modes: "text/plain".into(),
            },
            AgentSkillData {
                skill_id: "escalation".into(),
                name: "Escalation Routing".into(),
                description: "Identify and route tickets requiring specialist intervention".into(),
                tags: "escalate,route,specialist,urgent,critical".into(),
                examples: "This needs engineering attention|Route to billing team|Escalate security concern".into(),
                input_modes: "text/plain,application/json".into(),
                output_modes: "application/json".into(),
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_listings_valid() {
        for vertical in [
            AgentVertical::Coding,
            AgentVertical::DataProcessing,
            AgentVertical::Support,
        ] {
            let listing = listing_for(vertical);
            assert!(!listing.agent_id.is_empty());
            assert!(!listing.name.is_empty());
            assert!(!listing.description.is_empty());
            assert!(!listing.url.is_empty());
            assert!(!listing.skills.is_empty());
            assert!(listing.price_per_request_micro > 0);
        }
    }

    #[test]
    fn coding_has_four_skills() {
        let listing = listing_for(AgentVertical::Coding);
        assert_eq!(listing.skills.len(), 4);
    }

    #[test]
    fn data_has_three_skills() {
        let listing = listing_for(AgentVertical::DataProcessing);
        assert_eq!(listing.skills.len(), 3);
    }

    #[test]
    fn support_has_three_skills() {
        let listing = listing_for(AgentVertical::Support);
        assert_eq!(listing.skills.len(), 3);
    }

    #[test]
    fn listings_serialize_to_json() {
        for vertical in [
            AgentVertical::Coding,
            AgentVertical::DataProcessing,
            AgentVertical::Support,
        ] {
            let listing = listing_for(vertical);
            let json = serde_json::to_string_pretty(&listing).unwrap();
            assert!(json.contains(&listing.agent_id));
        }
    }

    #[test]
    fn skill_ids_unique_within_listing() {
        for vertical in [
            AgentVertical::Coding,
            AgentVertical::DataProcessing,
            AgentVertical::Support,
        ] {
            let listing = listing_for(vertical);
            let mut ids: Vec<&str> = listing.skills.iter().map(|s| s.skill_id.as_str()).collect();
            let original_len = ids.len();
            ids.sort();
            ids.dedup();
            assert_eq!(
                ids.len(),
                original_len,
                "duplicate skill IDs in {:?}",
                vertical
            );
        }
    }
}
