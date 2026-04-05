use arcan_core::error::CoreError;
use serde::Deserialize;
use std::process::Command;
use std::time::{Duration, Instant};

/// Resolve the Ollama base URL from `OLLAMA_BASE_URL` or default.
pub fn resolve_base_url() -> String {
    std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".to_string())
}

/// Check if the Ollama server is reachable at `base_url`.
pub fn is_ollama_running(base_url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()
        .and_then(|client| client.get(base_url).send().ok())
        .is_some_and(|resp| resp.status().is_success())
}

/// Ensure Ollama is running, auto-starting it if necessary.
///
/// 1. Returns immediately if already running.
/// 2. Checks `ollama --version` to verify installation.
/// 3. Spawns `ollama serve` as a background process.
/// 4. Polls health for up to ~10 seconds with exponential backoff.
pub fn ensure_ollama_running(base_url: &str) -> Result<(), CoreError> {
    if is_ollama_running(base_url) {
        tracing::debug!("Ollama already running at {base_url}");
        return Ok(());
    }

    // Verify ollama is installed
    let version_ok = Command::new("ollama")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !version_ok {
        return Err(CoreError::Provider(
            "Ollama is not installed. Install from https://ollama.com and try again.".to_string(),
        ));
    }

    tracing::info!("Ollama not running — starting `ollama serve` in background");

    // Spawn detached background process
    Command::new("ollama")
        .arg("serve")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| CoreError::Provider(format!("failed to spawn `ollama serve`: {e}")))?;

    // Poll health with backoff: 250ms, 500ms, 1s, 1s, 1s, ... up to ~10s
    let start = Instant::now();
    let timeout = Duration::from_secs(10);
    let mut delay = Duration::from_millis(250);

    while start.elapsed() < timeout {
        std::thread::sleep(delay);
        if is_ollama_running(base_url) {
            tracing::info!("Ollama started successfully at {base_url}");
            return Ok(());
        }
        delay = (delay * 2).min(Duration::from_secs(1));
    }

    Err(CoreError::Provider(format!(
        "Ollama did not become ready at {base_url} within {timeout:?} after auto-start"
    )))
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    name: String,
}

/// List locally downloaded Ollama models by querying `GET /api/tags`.
pub fn list_ollama_models(base_url: &str) -> Result<Vec<String>, CoreError> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));

    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CoreError::Provider(format!("HTTP client error: {e}")))?
        .get(&url)
        .send()
        .map_err(|e| CoreError::Provider(format!("failed to query Ollama models at {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(CoreError::Provider(format!(
            "Ollama /api/tags returned status {}",
            response.status()
        )));
    }

    let tags: TagsResponse = response
        .json()
        .map_err(|e| CoreError::Provider(format!("failed to parse Ollama model list: {e}")))?;

    Ok(tags
        .models
        .into_iter()
        .map(|m| {
            // Strip `:latest` suffix for cleaner display
            m.name
                .strip_suffix(":latest")
                .map(String::from)
                .unwrap_or(m.name)
        })
        .collect())
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
    }

    #[test]
    fn is_ollama_running_returns_false_for_bad_url() {
        assert!(!is_ollama_running("http://127.0.0.1:1"));
    }

    #[test]
    fn list_ollama_models_errors_on_bad_url() {
        let result = list_ollama_models("http://127.0.0.1:1");
        assert!(result.is_err());
    }
}
