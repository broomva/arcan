//! Policy violation audit events for arcand (BRO-224).
//!
//! Every capability violation, policy gate hit, and rate limit event is
//! emitted as a structured Lago event for audit, security monitoring, and
//! billing reconciliation.
//!
//! **Event namespace**: `policy.*` — follows the same `"domain.event"` pattern
//! used by `skill.*` (skill_events.rs) and `autonomic.*`.
//!
//! **Usage in arcand**: call `event_kind(data)` and pass the result to
//! `KernelRuntime::record_external_event` to write into the session journal
//! without needing a direct `lago_core::Journal` reference.
//!
//! **Usage in projections**: call `build_event(session_id, branch_id, data)`
//! and append the returned `EventEnvelope` to a journal directly.

use aios_protocol::EventKind;
use lago_core::{BranchId, EventEnvelope, EventId, EventPayload, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

// ─── Event type constant ───────────────────────────────────────────────────

/// Event type string written into every `policy.violation` envelope.
pub const EVENT_TYPE: &str = "policy.violation";

// ─── ViolationType ─────────────────────────────────────────────────────────

/// Discriminates why a policy enforcement point was triggered.
///
/// Serialised as `snake_case` strings in the event payload so downstream
/// consumers (dashboards, alerting, Nous quality scorer) can pattern-match
/// without coupling to Rust enum ordinals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationType {
    /// A capability required by the operation is not in the session's `PolicySet`.
    CapabilityBlocked,
    /// Path traversal attempt outside the session sandbox boundary.
    PathTraversal,
    /// A shell command is not in the tier's allowed command set.
    CommandNotAllowed,
    /// The session has exceeded its Lago event budget for this billing period.
    EventBudgetExceeded,
    /// Per-user, per-tier request throttling limit reached (token bucket empty).
    RateLimitExceeded,
    /// The model requested is not allowed for this tier.
    ModelNotAllowed,
    /// A skill invocation was blocked because its declared tools exceed tier capabilities.
    SkillNotAllowed,
    /// The identity token has expired (`exp` claim in the past).
    TokenExpired,
    /// The identity token failed validation (wrong secret, malformed, missing claims).
    AuthenticationError,
}

impl ViolationType {
    /// Returns the canonical string representation, matching the `serde` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CapabilityBlocked => "capability_blocked",
            Self::PathTraversal => "path_traversal",
            Self::CommandNotAllowed => "command_not_allowed",
            Self::EventBudgetExceeded => "event_budget_exceeded",
            Self::RateLimitExceeded => "rate_limit_exceeded",
            Self::ModelNotAllowed => "model_not_allowed",
            Self::SkillNotAllowed => "skill_not_allowed",
            Self::TokenExpired => "token_expired",
            Self::AuthenticationError => "authentication_error",
        }
    }
}

// ─── PolicyViolationData ───────────────────────────────────────────────────

/// Structured payload for a `policy.violation` Lago event.
///
/// Serialised as the `data` field of `EventPayload::Custom { event_type:
/// "policy.violation", data: <this> }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolationData {
    /// What kind of enforcement was triggered.
    pub violation_type: ViolationType,
    /// The capability string that was blocked, e.g. `"exec:cmd:rm"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// The value that triggered the violation (command, path, skill name).
    /// Sensitive absolute paths are redacted to their last component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_value: Option<String>,
    /// The tier at the time of violation, e.g. `"anonymous"`, `"free"`.
    pub tier: String,
    /// The user_id or session_id that was blocked/throttled.
    pub subject: String,
}

// ─── Event builders ────────────────────────────────────────────────────────

/// Build an `EventKind::Custom` for a policy violation.
///
/// Pass the result to `KernelRuntime::record_external_event` from within
/// `arcand` request handlers — no direct journal reference required.
pub fn event_kind(data: &PolicyViolationData) -> EventKind {
    EventKind::Custom {
        event_type: EVENT_TYPE.to_string(),
        data: json!(data),
    }
}

