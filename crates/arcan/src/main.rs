mod daemon;

use aios_protocol::{
    ApprovalPort, EventStorePort, ModelProviderPort, PolicyGatePort, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use arcan_aios_adapters::{
    ArcanApprovalAdapter, ArcanHarnessAdapter, ArcanPolicyAdapter, ArcanProviderAdapter,
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
    #[arg(long, default_value_t = 3000, global = true)]
    port: u16,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon in foreground
    Serve {
        /// Maximum orchestrator iterations per run
        #[arg(long, default_value_t = 10)]
        max_iterations: u32,

        /// Approval timeout in seconds (default 300 = 5 minutes)
        #[arg(long, default_value_t = 300)]
        approval_timeout: u64,
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

async fn most_recent_session_from_journal(data_dir: &Path) -> anyhow::Result<Option<String>> {
    let journal_path = data_dir.join("journal.redb");
    if !journal_path.exists() {
        return Ok(None);
    }

    let journal = RedbJournal::open(&journal_path)?;

    // Preferred path when sessions are indexed in Lago.
    let sessions = journal.list_sessions().await?;
    if let Some(session) = sessions.into_iter().max_by_key(|entry| entry.created_at) {
        return Ok(Some(session.session_id.to_string()));
    }

    // Fallback for runtime paths that only append events.
    let events = journal.read(EventQuery::new()).await?;
    Ok(events
        .into_iter()
        .max_by_key(|event| event.timestamp)
        .map(|event| event.session_id.to_string()))
}

async fn resolve_chat_session(data_dir: &Path, requested: Option<String>) -> String {
    if let Some(session) = requested {
        return session;
    }

    if let Some(session) = read_last_session_hint(data_dir) {
        return session;
    }

    match most_recent_session_from_journal(data_dir).await {
        Ok(Some(session)) => return session,
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "failed to detect most recent session from journal"),
    }

    "default".to_owned()
}

fn run_serve(
    data_dir: &Path,
    port: u16,
    _max_iterations: u32,
    _approval_timeout: u64,
) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;

    // --- Lago persistence ---
    let journal_path = data_dir.join("journal.redb");
    let blobs_path = data_dir.join("blobs");
    std::fs::create_dir_all(&blobs_path)?;

    tracing::info!(
        workspace = %workspace_root.display(),
        journal = %journal_path.display(),
        blobs = %blobs_path.display(),
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
    // Selection order: ARCAN_PROVIDER env var > auto-detect from API keys > MockProvider
    let provider_name = std::env::var("ARCAN_PROVIDER").unwrap_or_default();
    let provider: Arc<dyn Provider> = match provider_name.as_str() {
        "mock" => {
            tracing::warn!("Provider: MockProvider (forced via ARCAN_PROVIDER=mock)");
            Arc::new(MockProvider)
        }
        "openai" => match arcan_provider::openai::OpenAiConfig::openai_from_env() {
            Ok(config) => {
                tracing::info!(model = %config.model, "Provider: OpenAI");
                Arc::new(arcan_provider::openai::OpenAiCompatibleProvider::new(
                    config,
                ))
            }
            Err(e) => {
                tracing::error!("ARCAN_PROVIDER=openai but config failed: {e}");
                return Err(e.into());
            }
        },
        "ollama" => match arcan_provider::openai::OpenAiConfig::ollama_from_env() {
            Ok(config) => {
                tracing::info!(model = %config.model, base_url = %config.base_url, "Provider: Ollama");
                Arc::new(arcan_provider::openai::OpenAiCompatibleProvider::new(
                    config,
                ))
            }
            Err(e) => {
                tracing::error!("ARCAN_PROVIDER=ollama but config failed: {e}");
                return Err(e.into());
            }
        },
        "anthropic" => match AnthropicConfig::from_env() {
            Ok(config) => {
                tracing::info!(model = %config.model, "Provider: Anthropic");
                Arc::new(AnthropicProvider::new(config))
            }
            Err(e) => {
                tracing::error!("ARCAN_PROVIDER=anthropic but config failed: {e}");
                return Err(e.into());
            }
        },
        // Auto-detect: try providers in order
        _ => {
            if let Ok(config) = AnthropicConfig::from_env() {
                tracing::info!(model = %config.model, "Provider: Anthropic (auto-detected)");
                Arc::new(AnthropicProvider::new(config))
            } else if let Ok(config) = arcan_provider::openai::OpenAiConfig::openai_from_env() {
                tracing::info!(model = %config.model, "Provider: OpenAI (auto-detected)");
                Arc::new(arcan_provider::openai::OpenAiCompatibleProvider::new(
                    config,
                ))
            } else {
                tracing::warn!(
                    "Provider: MockProvider (set ARCAN_PROVIDER or API key env vars for real LLM)"
                );
                Arc::new(MockProvider)
            }
        }
    };

    // --- Canonical aiOS runtime adapters ---
    let event_store: Arc<dyn EventStorePort> =
        Arc::new(LagoAiosEventStoreAdapter::new(journal.clone()));
    let provider_adapter: Arc<dyn ModelProviderPort> =
        Arc::new(ArcanProviderAdapter::new(provider, registry.definitions()));
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

    // Build provider stack and blocking HTTP clients before entering Tokio runtime.
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

        tracing::info!("Server shut down gracefully");
        Ok(())
    })
}

