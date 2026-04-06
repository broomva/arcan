//! Praxis tools for Opsis world state interaction.

use std::sync::Arc;

use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext, ToolRegistry};
use serde_json::json;
use tokio::sync::RwLock;

use opsis_core::event::WorldDelta;
use opsis_core::state::StateDomain;

use crate::client::OpsisClient;

// ── opsis_observe ───────────────────────────────────────────────────

pub struct OpsisObserveTool {
    client: Arc<OpsisClient>,
}

impl OpsisObserveTool {
    pub fn new(client: Arc<OpsisClient>) -> Self {
        Self { client }
    }
}

impl Tool for OpsisObserveTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "opsis_observe".into(),
            description: "Publish an observation to the Opsis world state engine".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "insight": { "type": "string", "description": "What the agent observed" },
                    "domain": { "type": "string", "description": "StateDomain (Emergency, Health, Finance, Trade, Conflict, Politics, Weather, Space, Ocean, Technology, Personal, Infrastructure)" },
                    "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "lat": { "type": "number", "description": "Optional latitude" },
                    "lon": { "type": "number", "description": "Optional longitude" }
                },
                "required": ["insight", "domain", "confidence"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("opsis".into()),
            tags: vec!["world-state".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let insight = call.input["insight"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let domain_str = call.input["domain"].as_str().unwrap_or("Personal");
        let confidence = call.input["confidence"].as_f64().unwrap_or(0.5) as f32;
        let location = match (call.input.get("lat"), call.input.get("lon")) {
            (Some(lat), Some(lon)) if lat.is_f64() && lon.is_f64() => Some(
                opsis_core::spatial::GeoPoint::new(lat.as_f64().unwrap(), lon.as_f64().unwrap()),
            ),
            _ => None,
        };

        let domain = parse_domain(domain_str);
        let client = self.client.clone();

        // Fire-and-forget: spawn async task from sync context.
        tokio::spawn(async move {
            if let Err(e) = client.observe(insight, confidence, domain, location).await {
                tracing::warn!(error = %e, "opsis_observe failed");
            }
        });

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "status": "observation published" }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── opsis_alert ─────────────────────────────────────────────────────

pub struct OpsisAlertTool {
    client: Arc<OpsisClient>,
}

impl OpsisAlertTool {
    pub fn new(client: Arc<OpsisClient>) -> Self {
        Self { client }
    }
}

impl Tool for OpsisAlertTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "opsis_alert".into(),
            description: "Publish an alert to the Opsis world state engine".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "Alert description" },
                    "domain": { "type": "string", "description": "StateDomain" },
                    "severity": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                },
                "required": ["message", "domain", "severity"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("opsis".into()),
            tags: vec!["world-state".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let message = call.input["message"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let domain_str = call.input["domain"].as_str().unwrap_or("Personal");
        let severity = call.input["severity"].as_f64().unwrap_or(0.5) as f32;

        let domain = parse_domain(domain_str);
        let client = self.client.clone();

        tokio::spawn(async move {
            if let Err(e) = client.alert(message, domain, severity).await {
                tracing::warn!(error = %e, "opsis_alert failed");
            }
        });

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "status": "alert published" }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── opsis_world_state ───────────────────────────────────────────────

pub struct OpsisWorldStateTool {
    snapshot: Arc<RwLock<Option<WorldDelta>>>,
}

impl OpsisWorldStateTool {
    pub fn new(snapshot: Arc<RwLock<Option<WorldDelta>>>) -> Self {
        Self { snapshot }
    }
}

impl Tool for OpsisWorldStateTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "opsis_world_state".into(),
            description: "Query the current Opsis world state snapshot".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "domains": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter to specific domains (all if omitted)"
                    }
                }
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("opsis".into()),
            tags: vec!["world-state".into()],
            timeout_secs: Some(5),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        // Read snapshot synchronously via try_read.
        let output = match self.snapshot.try_read() {
            Ok(guard) => match guard.as_ref() {
                Some(delta) => {
                    let domains_filter: Option<Vec<String>> = call
                        .input
                        .get("domains")
                        .and_then(|v| serde_json::from_value(v.clone()).ok());

                    let state_lines: Vec<_> = delta
                        .state_line_deltas
                        .iter()
                        .filter(|sld| {
                            domains_filter.as_ref().is_none_or(|domains| {
                                domains.iter().any(|d| parse_domain(d) == sld.domain)
                            })
                        })
                        .map(|sld| {
                            json!({
                                "domain": sld.domain,
                                "activity": sld.activity,
                                "trend": sld.trend,
                                "event_count": sld.new_events.len(),
                                "hotspots": sld.hotspots.iter().map(|h| json!({
                                    "lat": h.center.lat,
                                    "lon": h.center.lon,
                                    "intensity": h.intensity,
                                })).collect::<Vec<_>>(),
                            })
                        })
                        .collect();

                    json!({
                        "tick": delta.tick.0,
                        "timestamp": delta.timestamp.to_rfc3339(),
                        "state_lines": state_lines,
                        "gaia_insights_count": delta.gaia_insights.len(),
                        "unrouted_events_count": delta.unrouted_events.len(),
                    })
                }
                None => json!({ "status": "no world state snapshot available yet" }),
            },
            Err(_) => json!({ "status": "snapshot temporarily unavailable" }),
        };

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output,
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

