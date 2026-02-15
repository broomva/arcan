use crate::memory_projection::MemoryProjection;
use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use lago_core::Journal;
use lago_core::event::{EventEnvelope, EventPayload, MemoryScope};
use lago_core::id::*;
use serde_json::json;
use std::sync::{Arc, RwLock};

fn tool_err(msg: impl Into<String>) -> CoreError {
    CoreError::ToolExecution {
        tool_name: "memory".to_string(),
        message: msg.into(),
    }
}

fn parse_scope(s: &str) -> Result<MemoryScope, CoreError> {
    match s {
        "session" => Ok(MemoryScope::Session),
        "user" => Ok(MemoryScope::User),
        "agent" => Ok(MemoryScope::Agent),
        "org" => Ok(MemoryScope::Org),
        other => Err(tool_err(format!("invalid scope: {other}"))),
    }
}

/// Tool that queries the memory projection for observations, reflections, and committed memories.
pub struct MemoryQueryTool {
    projection: Arc<RwLock<MemoryProjection>>,
}

impl MemoryQueryTool {
    pub fn new(projection: Arc<RwLock<MemoryProjection>>) -> Self {
        Self { projection }
    }
}

impl Tool for MemoryQueryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_query".to_string(),
            description: "Query agent memory for a given scope. Returns observations, reflections, and committed memories.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["session", "user", "agent", "org"],
                        "description": "Memory scope to query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of entries to return"
                    }
                },
                "required": ["scope"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("memory".to_string()),
            tags: vec!["memory".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let scope_str = call
            .input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tool_err("missing 'scope' field".to_string()))?;

        let scope = parse_scope(scope_str)?;
        let limit = call
            .input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize);

        let proj = self
            .projection
            .read()
            .map_err(|e| tool_err(format!("lock error: {e}")))?;
        let result = proj.query(scope);

        let observations: Vec<&String> = if let Some(lim) = limit {
            result.observations.iter().take(lim).collect()
        } else {
            result.observations.iter().collect()
        };

        let committed: Vec<serde_json::Value> = result
            .committed
            .iter()
            .map(|c| {
                json!({
                    "memory_id": c.memory_id,
                    "committed_ref": c.committed_ref,
                })
            })
            .collect();

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "entries": observations,
                "observation_summary": format!("{} observations", result.observations.len()),
                "reflection": result.reflection,
                "committed": committed,
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Tool that proposes new memory entries by writing a MemoryProposed event to the journal.
pub struct MemoryProposeTool {
    journal: Arc<dyn Journal>,
    default_branch_id: BranchId,
}

impl MemoryProposeTool {
    /// Create with explicit session and branch IDs (for testing or fixed-session use).
    pub fn with_session(
        journal: Arc<dyn Journal>,
        _session_id: SessionId,
        branch_id: BranchId,
    ) -> Self {
        Self {
            journal,
            default_branch_id: branch_id,
        }
    }

    /// Create with journal only — session_id derived from ToolContext at execution time.
    pub fn new(journal: Arc<dyn Journal>) -> Self {
        Self {
            journal,
            default_branch_id: BranchId::from_string("main"),
        }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }
}

impl Tool for MemoryProposeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_propose".to_string(),
            description: "Propose new memory entries for storage. Creates a MemoryProposed event."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["session", "user", "agent", "org"],
                        "description": "Memory scope for the proposal"
                    },
                    "entries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content": { "type": "string" }
                            },
                            "required": ["content"]
                        },
                        "description": "Memory entries to propose"
                    }
                },
                "required": ["scope", "entries"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("memory".to_string()),
            tags: vec!["memory".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let session_id = SessionId::from_string(&ctx.session_id);
        let branch_id = &self.default_branch_id;

        let scope_str = call
            .input
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tool_err("missing 'scope' field".to_string()))?;

        let scope = parse_scope(scope_str)?;

        let entries = call
            .input
            .get("entries")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| tool_err("missing or invalid 'entries' field".to_string()))?;

        if entries.is_empty() {
            return Err(tool_err("entries must not be empty".to_string()));
        }

        // Validate entries have content
        for entry in entries {
            if entry
                .get("content")
                .and_then(serde_json::Value::as_str)
                .is_none()
            {
                return Err(tool_err(
                    "each entry must have a 'content' string field".to_string(),
                ));
            }
        }

        let proposal_id = MemoryId::new();
        let entries_json = serde_json::to_string(entries)
            .map_err(|e| tool_err(format!("serialize error: {e}")))?;
        let entries_ref = BlobHash::from_hex(format!("{:x}", md5_hash(entries_json.as_bytes())));

        let head_seq = self
            .block_on(self.journal.head_seq(&session_id, branch_id))
            .map_err(|e| tool_err(format!("journal error: {e}")))?;
        let seq = head_seq + 1;

        let envelope = EventEnvelope {
            event_id: EventId::new(),
            session_id,
            branch_id: branch_id.clone(),
            run_id: Some(RunId::from_string(&ctx.run_id)),
            seq,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::MemoryProposed {
                scope,
                proposal_id: proposal_id.clone(),
                entries_ref,
                source_run_id: Some(ctx.run_id.clone()),
            },
            metadata: std::collections::HashMap::new(),
            schema_version: 1,
        };

        self.block_on(self.journal.append(envelope))
            .map_err(|e| tool_err(format!("journal append error: {e}")))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "proposal_id": proposal_id.as_str(),
                "entry_count": entries.len(),
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Tool that commits a previously proposed memory by writing a MemoryCommitted event.
pub struct MemoryCommitTool {
    journal: Arc<dyn Journal>,
    default_branch_id: BranchId,
}

