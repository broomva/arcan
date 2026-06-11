//! Integration tests for the full Praxis + LagoTrackedFs stack.
//!
//! These tests wire the production tool chain:
//!   HTTP API → KernelRuntime → ArcanHarnessAdapter → PraxisToolBridge
//!     → Praxis tools (FsPort) → LagoTrackedFs → Lago journal
//!
//! `proposed_tool` in RunRequest bypasses the LLM provider and directly
//! executes the named tool, making these tests fully deterministic.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aios_protocol::sandbox::NetworkPolicy;
use aios_protocol::tool::{Tool, ToolCall, ToolContext};
use aios_protocol::{
    ApprovalPort, EventStorePort, ModelProviderPort, PolicyGatePort, PolicySet, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use arcan_aios_adapters::{
    ArcanApprovalAdapter, ArcanHarnessAdapter, ArcanPolicyAdapter, ArcanProviderAdapter,
    StreamingSenderHandle,
};
use arcan_core::runtime::ToolRegistry;
use arcan_harness::bridge::PraxisToolBridge;
use arcan_harness::{FsPolicy, FsPort, LocalCommandRunner, LocalFs, SandboxPolicy};
use arcan_lago::{LagoTrackedFs, ReconcilingTool, run_event_writer};
use arcand::canonical::create_canonical_router;
use arcand::mock::MockProvider;
use lago_aios_eventstore_adapter::LagoAiosEventStoreAdapter;
use lago_core::{BranchId, SessionId};
use lago_fs::SnapshotLimits;
use lago_fs::{FsTracker, Manifest};
use lago_journal::RedbJournal;
use lago_store::{BlobBackend, BlobStore, LocalBlobBackend};
use praxis_tools::edit::EditFileTool;
use praxis_tools::fs::{GlobTool, GrepTool, ListDirTool, ReadFileTool, WriteFileTool};
use praxis_tools::shell::BashTool;
use reqwest::StatusCode;
use serde_json::json;

fn unique_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("arcan-praxis-{name}-{nanos}"))
}

