// `agent_cmd` lives in the library half of this crate (see
// `src/lib.rs`) so integration tests can exercise it directly. The
// binary just re-uses the same module via `arcan::agent_cmd`.
use arcan::agent_cmd;
mod chronos_wiring;
mod cli_run;
mod config;
mod consolidator;
mod daemon;
mod embedding;
mod ephemeral_journal;
mod ergon_wiring;
mod factory;
mod identity_loader;
mod markdown;
mod memory_observer;
mod memory_tools;
mod nous_observer;
#[cfg(feature = "opsis")]
mod opsis_observer;
mod prompt;
mod sandbox_router;
mod shell;
mod skills;
mod spinner;

use aios_protocol::sandbox::NetworkPolicy;
use aios_protocol::{
    ApprovalPort, EventStorePort, ModelProviderPort, PolicyGatePort, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig, TurnMiddleware};
use arcan_aios_adapters::{
    ArcanApprovalAdapter, ArcanHarnessAdapter, ArcanPolicyAdapter, ArcanProviderAdapter,
    AutonomicPolicyAdapter, EconomicGateHandle, StreamingSenderHandle,
};
use arcan_core::runtime::{Provider, ToolRegistry};
use arcan_harness::bridge::PraxisToolBridge;
use arcan_harness::{FsPolicy, FsPort, LocalFs, SandboxPolicy};
use arcan_lago::{
    EventSearchTool, FreeTierJournal, KnowledgeEventMiddleware, LagoPolicyConfig, LagoTrackedFs,
    MemoryCommitTool, MemoryProjection, MemoryProposeTool, MemoryQueryTool, ReconcilingTool,
    RemoteBlobBackend, RemoteLagoJournal, SessionJournalSelector, run_event_writer,
};
use arcan_provider::anthropic::{AnthropicConfig, AnthropicProvider};
use arcand::mock::MockProvider;
use clap::{Parser, Subcommand};
use config::ResolvedConfig;
use lago_aios_eventstore_adapter::LagoAiosEventStoreAdapter;
use lago_core::{BranchId, SessionId};
use lago_fs::{FsTracker, SnapshotLimits};
use lago_journal::RedbJournal;
use lago_store::{BlobBackend, LocalBlobBackend};
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

    /// Default subscription tier for unauthenticated sessions (anonymous, free, pro, enterprise).
    /// Set to "pro" for full local/OSS access without the auth stack.
    #[arg(long, global = true, env = "ARCAN_DEFAULT_TIER")]
    default_tier: Option<String>,

    /// Enable the Prosopon display-server sidecar on this address.
    /// Requires `--features prosopon` at build time; silently ignored otherwise.
    #[arg(long, global = true, value_name = "ADDR")]
    prosopon_port: Option<std::net::SocketAddr>,

    /// Directory to load authored agents from (`<dir>/<name>.md` files).
    /// Defaults to `./agents/` relative to the binary's CWD. If the
    /// directory does not exist, arcan logs a warning and starts with
    /// an empty agent registry — `spawn_agent` calls then fail-closed
    /// with a model-visible `unknown_agent` error rather than crashing.
    /// See `agents/README.md` for the authoring format.
    #[arg(long, global = true, env = "ARCAN_AGENTS_DIR", value_name = "DIR")]
    agents_dir: Option<PathBuf>,

    /// Directory holding the `life init` anima identity (`soul.json` +
    /// `seed.local.bin`). When the identity loads, `arcan serve` signs
    /// ergon workflow session boundaries through the custody-backed
    /// soul attester (stable agent DID). Defaults to `.life/identity`
    /// when that directory exists; unset otherwise — workflow ticks
    /// then keep the noop attester with a boot warning.
    #[arg(
        long,
        global = true,
        env = "ARCAN_ANIMA_IDENTITY_DIR",
        value_name = "DIR"
    )]
    anima_identity_dir: Option<PathBuf>,

    /// Bind arcand's substrate-plane gRPC server (`arcan.v1.AgentSubstrate`)
    /// on this Unix-domain socket alongside the HTTP `:3000` server.
    /// When unset, the gRPC server is NOT started — Topology-A users
    /// keep the existing HTTP-only experience. Topology-B operators
    /// set this so lifed's arcan-proxy can dial in. BRO-1016.
    #[arg(long, global = true, env = "ARCAN_UDS_SOCKET", value_name = "PATH")]
    uds_socket: Option<PathBuf>,

    /// Workspace root for agent file tools (read_file, write_file, bash, …).
    /// Created if missing; must be writable by the arcan process. Defaults
    /// to the process CWD — deployments whose CWD is a read-only install
    /// dir (e.g. a root-owned image WORKDIR) MUST set this to a writable
    /// path or every file-tool write fails before reaching lago. BRO-1490.
    /// (Env is ARCAN_WORKSPACE_DIR, not ARCAN_WORKSPACE — the latter is
    /// already the per-run hook variable set by arcan-core hooks.)
    #[arg(long, global = true, env = "ARCAN_WORKSPACE_DIR", value_name = "DIR")]
    workspace: Option<PathBuf>,
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

        /// Enable the Chronos M2 kernel wake-loop (opt-in). Wakes drive `tick_on_branch`.
        #[arg(long, env = "ARCAN_CHRONOS_ENABLED")]
        chronos: bool,

        /// Bind address for the Chronos HTTP wake-ingest API (e.g. `127.0.0.1:3737`).
        /// Requires `--chronos`. Unset ⇒ heartbeat-only.
        #[arg(long, env = "ARCAN_CHRONOS_HTTP_BIND")]
        chronos_http_bind: Option<std::net::SocketAddr>,
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
    /// Inspect, scaffold, and dry-run validate authored agents
    /// (`agents/<name>.md` files). See agents/README.md for the
    /// authoring format. (BRO-1008)
    Agent {
        #[command(subcommand)]
        action: AgentAction,
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
enum AgentAction {
    /// List every agent loaded from `<--agents-dir>/<name>.md`
    /// with model, max_turns, and the first line of instructions.
    List,
    /// Pretty-print the full `AgentSpec` (schemas, tools,
    /// instructions) for one agent.
    Show {
        /// The agent name (matches the filename stem under
        /// `<--agents-dir>`).
        name: String,
    },
    /// Scaffold a new `<--agents-dir>/<name>.md` from a template.
    /// Refuses to overwrite an existing file.
    New {
        /// Stable agent name (becomes both the filename stem and
        /// the `name:` frontmatter field).
        name: String,
        /// Override the default model (`claude-sonnet-4-5-20250929`).
        #[arg(long)]
        model: Option<String>,
        /// Override the default placeholder body. Useful for
        /// pasting an existing prompt into a fresh scaffold.
        #[arg(long)]
        instructions: Option<String>,
    },
    /// Validate an input JSON document against an agent's
    /// `input_schema` (`--dry-run`), or execute the agent against
    /// the configured LLM provider (`--live`). Exactly one mode must
    /// be selected — bare `arcan agent test` errors so no invocation
    /// spends money by accident.
    Test {
        /// The agent name (matches the filename stem under
        /// `<--agents-dir>`).
        name: String,
        /// Input JSON. Either a literal JSON document
        /// (`'{"key": 1}'`) or `@<path>` to read from a file.
        #[arg(long)]
        input: String,
        /// Validate against `input_schema` only — do not execute.
        #[arg(long)]
        dry_run: bool,
        /// Execute the agent against the configured provider
        /// (resolved exactly like `arcan serve`: --provider/--model
        /// flags, config file, API-key env vars). Costs money;
        /// cumulative spend is capped at
        /// `agent_cmd::AGENT_TEST_MAX_TOKENS` tokens.
        #[arg(long, conflicts_with = "dry_run")]
        live: bool,
    },
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

/// Bind arcand's substrate-plane gRPC server (`arcan.v1.AgentSubstrate`)
/// on a Unix-domain socket. Mirrors the soma pattern at
/// `crates/life-kernel/soma/src/listener/unix.rs` — remove any stale
/// socket, ensure the parent directory exists, bind, then serve until
/// the workspace shutdown signal fires.
///
/// BRO-1016: this is the entry point lifed's `arcan-proxy` reaches in
/// Topology B. The HTTP `:3000` server (Topology A) keeps running
/// alongside this one — both share the same `Arc<KernelRuntime>`.
async fn serve_substrate_uds(
    socket_path: PathBuf,
    runtime: Arc<aios_runtime::KernelRuntime>,
) -> anyhow::Result<()> {
    use arcan_substrate_proto::arcan::v1::agent_substrate_server::AgentSubstrateServer;

    if let Some(parent) = socket_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("create parent dir {}: {e}", parent.display()))?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)
            .map_err(|e| anyhow::anyhow!("unlink stale socket {}: {e}", socket_path.display()))?;
    }

    let listener = tokio::net::UnixListener::bind(&socket_path)
        .map_err(|e| anyhow::anyhow!("bind {}: {e}", socket_path.display()))?;

    tracing::info!(
        socket = %socket_path.display(),
        "arcan substrate-plane gRPC listening (arcan.v1.AgentSubstrate)"
    );

    let service = arcand::substrate::SubstrateService::new(runtime);
    let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);

    tonic::transport::Server::builder()
        .add_service(AgentSubstrateServer::new(service))
        .serve_with_incoming_shutdown(incoming, shutdown_signal())
        .await
        .map_err(|e| anyhow::anyhow!("substrate serve: {e}"))?;

    // Clean up the socket file on graceful shutdown. Best-effort.
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}

