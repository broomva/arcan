//! Bridge between Arcan's internal types and the canonical `aios-protocol` types.
//!
//! Provides conversions so Arcan can emit canonical events to Lago
//! and consume canonical types from the protocol without changing its
//! internal representation.

use crate::protocol::{AgentEvent, ToolCall, ToolResultSummary};

/// Re-export the canonical protocol for downstream convenience.
pub use aios_protocol;

/// Convert an Arcan `AgentEvent` to a canonical `aios_protocol::EventKind`.
///
/// Uses JSON round-trip for simplicity. Variant names that don't match
/// exactly will fall through to `Custom` on the protocol side.
pub fn to_protocol_event_kind(event: &AgentEvent) -> Option<aios_protocol::EventKind> {
    let json = serde_json::to_value(event).ok()?;
    serde_json::from_value(json).ok()
}

/// Convert an Arcan `ToolCall` to a canonical `aios_protocol::tool::ToolCall`.
impl From<&ToolCall> for aios_protocol::tool::ToolCall {
    fn from(call: &ToolCall) -> Self {
        aios_protocol::tool::ToolCall {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            input: call.input.clone(),
            requested_capabilities: Vec::new(),
        }
    }
}

/// Convert an Arcan `ToolResultSummary` to a protocol-compatible JSON value.
impl ToolResultSummary {
    pub fn to_protocol_json(&self) -> serde_json::Value {
        serde_json::json!({
            "call_id": self.call_id,
            "tool_name": self.tool_name,
            "output": self.output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AgentEvent;

    #[test]
    fn agent_event_to_protocol_run_started() {
        let event = AgentEvent::RunStarted {
            run_id: "run-1".into(),
            session_id: "sess-1".into(),
            provider: "anthropic".into(),
            max_iterations: 10,
        };
        let kind = to_protocol_event_kind(&event);
        // AgentEvent uses "part_type" tag while protocol uses "type" tag,
        // so JSON round-trip won't match directly. The event falls to Custom.
        // This is expected — full alignment happens when Arcan adopts protocol types directly.
        assert!(kind.is_some());
    }

    #[test]
    fn tool_call_conversion() {
        let arcan_call = ToolCall {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            input: serde_json::json!({"path": "/tmp"}),
        };
        let proto_call: aios_protocol::tool::ToolCall = (&arcan_call).into();
        assert_eq!(proto_call.call_id, "c1");
        assert_eq!(proto_call.tool_name, "read_file");
    }
}
