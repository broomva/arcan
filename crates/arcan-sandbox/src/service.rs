//! `SandboxService` — session-scoped sandbox orchestration layer (BRO-253).
//!
//! `SandboxService` is the single entry point for all Arcan tool code that
//! needs sandbox execution. It owns a `SandboxRegistry`, enforces per-tier
//! `SandboxServicePolicy`, tracks live sessions in a `DashMap`, and drives
//! the idle-reaper background task.
//!
//! # Architecture
//!
//! ```text
//!   BashTool / FilesystemTool (BRO-259)
//!          │
//!          ▼
//!   SandboxService (this module)
//!     ├─ SandboxRegistry  ──► Arc<dyn SandboxProvider>
//!     ├─ DashMap<session_id, SandboxSession>
//!     └─ Arc<dyn SandboxEventSink>
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use tracing::{debug, info, warn};

use crate::capability::SandboxCapabilitySet;
use crate::error::SandboxError;
use crate::event::{SandboxEvent, SandboxEventKind};
use crate::provider::SandboxProvider;
use crate::sink::SandboxEventSink;
use crate::types::{
    ExecRequest, ExecResult, PersistencePolicy, SandboxHandle, SandboxId, SandboxResources,
    SandboxSpec, SandboxStatus,
};

// ── SandboxServicePolicy ─────────────────────────────────────────────────────

/// Per-tier sandbox execution policy.
///
/// Distinct from `praxis_core::sandbox::SandboxPolicy` (low-level tool
/// execution). This governs the orchestration layer: provider routing,
/// concurrency limits, and idle-reaper thresholds.
#[derive(Debug, Clone)]
pub struct SandboxServicePolicy {
    /// Ordered list of provider names tried when routing a new sandbox.
    pub allowed_providers: Vec<&'static str>,
    /// Default persistence strategy written into new sandbox specs.
    pub persistence: PersistencePolicy,
    /// Maximum concurrent sandboxes per agent session.
    pub max_concurrent: u32,
    /// Inactivity seconds before the idle reaper auto-snapshots a sandbox.
    pub idle_timeout_secs: u64,
    /// Seconds from creation after which the reaper force-destroys a sandbox.
    pub max_lifetime_secs: u64,
    /// Compute resources applied to every new sandbox spec.
    pub resources: SandboxResources,
}

impl SandboxServicePolicy {
    /// Free-tier: local-only, ephemeral, 1 concurrent, 30 s idle.
    pub fn free() -> Self {
        Self {
            allowed_providers: vec!["local", "bubblewrap"],
            persistence: PersistencePolicy::Ephemeral,
            max_concurrent: 1,
            idle_timeout_secs: 30,
            max_lifetime_secs: 3_600,
            resources: SandboxResources::default(),
        }
    }

    /// Pro-tier: vercel|e2b|local, persistent 300 s idle, 3 concurrent.
    pub fn pro() -> Self {
        Self {
            allowed_providers: vec!["vercel", "e2b", "local"],
            persistence: PersistencePolicy::Persistent {
                idle_timeout_secs: 300,
            },
            max_concurrent: 3,
            idle_timeout_secs: 300,
            max_lifetime_secs: 86_400,
            resources: SandboxResources {
                vcpus: 2,
                memory_mb: 2_048,
                disk_mb: 8_192,
                timeout_secs: 300,
            },
        }
    }

    /// Enterprise-tier: e2b|vercel|local, persistent 3600 s idle, 10 concurrent.
    pub fn enterprise() -> Self {
        Self {
            allowed_providers: vec!["e2b", "vercel", "local"],
            persistence: PersistencePolicy::Persistent {
                idle_timeout_secs: 3_600,
            },
            max_concurrent: 10,
            idle_timeout_secs: 3_600,
            max_lifetime_secs: 604_800, // 7 days
            resources: SandboxResources {
                vcpus: 4,
                memory_mb: 8_192,
                disk_mb: 32_768,
                timeout_secs: 3_600,
            },
        }
    }
}

impl Default for SandboxServicePolicy {
    fn default() -> Self {
        Self::free()
    }
}

