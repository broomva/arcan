mod cli_run;
mod config;
mod consolidator;
mod daemon;
mod embedding;
mod ephemeral_journal;
mod factory;
mod markdown;
mod memory_tools;
mod nous_observer;
mod prompt;
mod sandbox_router;
mod shell;
mod skills;
mod spinner;

use aios_protocol::sandbox::NetworkPolicy;
use aios_protocol::{
    ApprovalPort, EventStorePort, ModelProviderPort, PolicyGatePort, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use arcan_aios_adapters::{
    ArcanApprovalAdapter, ArcanHarnessAdapter, ArcanPolicyAdapter, ArcanProviderAdapter,
    AutonomicPolicyAdapter, EconomicGateHandle, StreamingSenderHandle,
};
use arcan_core::runtime::{Provider, ToolRegistry};
use arcan_harness::bridge::PraxisToolBridge;
use arcan_harness::{FsPolicy, FsPort, LocalFs, SandboxPolicy};
use arcan_lago::{
    FreeTierJournal, LagoPolicyConfig, LagoTrackedFs, MemoryCommitTool, MemoryProjection,
    MemoryProposeTool, MemoryQueryTool, RemoteLagoJournal, SessionJournalSelector,
    run_event_writer,
};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcand::mock::MockProvider;
use clap::{Parser, Subcommand};
use config::ResolvedConfig;
use lago_aios_eventstore_adapter::LagoAiosEventStoreAdapter;
use lago_core::{BranchId, SessionId};
use lago_fs::{FsTracker, Manifest};
use lago_journal::RedbJournal;
use life_vigil::VigConfig;
use nous_observer::NousToolObserver;
use praxis_tools::edit::EditFileTool;
use praxis_tools::fs::{GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use praxis_tools::memory::{ReadMemoryTool, WriteMemoryTool};
use praxis_tools::shell::BashTool;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "arcan",
    version,
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

    /// Serve console assets from a local directory (dev mode)
    #[arg(long, global = true)]
    console_dir: Option<PathBuf>,

    /// LLM provider (anthropic, openai, ollama, mock)
    #[arg(long, global = true)]
    provider: Option<String>,

    /// Model name override
    #[arg(long, global = true)]
    model: Option<String>,

    /// Autonomic homeostasis controller URL (advisory gating, env: ARCAN_AUTONOMIC_URL)
    #[arg(long, global = true)]
    autonomic_url: Option<String>,

    /// Spaces backend: mock (default) or spacetimedb (env: ARCAN_SPACES_BACKEND)
    #[arg(long, global = true)]
    spaces_backend: Option<String>,

    /// SpacetimeDB auth token override (env: SPACETIMEDB_TOKEN)
    #[arg(long, global = true)]
    spaces_token: Option<String>,

    /// Bare mode: minimal system prompt and core tools only, for small-context models (≤4K tokens)
    #[arg(long, global = true)]
    bare: bool,
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
    /// Authenticate with an LLM provider via OAuth
    Login {
        /// Provider to authenticate with (e.g. "openai")
        provider: String,

        /// Use device code flow instead of browser-based PKCE (for headless environments)
        #[arg(long)]
        device: bool,
    },
    /// Remove stored OAuth credentials for a provider
    Logout {
        /// Provider to log out of (e.g. "openai")
        provider: String,
    },
    /// Open the interactive API docs (Scalar UI) in the browser
    Api {
        /// Daemon URL (skip auto-start, connect to existing)
        #[arg(long)]
        url: Option<String>,

        /// Write the OpenAPI spec JSON to a file instead of opening the browser
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Manage skill discovery and registration
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// Interactive REPL with slash commands (single-process, no daemon)
    Shell {
        /// Session ID to use (creates new if not provided)
        #[arg(short, long)]
        session: Option<String>,

        /// Auto-approve all tool executions (skip permission prompts)
        #[arg(short, long)]
        yes: bool,

        /// Resume the most recent session (or the one specified by --session)
        #[arg(short, long)]
        resume: bool,

        /// Session budget in USD. Warns at 80%, stops new LLM calls at 100%.
        #[arg(short, long)]
        budget: Option<f64>,

        /// Display model reasoning/thinking tokens in the output
        #[arg(long)]
        show_reasoning: bool,
    },
}

#[derive(Subcommand)]
enum SkillsAction {
    /// List discovered skills
    List {
        /// Force fresh discovery instead of using cache
        #[arg(long)]
        refresh: bool,
    },
    /// Sync skills from ~/.agents/skills/ into .arcan/skills/ via symlinks
    Sync,
    /// Show skill discovery directories
    Dirs,
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
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to install SIGHUP handler");
        tokio::select! {
            _ = ctrl_c => { tracing::info!("Received SIGINT (Ctrl-C)"); },
            _ = sigterm.recv() => { tracing::info!("Received SIGTERM"); },
            _ = sighup.recv() => { tracing::info!("Received SIGHUP (terminal closed or config reload)"); },
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        tracing::info!("Received Ctrl-C");
    }

    tracing::info!("Shutting down gracefully, draining connections...");
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

/// Try to create an OpenAI provider from stored OAuth credentials.
fn try_openai_oauth_provider() -> Option<Arc<dyn Provider>> {
    let tokens = arcan_provider::oauth::load_tokens("openai").ok()?;
    let credential = Arc::new(arcan_provider::oauth::OAuthCredential::openai(tokens));
    let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
    let config = arcan_provider::openai::OpenAiConfig::from_oauth(credential, model.clone());
    tracing::info!(%model, "Provider: OpenAI (OAuth)");
    Some(Arc::new(
        arcan_provider::openai::OpenAiCompatibleProvider::new(config),
    ))
}

/// Try to create an Anthropic provider from stored OAuth credentials.
fn try_anthropic_oauth_provider() -> Option<Arc<dyn Provider>> {
    let tokens = arcan_provider::oauth::load_tokens("anthropic").ok()?;
    let credential = Arc::new(arcan_provider::oauth::OAuthCredential::anthropic(tokens));
    let model =
        std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());
    let config = AnthropicConfig {
        credential,
        model: model.clone(),
        max_tokens: 4096,
        base_url: "https://api.anthropic.com".to_string(),
    };
    tracing::info!(%model, "Provider: Anthropic (OAuth)");
    Some(Arc::new(AnthropicProvider::new(config)))
}

/// Build provider from resolved configuration.
fn build_provider(resolved: &ResolvedConfig) -> anyhow::Result<Arc<dyn Provider>> {
    let pc = resolved.provider_config.as_ref();

    match resolved.provider.as_str() {
        "mock" => {
            tracing::warn!("Provider: MockProvider (forced via config)");
            Ok(Arc::new(MockProvider))
        }
        "openai" | "codex" | "openai-codex" => {
            // Try OAuth credential first, then fall back to env var / resolved config.
            if let Some(p) = try_openai_oauth_provider() {
                return Ok(p);
            }
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
        "apfel" | "apple" => {
            let base_url = arcan_provider::apfel::resolve_base_url();
            arcan_provider::apfel::ensure_apfel_running(&base_url)?;
            let config = arcan_provider::openai::OpenAiConfig::apfel_from_resolved(
                pc.and_then(|p| p.base_url.as_deref()),
                pc.and_then(|p| p.max_tokens),
            )?;
            if let Ok(info) = arcan_provider::apfel::model_info(&config.base_url) {
                tracing::info!(
                    model = %info.model,
                    context_window = info.context_window,
                    version = %info.version,
                    "Provider: apfel (Apple on-device)"
                );
            } else {
                tracing::info!(base_url = %config.base_url, "Provider: apfel (Apple on-device)");
            }
            if !resolved.bare {
                tracing::warn!(
                    "Tip: apfel has a 4K context window. Consider using --bare for better results."
                );
            }
            Ok(Arc::new(
                arcan_provider::openai::OpenAiCompatibleProvider::new(config),
            ))
        }
        "anthropic" | "claude" => {
            // Try OAuth credential first, then fall back to env var.
            if let Some(p) = try_anthropic_oauth_provider() {
                return Ok(p);
            }
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
            } else if let Some(p) = try_anthropic_oauth_provider() {
                Ok(p)
            } else if let Some(p) = try_openai_oauth_provider() {
                Ok(p)
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

fn run_serve(
    data_dir: &Path,
    resolved: &ResolvedConfig,
    console_dir: Option<PathBuf>,
    tokio_runtime: &tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    // The Tokio runtime is entered (via `_rt_guard` in main) but NOT blocked
    // on yet.  This means:
    //   - async reqwest Client + tonic Channel work (they find `Handle::current()`)
    //   - reqwest::blocking::Client also works (no nested runtime panic)
    //   - `tokio::spawn()` works (tasks are queued, run when block_on starts)
    let workspace_root = std::env::current_dir()?;

    // --- Lago persistence ---
    //
    // When LAGO_URL is set arcan forwards all events to the remote Lago daemon
    // (durable across redeploys).  Otherwise it opens a local RedbJournal
    // (suitable for dev and single-container deployments).
    let blobs_path = data_dir.join("blobs");
    std::fs::create_dir_all(&blobs_path)?;

    let journal: Arc<dyn lago_core::Journal> = if let Ok(lago_url) = std::env::var("LAGO_URL") {
        tracing::info!(
            workspace = %workspace_root.display(),
            lago_url = %lago_url,
            provider = %resolved.provider,
            model = ?resolved.model,
            port = resolved.port,
            "Starting arcan (remote Lago journal)"
        );
        Arc::new(RemoteLagoJournal::new(lago_url))
    } else {
        let journal_path = data_dir.join("journal.redb");
        tracing::info!(
            workspace = %workspace_root.display(),
            journal = %journal_path.display(),
            blobs = %blobs_path.display(),
            provider = %resolved.provider,
            model = ?resolved.model,
            port = resolved.port,
            "Starting arcan (local RedbJournal)"
        );
        Arc::new(RedbJournal::open(&journal_path)?)
    };

    let blob_store = Arc::new(lago_store::BlobStore::open(&blobs_path)?);

    // BRO-217: Wrap in SessionJournalSelector — routes memory events for anonymous
    // sessions to EphemeralJournal (discard). The raw journal is retained for
    // LagoAiosEventStoreAdapter (audit events always persist regardless of tier).
    let session_selector = Arc::new(SessionJournalSelector::new(journal.clone()));
    // BRO-218/219: FreeTierJournal wraps session_selector so the write chain is:
    //   memory_tools → free_tier_journal (TTL-tag if registered)
    //               → session_selector (discard if anonymous) → raw journal.
    let free_tier_journal = Arc::new(FreeTierJournal::new(
        session_selector.clone() as Arc<dyn lago_core::Journal>,
        LagoPolicyConfig::default(),
    ));
    let memory_journal: Arc<dyn lago_core::Journal> = free_tier_journal.clone();

    // --- Lago-tracked filesystem (O(1) write tracking via FsTracker) ---
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store.clone()));
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);
    let tracked_fs: Arc<dyn FsPort> = Arc::new(LagoTrackedFs::new(local_fs, tracker, fs_event_tx));

    let sandbox_policy = SandboxPolicy {
        workspace_root: workspace_root.clone(),
        shell_enabled: true,
        network: NetworkPolicy::AllowAll,
        allowed_env: BTreeSet::new(),
        max_execution_ms: 10_000,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
    };

    // --- Sandbox provider (tiered: Vercel → bwrap → subprocess) ---
    let sandbox_provider = crate::sandbox_router::build_sandbox_provider_with_fallback();

    // --- Tools (Praxis canonical implementations, bridged into Arcan) ---
    let mut registry = ToolRegistry::default();

    if resolved.bare {
        // Bare mode: NO tool schemas sent to model (described in system prompt instead).
        // Small models (≤4K context) hallucinate function calls when tools are present.
        tracing::info!("Bare mode: no tools registered, minimal prompt (for small-context models)");
    } else {
        // Core tools
        registry.register(PraxisToolBridge::new(ReadFileTool::new(tracked_fs.clone())));
        registry.register(PraxisToolBridge::new(WriteFileTool::new(
            tracked_fs.clone(),
        )));
        registry.register(PraxisToolBridge::new(EditFileTool::new(tracked_fs.clone())));
        registry.register(PraxisToolBridge::new(GlobTool::new(tracked_fs.clone())));
        registry.register(PraxisToolBridge::new(GrepTool::new(tracked_fs.clone())));

        let runner: Box<dyn praxis_core::sandbox::CommandRunner> = Box::new(
            arcan_praxis::SandboxCommandRunner::new(sandbox_provider.clone()),
        );
        registry.register(PraxisToolBridge::new(BashTool::new(sandbox_policy, runner)));

        // Extended tools
        {
            registry.register(PraxisToolBridge::new(ListDirTool::new(tracked_fs)));

            let memory_dir = data_dir.join("memory");
            std::fs::create_dir_all(&memory_dir)?;
            registry.register(PraxisToolBridge::new(ReadMemoryTool::new(
                memory_dir.clone(),
            )));
            registry.register(PraxisToolBridge::new(WriteMemoryTool::new(memory_dir)));

            // --- Governed memory tools (event-sourced via Lago) ---
            let memory_projection = Arc::new(RwLock::new(MemoryProjection::new()));
            registry.register(MemoryQueryTool::new(memory_projection));
            registry.register(MemoryProposeTool::new(memory_journal.clone()));
            registry.register(MemoryCommitTool::new(memory_journal));
        }
    } // else (not bare)

    // --- Skill discovery (scan directories for SKILL.md files) ---
    let mut skill_registry_arc: Option<Arc<praxis_skills::registry::SkillRegistry>> = None;
    if resolved.skills_enabled && !resolved.bare {
        let skills_dir = data_dir.join("skills");
        std::fs::create_dir_all(&skills_dir).ok();

        // Auto-sync: symlink skills from ~/.agents/skills/ and .agents/skills/ into .arcan/skills/
        // so that `npx skills add` installs are immediately visible to Arcan.
        match skills::sync_skills_to_arcan(data_dir) {
            Ok(0) => {}
            Ok(n) => tracing::info!(synced = n, "auto-synced skills into .arcan/skills/"),
            Err(e) => tracing::debug!(error = %e, "skill auto-sync skipped"),
        }

        match skills::discover_skills(
            &resolved.skill_dirs,
            data_dir,
            resolved.skills_write_registry,
        ) {
            Ok(skill_registry) => {
                if skill_registry.count() > 0 {
                    tracing::info!(
                        count = skill_registry.count(),
                        "skills discovered and registered"
                    );
                    skill_registry_arc = Some(Arc::new(skill_registry));
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "skill discovery failed (non-fatal)");
            }
        }
    }

    // --- Spaces distributed networking (opt-in, skipped in bare mode) ---
    #[cfg(feature = "spaces")]
    if !resolved.bare {
        let spaces_port: Arc<dyn arcan_spaces::SpacesPort> = match resolved.spaces_backend.as_str()
        {
            "spacetimedb" | "mainnet" => {
                let stdb_config = arcan_spaces::SpacetimeDbConfig::resolve(
                    resolved.spaces_host.as_deref(),
                    resolved.spaces_database_id.as_deref(),
                    resolved.spaces_token.as_deref(),
                )
                .map_err(|e| anyhow::anyhow!("spaces config: {e}"))?;
                tracing::info!(
                    host = %stdb_config.host,
                    database = %stdb_config.database_id,
                    "Spaces: SpacetimeDB backend"
                );
                Arc::new(arcan_spaces::SpacetimeDbClient::new(stdb_config))
            }
            _ => {
                tracing::info!("Spaces: mock backend");
                Arc::new(arcan_spaces::MockSpacesClient::default_hub())
            }
        };
        arcan_spaces::register_spaces_tools(&mut registry, spaces_port);
    }

    // --- Provider ---
    let provider = build_provider(resolved)?;

    // Swappable handle for live provider switching
    let provider_handle: arcan_core::runtime::SwappableProviderHandle =
        Arc::new(std::sync::RwLock::new(provider));
    let provider_factory: Arc<dyn arcan_core::runtime::ProviderFactory> =
        Arc::new(factory::ArcanProviderFactory);

    // --- Canonical aiOS runtime adapters ---
    let event_store: Arc<dyn EventStorePort> =
        Arc::new(LagoAiosEventStoreAdapter::new(journal.clone()));
    let base_policy: Arc<dyn PolicyGatePort> =
        Arc::new(ArcanPolicyAdapter::new(aios_protocol::PolicySet::default()));

    // --- Autonomic advisory gate (embedded by default, remote via --autonomic-url) ---
    let (policy_gate, economic_handle): (Arc<dyn PolicyGatePort>, EconomicGateHandle) =
        if let Some(url) = &resolved.autonomic_url {
            // Remote: consult standalone Autonomic daemon via HTTP.
            tracing::info!(url = %url, "Autonomic advisory enabled (remote)");
            let adapter = AutonomicPolicyAdapter::new_remote(base_policy, url.clone());
            let handle = adapter.economic_handle();
            (Arc::new(adapter), handle)
        } else {
            // Embedded: run controller in-process (default).
            tracing::info!("Autonomic advisory enabled (embedded)");
            let adapter = AutonomicPolicyAdapter::new_embedded(base_policy);
            let handle = adapter.economic_handle();
            (Arc::new(adapter), handle)
        };

    // --- Provider adapter (wired with economic handle for cost gating) ---
    // Shared handle: starts empty, filled after runtime creation.
    let streaming_sender: StreamingSenderHandle = Arc::new(std::sync::Mutex::new(None));
    let tool_definitions = registry.definitions();
    let adapter = ArcanProviderAdapter::from_handle(
        provider_handle.clone(),
        tool_definitions,
        streaming_sender.clone(),
    )
    .with_economic_handle(economic_handle);

    // The skill catalog is now injected per-session in arcand/src/canonical.rs,
    // filtered by the session's tier policy (BRO-214).

    let provider_adapter: Arc<dyn ModelProviderPort> = Arc::new(adapter);

    // --- Nous metacognitive evaluation ---
    // Initialize the eval registry and log scores via tracing on every tool execution.
    let nous_registry = match nous_heuristics::default_registry() {
        Ok(reg) => {
            tracing::info!(evaluators = reg.len(), "Nous eval active");
            Some(reg)
        }
        Err(e) => {
            tracing::warn!(error = %e, "Nous eval init failed (non-fatal)");
            None
        }
    };

    // --- Async judge provider (LLM-as-judge for run-level evaluation) ---
    // Requires ANTHROPIC_API_KEY. Builds reqwest::blocking::Client, so must
    // be initialized BEFORE entering the Tokio runtime.
    let judge_provider: Option<Arc<dyn nous_judge::JudgeProvider>> =
        match nous_judge::AnthropicJudgeProvider::from_env() {
            Ok(provider) => {
                tracing::info!("async judge evaluators enabled (AnthropicJudgeProvider)");
                Some(Arc::new(provider))
            }
            Err(_) => {
                tracing::info!("async judge evaluators disabled (ANTHROPIC_API_KEY not set)");
                None
            }
        };

    let score_store = nous_api::ScoreStore::new();
    let nous_observer: Option<Arc<NousToolObserver>> = nous_registry.map(|r| {
        Arc::new(
            NousToolObserver::with_full_config(
                r,
                journal.clone(),
                "default",
                "arcan",
                judge_provider,
            )
            .with_score_store(score_store.clone()),
        )
    });

    let mut harness = ArcanHarnessAdapter::new(registry);
    if let Some(ref obs) = nous_observer {
        harness = harness.with_observer(obs.clone());
    }
    let tool_harness: Arc<dyn ToolHarnessPort> = Arc::new(harness);

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

    let data_dir_owned = data_dir.to_path_buf();
    let port = resolved.port;

    // ── Async phase (inside Tokio block_on) ────────────────────────────
    //
    // The runtime was created in main() and entered via `_rt_guard`.
    // Now we block_on to run the HTTP server and async initialization.
    tokio_runtime.block_on(async move {
        // --- Background FS event writer (persists tracked writes to Lago journal) ---
        let session_id = SessionId::from_string("default");
        let branch_id = BranchId::from_string("main");
        tokio::spawn(run_event_writer(
            fs_event_rx,
            journal,
            session_id,
            branch_id,
        ));

        // --- HTTP Server ---
        let sandbox_store = Arc::new(arcan_sandbox::InMemorySessionStore::new());

        // Build run observers for canonical router (async judge on run completion).
        let mut run_observers: Vec<Arc<dyn arcan_aios_adapters::tools::ToolHarnessObserver>> =
            Vec::new();

        if let Some(obs) = nous_observer {
            run_observers.push(obs);
        }

        {
            // Register lifecycle observer for session cleanup.
            // Tier defaults to Anonymous (conservative: destroys on run end).
            // BRO-253 will add per-session tier extraction.
            use aios_protocol::SubscriptionTier;
            use arcan_aios_adapters::SandboxLifecycleObserver;
            run_observers.push(Arc::new(SandboxLifecycleObserver::new(
                sandbox_provider.clone(),
                Arc::clone(&sandbox_store),
                SubscriptionTier::Anonymous,
            )));
        }

        let mut router = arcand::canonical::create_canonical_router_with_skills(
            runtime,
            provider_handle,
            provider_factory,
            skill_registry_arc,
            Some(score_store),
            run_observers,
            None, // identity — use BasicIdentity default; Anima can be wired later
            data_dir,
            Some(workspace_root),    // BRO-366: workspace root for liquid prompt
            Some(session_selector),  // BRO-217: ephemeral journal routing for anonymous tiers
            Some(free_tier_journal), // BRO-218: TTL tagging for free-tier sessions
            resolved.bare,           // minimal prompt for small-context models
        );

        // --- Console UI ---
        #[cfg(feature = "console")]
        {
            let console_config = arcan_console::ConsoleConfig {
                override_dir: console_dir,
            };
            router = router.nest("/console", arcan_console::console_router(console_config));
        }

        // --- CORS (for dev mode with separate Vite server) ---
        let cors = tower_http::cors::CorsLayer::permissive();
        let router = router.layer(cors);

        // Bind to 0.0.0.0 when ARCAN_BIND_ADDR is set (containers/production),
        // otherwise default to 127.0.0.1 (local development).
        let addr: std::net::SocketAddr = std::env::var("ARCAN_BIND_ADDR")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], port)));
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
                resolved.bare,
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
                resolved.bare,
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

#[allow(clippy::print_stderr)]
fn run_login(provider: &str, device: bool, data_dir: &Path) -> anyhow::Result<()> {
    // Normalize to canonical provider name for config persistence.
    let canonical = match provider {
        "codex" | "openai-codex" => "openai",
        other => other,
    };

    match provider {
        "openai" | "codex" | "openai-codex" => {
            if device {
                arcan_provider::oauth::device_login_openai().map_err(|e| anyhow::anyhow!("{e}"))?;
            } else {
                arcan_provider::oauth::pkce_login_openai().map_err(|e| anyhow::anyhow!("{e}"))?;
            }
        }
        "anthropic" | "claude" => {
            arcan_provider::oauth::pkce_login_anthropic().map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unknown provider '{provider}'. Supported: anthropic, openai"
            ));
        }
    }

    // Set the provider as the new default after successful login.
    let mut file_config = config::load_config(data_dir);
    file_config.set_key("provider", canonical).ok();
    config::save_config(data_dir, &file_config)?;
    eprintln!("Default provider set to '{canonical}'");

    Ok(())
}

