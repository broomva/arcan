use lago_core::error::LagoResult;
use lago_core::event::{EventEnvelope, EventPayload, MemoryScope};
use lago_core::projection::Projection;
use std::collections::{BTreeMap, HashSet};

/// A committed memory entry tracked by the projection.
#[derive(Debug, Clone)]
pub struct CommittedMemory {
    pub memory_id: String,
    pub committed_ref: String,
    pub supersedes: Option<String>,
}

/// Result of querying memory for a specific scope.
#[derive(Debug, Clone)]
pub struct MemoryQueryResult {
    pub observations: Vec<String>,
    pub reflection: Option<String>,
    pub committed: Vec<CommittedMemory>,
}

/// Projection that builds queryable memory state from memory events.
///
/// Tracks observations, reflections, committed memories, and tombstones
/// per scope. Tombstoned memory IDs are excluded from queries.
pub struct MemoryProjection {
    observations: BTreeMap<MemoryScope, Vec<String>>,
    reflections: BTreeMap<MemoryScope, String>,
    committed: BTreeMap<MemoryScope, Vec<CommittedMemory>>,
    tombstoned: HashSet<String>,
}

impl MemoryProjection {
    pub fn new() -> Self {
        Self {
            observations: BTreeMap::new(),
            reflections: BTreeMap::new(),
            committed: BTreeMap::new(),
            tombstoned: HashSet::new(),
        }
    }

    /// Query all memory for a given scope.
    pub fn query(&self, scope: MemoryScope) -> MemoryQueryResult {
        let observations = self.observations.get(&scope).cloned().unwrap_or_default();

        let reflection = self.reflections.get(&scope).cloned();

        let committed = self
            .committed
            .get(&scope)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| !self.tombstoned.contains(&e.memory_id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        MemoryQueryResult {
            observations,
            reflection,
            committed,
        }
    }

    /// Get the latest reflection for a scope.
    pub fn latest_reflection(&self, scope: MemoryScope) -> Option<&str> {
        self.reflections.get(&scope).map(String::as_str)
    }

    /// Count observations for a scope.
    pub fn observation_count(&self, scope: MemoryScope) -> usize {
        self.observations.get(&scope).map(Vec::len).unwrap_or(0)
    }
}

impl Default for MemoryProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl Projection for MemoryProjection {
    fn on_event(&mut self, event: &EventEnvelope) -> LagoResult<()> {
        match &event.payload {
            EventPayload::ObservationAppended {
                scope,
                observation_ref,
                ..
            } => {
                self.observations
                    .entry(*scope)
                    .or_default()
                    .push(observation_ref.to_string());
            }
            EventPayload::ReflectionCompacted {
                scope, summary_ref, ..
            } => {
                self.reflections.insert(*scope, summary_ref.to_string());
            }
            EventPayload::MemoryCommitted {
                scope,
                memory_id,
                committed_ref,
                supersedes,
            } => {
                // If this supersedes another memory, tombstone the old one
                if let Some(old_id) = supersedes {
                    self.tombstoned.insert(old_id.to_string());
                }
                self.committed
                    .entry(*scope)
                    .or_default()
                    .push(CommittedMemory {
                        memory_id: memory_id.to_string(),
                        committed_ref: committed_ref.to_string(),
                        supersedes: supersedes.as_ref().map(ToString::to_string),
                    });
            }
            EventPayload::MemoryTombstoned { memory_id, .. } => {
                self.tombstoned.insert(memory_id.to_string());
            }
            _ => {}
        }
        Ok(())
    }

    fn name(&self) -> &str {
        "arcan::memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::event::{EventEnvelope, EventPayload, MemoryScope};
    use lago_core::id::*;
    use std::collections::HashMap;

    fn make_envelope(seq: u64, payload: EventPayload) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::from_string("EVT001"),
            session_id: SessionId::from_string("SESS001"),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq,
            timestamp: 1_700_000_000_000_000 + seq,
            parent_id: None,
            payload,
            metadata: HashMap::new(),
            schema_version: 1,
        }
    }

    #[test]
    fn tracks_per_scope_observations() {
        let mut proj = MemoryProjection::new();
        proj.on_event(&make_envelope(
            1,
            EventPayload::ObservationAppended {
                scope: MemoryScope::Session,
                observation_ref: BlobHash::from_hex("obs1").into(),
                source_run_id: None,
            },
        ))
        .unwrap();
        proj.on_event(&make_envelope(
            2,
            EventPayload::ObservationAppended {
                scope: MemoryScope::User,
                observation_ref: BlobHash::from_hex("obs2").into(),
                source_run_id: None,
            },
        ))
        .unwrap();
        proj.on_event(&make_envelope(
            3,
            EventPayload::ObservationAppended {
                scope: MemoryScope::Session,
                observation_ref: BlobHash::from_hex("obs3").into(),
                source_run_id: None,
            },
        ))
        .unwrap();

        assert_eq!(proj.observation_count(MemoryScope::Session), 2);
        assert_eq!(proj.observation_count(MemoryScope::User), 1);
        assert_eq!(proj.observation_count(MemoryScope::Agent), 0);
    }

