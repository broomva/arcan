use arcan_core::protocol::AgentEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub event: AgentEvent,
}

#[derive(Debug, Clone)]
pub struct AppendEvent {
    pub session_id: String,
    pub parent_id: Option<String>,
    pub event: AgentEvent,
}

pub trait SessionRepository: Send + Sync {
    fn append(&self, request: AppendEvent) -> Result<EventRecord, StoreError>;
    fn load_session(&self, session_id: &str) -> Result<Vec<EventRecord>, StoreError>;
    fn load_children(&self, parent_id: &str) -> Result<Vec<EventRecord>, StoreError>;
    fn head(&self, session_id: &str) -> Result<Option<EventRecord>, StoreError>;
}

#[derive(Default)]
pub struct InMemorySessionRepository {
    by_session: RwLock<HashMap<String, Vec<EventRecord>>>,
}

impl SessionRepository for InMemorySessionRepository {
    fn append(&self, request: AppendEvent) -> Result<EventRecord, StoreError> {
        let record = EventRecord {
            id: Uuid::new_v4().to_string(),
            session_id: request.session_id,
            parent_id: request.parent_id,
            timestamp: Utc::now(),
            event: request.event,
        };

        let mut guard = self
            .by_session
            .write()
            .map_err(|_| StoreError::PoisonedLock("in-memory write".to_string()))?;

        guard
            .entry(record.session_id.clone())
            .or_default()
            .push(record.clone());

        Ok(record)
    }

    fn load_session(&self, session_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        let guard = self
            .by_session
            .read()
            .map_err(|_| StoreError::PoisonedLock("in-memory read".to_string()))?;
        Ok(guard.get(session_id).cloned().unwrap_or_default())
    }

    fn load_children(&self, parent_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        let guard = self
            .by_session
            .read()
            .map_err(|_| StoreError::PoisonedLock("in-memory read".to_string()))?;

        let mut out = Vec::new();
        for records in guard.values() {
            for record in records {
                if record.parent_id.as_deref() == Some(parent_id) {
                    out.push(record.clone());
                }
            }
        }

        Ok(out)
    }

    fn head(&self, session_id: &str) -> Result<Option<EventRecord>, StoreError> {
        let guard = self
            .by_session
            .read()
            .map_err(|_| StoreError::PoisonedLock("in-memory read".to_string()))?;
        Ok(guard
            .get(session_id)
            .and_then(|records| records.last().cloned()))
    }
}

pub struct JsonlSessionRepository {
    root: PathBuf,
}

impl JsonlSessionRepository {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn session_file(&self, session_id: &str) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    fn ensure_root(&self) -> Result<(), StoreError> {
        create_dir_all(&self.root).map_err(|source| StoreError::Io {
            path: self.root.clone(),
            source,
        })
    }

    fn read_records(path: &Path) -> Result<Vec<EventRecord>, StoreError> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(path).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        let reader = BufReader::new(file);
        let mut records = Vec::new();

        for line in reader.lines() {
            let line = line.map_err(|source| StoreError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }

            let record: EventRecord =
                serde_json::from_str(&line).map_err(|source| StoreError::Serde { source })?;
            records.push(record);
        }

        Ok(records)
    }
}

