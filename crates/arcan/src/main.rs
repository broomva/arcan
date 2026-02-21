use arcan_core::runtime::{Orchestrator, OrchestratorConfig, Provider, ToolRegistry};
use arcan_harness::edit::EditFileTool;
use arcan_harness::fs::{FsPolicy, GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use arcan_harness::memory::{ReadMemoryTool, WriteMemoryTool};
use arcan_harness::sandbox::{BashTool, LocalCommandRunner, NetworkPolicy, SandboxPolicy};
use arcan_lago::{
    ApprovalGate, LagoPolicyMiddleware, LagoSessionRepository, MemoryCommitTool, MemoryProjection,
    MemoryProposeTool, MemoryQueryTool,
};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcand::{r#loop::AgentLoop, mock::MockProvider, server::create_router_full};
use clap::Parser;
use lago_journal::RedbJournal;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "arcan",
    about = "Arcan agent runtime with streaming and tool execution"
)]
struct Cli {
    /// Data directory for journal and blob storage
    #[arg(long, default_value = ".arcan")]
    data_dir: PathBuf,

    /// HTTP listen port
    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Maximum orchestrator iterations per run
    #[arg(long, default_value_t = 10)]
    max_iterations: u32,

    /// Approval timeout in seconds (default 300 = 5 minutes)
    #[arg(long, default_value_t = 300)]
    approval_timeout: u64,
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

fn main() -> anyhow::Result<()> {
    // Structured logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let workspace_root = std::env::current_dir()?;
    let data_dir = if cli.data_dir.is_relative() {
        workspace_root.join(&cli.data_dir)
    } else {
        cli.data_dir.clone()
    };

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
    let _blob_store = lago_store::BlobStore::open(&blobs_path)?;
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);
    let session_repo = Arc::new(LagoSessionRepository::new(journal.clone()));

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

    // --- Approval gate ---
    let approval_gate = Arc::new(ApprovalGate::new(Duration::from_secs(cli.approval_timeout)));

    // --- Policy middleware ---
    let tool_annotations: std::collections::HashMap<String, _> = registry
        .definitions()
        .into_iter()
        .filter_map(|def| def.annotations.map(|ann| (def.name, ann)))
        .collect();
    let policy_engine = lago_policy::PolicyEngine::new();
    let policy_mw =
        LagoPolicyMiddleware::with_gate(policy_engine, tool_annotations, approval_gate.clone());
    let middlewares: Vec<Arc<dyn arcan_core::runtime::Middleware>> = vec![Arc::new(policy_mw)];

    // --- Orchestrator ---
    let config = OrchestratorConfig {
        max_iterations: cli.max_iterations,
        context: Some(arcan_core::context::ContextConfig::default()),
        context_compiler: None,
    };
    let orchestrator = Arc::new(Orchestrator::new(provider, registry, middlewares, config));

    // --- Agent Loop ---
    let agent_loop = Arc::new(AgentLoop::with_approval_gate(
        session_repo,
        orchestrator.clone(),
        approval_gate.clone(),
    ));

    // Build provider stack and blocking HTTP clients before entering Tokio runtime.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        // --- HTTP Server ---
        let resolver: Arc<dyn arcan_core::runtime::ApprovalResolver> = approval_gate;
        let router = create_router_full(agent_loop, orchestrator, Some(resolver)).await;
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cli.port));
        let listener = TcpListener::bind(addr).await?;

        tracing::info!(%addr, "Listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        tracing::info!("Server shut down gracefully");
        Ok(())
    })
}
