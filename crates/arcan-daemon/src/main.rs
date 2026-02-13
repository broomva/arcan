use arcan_core::runtime::{Orchestrator, OrchestratorConfig, ToolRegistry};
use arcan_daemon::{mock::MockProvider, r#loop::AgentLoop, server::create_router};
use arcan_harness::fs::{FsPolicy, ReadFileTool, WriteFileTool, ListDirTool};
use arcan_harness::edit::EditFileTool;
use arcan_harness::sandbox::{LocalCommandRunner, BashTool, SandboxPolicy, NetworkPolicy};
use arcan_store::session::JsonlSessionRepository;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::path::PathBuf;
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
        network: NetworkPolicy::AllowAll, // For MVP
        allowed_env: BTreeSet::new(), // allow all by default if empty? No, need to check policy implementation
        max_execution_ms: 10000,
        max_output_bytes: 1024 * 1024,
        max_memory_mb: 512,
        max_processes: 10,
    };

    // 3. Initialize Tools
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadFileTool::new(fs_policy.clone())));
    registry.register(Arc::new(WriteFileTool::new(fs_policy.clone())));
    registry.register(Arc::new(ListDirTool::new(fs_policy.clone())));
    registry.register(Arc::new(EditFileTool::new(fs_policy.clone())));
    
    let runner = Box::new(LocalCommandRunner);
    registry.register(Arc::new(BashTool::new(sandbox_policy, runner)));

    // 4. Initialize Provider (Mock for now)
    let provider = Arc::new(MockProvider);

    // 5. Initialize Orchestrator
    let config = OrchestratorConfig {
        max_iterations: 10,
    };
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
