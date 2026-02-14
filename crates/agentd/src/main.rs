use arcan_core::runtime::{Orchestrator, OrchestratorConfig, Provider, ToolRegistry};
use arcan_daemon::{mock::MockProvider, r#loop::AgentLoop, server::create_router};
use arcan_harness::edit::EditFileTool;
use arcan_harness::fs::{FsPolicy, GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use arcan_harness::memory::{ReadMemoryTool, WriteMemoryTool};
use arcan_harness::sandbox::{BashTool, LocalCommandRunner, NetworkPolicy, SandboxPolicy};
use arcan_lago::{LagoPolicyMiddleware, LagoSessionRepository};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use clap::Parser;
use lago_journal::RedbJournal;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "agentd", about = "Arcan agent daemon with Lago persistence")]
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        "Starting agentd"
    );

    let journal = RedbJournal::open(&journal_path)?;
    let _blob_store = lago_store::BlobStore::open(&blobs_path)?;
    let session_repo = Arc::new(LagoSessionRepository::new(Arc::new(journal)));

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

    // --- Provider ---
    let provider: Arc<dyn Provider> = match AnthropicConfig::from_env() {
        Ok(config) => {
            tracing::info!(model = %config.model, "Provider: Anthropic");
            Arc::new(AnthropicProvider::new(config))
        }
        Err(_) => {
            tracing::warn!("Provider: MockProvider (set ANTHROPIC_API_KEY for real LLM)");
            Arc::new(MockProvider)
        }
    };

    // --- Policy middleware ---
    let tool_annotations: std::collections::HashMap<String, _> = registry
        .definitions()
        .into_iter()
        .filter_map(|def| def.annotations.map(|ann| (def.name, ann)))
        .collect();
    let policy_engine = lago_policy::PolicyEngine::new();
    let policy_mw = LagoPolicyMiddleware::new(policy_engine, tool_annotations);
    let middlewares: Vec<Arc<dyn arcan_core::runtime::Middleware>> = vec![Arc::new(policy_mw)];

    // --- Orchestrator ---
    let config = OrchestratorConfig {
        max_iterations: cli.max_iterations,
    };
    let orchestrator = Arc::new(Orchestrator::new(provider, registry, middlewares, config));

    // --- Agent Loop ---
    let agent_loop = Arc::new(AgentLoop::new(session_repo, orchestrator));

    // --- HTTP Server ---
    let router = create_router(agent_loop).await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = TcpListener::bind(addr).await?;

    tracing::info!(%addr, "Listening");
    axum::serve(listener, router).await?;

    Ok(())
}