// ── Registration ────────────────────────────────────────────────────

/// Register all Opsis Praxis tools into a tool registry.
pub fn register_opsis_tools(
    registry: &mut ToolRegistry,
    client: Arc<OpsisClient>,
    snapshot: Arc<RwLock<Option<WorldDelta>>>,
) {
    registry.register(OpsisObserveTool::new(client.clone()));
    registry.register(OpsisAlertTool::new(client));
    registry.register(OpsisWorldStateTool::new(snapshot));
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_domain(s: &str) -> StateDomain {
    match s {
        "Emergency" => StateDomain::Emergency,
        "Health" => StateDomain::Health,
        "Finance" => StateDomain::Finance,
        "Trade" => StateDomain::Trade,
        "Conflict" => StateDomain::Conflict,
        "Politics" => StateDomain::Politics,
        "Weather" => StateDomain::Weather,
        "Space" => StateDomain::Space,
        "Ocean" => StateDomain::Ocean,
        "Technology" => StateDomain::Technology,
        "Personal" => StateDomain::Personal,
        "Infrastructure" => StateDomain::Infrastructure,
        other => StateDomain::Custom(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_domain_known_variants() {
        assert_eq!(parse_domain("Emergency"), StateDomain::Emergency);
        assert_eq!(parse_domain("Finance"), StateDomain::Finance);
        assert_eq!(parse_domain("Weather"), StateDomain::Weather);
    }

    #[test]
    fn parse_domain_custom() {
        assert_eq!(
            parse_domain("MyDomain"),
            StateDomain::Custom("MyDomain".into())
        );
    }

    #[test]
    fn tool_definitions_valid() {
        let client = OpsisClient::new("http://localhost:3010", "test".into()).unwrap();
        let client = Arc::new(client);
        let snapshot = Arc::new(RwLock::new(None));

        let observe = OpsisObserveTool::new(client.clone());
        let def = observe.definition();
        assert_eq!(def.name, "opsis_observe");
        assert_eq!(def.category, Some("opsis".into()));

        let alert = OpsisAlertTool::new(client);
        assert_eq!(alert.definition().name, "opsis_alert");

        let world = OpsisWorldStateTool::new(snapshot);
        assert_eq!(world.definition().name, "opsis_world_state");
    }

    #[test]
    fn world_state_tool_no_snapshot() {
        let snapshot = Arc::new(RwLock::new(None));
        let tool = OpsisWorldStateTool::new(snapshot);

        let call = ToolCall {
            call_id: "test".into(),
            tool_name: "opsis_world_state".into(),
            input: json!({}),
        };
        let ctx = ToolContext {
            run_id: "r".into(),
            session_id: "s".into(),
            iteration: 1,
        };

        let result = tool.execute(&call, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(
            result.output["status"]
                .as_str()
                .unwrap()
                .contains("no world state")
        );
    }
}
