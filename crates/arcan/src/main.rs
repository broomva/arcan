mod cli_run;
mod config;
mod daemon;

use aios_protocol::{
    ApprovalPort, EventStorePort, ModelProviderPort, PolicyGatePort, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use arcan_aios_adapters::{
    ArcanApprovalAdapter, ArcanHarnessAdapter, ArcanPolicyAdapter, ArcanProviderAdapter,
    StreamingSenderHandle,
};
use arcan_core::runtime::{Provider, ToolRegistry};
use arcan_harness::edit::EditFileTool;
use arcan_harness::fs::{FsPolicy, GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use arcan_harness::memory::{ReadMemoryTool, WriteMemoryTool};
use arcan_harness::sandbox::{BashTool, LocalCommandRunner, NetworkPolicy, SandboxPolicy};
use arcan_lago::{MemoryCommitTool, MemoryProjection, MemoryProposeTool, MemoryQueryTool};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcand::{canonical::create_canonical_router, mock::MockProvider};
use clap::{Parser, Subcommand};
use config::ResolvedConfig;
use lago_aios_eventstore_adapter::LagoAiosEventStoreAdapter;
use lago_core::{
    BranchId, EventEnvelope, EventId, EventPayload, EventQuery, Journal, Projection, SessionId,
};
use lago_fs::{DiffEntry, ManifestProjection};
use lago_journal::RedbJournal;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

struct LakeFsObserver {
    workspace_root: PathBuf,
    journal: Arc<dyn Journal>,
    blob_store: Arc<lago_store::BlobStore>,
}

#[async_trait::async_trait]
impl arcan_aios_adapters::tools::ToolHarnessObserver for LakeFsObserver {
    async fn post_execute(&self, session_id: String, _tool_name: String) {
        // We skip memory tools since they don't modify the FS directly, and bash since we might want more granular control, but for now we trace everything.
        let sess_id = SessionId::from_string(session_id.clone());
        let branch_id = BranchId::from_string("main");

        let events = match self
            .journal
            .read(EventQuery::new().session(lago_core::SessionId::from_string(session_id)))
            .await
        {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!(%e, "LakeFsObserver: failed to read journal");
                return;
            }
        };

        let mut proj = ManifestProjection::new();
        for e in events {
            if e.branch_id == branch_id {
                let _ = proj.on_event(&e);
            }
        }

        let current_manifest = proj.manifest().clone();

        let new_manifest =
            match lago_fs::snapshot(&self.workspace_root, &current_manifest, &self.blob_store) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(%e, "LakeFsObserver: failed to snapshot workspace");
                    return;
                }
            };

        let diffs = lago_fs::diff(&current_manifest, &new_manifest);
        if diffs.is_empty() {
            return;
        }

        tracing::info!(
            diff_count = diffs.len(),
            "LakeFsObserver: workspace changed, emitting events"
        );

        for diff in diffs {
            let payload = match diff {
                DiffEntry::Added { path, entry } => EventPayload::FileWrite {
                    path,
                    blob_hash: entry.blob_hash.into(),
                    size_bytes: entry.size_bytes,
                    content_type: entry.content_type,
                },
                DiffEntry::Modified {
                    path, new: entry, ..
                } => EventPayload::FileWrite {
                    path,
                    blob_hash: entry.blob_hash.into(),
                    size_bytes: entry.size_bytes,
                    content_type: entry.content_type,
                },
                DiffEntry::Removed { path, .. } => EventPayload::FileDelete { path },
            };

            let envelope = EventEnvelope {
                event_id: EventId::new(),
                session_id: sess_id.clone(),
                branch_id: branch_id.clone(),
                run_id: None,
                seq: 0,
                timestamp: EventEnvelope::now_micros(),
                parent_id: None,
                payload,
                metadata: std::collections::HashMap::new(),
                schema_version: 1,
            };

            if let Err(e) = self.journal.append(envelope).await {
                tracing::warn!(%e, "LakeFsObserver: failed to append event");
            }
        }
    }
}