/// Build the full production-equivalent runtime with Praxis tools + LagoTrackedFs.
///
/// Returns `(runtime, provider_handle, provider_factory, workspace_root)`.
fn build_praxis_runtime(
    root: &Path,
) -> (
    Arc<KernelRuntime>,
    arcan_core::runtime::SwappableProviderHandle,
    Arc<dyn arcan_core::runtime::ProviderFactory>,
    PathBuf,
) {
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).expect("create workspace dir");

    let journal_path = root.join("journal.redb");
    let blobs_path = root.join("blobs");
    std::fs::create_dir_all(&blobs_path).expect("create blobs dir");

    // --- Lago persistence ---
    let journal = RedbJournal::open(&journal_path).expect("open journal");
    let blob_store: Arc<dyn lago_store::BlobBackend> = Arc::new(LocalBlobBackend::new(Arc::new(
        BlobStore::open(&blobs_path).expect("open blob store"),
    )));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // --- Lago-tracked filesystem (O(1) write tracking) ---
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store.clone()));
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);
    let tracked_fs: Arc<dyn FsPort> = Arc::new(LagoTrackedFs::new(local_fs, tracker, fs_event_tx));

    // --- Praxis tools (bridged into Arcan via PraxisToolBridge) ---
    let sandbox_policy = SandboxPolicy {
        workspace_root: workspace_root.clone(),
        shell_enabled: true,
        network: NetworkPolicy::AllowAll,
        allowed_env: BTreeSet::new(),
        max_execution_ms: 10_000,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
    };

    let mut registry = ToolRegistry::default();
    registry.register(PraxisToolBridge::new(ReadFileTool::new(tracked_fs.clone())));
    registry.register(PraxisToolBridge::new(WriteFileTool::new(
        tracked_fs.clone(),
    )));
    registry.register(PraxisToolBridge::new(ListDirTool::new(tracked_fs.clone())));
    registry.register(PraxisToolBridge::new(EditFileTool::new(tracked_fs.clone())));
    registry.register(PraxisToolBridge::new(GlobTool::new(tracked_fs.clone())));
    registry.register(PraxisToolBridge::new(GrepTool::new(tracked_fs)));

    let runner = Box::new(LocalCommandRunner);
    registry.register(PraxisToolBridge::new(BashTool::new(sandbox_policy, runner)));

    // --- Provider (mock — bypassed by proposed_tool anyway) ---
    let provider: Arc<dyn arcan_core::runtime::Provider> = Arc::new(MockProvider);
    let provider_handle: arcan_core::runtime::SwappableProviderHandle =
        Arc::new(RwLock::new(provider));

    let streaming_sender: StreamingSenderHandle = Arc::new(std::sync::Mutex::new(None));
    let provider_adapter: Arc<dyn ModelProviderPort> = Arc::new(ArcanProviderAdapter::from_handle(
        provider_handle.clone(),
        registry.definitions(),
        streaming_sender.clone(),
    ));

    // --- Tool harness ---
    let tool_harness: Arc<dyn ToolHarnessPort> = Arc::new(ArcanHarnessAdapter::new(registry));

    // --- Policy + Approvals ---
    let policy_gate: Arc<dyn PolicyGatePort> =
        Arc::new(ArcanPolicyAdapter::new(PolicySet::default()));
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ArcanApprovalAdapter::new());

    // --- Event store (Lago-backed) ---
    let event_store: Arc<dyn EventStorePort> =
        Arc::new(LagoAiosEventStoreAdapter::new(journal.clone()));

    // --- Kernel runtime ---
    let runtime = Arc::new(KernelRuntime::new(
        RuntimeConfig::new(root.to_path_buf()),
        event_store,
        provider_adapter,
        tool_harness,
        approvals,
        policy_gate,
    ));

    *streaming_sender.lock().unwrap() = Some(runtime.event_sender());

    // --- Background FS event writer ---
    let session_id = SessionId::from_string("test-session");
    let branch_id = BranchId::from_string("main");
    tokio::spawn(run_event_writer(
        fs_event_rx,
        journal,
        session_id,
        branch_id,
    ));

    // --- Provider factory (stub) ---
    struct StubFactory;
    impl arcan_core::runtime::ProviderFactory for StubFactory {
        fn build(
            &self,
            _spec: &str,
        ) -> Result<Arc<dyn arcan_core::runtime::Provider>, arcan_core::error::CoreError> {
            Ok(Arc::new(MockProvider))
        }
        fn available_providers(&self) -> Vec<String> {
            vec!["mock".to_string()]
        }
    }
    let factory: Arc<dyn arcan_core::runtime::ProviderFactory> = Arc::new(StubFactory);

    (runtime, provider_handle, factory, workspace_root)
}

/// Helper: start a test HTTP server and return (base_url, server_handle).
async fn start_test_server(
    runtime: Arc<KernelRuntime>,
    provider_handle: arcan_core::runtime::SwappableProviderHandle,
    factory: Arc<dyn arcan_core::runtime::ProviderFactory>,
) -> (String, tokio::task::JoinHandle<()>) {
    let router = create_canonical_router(runtime, provider_handle, factory);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), server)
}

