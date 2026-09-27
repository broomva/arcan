//! Owner-scoped memory over the authenticated HTTP plane (BRO-1491).
//!
//! Its own test binary because it turns bearer auth on for the whole process
//! (`ARCAN_JWT_SECRET`), which would 401 every unauthenticated test in
//! `canonical_api.rs`.
//!
//! The runtime runs the real `ArcanHarnessAdapter` with the real praxis
//! memory tools scoped by `MemoryLocation::PerOwner`, so a proposed
//! `read_memory` / `write_memory` travels the same path as in `arcan serve`:
//! HTTP → session-owner route layer → kernel tick → harness → tool → owner
//! binding → owner directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::{SystemTime, UNIX_EPOCH};

use aios_events::{EventJournal, EventStreamHub, FileEventStore};
use aios_policy::{ApprovalQueue, SessionPolicyEngine};
use aios_protocol::owner_scope::{MemoryLocation, bind_session_owner};
use aios_protocol::{
    ApprovalPort, EventStorePort, KernelResult, ModelCompletion, ModelCompletionRequest,
    ModelDirective, ModelProviderPort, ModelStopReason, PolicyGatePort, PolicySet, ToolHarnessPort,
};
use aios_runtime::{KernelRuntime, RuntimeConfig};
use arcan_aios_adapters::tools::ArcanHarnessAdapter;
use arcan_core::error::CoreError;
use arcan_core::runtime::{Provider, ProviderFactory, SwappableProviderHandle, ToolRegistry};
use arcan_harness::bridge::PraxisToolBridge;
use arcand::canonical::create_canonical_router_with_skills;
use async_trait::async_trait;
use praxis_tools::memory::{ReadMemoryTool, WriteMemoryTool};
use reqwest::StatusCode;
use serde_json::{Value, json};

const SECRET: &str = "owner-scoped-memory-test-secret";

fn enable_bearer_auth() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: set once, before any server in this binary reads it; every
        // test in this binary wants the same value.
        unsafe { std::env::set_var("ARCAN_JWT_SECRET", SECRET) };
    });
}

