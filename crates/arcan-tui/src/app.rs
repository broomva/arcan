use crate::event::{TuiEvent, event_pump};
use crate::models::state::AppState;
use crate::models::ui_block::UiBlock;
use crate::network::{NetworkClient, NetworkConfig};
use crate::ui;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
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
    events: mpsc::Receiver<TuiEvent>,
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

        // Merge terminal + network + ticks into a single event stream
        let events = event_pump(rx, Duration::from_millis(50));

        Self {
            state: AppState::new(),
            should_quit: false,
            client,
            events,
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

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        // Initial draw
        terminal.draw(|f| ui::draw(f, &mut self.state))?;

        while let Some(event) = self.events.recv().await {
            match event {
                TuiEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    self.handle_key(key.code, key.modifiers).await;
                }
                TuiEvent::Network(agent_event) => {
                    self.state.apply_event(agent_event);
                }
                TuiEvent::Tick => {
                    // Clear expired error flashes (5 second TTL)
                    self.state
                        .clear_expired_errors(chrono::Duration::seconds(5));
                }
                TuiEvent::Resize(_, _) => {
                    // Will redraw below
                }
                _ => {}
            }

            // Redraw after every event
            terminal.draw(|f| ui::draw(f, &mut self.state))?;

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Focus-independent keys
        match code {
            KeyCode::Esc => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Tab => {
                self.state.focus = self.state.focus.next();
                return;
            }
            _ => {}
        }

        // Focus-dependent key handling
        match self.state.focus {
            crate::focus::FocusTarget::ChatLog => self.handle_scroll_key(code),
            crate::focus::FocusTarget::InputBar => self.handle_input_key(code).await,
        }
    }

    fn handle_scroll_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.state.scroll.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.state.scroll.scroll_down(1),
            KeyCode::PageUp => self.state.scroll.page_up(),
            KeyCode::PageDown => self.state.scroll.page_down(),
            KeyCode::Home | KeyCode::Char('g') => {
                let max = self.state.scroll.total_lines;
                self.state.scroll.scroll_up(max);
            }
            KeyCode::End | KeyCode::Char('G') => self.state.scroll.scroll_to_bottom(),
            _ => {}
        }
    }

    async fn handle_input_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(c) => {
                self.state.input_buffer.push(c);
            }
            KeyCode::Backspace => {
                self.state.input_buffer.pop();
            }
            KeyCode::Enter => {
                self.handle_submit().await;
            }
            // Allow PageUp/PageDown even in input mode for convenience
            KeyCode::PageUp => self.state.scroll.page_up(),
            KeyCode::PageDown => self.state.scroll.scroll_to_bottom(),
            _ => {}
        }
    }

    async fn handle_submit(&mut self) {
        let msg = self.state.input_buffer.trim().to_string();
        if msg.is_empty() {
            return;
        }

        if msg == "/clear" {
            self.state.blocks.clear();
            self.state.streaming_text = None;
            self.state.is_busy = false;
        } else if msg == "/help" {
            self.push_system_alert(
                "Commands: /clear, /model [provider[:model]], /approve <id> <yes|no> [reason], /help",
            );
        } else if self.handle_model_command(&msg).await {
            // handled
        } else if msg.starts_with("/approve") {
            self.handle_approve_command(&msg);
        } else {
            // Normal message — submit run
            self.state.is_busy = true;
            self.state.blocks.push(UiBlock::HumanMessage {
                text: msg.clone(),
                timestamp: Utc::now(),
            });

            // Auto-follow on new message
            self.state.scroll.scroll_to_bottom();

            let submit_client = self.client.clone();
            tokio::spawn(async move {
                if let Err(e) = submit_client.submit_run(&msg, None).await {
                    tracing::error!("Submit error: {}", e);
                }
            });
        }
        self.state.input_buffer.clear();
    }

    fn handle_approve_command(&mut self, msg: &str) {
        let parts: Vec<&str> = msg.split_whitespace().collect();
        if parts.len() >= 3 {
            let approval_id = parts[1].to_string();
            let decision = match parts[2].to_ascii_lowercase().as_str() {
                "yes" | "y" | "approved" | "approve" => "approved".to_string(),
                "no" | "n" | "denied" | "deny" => "denied".to_string(),
                invalid => {
                    self.push_system_alert(format!(
                        "Invalid approval decision '{}'. Use yes/no.",
                        invalid
                    ));
                    return;
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
                    .submit_approval(&approval_id, &decision, reason.as_deref())
                    .await
                {
                    tracing::error!("Submit approval error: {}", e);
                }
            });
        } else {
            self.push_system_alert("Usage: /approve <id> <yes|no> [reason]");
        }
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
