use anyhow::Result;
use arcan_core::protocol::{AgentEvent, ChatMessage};
use arcan_core::runtime::{ApprovalGateHook, Orchestrator, RunInput, RunOutput};
use arcan_core::state::AppState;
use arcan_store::session::{AppendEvent, SessionRepository};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct AgentLoop {
    pub session_repo: Arc<dyn SessionRepository>,
    pub orchestrator: Arc<Orchestrator>,
    pub approval_gate: Option<Arc<dyn ApprovalGateHook>>,
}

impl AgentLoop {
    pub fn new(session_repo: Arc<dyn SessionRepository>, orchestrator: Arc<Orchestrator>) -> Self {
        Self {
            session_repo,
            orchestrator,
            approval_gate: None,
        }
    }

    pub fn with_approval_gate(
        session_repo: Arc<dyn SessionRepository>,
        orchestrator: Arc<Orchestrator>,
        gate: Arc<dyn ApprovalGateHook>,
    ) -> Self {
        Self {
            session_repo,
            orchestrator,
            approval_gate: Some(gate),
        }
    }

    pub async fn run(
        &self,
        session_id: &str,
        user_message: String,
        event_sender: mpsc::Sender<AgentEvent>,
    ) -> Result<RunOutput> {
        let orchestrator = self.orchestrator.clone();
        let session_repo = self.session_repo.clone();
        let session_id_owned = session_id.to_string();
        let approval_gate = self.approval_gate.clone();

        // Run everything in spawn_blocking since SessionRepository is synchronous
        // and may use block_on internally (e.g. LagoSessionRepository).
        let output = tokio::task::spawn_blocking(move || -> Result<RunOutput> {
            // 1. Load Session History & Reconstruct State
            let history = session_repo.load_session(&session_id_owned)?;
            let mut state = AppState::default();
            let mut messages: Vec<ChatMessage> = Vec::new();

            // Replay history to build state and context
            for record in history {
                match record.event {
                    AgentEvent::StatePatched { patch, .. } => {
                        let _ = state.apply_patch(&patch);
                    }
                    AgentEvent::TextDelta { delta, .. } => {
                        // Aggregate deltas into the last assistant message
                        if let Some(last) = messages.last_mut() {
                            if last.role == arcan_core::protocol::Role::Assistant {
                                last.content.push_str(&delta);
                            } else {
                                messages.push(ChatMessage::assistant(delta));
                            }
                        } else {
                            messages.push(ChatMessage::assistant(delta));
                        }
                    }
                    AgentEvent::ToolCallCompleted { result, .. } => {
                        let output_str = serde_json::to_string(&result.output)
                            .unwrap_or_else(|_| "{}".to_string());
                        messages.push(ChatMessage::tool_result(&result.call_id, output_str));
                    }
                    _ => {}
                }
            }

            // Append the new user message
            messages.push(ChatMessage::user(user_message));

            let run_id = Uuid::new_v4().to_string();

            // 2. Wire approval gate event handler
            if let Some(ref gate) = approval_gate {
                let sender = event_sender.clone();
                let repo = session_repo.clone();
                let sid = session_id_owned.clone();
                gate.set_event_handler(Arc::new(move |event| {
                    let _ = sender.blocking_send(event.clone());
                    let _ = repo.append(AppendEvent {
                        session_id: sid.clone(),
                        event,
                        parent_id: None,
                    });
                }));
            }

            // 3. Prepare Run Input
            let input = RunInput {
                run_id,
                session_id: session_id_owned.clone(),
                messages,
                state,
            };

            // 4. Run Orchestrator (Provider::complete is synchronous)
            let session_repo_inner = session_repo.clone();
            let sid = session_id_owned.clone();
            let result = orchestrator.run(input, |event| {
                let _ = event_sender.blocking_send(event.clone());

                let _ = session_repo_inner.append(AppendEvent {
                    session_id: sid.clone(),
                    event,
                    parent_id: None,
                });
            });

            // 5. Clear approval gate handler
            if let Some(ref gate) = approval_gate {
                gate.clear_event_handler();
            }

            Ok(result)
        })
        .await??;

        Ok(output)
    }
}