fn token(sub: &str) -> String {
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({ "sub": sub, "email": format!("{sub}@test"), "exp": exp }),
        &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

struct StubProvider;
impl Provider for StubProvider {
    fn name(&self) -> &str {
        "test-stub"
    }
    fn complete(
        &self,
        _: &arcan_core::runtime::ProviderRequest,
    ) -> Result<arcan_core::protocol::ModelTurn, CoreError> {
        Err(CoreError::Provider("stub".into()))
    }
}
struct StubFactory;
impl ProviderFactory for StubFactory {
    fn build(&self, _spec: &str) -> Result<Arc<dyn Provider>, CoreError> {
        Ok(Arc::new(StubProvider))
    }
    fn available_providers(&self) -> Vec<String> {
        vec!["test-stub".into()]
    }
}

/// Records every system prompt, answers with a fixed text turn.
#[derive(Clone, Default)]
struct CapturingProvider {
    prompts: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait]
impl ModelProviderPort for CapturingProvider {
    async fn complete(&self, request: ModelCompletionRequest) -> KernelResult<ModelCompletion> {
        self.prompts.lock().unwrap().push((
            request.session_id.as_str().to_owned(),
            request.system_prompt.clone().unwrap_or_default(),
        ));
        Ok(ModelCompletion {
            provider: "test".into(),
            model: "test-model".into(),
            llm_call_record: None,
            directives: vec![ModelDirective::Message {
                role: "assistant".into(),
                content: "ok".into(),
            }],
            stop_reason: ModelStopReason::Completed,
            usage: None,
            final_answer: Some("ok".into()),
        })
    }
}

struct Daemon {
    base: String,
    runtime: Arc<KernelRuntime>,
    root: PathBuf,
    prompts: Arc<Mutex<Vec<(String, String)>>>,
    client: reqwest::Client,
    server: tokio::task::JoinHandle<()>,
    _tmp: tempfile::TempDir,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn start_daemon() -> Daemon {
    enable_bearer_auth();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let location = MemoryLocation::for_deployment(&root, true);

    let mut registry = ToolRegistry::default();
    registry.register(PraxisToolBridge::new(ReadMemoryTool::scoped(
        location.clone(),
    )));
    registry.register(PraxisToolBridge::new(WriteMemoryTool::scoped(
        location.clone(),
    )));
    let harness: Arc<dyn ToolHarnessPort> =
        Arc::new(ArcanHarnessAdapter::new(registry).with_sessions_dir(root.join("sessions")));

    let journal = Arc::new(EventJournal::new(
        Arc::new(FileEventStore::new(root.join("kernel"))),
        EventStreamHub::new(1024),
    ));
    let event_store: Arc<dyn EventStorePort> = journal;
    let policy_gate: Arc<dyn PolicyGatePort> =
        Arc::new(SessionPolicyEngine::new(PolicySet::default()));
    let approvals: Arc<dyn ApprovalPort> = Arc::new(ApprovalQueue::default());
    let provider = CapturingProvider::default();
    let prompts = provider.prompts.clone();
    let runtime = Arc::new(KernelRuntime::new(
        RuntimeConfig::new(root.clone()),
        event_store,
        Arc::new(provider),
        harness,
        approvals,
        policy_gate,
    ));

    let router = create_canonical_router_with_skills(
        runtime.clone(),
        Arc::new(std::sync::RwLock::new(
            Arc::new(StubProvider) as Arc<dyn Provider>
        )) as SwappableProviderHandle,
        Arc::new(StubFactory),
        None,
        None,
        Vec::new(),
        None,
        root.clone(),
        Some(root.join("workspace")),
        None,
        None,
        false,
        None,
        location,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Daemon {
        base: format!("http://{addr}"),
        runtime,
        root,
        prompts,
        client: reqwest::Client::new(),
        server,
        _tmp: tmp,
    }
}

impl Daemon {
    async fn create_session(&self, as_user: &str, session_id: &str) -> reqwest::Response {
        self.client
            .post(format!("{}/sessions", self.base))
            .bearer_auth(token(as_user))
            // A client-supplied owner must be ignored in multi-tenant mode.
            .json(&json!({ "session_id": session_id, "owner": "victim" }))
            .send()
            .await
            .unwrap()
    }

    async fn run_tool(
        &self,
        as_user: &str,
        session_id: &str,
        tool: &str,
        input: Value,
    ) -> reqwest::Response {
        self.client
            .post(format!("{}/sessions/{session_id}/runs", self.base))
            .bearer_auth(token(as_user))
            .json(&json!({
                "objective": "memory op",
                "proposed_tool": { "tool_name": tool, "input": input, "requested_capabilities": [] }
            }))
            .send()
            .await
            .unwrap()
    }

    async fn events_text(&self, as_user: &str, session_id: &str) -> (StatusCode, String) {
        let response = self
            .client
            .get(format!("{}/sessions/{session_id}/events", self.base))
            .bearer_auth(token(as_user))
            .send()
            .await
            .unwrap();
        (response.status(), response.text().await.unwrap())
    }

    fn owner_memory(&self, owner: &str) -> PathBuf {
        self.root.join("owners").join(owner).join("memory")
    }

    fn prompts_for(&self, session_id: &str) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|(sid, _)| sid == session_id)
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

#[tokio::test]
async fn owner_a_cannot_read_or_write_owner_b_memory() {
    let d = start_daemon().await;
    assert_eq!(
        d.create_session("bob", "sess-bob").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        d.create_session("alice", "sess-alice").await.status(),
        StatusCode::OK
    );

    // The owner is the token's subject, never the body's `owner` field.
    let bindings = d.root.join("session-owners");
    assert_eq!(
        read_file(&bindings.join("sess-bob")).as_deref(),
        Some("bob")
    );
    assert_eq!(
        read_file(&bindings.join("sess-alice")).as_deref(),
        Some("alice")
    );

    // Bob writes a secret into his memory.
    let r = d
        .run_tool(
            "bob",
            "sess-bob",
            "write_memory",
            json!({"key": "secret", "content": "BOB-SECRET-7f3a"}),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        read_file(&d.owner_memory("bob").join("secret.md")).as_deref(),
        Some("BOB-SECRET-7f3a")
    );

    // Arm 1 — Alice reads the same key in her own session: her memory, not Bob's.
    let r = d
        .run_tool(
            "alice",
            "sess-alice",
            "read_memory",
            json!({"key": "secret"}),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    let (status, alice_events) = d.events_text("alice", "sess-alice").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        alice_events.contains("read_memory"),
        "the read must have executed"
    );
    assert!(
        !alice_events.contains("BOB-SECRET-7f3a"),
        "alice's read returned bob's memory"
    );

    // Positive control — Bob reading his own key does see it.
    d.run_tool("bob", "sess-bob", "read_memory", json!({"key": "secret"}))
        .await;
    let (_, bob_events) = d.events_text("bob", "sess-bob").await;
    assert!(
        bob_events.contains("BOB-SECRET-7f3a"),
        "positive control: bob sees his own memory"
    );

    // Arm 2 — Alice writes the same key: Bob's file is untouched.
    d.run_tool(
        "alice",
        "sess-alice",
        "write_memory",
        json!({"key": "secret", "content": "alice-note"}),
    )
    .await;
    assert_eq!(
        read_file(&d.owner_memory("bob").join("secret.md")).as_deref(),
        Some("BOB-SECRET-7f3a")
    );
    assert_eq!(
        read_file(&d.owner_memory("alice").join("secret.md")).as_deref(),
        Some("alice-note")
    );

    // Arm 3 — Alice drives Bob's session directly: every session route is 404.
    let r = d
        .run_tool("alice", "sess-bob", "read_memory", json!({"key": "secret"}))
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let (status, body) = d.events_text("alice", "sess-bob").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("BOB-SECRET-7f3a"));
    let r = d
        .client
        .post(format!("{}/sessions/sess-bob/messages", d.base))
        .bearer_auth(token("alice"))
        .json(&json!({ "content": "read my memory" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // Arm 3b — every other session route, including the POSTs next to the
    // exempt `/runs`, answers 404 to another owner (positive control: bob).
    for (method, route) in [
        ("POST", "branches"),
        ("GET", "branches"),
        ("POST", "approvals/some-approval"),
        ("POST", "signal"),
        ("GET", "state"),
        ("PATCH", "identity"),
    ] {
        let send = |user: &str| {
            d.client
                .request(
                    method.parse().unwrap(),
                    format!("{}/sessions/sess-bob/{route}", d.base),
                )
                .bearer_auth(token(user))
                .json(&json!({ "name": "x", "approved": true, "user_id": user, "signal": "x" }))
                .send()
        };
        let as_alice = send("alice").await.unwrap();
        assert_eq!(
            as_alice.status(),
            StatusCode::NOT_FOUND,
            "{method} {route} as alice"
        );
        let as_bob = send("bob").await.unwrap().status();
        assert_ne!(
            as_bob,
            StatusCode::NOT_FOUND,
            "{method} {route}: the owner must get past the layer"
        );
    }

    // Arm 3c — a grammar-invalid alias of bob's id never passes the layer
    // (FileEventStore resolves `./sess-bob` to bob's own event file).
    for path in [
        ".%2Fsess-bob",
        "sess-bob%2F.",
        "x%2F..%2Fsess-bob",
        "sess-bob%00",
    ] {
        for route in ["events", "state"] {
            let r = d
                .client
                .get(format!("{}/sessions/{path}/{route}", d.base))
                .bearer_auth(token("alice"))
                .send()
                .await
                .unwrap();
            let status = r.status();
            let body = r.text().await.unwrap();
            assert!(
                !body.contains("BOB-SECRET-7f3a"),
                "{path}/{route} leaked: {body}"
            );
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}/{route}");
        }
    }

    // Arm 4 — Alice cannot re-create (and so claim) Bob's session id.
    assert_eq!(
        d.create_session("alice", "sess-bob").await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        read_file(&bindings.join("sess-bob")).as_deref(),
        Some("bob")
    );
}

#[tokio::test]
async fn the_prompt_carries_only_the_session_owners_memory() {
    let d = start_daemon().await;
    // Seed each owner's memory, and the legacy shared store.
    std::fs::create_dir_all(d.owner_memory("alice")).unwrap();
    std::fs::create_dir_all(d.owner_memory("bob")).unwrap();
    std::fs::create_dir_all(d.root.join("memory")).unwrap();
    std::fs::write(d.owner_memory("alice").join("a.md"), "ALICE-MEM-11").unwrap();
    std::fs::write(d.owner_memory("bob").join("b.md"), "BOB-MEM-22").unwrap();
    std::fs::write(d.root.join("memory/legacy.md"), "LEGACY-SHARED-33").unwrap();

    for (user, sid) in [("alice", "p-alice"), ("bob", "p-bob")] {
        assert_eq!(d.create_session(user, sid).await.status(), StatusCode::OK);
        let r = d
            .client
            .post(format!("{}/sessions/{sid}/runs", d.base))
            .bearer_auth(token(user))
            .json(&json!({ "objective": "hello" }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    let alice = d.prompts_for("p-alice").join("\n");
    let bob = d.prompts_for("p-bob").join("\n");
    assert!(
        alice.contains("ALICE-MEM-11"),
        "positive control: alice sees her memory"
    );
    assert!(
        !alice.contains("BOB-MEM-22"),
        "alice's prompt carries bob's memory"
    );
    assert!(
        bob.contains("BOB-MEM-22"),
        "positive control: bob sees his memory"
    );
    assert!(
        !bob.contains("ALICE-MEM-11"),
        "bob's prompt carries alice's memory"
    );
    for prompt in [&alice, &bob] {
        assert!(
            !prompt.contains("LEGACY-SHARED-33"),
            "legacy shared memory leaked into a tenant prompt"
        );
    }
}

#[tokio::test]
async fn one_owner_shares_memory_across_sessions() {
    let d = start_daemon().await;
    assert_eq!(
        d.create_session("alice", "s-1").await.status(),
        StatusCode::OK
    );
    // A session created implicitly by a run is also bound to its caller.
    d.run_tool(
        "alice",
        "s-1",
        "write_memory",
        json!({"key": "prefs", "content": "ALICE-PREF-44"}),
    )
    .await;
    let r = d
        .run_tool("alice", "s-2-auto", "read_memory", json!({"key": "prefs"}))
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        read_file(&d.root.join("session-owners/s-2-auto")).as_deref(),
        Some("alice")
    );
    let (_, events) = d.events_text("alice", "s-2-auto").await;
    assert!(
        events.contains("ALICE-PREF-44"),
        "same owner, second session, same memory"
    );
}

#[tokio::test]
async fn malicious_subjects_are_rejected_and_create_nothing() {
    let d = start_daemon().await;
    for sub in ["../bob", "..", "a/b", "alice@example.com", "-x", "%2e%2e"] {
        let r = d.create_session(sub, "evil-session").await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "subject {sub:?}");
        let r = d
            .run_tool(sub, "evil-auto", "read_memory", json!({"key": "k"}))
            .await;
        assert_eq!(
            r.status(),
            StatusCode::FORBIDDEN,
            "subject {sub:?} on the auto-create path"
        );
    }
    assert!(!d.root.join("session-owners/evil-session").exists());
    assert!(!d.root.join("sessions/evil-session").exists());
    assert!(!d.root.join("bob").exists());
    let owners = d.root.join("owners");
    assert!(!owners.exists() || std::fs::read_dir(&owners).unwrap().next().is_none());
}

#[tokio::test]
async fn memory_export_is_limited_to_the_caller() {
    let d = start_daemon().await;
    // No journal is configured here, so an authorized call reaches the
    // handler and answers 503; a refused one never gets that far.
    let r = d
        .client
        .get(format!("{}/user/memory/export?user_id=bob", d.base))
        .bearer_auth(token("alice"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = d
        .client
        .get(format!("{}/user/memory/export?user_id=bob", d.base))
        .bearer_auth(token("bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "positive control"
    );
    let r = d
        .client
        .post(format!("{}/user/memory/migrate-to-pro", d.base))
        .bearer_auth(token("alice"))
        .json(&json!({ "user_id": "bob" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_pre_existing_workspace_can_be_claimed_by_nobody() {
    let d = start_daemon().await;
    // A workspace from before owner scoping, with no binding.
    std::fs::create_dir_all(d.root.join("sessions/legacy-1")).unwrap();
    assert_eq!(
        d.create_session("alice", "legacy-1").await.status(),
        StatusCode::CONFLICT
    );
    let r = d
        .run_tool("alice", "legacy-1", "read_memory", json!({"key": "notes"}))
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(
        !d.root.join("session-owners/legacy-1").exists(),
        "a pre-existing workspace must not be claimed by the first caller"
    );
}

/// P20 round 3: an event stream that exists without a workspace (a
/// pre-owner-scoping session whose directory is gone) must not be claimable.
#[tokio::test]
async fn a_pre_existing_event_stream_without_a_workspace_can_be_claimed_by_nobody() {
    use aios_protocol::{ModelRouting, SessionId};
    let d = start_daemon().await;
    d.runtime
        .create_session_with_id(
            SessionId::from_string("legacy-bob"),
            "bob",
            PolicySet::default(),
            ModelRouting::default(),
        )
        .await
        .unwrap();
    std::fs::remove_dir_all(d.root.join("sessions/legacy-bob")).unwrap();
    let events = d
        .runtime
        .read_events(&SessionId::from_string("legacy-bob"), 0, 1)
        .await
        .unwrap();
    assert!(!events.is_empty(), "control: the stream exists");
    assert_eq!(
        d.create_session("alice", "legacy-bob").await.status(),
        StatusCode::CONFLICT
    );
    let r = d
        .run_tool("alice", "legacy-bob", "read_memory", json!({"key": "k"}))
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let (status, _) = d.events_text("alice", "legacy-bob").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!d.root.join("session-owners/legacy-bob").exists());
}

/// P20 round 3: the streams the daemon writes cross-tenant data into
/// (`"default"`: filesystem events, Nous judgments) are claimable by nobody,
/// even before their first event is written.
#[tokio::test]
async fn the_daemons_system_streams_can_be_claimed_by_nobody() {
    let d = start_daemon().await;
    for sid in arcand::canonical::RESERVED_SYSTEM_SESSIONS {
        assert_eq!(
            d.create_session("alice", sid).await.status(),
            StatusCode::CONFLICT,
            "{sid}"
        );
        let r = d
            .run_tool("alice", sid, "read_memory", json!({"key": "k"}))
            .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{sid}");
        let (status, _) = d.events_text("alice", sid).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{sid}");
        assert!(!d.root.join("session-owners").join(sid).exists(), "{sid}");
    }
}

/// P20 round 4: a case variant of a reserved or owned id never reaches that
/// session (only meaningful on a case-insensitive data dir, e.g. macOS APFS).
#[tokio::test]
async fn a_case_variant_id_never_reaches_another_session() {
    let d = start_daemon().await;
    assert_eq!(
        d.create_session("bob", "sess-bob").await.status(),
        StatusCode::OK
    );
    d.run_tool(
        "bob",
        "sess-bob",
        "write_memory",
        json!({"key": "secret", "content": "BOB-CASE-SECRET"}),
    )
    .await;
    let insensitive = d.root.join("session-owners/SESS-BOB").exists();
    // Alice claims case variants of a reserved stream and of bob's session.
    let default_claim = d.create_session("alice", "DEFAULT").await.status();
    let bob_claim = d.create_session("alice", "SESS-BOB").await.status();
    if insensitive {
        assert_eq!(
            bob_claim,
            StatusCode::CONFLICT,
            "SESS-BOB aliases sess-bob here"
        );
    }
    let _ = default_claim;
    for (sid, route) in [
        ("default", "events"),
        ("sess-bob", "events"),
        ("sess-bob", "state"),
    ] {
        let r = d
            .client
            .get(format!("{}/sessions/{sid}/{route}", d.base))
            .bearer_auth(token("alice"))
            .send()
            .await
            .unwrap();
        let status = r.status();
        let body = r.text().await.unwrap();
        assert!(
            !body.contains("BOB-CASE-SECRET"),
            "{sid}/{route} leaked: {body}"
        );
        assert_eq!(status, StatusCode::NOT_FOUND, "{sid}/{route}");
    }
    for sid in ["SESS-BOB", "Sess-Bob"] {
        let r = d
            .client
            .get(format!("{}/sessions/{sid}/events", d.base))
            .bearer_auth(token("alice"))
            .send()
            .await
            .unwrap();
        let body = r.text().await.unwrap();
        assert!(!body.contains("BOB-CASE-SECRET"), "{sid} leaked: {body}");
    }
}

/// Reviewer PoC 1 (P20 round 1): a stream opened on an id before its owner
/// creates it must not deliver that owner's events.
#[tokio::test]
async fn a_stream_opened_before_the_session_exists_is_refused() {
    let d = start_daemon().await;
    let stream = |user: &'static str| {
        d.client
            .get(format!("{}/sessions/victim-chat/events/stream", d.base))
            .bearer_auth(token(user))
            .send()
    };
    assert_eq!(
        stream("mallory").await.unwrap().status(),
        StatusCode::NOT_FOUND,
        "pre-subscription must be refused"
    );
    assert_eq!(
        d.create_session("bob", "victim-chat").await.status(),
        StatusCode::OK
    );
    d.run_tool(
        "bob",
        "victim-chat",
        "write_memory",
        json!({"key": "secret", "content": "BOB-SECRET-POC"}),
    )
    .await;
    assert_eq!(
        stream("mallory").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    // Positive control: the owner's own stream opens.
    assert_eq!(stream("bob").await.unwrap().status(), StatusCode::OK);
}

/// Reviewer PoC 2 (P20 round 1): a server-generated session must never be
/// visible without its owner binding. A poller that opens a stream on every
/// new session it sees in `GET /sessions` must never get one open.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_server_generated_session_is_never_visible_unbound() {
    let d = Arc::new(start_daemon().await);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let poller = {
        let d = d.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            let mut seen = std::collections::HashSet::new();
            let mut attempts = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let list: Value = d
                    .client
                    .get(format!("{}/sessions", d.base))
                    .bearer_auth(token("mallory"))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap_or(Value::Null);
                let ids: Vec<String> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s["session_id"].as_str().map(str::to_owned))
                    .collect();
                for sid in ids {
                    if seen.insert(sid.clone()) {
                        let status = d
                            .client
                            .get(format!("{}/sessions/{sid}/events/stream", d.base))
                            .bearer_auth(token("mallory"))
                            .send()
                            .await
                            .unwrap()
                            .status();
                        attempts.push((sid, status));
                    }
                }
            }
            attempts
        })
    };
    for _ in 0..25 {
        let r = d
            .client
            .post(format!("{}/sessions", d.base))
            .bearer_auth(token("bob"))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let manifest: Value = r.json().await.unwrap();
        let sid = manifest["session_id"].as_str().unwrap().to_owned();
        assert_eq!(
            read_file(&d.root.join("session-owners").join(&sid)).as_deref(),
            Some("bob"),
            "a server-generated session is bound to its creator"
        );
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let attempts = poller.await.unwrap();
    assert!(
        !attempts.is_empty(),
        "the poller must have seen sessions (vacuity check)"
    );
    for (sid, status) in &attempts {
        assert_eq!(
            *status,
            StatusCode::NOT_FOUND,
            "mallory opened a stream on {sid}"
        );
    }
}

#[tokio::test]
async fn the_identity_upgrade_cannot_name_another_user() {
    let d = start_daemon().await;
    assert_eq!(
        d.create_session("mallory", "m-1").await.status(),
        StatusCode::OK
    );
    let patch = |user: &'static str| {
        d.client
            .patch(format!("{}/sessions/m-1/identity", d.base))
            .bearer_auth(token("mallory"))
            .json(&json!({ "user_id": user }))
            .send()
    };
    assert_eq!(patch("bob").await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(
        patch("mallory").await.unwrap().status(),
        StatusCode::OK,
        "positive control"
    );
}

#[tokio::test]
async fn the_substrate_plane_cannot_drive_an_owned_session() {
    use arcan_substrate_proto::arcan::v1::agent_substrate_server::AgentSubstrate;
    use arcan_substrate_proto::arcan::v1::{CreateAgentReq, DispatchMessageReq};
    let d = start_daemon().await;
    assert_eq!(
        d.create_session("bob", "sub-bob").await.status(),
        StatusCode::OK
    );
    // A second runtime over the same data dir stands in for the substrate
    // plane: the guard reads the binding from the data dir, not the runtime.
    let runtime = Arc::new(KernelRuntime::new(
        RuntimeConfig::new(d.root.clone()),
        Arc::new(EventJournal::new(
            Arc::new(FileEventStore::new(d.root.join("kernel-sub"))),
            EventStreamHub::new(16),
        )),
        Arc::new(CapturingProvider::default()),
        Arc::new(ArcanHarnessAdapter::new(ToolRegistry::default())),
        Arc::new(ApprovalQueue::default()),
        Arc::new(SessionPolicyEngine::new(PolicySet::default())),
    ));
    let service = arcand::substrate::SubstrateService::new(runtime);
    let sid = |v: &str| Some(aios_proto::aios::v1::SessionId { value: v.into() });
    let err = service
        .create_agent(tonic::Request::new(CreateAgentReq {
            sid: sid("sub-bob"),
            label: String::new(),
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    let err = service
        .dispatch_message(tonic::Request::new(DispatchMessageReq {
            sid: sid("sub-bob"),
            content: "read bob's memory".into(),
            tool_definitions: vec![],
            branch: String::new(),
        }))
        .await
        .err()
        .expect("dispatch on an owned session must be refused");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
    // The daemon's reserved cross-tenant streams are refused too.
    for reserved in arcand::canonical::RESERVED_SYSTEM_SESSIONS {
        let err = service
            .create_agent(tonic::Request::new(CreateAgentReq {
                sid: sid(reserved),
                label: String::new(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied, "{reserved}");
    }
    // Positive control: the substrate creates its own sessions, claimed as
    // permanently unowned before they exist.
    service
        .create_agent(tonic::Request::new(CreateAgentReq {
            sid: sid("sub-free"),
            label: String::new(),
        }))
        .await
        .expect("unowned sessions stay available to the substrate plane");
    assert_eq!(
        aios_protocol::owner_scope::read_binding(&d.root, "sub-free"),
        Ok(aios_protocol::owner_scope::Binding::Unowned)
    );
    // Nobody can later take the substrate's session as an owner...
    assert!(bind_session_owner(&d.root, "sub-free", "carol").is_err());
    assert_eq!(
        d.create_session("carol", "sub-free").await.status(),
        StatusCode::CONFLICT
    );
    // ...and no authenticated HTTP caller can read it.
    let (status, _) = d.events_text("carol", "sub-free").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let r = d
        .run_tool("carol", "sub-free", "read_memory", json!({"key": "k"}))
        .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