#[allow(clippy::print_stdout)]
async fn run_api(
    data_dir: PathBuf,
    resolved: &ResolvedConfig,
    url: Option<String>,
    output: Option<PathBuf>,
) -> anyhow::Result<()> {
    // If --output is given, write the spec to a file and exit.
    if let Some(path) = output {
        let spec = arcand::canonical::openapi_spec();
        let json = serde_json::to_string_pretty(&spec)?;
        std::fs::write(&path, &json)?;
        println!("OpenAPI spec written to {}", path.display());
        return Ok(());
    }

    // Ensure daemon is running.
    let base_url = match url {
        Some(u) => u,
        None => {
            daemon::ensure_daemon(
                &data_dir,
                resolved.port,
                Some(resolved.provider.as_str()).filter(|s| !s.is_empty()),
                resolved.model.as_deref(),
                resolved.bare,
            )
            .await?
        }
    };

    let docs_url = format!("{base_url}/docs");
    println!("API docs available at: {docs_url}");

    // Try to open the browser.
    let opened = std::process::Command::new("open")
        .arg(&docs_url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    if !opened {
        println!("Open the URL above in your browser to explore the API.");
    }

    println!();
    println!("Raw spec: {base_url}/openapi.json");

    Ok(())
}

#[allow(clippy::print_stderr)]
fn run_logout(provider: &str, data_dir: &Path) -> anyhow::Result<()> {
    // Normalize provider name for credential lookup.
    let normalized = match provider {
        "codex" | "openai-codex" => "openai",
        other => other,
    };
    arcan_provider::oauth::remove_tokens(normalized).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Clear the default provider if it matches the one being logged out.
    let mut file_config = config::load_config(data_dir);
    if file_config.defaults.provider.as_deref() == Some(normalized) {
        file_config.defaults.provider = None;
        config::save_config(data_dir, &file_config)?;
        eprintln!("Default provider cleared");
    }

    eprintln!("Logged out of {provider}");
    Ok(())
}

#[allow(clippy::print_stdout)]
fn run_skills(
    data_dir: &Path,
    resolved: &config::ResolvedConfig,
    action: &SkillsAction,
) -> anyhow::Result<()> {
    match action {
        SkillsAction::List { refresh } => {
            let refresh = *refresh;
            if !refresh {
                // Try cache first
                if skills::print_cached_skills(data_dir) {
                    return Ok(());
                }
            }

            // Full discovery
            let registry = skills::discover_skills(
                &resolved.skill_dirs,
                data_dir,
                resolved.skills_write_registry,
            )?;
            skills::print_skills_list(&registry);
        }
        SkillsAction::Sync => {
            println!("Syncing skills into .arcan/skills/...");
            let synced = skills::sync_skills_to_arcan(data_dir)?;

            // Re-discover after sync
            let registry = skills::discover_skills(
                &resolved.skill_dirs,
                data_dir,
                resolved.skills_write_registry,
            )?;

            println!();
            println!(
                "Synced {} new skill(s). Total discovered: {}",
                synced,
                registry.count()
            );
        }
        SkillsAction::Dirs => {
            println!("Skill discovery directories:");
            for dir in &resolved.skill_dirs {
                let exists = dir.exists();
                let count = if exists {
                    // Count SKILL.md files
                    walkdir::WalkDir::new(dir)
                        .into_iter()
                        .filter_map(Result::ok)
                        .filter(|e| {
                            e.file_type().is_file()
                                && e.file_name()
                                    .to_string_lossy()
                                    .eq_ignore_ascii_case("SKILL.md")
                        })
                        .count()
                } else {
                    0
                };
                let status = if exists {
                    format!("{count} skill(s)")
                } else {
                    "(not found)".to_string()
                };
                println!("  {} — {}", dir.display(), status);
            }
        }
    }

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
            // Build the Tokio runtime FIRST.  Vigil's OTLP exporter uses
            // tonic which calls `Endpoint::connect_lazy()` → `TokioExecutor`
            // → `tokio::spawn()`, which panics without an active runtime.
            // The runtime is entered (not blocked on) so that:
            //   - tonic/hyper-util can find the Handle via `Handle::current()`
            //   - reqwest::blocking::Client (used by LLM providers) can still
            //     construct its own internal runtime without "nested runtime" panics
            let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let _rt_guard = tokio_runtime.enter();

            // Structured logging + optional OTel export via Vigil
            let _vigil_guard =
                life_vigil::init_telemetry(VigConfig::for_service("arcan").with_env_overrides())
                    .expect("failed to initialize telemetry");

            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                max_iterations,
                approval_timeout,
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );

            run_serve(&data_dir, &resolved, cli.console_dir, &tokio_runtime)
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
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_chat(data_dir, &resolved, session, url))
        }
        Some(Command::Login { provider, device }) => run_login(&provider, device, &data_dir),
        Some(Command::Logout { provider }) => run_logout(&provider, &data_dir),
        Some(Command::Api { url, output }) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_api(data_dir, &resolved, url, output))
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
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_message(
                data_dir, &resolved, message, session, url, json,
            ))
        }
        Some(Command::Config { action }) => run_config(&data_dir, action),
        Some(Command::Skills { action }) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );
            run_skills(&data_dir, &resolved, &action)
        }
        Some(Command::Shell {
            session,
            yes,
            resume,
            budget,
            show_reasoning,
        }) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
            );

            // Build the Tokio runtime FIRST — same as `serve` mode.
            // Vigil's OTLP exporter uses tonic which needs `Handle::current()`.
            let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            let _rt_guard = tokio_runtime.enter();

            // Shell mode: file-based fmt layer (avoids clobbering the terminal)
            // combined with an optional OTel layer for OTLP span export (BRO-372).
            let log_dir = data_dir.join("logs");
            std::fs::create_dir_all(&log_dir)?;
            let _vigil_guard = shell::init_shell_telemetry(&log_dir, "arcan-shell");

            shell::run_shell(
                &data_dir,
                &resolved,
                session.as_deref(),
                yes,
                resume,
                budget,
                show_reasoning,
            )
        }
        Some(Command::Status) => {
            let resolved = config::resolve(
                &file_config,
                cli.provider.as_deref(),
                cli.model.as_deref(),
                cli.port,
                None,
                None,
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
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
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
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
                cli.autonomic_url.as_deref(),
                cli.spaces_backend.as_deref(),
                cli.spaces_token.as_deref(),
                cli.bare,
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