// ── SandboxRegistry ──────────────────────────────────────────────────────────

/// Named registry of `SandboxProvider` implementations.
///
/// Providers are registered by their `name()` value (e.g. `"local"`,
/// `"vercel"`).  The registry routes `create()` calls based on the ordered
/// `allowed_providers` list in `SandboxServicePolicy`.
pub struct SandboxRegistry {
    providers: HashMap<&'static str, Arc<dyn SandboxProvider>>,
    default: &'static str,
}

impl SandboxRegistry {
    /// Create an empty registry with the given fallback provider name.
    pub fn new(default: &'static str) -> Self {
        Self {
            providers: HashMap::new(),
            default,
        }
    }

    /// Register a provider.  Replaces any existing entry with the same name.
    pub fn register(&mut self, provider: Arc<dyn SandboxProvider>) {
        self.providers.insert(provider.name(), provider);
    }

    /// Look up a provider by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn SandboxProvider>> {
        self.providers.get(name).cloned()
    }

    /// Return the default provider, panicking if the registry is empty.
    pub fn get_default(&self) -> Arc<dyn SandboxProvider> {
        self.providers
            .get(self.default)
            .or_else(|| self.providers.values().next())
            .cloned()
            .expect("SandboxRegistry: no providers registered")
    }

    /// Select the best provider for the given policy.
    ///
    /// Tries each name in `policy.allowed_providers` in order; falls back to
    /// the default if none are registered.
    pub fn route(&self, policy: &SandboxServicePolicy) -> Arc<dyn SandboxProvider> {
        for name in &policy.allowed_providers {
            if let Some(p) = self.providers.get(name) {
                return Arc::clone(p);
            }
        }
        self.get_default()
    }
}

// ── SandboxSession ────────────────────────────────────────────────────────────

/// In-memory state for a live sandbox bound to an agent session.
#[derive(Debug)]
pub struct SandboxSession {
    /// Stable provider-assigned sandbox ID.
    pub sandbox_id: SandboxId,
    /// Arcan agent identifier.
    pub agent_id: String,
    /// Chat/task session identifier.
    pub session_id: String,
    /// Name of the provider that owns this sandbox (must match registry key).
    pub provider_name: &'static str,
    /// Monotonic timer set at session creation — used for max-lifetime checks.
    pub created_at: Instant,
    /// UTC timestamp set at session creation — used in handle reconstruction.
    pub created_at_utc: DateTime<Utc>,
    /// Monotonic timer updated on every run call — used for idle detection.
    pub last_exec_at: Instant,
    /// Current lifecycle status.
    pub status: SandboxStatus,
}

// ── SandboxService ────────────────────────────────────────────────────────────

/// Session-scoped sandbox orchestration service.
///
/// # Thread safety
///
/// `SandboxService` is `Send + Sync`.  The session map uses `DashMap` for
/// lock-free concurrent access.  Provider calls are async and never hold any
/// internal lock.
///
/// # Usage
///
/// ```rust,ignore
/// let service = Arc::new(SandboxService::new(registry, sink, policy));
/// tokio::spawn(Arc::clone(&service).run_idle_reaper());
///
/// let result = service
///     .run("agent-1", "sess-1", ExecRequest::shell("echo hi"))
///     .await?;
/// ```
pub struct SandboxService {
    registry: SandboxRegistry,
    sessions: DashMap<String, SandboxSession>,
    event_sink: Arc<dyn SandboxEventSink>,
    policy: SandboxServicePolicy,
}

impl SandboxService {
    /// Construct a new service.
    pub fn new(
        registry: SandboxRegistry,
        event_sink: Arc<dyn SandboxEventSink>,
        policy: SandboxServicePolicy,
    ) -> Self {
        Self {
            registry,
            sessions: DashMap::new(),
            event_sink,
            policy,
        }
    }

    // ── public API ────────────────────────────────────────────────────────────

