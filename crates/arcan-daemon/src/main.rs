use arcan_core::runtime::{Orchestrator, OrchestratorConfig, Provider, ToolRegistry};
use arcan_daemon::{mock::MockProvider, r#loop::AgentLoop, server::create_router};
use arcan_harness::edit::EditFileTool;
use arcan_harness::fs::{FsPolicy, GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use arcan_harness::memory::{ReadMemoryTool, WriteMemoryTool};
use arcan_harness::sandbox::{BashTool, LocalCommandRunner, NetworkPolicy, SandboxPolicy};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcan_store::session::JsonlSessionRepository;
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Setup paths
    let workspace_root = std::env::current_dir()?;
    let session_root = workspace_root.join(".arcan/sessions");
    std::fs::create_dir_all(&session_root)?;

    println!("Starting arcan-daemon...");
    println!("Workspace: {}", workspace_root.display());
    println!("Sessions: {}", session_root.display());

    // 2. Initialize Policies
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

    // 3. Initialize Tools
    let mut registry = ToolRegistry::default();
    registry.register(ReadFileTool::new(fs_policy.clone()));
    registry.register(WriteFileTool::new(fs_policy.clone()));
    registry.register(ListDirTool::new(fs_policy.clone()));
    registry.register(EditFileTool::new(fs_policy.clone()));

    registry.register(GlobTool::new(fs_policy.clone()));
    registry.register(GrepTool::new(fs_policy));

    let runner = Box::new(LocalCommandRunner);
    registry.register(BashTool::new(sandbox_policy, runner));

    // Memory tools
    let memory_dir = workspace_root.join(".arcan/memory");
    registry.register(ReadMemoryTool::new(memory_dir.clone()));
    registry.register(WriteMemoryTool::new(memory_dir));

    // 4. Initialize Provider — use Anthropic if API key is set, otherwise MockProvider
    let provider: Arc<dyn Provider> = match AnthropicConfig::from_env() {
        Ok(config) => {
            println!("Provider: Anthropic ({})", config.model);
            Arc::new(AnthropicProvider::new(config))
        }
        Err(_) => {
            println!("Provider: MockProvider (set ANTHROPIC_API_KEY for real LLM)");
            Arc::new(MockProvider)
        }
    };

    // 5. Initialize Orchestrator
    let config = OrchestratorConfig { max_iterations: 10 };
    let orchestrator = Arc::new(Orchestrator::new(provider, registry, vec![], config));

    // 6. Initialize Session Repo
    let session_repo = Arc::new(JsonlSessionRepository::new(session_root));

    // 7. Initialize Agent Loop
    let agent_loop = Arc::new(AgentLoop::new(session_repo, orchestrator));

    // 8. Start Server
    let router = create_router(agent_loop).await;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;

    println!("Listening on http://{}", addr);
    axum::serve(listener, router).await?;

    Ok(())
}
