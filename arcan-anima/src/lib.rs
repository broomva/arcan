//! # Arcan-Anima — Bridge between Agent Runtime and Identity Layer
//!
//! This crate connects Arcan's agent loop to Anima's soul/identity/belief
//! model. When the agent starts, it reconstructs [`AgentSelf`] from Lago
//! events and makes identity data available in [`AppState`].
//!
//! ## Functions
//!
//! - [`reconstruct_agent_self`] — replay Anima events from Lago to build `AgentSelf`
//! - [`emit_soul_genesis`] — write a soul genesis event to the Lago journal
//! - [`inject_anima_context`] — merge agent identity info into `AppState`

use std::collections::HashMap;
use std::sync::Arc;

use anima_core::agent_self::AgentSelf;
use anima_core::belief::AgentBelief;
use anima_core::event::AnimaEventKind;
use anima_core::identity::AgentIdentity;
use anima_core::soul::AgentSoul;
use anima_lago::genesis::{create_genesis_event, reconstruct_soul};
use anima_lago::projection;
use arcan_core::protocol::{StatePatch, StatePatchFormat, StatePatchSource};
use arcan_core::state::AppState;
use lago_core::event::EventPayload;
use lago_core::{BranchId, EventEnvelope, EventId, EventQuery, Journal, SeqNo, SessionId};
use thiserror::Error;

/// Errors that can occur during Arcan-Anima bridge operations.
#[derive(Debug, Error)]
pub enum AnimaBridgeError {
    /// No soul genesis event found in the journal for this session.
    #[error("no soul genesis event found for session {session_id}")]
    NoSoulGenesis { session_id: String },

    /// The soul genesis event failed integrity verification.
    #[error("soul integrity error: {0}")]
    SoulIntegrity(String),

    /// Failed to construct AgentSelf from components.
    #[error("agent self construction error: {0}")]
    AgentSelfConstruction(String),

    /// Lago journal I/O error.
    #[error("journal error: {0}")]
    Journal(String),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result alias for bridge operations.
pub type AnimaBridgeResult<T> = Result<T, AnimaBridgeError>;

/// Reconstruct an [`AgentSelf`] by replaying Anima events from the Lago journal.
///
/// This reads all `EventPayload::Custom` events with the `"anima."` namespace
/// prefix from the given session, extracts the soul from the genesis event,
/// folds belief-affecting events into an `AgentBelief`, and assembles an
/// `AgentSelf` from the reconstructed parts.
///
/// The `identity` parameter provides the cryptographic identity (which is not
/// stored in the journal — it comes from the keystore or session config).
///
/// # Errors
///
/// Returns [`AnimaBridgeError::NoSoulGenesis`] if no soul genesis event is
/// found, or [`AnimaBridgeError::SoulIntegrity`] if the soul hash fails
/// verification.
pub fn reconstruct_agent_self(
    journal: &Arc<dyn Journal>,
    session_id: &SessionId,
    branch_id: &BranchId,
    identity: AgentIdentity,
) -> AnimaBridgeResult<AgentSelf> {
    let envelopes = block_on(
        journal.read(
            EventQuery::new()
                .session(session_id.clone())
                .branch(branch_id.clone()),
        ),
    )
    .map_err(|e| AnimaBridgeError::Journal(e.to_string()))?;

    // Extract Anima events from Custom payloads
    let anima_events = extract_anima_events(&envelopes);

    if anima_events.is_empty() {
        return Err(AnimaBridgeError::NoSoulGenesis {
            session_id: session_id.to_string(),
        });
    }

    // Find the soul genesis event (always the first anima event)
    let soul = find_and_reconstruct_soul(&anima_events)?;

    // Fold all events into belief state
    let belief = replay_beliefs(&anima_events);

    // Assemble AgentSelf (using unchecked constructor since data comes from
    // our own trusted journal)
    Ok(AgentSelf::from_parts_unchecked(soul, identity, belief))
}

/// Write a soul genesis event to the Lago journal.
///
/// This creates the first Anima event for an agent session. The soul
/// is serialized into an `AnimaEventKind::SoulGenesis` and wrapped
/// in a Lago `EventEnvelope` with `EventPayload::Custom`.
///
/// # Errors
///
/// Returns an error if the soul cannot be serialized or the journal
/// append fails.
pub fn emit_soul_genesis(
    journal: &Arc<dyn Journal>,
    session_id: &SessionId,
    branch_id: &BranchId,
    soul: &AgentSoul,
) -> AnimaBridgeResult<SeqNo> {
    let genesis =
        create_genesis_event(soul).map_err(|e| AnimaBridgeError::SoulIntegrity(e.to_string()))?;

    let envelope = anima_event_to_envelope(
        &genesis, session_id, branch_id, 0, // Genesis is always seq 0
    );

    let seq =
        block_on(journal.append(envelope)).map_err(|e| AnimaBridgeError::Journal(e.to_string()))?;

    tracing::info!(
        session_id = %session_id,
        soul_hash = %soul.soul_hash(),
        seq = seq,
        "soul genesis event emitted to Lago"
    );

    Ok(seq)
}

/// Inject Anima identity context into [`AppState`] as a merge patch.
///
/// This adds a well-known `"anima"` key to the state containing the
/// agent's identity summary: name, mission, agent ID, active status,
/// soul hash, and DID (if available).
///
/// Arcan's agent loop can then read this data from state to include
/// identity context in system prompts or tool decisions.
pub fn inject_anima_context(
    state: &mut AppState,
    agent_self: &AgentSelf,
) -> Result<(), arcan_core::state::StateError> {
    let anima_data = serde_json::json!({
        "anima": {
            "agent_id": agent_self.agent_id(),
            "name": agent_self.name(),
            "mission": agent_self.mission(),
            "soul_hash": agent_self.soul_hash(),
            "is_active": agent_self.is_active(),
            "did": agent_self.did(),
            "capabilities_count": agent_self.beliefs().capabilities.len(),
            "trust_peers_count": agent_self.beliefs().trust_scores.len(),
        }
    });

    let patch = StatePatch {
        format: StatePatchFormat::MergePatch,
        patch: anima_data,
        source: StatePatchSource::System,
    };

    state.apply_patch(&patch)
}

// ─── Internal helpers ────────────────────────────────────────────────────────

/// Run an async future on the current tokio runtime from a sync context.
///
/// This follows the same pattern as `LagoSessionRepository::block_on` in
/// arcan-lago — safe because Arcan's orchestrator runs inside
/// `tokio::task::spawn_blocking`.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
}

