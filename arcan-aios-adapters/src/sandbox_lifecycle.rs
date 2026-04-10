//! [`SandboxLifecycleObserver`] — tier-aware sandbox cleanup on run end.
//!
//! Implements [`ToolHarnessObserver::on_run_finished`] to clean up sandboxes
//! when agent runs complete, with tier-specific retention semantics:
//!
//! | Tier | Action on run end |
//! |------|-------------------|
//! | `Anonymous` | Destroy sandbox immediately + remove from store |
//! | `Free` / `Pro` | `snapshot()` → stop current session (v2: auto-snapshot for persistent sandboxes) |
//! | `Enterprise` | No-op — managed externally |
//!
//! ## Vercel v2 auto-persistence
//!
//! When using [`arcan_provider_vercel::VercelSandboxProvider`] with the v2 API,
//! `snapshot()` calls `POST /v2/sandboxes/sessions/{id}/stop` which automatically
//! saves the filesystem state for persistent sandboxes.  No separate snapshot
//! management is required — the sandbox name (stored in the session store as
//! [`arcan_sandbox::SandboxId`]) is the stable resume handle.

use std::sync::Arc;

use aios_protocol::SubscriptionTier;
use arcan_sandbox::{InMemorySessionStore, SandboxProvider, SandboxSessionStore};

use crate::tools::{RunCompletionContext, ToolHarnessObserver};
use async_trait::async_trait;

/// Tier-aware sandbox lifecycle observer.
///
/// Register this as a [`ToolHarnessObserver`] to automatically clean up (or
/// snapshot) sandboxes when agent runs finish.
///
/// # Tier semantics
///
/// - **Anonymous**: sandbox is destroyed immediately and removed from the
///   session store. No state is retained.
/// - **Free / Pro**: sandbox is snapshotted (state preserved for next session).
///   If snapshotting fails, the sandbox is destroyed to avoid resource leaks.
/// - **Enterprise**: no action — enterprise sandbox lifecycle is managed by
///   an external orchestrator.
pub struct SandboxLifecycleObserver {
    provider: Arc<dyn SandboxProvider>,
    store: Arc<InMemorySessionStore>,
    tier: SubscriptionTier,
}

impl SandboxLifecycleObserver {
    /// Construct a new observer.
    ///
    /// `tier` determines the cleanup policy applied when `on_run_finished` is
    /// called. Use `SubscriptionTier::Anonymous` as a conservative default
    /// when the tier is not yet known at observer construction time.
    pub fn new(
        provider: Arc<dyn SandboxProvider>,
        store: Arc<InMemorySessionStore>,
        tier: SubscriptionTier,
    ) -> Self {
        Self {
            provider,
            store,
            tier,
        }
    }
}

#[async_trait]
impl ToolHarnessObserver for SandboxLifecycleObserver {
    async fn post_execute(&self, _session_id: String, _tool_name: String, _is_error: bool) {
        // No-op: lifecycle is managed at run granularity, not tool granularity.
    }

