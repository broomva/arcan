//! Pure translation layer: `aios_protocol::EventKind` → `Vec<ProsoponEvent>`.

use aios_protocol::EventKind;
use prosopon_core::ProsoponEvent;

use crate::state::TranslationState;

/// Translate a single `EventKind` into zero or more `ProsoponEvent`s.
///
/// Total over every currently-known variant and includes a `_` wildcard
/// for `#[non_exhaustive]` forward compatibility.
pub fn translate(state: &mut TranslationState, kind: &EventKind) -> Vec<ProsoponEvent> {
    use prosopon_core::{Intent, Node, NodeId, Scene, SignalValue, Topic};

    let _ = state; // used by future arms

    match kind {
        EventKind::SessionCreated { name, .. } => {
            let root = Node::new(Intent::Section {
                title: Some(name.clone()),
                collapsible: false,
            });
            vec![ProsoponEvent::SceneReset {
                scene: Scene::new(root),
            }]
        }

        EventKind::RunStarted {
            provider,
            max_iterations,
        } => {
            vec![
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.status"),
                    value: SignalValue::Scalar(serde_json::json!("running")),
                    ts: chrono::Utc::now(),
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.provider"),
                    value: SignalValue::Scalar(serde_json::json!(provider)),
                    ts: chrono::Utc::now(),
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.max_iterations"),
                    value: SignalValue::Scalar(serde_json::json!(max_iterations)),
                    ts: chrono::Utc::now(),
                },
            ]
        }

        EventKind::RunErrored { error } => {
            let node = Node::new(Intent::Prose { text: error.clone() }).attr(
                "semantic_role",
                serde_json::json!("error"),
            );
            vec![
                ProsoponEvent::NodeAdded {
                    parent: NodeId::new(),
                    node,
                },
                ProsoponEvent::SignalChanged {
                    topic: Topic::from("run.status"),
                    value: SignalValue::Scalar(serde_json::json!("errored")),
                    ts: chrono::Utc::now(),
                },
            ]
        }

        _ => Vec::new(),
    }
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
        let kind = EventKind::RunErrored { error: "boom".into() };
        let events = translate(&mut st(), &kind);
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], ProsoponEvent::NodeAdded { .. }));
        assert!(matches!(events[1], ProsoponEvent::SignalChanged { .. }));
    }

    #[test]
    fn unknown_variant_is_empty() {
        let kind = EventKind::SessionClosed { reason: "idle".into() };
        assert!(translate(&mut st(), &kind).is_empty());
    }
}
