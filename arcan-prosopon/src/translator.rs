//! Pure translation layer: `aios_protocol::EventKind` → `Vec<ProsoponEvent>`.

use aios_protocol::EventKind;
use prosopon_core::ProsoponEvent;

use crate::state::TranslationState;

/// Translate a single `EventKind` into zero or more `ProsoponEvent`s.
///
/// Total over every currently-known variant and includes a `_` wildcard
/// for `#[non_exhaustive]` forward compatibility.
pub fn translate(state: &mut TranslationState, kind: &EventKind) -> Vec<ProsoponEvent> {
    use prosopon_core::{ChildrenPatch, Intent, Node, NodePatch, Scene, SignalValue, Topic};

    match kind {
        EventKind::SessionCreated { name, .. } => {
            // A new session resets per-session bookkeeping so a resumed translator
            // state doesn't carry prior streams into the new scene.
            state.streams_by_iteration.clear();
            state.current_iteration = None;

            let root = Node::new(Intent::Section {
                title: Some(name.clone()),
                collapsible: false,
            })
            .with_id(state.scene_root.clone());
            vec![ProsoponEvent::SceneReset {
                scene: Scene::new(root),
            }]
        }

        EventKind::RunStarted {
            provider,
            max_iterations,
        } => {
            let ts = chrono::Utc::now();
            vec![
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.status"),
                    value: SignalValue::Scalar(serde_json::json!("running")),
                    ts,
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.provider"),
                    value: SignalValue::Scalar(serde_json::json!(provider)),
                    ts,
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.max_iterations"),
                    value: SignalValue::Scalar(serde_json::json!(max_iterations)),
                    ts,
                },
            ]
        }

        EventKind::RunErrored { error } => {
            let node = Node::new(Intent::Prose {
                text: error.clone(),
            })
            .attr("semantic_role", serde_json::json!("error"));
            vec![
                ProsoponEvent::NodeAdded {
                    parent: state.scene_root.clone(),
                    node,
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.status"),
                    value: SignalValue::Scalar(serde_json::json!("errored")),
                    ts: chrono::Utc::now(),
                },
            ]
        }

        EventKind::UserMessage { content } => {
            let prose = Node::new(Intent::Prose {
                text: content.clone(),
            });
            let section = Node::new(Intent::Section {
                title: Some("User".into()),
                collapsible: false,
            })
            .child(prose);
            vec![ProsoponEvent::NodeAdded {
                parent: state.scene_root.clone(),
                node: section,
            }]
        }

        EventKind::AssistantTextDelta { delta, index } | EventKind::TextDelta { delta, index } => {
            use prosopon_core::{ChunkPayload, StreamChunk, StreamId, StreamKind};
            let iteration = index.or(state.current_iteration).unwrap_or(0);
            let mut events = Vec::with_capacity(2);

            let stream_id = match state.streams_by_iteration.get(&iteration) {
                Some(id) => id.clone(),
                None => {
                    let id = StreamId::from_raw(format!("stream:iter-{iteration}"));
                    let stream_node = Node::new(Intent::Stream {
                        id: id.clone(),
                        kind: StreamKind::Text,
                    });
                    events.push(ProsoponEvent::NodeAdded {
                        parent: state.scene_root.clone(),
                        node: stream_node,
                    });
                    state.streams_by_iteration.insert(iteration, id.clone());
                    id
                }
            };

            let seq = state.stream_seq.entry(stream_id.clone()).or_insert(0);
            let this_seq = *seq;
            *seq += 1;

            events.push(ProsoponEvent::StreamChunk {
                id: stream_id,
                chunk: StreamChunk {
                    seq: this_seq,
                    payload: ChunkPayload::Text {
                        text: delta.clone(),
                    },
                    final_: false,
                },
            });
            events
        }

        EventKind::AssistantMessageCommitted { content, .. }
        | EventKind::Message { content, .. } => {
            let section = Node::new(Intent::Section {
                title: Some("Assistant".into()),
                collapsible: false,
            })
            .child(Node::new(Intent::Prose {
                text: content.clone(),
            }));
            vec![ProsoponEvent::NodeAdded {
                parent: state.scene_root.clone(),
                node: section,
            }]
        }

        EventKind::ToolCallRequested {
            call_id,
            tool_name,
            arguments,
            ..
        } => {
            let node = Node::new(Intent::ToolCall {
                name: tool_name.clone(),
                args: arguments.clone(),
                stream: None,
            })
            .with_id(tool_node_id(call_id));
            vec![ProsoponEvent::NodeAdded {
                parent: state.scene_root.clone(),
                node,
            }]
        }

        EventKind::ToolCallCompleted {
            call_id,
            result,
            status,
            ..
        } => {
            use aios_protocol::SpanStatus;
            let Some(c) = call_id.as_ref() else {
                return Vec::new(); // no call_id → can't match a prior ToolCall; drop
            };
            let success = matches!(status, SpanStatus::Ok);
            let result_node = Node::new(Intent::ToolResult {
                success,
                payload: result.clone(),
            });
            vec![ProsoponEvent::NodeUpdated {
                id: tool_node_id(c),
                patch: NodePatch {
                    children: Some(ChildrenPatch::Append {
                        children: vec![result_node],
                    }),
                    ..NodePatch::default()
                },
            }]
        }

        EventKind::ToolCallFailed { call_id, error, .. } => {
            let id = tool_node_id(call_id);
            let result_node = Node::new(Intent::ToolResult {
                success: false,
                payload: serde_json::json!({ "error": error }),
            });
            vec![ProsoponEvent::NodeUpdated {
                id,
                patch: NodePatch {
                    children: Some(ChildrenPatch::Append {
                        children: vec![result_node],
                    }),
                    ..NodePatch::default()
                },
            }]
        }

        EventKind::ApprovalRequested {
            approval_id,
            tool_name,
            risk,
            ..
        } => {
            let node = Node::new(Intent::Confirm {
                message: format!("Approve {tool_name}?"),
                severity: severity_for(*risk),
            })
            .with_id(approval_node_id(approval_id))
            .attr("approval_id", serde_json::json!(approval_id.to_string()));
            vec![ProsoponEvent::NodeAdded {
                parent: state.scene_root.clone(),
                node,
            }]
        }

        EventKind::ApprovalResolved {
            approval_id,
            decision,
            ..
        } => {
            use prosopon_core::{Lifecycle, NodeStatus};
            let id = approval_node_id(approval_id);
            let ts = chrono::Utc::now();
            vec![
                ProsoponEvent::NodeUpdated {
                    id,
                    patch: NodePatch {
                        lifecycle: Some(Lifecycle::now().with_status(NodeStatus::Resolved)),
                        ..NodePatch::default()
                    },
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from(format!("approval.{approval_id}").as_str()),
                    value: SignalValue::Scalar(serde_json::json!(
                        format!("{decision:?}").to_lowercase()
                    )),
                    ts,
                },
            ]
        }

        EventKind::StatePatched { revision, .. } => vec![ProsoponEvent::SignalChanged {
            topic: Topic::from("state.revision"),
            value: SignalValue::Scalar(serde_json::json!(*revision)),
            ts: chrono::Utc::now(),
        }],

        EventKind::ContextCompacted {
            tokens_before,
            tokens_after,
            ..
        } => {
            let ts = chrono::Utc::now();
            let node = Node::new(Intent::Prose {
                text: format!("Compacted {tokens_before}→{tokens_after} tokens"),
            })
            .attr("emphasis", serde_json::json!("low"));
            vec![
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("context.tokens"),
                    value: SignalValue::Scalar(serde_json::json!(*tokens_after)),
                    ts,
                },
                ProsoponEvent::NodeAdded {
                    parent: state.scene_root.clone(),
                    node,
                },
            ]
        }

        EventKind::StepStarted { index } => vec![ProsoponEvent::SignalChanged {
            topic: Topic::from("iteration"),
            value: SignalValue::Scalar(serde_json::json!(*index)),
            ts: chrono::Utc::now(),
        }],

        EventKind::PolicyEvaluated {
            tool_name,
            decision,
            ..
        } => vec![ProsoponEvent::SignalChanged {
            topic: Topic::from(format!("policy.{tool_name}").as_str()),
            value: SignalValue::Scalar(serde_json::json!(format!("{decision:?}").to_lowercase())),
            ts: chrono::Utc::now(),
        }],

        EventKind::KnowledgeSearched {
            query,
            result_count,
            ..
        } => {
            let node = Node::new(Intent::Prose {
                text: format!("Searched: {query} ({result_count})"),
            })
            .attr("emphasis", serde_json::json!("low"));
            vec![ProsoponEvent::NodeAdded {
                parent: state.scene_root.clone(),
                node,
            }]
        }

        _ => Vec::new(),
    }
}