    async fn on_run_finished(&self, session_id: String, _context: RunCompletionContext) {
        let Some(sandbox_id) = self.store.lookup(&session_id) else {
            return; // no sandbox registered for this session
        };

        match self.tier {
            SubscriptionTier::Anonymous => {
                // Destroy immediately — ephemeral, no retention.
                if let Err(e) = self.provider.destroy(&sandbox_id).await {
                    tracing::warn!(
                        session = %session_id,
                        error = %e,
                        "failed to destroy anonymous sandbox"
                    );
                }
                self.store.remove(&session_id);
                tracing::info!(session = %session_id, "anonymous sandbox destroyed");
            }

            SubscriptionTier::Free | SubscriptionTier::Pro => {
                // Snapshot for persistence — sandbox resumes on next session.
                match self.provider.snapshot(&sandbox_id).await {
                    Ok(snapshot_id) => {
                        tracing::info!(
                            session = %session_id,
                            snapshot = %snapshot_id,
                            "sandbox snapshotted for persistence"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            session = %session_id,
                            error = %e,
                            "failed to snapshot sandbox — destroying to avoid resource leak"
                        );
                        if let Err(destroy_err) = self.provider.destroy(&sandbox_id).await {
                            tracing::warn!(
                                session = %session_id,
                                error = %destroy_err,
                                "failed to destroy sandbox after snapshot failure"
                            );
                        }
                        self.store.remove(&session_id);
                    }
                }
            }

            SubscriptionTier::Enterprise => {
                // Enterprise sandboxes are managed externally — do nothing.
                tracing::debug!(
                    session = %session_id,
                    "enterprise sandbox lifecycle deferred to external manager"
                );
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use aios_protocol::SubscriptionTier;
    use arcan_sandbox::{
        InMemorySessionStore, SandboxCapabilitySet, SandboxError, SandboxId, SandboxInfo,
        SandboxProvider, SandboxSessionStore, SandboxSpec,
        types::{ExecRequest, ExecResult, SandboxHandle, SandboxStatus, SnapshotId},
    };
    use async_trait::async_trait;
    use chrono::Utc;

    use super::SandboxLifecycleObserver;
    use crate::tools::{RunCompletionContext, ToolHarnessObserver};

    // ── MockProvider ─────────────────────────────────────────────────────────

    /// Records which operations were called, for assertion in tests.
    #[derive(Default)]
    struct CallLog {
        destroys: Vec<String>,
        snapshots: Vec<String>,
    }

    struct MockProvider {
        log: Arc<Mutex<CallLog>>,
        /// When `true`, `snapshot()` returns an error.
        snapshot_fails: bool,
    }

    impl MockProvider {
        fn new() -> (Self, Arc<Mutex<CallLog>>) {
            let log = Arc::new(Mutex::new(CallLog::default()));
            (
                Self {
                    log: Arc::clone(&log),
                    snapshot_fails: false,
                },
                log,
            )
        }

        fn with_failing_snapshot() -> (Self, Arc<Mutex<CallLog>>) {
            let log = Arc::new(Mutex::new(CallLog::default()));
            (
                Self {
                    log: Arc::clone(&log),
                    snapshot_fails: true,
                },
                log,
            )
        }
    }

    fn dummy_handle(id: &str) -> SandboxHandle {
        SandboxHandle {
            id: SandboxId(id.to_owned()),
            name: "test".to_owned(),
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: "mock".to_owned(),
            metadata: serde_json::Value::Null,
        }
    }

    #[async_trait]
    impl SandboxProvider for MockProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn capabilities(&self) -> SandboxCapabilitySet {
            SandboxCapabilitySet::FILESYSTEM_READ
        }

        async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, SandboxError> {
            Ok(dummy_handle(&spec.name))
        }

        async fn resume(&self, id: &SandboxId) -> Result<SandboxHandle, SandboxError> {
            Ok(dummy_handle(&id.0))
        }

        async fn run(
            &self,
            _id: &SandboxId,
            _req: ExecRequest,
        ) -> Result<ExecResult, SandboxError> {
            Ok(ExecResult {
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                duration_ms: 0,
            })
        }

        async fn snapshot(&self, id: &SandboxId) -> Result<SnapshotId, SandboxError> {
            if self.snapshot_fails {
                return Err(SandboxError::ProviderError {
                    provider: "mock",
                    message: "snapshot failure injected by test".into(),
                });
            }
            self.log.lock().unwrap().snapshots.push(id.0.clone());
            Ok(SnapshotId(id.0.clone()))
        }

        async fn destroy(&self, id: &SandboxId) -> Result<(), SandboxError> {
            self.log.lock().unwrap().destroys.push(id.0.clone());
            Ok(())
        }

        async fn list(&self) -> Result<Vec<SandboxInfo>, SandboxError> {
            Ok(vec![])
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /// Build an observer + store with a sandbox pre-registered for `session_id`.
    ///
    /// The sandbox is always registered with `Free` tier so the store entry
    /// does not expire during the test (Anonymous TTL = 0 causes immediate
    /// expiry). The `tier` parameter is passed only to the observer to
    /// control which lifecycle action is taken.
    fn setup(
        provider: Arc<dyn SandboxProvider>,
        tier: SubscriptionTier,
        session_id: &str,
        sandbox_id: &str,
    ) -> (SandboxLifecycleObserver, Arc<InMemorySessionStore>) {
        let store = Arc::new(InMemorySessionStore::new());
        // Always register with Free tier to avoid zero-TTL expiry in tests.
        // The observer tier controls cleanup behaviour, not the store TTL.
        store.register(
            session_id,
            SandboxId(sandbox_id.to_owned()),
            SubscriptionTier::Free,
        );
        let obs = SandboxLifecycleObserver::new(provider, Arc::clone(&store), tier);
        (obs, store)
    }

    // ── Test 1: anonymous tier destroys sandbox ───────────────────────────────

    #[tokio::test]
    async fn observer_destroys_anon_sandbox_on_run_finished() {
        let (mock, log) = MockProvider::new();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);

        let (obs, _store) = setup(
            Arc::clone(&provider),
            SubscriptionTier::Anonymous,
            "sess-anon",
            "sbx-1",
        );

        obs.on_run_finished("sess-anon".into(), RunCompletionContext::default())
            .await;

        let log = log.lock().unwrap();
        assert!(
            log.destroys.contains(&"sbx-1".to_owned()),
            "destroy should have been called for sbx-1"
        );
        assert!(
            log.snapshots.is_empty(),
            "snapshot should NOT be called for anonymous tier"
        );
    }

    // ── Test 2: session removed from store after destroy ─────────────────────

    #[tokio::test]
    async fn observer_removes_session_from_store_after_destroy() {
        let (mock, _log) = MockProvider::new();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);

        let (obs, store) = setup(
            Arc::clone(&provider),
            SubscriptionTier::Anonymous,
            "sess-remove",
            "sbx-2",
        );

        assert!(
            store.lookup("sess-remove").is_some(),
            "entry should exist before run end"
        );
        obs.on_run_finished("sess-remove".into(), RunCompletionContext::default())
            .await;
        assert!(
            store.lookup("sess-remove").is_none(),
            "entry should be removed after anon cleanup"
        );
    }

    // ── Test 3: no sandbox in store — no-op ──────────────────────────────────

    #[tokio::test]
    async fn observer_skips_when_no_sandbox_in_store() {
        let (mock, log) = MockProvider::new();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);
        let store = Arc::new(InMemorySessionStore::new());
        // Note: no sandbox registered in store.
        let obs = SandboxLifecycleObserver::new(provider, store, SubscriptionTier::Anonymous);

        obs.on_run_finished("sess-missing".into(), RunCompletionContext::default())
            .await;

        let log = log.lock().unwrap();
        assert!(
            log.destroys.is_empty(),
            "no destroy should be called if no sandbox in store"
        );
        assert!(log.snapshots.is_empty());
    }

    // ── Test 4: free tier snapshots ──────────────────────────────────────────

    #[tokio::test]
    async fn observer_snapshots_free_tier_sandbox() {
        let (mock, log) = MockProvider::new();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);

        let (obs, store) = setup(
            Arc::clone(&provider),
            SubscriptionTier::Free,
            "sess-free",
            "sbx-3",
        );

        obs.on_run_finished("sess-free".into(), RunCompletionContext::default())
            .await;

        let log = log.lock().unwrap();
        assert!(
            log.snapshots.contains(&"sbx-3".to_owned()),
            "snapshot should have been called for free tier"
        );
        assert!(
            log.destroys.is_empty(),
            "destroy should NOT be called on successful snapshot"
        );
        // Entry should still be in store (sandbox persisted).
        drop(log);
        assert!(
            store.lookup("sess-free").is_some(),
            "free-tier session should remain in store after snapshot"
        );
    }

