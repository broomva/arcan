use crate::command::{self, Command, ModelSubcommand};
use crate::event::{TuiEvent, event_pump};
use crate::focus::FocusTarget;
use crate::models::state::{AppState, ConnectionStatus};
use crate::models::ui_block::UiBlock;
use crate::network::{NetworkClient, NetworkConfig};
use crate::ui;
use crate::widgets::input_bar::InputBarState;
use crate::widgets::markdown::MarkdownRenderer;
use crate::widgets::session_browser::{SessionBrowserState, SessionEntry};
use crate::widgets::state_inspector::{AgentStateSnapshot, StateInspectorState};
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
    pub session_browser: SessionBrowserState,
    pub state_inspector: StateInspectorState,
    /// Markdown renderer with caching for assistant message formatting.
    pub markdown: MarkdownRenderer,
    /// Whether the side panels (session browser + state inspector) are visible.
    pub show_panels: bool,
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
            session_browser: SessionBrowserState::new(),
            state_inspector: StateInspectorState::new(),
            markdown: MarkdownRenderer::new(),
            show_panels: false,
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
        terminal.draw(|f| ui::draw(f, self))?;

        while let Some(event) = self.events.recv().await {
            match event {
                TuiEvent::Key(key) if key.kind == KeyEventKind::Press => {
                    self.handle_key(key).await;
                }
                TuiEvent::Network(agent_event) => {
                    self.state.connection_status = ConnectionStatus::Connected;
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

            terminal.draw(|f| ui::draw(f, self))?;

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
                // If panels are shown, close them first; otherwise quit
                if self.show_panels {
                    self.show_panels = false;
                    self.state.focus = FocusTarget::InputBar;
                    return;
                }
                self.should_quit = true;
                return;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Tab => {
                if self.show_panels && key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.state.focus = self.state.focus.next_all();
                } else {
                    self.state.focus = self.state.focus.next();
                }
                return;
            }
            _ => {}
        }

        // Focus-dependent key handling
        match self.state.focus {
            FocusTarget::ChatLog => self.handle_scroll_key(key.code),
            FocusTarget::InputBar => self.handle_input_key(key).await,
            FocusTarget::SessionBrowser => self.handle_session_browser_key(key.code).await,
            FocusTarget::StateInspector => {
                // State inspector is read-only; scroll keys could be added later
            }
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
                self.input_bar.input(key);
            }
        }
    }

    async fn handle_session_browser_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.session_browser.previous(),
            KeyCode::Down | KeyCode::Char('j') => self.session_browser.next(),
            KeyCode::Enter => {
                // Switch to selected session
                if let Some(id) = self.session_browser.selected_session_id() {
                    let id = id.to_string();
                    self.push_system_alert(format!("Selected session: {id}"));
                    // Could wire up session switching here in the future
                }
            }
            KeyCode::Char('r') => {
                // Refresh session list
                self.fetch_sessions().await;
            }
            _ => {}
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
                    "Commands: /clear, /model, /approve <id> <yes|no>, /sessions, /state, /help",
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
                self.submit_approval(approval_id, decision, reason);
            }
            Ok(Command::Sessions) => {
                self.show_panels = true;
                self.state.focus = FocusTarget::SessionBrowser;
                self.fetch_sessions().await;
            }
            Ok(Command::State) => {
                self.show_panels = true;
                self.state.focus = FocusTarget::StateInspector;
                self.fetch_state().await;
            }
            Ok(Command::SendMessage(text)) => {
                self.submit_message(text);
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

    fn submit_message(&mut self, text: String) {
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

    fn submit_approval(&mut self, approval_id: String, decision: String, reason: Option<String>) {
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

    async fn fetch_sessions(&mut self) {
        self.session_browser.set_loading();
        let client = self.client.clone();
        match client.list_sessions().await {
            Ok(sessions) => {
                let entries: Vec<SessionEntry> = sessions
                    .into_iter()
                    .filter_map(|v| {
                        Some(SessionEntry {
                            session_id: v.get("session_id")?.as_str()?.to_string(),
                            owner: v
                                .get("owner")
                                .and_then(|o| o.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            created_at: v
                                .get("created_at")
                                .and_then(|t| t.as_str())
                                .and_then(|s| s.parse().ok())
                                .unwrap_or_else(Utc::now),
                        })
                    })
                    .collect();
                self.session_browser.set_sessions(entries);
            }
            Err(e) => {
                self.session_browser.set_error(e.to_string());
            }
        }
    }

    async fn fetch_state(&mut self) {
        self.state_inspector.set_loading();
        let client = self.client.clone();
        match client.get_session_state(None).await {
            Ok(v) => {
                let state_obj = v.get("state").cloned().unwrap_or_default();
                let budget = state_obj.get("budget").cloned().unwrap_or_default();

                let snapshot = AgentStateSnapshot {
                    session_id: v
                        .get("session_id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string(),
                    branch: v
                        .get("branch")
                        .and_then(|s| s.as_str())
                        .unwrap_or("main")
                        .to_string(),
                    mode: v
                        .get("mode")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Unknown")
                        .to_string(),
                    progress: state_obj
                        .get("progress")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0) as f32,
                    uncertainty: state_obj
                        .get("uncertainty")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0) as f32,
                    risk_level: state_obj
                        .get("risk_level")
                        .and_then(|s| s.as_str())
                        .unwrap_or("Low")
                        .to_string(),
                    error_streak: state_obj
                        .get("error_streak")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0) as u32,
                    context_pressure: state_obj
                        .get("context_pressure")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0) as f32,
                    side_effect_pressure: state_obj
                        .get("side_effect_pressure")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0) as f32,
                    human_dependency: state_obj
                        .get("human_dependency")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0) as f32,
                    tokens_remaining: budget
                        .get("tokens_remaining")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0),
                    time_remaining_ms: budget
                        .get("time_remaining_ms")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0),
                    cost_remaining_usd: budget
                        .get("cost_remaining_usd")
                        .and_then(|n| n.as_f64())
                        .unwrap_or(0.0),
                    tool_calls_remaining: budget
                        .get("tool_calls_remaining")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0) as u32,
                    error_budget_remaining: budget
                        .get("error_budget_remaining")
                        .and_then(|n| n.as_u64())
                        .unwrap_or(0) as u32,
                    version: v.get("version").and_then(|n| n.as_u64()).unwrap_or(0),
                };
                self.state_inspector.set_snapshot(snapshot);
            }
            Err(e) => {
                self.state_inspector.set_error(e.to_string());
            }
        }
    }
}

// Command parsing tests are now in command.rs
