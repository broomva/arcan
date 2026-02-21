mod app;
mod models;
#[cfg(test)]
mod models_test;
mod network;
mod ui;

use app::App;
use clap::Parser;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// URL of the arcan daemon
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    url: String,

    /// Session ID to attach to
    #[arg(short, long)]
    session: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Set up file-based tracing so we don't clobber the TUI stdout
    let file_appender = tracing_appender::rolling::never(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join(".arcan"),
        "tui.log",
    );
    tracing_subscriber::fmt()
        .with_writer(file_appender)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Starting Arcan TUI for session {}", args.session);
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let config = network::NetworkConfig {
        base_url: args.url,
        session_id: args.session,
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

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}
