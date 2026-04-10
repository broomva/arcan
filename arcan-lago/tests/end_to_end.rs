//! End-to-end integration test for the Arcan ↔ Lago bridge.
//!
//! Exercises the full cycle: Arcan events → Lago journal → read back →
//! AppStateProjection → verify reconstructed state.

use std::sync::Arc;

use aios_protocol::{
    BranchId as KernelBranchId, EventKind, EventRecord, SessionId as KernelSessionId, SpanStatus,
    ToolRunId,
};
use arcan_core::protocol::{
    AgentEvent, RunStopReason, StatePatch, StatePatchFormat, StatePatchSource, ToolCall,
    ToolResultSummary,
};
use arcan_lago::{
    AppStateProjection, LagoPolicyMiddleware, LagoSessionRepository, derive_knowledge_records,
};
use arcan_store::session::{AppendEvent, SessionRepository};
use lago_core::event::PolicyDecisionKind;
use lago_core::{Journal, Projection, protocol_bridge};
use lago_journal::RedbJournal;
use lago_policy::engine::PolicyEngine;
use lago_policy::rule::{MatchCondition, Rule};
use serde_json::json;

fn open_journal() -> Arc<RedbJournal> {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("e2e-test.redb");
    std::mem::forget(dir);
    Arc::new(RedbJournal::open(db_path).unwrap())
}

async fn append_protocol_record(journal: &dyn Journal, record: EventRecord) {
    let envelope = protocol_bridge::from_protocol(&record.to_envelope()).unwrap();
    journal.append(envelope).await.unwrap();
}

/// Full agent session lifecycle: start → tool call → tool result → state patch → finish.
/// Verifies that all events survive the Arcan → Lago → Arcan round trip and that
/// the AppStateProjection correctly reconstructs conversation history and app state.
#[tokio::test]
async fn full_session_lifecycle_round_trip() {
    let journal = open_journal();
    let repo = Arc::new(LagoSessionRepository::new(journal.clone()));

    let session_id = "e2e-session-1";

    // Simulate a complete agent session
    let events = vec![
        AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: session_id.into(),
            provider: "anthropic".into(),
            max_iterations: 10,
        },
        AgentEvent::IterationStarted {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
        },
        AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
            delta: "Let me read ".into(),
        },
        AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
            delta: "that file for you.".into(),
        },
        AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
            call: ToolCall {
                call_id: "call-1".into(),
                tool_name: "read_file".into(),
                input: serde_json::json!({"path": "/tmp/test.txt"}),
            },
        },
        AgentEvent::ToolCallCompleted {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
            result: ToolResultSummary {
                call_id: "call-1".into(),
                tool_name: "read_file".into(),
                output: serde_json::json!({"content": "Hello, World!"}),
            },
        },
        AgentEvent::StatePatched {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 1,
            patch: StatePatch {
                format: StatePatchFormat::MergePatch,
                patch: serde_json::json!({"cwd": "/tmp", "last_file": "test.txt"}),
                source: StatePatchSource::Tool,
            },
            revision: 1,
        },
        AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: session_id.into(),
            iteration: 2,
            delta: "The file contains: Hello, World!".into(),
        },
        AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: session_id.into(),
            reason: RunStopReason::Completed,
            total_iterations: 2,
            final_answer: Some("The file contains: Hello, World!".into()),
            usage: None,
        },
    ];

    // Write all events through the repository (Arcan → Lago journal)
    let events_clone = events.clone();
    tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || {
            for event in events_clone {
                repo.append(AppendEvent {
                    session_id: session_id.to_string(),
                    branch_id: "main".to_string(),
                    parent_id: None,
                    event,
                })
                .expect("append should succeed");
            }
        }
    })
    .await
    .unwrap();

    // Read back through the repository (Lago journal → Arcan events)
    let records = tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || {
            repo.load_session(session_id, "main")
                .expect("load should succeed")
        }
    })
    .await
    .unwrap();

    // Verify all mappable events came back
    assert_eq!(records.len(), 9, "all 9 events should round-trip");

    // Verify event types in order
    assert!(matches!(records[0].event, AgentEvent::RunStarted { .. }));
    assert!(matches!(
        records[1].event,
        AgentEvent::IterationStarted { .. }
    ));
    assert!(matches!(records[2].event, AgentEvent::TextDelta { .. }));
    assert!(matches!(records[3].event, AgentEvent::TextDelta { .. }));
    assert!(matches!(
        records[4].event,
        AgentEvent::ToolCallRequested { .. }
    ));
    assert!(matches!(
        records[5].event,
        AgentEvent::ToolCallCompleted { .. }
    ));
    assert!(matches!(records[6].event, AgentEvent::StatePatched { .. }));
    assert!(matches!(records[7].event, AgentEvent::TextDelta { .. }));
    assert!(matches!(records[8].event, AgentEvent::RunFinished { .. }));

    // Replay through AppStateProjection via the journal directly
    let session_id_lago = lago_core::SessionId::from(session_id.to_string());
    let branch_id = lago_core::BranchId::from("main");
    let query = lago_core::EventQuery::new()
        .session(session_id_lago)
        .branch(branch_id);
    let envelopes = journal.read(query).await.expect("read should succeed");

    let mut projection = AppStateProjection::new();
    for envelope in &envelopes {
        projection
            .on_event(envelope)
            .expect("projection should succeed");
    }

    // Verify reconstructed conversation history
    // The projection aggregates consecutive text deltas into a single assistant
    // message, so we get: assistant (aggregated deltas), tool result, assistant.
    let messages = projection.messages();
    assert_eq!(
        messages.len(),
        3,
        "should have 3 messages: 1 aggregated assistant, 1 tool, 1 assistant"
    );

    // First message: aggregated text deltas from iteration 1
    assert_eq!(messages[0].content, "Let me read that file for you.");
    assert_eq!(messages[0].role, arcan_core::protocol::Role::Assistant);

    // Second message: tool result
    assert_eq!(messages[1].role, arcan_core::protocol::Role::Tool);
    assert!(messages[1].content.contains("Hello, World!"));

    // Third message: assistant text from iteration 2
    assert_eq!(messages[2].role, arcan_core::protocol::Role::Assistant);
    assert_eq!(messages[2].content, "The file contains: Hello, World!");

    // Verify reconstructed app state
    let state = projection.state();
    assert_eq!(state.cwd(), Some("/tmp".to_string()));
}

