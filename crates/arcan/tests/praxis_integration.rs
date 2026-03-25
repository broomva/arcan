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
use arcan_lago::{LagoTrackedFs, run_event_writer};
use arcand::canonical::create_canonical_router;
use arcand::mock::MockProvider;
use lago_aios_eventstore_adapter::LagoAiosEventStoreAdapter;
use lago_core::{BranchId, SessionId};
use lago_fs::{FsTracker, Manifest};
use lago_journal::RedbJournal;
use lago_store::BlobStore;
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
    let blob_store = Arc::new(BlobStore::open(&blobs_path).expect("open blob store"));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // --- Lago-tracked filesystem (O(1) write tracking) ---
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store));
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
    let blob_store = Arc::new(BlobStore::open(&blobs_path).expect("open blob store"));
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    // Set up tracked filesystem
    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store));
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

    // Give the background writer time to flush
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify the file exists on disk
    assert!(workspace_root.join("tracked.txt").exists());

    // Verify the event was persisted in the Lago journal
    let query = lago_core::EventQuery::new()
        .session(session_id)
        .branch(branch_id);
    let events = journal.read(query).await.expect("read journal");

    assert!(
        !events.is_empty(),
        "expected at least one event in the Lago journal after tracked write"
    );

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
    let blob_store = Arc::new(BlobStore::open(&blobs_path).unwrap());
    let journal: Arc<dyn lago_core::Journal> = Arc::new(journal);

    let fs_policy = FsPolicy::new(workspace_root.clone());
    let local_fs = LocalFs::new(fs_policy);
    let tracker = Arc::new(FsTracker::new(Manifest::new(), blob_store));
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