async fn run_chat(
    data_dir: PathBuf,
    port: u16,
    session: Option<String>,
    url: Option<String>,
) -> anyhow::Result<()> {
    let session = resolve_chat_session(&data_dir, session).await;
    tracing::info!(session = %session, "launching TUI");
    let base_url = match url {
        Some(u) => u,
        None => daemon::ensure_daemon(&data_dir, port).await?,
    };

    if let Err(error) = persist_last_session_hint(&data_dir, &session) {
        tracing::warn!(%error, "failed to persist last_session hint");
    }

    arcan_tui::run_tui(base_url, session).await
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(&cli.data_dir)?;

    match cli.command {
        Some(Command::Serve {
            max_iterations,
            approval_timeout,
        }) => {
            // Structured logging to stderr for daemon mode
            tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::from_default_env())
                .init();

            run_serve(&data_dir, cli.port, max_iterations, approval_timeout)
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

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_chat(data_dir, cli.port, session, url))
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

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_chat(data_dir, cli.port, None, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::{
        BranchId as LagoBranchId, EventEnvelope, EventId as LagoEventId,
        EventPayload as LagoEventPayload, SessionId as LagoSessionId,
    };
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
    async fn resolve_chat_session_prefers_explicit_session() {
        let dir = unique_temp_dir("arcan-resolve-explicit");
        persist_last_session_hint(&dir, "hint-session").expect("persist hint");
        let selected = resolve_chat_session(&dir, Some("explicit-session".to_owned())).await;
        assert_eq!(selected, "explicit-session");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolve_chat_session_uses_last_session_hint() {
        let dir = unique_temp_dir("arcan-resolve-hint");
        persist_last_session_hint(&dir, "hint-session").expect("persist hint");
        let selected = resolve_chat_session(&dir, None).await;
        assert_eq!(selected, "hint-session");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn most_recent_session_falls_back_to_latest_event_timestamp() {
        let dir = unique_temp_dir("arcan-resolve-journal");
        let journal_path = dir.join("journal.redb");
        let journal = RedbJournal::open(&journal_path).expect("open journal");

        let mk_event = |session: &str, timestamp: u64| EventEnvelope {
            event_id: LagoEventId::new(),
            session_id: LagoSessionId::from_string(session),
            branch_id: LagoBranchId::from_string("main"),
            run_id: None,
            seq: 0,
            timestamp,
            parent_id: None,
            payload: LagoEventPayload::Message {
                role: "user".to_owned(),
                content: "hello".to_owned(),
                model: None,
                token_usage: None,
            },
            metadata: std::collections::HashMap::new(),
            schema_version: 1,
        };

        journal
            .append(mk_event("session-old", 1_000))
            .await
            .expect("append old event");
        journal
            .append(mk_event("session-new", 2_000))
            .await
            .expect("append new event");
        drop(journal);

        let selected = most_recent_session_from_journal(&dir)
            .await
            .expect("resolve most recent session");
        assert_eq!(selected.as_deref(), Some("session-new"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