/// Verify that policy evaluation works end-to-end with the default policy pattern:
/// deny shell, approve critical, allow filesystem.
#[tokio::test]
async fn policy_middleware_with_default_rules() {
    use arcan_core::protocol::ToolAnnotations;
    use arcan_core::runtime::{Middleware, ToolContext};
    use std::collections::HashMap;

    // Build a policy engine matching the default-policy.toml pattern
    let mut engine = PolicyEngine::new();
    engine.add_rule(Rule {
        id: "deny-shell".into(),
        name: "Deny shell".into(),
        priority: 10,
        condition: MatchCondition::ToolName("exec_shell".into()),
        decision: PolicyDecisionKind::Deny,
        explanation: Some("Shell execution not permitted".into()),
        required_sandbox: None,
    });
    engine.add_rule(Rule {
        id: "approve-critical".into(),
        name: "Approve critical".into(),
        priority: 20,
        condition: MatchCondition::RiskAtLeast(lago_core::event::RiskLevel::Critical),
        decision: PolicyDecisionKind::RequireApproval,
        explanation: None,
        required_sandbox: None,
    });
    engine.add_rule(Rule {
        id: "allow-filesystem".into(),
        name: "Allow filesystem".into(),
        priority: 50,
        condition: MatchCondition::ToolPattern("file_*".into()),
        decision: PolicyDecisionKind::Allow,
        explanation: None,
        required_sandbox: None,
    });
    engine.add_rule(Rule {
        id: "default-allow".into(),
        name: "Default allow".into(),
        priority: 1000,
        condition: MatchCondition::Always,
        decision: PolicyDecisionKind::Allow,
        explanation: None,
        required_sandbox: None,
    });

    let mut annotations = HashMap::new();
    annotations.insert(
        "exec_shell".to_string(),
        ToolAnnotations {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: true,
            requires_confirmation: true,
        },
    );
    annotations.insert(
        "file_read".to_string(),
        ToolAnnotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
            requires_confirmation: false,
        },
    );
    annotations.insert(
        "file_write".to_string(),
        ToolAnnotations {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: false,
            requires_confirmation: false,
        },
    );

    let middleware = LagoPolicyMiddleware::new(engine, annotations);
    let ctx = ToolContext {
        run_id: "r1".into(),
        session_id: "s1".into(),
        iteration: 1,
    };

    // Shell should be denied
    let shell_call = ToolCall {
        call_id: "c1".into(),
        tool_name: "exec_shell".into(),
        input: serde_json::json!({"command": "rm -rf /"}),
    };
    let result = middleware.pre_tool_call(&ctx, &shell_call);
    assert!(result.is_err(), "shell should be denied");
    assert!(
        result.unwrap_err().to_string().contains("not permitted"),
        "error should mention denial"
    );

    // File read should be allowed
    let read_call = ToolCall {
        call_id: "c2".into(),
        tool_name: "file_read".into(),
        input: serde_json::json!({"path": "/tmp/test.txt"}),
    };
    assert!(
        middleware.pre_tool_call(&ctx, &read_call).is_ok(),
        "file_read should be allowed"
    );

    // File write should be allowed
    let write_call = ToolCall {
        call_id: "c3".into(),
        tool_name: "file_write".into(),
        input: serde_json::json!({"path": "/tmp/out.txt", "content": "data"}),
    };
    assert!(
        middleware.pre_tool_call(&ctx, &write_call).is_ok(),
        "file_write should be allowed"
    );

    // Unknown tool should be allowed (default-allow)
    let unknown_call = ToolCall {
        call_id: "c4".into(),
        tool_name: "custom_tool".into(),
        input: serde_json::json!({}),
    };
    assert!(
        middleware.pre_tool_call(&ctx, &unknown_call).is_ok(),
        "unknown tools should be allowed by default"
    );
}