/// Send a proposed_tool run request and return the JSON response.
async fn run_tool(
    client: &reqwest::Client,
    base: &str,
    session: &str,
    tool_name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let response = client
        .post(format!("{base}/sessions/{session}/runs"))
        .json(&json!({
            "objective": format!("test {tool_name}"),
            "proposed_tool": {
                "tool_name": tool_name,
                "input": input
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "tool {tool_name} run failed"
    );
    response.json().await.unwrap()
}

// ─── Integration Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn write_file_through_full_stack() {
    let root = unique_root("write");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    // Create session first
    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    // Write a file via proposed_tool
    let result = run_tool(
        &client,
        &base,
        "test-session",
        "write_file",
        json!({ "path": "hello.txt", "content": "Hello from Praxis!" }),
    )
    .await;

    // Verify the run completed (events_emitted > 0)
    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from write_file run");

    // Verify file was actually written to disk
    let file_path = workspace.join("hello.txt");
    assert!(file_path.exists(), "hello.txt should exist on disk");
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "Hello from Praxis!");

    // Small delay to let background event writer flush
    tokio::time::sleep(Duration::from_millis(100)).await;

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn read_file_through_full_stack() {
    let root = unique_root("read");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    // Pre-create a file in the workspace
    std::fs::write(workspace.join("existing.txt"), "pre-existing content").unwrap();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    // Read the file via proposed_tool
    let result = run_tool(
        &client,
        &base,
        "test-session",
        "read_file",
        json!({ "path": "existing.txt" }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from read_file run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_dir_through_full_stack() {
    let root = unique_root("listdir");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    // Create some files
    std::fs::write(workspace.join("a.txt"), "aaa").unwrap();
    std::fs::write(workspace.join("b.txt"), "bbb").unwrap();
    std::fs::create_dir_all(workspace.join("subdir")).unwrap();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    let result = run_tool(
        &client,
        &base,
        "test-session",
        "list_dir",
        json!({ "path": "." }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from list_dir run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn write_then_read_round_trip() {
    let root = unique_root("roundtrip");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    // Write
    run_tool(
        &client,
        &base,
        "test-session",
        "write_file",
        json!({ "path": "roundtrip.txt", "content": "round trip data" }),
    )
    .await;

    assert!(workspace.join("roundtrip.txt").exists());

    // Read back
    let result = run_tool(
        &client,
        &base,
        "test-session",
        "read_file",
        json!({ "path": "roundtrip.txt" }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from read_file run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn glob_finds_files() {
    let root = unique_root("glob");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    // Create files matching a glob pattern
    std::fs::write(workspace.join("foo.rs"), "fn main() {}").unwrap();
    std::fs::write(workspace.join("bar.rs"), "fn bar() {}").unwrap();
    std::fs::write(workspace.join("baz.txt"), "text").unwrap();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    let result = run_tool(
        &client,
        &base,
        "test-session",
        "glob",
        json!({ "pattern": "*.rs" }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from glob run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn grep_searches_content() {
    let root = unique_root("grep");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    std::fs::write(
        workspace.join("search_me.txt"),
        "needle in a haystack\nno match here",
    )
    .unwrap();
    std::fs::write(workspace.join("other.txt"), "another needle found").unwrap();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    let result = run_tool(
        &client,
        &base,
        "test-session",
        "grep",
        json!({ "pattern": "needle", "path": "." }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from grep run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn bash_executes_command() {
    let root = unique_root("bash");
    let (runtime, handle, factory, _workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/sessions"))
        .json(&json!({ "session_id": "test-session" }))
        .send()
        .await
        .unwrap();

    let result = run_tool(
        &client,
        &base,
        "test-session",
        "bash",
        json!({ "command": "echo hello" }),
    )
    .await;

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from bash run");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn mock_provider_triggers_write_file() {
    let root = unique_root("mock-write");
    let (runtime, handle, factory, workspace) = build_praxis_runtime(&root);
    let (base, server) = start_test_server(runtime, handle, factory).await;
    let client = reqwest::Client::new();

    // Create session with a permissive policy so the mock provider's write_file
    // call isn't blocked by capability enforcement (this test exercises tool
    // execution, not policy gating — policy tests live in arcan-aios-adapters).
    client
        .post(format!("{base}/sessions"))
        .json(&json!({
            "session_id": "test-session",
            "policy": {
                "allow_capabilities": ["*"],
                "gate_capabilities": [],
                "max_tool_runtime_secs": 30,
                "max_events_per_turn": 256
            }
        }))
        .send()
        .await
        .unwrap();

    // The MockProvider triggers write_file when message contains "file"
    let response = client
        .post(format!("{base}/sessions/test-session/runs"))
        .json(&json!({ "objective": "Please write a file for me" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let result: serde_json::Value = response.json().await.unwrap();

    let events = result["events_emitted"].as_u64().unwrap_or(0);
    assert!(events > 0, "expected events from mock provider run");

    // The mock provider writes "test.txt" with "Hello from Mock Provider"
    // Give the tool a moment to execute
    tokio::time::sleep(Duration::from_millis(200)).await;

    let file_path = workspace.join("test.txt");
    assert!(
        file_path.exists(),
        "test.txt should have been created by mock provider's write_file tool call"
    );
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, "Hello from Mock Provider");

    server.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn lago_events_track_writes() {
    let root = unique_root("lago-track");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();

    let journal_path = root.join("journal.redb");
    let blobs_path = root.join("blobs");
    std::fs::create_dir_all(&blobs_path).unwrap();

    // Open journal directly to verify events later
    let journal = RedbJournal::open(&journal_path).expect("open journal");
    let blob_store: Arc<dyn lago_store::BlobBackend> = Arc::new(LocalBlobBackend::new(Arc::new(
        BlobStore::open(&blobs_path).expect("open blob store"),
    )));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // Set up tracked filesystem
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store.clone()));
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);
    let tracked_fs: Arc<dyn FsPort> = Arc::new(LagoTrackedFs::new(local_fs, tracker, fs_event_tx));

    // Start background event writer
    let session_id = SessionId::from_string("lago-test");
    let branch_id = BranchId::from_string("main");
    let journal_for_writer = journal.clone();
    tokio::spawn(run_event_writer(
        fs_event_rx,
        journal_for_writer,
        session_id.clone(),
        branch_id.clone(),
    ));

    // Write a file through the tracked filesystem (simulating tool execution)
    tracked_fs
        .write(&workspace_root.join("tracked.txt"), b"tracked content")
        .unwrap();

    // Verify the file exists on disk
    assert!(workspace_root.join("tracked.txt").exists());

    // Verify the event was persisted in the Lago journal. The writer
    // runs as a background task draining an mpsc channel into a
    // synchronous redb commit — a fixed sleep loses that race on
    // loaded CI runners (observed: PR #1713 Test (Linux), 2026-06-11).
    // Poll with a deadline instead: fast on the happy path, generous
    // (5s) under contention.
    let query = lago_core::EventQuery::new()
        .session(session_id)
        .branch(branch_id);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let events = loop {
        let events = journal.read(query.clone()).await.expect("read journal");
        if !events.is_empty() {
            break events;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected at least one event in the Lago journal after tracked write (5s)"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    // Verify the event is a FileWrite
    let has_file_write = events.iter().any(|e| {
        matches!(
            &e.payload,
            lago_core::event::EventPayload::FileWrite { path, .. }
            if path.contains("tracked.txt")
        )
    });
    assert!(
        has_file_write,
        "expected FileWrite event for tracked.txt in journal, got: {:?}",
        events.iter().map(|e| &e.payload).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn multiple_writes_produce_journal_events() {
    let root = unique_root("multi-write");
    let workspace_root = root.join("workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();

    let journal_path = root.join("journal.redb");
    let blobs_path = root.join("blobs");
    std::fs::create_dir_all(&blobs_path).unwrap();

    let journal = RedbJournal::open(&journal_path).unwrap();
    let blob_store: Arc<dyn BlobBackend> = Arc::new(LocalBlobBackend::new(Arc::new(
        BlobStore::open(&blobs_path).unwrap(),
    )));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store.clone()));
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);
    let tracked_fs: Arc<dyn FsPort> = Arc::new(LagoTrackedFs::new(local_fs, tracker, fs_event_tx));

    let session_id = SessionId::from_string("multi-test");
    let branch_id = BranchId::from_string("main");
    tokio::spawn(run_event_writer(
        fs_event_rx,
        journal.clone(),
        session_id.clone(),
        branch_id.clone(),
    ));

    // Write multiple files
    tracked_fs
        .write(&workspace_root.join("a.txt"), b"aaa")
        .unwrap();
    tracked_fs
        .write(&workspace_root.join("b.txt"), b"bbb")
        .unwrap();
    tracked_fs
        .write(&workspace_root.join("c.txt"), b"ccc")
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let query = lago_core::EventQuery::new()
        .session(session_id)
        .branch(branch_id);
    let events = journal.read(query).await.unwrap();

    let file_writes: Vec<_> = events
        .iter()
        .filter(|e| matches!(&e.payload, lago_core::event::EventPayload::FileWrite { .. }))
        .collect();

    assert_eq!(
        file_writes.len(),
        3,
        "expected 3 FileWrite events, got {}",
        file_writes.len()
    );

    let _ = std::fs::remove_dir_all(root);
}

// ── Exec-path (shell) manifest reconciliation (BRO-1477) ───────────────────
//
// These tests wire the production exec-path: a real BashTool (executing via
// LocalCommandRunner) wrapped in ReconcilingTool, draining into a real
// RedbJournal via run_event_writer — mirroring how `arcan serve` wires the
// shell tool in main.rs. They prove that files a *shell command* creates,
// modifies, or deletes land in the lago manifest, blob store, and journal,
// not just files written through the FsPort tool path.

/// Wire a real shell tool wrapped in ReconcilingTool over a real journal.
///
/// Returns `(reconciling_tool, journal, tracker, blob_store, query)`. The
/// background `run_event_writer` is spawned and drains into `journal`.
#[allow(clippy::type_complexity)]
fn build_exec_path(
    root: &Path,
    workspace_root: &Path,
    limits: SnapshotLimits,
) -> (
    ReconcilingTool<BashTool>,
    Arc<dyn lago_core::Journal>,
    Arc<FsTracker>,
    Arc<dyn BlobBackend>,
    lago_core::EventQuery,
) {
    let journal_path = root.join("journal.redb");
    let blobs_path = root.join("blobs");
    std::fs::create_dir_all(&blobs_path).unwrap();

    let journal = RedbJournal::open(&journal_path).expect("open journal");
    let blob_store: Arc<dyn lago_store::BlobBackend> = Arc::new(LocalBlobBackend::new(Arc::new(
        BlobStore::open(&blobs_path).expect("open blob store"),
    )));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store.clone()));
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);

    let session_id = SessionId::from_string("exec-path-test");
    let branch_id = BranchId::from_string("main");
    tokio::spawn(run_event_writer(
        fs_event_rx,
        journal.clone(),
        session_id.clone(),
        branch_id.clone(),
    ));

    let sandbox_policy = SandboxPolicy {
        workspace_root: workspace_root.to_path_buf(),
        shell_enabled: true,
        network: NetworkPolicy::AllowAll,
        allowed_env: BTreeSet::new(),
        max_execution_ms: 10_000,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
    };
    let runner = Box::new(LocalCommandRunner);
    let tool = ReconcilingTool::new(
        BashTool::new(sandbox_policy, runner),
        tracker.clone(),
        fs_event_tx,
        workspace_root.to_path_buf(),
    )
    .with_limits(limits);

    let query = lago_core::EventQuery::new()
        .session(session_id)
        .branch(branch_id);

    (tool, journal, tracker, blob_store, query)
}

fn exec_call(command: &str) -> ToolCall {
    ToolCall {
        call_id: "exec-call".into(),
        tool_name: "bash".into(),
        input: json!({ "command": command }),
        requested_capabilities: vec![],
    }
}

fn exec_ctx() -> ToolContext {
    ToolContext {
        run_id: "exec-run".into(),
        session_id: "exec-path-test".into(),
        iteration: 0,
        ..Default::default()
    }
}

/// Deadline-poll the journal for a payload matching `pred` (no fixed sleep —
/// the writer is a background task draining into a synchronous redb commit).
async fn await_event<F>(
    journal: &Arc<dyn lago_core::Journal>,
    query: &lago_core::EventQuery,
    pred: F,
    what: &str,
) where
    F: Fn(&lago_core::event::EventPayload) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let events = journal.read(query.clone()).await.expect("read journal");
        if events.iter().any(|e| pred(&e.payload)) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {what} in the Lago journal within 5s, got: {:?}",
            events.iter().map(|e| &e.payload).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn exec_path_create_tracks_manifest_blob_and_journal() {
    let root = unique_root("exec-create");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let (tool, journal, tracker, blob_store, query) =
        build_exec_path(&root, &workspace, SnapshotLimits::default());

    // Shell command writes a brand-new file directly in the workspace,
    // bypassing the FsPort write path entirely.
    let result = tool
        .execute(
            &exec_call("printf 'shell wrote this' > shell_made.txt"),
            &exec_ctx(),
        )
        .unwrap();
    assert_eq!(result.output["exit_code"], 0);
    assert!(workspace.join("shell_made.txt").exists());

    // (a) the manifest contains the new file
    let manifest = tracker.manifest();
    let entry = manifest
        .get("/shell_made.txt")
        .expect("manifest should contain the shell-created file");
    assert_eq!(entry.size_bytes, "shell wrote this".len() as u64);

    // (b) its content is in the blob store at the manifest's hash
    assert!(
        blob_store.exists(&entry.blob_hash),
        "blob store should contain the file's content at the manifest hash"
    );
    let stored = blob_store.get(&entry.blob_hash).unwrap();
    assert_eq!(stored, b"shell wrote this");

    // (c) a FileWrite event for it landed in the journal
    await_event(
        &journal,
        &query,
        |p| matches!(p, lago_core::event::EventPayload::FileWrite { path, .. } if path == "/shell_made.txt"),
        "FileWrite for /shell_made.txt",
    )
    .await;

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn exec_path_modify_and_delete_track_journal() {
    let root = unique_root("exec-mod-del");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let (tool, journal, tracker, _blob, query) =
        build_exec_path(&root, &workspace, SnapshotLimits::default());

    // Create, then modify (different length so the size fast-path can't mask
    // the change), then delete — each via a separate shell invocation.
    tool.execute(&exec_call("printf 'v1' > f.txt"), &exec_ctx())
        .unwrap();
    await_event(
        &journal,
        &query,
        |p| matches!(p, lago_core::event::EventPayload::FileWrite { path, .. } if path == "/f.txt"),
        "initial FileWrite for /f.txt",
    )
    .await;

    tool.execute(
        &exec_call("printf 'a much longer version two body' > f.txt"),
        &exec_ctx(),
    )
    .unwrap();

    tool.execute(&exec_call("rm f.txt"), &exec_ctx()).unwrap();
    assert!(!workspace.join("f.txt").exists());

    // The manifest reflects the deletion.
    assert!(!tracker.manifest().exists("/f.txt"));

    // A FileDelete event for it is in the journal.
    await_event(
        &journal,
        &query,
        |p| matches!(p, lago_core::event::EventPayload::FileDelete { path } if path == "/f.txt"),
        "FileDelete for /f.txt",
    )
    .await;

    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn exec_path_bounded_walk_skips_git_and_oversized_file() {
    let root = unique_root("exec-bounded");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    // A .git dir (must be pruned) and a file that will exceed the per-file cap.
    std::fs::create_dir_all(workspace.join(".git/objects")).unwrap();
    std::fs::write(workspace.join(".git/config"), "[core]").unwrap();

    // Tight per-file cap so the "big" file the shell writes is skipped, but a
    // small file is still tracked. File-count cap stays generous.
    let limits = SnapshotLimits {
        max_files: 10_000,
        max_file_bytes: 16,
    };
    let (tool, journal, tracker, _blob, query) = build_exec_path(&root, &workspace, limits);

    // One shell command writes both a small (tracked) and a large (skipped)
    // file plus touches inside .git (pruned). Must not panic; tool succeeds.
    let result = tool
        .execute(
            &exec_call(
                "printf 'small' > ok.txt; \
                 head -c 4096 /dev/zero > big.bin; \
                 printf 'junk' > .git/objects/loose",
            ),
            &exec_ctx(),
        )
        .unwrap();
    assert_eq!(
        result.output["exit_code"], 0,
        "shell tool must still succeed"
    );

    // Small file tracked …
    await_event(
        &journal,
        &query,
        |p| matches!(p, lago_core::event::EventPayload::FileWrite { path, .. } if path == "/ok.txt"),
        "FileWrite for /ok.txt",
    )
    .await;

    // … oversized + .git contents are NOT in the manifest.
    let manifest = tracker.manifest();
    assert!(manifest.exists("/ok.txt"));
    assert!(
        !manifest.exists("/big.bin"),
        "oversized file must be skipped under the per-file cap"
    );
    assert!(!manifest.exists("/.git"));
    assert!(!manifest.exists("/.git/config"));
    assert!(!manifest.exists("/.git/objects/loose"));

    let _ = std::fs::remove_dir_all(root);
}

/// Like [`build_exec_path`] but baselines the tracker against the
/// already-populated `workspace_root` (via `FsTracker::with_baseline`),
/// mirroring how `main.rs` seeds the tracker at boot. Used to prove the
/// first shell exec does NOT emit spurious writes for pre-existing files.
#[allow(clippy::type_complexity)]
fn build_exec_path_baselined(
    root: &Path,
    workspace_root: &Path,
    limits: SnapshotLimits,
) -> (
    ReconcilingTool<BashTool>,
    Arc<dyn lago_core::Journal>,
    Arc<FsTracker>,
    Arc<dyn BlobBackend>,
    lago_core::EventQuery,
) {
    let journal_path = root.join("journal.redb");
    let blobs_path = root.join("blobs");
    std::fs::create_dir_all(&blobs_path).unwrap();

    let journal = RedbJournal::open(&journal_path).expect("open journal");
    let blob_store: Arc<dyn lago_store::BlobBackend> = Arc::new(LocalBlobBackend::new(Arc::new(
        BlobStore::open(&blobs_path).expect("open blob store"),
    )));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // Baseline against the populated workspace using the SAME limits the
    // reconciler will use — exactly the main.rs wiring under test.
    let tracker = Arc::new(
        FsTracker::with_baseline(workspace_root, blob_store.clone(), limits)
            .expect("baseline snapshot"),
    );
    let (fs_event_tx, fs_event_rx) = tokio::sync::mpsc::channel(1000);

    let session_id = SessionId::from_string("exec-path-baseline");
    let branch_id = BranchId::from_string("main");
    tokio::spawn(run_event_writer(
        fs_event_rx,
        journal.clone(),
        session_id.clone(),
        branch_id.clone(),
    ));

    let sandbox_policy = SandboxPolicy {
        workspace_root: workspace_root.to_path_buf(),
        shell_enabled: true,
        network: NetworkPolicy::AllowAll,
        allowed_env: BTreeSet::new(),
        max_execution_ms: 10_000,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
    };
    let runner = Box::new(LocalCommandRunner);
    let tool = ReconcilingTool::new(
        BashTool::new(sandbox_policy, runner),
        tracker.clone(),
        fs_event_tx,
        workspace_root.to_path_buf(),
    )
    .with_limits(limits);

    let query = lago_core::EventQuery::new()
        .session(session_id)
        .branch(branch_id);

    (tool, journal, tracker, blob_store, query)
}

#[tokio::test]
async fn exec_path_baseline_suppresses_spurious_preexisting_writes() {
    // Must-fix #1 (end-to-end, real wiring): a tracker baselined against a
    // populated workspace must NOT emit a FileWrite for pre-existing files on
    // the first shell exec. Only the file the command actually created should
    // reach the journal.
    let root = unique_root("exec-baseline");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("sub")).unwrap();

    // Pre-populate BEFORE building the tracker — these are the "live CWD" files
    // that the buggy empty-manifest seed would have diffed into existence.
    std::fs::write(workspace.join("preexisting.txt"), "i was here first").unwrap();
    std::fs::write(workspace.join("sub/nested.txt"), "me too").unwrap();

    let (tool, journal, tracker, _blob, query) =
        build_exec_path_baselined(&root, &workspace, SnapshotLimits::default());

    // Baseline already recorded the pre-existing files (no events emitted).
    assert!(tracker.manifest().exists("/preexisting.txt"));
    assert!(tracker.manifest().exists("/sub/nested.txt"));

    // A shell command creates ONE genuinely new file, touching nothing else.
    let result = tool
        .execute(&exec_call("printf 'brand new' > fresh.txt"), &exec_ctx())
        .unwrap();
    assert_eq!(result.output["exit_code"], 0);

    // The new file's FileWrite reaches the journal (proves the reconcile ran).
    await_event(
        &journal,
        &query,
        |p| matches!(p, lago_core::event::EventPayload::FileWrite { path, .. } if path == "/fresh.txt"),
        "FileWrite for /fresh.txt",
    )
    .await;

    // Now the negative assertion is deterministic: the reconcile that emitted
    // /fresh.txt is the SAME pass that would have emitted the pre-existing files
    // had the baseline been broken. None must appear in the journal.
    let events = journal.read(query.clone()).await.expect("read journal");
    let spurious: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.payload {
            lago_core::event::EventPayload::FileWrite { path, .. }
                if path == "/preexisting.txt" || path == "/sub/nested.txt" =>
            {
                Some(path.as_str())
            }
            _ => None,
        })
        .collect();
    assert!(
        spurious.is_empty(),
        "baseline must suppress writes for pre-existing files; got spurious: {spurious:?}"
    );

    let _ = std::fs::remove_dir_all(root);
}
