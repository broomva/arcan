use std::fs;
use std::path::Path;
use std::time::Duration;

/// Check whether a daemon is already healthy at the given URL.
async fn is_daemon_healthy(base_url: &str) -> bool {
    let url = format!("{base_url}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build();
    let Ok(client) = client else { return false };
    matches!(client.get(&url).send().await, Ok(resp) if resp.status().is_success())
}

/// Ensure a daemon is running on the given port.
///
/// 1. Probe `GET /health`. If healthy, return immediately.
/// 2. Otherwise spawn `arcan serve` as a detached child process, writing
///    stdout/stderr to `{data_dir}/daemon.log` and the PID to
///    `{data_dir}/daemon.pid`.
/// 3. Poll `/health` until it succeeds (up to ~12 s).
pub async fn ensure_daemon(data_dir: &Path, port: u16) -> anyhow::Result<String> {
    let base_url = format!("http://127.0.0.1:{port}");

    if is_daemon_healthy(&base_url).await {
        tracing::info!("Daemon already running on {base_url}");
        return Ok(base_url);
    }

    tracing::info!("Starting daemon on port {port}...");

    fs::create_dir_all(data_dir)?;
    let log_file = fs::File::create(data_dir.join("daemon.log"))?;
    let stderr_log = log_file.try_clone()?;

    // Resolve the current executable so `arcan serve` uses the same binary.
    let exe = std::env::current_exe()?;

    let child = std::process::Command::new(exe)
        .args([
            "serve",
            "--port",
            &port.to_string(),
            "--data-dir",
            &data_dir.to_string_lossy(),
        ])
        .stdout(log_file)
        .stderr(stderr_log)
        .stdin(std::process::Stdio::null())
        .spawn()?;

    // Write PID for later inspection / cleanup.
    fs::write(data_dir.join("daemon.pid"), child.id().to_string())?;

    // Poll health endpoint.
    let max_retries = 60;
    let delay = Duration::from_millis(200);
    for i in 0..max_retries {
        tokio::time::sleep(delay).await;
        if is_daemon_healthy(&base_url).await {
            tracing::info!("Daemon healthy after {} ms", (i + 1) * 200);
            return Ok(base_url);
        }
    }

    anyhow::bail!(
        "Daemon failed to become healthy within {}s. Check {}/daemon.log for details.",
        max_retries * 200 / 1000,
        data_dir.display()
    );
}
