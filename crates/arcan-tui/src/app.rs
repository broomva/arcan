use crate::client::AgentClientPort;
use crate::command::{self, Command, ModelSubcommand};
use crate::event::{ScrollDirection, TuiEvent, event_pump};
use crate::focus::FocusTarget;
use crate::models::state::{AppState, ConnectionStatus};
use crate::models::ui_block::UiBlock;
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
    pub client: Arc<dyn AgentClientPort>,
    pub session_browser: SessionBrowserState,
    pub state_inspector: StateInspectorState,
    /// Markdown renderer with caching for assistant message formatting.
    pub markdown: MarkdownRenderer,
    /// Whether the side panels (session browser + state inspector) are visible.
    pub show_panels: bool,
    pub(crate) events: mpsc::Receiver<TuiEvent>,
    /// Sender half of the unified event channel, used to inject network events
    /// after a session switch.
    pub(crate) event_tx: mpsc::Sender<TuiEvent>,
}

impl App {
    /// Create a new `App` from a pre-built client. The client's `subscribe_events`
    /// method is called to obtain the initial event stream.
    pub fn new(client: Arc<dyn AgentClientPort>) -> Self {
        let network_rx = client.subscribe_events();

        // Merge terminal + network + ticks into a single event stream
        let (events, event_tx) = event_pump(network_rx, Duration::from_millis(50));

        let base_url = client.base_url();
        let session_id = client.session_id();

        let mut app = Self {
            state: AppState::new(),
            input_bar: InputBarState::new(),
            should_quit: false,
            client,
            session_browser: SessionBrowserState::new(),
            state_inspector: StateInspectorState::new(),
            markdown: MarkdownRenderer::new(),
            show_panels: false,
            events,
            event_tx,
        };

        app.state.session_id = Some(session_id.clone());
        app.push_system_alert(format!("Arcan TUI | {base_url} | Session: {session_id}"));

        app
    }

    pub(crate) fn push_system_alert(&mut self, text: impl Into<String>) {
        self.state.blocks.push(UiBlock::SystemAlert {
            text: text.into(),
            timestamp: Utc::now(),
        });
        self.state.scroll.scroll_to_bottom();
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
                TuiEvent::ConnectionLost => {
                    self.state.connection_status = ConnectionStatus::Disconnected;
                    self.state.flash_error("Connection to daemon lost");
                    if self.state.is_busy {
                        self.state.is_busy = false;
                        self.push_system_alert(
                            "Connection lost while waiting for response. \
                             Restart the daemon and try again.",
                        );
                    }
                }
                TuiEvent::MouseScroll(direction) => {
                    // Mouse scroll works regardless of focus — always scrolls chat log.
                    // 3 lines per scroll tick feels natural on mobile and desktop.
                    match direction {
                        ScrollDirection::Up => self.state.scroll.scroll_up(3),
                        ScrollDirection::Down => self.state.scroll.scroll_down(3),
                    }
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

    pub(crate) async fn handle_key(&mut self, key: KeyEvent) {
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
            KeyCode::PageDown => self.state.scroll.page_down(),
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
                    let new_id = id.to_string();
                    self.switch_to_session(new_id).await;
                }
            }
            KeyCode::Char('r') => {
                // Refresh session list
                self.fetch_sessions().await;
            }
            _ => {}
        }
    }

    async fn switch_to_session(&mut self, new_id: String) {
        match self.client.switch_session(&new_id).await {
            Ok(new_rx) => {
                // Spawn a forwarding task from the new receiver to our event channel
                let tx = self.event_tx.clone();
                tokio::spawn(async move {
                    let mut rx = new_rx;
                    while let Some(agent_event) = rx.recv().await {
                        if tx.send(TuiEvent::Network(agent_event)).await.is_err() {
                            break;
                        }
                    }
                });

                // Reset state for the new session
                self.state.reset_for_session_switch(new_id.clone());

                // Close panels, focus input
                self.show_panels = false;
                self.state.focus = FocusTarget::InputBar;

                self.push_system_alert(format!("Switched to session: {new_id}"));
            }
            Err(e) => {
                self.push_system_alert(format!("Failed to switch session: {e}"));
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
            ModelSubcommand::ShowCurrent => {
                self.push_system_alert(
                    "Model info: check ARCAN_PROVIDER env var or use `arcan status` \
                     in another terminal. The /model command is not yet wired to the daemon API.",
                );
            }
            ModelSubcommand::Set { provider, model } => {
                let requested = match model {
                    Some(ref m) => format!("{provider}:{m}"),
                    None => provider.clone(),
                };
                self.push_system_alert(format!(
                    "Model switching ({requested}) is not yet available via the TUI. \
                     Set ARCAN_PROVIDER before starting the daemon.",
                ));
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
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = submit_client.submit_run(&text, None).await {
                tracing::error!("Submit error: {}", e);
                // Surface the error back to the UI so is_busy resets and user sees feedback
                let _ = tx
                    .send(TuiEvent::Network(
                        arcan_core::protocol::AgentEvent::RunErrored {
                            run_id: "submit".to_string(),
                            session_id: String::new(),
                            error: format!("Failed to send message: {e}"),
                        },
                    ))
                    .await;
            }
        });
    }

    fn submit_approval(&mut self, approval_id: String, decision: String, reason: Option<String>) {
        let submit_client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = submit_client
                .submit_approval(&approval_id, &decision, reason.as_deref())
                .await
            {
                tracing::error!("Submit approval error: {}", e);
                let _ = tx
                    .send(TuiEvent::Network(
                        arcan_core::protocol::AgentEvent::RunErrored {
                            run_id: "approval".to_string(),
                            session_id: String::new(),
                            error: format!("Approval failed: {e}"),
                        },
                    ))
                    .await;
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
                    .map(|s| SessionEntry {
                        session_id: s.session_id,
                        owner: s.owner,
                        created_at: s
                            .created_at
                            .as_deref()
                            .and_then(|ts| ts.parse().ok())
                            .unwrap_or_else(Utc::now),
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
            Ok(resp) => {
                let snapshot = AgentStateSnapshot {
                    session_id: resp.session_id,
                    branch: resp.branch,
                    mode: resp.mode,
                    progress: resp.state.progress as f32,
                    uncertainty: resp.state.uncertainty as f32,
                    risk_level: resp.state.risk_level,
                    error_streak: resp.state.error_streak as u32,
                    context_pressure: resp.state.context_pressure as f32,
                    side_effect_pressure: resp.state.side_effect_pressure as f32,
                    human_dependency: resp.state.human_dependency as f32,
                    tokens_remaining: resp.state.budget.tokens_remaining,
                    time_remaining_ms: resp.state.budget.time_remaining_ms,
                    cost_remaining_usd: resp.state.budget.cost_remaining_usd,
                    tool_calls_remaining: resp.state.budget.tool_calls_remaining as u32,
                    error_budget_remaining: resp.state.budget.error_budget_remaining as u32,
                    version: resp.version,
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
