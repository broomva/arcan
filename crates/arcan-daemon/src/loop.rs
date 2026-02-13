use anyhow::{Context, Result};
use arcan_core::protocol::{AgentEvent, AppState, ChatMessage, RunStopReason};
use arcan_core::runtime::{Orchestrator, RunInput, RunOutput};
use arcan_store::session::{AppendEvent, SessionRepository};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

pub struct AgentLoop {
    pub session_repo: Arc<dyn SessionRepository>,
    pub orchestrator: Arc<Orchestrator>,
}

impl AgentLoop {
    pub fn new(
        session_repo: Arc<dyn SessionRepository>,
        orchestrator: Arc<Orchestrator>,
    ) -> Self {
        Self {
            session_repo,
            orchestrator,
        }
    }

    pub async fn run(
        &self,
        session_id: &str,
        user_message: String,
        event_sender: mpsc::Sender<AgentEvent>,
    ) -> Result<RunOutput> {
        // 1. Load Session History & Reconstruct State
        let history = self.session_repo.load_session(session_id)?;
        let mut state = AppState::default();
        let mut messages = Vec::new();

        // Replay history to build state and context
        // This is a simplified replay. In a real system, we might snapshot state.
        for record in history {
            match record.event {
                AgentEvent::StatePatched { patch, .. } => {
                    // Apply patch to state
                    // We ignore errors here for now as we trust the admitted history
                    let _ = state.apply_patch(&patch); 
                }
                AgentEvent::RunStarted { .. } => {
                    // Start of a run
                }
                AgentEvent::TextDelta { delta, .. } => {
                     // In a real system we'd aggregate these into a turn.
                     // For now, let's assume we can reconstruct messages from bigger chunks if we had them.
                     // But actually arcan-core `RunInput` takes `Vec<ChatMessage>`.
                     // We need to reconstruct the conversation history.
                     // This simple replay is tricky if we only have fine-grained events.
                     // A better approach for the context window is to store "Snapshots" or "Turns" in the DB,
                     // OR to have a way to aggregate events into messages.
                     
                     // For MVP, let's just append the user message to an empty context
                     // and assume the model can handle it, OR implement proper reconstruction later.
                     // A proper reconstruction would involve tracking the current "Turn" and appending to it.
                }
                AgentEvent::ToolCallCompleted { result, .. } => {
                     // messages.push(ChatMessage::Tool(result...));
                }
                // ... handle other events
                _ => {}
            }
        }
        
        // TODO: proper message history reconstruction. 
        // For now, we just pass the user message.
        messages.push(ChatMessage::user(user_message));

        let run_id = Uuid::new_v4().to_string();
        
        // 2. Prepare Run Input
        let input = RunInput {
            run_id: run_id.clone(),
            session_id: session_id.to_string(),
            messages,
            state: state.clone(),
        };

        // 3. Run Orchestrator (blocking, so we use spawn_blocking if we were async, but here we are in async fn)
        // Since Orchestrator::run is blocking, we should wrap it.
        let orchestrator = self.orchestrator.clone();
        let session_repo = self.session_repo.clone();
        let run_id_clone = run_id.clone();
        let session_id_clone = session_id.to_string();

        let output = tokio::task::spawn_blocking(move || {
            orchestrator.run(input, |event| {
                // 1. Send to channel (ignore error if receiver dropped)
                let _ = event_sender.blocking_send(event.clone());

                // 2. Persist to DB
                // We need to determine parent_id. creating a chain.
                // For simplicity, we can just append. `JsonlSessionRepository` handles linear log.
                // But `AppendEvent` requires `parent_id`.
                // We need to track the last event ID.
                // In this scope, we don't have easy access to the "last event ID" from the outer scope 
                // unless we pass it in or track it here.
                // Let's assume `session_repo.append` handles it or we pass None for now/root.
                
                let _ = session_repo.append(AppendEvent {
                    session_id: session_id_clone.clone(),
                    event,
                    parent_id: None, // TODO: track parent_id for DAG
                });
            })
        }).await??;

        Ok(output)
    }
}
