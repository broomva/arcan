use crate::models::{AppState, UiBlock};
use crate::network::{NetworkClient, NetworkConfig};
use crate::ui;
use arcan_core::protocol::AgentEvent;
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Terminal, backend::Backend};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, PartialEq, Eq)]
enum ModelCommand {
    ShowCurrent,
    Set {
        provider: String,
        model: Option<String>,
    },
}

fn parse_model_command(message: &str) -> Result<Option<ModelCommand>, String> {
    let trimmed = message.trim();
    if !trimmed.starts_with("/model") {
        return Ok(None);
    }

    let remainder = trimmed.trim_start_matches("/model").trim();
    if remainder.is_empty() {
        return Ok(Some(ModelCommand::ShowCurrent));
    }

    if remainder.contains(char::is_whitespace) {
        return Err("Usage: /model | /model <provider> | /model <provider>:<model>".to_string());
    }

    if let Some((provider, model)) = remainder.split_once(':') {
        if provider.is_empty() || model.is_empty() {
            return Err("Usage: /model <provider>:<model> (both values are required)".to_string());
        }
        return Ok(Some(ModelCommand::Set {
            provider: provider.to_string(),
            model: Some(model.to_string()),
        }));
    }

    Ok(Some(ModelCommand::Set {
        provider: remainder.to_string(),
        model: None,
    }))
}

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

    fn push_system_alert(&mut self, text: impl Into<String>) {
        self.state.blocks.push(UiBlock::SystemAlert {
            text: text.into(),
            timestamp: Utc::now(),
        });
    }

    async fn handle_model_command(&mut self, message: &str) -> bool {
        let parsed = match parse_model_command(message) {
            Ok(command) => command,
            Err(usage) => {
                self.push_system_alert(usage);
                return true;
            }
        };

        let Some(command) = parsed else {
            return false;
        };

        match command {
            ModelCommand::ShowCurrent => match self.client.get_model().await {
                Ok(model) => self.push_system_alert(format!("Current model: {model}")),
                Err(err) => self.push_system_alert(format!("Failed to fetch model: {err}")),
            },
            ModelCommand::Set { provider, model } => {
                match self.client.set_model(&provider, model.as_deref()).await {
                    Ok(active_model) => {
                        self.push_system_alert(format!("Switched model: {active_model}"))
                    }
                    Err(err) => self.push_system_alert(format!("Failed to switch model: {err}")),
                }
            }
        }

        true
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
                            KeyCode::Char('c')
                                if key
                                    .modifiers
                                    .contains(crossterm::event::KeyModifiers::CONTROL) =>
                            {
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
                                        self.state.streaming_text = None;
                                        self.state.is_busy = false;
                                    } else if self.handle_model_command(&msg).await {
                                    } else if msg.starts_with("/approve") {
                                        let parts: Vec<&str> = msg.split_whitespace().collect();
                                        if parts.len() >= 3 {
                                            let approval_id = parts[1].to_string();
                                            let decision = match parts[2]
                                                .to_ascii_lowercase()
                                                .as_str()
                                            {
                                                "yes" | "y" | "approved" | "approve" => {
                                                    "approved".to_string()
                                                }
                                                "no" | "n" | "denied" | "deny" => {
                                                    "denied".to_string()
                                                }
                                                invalid => {
                                                    tracing::warn!(
                                                        "Invalid approval decision '{}'. Use yes/no.",
                                                        invalid
                                                    );
                                                    self.state.input_buffer.clear();
                                                    continue;
                                                }
                                            };
                                            let reason = if parts.len() > 3 {
                                                Some(parts[3..].join(" "))
                                            } else {
                                                None
                                            };

                                            let submit_client = self.client.clone();
                                            tokio::spawn(async move {
                                                if let Err(e) = submit_client
                                                    .submit_approval(
                                                        &approval_id,
                                                        &decision,
                                                        reason.as_deref(),
                                                    )
                                                    .await
                                                {
                                                    tracing::error!("Submit approval error: {}", e);
                                                }
                                            });
                                        } else {
                                            tracing::warn!(
                                                "Invalid /approve syntax. Use: /approve <id> <yes|no> [reason]"
                                            );
                                        }
                                    } else {
                                        self.state.is_busy = true;
                                        self.state.blocks.push(UiBlock::HumanMessage {
                                            text: msg.clone(),
                                            timestamp: Utc::now(),
                                        });
                                        let submit_client = self.client.clone();
                                        tokio::spawn(async move {
                                            if let Err(e) =
                                                submit_client.submit_run(&msg, None).await
                                            {
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

#[cfg(test)]
mod tests {
    use super::{ModelCommand, parse_model_command};

    #[test]
    fn parse_non_model_command() {
        assert_eq!(parse_model_command("hello").unwrap(), None);
    }

    #[test]
    fn parse_show_current_model() {
        assert_eq!(
            parse_model_command("/model").unwrap(),
            Some(ModelCommand::ShowCurrent)
        );
    }

    #[test]
    fn parse_set_provider_only() {
        assert_eq!(
            parse_model_command("/model mock").unwrap(),
            Some(ModelCommand::Set {
                provider: "mock".to_string(),
                model: None
            })
        );
    }

    #[test]
    fn parse_set_provider_with_model() {
        assert_eq!(
            parse_model_command("/model ollama:qwen2.5").unwrap(),
            Some(ModelCommand::Set {
                provider: "ollama".to_string(),
                model: Some("qwen2.5".to_string())
            })
        );
    }

    #[test]
    fn parse_rejects_incomplete_provider_model() {
        let err = parse_model_command("/model ollama:").unwrap_err();
        assert!(err.contains("required"), "got: {err}");
    }

    #[test]
    fn parse_rejects_extra_spaces() {
        let err = parse_model_command("/model ollama qwen2.5").unwrap_err();
        assert!(err.contains("Usage"), "got: {err}");
    }
}