#[derive(Parser)]
#[command(
    name = "arcan",
    about = "Arcan agent runtime with streaming and tool execution"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Data directory for journal and blob storage
    #[arg(long, default_value = ".arcan", global = true)]
    data_dir: PathBuf,

    /// HTTP listen port
    #[arg(long, global = true)]
    port: Option<u16>,

    /// LLM provider (anthropic, openai, ollama, mock)
    #[arg(long, global = true)]
    provider: Option<String>,

    /// Model name override
    #[arg(long, global = true)]
    model: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon in foreground
    Serve {
        /// Maximum orchestrator iterations per run
        #[arg(long)]
        max_iterations: Option<u32>,

        /// Approval timeout in seconds (default 300 = 5 minutes)
        #[arg(long)]
        approval_timeout: Option<u64>,
    },
    /// Launch the TUI client (auto-starts daemon if needed)
    Chat {
        /// Session ID to attach to (defaults to most recent session)
        #[arg(short, long)]
        session: Option<String>,

        /// Daemon URL (skip auto-start, connect to existing)
        #[arg(long)]
        url: Option<String>,
    },
    /// Send a single message and print the response (non-interactive)
    Run {
        /// The message to send
        message: String,

        /// Session ID (created if it doesn't exist)
        #[arg(short, long)]
        session: Option<String>,

        /// Daemon URL (skip auto-start, connect to existing)
        #[arg(long)]
        url: Option<String>,

        /// Output raw JSON events (one per line) instead of formatted text
        #[arg(long)]
        json: bool,
    },
    /// Manage persistent configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show daemon status, provider, model, and session info
    Status,
    /// Stop the running daemon
    Stop,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set a config value (e.g., `arcan config set provider ollama`)
    Set {
        /// Config key (provider, model, port, or dotted path like providers.ollama.base_url)
        key: String,
        /// Value to set
        value: String,
    },
    /// Get a config value
    Get {
        /// Config key
        key: String,
    },
    /// List all configuration
    List,
    /// Initialize a default config file
    Init,
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {},
            _ = sigterm.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
    }

    tracing::info!("Received shutdown signal, draining connections...");
}

fn resolve_data_dir(data_dir: &PathBuf) -> anyhow::Result<PathBuf> {
    let workspace_root = std::env::current_dir()?;
    Ok(if data_dir.is_relative() {
        workspace_root.join(data_dir)
    } else {
        data_dir.clone()
    })
}

fn read_last_session_hint(data_dir: &Path) -> Option<String> {
    let path = data_dir.join("last_session");
    let raw = std::fs::read_to_string(path).ok()?;
    let session = raw.trim();
    if session.is_empty() {
        None
    } else {
        Some(session.to_owned())
    }
}

fn persist_last_session_hint(data_dir: &Path, session: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join("last_session"), session)?;
    Ok(())
}

/// Resolve session via the daemon's HTTP API first, falling back to local hints.
/// This avoids opening the journal (which would conflict with the daemon's redb lock).
async fn resolve_session(
    data_dir: &Path,
    requested: Option<String>,
    base_url: Option<&str>,
) -> String {
    if let Some(session) = requested {
        return session;
    }

    if let Some(session) = read_last_session_hint(data_dir) {
        return session;
    }

    // Try the HTTP API if a daemon URL is available (avoids redb lock conflict).
    if let Some(url) = base_url {
        if let Some(session) = cli_run::resolve_session_via_api(url).await {
            return session;
        }
    }

    "default".to_owned()
}

/// Build provider from resolved configuration.
fn build_provider(resolved: &ResolvedConfig) -> anyhow::Result<Arc<dyn Provider>> {
    let pc = resolved.provider_config.as_ref();

    match resolved.provider.as_str() {
        "mock" => {
            tracing::warn!("Provider: MockProvider (forced via config)");
            Ok(Arc::new(MockProvider))
        }
        "openai" => {
            let config = arcan_provider::openai::OpenAiConfig::openai_from_resolved(
                resolved.model.as_deref(),
                pc.and_then(|p| p.base_url.as_deref()),
                pc.and_then(|p| p.max_tokens),
            )?;
            tracing::info!(model = %config.model, "Provider: OpenAI");
            Ok(Arc::new(
                arcan_provider::openai::OpenAiCompatibleProvider::new(config),
            ))
        }
        "ollama" => {
            let config = arcan_provider::openai::OpenAiConfig::ollama_from_resolved(
                resolved.model.as_deref(),
                pc.and_then(|p| p.base_url.as_deref()),
                pc.and_then(|p| p.max_tokens),
                pc.and_then(|p| p.enable_streaming),
            )?;
            tracing::info!(model = %config.model, base_url = %config.base_url, "Provider: Ollama");
            Ok(Arc::new(
                arcan_provider::openai::OpenAiCompatibleProvider::new(config),
            ))
        }
        "anthropic" => {
            let config = AnthropicConfig::from_resolved(
                resolved.model.as_deref(),
                pc.and_then(|p| p.base_url.as_deref()),
                pc.and_then(|p| p.max_tokens),
            )?;
            tracing::info!(model = %config.model, "Provider: Anthropic");
            Ok(Arc::new(AnthropicProvider::new(config)))
        }
        // Auto-detect: try providers in order
        _ => {
            if let Ok(config) = AnthropicConfig::from_env() {
                tracing::info!(model = %config.model, "Provider: Anthropic (auto-detected)");
                Ok(Arc::new(AnthropicProvider::new(config)))
            } else if let Ok(config) = arcan_provider::openai::OpenAiConfig::openai_from_env() {
                tracing::info!(model = %config.model, "Provider: OpenAI (auto-detected)");
                Ok(Arc::new(
                    arcan_provider::openai::OpenAiCompatibleProvider::new(config),
                ))
            } else {
                tracing::warn!(
                    "Provider: MockProvider (set ARCAN_PROVIDER or API key env vars for real LLM)"
                );
                Ok(Arc::new(MockProvider))
            }
        }
    }
}