/// Extract Anima events from Lago envelopes.
///
/// Filters for `EventPayload::Custom` with `"anima."` prefix and
/// deserializes them into `AnimaEventKind` values.
fn extract_anima_events(
    envelopes: &[EventEnvelope],
) -> Vec<(AnimaEventKind, SeqNo, chrono::DateTime<chrono::Utc>)> {
    let mut events = Vec::new();

    for envelope in envelopes {
        if let EventPayload::Custom {
            event_type, data, ..
        } = &envelope.payload
            && event_type.starts_with(AnimaEventKind::NAMESPACE)
            && let Some(anima_event) = AnimaEventKind::from_custom(event_type, data)
        {
            let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp(
                (envelope.timestamp / 1_000_000) as i64,
                ((envelope.timestamp % 1_000_000) * 1_000) as u32,
            )
            .unwrap_or_else(chrono::Utc::now);

            events.push((anima_event, envelope.seq, timestamp));
        }
    }

    events
}

/// Find the SoulGenesis event and reconstruct the soul from it.
fn find_and_reconstruct_soul(
    events: &[(AnimaEventKind, SeqNo, chrono::DateTime<chrono::Utc>)],
) -> AnimaBridgeResult<AgentSoul> {
    for (event, _, _) in events {
        if matches!(event, AnimaEventKind::SoulGenesis { .. }) {
            return reconstruct_soul(event)
                .map_err(|e| AnimaBridgeError::SoulIntegrity(e.to_string()));
        }
    }

    Err(AnimaBridgeError::NoSoulGenesis {
        session_id: "unknown".into(),
    })
}

/// Replay all Anima events to reconstruct belief state.
fn replay_beliefs(
    events: &[(AnimaEventKind, SeqNo, chrono::DateTime<chrono::Utc>)],
) -> AgentBelief {
    projection::replay(events)
}

