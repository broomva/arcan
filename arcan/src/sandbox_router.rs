//! Sandbox backend selection from the `ARCAN_SANDBOX_BACKEND` environment variable.
//!
//! # Supported values
//!
//! | `ARCAN_SANDBOX_BACKEND` | Provider | Notes |
//! |------------------------|----------|-------|
//! | `"vercel"` | [`VercelSandboxProvider`] | Requires `VERCEL_TOKEN`, optional `VERCEL_TEAM_ID` / `VERCEL_PROJECT_ID` |
//! | `"local"` | [`LocalSandboxProvider`] | Docker or nsjail (BRO-244) |
//! | `"bwrap"` / `"bubblewrap"` | [`BubblewrapProvider`] | Linux namespaces, bwrap fallback (BRO-245) |
//! | absent / `"none"` | — | Sandbox provider disabled |
//!
//! Returns `None` if the env var is unset, empty, or `"none"`.  A `None`
//! return value is non-fatal — the agent runtime continues without sandbox
//! provider support.
//!
//! ## Vercel v2 API (BRO-263)
//!
//! The Vercel provider uses the v2 named-sandbox API.  Set `VERCEL_PROJECT_ID`
//! to enable project-scoped sandbox listing.  The sandbox name is derived
//! deterministically as `arcan-{session_id}` via
//! [`arcan_sandbox::sandbox_name_for_session`].

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

/// Auto-detect the best available sandbox provider, always returning one.
///
/// Implements a tiered detection chain:
///
/// 1. If `ARCAN_SANDBOX_BACKEND` is explicitly set (and not `"auto"`), delegate to
///    [`build_sandbox_provider`].  If it returns `Some`, use it.  If it returns
///    `None` (init failure), log a warning and fall through to auto-detection.
/// 2. If `VERCEL_TOKEN` is present in the environment, attempt
///    [`arcan_provider_vercel::VercelSandboxProvider::from_env`].  On success,
///    return it.  On failure, log a warning and continue.
/// 3. Create [`arcan_provider_bubblewrap::BubblewrapProvider::from_env`].  If the
///    provider reports `use_bwrap = true`, log "Sandbox: Bubblewrap (Linux namespace
///    isolation)"; otherwise log "Sandbox: subprocess fallback (workspace directory
///    isolation)".
///
/// This function **never** returns `None` — the minimum guarantee is
/// subprocess-level workspace isolation.
pub fn build_sandbox_provider_with_fallback() -> Arc<dyn SandboxProvider> {
    let explicit_backend = std::env::var("ARCAN_SANDBOX_BACKEND")
        .unwrap_or_default()
        .to_lowercase();

    // Tier 1: honour an explicit (non-auto) backend setting.
    if !explicit_backend.is_empty() && explicit_backend != "auto" {
        match build_sandbox_provider() {
            Some(provider) => return provider,
            None => {
                tracing::warn!(
                    backend = %explicit_backend,
                    "ARCAN_SANDBOX_BACKEND explicit value failed init; falling through to auto-detect"
                );
            }
        }
    }

    // Tier 2: Vercel sandbox when VERCEL_TOKEN is available.
    if std::env::var("VERCEL_TOKEN").is_ok() {
        match arcan_provider_vercel::VercelSandboxProvider::from_env() {
            Ok(p) => {
                tracing::info!("Sandbox: Vercel (auto-detected via VERCEL_TOKEN)");
                return Arc::new(p);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "VERCEL_TOKEN present but Vercel provider init failed; continuing auto-detect"
                );
            }
        }
    }

    // Tier 3: Bubblewrap / subprocess fallback — always succeeds.
    let p = arcan_provider_bubblewrap::BubblewrapProvider::from_env();
    if p.use_bwrap {
        tracing::info!("Sandbox: Bubblewrap (Linux namespace isolation)");
    } else {
        tracing::info!("Sandbox: subprocess fallback (workspace directory isolation)");
    }
    Arc::new(p)
}