fn run_serve(data_dir: &Path, resolved: &ResolvedConfig) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;

    // --- Lago persistence ---
    let journal_path = data_dir.join("journal.redb");
    let blobs_path = data_dir.join("blobs");
    std::fs::create_dir_all(&blobs_path)?;

    tracing::info!(
        workspace = %workspace_root.display(),
        journal = %journal_path.display(),
        blobs = %blobs_path.display(),
        provider = %resolved.provider,
        model = ?resolved.model,
        port = resolved.port,
        "Starting arcan"
    );

    let journal = RedbJournal::open(&journal_path)?;
    let _blob_store = Arc::new(lago_store::BlobStore::open(&blobs_path)?);
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // --- Policies ---
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let sandbox_policy = SandboxPolicy {
        workspace_root: workspace_root.clone(),
        shell_enabled: true,
        network: NetworkPolicy::AllowAll,
        allowed_env: BTreeSet::new(),
        max_execution_ms: 10_000,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
        max_processes: 10,
        max_memory_mb: 512,
    };

    // --- Tools ---
    let mut registry = ToolRegistry::default();
    registry.register(ReadFileTool::new(fs_policy.clone()));
    registry.register(WriteFileTool::new(fs_policy.clone()));
    registry.register(ListDirTool::new(fs_policy.clone()));
    registry.register(EditFileTool::new(fs_policy.clone()));
    registry.register(GlobTool::new(fs_policy.clone()));
    registry.register(GrepTool::new(fs_policy));

    let runner = Box::new(LocalCommandRunner);
    registry.register(BashTool::new(sandbox_policy, runner));

    let memory_dir = data_dir.join("memory");
    std::fs::create_dir_all(&memory_dir)?;
    registry.register(ReadMemoryTool::new(memory_dir.clone()));
    registry.register(WriteMemoryTool::new(memory_dir));

    // --- Governed memory tools (event-sourced via Lago) ---
    let memory_projection = Arc::new(RwLock::new(MemoryProjection::new()));
    registry.register(MemoryQueryTool::new(memory_projection));
    registry.register(MemoryProposeTool::new(journal.clone()));
    registry.register(MemoryCommitTool::new(journal.clone()));

    // --- Provider ---
    let provider = build_provider(resolved)?;

    // --- Canonical aiOS runtime adapters ---
    let event_store: Arc<dyn EventStorePort> =
        Arc::new(LagoAiosEventStoreAdapter::new(journal.clone()));
    // Shared handle: starts empty, filled after runtime creation.
    let streaming_sender: StreamingSenderHandle = Arc::new(std::sync::Mutex::new(None));
    let provider_adapter: Arc<dyn ModelProviderPort> = Arc::new(ArcanProviderAdapter::new(
        provider,
        registry.definitions(),
        streaming_sender.clone(),
    ));
    let observer = Arc::new(LakeFsObserver {
        workspace_root: workspace_root.clone(),
        journal: journal.clone(),
        blob_store: _blob_store,
    });
    let tool_harness: Arc<dyn ToolHarnessPort> =
        Arc::new(ArcanHarnessAdapter::new(registry).with_observer(observer));
    let policy_gate: Arc<dyn PolicyGatePort> =
        Arc::new(ArcanPolicyAdapter::new(aios_protocol::PolicySet::default()));
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ArcanApprovalAdapter::new());

    let runtime = Arc::new(KernelRuntime::new(
        RuntimeConfig::new(data_dir.to_path_buf()),
        event_store,
        provider_adapter,
        tool_harness,
        approvals,
        policy_gate,
    ));

    // Wire the broadcast sender now that the runtime exists.
    *streaming_sender.lock().unwrap() = Some(runtime.event_sender());

    // Build provider stack and blocking HTTP clients before entering Tokio runtime.
    let data_dir_owned = data_dir.to_path_buf();
    let port = resolved.port;
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    tokio_runtime.block_on(async move {
        // --- HTTP Server ---
        let router = create_canonical_router(runtime);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr).await?;

        tracing::info!(%addr, "Listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        // Clean up PID file on graceful shutdown.
        daemon::remove_pid_file(&data_dir_owned);

        tracing::info!("Server shut down gracefully");
        Ok(())
    })
}

