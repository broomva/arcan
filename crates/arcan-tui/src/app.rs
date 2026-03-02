use crate::command::{self, Command, ModelSubcommand};
use crate::event::{TuiEvent, event_pump};
use crate::models::state::AppState;
use crate::models::ui_block::UiBlock;
use crate::network::{NetworkClient, NetworkConfig};
use crate::ui;
use crate::widgets::input_bar::InputBarState;
use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::Backend};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct App {
    pub state: AppState,
    pub input_bar: InputBarState,
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
            input_bar: InputBarState::new(),
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

    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> anyhow::Result<()>
    where
        B::Error: Send + Sync + 'static,
    {
        // Initial draw
        terminal.draw(|f| ui::draw(f, &mut self.state, &self.input_bar))?;

        while let Some(event) = self.events.recv().await {
            match event {
                TuiEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    self.handle_key(key).await;
                }
                TuiEvent::Network(agent_event) => {
                    self.state.apply_event(agent_event);
                }
                TuiEvent::Tick => {
                    self.state
                        .clear_expired_errors(chrono::Duration::seconds(5));
                }
                TuiEvent::Resize(_, _) => {
                    // Will redraw below
                }
                _ => {}
            }

            terminal.draw(|f| ui::draw(f, &mut self.state, &self.input_bar))?;

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) {
        // Focus-independent keys
        match key.code {
            KeyCode::Esc => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
            crate::focus::FocusTarget::ChatLog => self.handle_scroll_key(key.code),
            crate::focus::FocusTarget::InputBar => self.handle_input_key(key).await,
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

    async fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.handle_submit().await;
            }
            KeyCode::Up => {
                self.input_bar.history_up();
            }
            KeyCode::Down => {
                self.input_bar.history_down();
            }
            KeyCode::PageUp => self.state.scroll.page_up(),
            KeyCode::PageDown => self.state.scroll.scroll_to_bottom(),
            _ => {
                // Forward all other keys to tui-textarea
                self.input_bar.input(key);
            }
        }
    }

    async fn handle_submit(&mut self) {
        let msg = self.input_bar.submit();
        let trimmed = msg.trim().to_string();
        if trimmed.is_empty() {
            return;
        }

        // Sync to input_buffer for backward compat
        self.state.input_buffer.clear();

        match command::parse(&trimmed) {
            Ok(Command::Clear) => {
                self.state.blocks.clear();
                self.state.streaming_text = None;
                self.state.is_busy = false;
            }
            Ok(Command::Help) => {
                self.push_system_alert(
                    "Commands: /clear, /model [provider[:model]], /approve <id> <yes|no> [reason], /help",
                );
            }
            Ok(Command::Model(subcmd)) => {
                self.execute_model_command(subcmd).await;
            }
            Ok(Command::Approve {
                approval_id,
                decision,
                reason,
            }) => {
                let submit_client = self.client.clone();
                tokio::spawn(async move {
                    if let Err(e) = submit_client
                        .submit_approval(&approval_id, &decision, reason.as_deref())
                        .await
                    {
                        tracing::error!("Submit approval error: {}", e);
                    }
                });
            }
            Ok(Command::SendMessage(text)) => {
                self.state.is_busy = true;
                self.state.blocks.push(UiBlock::HumanMessage {
                    text: text.clone(),
                    timestamp: Utc::now(),
                });
                self.state.scroll.scroll_to_bottom();

                let submit_client = self.client.clone();
                tokio::spawn(async move {
                    if let Err(e) = submit_client.submit_run(&text, None).await {
                        tracing::error!("Submit error: {}", e);
                    }
                });
            }
            Err(err) => {
                self.push_system_alert(err);
            }
        }
    }

    async fn execute_model_command(&mut self, subcmd: ModelSubcommand) {
        match subcmd {
            ModelSubcommand::ShowCurrent => match self.client.get_model().await {
                Ok(model) => self.push_system_alert(format!("Current model: {model}")),
                Err(err) => self.push_system_alert(format!("Failed to fetch model: {err}")),
            },
            ModelSubcommand::Set { provider, model } => {
                match self.client.set_model(&provider, model.as_deref()).await {
                    Ok(active_model) => {
                        self.push_system_alert(format!("Switched model: {active_model}"))
                    }
                    Err(err) => self.push_system_alert(format!("Failed to switch model: {err}")),
                }
            }
        }
    }
}

// Command parsing tests are now in command.rs