    /// Return a handle to this session's sandbox, creating one if needed.
    ///
    /// - `Running` sandbox → returns immediately.
    /// - `Snapshotted` sandbox → transparently resumes before returning.
    /// - Any other status → provisions a new sandbox.
    pub async fn get_or_create(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SandboxHandle, SandboxError> {
        if let Some(snap) = self.session_snapshot(session_id) {
            match snap.status {
                SandboxStatus::Running => return Ok(self.build_handle(&snap)),
                SandboxStatus::Snapshotted => {
                    return self.resume_session(agent_id, session_id, snap).await;
                }
                _ => {
                    self.sessions.remove(session_id);
                }
            }
        }
        self.create_session(agent_id, session_id).await
    }

    /// Execute a command in this session's sandbox.
    ///
    /// Creates or resumes the sandbox transparently before execution.
    pub async fn run(
        &self,
        agent_id: &str,
        session_id: &str,
        req: ExecRequest,
    ) -> Result<ExecResult, SandboxError> {
        let handle = self.get_or_create(agent_id, session_id).await?;

        let provider = {
            let snap = self
                .session_snapshot(session_id)
                .ok_or_else(|| SandboxError::NotFound(handle.id.clone()))?;
            self.registry
                .get(snap.provider_name)
                .ok_or_else(|| SandboxError::NotFound(handle.id.clone()))?
        };

        let result = provider.run(&handle.id, req).await;

        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.last_exec_at = Instant::now();
            if let Ok(ref r) = result {
                self.event_sink.emit(SandboxEvent::now(
                    entry.sandbox_id.clone(),
                    agent_id,
                    session_id,
                    SandboxEventKind::ExecCompleted {
                        exit_code: r.exit_code,
                        duration_ms: r.duration_ms,
                    },
                    entry.provider_name,
                ));
            }
        }

        result
    }

    /// Explicitly snapshot this session's sandbox.
    ///
    /// Marks the session as `Snapshotted`.  A subsequent `get_or_create` or
    /// `run` will transparently resume it.
    pub async fn snapshot_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<(), SandboxError> {
        let snap = self
            .session_snapshot(session_id)
            .ok_or_else(|| SandboxError::NotFound(SandboxId(session_id.into())))?;

        let provider = self
            .registry
            .get(snap.provider_name)
            .ok_or_else(|| SandboxError::NotFound(snap.sandbox_id.clone()))?;

        let snapshot_id = provider.snapshot(&snap.sandbox_id).await?;

        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.status = SandboxStatus::Snapshotted;
        }

        self.event_sink.emit(SandboxEvent::now(
            snap.sandbox_id,
            agent_id,
            session_id,
            SandboxEventKind::Snapshotted {
                snapshot_id: snapshot_id.0,
            },
            snap.provider_name,
        ));

