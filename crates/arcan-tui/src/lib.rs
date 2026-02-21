pub mod app;
pub mod models;
#[cfg(test)]
mod models_test;
pub mod network;
pub mod ui;

use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use network::NetworkConfig;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

/// Launch the TUI, connecting to an already-running daemon.
///
/// This sets up the terminal, runs the event loop, and restores the terminal on
/// exit. The caller is responsible for ensuring the daemon is reachable at
/// `base_url` before calling this function.
pub async fn run_tui(base_url: String, session_id: String) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let config = NetworkConfig {
        base_url,
        session_id,
    };
    let mut app = App::new(config);
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