impl MemoryCommitTool {
    /// Create with explicit session and branch IDs (for testing or fixed-session use).
    pub fn with_session(
        journal: Arc<dyn Journal>,
        _session_id: SessionId,
        branch_id: BranchId,
    ) -> Self {
        Self {
            journal,
            default_branch_id: branch_id,
        }
    }

    /// Create with journal only — session_id derived from ToolContext at execution time.
    pub fn new(journal: Arc<dyn Journal>) -> Self {
        Self {
            journal,
            default_branch_id: BranchId::from_string("main"),
        }
    }

    fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        tokio::runtime::Handle::current().block_on(f)
    }
}

impl Tool for MemoryCommitTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_commit".to_string(),
            description: "Commit a previously proposed memory. Creates a MemoryCommitted event."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "proposal_id": {
                        "type": "string",
                        "description": "ID of the proposal to commit"
                    },
                    "supersedes": {
                        "type": "string",
                        "description": "Optional ID of a memory this replaces"
                    }
                },
                "required": ["proposal_id"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("memory".to_string()),
            tags: vec!["memory".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let session_id = SessionId::from_string(&ctx.session_id);
        let branch_id = &self.default_branch_id;

        let proposal_id_str = call
            .input
            .get("proposal_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tool_err("missing 'proposal_id' field".to_string()))?;

        if proposal_id_str.is_empty() {
            return Err(tool_err("proposal_id must not be empty".to_string()));
        }

        let supersedes = call
            .input
            .get("supersedes")
            .and_then(serde_json::Value::as_str)
            .map(MemoryId::from_string);

        let memory_id = MemoryId::new();
        let committed_ref = BlobHash::from_hex(format!("committed-{proposal_id_str}"));

        let head_seq = self
            .block_on(self.journal.head_seq(&session_id, branch_id))
            .map_err(|e| tool_err(format!("journal error: {e}")))?;
        let seq = head_seq + 1;

        let envelope = EventEnvelope {
            event_id: EventId::new(),
            session_id,
            branch_id: branch_id.clone(),
            run_id: None,
            seq,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::MemoryCommitted {
                scope: MemoryScope::Session, // Default scope for commit
                memory_id: memory_id.clone(),
                committed_ref,
                supersedes,
            },
            metadata: std::collections::HashMap::new(),
            schema_version: 1,
        };

        self.block_on(self.journal.append(envelope))
            .map_err(|e| tool_err(format!("journal append error: {e}")))?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "memory_id": memory_id.as_str(),
                "committed": true,
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Simple hash for generating blob references (not cryptographic).
fn md5_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_journal::RedbJournal;

    fn make_test_env() -> (Arc<dyn Journal>, SessionId, BranchId) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.redb");
        std::mem::forget(dir);
        let journal = RedbJournal::open(db_path).unwrap();
        let session_id = SessionId::from_string("test-session");
        let branch_id = BranchId::from_string("main");
        (Arc::new(journal), session_id, branch_id)
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            run_id: "run-1".to_string(),
            session_id: "test-session".to_string(),
            iteration: 1,
        }
    }

    #[tokio::test]
    async fn query_returns_observations() {
        let proj = Arc::new(RwLock::new(MemoryProjection::new()));

        // Feed the projection some events
        {
            let mut p = proj.write().unwrap();
            use lago_core::projection::Projection;
            p.on_event(&EventEnvelope {
                event_id: EventId::new(),
                session_id: SessionId::from_string("s1"),
                branch_id: BranchId::from_string("main"),
                run_id: None,
                seq: 1,
                timestamp: 100,
                parent_id: None,
                payload: EventPayload::ObservationAppended {
                    scope: MemoryScope::Session,
                    observation_ref: BlobHash::from_hex("obs1"),
                    source_run_id: None,
                },
                metadata: std::collections::HashMap::new(),
                schema_version: 1,
            })
            .unwrap();
        }

        let tool = MemoryQueryTool::new(proj);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "memory_query".to_string(),
            input: json!({"scope": "session"}),
        };
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert!(!result.is_error);
        assert!(result.output["entries"].as_array().unwrap().len() == 1);
    }

    #[tokio::test]
    async fn query_empty_scope_returns_empty() {
        let proj = Arc::new(RwLock::new(MemoryProjection::new()));
        let tool = MemoryQueryTool::new(proj);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "memory_query".to_string(),
            input: json!({"scope": "agent"}),
        };
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert!(result.output["entries"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn query_validates_scope() {
        let proj = Arc::new(RwLock::new(MemoryProjection::new()));
        let tool = MemoryQueryTool::new(proj);
        let call = ToolCall {
            call_id: "c1".to_string(),
            tool_name: "memory_query".to_string(),
            input: json!({"scope": "invalid"}),
        };
        let result = tool.execute(&call, &make_ctx());
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn propose_creates_event() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool =
            MemoryProposeTool::with_session(journal.clone(), session_id.clone(), branch_id.clone());

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_propose".to_string(),
                input: json!({
                    "scope": "session",
                    "entries": [{"content": "User likes dark mode"}]
                }),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap()
        .unwrap();

        assert!(!result.is_error);
        assert!(result.output["proposal_id"].as_str().is_some());
        assert_eq!(result.output["entry_count"], 1);
    }

    #[tokio::test]
    async fn propose_validates_entries() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool = MemoryProposeTool::with_session(journal, session_id, branch_id);

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_propose".to_string(),
                input: json!({
                    "scope": "session",
                    "entries": []
                }),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap();

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn propose_returns_proposal_id() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool = MemoryProposeTool::with_session(journal, session_id, branch_id);

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_propose".to_string(),
                input: json!({
                    "scope": "user",
                    "entries": [{"content": "test entry"}]
                }),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap()
        .unwrap();

        let pid = result.output["proposal_id"].as_str().unwrap();
        assert!(!pid.is_empty());
    }

    #[tokio::test]
    async fn commit_creates_committed_event() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool =
            MemoryCommitTool::with_session(journal.clone(), session_id.clone(), branch_id.clone());

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_commit".to_string(),
                input: json!({"proposal_id": "PROP001"}),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap()
        .unwrap();

        assert!(!result.is_error);
        assert!(result.output["memory_id"].as_str().is_some());
        assert_eq!(result.output["committed"], true);
    }

    #[tokio::test]
    async fn commit_with_supersedes() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool = MemoryCommitTool::with_session(journal, session_id, branch_id);

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_commit".to_string(),
                input: json!({
                    "proposal_id": "PROP002",
                    "supersedes": "MEM_OLD"
                }),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap()
        .unwrap();

        assert!(!result.is_error);
        assert_eq!(result.output["committed"], true);
    }

    #[tokio::test]
    async fn invalid_proposal_rejected() {
        let (journal, session_id, branch_id) = make_test_env();
        let tool = MemoryCommitTool::with_session(journal, session_id, branch_id);

        let result = tokio::task::spawn_blocking(move || {
            let call = ToolCall {
                call_id: "c1".to_string(),
                tool_name: "memory_commit".to_string(),
                input: json!({"proposal_id": ""}),
            };
            tool.execute(&call, &make_ctx())
        })
        .await
        .unwrap();

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn tool_definitions_have_correct_schemas() {
        let proj = Arc::new(RwLock::new(MemoryProjection::new()));
        let (journal, session_id, branch_id) = make_test_env();

        let query_tool = MemoryQueryTool::new(proj);
        let propose_tool =
            MemoryProposeTool::with_session(journal.clone(), session_id.clone(), branch_id.clone());
        let commit_tool = MemoryCommitTool::with_session(journal, session_id, branch_id);

        let q_def = query_tool.definition();
        assert_eq!(q_def.name, "memory_query");
        assert!(q_def.input_schema["properties"]["scope"].is_object());

        let p_def = propose_tool.definition();
        assert_eq!(p_def.name, "memory_propose");
        assert!(p_def.input_schema["properties"]["entries"].is_object());

        let c_def = commit_tool.definition();
        assert_eq!(c_def.name, "memory_commit");
        assert!(c_def.input_schema["properties"]["proposal_id"].is_object());
    }
}
