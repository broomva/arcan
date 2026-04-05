mod assets;

use axum::Router;
use std::path::PathBuf;

/// Configuration for the console asset server.
#[derive(Debug, Clone, Default)]
pub struct ConsoleConfig {
    /// Override the embedded assets with a local directory (for dev mode).
    pub override_dir: Option<PathBuf>,
}

/// Create a router that serves the admin console SPA.
///
/// When `override_dir` is set, files are served from disk (for `npm run dev` workflows).
/// Otherwise, assets are served from the binary via `rust-embed`.
pub fn console_router(config: ConsoleConfig) -> Router {
    if let Some(dir) = config.override_dir {
        tracing::info!(dir = %dir.display(), "Console: serving from filesystem");
        assets::filesystem_router(&dir)
    } else {
        tracing::info!("Console: serving embedded assets");
        assets::embedded_router()
    }
}