        debug!(session_id, "sandbox snapshotted");
        Ok(())
    }

    /// Destroy the sandbox for this agent session.
    ///
    /// Idempotent: returns `Ok(())` if no sandbox exists for the session.
    pub async fn destroy_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<(), SandboxError> {
        let Some((_, entry)) = self.sessions.remove(session_id) else {
            return Ok(());
        };

        let provider = self
            .registry
            .get(entry.provider_name)
            .ok_or_else(|| SandboxError::NotFound(entry.sandbox_id.clone()))?;

        provider.destroy(&entry.sandbox_id).await?;

        self.event_sink.emit(SandboxEvent::now(
            entry.sandbox_id,
            agent_id,
            session_id,
            SandboxEventKind::Destroyed,
            entry.provider_name,
        ));

        debug!(session_id, "sandbox session destroyed");
        Ok(())
    }

    /// Background idle-reaper — must be spawned as a tokio task.
    ///
    /// Runs every 10 seconds:
    /// - Snapshots sessions idle ≥ `policy.idle_timeout_secs`.
    /// - Destroys sessions older than `policy.max_lifetime_secs`.
    ///
    /// ```rust,ignore
    /// tokio::spawn(Arc::clone(&service).run_idle_reaper());
    /// ```
    pub async fn run_idle_reaper(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            self.tick_reaper().await;
        }
    }

    /// Number of active sessions tracked by this service.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    // ── internal helpers ──────────────────────────────────────────────────────

    /// Clone the minimal fields needed from a session entry without holding
    /// the DashMap reference across await points (DashMap Ref is !Send).
    fn session_snapshot(&self, session_id: &str) -> Option<SessionSnapshot> {
        self.sessions.get(session_id).map(|e| SessionSnapshot {
            sandbox_id: e.sandbox_id.clone(),
            session_id: e.session_id.clone(),
            provider_name: e.provider_name,
            created_at_utc: e.created_at_utc,
            status: e.status.clone(),
        })
    }

    async fn create_session(
        &self,
        agent_id: &str,
        session_id: &str,
    ) -> Result<SandboxHandle, SandboxError> {
        let provider = self.registry.route(&self.policy);

        let mut labels = HashMap::new();
        labels.insert("agent_id".into(), agent_id.to_owned());
        labels.insert("session_id".into(), session_id.to_owned());

        let spec = SandboxSpec {
            name: format!("arcan-{session_id}"),
            image: None,
            resources: self.policy.resources.clone(),
            env: HashMap::new(),
            persistence: self.policy.persistence.clone(),
            capabilities: SandboxCapabilitySet::FILESYSTEM_READ
                | SandboxCapabilitySet::FILESYSTEM_WRITE,
            labels,
        };

        let handle = provider.create(spec).await?;
        let now = Instant::now();

        self.sessions.insert(
            session_id.to_owned(),
            SandboxSession {
                sandbox_id: handle.id.clone(),
                agent_id: agent_id.to_owned(),
                session_id: session_id.to_owned(),
                provider_name: provider.name(),
                created_at: now,
                created_at_utc: handle.created_at,
                last_exec_at: now,
                status: handle.status.clone(),
            },
        );

        self.event_sink.emit(SandboxEvent::now(
            handle.id.clone(),
            agent_id,
            session_id,
            SandboxEventKind::Created,
            provider.name(),
        ));

        info!(
            sandbox_id = %handle.id,
            session_id,
            provider = provider.name(),
            "sandbox created"
        );

        Ok(handle)
    }

    async fn resume_session(
        &self,
        agent_id: &str,
        session_id: &str,
        snap: SessionSnapshot,
    ) -> Result<SandboxHandle, SandboxError> {
        let provider = self
            .registry
            .get(snap.provider_name)
            .ok_or_else(|| SandboxError::NotFound(snap.sandbox_id.clone()))?;

        let handle = provider.resume(&snap.sandbox_id).await?;

        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.status = handle.status.clone();
            entry.last_exec_at = Instant::now();
        }

        self.event_sink.emit(SandboxEvent::now(
            snap.sandbox_id.clone(),
            agent_id,
            session_id,
            SandboxEventKind::Resumed {
                from_snapshot: snap.sandbox_id.0.clone(),
            },
            snap.provider_name,
        ));

        debug!(sandbox_id = %snap.sandbox_id, session_id, "sandbox resumed");
        Ok(handle)
    }

    async fn tick_reaper(&self) {
        let now = Instant::now();

        let mut to_snapshot: Vec<(String, String)> = Vec::new();
        let mut to_destroy: Vec<(String, String)> = Vec::new();

        for entry in self.sessions.iter() {
            let idle_secs = now.duration_since(entry.last_exec_at).as_secs();
            let lifetime_secs = now.duration_since(entry.created_at).as_secs();

            if lifetime_secs >= self.policy.max_lifetime_secs {
                to_destroy.push((entry.agent_id.clone(), entry.session_id.clone()));
            } else if idle_secs >= self.policy.idle_timeout_secs
                && entry.status == SandboxStatus::Running
            {
                to_snapshot.push((entry.agent_id.clone(), entry.session_id.clone()));
            }
        }

        for (agent_id, session_id) in to_snapshot {
            if let Err(e) = self.snapshot_session(&agent_id, &session_id).await {
                warn!(session_id, error = %e, "idle reaper: snapshot failed (non-fatal)");
            }
        }

        for (agent_id, session_id) in to_destroy {
            if let Err(e) = self.destroy_session(&agent_id, &session_id).await {
                warn!(session_id, error = %e, "idle reaper: destroy failed (non-fatal)");
            }
        }
    }

    fn build_handle(&self, snap: &SessionSnapshot) -> SandboxHandle {
        SandboxHandle {
            id: snap.sandbox_id.clone(),
            name: format!("arcan-{}", snap.session_id),
            status: snap.status.clone(),
            created_at: snap.created_at_utc,
            provider: snap.provider_name.to_owned(),
            metadata: serde_json::Value::Null,
        }
    }
}

