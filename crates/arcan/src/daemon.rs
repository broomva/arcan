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

/// Check if a PID file exists and the process is alive.
/// Returns `Some(pid)` if alive, `None` if dead or no PID file.
fn check_existing_pid(data_dir: &Path) -> Option<u32> {
    let pid_path = data_dir.join("daemon.pid");
    let raw = fs::read_to_string(&pid_path).ok()?;
    let pid: u32 = raw.trim().parse().ok()?;

    if is_process_alive(pid) {
        Some(pid)
    } else {
        tracing::warn!(pid, "Stale daemon.pid found (process dead), removing");
        let _ = fs::remove_file(&pid_path);
        None
    }
}

/// Check if a process with the given PID is alive using `kill -0`.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    // On non-Unix, we can't cheaply check — assume alive if PID file exists.
    true
}

/// Remove the daemon.pid file (called on graceful shutdown).
pub fn remove_pid_file(data_dir: &Path) {
    let pid_path = data_dir.join("daemon.pid");
    if pid_path.exists() {
        let _ = fs::remove_file(&pid_path);
        tracing::info!("Removed daemon.pid");
    }
}

/// Ensure a daemon is running on the given port.
///
/// 1. Probe `GET /health`. If healthy, return immediately.
/// 2. Check `daemon.pid` — if process alive but health fails, wait briefly.
/// 3. If PID is stale, remove PID file and proceed.
/// 4. Spawn `arcan serve` as a detached child process.
/// 5. Poll `/health` until it succeeds (up to ~12 s).
pub async fn ensure_daemon(data_dir: &Path, port: u16) -> anyhow::Result<String> {
    let base_url = format!("http://127.0.0.1:{port}");

    if is_daemon_healthy(&base_url).await {
        tracing::info!("Daemon already running on {base_url}");
        return Ok(base_url);
    }

    // Check PID file before spawning a new daemon.
    if let Some(pid) = check_existing_pid(data_dir) {
        // Process is alive but health check failed — it might be starting up.
        tracing::info!(pid, "Daemon process alive, waiting for health...");
        let delay = Duration::from_millis(200);
        for _ in 0..15 {
            tokio::time::sleep(delay).await;
            if is_daemon_healthy(&base_url).await {
                tracing::info!("Existing daemon became healthy on {base_url}");
                return Ok(base_url);
            }
        }
        // Still not healthy after 3s — the process might be stuck.
        tracing::warn!(
            pid,
            "Daemon process alive but not healthy after 3s, spawning new"
        );
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