/// Verify multiple sessions can coexist in the same journal.
#[tokio::test]
async fn multiple_sessions_isolated() {
    let journal = open_journal();
    let repo = Arc::new(LagoSessionRepository::new(journal));

    tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || {
            // Session 1
            repo.append(AppendEvent {
                session_id: "session-a".into(),
                branch_id: "main".into(),
                parent_id: None,
                event: AgentEvent::TextDelta {
                    run_id: "r1".into(),
                    session_id: "session-a".into(),
                    iteration: 1,
                    delta: "Hello from A".into(),
                },
            })
            .unwrap();

            // Session 2
            repo.append(AppendEvent {
                session_id: "session-b".into(),
                branch_id: "main".into(),
                parent_id: None,
                event: AgentEvent::TextDelta {
                    run_id: "r2".into(),
                    session_id: "session-b".into(),
                    iteration: 1,
                    delta: "Hello from B".into(),
                },
            })
            .unwrap();

            // Session 1 again
            repo.append(AppendEvent {
                session_id: "session-a".into(),
                branch_id: "main".into(),
                parent_id: None,
                event: AgentEvent::RunFinished {
                    run_id: "r1".into(),
                    session_id: "session-a".into(),
                    reason: RunStopReason::Completed,
                    total_iterations: 1,
                    final_answer: Some("done A".into()),
                    usage: None,
                },
            })
            .unwrap();
        }
    })
    .await
    .unwrap();

    // Load session A
    let records_a = tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || repo.load_session("session-a", "main").unwrap()
    })
    .await
    .unwrap();
    assert_eq!(records_a.len(), 2, "session-a should have 2 events");

    // Load session B
    let records_b = tokio::task::spawn_blocking({
        let repo = repo.clone();
        move || repo.load_session("session-b", "main").unwrap()
    })
    .await
    .unwrap();
    assert_eq!(records_b.len(), 1, "session-b should have 1 event");
}