// ── SessionSnapshot ───────────────────────────────────────────────────────────

/// Value-copy of the session fields needed across await points.
///
/// Excludes `agent_id` (always passed as a parameter) and `created_at`
/// (monotonic, only needed on the live entry for idle calculations).
#[derive(Debug, Clone)]
struct SessionSnapshot {
    sandbox_id: SandboxId,
    session_id: String,
    provider_name: &'static str,
    created_at_utc: DateTime<Utc>,
    status: SandboxStatus,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::NoopSink;
    use crate::types::{SandboxInfo, SnapshotId};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    // ── Stub provider ─────────────────────────────────────────────────────────

    struct StubProvider {
        pname: &'static str,
        create_count: AtomicUsize,
        run_count: AtomicUsize,
        snapshot_count: AtomicUsize,
        destroy_count: AtomicUsize,
        resume_count: AtomicUsize,
        run_error: Mutex<Option<String>>,
    }

    impl StubProvider {
        fn new(pname: &'static str) -> Arc<Self> {
            Arc::new(Self {
                pname,
                create_count: AtomicUsize::new(0),
                run_count: AtomicUsize::new(0),
                snapshot_count: AtomicUsize::new(0),
                destroy_count: AtomicUsize::new(0),
                resume_count: AtomicUsize::new(0),
                run_error: Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl SandboxProvider for StubProvider {
        fn name(&self) -> &'static str {
            self.pname
        }

        fn capabilities(&self) -> SandboxCapabilitySet {
            SandboxCapabilitySet::all()
        }

        async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
            self.create_count.fetch_add(1, Ordering::SeqCst);
            Ok(SandboxHandle {
                id: SandboxId(format!("{}-box", spec.name)),
                name: spec.name,
                status: SandboxStatus::Running,
                created_at: Utc::now(),
                provider: self.pname.to_owned(),
                metadata: serde_json::Value::Null,
            })
        }

        async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
            self.resume_count.fetch_add(1, Ordering::SeqCst);
            Ok(SandboxHandle {
                id: id.clone(),
                name: id.0.clone(),
                status: SandboxStatus::Running,
                created_at: Utc::now(),
                provider: self.pname.to_owned(),
                metadata: serde_json::Value::Null,
            })
        }

        async fn run(
            &self,
            _id: &SandboxId,
            req: ExecRequest,
        ) -> Result<ExecResult, SandboxError> {
            self.run_count.fetch_add(1, Ordering::SeqCst);
            if let Some(msg) = self.run_error.lock().unwrap().as_deref() {
                return Err(SandboxError::ProviderError {
                    provider: self.pname,
                    message: msg.to_owned(),
                });
            }
            Ok(ExecResult {
                stdout: req.command.join(" ").into_bytes(),
                stderr: vec![],
                exit_code: 0,
                duration_ms: 1,
            })
        }

        async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
            self.snapshot_count.fetch_add(1, Ordering::SeqCst);
            Ok(SnapshotId(format!("{}-snap", id.0)))
        }

        async fn destroy(&self, _id: &SandboxId) -> Result<(), SandboxError> {
            self.destroy_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
            Ok(vec![])
        }
    }

