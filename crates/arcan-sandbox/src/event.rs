//! Structured events emitted by the sandbox lifecycle.
//!
//! `SandboxEvent` is the canonical record written to Lago / SpacetimeDB for
//! every observable sandbox state transition. Consumers can subscribe to these
//! events for billing, audit, and observability (BRO-257).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::SandboxId;

/// Discriminated event payload — one variant per observable lifecycle step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SandboxEventKind {
    /// The sandbox was accepted and provisioning began.
    Created,
    /// The sandbox is ready to accept `exec` calls.
    Started,
    /// An `exec` call completed (success or failure).
    ExecCompleted {
        /// Exit code returned by the process.
        exit_code: i32,
        /// Wall-clock milliseconds from process start to exit.
        duration_ms: u64,
    },
    /// The sandbox filesystem was snapshotted.
    Snapshotted {
        /// Identifier of the resulting snapshot.
        snapshot_id: String,
    },
    /// A previously-snapshotted sandbox was resumed.
    Resumed {
        /// Snapshot from which the sandbox was restored.
        from_snapshot: String,
    },
    /// The sandbox was permanently destroyed.
    Destroyed,
    /// The sandbox encountered an unrecoverable error.
    Failed {
        /// Human-readable description of the failure.
        reason: String,
    },
}

/// A single lifecycle event for a sandbox, ready to be persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxEvent {
    /// Sandbox this event belongs to.
    pub sandbox_id: SandboxId,
    /// Agent that owns the sandbox session.
    pub agent_id: String,
    /// Session within which the sandbox was created.
    pub session_id: String,
    /// What happened.
    pub kind: SandboxEventKind,
    /// Provider that emitted the event.
    pub provider: String,
    /// Wall-clock time of the event.
    pub timestamp: DateTime<Utc>,
}

impl SandboxEvent {
    /// Convenience constructor — sets `timestamp` to `Utc::now()`.
    pub fn now(
        sandbox_id: SandboxId,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        kind: SandboxEventKind,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            sandbox_id,
            agent_id: agent_id.into(),
            session_id: session_id.into(),
            kind,
            provider: provider.into(),
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_serde_roundtrip() {
        let kinds = [
            SandboxEventKind::Created,
            SandboxEventKind::Started,
            SandboxEventKind::ExecCompleted { exit_code: 0, duration_ms: 100 },
            SandboxEventKind::Snapshotted { snapshot_id: "snap-1".into() },
            SandboxEventKind::Resumed { from_snapshot: "snap-1".into() },
            SandboxEventKind::Destroyed,
            SandboxEventKind::Failed { reason: "oom".into() },
        ];
        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let back: SandboxEventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, back);
        }
    }

    #[test]
    fn event_kind_tag_encoding() {
        let kind = SandboxEventKind::Created;
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"kind\":\"created\""));
    }

    #[test]
    fn sandbox_event_now_constructor() {
        let event = SandboxEvent::now(
            SandboxId("s1".into()),
            "agent-42",
            "sess-99",
            SandboxEventKind::Started,
            "local",
        );
        assert_eq!(event.agent_id, "agent-42");
        assert_eq!(event.session_id, "sess-99");
        assert_eq!(event.provider, "local");
        assert_eq!(event.kind, SandboxEventKind::Started);
    }

    #[test]
    fn sandbox_event_serde_roundtrip() {
        let event = SandboxEvent::now(
            SandboxId("s2".into()),
            "agent-1",
            "sess-1",
            SandboxEventKind::ExecCompleted { exit_code: 1, duration_ms: 250 },
            "vercel",
        );
        let json = serde_json::to_string(&event).unwrap();
        let back: SandboxEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(event.sandbox_id, back.sandbox_id);
        assert_eq!(event.kind, back.kind);
        assert_eq!(event.provider, back.provider);
    }
}