/// Resolve the effective data directory.
///
/// Priority:
/// 1. Explicit `--data-dir` override (anything other than the default `.arcan`)
/// 2. `.life/arcan/` if a `.life/` project root is found (new convention)
/// 3. Fall back to `.arcan/` relative to cwd (legacy convention)
fn resolve_data_dir(data_dir: &PathBuf) -> anyhow::Result<PathBuf> {
    let workspace_root = std::env::current_dir()?;
    let default_legacy = PathBuf::from(".arcan");

    // If user explicitly passed a non-default path, honour it.
    if *data_dir != default_legacy {
        return Ok(if data_dir.is_relative() {
            workspace_root.join(data_dir)
        } else {
            data_dir.clone()
        });
    }

    // New convention: use .life/arcan/ when a .life/ project root exists.
    if let Some(project_root) = life_paths::find_project_root() {
        return Ok(project_root.join(".life").join("arcan"));
    }

    // Legacy fallback: .arcan/ relative to cwd.
    Ok(workspace_root.join(data_dir))
}

/// Resolve the agent file-tool workspace root (BRO-1490).
///
/// An explicit `--workspace` / `ARCAN_WORKSPACE_DIR` is created if missing
/// and canonicalized (FsPolicy compares canonical paths, and the boot log
/// should show the real location). Without one, fall back to the process
/// CWD — correct for `arcan serve` run from a project checkout, wrong for
/// images whose WORKDIR is a root-owned install dir.
fn resolve_workspace_root(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    match explicit {
        Some(dir) => {
            std::fs::create_dir_all(&dir).map_err(|error| {
                anyhow::anyhow!("cannot create workspace dir {}: {error}", dir.display())
            })?;
            dir.canonicalize().map_err(|error| {
                anyhow::anyhow!("cannot canonicalize workspace {}: {error}", dir.display())
            })
        }
        None => Ok(std::env::current_dir()?),
    }
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
    if let Some(url) = base_url
        && let Some(session) = cli_run::resolve_session_via_api(url).await
    {
        return session;
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
    let model = std::env::var("ANTHROPIC_MODEL")
        .unwrap_or_else(|_| "claude-sonnet-4-5-20250929".to_string());
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

#[allow(clippy::too_many_arguments)]
fn run_serve(
    data_dir: &Path,
    resolved: &ResolvedConfig,
    console_dir: Option<PathBuf>,
    prosopon_port: Option<std::net::SocketAddr>,
    agents_dir: Option<PathBuf>,
    uds_socket: Option<PathBuf>,
    workspace: Option<PathBuf>,
    tokio_runtime: &tokio::runtime::Runtime,
    chronos_enabled: bool,
    chronos_http_bind: Option<std::net::SocketAddr>,
) -> anyhow::Result<()> {
    // The Tokio runtime is entered (via `_rt_guard` in main) but NOT blocked
    // on yet.  This means:
    //   - async reqwest Client + tonic Channel work (they find `Handle::current()`)
    //   - reqwest::blocking::Client also works (no nested runtime panic)
    //   - `tokio::spawn()` works (tasks are queued, run when block_on starts)
    let workspace_root = resolve_workspace_root(workspace)?;

    // BRO-1490 defence-in-depth: a non-writable workspace surfaces at
    // runtime as an opaque per-tool io error ("No such file or directory")
    // deep inside a chat turn. Probe once at boot and say exactly what is
    // wrong while the operator is still watching the boot log. Warn-only:
    // a read-only workspace still serves read_file/grep/glob.
    let probe = workspace_root.join(format!(".arcan-write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
        }
        Err(error) => tracing::warn!(
            workspace = %workspace_root.display(),
            %error,
            "agent workspace is NOT writable — file tools (write_file, edit_file, bash) \
             will fail; set --workspace / ARCAN_WORKSPACE_DIR to a writable directory"
        ),
    }

    // --- Lago persistence ---
    //
    // When LAGO_URL is set arcan forwards all events to the remote Lago daemon
    // (durable across redeploys).  Otherwise it opens a local RedbJournal
    // (suitable for dev and single-container deployments).
    let blobs_path = data_dir.join("blobs");
    std::fs::create_dir_all(&blobs_path)?;

    // Capture LAGO_URL once: it selects BOTH the event journal (above) and the
    // blob backend (below), so a remote-journal deployment also stores blob
    // *content* remotely — otherwise events would go to lagod while the bytes
    // stayed on ephemeral local disk (BRO-1478).
    let lago_url_opt = std::env::var("LAGO_URL").ok();

    let journal: Arc<dyn lago_core::Journal> = if let Some(lago_url) = &lago_url_opt {
        tracing::info!(
            workspace = %workspace_root.display(),
            lago_url = %lago_url,
            provider = %resolved.provider,
            model = ?resolved.model,
            port = resolved.port,
            "Starting arcan (remote Lago journal)"
        );
        Arc::new(RemoteLagoJournal::new(lago_url.clone()))
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

    // Blob backend tracks the journal's locality: remote lagod over HTTP when
    // LAGO_URL is set, local content-addressed store otherwise. The local
    // store is only opened in the local case (it's the tracker's sole
    // consumer), so a remote deployment doesn't create an unused blobs dir.
    let blob_backend: Arc<dyn BlobBackend> = if let Some(lago_url) = &lago_url_opt {
        tracing::info!(lago_url = %lago_url, "arcan blob content: remote lago blob store");
        Arc::new(RemoteBlobBackend::new(lago_url.clone()))
    } else {
        let blob_store = Arc::new(lago_store::BlobStore::open(&blobs_path)?);
        Arc::new(LocalBlobBackend::new(blob_store))
    };

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
    // Baseline the tracker against the live, already-populated workspace at
    // boot. Seeding with an empty manifest would make the first exec-path
    // reconcile diff the whole workspace against nothing — emitting a spurious
    // FileWrite for EVERY pre-existing file. `with_baseline` records that prior
    // state up front (no events), so reconcile only reports genuine post-boot
    // changes. The baseline limits MUST match the exec-path reconciler's limits
    // (ReconcilingTool uses SnapshotLimits::default()) so both see the same file
    // set — a mismatch would resurface as phantom diffs on the first reconcile.
    // Content is addressed through the selected `blob_backend` (local or remote
    // lagod), so the baseline honours the same durability target as runtime
    // writes.
    let tracker = Arc::new(FsTracker::with_baseline(
        &workspace_root,
        blob_backend.clone(),
        SnapshotLimits::default(),
    )?);
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);
    // Share the tracker + event channel with the exec-path reconciler below so
    // shell-tool writes land in the same manifest/blob-store/journal that the
    // FsPort write path uses. The FsPort write path takes its own clones.
    let exec_tracker = tracker.clone();
    let exec_fs_event_tx = fs_event_tx.clone();
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
        // Wrap the shell tool so its filesystem side effects are reconciled
        // into the lago manifest/blob-store/journal after each run — shell
        // commands write directly to the workspace, bypassing LagoTrackedFs.
        let bash_tool = ReconcilingTool::new(
            BashTool::new(sandbox_policy, runner),
            exec_tracker,
            exec_fs_event_tx,
            workspace_root.clone(),
        );
        registry.register(PraxisToolBridge::new(bash_tool));

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

            // Cross-session event search (BRO-432)
            registry.register(EventSearchTool::new(journal.clone(), None));
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

    // --- Opsis world state bridge (opt-in via OPSIS_URL env var) ---
    #[cfg(feature = "opsis")]
    let opsis_client: Option<Arc<arcan_opsis::OpsisClient>> = if !resolved.bare {
        match std::env::var("OPSIS_URL") {
            Ok(url) => {
                let agent_id = format!("arcan-agent:{}", fastrand::u32(..));
                match arcan_opsis::OpsisClient::new(&url, agent_id) {
                    Ok(client) => {
                        let client = Arc::new(client);
                        let injector = arcan_opsis::WorldStateInjector::new(&url)
                            .map_err(|e| anyhow::anyhow!("opsis world state injector: {e}"))?;
                        let snapshot = injector.snapshot_handle();
                        arcan_opsis::register_opsis_tools(&mut registry, client.clone(), snapshot);
                        // Spawn SSE loop to continuously update the world state snapshot.
                        let injector = Arc::new(injector);
                        injector.spawn_sse_loop(&url);
                        tracing::info!(url = %url, "Opsis bridge enabled (3 tools registered + SSE loop)");
                        Some(client)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Opsis client init failed — running without opsis");
                        None
                    }
                }
            }
            Err(_) => None,
        }
    } else {
        None
    };

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
    // Names of the kernel's governed (registry) tools — captured before
    // `tool_definitions` is moved into the adapter. Wired into the
    // KernelRuntime below so the client-tool handoff path enforces
    // registry-wins on name collisions (a client tool sharing a registry
    // name is NOT handed back to the client; the registry tool runs).
    let registry_tool_names: Vec<String> =
        tool_definitions.iter().map(|t| t.name.clone()).collect();
    let adapter = ArcanProviderAdapter::from_handle(
        provider_handle.clone(),
        tool_definitions,
        streaming_sender.clone(),
    )
    .with_economic_handle(economic_handle.clone());

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

    let turn_middlewares: Vec<Arc<dyn TurnMiddleware>> =
        vec![Arc::new(KnowledgeEventMiddleware::new(event_store.clone()))];

    let kernel_runtime = KernelRuntime::with_turn_middlewares(
        RuntimeConfig::new(data_dir.to_path_buf()),
        event_store,
        provider_adapter,
        tool_harness,
        approvals,
        policy_gate,
        turn_middlewares,
    )
    .with_registry_tool_names(registry_tool_names);

    // Register the ergon-workflow tick dispatcher (BRO-1001). No
    // workflows are registered by default — adopting daemons override
    // this section to register their concrete `ergon::Workflow` impls
    // before the runtime starts serving. Until any are registered,
    // `TickKind::Workflow` ticks fail with "unknown workflow", which
    // matches the spec's expectation of opt-in workflow support.
    //
    // Wire the spawn_agent substrate (BRO-1007b + BRO-1010): authored
    // agents are loaded from `<agents_dir>/<name>.md` (default
    // `./agents/`) via [`ergon::FsAgentRegistry::load`]. If the
    // directory does not exist or no agents are present, arcan falls
    // back to an empty [`ergon::InMemoryAgentRegistry`] with a
    // warning, so `spawn_agent` calls fail-closed with
    // `unknown_agent` rather than crashing the boot. The blessed
    // agents shipped in `agents/` (general, goal-pursuer, goal-judge)
    // populate this on a normal install.
    let workflow_registry = Arc::new(arcan_ergon::WorkflowRegistry::new());
    let agents_dir_resolved = agents_dir.unwrap_or_else(|| PathBuf::from("agents"));
    let agent_registry: Arc<dyn ergon::AgentRegistry> = if agents_dir_resolved.is_dir() {
        match ergon::FsAgentRegistry::load(&agents_dir_resolved) {
            Ok(registry) => {
                // Bring the `AgentRegistry::len` trait method into scope to count
                // the loaded agents for the boot log.
                use ergon::AgentRegistry as _;
                let count = tokio_runtime.block_on(registry.len());
                tracing::info!(
                    target: "arcan.agents",
                    "loaded {count} authored agent(s) from {dir}",
                    count = count,
                    dir = agents_dir_resolved.display(),
                );
                Arc::new(registry)
            }
            Err(e) => {
                tracing::warn!(
                    target: "arcan.agents",
                    "failed to load agents from `{dir}`: {err}; starting with empty registry",
                    dir = agents_dir_resolved.display(),
                    err = e,
                );
                Arc::new(ergon::InMemoryAgentRegistry::new())
            }
        }
    } else {
        tracing::warn!(
            target: "arcan.agents",
            "no agents directory at `{dir}`; spawn_agent calls will return unknown_agent. \
             Pass --agents-dir or place authored agents in ./agents/. See agents/README.md.",
            dir = agents_dir_resolved.display(),
        );
        Arc::new(ergon::InMemoryAgentRegistry::new())
    };
    // Harness Phase-2 gap closure (2026-06-10): workflow ticks get the
    // real substrate wiring instead of buffer-only sinks + noop hooks.
    // - stream events fan out to a per-session LagoSink over the SAME
    //   journal the kernel's EventStorePort writes to → `lago replay`
    //   sees workflow ticks;
    // - inference passes the Autonomic economic gate (Hibernate denies,
    //   Hustle clamps max_tokens) — mirroring the Direct path;
    // - responses are scored through the Nous evaluator registry when
    //   it initialized.
    // - session boundaries are attested through the custody-backed
    //   `AgentAttestationAdapter` when a `life init` identity is
    //   configured (see below); unconfigured hosts keep the explicit
    //   Noop fallback with a boot warning.
    let mut workflow_inputs = arcan_ergon::runner::WorkflowRunInputs::empty()
        .with_agent_registry(agent_registry)
        .with_stream_sink_factory(ergon_wiring::lago_stream_sink_factory(journal.clone()))
        .with_budget_gate(Arc::new(ergon_wiring::EconomicBudgetGate::new(
            economic_handle,
        )));
    // The Direct-path observer owns its registry instance, so build a
    // second one for the workflow scorer — the heuristic evaluators
    // are stateless, two instances are equivalent.
    match nous_heuristics::default_registry() {
        Ok(registry) => {
            workflow_inputs = workflow_inputs.with_response_scorer(Arc::new(
                ergon_nous_adapter::NousAdapter::new(Arc::new(registry)),
            ));
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "ergon workflow ticks: Nous registry unavailable — response scoring falls back to noop"
            );
        }
    }
    // Anima soul attestation: load the `life init` identity from disk
    // (default `.life/identity`, override via --anima-identity-dir /
    // ARCAN_ANIMA_IDENTITY_DIR / `defaults.anima_identity_dir`) and
    // sign workflow session boundaries under the agent's stable DID.
    // A configured-but-corrupt identity FAILS the boot (see
    // `identity_loader`); an absent identity falls back to the noop
    // attester with a warning.
    match resolved.anima_identity_dir.as_deref() {
        Some(identity_dir) => match identity_loader::load_custody_from_disk(identity_dir)? {
            Some(custody) => {
                let attester = ergon_anima_adapter::AgentAttestationAdapter::new(custody);
                tracing::info!(
                    did = attester.agent_did(),
                    identity_dir = %identity_dir.display(),
                    "ergon workflow ticks: soul attestation wired to anima custody"
                );
                workflow_inputs = workflow_inputs.with_soul_attester(Arc::new(attester));
            }
            None => {
                tracing::warn!(
                    identity_dir = %identity_dir.display(),
                    "ergon workflow ticks: no anima identity at the configured directory — \
                     soul attestation falls back to noop (run `life init` to create one)"
                );
            }
        },
        None => {
            tracing::warn!(
                "ergon workflow ticks: anima custody not configured — soul attestation falls \
                 back to noop"
            );
        }
    }
    let workflow_inputs = Arc::new(workflow_inputs);
    let workflow_dispatcher: Arc<dyn aios_runtime::WorkflowTickDispatcher> = Arc::new(
        arcan_ergon::ErgonWorkflowDispatcher::new(workflow_registry, workflow_inputs),
    );
    let kernel_runtime = kernel_runtime.with_workflow_dispatcher(workflow_dispatcher);

    let runtime = Arc::new(kernel_runtime);

    // Wire the broadcast sender now that the runtime exists.
    *streaming_sender.lock().unwrap() = Some(runtime.event_sender());

    // --- Chronos M2 — kernel wake handoff (opt-in via `--chronos`) ---
    // Spawns the wake-loop + HTTP wake-ingest API onto the entered tokio runtime; tasks queue now
    // and run once the server's `block_on` starts. A plain `arcan serve` (no `--chronos`) skips
    // this entirely, so the default runtime path is byte-for-byte unchanged.
    if chronos_enabled {
        chronos_wiring::spawn_chronos(Arc::clone(&runtime), journal.clone(), chronos_http_bind);
    }

    // ── Prosopon display-server sidecar (opt-in via `--features prosopon`) ──
    //
    // The field `prosopon_port` is always present in the CLI struct so the flag
    // is parseable in all builds. The boot logic is conditionally compiled; in a
    // build without the feature the block below is a no-op and clippy won't
    // complain about unused variables.
    #[cfg(feature = "prosopon")]
    if let Some(addr) = prosopon_port {
        use arcan_prosopon::ArcanProsoponBridge;
        use prosopon_compositor_glass::glass_surface;
        use prosopon_daemon::{DaemonConfig, DaemonServer};

        let event_rx = runtime.subscribe_events();

        let bind_result = tokio_runtime.block_on(DaemonServer::bind(DaemonConfig {
            addr,
            surface: Some(glass_surface()),
        }));

        match bind_result {
            Ok(server) => {
                let fanout = server.fanout();
                let bridge = ArcanProsoponBridge::new(fanout);
                let _bridge_handle = bridge.spawn(event_rx);
                tokio_runtime.spawn(async move {
                    if let Err(err) = server.serve().await {
                        tracing::error!(error = %err, "prosopon-daemon serve failed");
                    }
                });
                tracing::info!(%addr, "arcan-prosopon: bridge online");
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    %addr,
                    "arcan-prosopon: failed to bind, arcan will continue without Prosopon"
                );
            }
        }
    }

    #[cfg(not(feature = "prosopon"))]
    if prosopon_port.is_some() {
        tracing::warn!(
            "--prosopon-port was set but arcan was not built with the `prosopon` feature; \
             display-server sidecar is disabled"
        );
    }

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

        // Memory extraction observer — writes key facts to .arcan/memory/
        {
            let memory_dir = data_dir.join("memory");
            run_observers.push(Arc::new(memory_observer::MemoryExtractionObserver::new(
                memory_dir,
            )));
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

        // Opsis world state observer — forwards tool events as ambient observations.
        #[cfg(feature = "opsis")]
        if let Some(ref client) = opsis_client {
            run_observers.push(Arc::new(opsis_observer::OpsisToolObserver::new(
                client.clone(),
            )));
        }

        // BRO-1016: keep an Arc<KernelRuntime> handle alive after the
        // HTTP router takes ownership of one, so the optional
        // substrate-plane gRPC server can share the same runtime.
        let runtime_for_substrate = Arc::clone(&runtime);
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
            resolved.default_tier.as_deref(), // OSS tier override
        );

        // ── Optional substrate-plane gRPC server (Topology B) ─────────
        //
        // When `--uds-socket <PATH>` (or `ARCAN_UDS_SOCKET`) is set, bind
        // `arcan.v1.AgentSubstrate` on the configured Unix-domain
        // socket. This is ADDITIVE to the HTTP `:3000` server below —
        // both run concurrently on a single shared `KernelRuntime`.
        // BRO-1016 closes the Topology B substrate-stub gap captured in
        // `research/entities/concept/topology-b-substrate-stub-gap.md`.
        if let Some(socket_path) = uds_socket.clone() {
            let substrate_runtime = runtime_for_substrate.clone();
            tokio::spawn(async move {
                if let Err(err) = serve_substrate_uds(socket_path, substrate_runtime).await {
                    tracing::error!(error = %err, "substrate-plane gRPC server exited with error");
                }
            });
        } else {
            tracing::debug!(
                "substrate-plane gRPC server NOT bound — pass --uds-socket <PATH> or set \
                 ARCAN_UDS_SOCKET to enable Topology-B routing"
            );
        }

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
            chronos,
            chronos_http_bind,
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
            );

            run_serve(
                &data_dir,
                &resolved,
                cli.console_dir,
                cli.prosopon_port,
                cli.agents_dir,
                cli.uds_socket,
                cli.workspace,
                &tokio_runtime,
                chronos,
                chronos_http_bind,
            )
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
            );
            run_skills(&data_dir, &resolved, &action)
        }
        Some(Command::Agent { action }) => {
            // Agent CLI handlers are mostly filesystem-only — no
            // daemon, no telemetry initialization needed. We build a
            // current-thread tokio runtime so the async
            // `AgentRegistry::get` / `names` paths still work without
            // pulling in the multi-thread executor.
            //
            // The one exception is `test --live` (BRO-1008): it needs
            // a real provider stack, resolved exactly like the serve
            // path (config file + CLI flags + env vars). The provider
            // adapter chain MUST be constructed in sync context and
            // its final Arc must drop in sync context too — the
            // Anthropic provider holds a `reqwest::blocking::Client`
            // whose inner tokio runtime panics if built or dropped
            // inside an async context (see
            // `arcan-ergon/tests/anthropic_agents_smoke.rs`). So the
            // live arm builds the chain HERE, before `block_on`, and
            // keeps an Arc alive in this scope until after it returns.
            let agents_dir = agent_cmd::resolve_agents_dir(cli.agents_dir);
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            match action {
                AgentAction::Test {
                    name,
                    input,
                    dry_run: false,
                    live: true,
                } => {
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
                        cli.default_tier.as_deref(),
                        cli.anima_identity_dir.as_deref(),
                    );
                    let provider = build_provider(&resolved)?;
                    let ergon_provider =
                        agent_cmd::build_ergon_provider(provider, &resolved.provider);
                    // `ergon_provider` (and the provider chain under
                    // it) drops at the end of this arm, in sync
                    // context, AFTER block_on returned — preserving
                    // the reqwest::blocking drop-order invariant.
                    runtime.block_on(agent_cmd::test_live(
                        &agents_dir,
                        &name,
                        &input,
                        Arc::clone(&ergon_provider),
                    ))
                }
                action => runtime.block_on(async move {
                    match action {
                        AgentAction::List => agent_cmd::list(&agents_dir).await,
                        AgentAction::Show { name } => agent_cmd::show(&agents_dir, &name).await,
                        AgentAction::New {
                            name,
                            model,
                            instructions,
                        } => agent_cmd::new_agent(
                            &agents_dir,
                            &name,
                            model.as_deref(),
                            instructions.as_deref(),
                        ),
                        AgentAction::Test {
                            name,
                            input,
                            dry_run,
                            live: _,
                        } => {
                            if !dry_run {
                                return Err(anyhow::anyhow!(
                                    "`arcan agent test` needs an explicit mode: pass --dry-run \
                                     to validate the input against the agent's input_schema \
                                     offline (free), or --live to execute the agent against the \
                                     configured LLM provider (costs money; capped at {} tokens).",
                                    agent_cmd::AGENT_TEST_MAX_TOKENS,
                                ));
                            }
                            agent_cmd::test_dry_run(&agents_dir, &name, &input).await
                        }
                    }
                }),
            }
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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
                cli.default_tier.as_deref(),
                cli.anima_identity_dir.as_deref(),
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

    #[test]
    fn workspace_root_defaults_to_cwd() {
        let resolved = resolve_workspace_root(None).expect("resolve");
        assert_eq!(resolved, std::env::current_dir().expect("cwd"));
    }

    #[test]
    fn workspace_root_creates_missing_explicit_dir() {
        let base = unique_temp_dir("arcan-workspace-create");
        let target = base.join("nested/agent-workspace");
        assert!(!target.exists());

        let resolved = resolve_workspace_root(Some(target.clone())).expect("resolve");

        assert!(target.is_dir(), "explicit workspace dir must be created");
        // Canonicalized result points at the same directory (macOS tempdirs
        // canonicalize through /private, so compare canonical forms).
        assert_eq!(resolved, target.canonicalize().expect("canonicalize"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn workspace_root_canonicalizes_existing_dir() {
        let base = unique_temp_dir("arcan-workspace-existing");
        // Route through `..` so the input is non-canonical.
        let indirect = base.join("sub/..");
        std::fs::create_dir_all(base.join("sub")).expect("mkdir");

        let resolved = resolve_workspace_root(Some(indirect)).expect("resolve");

        assert_eq!(resolved, base.canonicalize().expect("canonicalize"));
        let _ = std::fs::remove_dir_all(base);
    }
}