    fn make_service(provider: Arc<StubProvider>) -> Arc<SandboxService> {
        let mut registry = SandboxRegistry::new("stub");
        registry.register(provider as Arc<dyn SandboxProvider>);
        Arc::new(SandboxService::new(
            registry,
            Arc::new(NoopSink),
            SandboxServicePolicy::free(),
        ))
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_or_create_provisions_new_sandbox() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        let handle = svc.get_or_create("agent-1", "sess-1").await.unwrap();
        assert_eq!(handle.provider, "stub");
        assert_eq!(p.create_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn get_or_create_returns_existing_running_sandbox() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.get_or_create("a", "s").await.unwrap();
        svc.get_or_create("a", "s").await.unwrap();

        assert_eq!(p.create_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_drives_create_then_provider_run() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        let result = svc
            .run("a", "s", ExecRequest::shell("echo hi"))
            .await
            .unwrap();

        assert_eq!(p.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(p.run_count.load(Ordering::SeqCst), 1);
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn run_reuses_existing_session() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.run("a", "s", ExecRequest::shell("echo 1")).await.unwrap();
        svc.run("a", "s", ExecRequest::shell("echo 2")).await.unwrap();

        assert_eq!(p.create_count.load(Ordering::SeqCst), 1);
        assert_eq!(p.run_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn snapshot_then_resume_on_get_or_create() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.get_or_create("a", "s").await.unwrap();
        svc.snapshot_session("a", "s").await.unwrap();

        assert_eq!(
            svc.sessions.get("s").unwrap().status,
            SandboxStatus::Snapshotted
        );

        svc.get_or_create("a", "s").await.unwrap();
        assert_eq!(p.resume_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            svc.sessions.get("s").unwrap().status,
            SandboxStatus::Running
        );
    }

    #[tokio::test]
    async fn destroy_session_removes_entry_and_calls_provider() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.get_or_create("a", "s").await.unwrap();
        assert_eq!(svc.session_count(), 1);

        svc.destroy_session("a", "s").await.unwrap();
        assert_eq!(svc.session_count(), 0);
        assert_eq!(p.destroy_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn destroy_nonexistent_session_is_idempotent() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.destroy_session("a", "ghost").await.unwrap();
        assert_eq!(p.destroy_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn idle_reaper_snapshots_idle_session() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.get_or_create("a", "s").await.unwrap();

        {
            let mut entry = svc.sessions.get_mut("s").unwrap();
            entry.last_exec_at =
                Instant::now() - Duration::from_secs(svc.policy.idle_timeout_secs + 1);
        }

        svc.tick_reaper().await;

        assert_eq!(p.snapshot_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            svc.sessions.get("s").unwrap().status,
            SandboxStatus::Snapshotted
        );
    }

    #[tokio::test]
    async fn idle_reaper_destroys_expired_session() {
        let p = StubProvider::new("stub");
        let svc = make_service(Arc::clone(&p));

        svc.get_or_create("a", "s").await.unwrap();

        {
            let mut entry = svc.sessions.get_mut("s").unwrap();
            entry.created_at =
                Instant::now() - Duration::from_secs(svc.policy.max_lifetime_secs + 1);
        }

        svc.tick_reaper().await;

        assert_eq!(p.destroy_count.load(Ordering::SeqCst), 1);
        assert_eq!(svc.session_count(), 0);
    }

    #[tokio::test]
    async fn registry_routes_by_policy_order() {
        let p_local = StubProvider::new("local");
        let p_vercel = StubProvider::new("vercel");

        let mut registry = SandboxRegistry::new("local");
        registry.register(Arc::clone(&p_local) as Arc<dyn SandboxProvider>);
        registry.register(Arc::clone(&p_vercel) as Arc<dyn SandboxProvider>);

        let pro = SandboxServicePolicy::pro();
        assert_eq!(registry.route(&pro).name(), "vercel");

        let free = SandboxServicePolicy::free();
        assert_eq!(registry.route(&free).name(), "local");
    }

    #[tokio::test]
    async fn policy_tier_defaults() {
        let free = SandboxServicePolicy::free();
        assert_eq!(free.max_concurrent, 1);
        assert_eq!(free.idle_timeout_secs, 30);
        assert!(matches!(free.persistence, PersistencePolicy::Ephemeral));

        let pro = SandboxServicePolicy::pro();
        assert_eq!(pro.max_concurrent, 3);
        assert!(matches!(
            pro.persistence,
            PersistencePolicy::Persistent { .. }
        ));

        let ent = SandboxServicePolicy::enterprise();
        assert_eq!(ent.max_concurrent, 10);
        assert_eq!(ent.idle_timeout_secs, 3_600);
    }
}