    #[test]
    fn stores_reflections() {
        let mut proj = MemoryProjection::new();
        proj.on_event(&make_envelope(
            1,
            EventPayload::ReflectionCompacted {
                scope: MemoryScope::Session,
                summary_ref: BlobHash::from_hex("summary1").into(),
                covers_through_seq: 10,
            },
        ))
        .unwrap();

        assert_eq!(
            proj.latest_reflection(MemoryScope::Session),
            Some("summary1")
        );
        assert_eq!(proj.latest_reflection(MemoryScope::User), None);

        // Second reflection replaces the first
        proj.on_event(&make_envelope(
            2,
            EventPayload::ReflectionCompacted {
                scope: MemoryScope::Session,
                summary_ref: BlobHash::from_hex("summary2").into(),
                covers_through_seq: 20,
            },
        ))
        .unwrap();

        assert_eq!(
            proj.latest_reflection(MemoryScope::Session),
            Some("summary2")
        );
    }

    #[test]
    fn tracks_committed_memories() {
        let mut proj = MemoryProjection::new();
        proj.on_event(&make_envelope(
            1,
            EventPayload::MemoryCommitted {
                scope: MemoryScope::User,
                memory_id: MemoryId::from_string("MEM001").into(),
                committed_ref: BlobHash::from_hex("ref1").into(),
                supersedes: None,
            },
        ))
        .unwrap();

        let result = proj.query(MemoryScope::User);
        assert_eq!(result.committed.len(), 1);
        assert_eq!(result.committed[0].memory_id, "MEM001");
    }

    #[test]
    fn respects_tombstones() {
        let mut proj = MemoryProjection::new();

        // Commit a memory
        proj.on_event(&make_envelope(
            1,
            EventPayload::MemoryCommitted {
                scope: MemoryScope::User,
                memory_id: MemoryId::from_string("MEM001").into(),
                committed_ref: BlobHash::from_hex("ref1").into(),
                supersedes: None,
            },
        ))
        .unwrap();

        // Tombstone it
        proj.on_event(&make_envelope(
            2,
            EventPayload::MemoryTombstoned {
                scope: MemoryScope::User,
                memory_id: MemoryId::from_string("MEM001").into(),
                reason: "outdated".to_string(),
            },
        ))
        .unwrap();

        let result = proj.query(MemoryScope::User);
        assert!(
            result.committed.is_empty(),
            "tombstoned memories should be excluded"
        );
    }

    #[test]
    fn handles_supersedes() {
        let mut proj = MemoryProjection::new();

        // Commit first memory
        proj.on_event(&make_envelope(
            1,
            EventPayload::MemoryCommitted {
                scope: MemoryScope::User,
                memory_id: MemoryId::from_string("MEM001").into(),
                committed_ref: BlobHash::from_hex("ref1").into(),
                supersedes: None,
            },
        ))
        .unwrap();

        // Commit second that supersedes first
        proj.on_event(&make_envelope(
            2,
            EventPayload::MemoryCommitted {
                scope: MemoryScope::User,
                memory_id: MemoryId::from_string("MEM002").into(),
                committed_ref: BlobHash::from_hex("ref2").into(),
                supersedes: Some(MemoryId::from_string("MEM001").into()),
            },
        ))
        .unwrap();

        let result = proj.query(MemoryScope::User);
        // MEM001 is tombstoned via supersedes, only MEM002 shows
        assert_eq!(result.committed.len(), 1);
        assert_eq!(result.committed[0].memory_id, "MEM002");
    }

    #[test]
    fn query_returns_correct_data() {
        let mut proj = MemoryProjection::new();

        // Add observation
        proj.on_event(&make_envelope(
            1,
            EventPayload::ObservationAppended {
                scope: MemoryScope::Session,
                observation_ref: BlobHash::from_hex("obs1").into(),
                source_run_id: Some("run-1".to_string()),
            },
        ))
        .unwrap();

        // Add reflection
        proj.on_event(&make_envelope(
            2,
            EventPayload::ReflectionCompacted {
                scope: MemoryScope::Session,
                summary_ref: BlobHash::from_hex("sum1").into(),
                covers_through_seq: 1,
            },
        ))
        .unwrap();

        // Add committed memory
        proj.on_event(&make_envelope(
            3,
            EventPayload::MemoryCommitted {
                scope: MemoryScope::Session,
                memory_id: MemoryId::from_string("MEM001").into(),
                committed_ref: BlobHash::from_hex("ref1").into(),
                supersedes: None,
            },
        ))
        .unwrap();

        let result = proj.query(MemoryScope::Session);
        assert_eq!(result.observations.len(), 1);
        assert_eq!(result.observations[0], "obs1");
        assert_eq!(result.reflection.as_deref(), Some("sum1"));
        assert_eq!(result.committed.len(), 1);

        // Empty scope returns empty
        let empty_result = proj.query(MemoryScope::Org);
        assert!(empty_result.observations.is_empty());
        assert!(empty_result.reflection.is_none());
        assert!(empty_result.committed.is_empty());
    }

    #[test]
    fn projection_name() {
        let proj = MemoryProjection::new();
        assert_eq!(proj.name(), "arcan::memory");
    }
}