async fn run_chat(
    data_dir: PathBuf,
    resolved: &ResolvedConfig,
    session: Option<String>,
    url: Option<String>,
) -> anyhow::Result<()> {
    // Ensure daemon is running first to avoid redb lock conflicts.
    let base_url = match url {
        Some(u) => u,
        None => {
            daemon::ensure_daemon(
                &data_dir,
                resolved.port,
                Some(resolved.provider.as_str()).filter(|s| !s.is_empty()),
                resolved.model.as_deref(),
            )
            .await?
        }
    };

    // Resolve session via API (no direct journal access).
    let session = resolve_session(&data_dir, session, Some(&base_url)).await;
    tracing::info!(session = %session, "launching TUI");

    if let Err(error) = persist_last_session_hint(&data_dir, &session) {
        tracing::warn!(%error, "failed to persist last_session hint");
    }

    arcan_tui::run_tui(base_url, session).await
}

async fn run_message(
    data_dir: PathBuf,
    resolved: &ResolvedConfig,
    message: String,
    session: Option<String>,
    url: Option<String>,
    json_output: bool,
) -> anyhow::Result<()> {
    // Ensure daemon is running first.
    let base_url = match url {
        Some(u) => u,
        None => {
            daemon::ensure_daemon(
                &data_dir,
                resolved.port,
                Some(resolved.provider.as_str()).filter(|s| !s.is_empty()),
                resolved.model.as_deref(),
            )
            .await?
        }
    };

    // Resolve session via API (no direct journal access).
    let session = resolve_session(&data_dir, session, Some(&base_url)).await;

    if let Err(error) = persist_last_session_hint(&data_dir, &session) {
        tracing::warn!(%error, "failed to persist last_session hint");
    }

    let exit_code = cli_run::run_cli(
        &base_url,
        &session,
        &message,
        json_output,
        resolved.model.as_deref(),
    )
    .await?;
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

#[allow(clippy::print_stdout)]
fn run_config(data_dir: &Path, action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Init => {
            let path = config::local_config_path(data_dir);
            if path.exists() {
                println!("Config file already exists: {}", path.display());
            } else {
                std::fs::create_dir_all(data_dir)?;
                std::fs::write(&path, config::default_config_content())?;
                println!("Created config: {}", path.display());
            }
        }
        ConfigAction::Set { key, value } => {
            let mut cfg = config::load_config(data_dir);
            cfg.set_key(&key, &value)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            config::save_config(data_dir, &cfg)?;
            println!("{key} = {value}");
        }
        ConfigAction::Get { key } => {
            let cfg = config::load_config(data_dir);
            match cfg.get_key(&key) {
                Some(value) => println!("{value}"),
                None => println!("(not set)"),
            }
        }
        ConfigAction::List => {
            let cfg = config::load_config(data_dir);
            let content = toml::to_string_pretty(&cfg)
                .map_err(|e| anyhow::anyhow!("failed to serialize config: {e}"))?;
            if content.trim().is_empty()
                || content.trim() == "[defaults]\n\n[agent]"
                || content
                    .lines()
                    .all(|l| l.trim().is_empty() || l.starts_with('['))
            {
                println!("(no config values set)");
                if let Some(path) = config::global_config_path() {
                    println!("Global config: {}", path.display());
                }
                println!(
                    "Local config:  {}",
                    config::local_config_path(data_dir).display()
                );
            } else {
                print!("{content}");
            }
        }
    }
    Ok(())
}