/// Wrap an Anima event in a Lago `EventEnvelope` using `EventPayload::Custom`.
fn anima_event_to_envelope(
    event: &AnimaEventKind,
    session_id: &SessionId,
    branch_id: &BranchId,
    seq: SeqNo,
) -> EventEnvelope {
    let mut metadata = HashMap::new();
    metadata.insert("source".to_string(), "arcan-anima".to_string());

    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: None,
        seq,
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload: EventPayload::Custom {
            event_type: event.event_type(),
            data: event.to_custom_data(),
        },
        metadata,
        schema_version: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anima_core::belief::AgentBelief;
    use anima_core::identity::LifecycleState;
    use anima_core::soul::SoulBuilder;
    use lago_journal::RedbJournal;

    fn test_soul() -> AgentSoul {
        SoulBuilder::new(
            "arcan-test-agent",
            "bridge integration testing",
            vec![1u8; 32],
        )
        .build()
    }

    fn test_identity() -> AgentIdentity {
        AgentIdentity {
            agent_id: "agt_arcan_bridge_001".into(),
            host_id: "host_arcan_test".into(),
            auth_public_key: vec![1u8; 32], // Must match soul's root key
            wallet_address: haima_core::wallet::WalletAddress {
                address: "0xtest".into(),
                chain: haima_core::wallet::ChainId::base(),
            },
            did: Some("did:key:z6MkTestBridge".into()),
            lifecycle: LifecycleState::Active,
            created_at: chrono::Utc::now(),
            expires_at: None,
            seed_blob_ref: None,
        }
    }

    fn make_journal() -> Arc<dyn Journal> {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-anima-bridge.redb");
        // Leak the tempdir so the file survives the test
        std::mem::forget(dir);
        Arc::new(RedbJournal::open(db_path).unwrap())
    }

    #[tokio::test]
    async fn soul_genesis_roundtrips_through_journal() {
        let journal = make_journal();
        let session_id = SessionId::new();
        let branch_id = BranchId::from("main");
        let soul = test_soul();

        // Emit genesis
        let seq = tokio::task::spawn_blocking({
            let journal = journal.clone();
            let session_id = session_id.clone();
            let branch_id = branch_id.clone();
            let soul = soul.clone();
            move || emit_soul_genesis(&journal, &session_id, &branch_id, &soul)
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(seq, 1); // First event in journal

        // Reconstruct
        let agent_self = tokio::task::spawn_blocking({
            let journal = journal.clone();
            let session_id = session_id.clone();
            let branch_id = branch_id.clone();
            let identity = test_identity();
            move || reconstruct_agent_self(&journal, &session_id, &branch_id, identity)
        })
        .await
        .unwrap()
        .unwrap();

        assert_eq!(agent_self.name(), "arcan-test-agent");
        assert_eq!(agent_self.mission(), "bridge integration testing");
        assert_eq!(agent_self.soul_hash(), soul.soul_hash());
        assert!(agent_self.is_active());
    }

    #[tokio::test]
    async fn reconstruct_with_belief_events() {
        let journal = make_journal();
        let session_id = SessionId::new();
        let branch_id = BranchId::from("main");
        let soul = test_soul();

        // Emit genesis + capability grant + trust update
        tokio::task::spawn_blocking({
            let journal = journal.clone();
            let session_id = session_id.clone();
            let branch_id = branch_id.clone();
            let soul = soul.clone();
            move || {
                emit_soul_genesis(&journal, &session_id, &branch_id, &soul).unwrap();

                // Emit a capability grant
                let grant_event = AnimaEventKind::CapabilityGranted {
                    capability: "chat:send".into(),
                    granted_by: "test-server".into(),
                    expires_at: None,
                    constraints: serde_json::json!({}),
                };
                let envelope = anima_event_to_envelope(&grant_event, &session_id, &branch_id, 1);
                block_on(journal.append(envelope)).unwrap();

                // Emit a trust update
                let trust_event = AnimaEventKind::TrustUpdated {
                    peer_id: "peer-alpha".into(),
                    new_score: 0.85,
                    interaction_success: true,
                };
                let envelope = anima_event_to_envelope(&trust_event, &session_id, &branch_id, 2);
                block_on(journal.append(envelope)).unwrap();
            }
        })
        .await
        .unwrap();

        // Reconstruct
        let agent_self = tokio::task::spawn_blocking({
            let journal = journal.clone();
            let session_id = session_id.clone();
            let branch_id = branch_id.clone();
            let identity = test_identity();
            move || reconstruct_agent_self(&journal, &session_id, &branch_id, identity)
        })
        .await
        .unwrap()
        .unwrap();

        // Verify beliefs were projected
        assert!(agent_self.beliefs().has_capability("chat:send"));
        assert_eq!(agent_self.beliefs().trust_scores.len(), 1);
        let trust = &agent_self.beliefs().trust_scores["peer-alpha"];
        assert!((trust.score - 0.85).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn no_genesis_returns_error() {
        let journal = make_journal();
        let session_id = SessionId::new();
        let branch_id = BranchId::from("main");

        let result = tokio::task::spawn_blocking({
            let journal = journal.clone();
            let session_id = session_id.clone();
            let branch_id = branch_id.clone();
            let identity = test_identity();
            move || reconstruct_agent_self(&journal, &session_id, &branch_id, identity)
        })
        .await
        .unwrap();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            AnimaBridgeError::NoSoulGenesis { .. }
        ));
    }

    #[test]
    fn inject_anima_context_adds_identity_to_state() {
        let soul = test_soul();
        let identity = test_identity();
        let agent_self = AgentSelf::from_parts_unchecked(soul, identity, AgentBelief::default());

        let mut state = AppState::default();
        inject_anima_context(&mut state, &agent_self).unwrap();

        // Verify the anima key was injected
        let anima = &state.data["anima"];
        assert_eq!(anima["agent_id"], "agt_arcan_bridge_001");
        assert_eq!(anima["name"], "arcan-test-agent");
        assert_eq!(anima["mission"], "bridge integration testing");
        assert_eq!(anima["is_active"], true);
        assert_eq!(anima["did"], "did:key:z6MkTestBridge");
        assert_eq!(anima["capabilities_count"], 0);
        assert_eq!(anima["trust_peers_count"], 0);

        // State revision should have incremented
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn inject_anima_context_preserves_existing_state() {
        let soul = test_soul();
        let identity = test_identity();
        let agent_self = AgentSelf::from_parts_unchecked(soul, identity, AgentBelief::default());

        let mut state = AppState::new(serde_json::json!({
            "cwd": "/home/agent",
            "open_files": ["main.rs"]
        }));

        inject_anima_context(&mut state, &agent_self).unwrap();

        // Existing state preserved
        assert_eq!(state.data["cwd"], "/home/agent");
        // Anima context added
        assert_eq!(state.data["anima"]["name"], "arcan-test-agent");
    }

    #[test]
    fn extract_anima_events_filters_correctly() {
        let session_id = SessionId::new();
        let branch_id = BranchId::from("main");

        let soul = test_soul();
        let genesis = create_genesis_event(&soul).unwrap();
        let anima_envelope = anima_event_to_envelope(&genesis, &session_id, &branch_id, 0);

        // Non-anima custom event
        let other_envelope = EventEnvelope {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            branch_id: branch_id.clone(),
            run_id: None,
            seq: 1,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::Custom {
                event_type: "finance.payment_settled".into(),
                data: serde_json::json!({"amount": 100}),
            },
            metadata: HashMap::new(),
            schema_version: 1,
        };

        // Non-custom event
        let run_envelope = EventEnvelope {
            event_id: EventId::new(),
            session_id: session_id.clone(),
            branch_id: branch_id.clone(),
            run_id: None,
            seq: 2,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::RunStarted {
                provider: "test".into(),
                max_iterations: 10,
            },
            metadata: HashMap::new(),
            schema_version: 1,
        };

        let envelopes = vec![anima_envelope, other_envelope, run_envelope];
        let events = extract_anima_events(&envelopes);

        // Only the anima genesis event should be extracted
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].0, AnimaEventKind::SoulGenesis { .. }));
    }

    #[test]
    fn anima_event_to_envelope_sets_correct_fields() {
        let session_id = SessionId::from("test-session");
        let branch_id = BranchId::from("main");
        let event = AnimaEventKind::TrustUpdated {
            peer_id: "peer-1".into(),
            new_score: 0.9,
            interaction_success: true,
        };

        let envelope = anima_event_to_envelope(&event, &session_id, &branch_id, 5);

        assert_eq!(envelope.session_id.as_str(), "test-session");
        assert_eq!(envelope.branch_id.as_str(), "main");
        assert_eq!(envelope.seq, 5);
        assert_eq!(envelope.metadata.get("source").unwrap(), "arcan-anima");

        if let EventPayload::Custom {
            event_type, data, ..
        } = &envelope.payload
        {
            assert_eq!(event_type, "anima.trust_updated");
            assert!(data.get("peer_id").is_some());
        } else {
            panic!("expected Custom payload");
        }
    }
}
