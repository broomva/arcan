pub mod app;
pub mod client;
pub mod command;
pub mod event;
pub mod focus;
pub mod history;
pub mod models;
#[cfg(test)]
mod models_test;
pub mod network;
#[cfg(test)]
pub mod test_utils;
pub mod theme;
pub mod ui;
pub mod widgets;

use app::App;
use client::AgentClientPort;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use network::{HttpAgentClient, NetworkConfig};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::sync::Arc;

/// Launch the TUI, connecting to an already-running daemon via HTTP/SSE.
///
/// This sets up the terminal, runs the event loop, and restores the terminal on
/// exit. The caller is responsible for ensuring the daemon is reachable at
/// `base_url` before calling this function.
pub async fn run_tui(base_url: String, session_id: String) -> anyhow::Result<()> {
    let config = NetworkConfig {
        base_url,
        session_id,
    };
    let client: Arc<dyn AgentClientPort> = Arc::new(HttpAgentClient::new(config));
    run_tui_with_client(client).await
}

/// Launch the TUI with a pre-built client implementing `AgentClientPort`.
///
/// This is the generic entry point for both HTTP/SSE mode and future in-process
/// mode. The caller provides the client; the TUI handles terminal setup and
/// teardown.
pub async fn run_tui_with_client(client: Arc<dyn AgentClientPort>) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let mut app = App::new(client);
    let res = app.run(&mut terminal).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(ref err) = res {
        eprintln!("{err:?}");
    }

    res
}