#[allow(clippy::print_stdout)]
async fn run_status(data_dir: &Path, resolved: &ResolvedConfig) -> anyhow::Result<()> {
    println!("Arcan Status");
    println!("============");

    // Config
    println!(
        "Provider:   {}",
        if resolved.provider.is_empty() {
            "(auto-detect)"
        } else {
            &resolved.provider
        }
    );
    println!(
        "Model:      {}",
        resolved.model.as_deref().unwrap_or("(provider default)")
    );
    println!("Port:       {}", resolved.port);
    println!("Data dir:   {}", data_dir.display());

    // Daemon status
    let base_url = format!("http://127.0.0.1:{}", resolved.port);
    match daemon::check_existing_pid(data_dir) {
        Some(pid) => {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()?;
            let healthy = matches!(
                client.get(format!("{base_url}/health")).send().await,
                Ok(resp) if resp.status().is_success()
            );
            if healthy {
                println!("Daemon:     running (PID {pid}, healthy)");
            } else {
                println!("Daemon:     running (PID {pid}, NOT healthy)");
            }
        }
        None => {
            println!("Daemon:     not running");
        }
    }

    // Last session
    match read_last_session_hint(data_dir) {
        Some(session) => println!("Session:    {session}"),
        None => println!("Session:    (none)"),
    }

    // Config file locations
    if let Some(global_path) = config::global_config_path() {
        let exists = global_path.exists();
        println!(
            "Global cfg: {} {}",
            global_path.display(),
            if exists { "" } else { "(not found)" }
        );
    }
    let local_path = config::local_config_path(data_dir);
    let exists = local_path.exists();
    println!(
        "Local cfg:  {} {}",
        local_path.display(),
        if exists { "" } else { "(not found)" }
    );

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(&cli.data_dir)?;

    // Load layered config.
    let file_config = config::load_config(&data_dir);

    match cli.command {
        Some(Command::Serve {
            max_iterations,
            approval_timeout,
        }) => {
            // Structured logging to stderr for daemon mode
            tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::from_default_env())
                .init();

            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                max_iterations,
                approval_timeout,
            );

            run_serve(&data_dir, &resolved)
        }
        Some(Command::Chat { session, url }) => {
            // File-based logging for TUI mode (don't clobber the terminal)
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            let file_appender = tracing_appender::rolling::never(&log_dir, "tui.log");
            tracing_subscriber::fmt()
                .with_writer(file_appender)
                .with_env_filter(EnvFilter::from_default_env())
                .init();

            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_chat(data_dir, &resolved, session, url))
        }
        Some(Command::Run {
            message,
            session,
            url,
            json,
        }) => {
            // File-based logging for CLI mode (don't clobber stdout)
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            let file_appender = tracing_appender::rolling::never(&log_dir, "run.log");
            tracing_subscriber::fmt()
                .with_writer(file_appender)
                .with_env_filter(EnvFilter::from_default_env())
                .init();

            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_message(
                data_dir, &resolved, message, session, url, json,
            ))
        }
        Some(Command::Config { action }) => run_config(&data_dir, action),
        Some(Command::Status) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_status(&data_dir, &resolved))
        }
        Some(Command::Stop) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(daemon::stop_daemon(&data_dir, resolved.port))
        }
        None => {
            // Default: launch TUI with auto-daemon (same as `arcan chat`)
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            let file_appender = tracing_appender::rolling::never(&log_dir, "tui.log");
            tracing_subscriber::fmt()
                .with_writer(file_appender)
                .with_env_filter(EnvFilter::from_default_env())
                .init();

            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_chat(data_dir, &resolved, None, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn last_session_hint_roundtrip() {
        let dir = unique_temp_dir("arcan-last-session-hint");
        persist_last_session_hint(&dir, "session-42").expect("persist hint");
        assert_eq!(read_last_session_hint(&dir).as_deref(), Some("session-42"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolve_session_prefers_explicit() {
        let dir = unique_temp_dir("arcan-resolve-explicit");
        persist_last_session_hint(&dir, "hint-session").expect("persist hint");
        let selected = resolve_session(&dir, Some("explicit-session".to_owned()), None).await;
        assert_eq!(selected, "explicit-session");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolve_session_uses_last_session_hint() {
        let dir = unique_temp_dir("arcan-resolve-hint");
        persist_last_session_hint(&dir, "hint-session").expect("persist hint");
        let selected = resolve_session(&dir, None, None).await;
        assert_eq!(selected, "hint-session");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolve_session_defaults_when_no_hint() {
        let dir = unique_temp_dir("arcan-resolve-default");
        let selected = resolve_session(&dir, None, None).await;
        assert_eq!(selected, "default");
        let _ = std::fs::remove_dir_all(dir);
    }
}
