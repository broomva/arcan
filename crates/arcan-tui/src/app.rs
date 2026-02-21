use crate::models::AppState;
use crate::network::{NetworkClient, NetworkConfig};
use crate::ui;
use arcan_core::protocol::AgentEvent;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{backend::Backend, Terminal};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub client: Arc<NetworkClient>,
    pub event_rx: mpsc::Receiver<AgentEvent>,
}

impl App {
    pub fn new(config: NetworkConfig) -> Self {
        let client = Arc::new(NetworkClient::new(config));
        let (tx, rx) = mpsc::channel(100);

        // Spawn network listener
        let listener_client = client.clone();
        tokio::spawn(async move {
            if let Err(e) = listener_client.listen_events(tx).await {
                tracing::error!("Event listener ended: {}", e);
            }
        });

        Self {
            state: AppState::new(),
            should_quit: false,
            client,
            event_rx: rx,
        }
    }

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()> {
        loop {
            // Draw UI
            terminal.draw(|f| ui::draw(f, &self.state))?;

            // Process external AgentEvents non-blocking
            while let Ok(event) = self.event_rx.try_recv() {
                self.state.apply_event(event);
            }

            // Handle UI events non-blocking
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Esc => self.should_quit = true,
                            KeyCode::Char('c') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                self.should_quit = true;
                            }
                            KeyCode::Char(c) => {
                                self.state.input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                self.state.input_buffer.pop();
                            }
                            KeyCode::Enter => {
                                let msg = self.state.input_buffer.trim().to_string();
                                if !msg.is_empty() {
                                    if msg == "/clear" {
                                        self.state.blocks.clear();
                                    } else if msg.starts_with("/approve") {
                                        let parts: Vec<&str> = msg.split_whitespace().collect();
                                        if parts.len() >= 3 {
                                            let approval_id = parts[1].to_string();
                                            let decision = parts[2].to_string();
                                            let reason = if parts.len() > 3 { Some(parts[3..].join(" ")) } else { None };
                                            
                                            let submit_client = self.client.clone();
                                            tokio::spawn(async move {
                                                if let Err(e) = submit_client.submit_approval(&approval_id, &decision, reason.as_deref()).await {
                                                    tracing::error!("Submit approval error: {}", e);
                                                }
                                            });
                                        } else {
                                            tracing::warn!("Invalid /approve syntax. Use: /approve <id> <yes|no> [reason]");
                                        }
                                    } else {
                                        let submit_client = self.client.clone();
                                        tokio::spawn(async move {
                                            if let Err(e) = submit_client.submit_run(&msg, None).await {
                                                tracing::error!("Submit error: {}", e);
                                            }
                                        });
                                    }
                                    self.state.input_buffer.clear();
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            if self.should_quit {
                break;
            }
        }
        Ok(())
    }
}