/// Build a Lago `EventEnvelope` for a policy violation.
///
/// Use this variant when you already hold a `&dyn lago_core::Journal`
/// reference (e.g. inside a `Projection` or a background task).
pub fn build_event(
    session_id: &SessionId,
    branch_id: &BranchId,
    data: &PolicyViolationData,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: None,
        seq: 0, // auto-assigned by journal on append
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload: EventPayload::Custom {
            event_type: EVENT_TYPE.to_string(),
            data: json!(data),
        },
        metadata: HashMap::new(),
        schema_version: 1,
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn violation_type_serialises_to_snake_case() {
        let v = ViolationType::RateLimitExceeded;
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#""rate_limit_exceeded""#);
    }

    #[test]
    fn violation_type_as_str_matches_serde() {
        let cases = [
            (ViolationType::CapabilityBlocked, "capability_blocked"),
            (ViolationType::PathTraversal, "path_traversal"),
            (ViolationType::CommandNotAllowed, "command_not_allowed"),
            (ViolationType::EventBudgetExceeded, "event_budget_exceeded"),
            (ViolationType::RateLimitExceeded, "rate_limit_exceeded"),
            (ViolationType::ModelNotAllowed, "model_not_allowed"),
            (ViolationType::SkillNotAllowed, "skill_not_allowed"),
            (ViolationType::TokenExpired, "token_expired"),
            (ViolationType::AuthenticationError, "authentication_error"),
        ];
        for (variant, expected) in cases {
            assert_eq!(variant.as_str(), expected);
            let serialised = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialised, format!(r#""{expected}""#));
        }
    }

    #[test]
    fn event_kind_encodes_violation_type_in_data() {
        let data = PolicyViolationData {
            violation_type: ViolationType::RateLimitExceeded,
            capability: None,
            attempted_value: None,
            tier: "anonymous".to_string(),
            subject: "session-abc".to_string(),
        };
        let kind = event_kind(&data);
        let EventKind::Custom {
            event_type,
            data: payload,
        } = kind
        else {
            panic!("expected Custom variant");
        };
        assert_eq!(event_type, EVENT_TYPE);
        assert_eq!(payload["violation_type"], "rate_limit_exceeded");
        assert_eq!(payload["tier"], "anonymous");
        assert_eq!(payload["subject"], "session-abc");
    }

    #[test]
    fn optional_fields_are_omitted_when_none() {
        let data = PolicyViolationData {
            violation_type: ViolationType::AuthenticationError,
            capability: None,
            attempted_value: None,
            tier: "free".to_string(),
            subject: "user-xyz".to_string(),
        };
        let json = serde_json::to_value(&data).unwrap();
        assert!(!json.as_object().unwrap().contains_key("capability"));
        assert!(!json.as_object().unwrap().contains_key("attempted_value"));
    }

    #[test]
    fn optional_fields_are_present_when_set() {
        let data = PolicyViolationData {
            violation_type: ViolationType::SkillNotAllowed,
            capability: Some("exec:cmd:*".to_string()),
            attempted_value: Some("deep-research".to_string()),
            tier: "free".to_string(),
            subject: "user-abc".to_string(),
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["capability"], "exec:cmd:*");
        assert_eq!(json["attempted_value"], "deep-research");
    }

    #[test]
    fn build_event_sets_correct_payload_type() {
        let session_id = SessionId::new();
        let branch_id = BranchId::from_string("main");
        let data = PolicyViolationData {
            violation_type: ViolationType::TokenExpired,
            capability: None,
            attempted_value: None,
            tier: "pro".to_string(),
            subject: "user-def".to_string(),
        };
        let envelope = build_event(&session_id, &branch_id, &data);
        let EventPayload::Custom {
            event_type,
            data: payload,
        } = &envelope.payload
        else {
            panic!("expected Custom payload");
        };
        assert_eq!(event_type, EVENT_TYPE);
        assert_eq!(payload["violation_type"], "token_expired");
        assert_eq!(envelope.session_id, session_id);
        assert_eq!(envelope.seq, 0); // journal assigns on append
    }
}