impl SessionRepository for JsonlSessionRepository {
    fn append(&self, request: AppendEvent) -> Result<EventRecord, StoreError> {
        self.ensure_root()?;

        let record = EventRecord {
            id: Uuid::new_v4().to_string(),
            session_id: request.session_id.clone(),
            parent_id: request.parent_id,
            timestamp: Utc::now(),
            event: request.event,
        };

        let path = self.session_file(&request.session_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;

        let line = serde_json::to_string(&record).map_err(|source| StoreError::Serde { source })?;
        file.write_all(line.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;

        Ok(record)
    }

    fn load_session(&self, session_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        Self::read_records(&self.session_file(session_id))
    }

    fn load_children(&self, parent_id: &str) -> Result<Vec<EventRecord>, StoreError> {
        self.ensure_root()?;

        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root).map_err(|source| StoreError::Io {
            path: self.root.clone(),
            source,
        })? {
            let entry = entry.map_err(|source| StoreError::Io {
                path: self.root.clone(),
                source,
            })?;

            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            for record in Self::read_records(&path)? {
                if record.parent_id.as_deref() == Some(parent_id) {
                    out.push(record);
                }
            }
        }

        Ok(out)
    }

    fn head(&self, session_id: &str) -> Result<Option<EventRecord>, StoreError> {
        Ok(Self::read_records(&self.session_file(session_id))?.pop())
    }
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("IO error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("serialization error: {source}")]
    Serde {
        #[source]
        source: serde_json::Error,
    },
    #[error("in-memory store lock was poisoned: {0}")]
    PoisonedLock(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::protocol::{AgentEvent, RunStopReason};

    fn make_event(run_id: &str, session_id: &str) -> AgentEvent {
        AgentEvent::RunFinished {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("ok".to_string()),
        }
    }

    #[test]
    fn appends_and_reads_head() {
        let store = InMemorySessionRepository::default();
        store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: None,
                event: make_event("r1", "s1"),
            })
            .expect("append should succeed");

        let head = store
            .head("s1")
            .expect("head should load")
            .expect("head exists");
        assert_eq!(head.session_id, "s1");
    }

    #[test]
    fn load_session_returns_all_events_in_order() {
        let store = InMemorySessionRepository::default();
        for i in 0..5 {
            store
                .append(AppendEvent {
                    session_id: "s1".to_string(),
                    parent_id: None,
                    event: make_event(&format!("r{i}"), "s1"),
                })
                .unwrap();
        }

        let records = store.load_session("s1").unwrap();
        assert_eq!(records.len(), 5);
    }

    #[test]
    fn empty_session_returns_empty() {
        let store = InMemorySessionRepository::default();
        let records = store.load_session("nonexistent").unwrap();
        assert!(records.is_empty());
        assert!(store.head("nonexistent").unwrap().is_none());
    }

    #[test]
    fn sessions_are_isolated() {
        let store = InMemorySessionRepository::default();
        store
            .append(AppendEvent {
                session_id: "a".to_string(),
                parent_id: None,
                event: make_event("r1", "a"),
            })
            .unwrap();
        store
            .append(AppendEvent {
                session_id: "b".to_string(),
                parent_id: None,
                event: make_event("r2", "b"),
            })
            .unwrap();

        assert_eq!(store.load_session("a").unwrap().len(), 1);
        assert_eq!(store.load_session("b").unwrap().len(), 1);
    }

    #[test]
    fn load_children_filters_by_parent() {
        let store = InMemorySessionRepository::default();
        let parent = store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: None,
                event: make_event("r1", "s1"),
            })
            .unwrap();

        store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: Some(parent.id.clone()),
                event: make_event("r2", "s1"),
            })
            .unwrap();

        store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: None,
                event: make_event("r3", "s1"),
            })
            .unwrap();

        let children = store.load_children(&parent.id).unwrap();
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn jsonl_repo_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonlSessionRepository::new(dir.path().to_path_buf());

        store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: None,
                event: make_event("r1", "s1"),
            })
            .unwrap();

        store
            .append(AppendEvent {
                session_id: "s1".to_string(),
                parent_id: None,
                event: make_event("r2", "s1"),
            })
            .unwrap();

        let records = store.load_session("s1").unwrap();
        assert_eq!(records.len(), 2);

        let head = store.head("s1").unwrap().unwrap();
        assert!(matches!(
            head.event,
            AgentEvent::RunFinished { ref run_id, .. } if run_id == "r2"
        ));
    }

    #[test]
    fn jsonl_repo_empty_session() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonlSessionRepository::new(dir.path().to_path_buf());
        assert!(store.load_session("nope").unwrap().is_empty());
        assert!(store.head("nope").unwrap().is_none());
    }
}
