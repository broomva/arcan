use std::fs;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// Check whether a daemon is already healthy at the given URL.
async fn is_daemon_healthy(base_url: &str) -> bool {
    daemon_health(base_url).await.is_some()
}

/// Probe the daemon's `/health` endpoint.
/// Returns the reported version string on success, or `None` if unreachable.
async fn daemon_health(base_url: &str) -> Option<String> {
    let url = format!("{base_url}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    // Old daemons without a version field return None here — treated as outdated.
    body.get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// Check if a PID file exists and the process is alive.
/// Returns `Some(pid)` if alive, `None` if dead or no PID file.
pub fn check_existing_pid(data_dir: &Path) -> Option<u32> {
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

/// Check if a process with the given PID is alive using kill(pid, 0).
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    // Signal 0 doesn't deliver a signal, just checks if the process exists.
    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[cfg(not(unix))]
pub fn is_process_alive(_pid: u32) -> bool {
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

/// Stop a running daemon by sending SIGTERM and waiting for exit.
pub async fn stop_daemon(data_dir: &Path, port: u16) -> anyhow::Result<()> {
    let base_url = format!("http://127.0.0.1:{port}");

    // Check PID file first.
    let Some(pid) = check_existing_pid(data_dir) else {
        // No live process — check if the port is healthy anyway.
        if is_daemon_healthy(&base_url).await {
            anyhow::bail!(
                "Daemon is healthy on {base_url} but no PID file found. Stop it manually."
            );
        }
        anyhow::bail!("No running daemon found.");
    };

    // Send SIGTERM — uses nix for safe signal delivery, no subprocess spawn.
    #[cfg(unix)]
    {
        signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
            .map_err(|e| anyhow::anyhow!("Failed to send SIGTERM to PID {pid}: {e}"))?;
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!("stop is only supported on Unix systems");
    }

    // Poll until process is dead (up to 10s).
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !is_process_alive(pid) {
            remove_pid_file(data_dir);
            return Ok(());
        }
    }

    anyhow::bail!("Daemon (PID {pid}) did not exit within 10 seconds");
}

/// Ensure a daemon is running on the given port with a matching version.
///
/// 1. Probe `GET /health`. If healthy **and** version matches, return immediately.
/// 2. If healthy but version mismatches, stop the old daemon and respawn.
/// 3. Check `daemon.pid` — if process alive but health fails, wait briefly.
/// 4. If PID is stale, remove PID file and proceed.
/// 5. Spawn `arcan serve` as a detached child process.
/// 6. Poll `/health` until it succeeds (up to ~12 s).
pub async fn ensure_daemon(
    data_dir: &Path,
    port: u16,
    provider: Option<&str>,
    model: Option<&str>,
) -> anyhow::Result<String> {
    let base_url = format!("http://127.0.0.1:{port}");
    let current_version = env!("CARGO_PKG_VERSION");

    if let Some(daemon_version) = daemon_health(&base_url).await {
        if daemon_version == current_version {
            tracing::info!("Daemon already running on {base_url} (v{daemon_version})");
            return Ok(base_url);
        }
        // Version mismatch — restart the daemon.
        tracing::warn!(
            daemon_version,
            current_version,
            "Daemon version mismatch, restarting"
        );
        if let Err(e) = stop_daemon(data_dir, port).await {
            tracing::warn!("Failed to stop outdated daemon: {e}, spawning anyway");
        }
    } else if is_daemon_healthy(&base_url).await {
        // Healthy but no version field — old daemon without version support.
        tracing::warn!("Daemon running without version info, restarting");
        if let Err(e) = stop_daemon(data_dir, port).await {
            tracing::warn!("Failed to stop old daemon: {e}, spawning anyway");
        }
    }

    // Check PID file before spawning a new daemon.
    if let Some(pid) = check_existing_pid(data_dir) {
        // Process is alive but health check failed — it might be starting up.
        tracing::info!(pid, "Daemon process alive, waiting for health...");
        let delay = Duration::from_millis(200);
        for _ in 0..15 {
            tokio::time::sleep(delay).await;
            if let Some(v) = daemon_health(&base_url).await {
                if v == current_version {
                    tracing::info!("Existing daemon became healthy on {base_url} (v{v})");
                    return Ok(base_url);
                }
            }
        }
        // Still not healthy after 3s — the process might be stuck.
        tracing::warn!(
            pid,
            "Daemon process alive but not healthy/current after 3s, spawning new"
        );
    }

    tracing::info!("Starting daemon on port {port}...");

    fs::create_dir_all(data_dir)?;
    let log_file = fs::File::create(data_dir.join("daemon.log"))?;
    let stderr_log = log_file.try_clone()?;

    // Resolve the current executable so `arcan serve` uses the same binary.
    let exe = std::env::current_exe()?;

    let port_str = port.to_string();
    let dir_str = data_dir.to_string_lossy().to_string();
    let mut args = vec!["serve", "--port", &port_str, "--data-dir", &dir_str];
    if let Some(p) = provider {
        args.extend(["--provider", p]);
    }
    if let Some(m) = model {
        args.extend(["--model", m]);
    }

    let child = std::process::Command::new(exe)
        .args(&args)
        .stdout(log_file)
        .stderr(stderr_log)
        .stdin(std::process::Stdio::null())
        .spawn()?;

    // Write PID atomically (write tmp → rename) to avoid readers seeing partial content.
    let child_pid = child.id();
    let pid_path = data_dir.join("daemon.pid");
    let tmp_path = data_dir.join("daemon.pid.tmp");
    if let Err(e) =
        fs::write(&tmp_path, child_pid.to_string()).and_then(|()| fs::rename(&tmp_path, &pid_path))
    {
        tracing::error!(child_pid, "Failed to write PID file, killing orphan: {e}");
        #[cfg(unix)]
        let _ = signal::kill(Pid::from_raw(child_pid as i32), Signal::SIGKILL);
        // Clean up temp file if rename failed but write succeeded.
        let _ = fs::remove_file(&tmp_path);
        anyhow::bail!("Failed to write daemon PID file: {e}");
    }
    // Child handle is intentionally dropped — the daemon runs detached.
    drop(child);

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
