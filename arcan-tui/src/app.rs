use crate::client::AgentClientPort;
use crate::command::{self, COMMANDS, Command, ModelSubcommand};
use crate::event::{ScrollDirection, TuiEvent, event_pump};
use crate::focus::FocusTarget;
use crate::models::state::{AppState, ConnectionStatus};
use crate::models::ui_block::UiBlock;
use crate::ui;
use crate::widgets::autocomplete::AutocompleteState;
use crate::widgets::input_bar::InputBarState;
use crate::widgets::markdown::MarkdownRenderer;
use crate::widgets::provider_picker::ProviderPickerState;
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
    /// Slash command autocomplete popup state.
    pub autocomplete: AutocompleteState,
    /// Provider picker popup state.
    pub provider_picker: ProviderPickerState,
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
            autocomplete: AutocompleteState::new(),
            provider_picker: ProviderPickerState::new(),
            events,
            event_tx,
        };

        app.state.session_id = Some(session_id.clone());
        app.push_system_alert(format!("Arcan TUI | {base_url} | Session: {session_id}"));

        // Async version check — warns if daemon is running a different version
        let version_client = app.client.clone();
        let version_tx = app.event_tx.clone();
        tokio::spawn(async move {
            let tui_version = env!("CARGO_PKG_VERSION");
            match version_client.get_daemon_version().await {
                Ok(daemon_version) if daemon_version != tui_version => {
                    let msg = format!(
                        "Version mismatch: TUI v{tui_version} / daemon v{daemon_version}. \
                         Restart the daemon: cargo run -p arcan -- serve"
                    );
                    let _ = version_tx.send(TuiEvent::SystemAlert(msg)).await;
                }
                Err(_) => {
                    let msg = format!(
                        "Could not verify daemon version (TUI v{tui_version}). \
                         The daemon may be outdated — consider restarting it."
                    );
                    let _ = version_tx.send(TuiEvent::SystemAlert(msg)).await;
                }
                Ok(_) => {} // versions match, nothing to report
            }
        });

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
                    let was_busy = self.state.is_busy;
                    self.state.apply_event(agent_event);
                    // When a run finishes, fetch status bar data from daemon
                    if was_busy && !self.state.is_busy {
                        self.refresh_status_bar();
                    }
                }
                TuiEvent::Tick => {
                    self.state
                        .clear_expired_errors(chrono::Duration::seconds(5));
                    if self.state.is_busy {
                        self.state.spinner.tick();
                    }
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
                TuiEvent::OAuthResult { result, .. } => match result {
                    Ok(msg) => self.push_system_alert(msg),
                    Err(msg) => self.push_system_alert(msg),
                },
                TuiEvent::SystemAlert(msg) => {
                    self.push_system_alert(msg);
                }
                TuiEvent::StatusBarUpdate {
                    context_pressure_pct,
                    autonomic_ruling,
                    cost_remaining,
                } => {
                    if let Some(pct) = context_pressure_pct {
                        self.state.context_pressure_pct = pct;
                    }
                    if let Some(ruling) = autonomic_ruling {
                        self.state.autonomic_ruling = Some(ruling);
                    }
                    if let Some(cost) = cost_remaining {
                        self.state.cost_remaining = Some(cost);
                    }
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
                // Dismiss provider picker first, then autocomplete, then panels, then quit
                if self.provider_picker.active {
                    self.provider_picker.dismiss();
                    return;
                }
                if self.autocomplete.active {
                    self.autocomplete.dismiss();
                    return;
                }
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
        // When provider picker is active, intercept navigation keys
        if self.provider_picker.active {
            match key.code {
                KeyCode::Up => {
                    self.provider_picker.previous();
                    return;
                }
                KeyCode::Down => {
                    self.provider_picker.next();
                    return;
                }
                KeyCode::Enter => {
                    if let Some(name) = self.provider_picker.accept() {
                        self.set_provider(&name).await;
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.provider_picker.dismiss();
                    return;
                }
                _ => return, // Ignore all other keys while picker is open
            }
        }

        // When autocomplete is active, intercept navigation keys
        if self.autocomplete.active {
            match key.code {
                KeyCode::Up => {
                    self.autocomplete.previous();
                    return;
                }
                KeyCode::Down => {
                    self.autocomplete.next();
                    return;
                }
                KeyCode::Tab => {
                    if let Some(command_name) = self.autocomplete.accept() {
                        self.input_bar.clear();
                        for ch in command_name.chars() {
                            self.input_bar
                                .input(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                        }
                        self.input_bar
                            .input(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
                    }
                    return;
                }
                KeyCode::Enter => {
                    // If there's an exact match or single suggestion, accept it;
                    // otherwise fall through to normal submit
                    if self.autocomplete.suggestions.len() == 1 {
                        if let Some(command_name) = self.autocomplete.accept() {
                            self.input_bar.clear();
                            for ch in command_name.chars() {
                                self.input_bar
                                    .input(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
                            }
                            self.input_bar
                                .input(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
                            return;
                        }
                    }
                    // Multiple suggestions: dismiss and submit as typed
                    self.autocomplete.dismiss();
                    self.handle_submit().await;
                    return;
                }
                _ => {
                    // Fall through to normal input handling, then update autocomplete
                }
            }
        }

        match key.code {
            KeyCode::Enter => {
                self.autocomplete.dismiss();
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
                // Update autocomplete after every keystroke
                self.autocomplete.update(self.input_bar.text());
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

        let parsed = command::parse(&trimmed);

        // Echo slash commands in the chat log so the user sees what they typed.
        // Skip for /clear (about to wipe the log) and plain messages (echoed by submit_message).
        if trimmed.starts_with('/') && !matches!(parsed, Ok(Command::Clear)) {
            self.state.blocks.push(UiBlock::HumanMessage {
                text: trimmed.clone(),
                timestamp: Utc::now(),
            });
            self.state.scroll.scroll_to_bottom();
        }

        match parsed {
            Ok(Command::Autonomic) => {
                self.show_autonomic().await;
            }
            Ok(Command::Clear) => {
                self.state.blocks.clear();
                self.state.streaming_text = None;
                self.state.is_busy = false;
            }
            Ok(Command::Compact) => {
                self.show_compact().await;
            }
            Ok(Command::Config) => {
                self.show_config().await;
            }
            Ok(Command::Help) => {
                let lines: Vec<String> = COMMANDS
                    .iter()
                    .map(|c| format!("  {:<12} {}", c.name, c.description))
                    .collect();
                self.push_system_alert(format!("Available commands:\n{}", lines.join("\n")));
            }
            Ok(Command::Context) => {
                self.show_context().await;
            }
            Ok(Command::Cost) => {
                self.show_cost().await;
            }
            Ok(Command::Memory) => {
                self.show_memory();
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
            Ok(Command::Status) => {
                self.show_status().await;
            }
            Ok(Command::Login { provider, device }) => {
                self.execute_login(provider, device);
            }
            Ok(Command::Logout { provider }) => {
                self.execute_logout(provider);
            }
            Ok(Command::Provider { name: None }) => {
                self.show_provider_status().await;
            }
            Ok(Command::Provider { name: Some(name) }) => {
                self.set_provider(&name).await;
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
                Ok(provider) => {
                    self.push_system_alert(format!("Active provider: {provider}"));
                }
                Err(e) => {
                    self.push_system_alert(format!("Failed to query provider: {e}"));
                }
            },
            ModelSubcommand::Set { provider, model } => {
                match self.client.set_model(&provider, model.as_deref()).await {
                    Ok(new_provider) => {
                        self.push_system_alert(format!("Provider switched to: {new_provider}"));
                    }
                    Err(e) => {
                        self.push_system_alert(format!("Failed to switch provider: {e}"));
                    }
                }
            }
        }
    }

    fn execute_login(&mut self, provider: String, device: bool) {
        let canonical = match provider.as_str() {
            "codex" | "openai-codex" => "openai",
            other => other,
        };

        match canonical {
            "openai" => {
                if device {
                    self.push_system_alert(
                        "Starting device code authentication for OpenAI...\n\
                         Requesting verification code...",
                    );
                } else {
                    self.push_system_alert(
                        "Starting browser-based authentication for OpenAI...\n\
                         Your browser should open automatically.",
                    );
                }

                let tx = self.event_tx.clone();
                let canonical = canonical.to_string();

                // OAuth flows use blocking HTTP + eprintln! output that would corrupt
                // the TUI alternate screen. Run on a dedicated OS thread with stderr
                // redirected to /dev/null.
                std::thread::spawn(move || {
                    // Redirect stderr to suppress eprintln! from the OAuth module.
                    // The TUI shows its own alerts instead.
                    let stderr_suppressed = suppress_stderr();

                    let result = if device {
                        arcan_provider::oauth::device_login_openai()
                    } else {
                        arcan_provider::oauth::pkce_login_openai()
                    };

                    // Capture any stderr output before restoring, then restore stderr
                    let captured = restore_stderr(stderr_suppressed);

                    let oauth_result = match result {
                        Ok(_tokens) => {
                            // Update config to set default provider
                            let mut config = crate::config::load_global_config();
                            if config.set_key("provider", &canonical).is_ok() {
                                let _ = crate::config::save_global_config(&config);
                            }
                            Ok(format!(
                                "Successfully authenticated with {canonical}. \
                                 Restart the daemon to use OAuth credentials."
                            ))
                        }
                        Err(e) => Err(format!("Login failed: {e}")),
                    };

                    // If device flow, extract user code from captured stderr and
                    // send it as a separate alert so the user can see it in the TUI.
                    if device {
                        if let Some(ref captured) = captured {
                            // Look for "Enter code: XXXX" in captured output
                            for line in captured.lines() {
                                if line.contains("Enter code:") || line.contains("visit:") {
                                    let _ = tx.blocking_send(TuiEvent::OAuthResult {
                                        provider: canonical.clone(),
                                        result: Ok(line.trim().to_string()),
                                    });
                                }
                            }
                        }
                    }

                    let _ = tx.blocking_send(TuiEvent::OAuthResult {
                        provider: canonical,
                        result: oauth_result,
                    });
                });
            }
            _ => {
                self.push_system_alert(format!("Unknown provider '{provider}'. Supported: openai"));
            }
        }
    }

    fn execute_logout(&mut self, provider: String) {
        let canonical = match provider.as_str() {
            "codex" | "openai-codex" => "openai",
            other => other,
        };

        match arcan_provider::oauth::remove_tokens(canonical) {
            Ok(()) => {
                // Clear default provider if it matches
                let mut config = crate::config::load_global_config();
                if config.defaults.provider.as_deref() == Some(canonical) {
                    config.defaults.provider = None;
                    let _ = crate::config::save_global_config(&config);
                }
                self.push_system_alert(format!(
                    "Logged out from {canonical}. Credentials removed."
                ));
            }
            Err(e) => {
                self.push_system_alert(format!("Logout failed: {e}"));
            }
        }
    }

    async fn show_provider_status(&mut self) {
        // Show interactive provider picker popup
        self.provider_picker.show_loading();

        match self.client.get_provider_info().await {
            Ok(info) => {
                self.provider_picker
                    .set_providers(info.provider, info.available);
            }
            Err(e) => {
                self.provider_picker.dismiss();
                self.push_system_alert(format!("Failed to query provider from daemon: {e}"));
            }
        }
    }

    /// Fire-and-forget: fetch context/autonomic/cost from daemon to update status bar.
    fn refresh_status_bar(&self) {
        let client = self.client.clone();
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            // Fetch all three in parallel
            let (ctx_result, cost_result) = tokio::join!(client.get_context(), client.get_cost(),);

            if let Ok(ctx) = ctx_result {
                let _ = tx
                    .send(TuiEvent::StatusBarUpdate {
                        context_pressure_pct: Some(ctx.pressure_percent),
                        autonomic_ruling: Some(ctx.ruling),
                        cost_remaining: None,
                    })
                    .await;
            }
            if let Ok(cost) = cost_result {
                let _ = tx
                    .send(TuiEvent::StatusBarUpdate {
                        context_pressure_pct: None,
                        autonomic_ruling: None,
                        cost_remaining: Some(cost.cost_remaining_usd),
                    })
                    .await;
            }
        });
    }

    async fn show_autonomic(&mut self) {
        match self.client.get_autonomic().await {
            Ok(info) => {
                self.push_system_alert(format!(
                    "Autonomic: {} — pressure {:.0}%, quality {:.2}\n  \
                     Context window: {}  |  Rationale: {}{}",
                    info.ruling,
                    info.pressure * 100.0,
                    info.quality_score,
                    info.context_window,
                    info.rationale,
                    info.target_tokens
                        .map(|t| format!("\n  Target tokens: {t}"))
                        .unwrap_or_default(),
                ));
            }
            Err(e) => {
                self.push_system_alert(format!("Failed to query autonomic: {e}"));
            }
        }
    }

    async fn show_context(&mut self) {
        match self.client.get_context().await {
            Ok(info) => {
                self.push_system_alert(format!(
                    "Context: {:.1}% used ({} / {} tokens) — ruling: {}",
                    info.pressure_percent, info.tokens_used, info.context_window, info.ruling,
                ));
            }
            Err(e) => {
                self.push_system_alert(format!("Failed to query context: {e}"));
            }
        }
    }

    async fn show_cost(&mut self) {
        match self.client.get_cost().await {
            Ok(info) => {
                let uptime_min = info.uptime_seconds / 60;
                self.push_system_alert(format!(
                    "Cost: ${:.4} remaining  |  {} tokens remaining  |  uptime: {}m",
                    info.cost_remaining_usd, info.tokens_remaining, uptime_min,
                ));
            }
            Err(e) => {
                self.push_system_alert(format!("Failed to query cost: {e}"));
            }
        }
    }

    async fn show_compact(&mut self) {
        // Stub: daemon does not yet expose a /compact POST endpoint.
        self.push_system_alert(
            "Compact not yet implemented in daemon. \
             This command will trigger context compaction once the daemon supports it.",
        );
    }

    async fn show_config(&mut self) {
        let mut lines = Vec::new();

        // Fetch provider info from daemon
        match self.client.get_provider_info().await {
            Ok(info) => {
                lines.push(format!("  provider: {}", info.provider));
                lines.push(format!("  available: {}", info.available.join(", ")));
            }
            Err(e) => {
                lines.push(format!("  provider: (error: {e})"));
            }
        }

        // Get model from daemon
        match self.client.get_model().await {
            Ok(model) => lines.push(format!("  model: {model}")),
            Err(_) => lines.push("  model: (unknown)".to_string()),
        }

        // Session & connection info
        lines.push(format!(
            "  session: {}",
            self.state.session_id.as_deref().unwrap_or("(none)")
        ));
        lines.push(format!("  daemon: {}", self.client.base_url()));

        // Local config
        let config = crate::config::load_global_config();
        if let Some(data_dir) = crate::config::global_config_path() {
            lines.push(format!("  config-file: {}", data_dir.display()));
        }
        if let Some(ref p) = config.defaults.provider {
            lines.push(format!("  config.defaults.provider: {p}"));
        }
        if let Some(ref m) = config.defaults.model {
            lines.push(format!("  config.defaults.model: {m}"));
        }

        self.push_system_alert(format!("Configuration:\n{}", lines.join("\n")));
    }

    fn show_memory(&mut self) {
        let memory_dir = dirs::data_dir()
            .or_else(dirs::home_dir)
            .map(|d| d.join(".arcan").join("memory"));

        let Some(dir) = memory_dir else {
            self.push_system_alert("Memory: could not determine data directory.");
            return;
        };

        if !dir.exists() {
            self.push_system_alert(format!(
                "Memory: no memory directory found at {}",
                dir.display()
            ));
            return;
        }

        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut files: Vec<String> = Vec::new();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        let size_str = if size < 1024 {
                            format!("{size} B")
                        } else {
                            format!("{:.1} KB", size as f64 / 1024.0)
                        };
                        files.push(format!("  {name:<30} {size_str}"));
                    }
                }
                if files.is_empty() {
                    self.push_system_alert(format!(
                        "Memory: directory is empty ({})",
                        dir.display()
                    ));
                } else {
                    files.sort();
                    self.push_system_alert(format!(
                        "Memory files ({}):\n{}",
                        dir.display(),
                        files.join("\n")
                    ));
                }
            }
            Err(e) => {
                self.push_system_alert(format!(
                    "Memory: could not read directory {}: {e}",
                    dir.display()
                ));
            }
        }
    }

    async fn show_status(&mut self) {
        let mut lines = Vec::new();

        // Session info
        lines.push(format!(
            "  Session: {}",
            self.state.session_id.as_deref().unwrap_or("(none)")
        ));

        // Provider/model
        match self.client.get_model().await {
            Ok(model) => lines.push(format!("  Provider: {model}")),
            Err(_) => lines.push("  Provider: (unknown)".to_string()),
        }

        // Context usage
        match self.client.get_context().await {
            Ok(ctx) => {
                lines.push(format!(
                    "  Context: {:.1}% ({} / {} tokens)",
                    ctx.pressure_percent, ctx.tokens_used, ctx.context_window,
                ));
                lines.push(format!("  Autonomic ruling: {}", ctx.ruling));
            }
            Err(e) => {
                lines.push(format!("  Context: (error: {e})"));
            }
        }

        // Autonomic details
        if let Ok(info) = self.client.get_autonomic().await {
            lines.push(format!(
                "  Autonomic: {} — quality {:.2}, pressure {:.0}%",
                info.ruling,
                info.quality_score,
                info.pressure * 100.0,
            ));
        }

        // Cost
        match self.client.get_cost().await {
            Ok(cost) => {
                let uptime_min = cost.uptime_seconds / 60;
                lines.push(format!(
                    "  Cost: ${:.4} remaining | {} tokens remaining | uptime: {}m",
                    cost.cost_remaining_usd, cost.tokens_remaining, uptime_min,
                ));
            }
            Err(e) => {
                lines.push(format!("  Cost: (error: {e})"));
            }
        }

        self.push_system_alert(format!("Status:\n{}", lines.join("\n")));
    }

    async fn set_provider(&mut self, name: &str) {
        match self.client.set_model(name, None).await {
            Ok(new_provider) => {
                self.push_system_alert(format!("Provider switched to: {new_provider}"));
            }
            Err(e) => {
                self.push_system_alert(format!("Failed to switch provider: {e}"));
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

/// Redirect stderr to a pipe so `eprintln!` output doesn't corrupt the TUI.
/// Returns the saved file descriptor and the read-end of the pipe, or None on failure.
#[cfg(unix)]
fn suppress_stderr() -> Option<(i32, i32)> {
    unsafe {
        let saved = libc::dup(2); // save original stderr fd
        if saved < 0 {
            return None;
        }
        let mut pipe_fds = [0i32; 2];
        if libc::pipe(pipe_fds.as_mut_ptr()) < 0 {
            libc::close(saved);
            return None;
        }
        // Redirect stderr (fd 2) to the write-end of the pipe
        libc::dup2(pipe_fds[1], 2);
        libc::close(pipe_fds[1]);
        Some((saved, pipe_fds[0])) // (saved_stderr, read_end)
    }
}

#[cfg(not(unix))]
fn suppress_stderr() -> Option<(i32, i32)> {
    None // On non-Unix, skip suppression
}

/// Restore stderr and return captured output.
#[cfg(unix)]
fn restore_stderr(state: Option<(i32, i32)>) -> Option<String> {
    let (saved, read_end) = state?;
    unsafe {
        // Restore original stderr
        libc::dup2(saved, 2);
        libc::close(saved);

        // Read captured output from pipe (non-blocking)
        let mut buf = vec![0u8; 4096];
        // Set pipe to non-blocking so read doesn't hang
        let flags = libc::fcntl(read_end, libc::F_GETFL);
        libc::fcntl(read_end, libc::F_SETFL, flags | libc::O_NONBLOCK);

        let n = libc::read(read_end, buf.as_mut_ptr().cast(), buf.len());
        libc::close(read_end);

        if n > 0 {
            buf.truncate(n as usize);
            Some(String::from_utf8_lossy(&buf).to_string())
        } else {
            None
        }
    }
}

#[cfg(not(unix))]
fn restore_stderr(_state: Option<(i32, i32)>) -> Option<String> {
    None
}

// Command parsing tests are now in command.rs
