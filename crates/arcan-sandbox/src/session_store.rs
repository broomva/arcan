//! Session store abstractions for Arcan sandbox lifecycle management.
//!
//! This module defines the [`SandboxSessionStore`] trait and its in-process
//! implementation [`InMemorySessionStore`]. A production Redis-backed
//! implementation is planned in BRO-244 as a separate crate
//! (`arcan-session-store-upstash`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use aios_protocol::policy::SubscriptionTier;

use crate::types::{SandboxHandle, SandboxId};

// ── TTL policy ───────────────────────────────────────────────────────────────

/// Returns the session TTL for a given [`SubscriptionTier`].
///
/// | Tier | TTL |
/// |------|-----|
/// | `Anonymous` | `Some(Duration::ZERO)` — cleared at session end via `expire()` / `remove()` |
/// | `Free` | `Some(7 days)` |
/// | `Pro` | `Some(90 days)` |
/// | `Enterprise` | `None` — no expiry |
pub fn tier_ttl(tier: SubscriptionTier) -> Option<Duration> {
    match tier {
        SubscriptionTier::Anonymous => Some(Duration::from_secs(0)),
        SubscriptionTier::Free => Some(Duration::from_secs(7 * 24 * 3600)),
        SubscriptionTier::Pro => Some(Duration::from_secs(90 * 24 * 3600)),
        SubscriptionTier::Enterprise => None,
    }
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Maps Arcan session IDs to provider-specific backend sandbox IDs,
/// with tier-aware TTL-based expiry.
///
/// Implementations:
/// - [`InMemorySessionStore`] — in-process hash map, for development and tests.
/// - `UpstashSessionStore` — Redis-backed, planned in BRO-244 (`arcan-session-store-upstash`).
pub trait SandboxSessionStore: Send + Sync + 'static {
    /// Register a mapping from `session_id` → `sandbox_id`.
    ///
    /// Overwrites any existing mapping for the same `session_id`.
    /// The entry's TTL is computed from `tier` via [`tier_ttl`].
    fn register(&self, session_id: &str, sandbox_id: SandboxId, tier: SubscriptionTier);

    /// Look up the sandbox ID for `session_id`.
    ///
    /// Returns `None` if the entry is missing or its TTL has elapsed.
    fn lookup(&self, session_id: &str) -> Option<SandboxId>;

    /// Explicitly remove the mapping for `session_id`.
    fn remove(&self, session_id: &str);

    /// Remove all entries whose TTL has expired.
    ///
    /// This is intended to be called periodically by the runtime or a
    /// background reaper task.
    fn expire(&self);
}

// ── Convenience extension ────────────────────────────────────────────────────

/// Extension methods for [`SandboxSessionStore`].
pub trait SandboxSessionStoreExt: SandboxSessionStore {
    /// Register a new sandbox handle, extracting the [`SandboxId`] automatically.
    fn register_handle(&self, session_id: &str, handle: &SandboxHandle, tier: SubscriptionTier) {
        self.register(session_id, handle.id.clone(), tier);
    }
}

impl<T: SandboxSessionStore> SandboxSessionStoreExt for T {}

// ── InMemorySessionStore ─────────────────────────────────────────────────────

struct SessionEntry {
    sandbox_id: SandboxId,
    /// Absolute deadline after which the entry is considered expired.
    /// `None` means the entry never expires (Enterprise tier).
    expires_at: Option<Instant>,
}

/// In-process session store backed by a `Mutex<HashMap>`.
///
/// Suitable for development, tests, and single-process deployments.
/// For multi-instance or persistent deployments use the planned
/// `arcan-session-store-upstash` crate (BRO-244).
pub struct InMemorySessionStore {
    entries: Mutex<HashMap<String, SessionEntry>>,
}

impl InMemorySessionStore {
    /// Create a new, empty session store.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxSessionStore for InMemorySessionStore {
    fn register(&self, session_id: &str, sandbox_id: SandboxId, tier: SubscriptionTier) {
        let expires_at = tier_ttl(tier).map(|ttl| Instant::now() + ttl);
        let entry = SessionEntry {
            sandbox_id,
            expires_at,
        };
        // Intentional: if the lock is poisoned the process is in an
        // unrecoverable state; propagating the panic is correct here.
        let mut map = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.insert(session_id.to_owned(), entry);
    }

    fn lookup(&self, session_id: &str) -> Option<SandboxId> {
        let map = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(session_id).and_then(|entry| {
            let expired = entry
                .expires_at
                .map(|deadline| Instant::now() > deadline)
                .unwrap_or(false);
            if expired {
                None
            } else {
                Some(entry.sandbox_id.clone())
            }
        })
    }

