use arcan_core::error::CoreError;
use serde::Deserialize;
use std::process::Command;
use std::time::{Duration, Instant};

/// Default apfel server port (avoids collision with Ollama's 11434).
pub const DEFAULT_PORT: u16 = 11435;

/// Resolve the apfel base URL from `APFEL_BASE_URL` or default.
pub fn resolve_base_url() -> String {
    std::env::var("APFEL_BASE_URL").unwrap_or_else(|_| format!("http://localhost:{DEFAULT_PORT}"))
}

/// Check if the apfel server is reachable at `base_url`.
pub fn is_apfel_running(base_url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()
        .and_then(|client| client.get(format!("{base_url}/health")).send().ok())
        .and_then(|resp| resp.json::<HealthResponse>().ok())
        .is_some_and(|h| h.status == "ok" && h.model_available)
}

/// Ensure apfel is running, auto-starting it if necessary.
///
/// 1. Returns immediately if already running.
/// 2. Checks `apfel --version` to verify installation.
/// 3. Spawns `apfel --serve --port <port>` as a background process.
/// 4. Polls health for up to ~10 seconds with exponential backoff.
pub fn ensure_apfel_running(base_url: &str) -> Result<(), CoreError> {
    if is_apfel_running(base_url) {
        tracing::debug!("apfel already running at {base_url}");
        return Ok(());
    }

    // Verify apfel is installed
    let version_ok = Command::new("apfel")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !version_ok {
        return Err(CoreError::Provider(
            "apfel is not installed. Install with: brew install Arthur-Ficial/tap/apfel"
                .to_string(),
        ));
    }

    // Extract port from base_url for --port flag
    let port = base_url
        .rsplit(':')
        .next()
        .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
        .unwrap_or(DEFAULT_PORT);

    tracing::info!("apfel not running — starting `apfel --serve --port {port}` in background");

    Command::new("apfel")
        .args(["--serve", "--port", &port.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| CoreError::Provider(format!("failed to spawn `apfel --serve`: {e}")))?;

    // Poll health with backoff: 250ms, 500ms, 1s, ... up to ~10s
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let mut delay = Duration::from_millis(250);

    while start.elapsed() < timeout {
        std::thread::sleep(delay);
        if is_apfel_running(base_url) {
            tracing::info!("apfel started successfully at {base_url}");
            return Ok(());
        }
        delay = (delay * 2).min(Duration::from_secs(1));
    }

    Err(CoreError::Provider(format!(
        "apfel did not become ready at {base_url} within {timeout:?} after auto-start"
    )))
}

/// Query apfel model info from the health endpoint.
pub fn model_info(base_url: &str) -> Result<ApfelModelInfo, CoreError> {
    let url = format!("{}/health", base_url.trim_end_matches('/'));

    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CoreError::Provider(format!("HTTP client error: {e}")))?
        .get(&url)
        .send()
        .map_err(|e| CoreError::Provider(format!("failed to query apfel at {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(CoreError::Provider(format!(
            "apfel /health returned status {}",
            response.status()
        )));
    }

    let health: HealthResponse = response
        .json()
        .map_err(|e| CoreError::Provider(format!("failed to parse apfel health: {e}")))?;

    Ok(ApfelModelInfo {
        model: health.model,
        context_window: health.context_window,
        version: health.version,
        languages: health.supported_languages,
    })
}

#[derive(Debug)]
pub struct ApfelModelInfo {
    pub model: String,
    pub context_window: u32,
    pub version: String,
    pub languages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
    model: String,
    model_available: bool,
    context_window: u32,
    version: String,
    #[serde(default)]
    supported_languages: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_base_url_returns_valid_url() {
        let url = resolve_base_url();
        assert!(
            url.starts_with("http"),
            "base URL should be an HTTP URL, got: {url}"
        );
        assert!(
            url.contains(&DEFAULT_PORT.to_string()),
            "default URL should use port {DEFAULT_PORT}, got: {url}"
        );
    }

    #[test]
    fn is_apfel_running_returns_false_for_bad_url() {
        assert!(!is_apfel_running("http://127.0.0.1:1"));
    }

    #[test]
    fn model_info_errors_on_bad_url() {
        let result = model_info("http://127.0.0.1:1");
        assert!(result.is_err());
    }
}
