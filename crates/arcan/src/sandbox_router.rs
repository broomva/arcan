//! Sandbox backend selection from the `ARCAN_SANDBOX_BACKEND` environment variable.
//!
//! # Supported values
//!
//! | `ARCAN_SANDBOX_BACKEND` | Provider | Notes |
//! |------------------------|----------|-------|
//! | `"vercel"` | [`VercelSandboxProvider`] | Requires `VERCEL_TOKEN` (BRO-242) |
//! | `"local"` | [`LocalSandboxProvider`] | Docker or nsjail (BRO-244) |
//! | `"bwrap"` / `"bubblewrap"` | [`BubblewrapProvider`] | Linux namespaces, bwrap fallback (BRO-245) |
//! | absent / `"none"` | — | Sandbox provider disabled |
//!
//! Returns `None` if the env var is unset, empty, or `"none"`.  A `None`
//! return value is non-fatal — the agent runtime continues without sandbox
//! provider support.

use std::sync::Arc;

use arcan_sandbox::SandboxProvider;

/// Read `ARCAN_SANDBOX_BACKEND` and instantiate the corresponding provider.
///
/// Returns `None` if no backend is configured or if initialisation fails.
/// All failures are logged via `tracing` before returning `None`.
pub fn build_sandbox_provider() -> Option<Arc<dyn SandboxProvider>> {
    match std::env::var("ARCAN_SANDBOX_BACKEND")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "vercel" => match arcan_provider_vercel::VercelSandboxProvider::from_env() {
            Ok(p) => {
                tracing::info!("Sandbox backend: Vercel (BRO-242)");
                Some(Arc::new(p))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "ARCAN_SANDBOX_BACKEND=vercel but provider init failed; sandbox disabled"
                );
                None
            }
        },

        "local" => match arcan_provider_local::LocalSandboxProvider::from_env() {
            Ok(p) => {
                tracing::info!("Sandbox backend: Local (Docker/nsjail, BRO-244)");
                Some(Arc::new(p))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "ARCAN_SANDBOX_BACKEND=local but provider init failed; sandbox disabled"
                );
                None
            }
        },

        "bwrap" | "bubblewrap" => {
            let p = arcan_provider_bubblewrap::BubblewrapProvider::from_env();
            tracing::info!(
                use_bwrap = p.use_bwrap,
                "Sandbox backend: Bubblewrap (BRO-245)"
            );
            Some(Arc::new(p))
        }

        "" | "none" => {
            tracing::debug!("ARCAN_SANDBOX_BACKEND not set — sandbox provider disabled");
            None
        }

        unknown => {
            tracing::warn!(
                backend = %unknown,
                "Unknown ARCAN_SANDBOX_BACKEND value — sandbox provider disabled"
            );
            None
        }
    }
}
