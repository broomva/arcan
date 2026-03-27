//! Core types for agent verticals.

use crate::listing::MarketplaceListing;
use haima_core::outcome::TaskContract;
use serde::{Deserialize, Serialize};

/// The three production agent verticals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentVertical {
    /// Code review, bug fixes, refactoring, test writing.
    Coding,
    /// ETL pipelines, data cleaning, report generation.
    DataProcessing,
    /// Ticket triage, FAQ response, escalation routing.
    Support,
}

impl std::fmt::Display for AgentVertical {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Coding => write!(f, "coding"),
            Self::DataProcessing => write!(f, "data_processing"),
            Self::Support => write!(f, "support"),
        }
    }
}

/// Which Praxis tools an agent vertical is permitted to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub read_file: bool,
    pub write_file: bool,
    pub edit_file: bool,
    pub list_dir: bool,
    pub glob: bool,
    pub grep: bool,
    pub bash: bool,
    pub read_memory: bool,
    pub write_memory: bool,
}

impl ToolPermissions {
    /// All tools enabled (coding agent).
    pub fn full() -> Self {
        Self {
            read_file: true,
            write_file: true,
            edit_file: true,
            list_dir: true,
            glob: true,
            grep: true,
            bash: true,
            read_memory: true,
            write_memory: true,
        }
    }

    /// Read-heavy + bash for pipeline execution (data agent).
    pub fn data_processing() -> Self {
        Self {
            read_file: true,
            write_file: true,
            edit_file: false,
            list_dir: true,
            glob: true,
            grep: true,
            bash: true,
            read_memory: true,
            write_memory: true,
        }
    }

    /// Read-only filesystem + memory (support agent).
    pub fn support() -> Self {
        Self {
            read_file: true,
            write_file: false,
            edit_file: false,
            list_dir: true,
            glob: true,
            grep: true,
            bash: false,
            read_memory: true,
            write_memory: true,
        }
    }

    /// Return the list of enabled tool names.
    pub fn enabled_tools(&self) -> Vec<&'static str> {
        let mut tools = Vec::new();
        if self.read_file {
            tools.push("read_file");
        }
        if self.write_file {
            tools.push("write_file");
        }
        if self.edit_file {
            tools.push("edit_file");
        }
        if self.list_dir {
            tools.push("list_dir");
        }
        if self.glob {
            tools.push("glob");
        }
        if self.grep {
            tools.push("grep");
        }
        if self.bash {
            tools.push("bash");
        }
        if self.read_memory {
            tools.push("read_memory");
        }
        if self.write_memory {
            tools.push("write_memory");
        }
        tools
    }
}

/// Complete configuration for an agent vertical.
///
/// Contains everything needed to instantiate an Arcan orchestrator, register
/// on the Spaces marketplace, and set up Haima billing.
#[derive(Debug, Clone)]
pub struct VerticalConfig {
    pub vertical: AgentVertical,
    agent_id: String,
    name: String,
    description: String,
    persona: String,
    rules: String,
    pub tools: ToolPermissions,
    pub max_iterations: u32,
}

impl VerticalConfig {
    /// Build the config for a specific vertical.
    pub fn for_vertical(vertical: AgentVertical) -> Self {
        match vertical {
            AgentVertical::Coding => crate::coding::config(),
            AgentVertical::DataProcessing => crate::data::config(),
            AgentVertical::Support => crate::support::config(),
        }
    }

    /// Stable agent identifier for marketplace registration.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Short description for marketplace listing.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// System prompt (Persona context block).
    pub fn persona(&self) -> &str {
        &self.persona
    }

    /// Behavioral rules (Rules context block).
    pub fn rules(&self) -> &str {
        &self.rules
    }

    /// Build context blocks for the Arcan context compiler.
    pub fn context_blocks(&self) -> Vec<(arcan_core::context_compiler::ContextBlockKind, String)> {
        use arcan_core::context_compiler::ContextBlockKind;
        vec![
            (ContextBlockKind::Persona, self.persona.clone()),
            (ContextBlockKind::Rules, self.rules.clone()),
        ]
    }

    /// Get the Haima task contract for this vertical.
    pub fn contract(&self) -> TaskContract {
        crate::contracts::contract_for(self.vertical)
    }

    /// Get the Spaces marketplace listing data.
    pub fn listing(&self) -> MarketplaceListing {
        crate::listing::listing_for(self.vertical)
    }

    /// Construct a new VerticalConfig (used by vertical modules).
    pub(crate) fn new(
        vertical: AgentVertical,
        agent_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        persona: impl Into<String>,
        rules: impl Into<String>,
        tools: ToolPermissions,
        max_iterations: u32,
    ) -> Self {
        Self {
            vertical,
            agent_id: agent_id.into(),
            name: name.into(),
            description: description.into(),
            persona: persona.into(),
            rules: rules.into(),
            tools,
            max_iterations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_display() {
        assert_eq!(AgentVertical::Coding.to_string(), "coding");
        assert_eq!(AgentVertical::DataProcessing.to_string(), "data_processing");
        assert_eq!(AgentVertical::Support.to_string(), "support");
    }

    #[test]
    fn vertical_serde_roundtrip() {
        let v = AgentVertical::Coding;
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "\"coding\"");
        let back: AgentVertical = serde_json::from_str(&json).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn full_permissions_has_all_tools() {
        let perms = ToolPermissions::full();
        assert_eq!(perms.enabled_tools().len(), 9);
    }

    #[test]
    fn support_permissions_no_write_no_bash() {
        let perms = ToolPermissions::support();
        let tools = perms.enabled_tools();
        assert!(!tools.contains(&"write_file"));
        assert!(!tools.contains(&"edit_file"));
        assert!(!tools.contains(&"bash"));
        assert!(tools.contains(&"read_file"));
        assert!(tools.contains(&"grep"));
    }

    #[test]
    fn data_permissions_no_edit() {
        let perms = ToolPermissions::data_processing();
        let tools = perms.enabled_tools();
        assert!(!tools.contains(&"edit_file"));
        assert!(tools.contains(&"bash"));
        assert!(tools.contains(&"write_file"));
    }

    #[test]
    fn context_blocks_has_persona_and_rules() {
        let config = VerticalConfig::for_vertical(AgentVertical::Coding);
        let blocks = config.context_blocks();
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].0,
            arcan_core::context_compiler::ContextBlockKind::Persona
        );
        assert_eq!(
            blocks[1].0,
            arcan_core::context_compiler::ContextBlockKind::Rules
        );
    }
}