    fn remove(&self, session_id: &str) {
        let mut map = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.remove(session_id);
    }

    fn expire(&self) {
        let now = Instant::now();
        let mut map = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_key, entry| {
            entry
                .expires_at
                .map(|deadline| now <= deadline)
                .unwrap_or(true)
        });
    }
}

// ── UpstashSessionStore stub ─────────────────────────────────────────────────

/// Production session store backed by Upstash Redis.
///
/// Planned in BRO-244 (separate crate: `arcan-session-store-upstash`).
/// Uses key prefix `arcan:session:` with `EXPIRE` for tier TTLs.
pub struct UpstashSessionStore; // placeholder — not yet implemented

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SandboxHandle, SandboxStatus};
    use chrono::Utc;

    fn make_store() -> InMemorySessionStore {
        InMemorySessionStore::new()
    }

    fn sandbox_id(s: &str) -> SandboxId {
        SandboxId(s.to_owned())
    }

    fn make_handle(id: &str) -> SandboxHandle {
        SandboxHandle {
            id: sandbox_id(id),
            name: "test".to_owned(),
            status: SandboxStatus::Running,
            created_at: Utc::now(),
            provider: "local".to_owned(),
            metadata: serde_json::Value::Null,
        }
    }

    // 1. Basic round-trip
    #[test]
    fn register_and_lookup_returns_sandbox_id() {
        let store = make_store();
        store.register("session-1", sandbox_id("box-1"), SubscriptionTier::Free);
        assert_eq!(store.lookup("session-1"), Some(sandbox_id("box-1")));
    }

    // 2. Missing entry
    #[test]
    fn lookup_missing_returns_none() {
        let store = make_store();
        assert_eq!(store.lookup("does-not-exist"), None);
    }

    // 3. Explicit remove
    #[test]
    fn remove_clears_entry() {
        let store = make_store();
        store.register("session-2", sandbox_id("box-2"), SubscriptionTier::Pro);
        store.remove("session-2");
        assert_eq!(store.lookup("session-2"), None);
    }

    // 4. Anonymous TTL is zero-duration
    #[test]
    fn anonymous_tier_ttl_is_zero() {
        assert_eq!(
            tier_ttl(SubscriptionTier::Anonymous),
            Some(Duration::from_secs(0))
        );
    }

    // 5. Free TTL is 7 days
    #[test]
    fn free_tier_ttl_is_7_days() {
        assert_eq!(
            tier_ttl(SubscriptionTier::Free),
            Some(Duration::from_secs(7 * 24 * 3600))
        );
    }

    // 6. Pro TTL is 90 days
    #[test]
    fn pro_tier_ttl_is_90_days() {
        assert_eq!(
            tier_ttl(SubscriptionTier::Pro),
            Some(Duration::from_secs(90 * 24 * 3600))
        );
    }

    // 7. Enterprise TTL is None
    #[test]
    fn enterprise_tier_ttl_is_none() {
        assert_eq!(tier_ttl(SubscriptionTier::Enterprise), None);
    }

    // 8. expire() removes zero-TTL (anonymous) entries
    #[test]
    fn expire_removes_zero_ttl_entries() {
        let store = make_store();
        // Anonymous TTL = Duration::ZERO → expires_at = Instant::now() + 0 = now.
        // By the time expire() runs, Instant::now() > that deadline (or == it).
        // We give it a tiny sleep to be deterministic across fast machines.
        store.register(
            "anon-session",
            sandbox_id("anon-box"),
            SubscriptionTier::Anonymous,
        );

        // Sleep 1 ms to ensure the anonymous deadline has passed.
        std::thread::sleep(Duration::from_millis(1));

        store.expire();
        assert_eq!(store.lookup("anon-session"), None);
    }

    // 9. expire() keeps non-expired (Free) entries alive
    #[test]
    fn expire_keeps_non_expired_entries() {
        let store = make_store();
        store.register(
            "free-session",
            sandbox_id("free-box"),
            SubscriptionTier::Free,
        );
        store.expire(); // 7-day TTL — should not be removed immediately
        assert_eq!(store.lookup("free-session"), Some(sandbox_id("free-box")));
    }

    // 10. register_handle_ext uses the SandboxHandle's id
    #[test]
    fn register_handle_ext() {
        let store = make_store();
        let handle = make_handle("handle-box");
        store.register_handle("session-handle", &handle, SubscriptionTier::Pro);
        assert_eq!(
            store.lookup("session-handle"),
            Some(sandbox_id("handle-box"))
        );
    }
}