#[tokio::test]
async fn reasoning_trace_can_be_reconstructed_from_trace_id() {
    let journal = open_journal();
    let session_id = KernelSessionId::from_string("trace-session");
    let branch_id = KernelBranchId::from_string("main");
    let trace_id = "trace-reasoning-123";

    let mut wake_up = EventRecord::new(
        session_id.clone(),
        branch_id.clone(),
        1,
        EventKind::KnowledgeRetrieved {
            note_count: 8,
            context_tokens: 600,
            source: "wake_up".into(),
        },
    );
    wake_up.trace_id = Some(trace_id.into());
    wake_up.span_id = Some("span-wake".into());
    append_protocol_record(journal.as_ref(), wake_up).await;

    let mut search_source = EventRecord::new(
        session_id.clone(),
        branch_id.clone(),
        2,
        EventKind::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some("call-search".into()),
            tool_name: "wiki_search".into(),
            result: json!({
                "status": "success",
                "output": {
                    "query": "temporal validity",
                    "results": "1. temporal-validity.md [score: 0.93]\n   grounded note",
                    "count": 1,
                    "top_relevance": 0.93,
                    "duration_ms": 15,
                    "context_tokens": 40
                }
            }),
            duration_ms: 15,
            status: SpanStatus::Ok,
        },
    );
    search_source.trace_id = Some(trace_id.into());
    search_source.span_id = Some("span-search".into());
    append_protocol_record(journal.as_ref(), search_source.clone()).await;
    for record in derive_knowledge_records(&search_source, 3) {
        append_protocol_record(journal.as_ref(), record).await;
    }

    let mut eval_event = EventRecord::new(
        session_id.clone(),
        branch_id.clone(),
        5,
        EventKind::Custom {
            event_type: "eval.AsyncCompleted".into(),
            data: json!({
                "evaluator": "reasoning_coherence",
                "score": 0.82,
                "label": "good",
                "layer": "reasoning"
            }),
        },
    );
    eval_event.trace_id = Some(trace_id.into());
    eval_event.span_id = Some("span-eval".into());
    append_protocol_record(journal.as_ref(), eval_event).await;

    let mut lint_source = EventRecord::new(
        session_id.clone(),
        branch_id.clone(),
        6,
        EventKind::ToolCallCompleted {
            tool_run_id: ToolRunId::default(),
            call_id: Some("call-lint".into()),
            tool_name: "wiki_lint".into(),
            result: json!({
                "status": "success",
                "output": {
                    "health_score": 0.82,
                    "note_count": 64,
                    "contradictions": 0,
                    "missing_pages": 2,
                    "orphans": 1
                }
            }),
            duration_ms: 11,
            status: SpanStatus::Ok,
        },
    );
    lint_source.trace_id = Some(trace_id.into());
    lint_source.span_id = Some("span-lint".into());
    append_protocol_record(journal.as_ref(), lint_source.clone()).await;
    for record in derive_knowledge_records(&lint_source, 7) {
        append_protocol_record(journal.as_ref(), record).await;
    }

    let query = lago_core::EventQuery::new()
        .session(lago_core::SessionId::from_string("trace-session"))
        .branch(lago_core::BranchId::from_string("main"));
    let trace_events: Vec<_> = journal
        .read(query)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|envelope| envelope.to_protocol())
        .filter(|event| event.trace_id.as_deref() == Some(trace_id))
        .collect();

    let kinds: Vec<_> = trace_events
        .iter()
        .map(|event| match &event.kind {
            EventKind::KnowledgeRetrieved { source, .. } if source == "wake_up" => "wake_up",
            EventKind::KnowledgeSearched { .. } => "searched",
            EventKind::Custom { event_type, .. } if event_type == "eval.AsyncCompleted" => "eval",
            EventKind::KnowledgeEvaluated { .. } => "evaluated",
            EventKind::ToolCallCompleted { tool_name, .. } if tool_name == "wiki_search" => {
                "search_source"
            }
            EventKind::ToolCallCompleted { tool_name, .. } if tool_name == "wiki_lint" => {
                "lint_source"
            }
            EventKind::KnowledgeRetrieved { source, .. } if source == "tool_search" => {
                "tool_retrieval"
            }
            _ => "other",
        })
        .collect();

    assert_eq!(
        kinds,
        vec![
            "wake_up",
            "search_source",
            "searched",
            "tool_retrieval",
            "eval",
            "lint_source",
            "evaluated",
        ]
    );
}