fn tool_node_id(call_id: &str) -> prosopon_core::NodeId {
    prosopon_core::NodeId::from_raw(format!("tool:{call_id}"))
}

fn severity_for(risk: aios_protocol::RiskLevel) -> prosopon_core::Severity {
    use aios_protocol::RiskLevel;
    use prosopon_core::Severity;
    match risk {
        RiskLevel::Low => Severity::Info,
        RiskLevel::Medium => Severity::Notice,
        RiskLevel::High => Severity::Warning,
        RiskLevel::Critical => Severity::Danger,
    }
}

fn approval_node_id(id: &aios_protocol::ApprovalId) -> prosopon_core::NodeId {
    prosopon_core::NodeId::from_raw(format!("approval:{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::EventKind;
    use prosopon_core::ProsoponEvent;

    fn st() -> TranslationState {
        TranslationState::new()
    }

    #[test]
    fn session_created_emits_scene_reset() {
        let kind = EventKind::SessionCreated {
            name: "sess-a".into(),
            config: serde_json::json!({}),
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ProsoponEvent::SceneReset { .. }));
    }

    #[test]
    fn run_started_emits_three_signal_changes() {
        let kind = EventKind::RunStarted {
            provider: "anthropic".into(),
            max_iterations: 8,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 3);
        for e in &events {
            assert!(matches!(e, ProsoponEvent::SignalChanged { .. }));
        }
    }

    #[test]
    fn run_errored_emits_error_prose_and_status_signal() {
        let kind = EventKind::RunErrored {
            error: "boom".into(),
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], ProsoponEvent::NodeAdded { .. }));
        assert!(matches!(events[1], ProsoponEvent::SignalChanged { .. }));
    }

    #[test]
    fn unknown_variant_is_empty() {
        let kind = EventKind::SessionClosed {
            reason: "idle".into(),
        };
        assert!(translate(&mut st(), &kind).is_empty());
    }

    #[test]
    fn translated_events_apply_cleanly_to_scene_store() {
        use prosopon_runtime::SceneStore;

        let mut state = TranslationState::new();

        // Session first — establishes the scene root.
        let reset = translate(
            &mut state,
            &EventKind::SessionCreated {
                name: "test".into(),
                config: serde_json::json!({}),
            },
        );
        assert_eq!(reset.len(), 1);

        // Build the scene from the SceneReset.
        let ProsoponEvent::SceneReset { scene } = reset.into_iter().next().unwrap() else {
            panic!("expected SceneReset");
        };
        let mut store = SceneStore::new(scene);

        // Now a RunErrored — its NodeAdded must target the scene root.
        let errs = translate(&mut state, &EventKind::RunErrored { error: "x".into() });
        for ev in errs {
            store.apply(ev).expect("event must apply cleanly");
        }

        let user = translate(
            &mut state,
            &EventKind::UserMessage {
                content: "q".into(),
            },
        );
        for ev in user {
            store.apply(ev).expect("user message should apply");
        }

        let delta = translate(
            &mut state,
            &EventKind::TextDelta {
                delta: "a".into(),
                index: Some(0),
            },
        );
        for ev in delta {
            store.apply(ev).expect("text delta should apply");
        }

        use aios_protocol::{ApprovalDecision, ApprovalId, RiskLevel};

        let approval_id = ApprovalId::default();
        let req = translate(
            &mut state,
            &EventKind::ApprovalRequested {
                approval_id: approval_id.clone(),
                call_id: "a".into(),
                tool_name: "shell".into(),
                arguments: serde_json::json!({}),
                risk: RiskLevel::Medium,
            },
        );
        for ev in req {
            store.apply(ev).expect("approval request applies");
        }

        let res = translate(
            &mut state,
            &EventKind::ApprovalResolved {
                approval_id,
                decision: ApprovalDecision::Approved,
                reason: None,
            },
        );
        for ev in res {
            store.apply(ev).expect("approval resolved applies");
        }
    }

    #[test]
    fn user_message_adds_section_with_prose() {
        use prosopon_core::Intent;
        let kind = EventKind::UserMessage {
            content: "hi".into(),
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeAdded { node, .. } => {
                assert!(matches!(node.intent, Intent::Section { .. }));
                assert_eq!(node.children.len(), 1);
                assert!(matches!(node.children[0].intent, Intent::Prose { .. }));
            }
            _ => panic!("expected NodeAdded"),
        }
    }

    #[test]
    fn first_text_delta_creates_stream_node_then_chunks() {
        let mut s = st();
        s.current_iteration = Some(3);
        let first = EventKind::TextDelta {
            delta: "he".into(),
            index: Some(3),
        };
        let second = EventKind::TextDelta {
            delta: "llo".into(),
            index: Some(3),
        };

        let a = translate(&mut s, &first);
        let b = translate(&mut s, &second);

        // First delta: NodeAdded (stream) + StreamChunk.
        assert_eq!(a.len(), 2);
        assert!(matches!(a[0], ProsoponEvent::NodeAdded { .. }));
        assert!(matches!(a[1], ProsoponEvent::StreamChunk { .. }));
        // Second delta: StreamChunk only.
        assert_eq!(b.len(), 1);
        assert!(matches!(b[0], ProsoponEvent::StreamChunk { .. }));
    }

    #[test]
    fn assistant_text_delta_uses_same_stream_as_text_delta() {
        let mut s = st();
        let a = translate(
            &mut s,
            &EventKind::AssistantTextDelta {
                delta: "x".into(),
                index: Some(1),
            },
        );
        let b = translate(
            &mut s,
            &EventKind::TextDelta {
                delta: "y".into(),
                index: Some(1),
            },
        );
        // Same iteration (1) → a creates the stream, b only adds a chunk.
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn assistant_message_committed_adds_assistant_section() {
        let kind = EventKind::AssistantMessageCommitted {
            role: "assistant".into(),
            content: "answer".into(),
            model: None,
            token_usage: None,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ProsoponEvent::NodeAdded { .. }));
    }

    #[test]
    fn message_variant_also_adds_assistant_section() {
        let kind = EventKind::Message {
            role: "assistant".into(),
            content: "answer".into(),
            model: None,
            token_usage: None,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ProsoponEvent::NodeAdded { .. }));
    }

    #[test]
    fn tool_call_requested_adds_tool_call_node() {
        use prosopon_core::Intent;
        let kind = EventKind::ToolCallRequested {
            call_id: "call-1".into(),
            tool_name: "shell".into(),
            arguments: serde_json::json!({"cmd": "ls"}),
            category: None,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeAdded { node, parent } => {
                assert_eq!(parent, &st().scene_root);
                assert!(matches!(node.intent, Intent::ToolCall { .. }));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tool_call_completed_updates_node_with_child_result() {
        use aios_protocol::SpanStatus;
        use prosopon_core::Intent;
        let kind = EventKind::ToolCallCompleted {
            tool_run_id: aios_protocol::ToolRunId::default(),
            call_id: Some("call-1".into()),
            tool_name: "shell".into(),
            result: serde_json::json!("ok"),
            duration_ms: 12,
            status: SpanStatus::Ok,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeUpdated { patch, .. } => {
                let cp = patch.children.as_ref().expect("children patch set");
                match cp {
                    prosopon_core::ChildrenPatch::Append { children } => {
                        assert_eq!(children.len(), 1);
                        match &children[0].intent {
                            Intent::ToolResult { success, .. } => assert!(*success),
                            _ => panic!("expected ToolResult"),
                        }
                    }
                    _ => panic!("expected Append"),
                }
            }
            _ => panic!("expected NodeUpdated"),
        }
    }

    #[test]
    fn tool_call_failed_updates_with_unsuccess_result() {
        use prosopon_core::Intent;
        let kind = EventKind::ToolCallFailed {
            call_id: "call-2".into(),
            tool_name: "shell".into(),
            error: "denied".into(),
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeUpdated { patch, .. } => {
                let cp = patch.children.as_ref().unwrap();
                if let prosopon_core::ChildrenPatch::Append { children } = cp {
                    if let Intent::ToolResult { success, .. } = &children[0].intent {
                        assert!(!success);
                    } else {
                        panic!()
                    }
                } else {
                    panic!()
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn tool_call_completed_with_no_call_id_emits_nothing() {
        use aios_protocol::SpanStatus;
        let kind = EventKind::ToolCallCompleted {
            tool_run_id: aios_protocol::ToolRunId::default(),
            call_id: None,
            tool_name: "shell".into(),
            result: serde_json::json!("orphan"),
            duration_ms: 1,
            status: SpanStatus::Ok,
        };
        assert!(translate(&mut st(), &kind).is_empty());
    }

    #[test]
    fn approval_requested_adds_confirm_node() {
        use aios_protocol::{ApprovalId, RiskLevel};
        use prosopon_core::{Intent, Severity};
        let kind = EventKind::ApprovalRequested {
            approval_id: ApprovalId::default(),
            call_id: "c".into(),
            tool_name: "shell".into(),
            arguments: serde_json::json!({}),
            risk: RiskLevel::High,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeAdded { node, parent } => {
                assert_eq!(parent, &st().scene_root);
                assert!(
                    matches!(&node.intent, Intent::Confirm { severity, .. } if *severity == Severity::Warning)
                );
            }
            _ => panic!("expected NodeAdded"),
        }
    }

    #[test]
    fn approval_resolved_updates_lifecycle_and_emits_decision_signal() {
        use aios_protocol::{ApprovalDecision, ApprovalId};
        use prosopon_core::NodeStatus;
        let id = ApprovalId::default();
        let kind = EventKind::ApprovalResolved {
            approval_id: id.clone(),
            decision: ApprovalDecision::Approved,
            reason: None,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 2);
        match &events[0] {
            ProsoponEvent::NodeUpdated { patch, .. } => {
                let lc = patch.lifecycle.as_ref().expect("lifecycle set");
                assert!(matches!(lc.status, NodeStatus::Resolved));
            }
            _ => panic!("expected NodeUpdated"),
        }
        assert!(matches!(events[1], ProsoponEvent::SignalChanged { .. }));
    }

    #[test]
    fn state_patched_emits_revision_signal() {
        let kind = EventKind::StatePatched {
            index: None,
            patch: serde_json::json!([]),
            revision: 42,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ProsoponEvent::SignalChanged { .. }));
    }

    #[test]
    fn context_compacted_emits_signal_and_prose() {
        let kind = EventKind::ContextCompacted {
            dropped_count: 3,
            tokens_before: 1000,
            tokens_after: 500,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], ProsoponEvent::SignalChanged { .. }));
        match &events[1] {
            ProsoponEvent::NodeAdded { parent, .. } => {
                assert_eq!(parent, &st().scene_root);
            }
            _ => panic!("expected NodeAdded"),
        }
    }

    #[test]
    fn step_started_emits_iteration_signal() {
        let kind = EventKind::StepStarted { index: 3 };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], ProsoponEvent::SignalChanged { .. }));
    }

    #[test]
    fn knowledge_searched_emits_prose() {
        use prosopon_core::Intent;
        let kind = EventKind::KnowledgeSearched {
            query: "foo".into(),
            result_count: 2,
            top_relevance: 0.9,
            duration_ms: 5,
        };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 1);
        match &events[0] {
            ProsoponEvent::NodeAdded { parent, node } => {
                assert_eq!(parent, &st().scene_root);
                assert!(matches!(node.intent, Intent::Prose { .. }));
            }
            _ => panic!("expected NodeAdded"),
        }
    }

    #[test]
    fn tool_call_round_trip_applies_to_scene_store() {
        use aios_protocol::SpanStatus;
        use prosopon_runtime::SceneStore;
        let mut state = TranslationState::new();

        let reset = translate(
            &mut state,
            &EventKind::SessionCreated {
                name: "s".into(),
                config: serde_json::json!({}),
            },
        );
        let ProsoponEvent::SceneReset { scene } = reset.into_iter().next().unwrap() else {
            panic!()
        };
        let mut store = SceneStore::new(scene);

        let req = translate(
            &mut state,
            &EventKind::ToolCallRequested {
                call_id: "call-1".into(),
                tool_name: "shell".into(),
                arguments: serde_json::json!({}),
                category: None,
            },
        );
        for ev in req {
            store.apply(ev).expect("tool request applies");
        }

        let done = translate(
            &mut state,
            &EventKind::ToolCallCompleted {
                tool_run_id: aios_protocol::ToolRunId::default(),
                call_id: Some("call-1".into()),
                tool_name: "shell".into(),
                result: serde_json::json!("ok"),
                duration_ms: 1,
                status: SpanStatus::Ok,
            },
        );
        for ev in done {
            store.apply(ev).expect("tool completion applies");
        }
    }
}