    // ── Test 5: snapshot failure falls back to destroy ────────────────────────

    #[tokio::test]
    async fn snapshot_failure_destroys_sandbox_as_fallback() {
        let (mock, log) = MockProvider::with_failing_snapshot();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);

        let (obs, store) = setup(
            Arc::clone(&provider),
            SubscriptionTier::Pro,
            "sess-pro-fail",
            "sbx-4",
        );

        obs.on_run_finished("sess-pro-fail".into(), RunCompletionContext::default())
            .await;

        let log = log.lock().unwrap();
        assert!(
            log.destroys.contains(&"sbx-4".to_owned()),
            "destroy should be called as fallback when snapshot fails"
        );
        drop(log);
        assert!(
            store.lookup("sess-pro-fail").is_none(),
            "entry should be removed after fallback destroy"
        );
    }

    // ── Test 6: enterprise tier does nothing ──────────────────────────────────

    #[tokio::test]
    async fn enterprise_tier_is_noop() {
        let (mock, log) = MockProvider::new();
        let provider: Arc<dyn SandboxProvider> = Arc::new(mock);

        let (obs, store) = setup(
            Arc::clone(&provider),
            SubscriptionTier::Enterprise,
            "sess-enterprise",
            "sbx-5",
        );

        obs.on_run_finished("sess-enterprise".into(), RunCompletionContext::default())
            .await;

        let log = log.lock().unwrap();
        assert!(log.destroys.is_empty(), "enterprise tier must not destroy");
        assert!(
            log.snapshots.is_empty(),
            "enterprise tier must not snapshot"
        );
        drop(log);
        assert!(
            store.lookup("sess-enterprise").is_some(),
            "enterprise session must remain in store"
        );
    }
}
