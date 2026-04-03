//! Lifecycle hooks for plugin extensibility in the Arcan agent runtime.
//!
//! External crates (Autonomic, Haima, Vigil, etc.) can implement
//! [`LifecycleHook`] to subscribe to agent lifecycle events without hardwiring
//! into the runtime. All methods have default no-op implementations so plugins
//! only override what they care about.
//!
//! Unlike [`Middleware`](crate::runtime::Middleware) and
//! [`TurnMiddleware`](crate::runtime::TurnMiddleware), lifecycle hooks are
//! **observational and fire-and-forget**: they cannot block, transform, or veto
//! operations. This makes them safe for telemetry, logging, billing, and
//! notification use-cases where the hook should never interrupt the agent loop.

use crate::runtime::{ProviderRequest, RunOutput};

/// Lifecycle hooks for the agent loop.
///
/// All methods have default no-op implementations so implementors only need
/// to override the events they care about.
///
/// # Difference from `Middleware` / `TurnMiddleware`
///
/// | Aspect | Middleware / TurnMiddleware | LifecycleHook |
/// |--------|----------------------------|---------------|
/// | Can block? | Yes (`Result<(), CoreError>`) | No (infallible) |
/// | Can mutate? | TurnMiddleware: yes | No (shared refs) |
/// | Session events? | No | Yes (`on_session_start`, `on_session_end`) |
/// | Use-case | Policy gates, transforms | Telemetry, billing, notifications |
///
/// # Example
///
/// ```rust
/// use arcan_core::lifecycle::LifecycleHook;
/// use arcan_core::runtime::{ProviderRequest, RunOutput};
///
/// struct TelemetryHook;
///
/// impl LifecycleHook for TelemetryHook {
///     fn on_session_start(&self, session_id: &str) {
///         println!("session started: {session_id}");
///     }
///
///     fn on_session_end(&self, session_id: &str, _output: &RunOutput) {
///         println!("session ended: {session_id}");
///     }
/// }
/// ```
pub trait LifecycleHook: Send + Sync {
    /// Called before a tool is executed.
    fn pre_tool_call(&self, _tool_name: &str, _input: &serde_json::Value) {}

    /// Called after a tool completes successfully.
    fn post_tool_call(&self, _tool_name: &str, _result: &str) {}

    /// Called before each LLM provider call.
    fn pre_llm_call(&self, _request: &ProviderRequest) {}

    /// Called after each LLM provider call.
    fn post_llm_call(&self, _request: &ProviderRequest) {}

    /// Called when a session starts (before the first iteration).
    fn on_session_start(&self, _session_id: &str) {}

    /// Called when a session ends (after the final output is assembled).
    fn on_session_end(&self, _session_id: &str, _output: &RunOutput) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A test hook that counts invocations.
    struct CountingHook {
        pre_tool: AtomicU32,
        post_tool: AtomicU32,
        pre_llm: AtomicU32,
        post_llm: AtomicU32,
        session_start: AtomicU32,
        session_end: AtomicU32,
    }

    impl CountingHook {
        fn new() -> Self {
            Self {
                pre_tool: AtomicU32::new(0),
                post_tool: AtomicU32::new(0),
                pre_llm: AtomicU32::new(0),
                post_llm: AtomicU32::new(0),
                session_start: AtomicU32::new(0),
                session_end: AtomicU32::new(0),
            }
        }
    }

    impl LifecycleHook for CountingHook {
        fn pre_tool_call(&self, _tool_name: &str, _input: &serde_json::Value) {
            self.pre_tool.fetch_add(1, Ordering::Relaxed);
        }

        fn post_tool_call(&self, _tool_name: &str, _result: &str) {
            self.post_tool.fetch_add(1, Ordering::Relaxed);
        }

        fn pre_llm_call(&self, _request: &ProviderRequest) {
            self.pre_llm.fetch_add(1, Ordering::Relaxed);
        }

        fn post_llm_call(&self, _request: &ProviderRequest) {
            self.post_llm.fetch_add(1, Ordering::Relaxed);
        }

        fn on_session_start(&self, _session_id: &str) {
            self.session_start.fetch_add(1, Ordering::Relaxed);
        }

        fn on_session_end(&self, _session_id: &str, _output: &RunOutput) {
            self.session_end.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn default_impls_are_noop() {
        // A struct with no overrides should compile and do nothing.
        struct NoopHook;
        impl LifecycleHook for NoopHook {}

        let hook = NoopHook;
        hook.pre_tool_call("test", &serde_json::json!({}));
        hook.post_tool_call("test", "ok");
        // Just verifying it compiles and runs without panic.
    }

    #[test]
    fn counting_hook_tracks_invocations() {
        let hook = Arc::new(CountingHook::new());

        hook.pre_tool_call("bash", &serde_json::json!({"command": "ls"}));
        hook.pre_tool_call("read_file", &serde_json::json!({"path": "/tmp"}));
        hook.post_tool_call("bash", "file1.txt");

        assert_eq!(hook.pre_tool.load(Ordering::Relaxed), 2);
        assert_eq!(hook.post_tool.load(Ordering::Relaxed), 1);
        assert_eq!(hook.pre_llm.load(Ordering::Relaxed), 0);
        assert_eq!(hook.post_llm.load(Ordering::Relaxed), 0);
        assert_eq!(hook.session_start.load(Ordering::Relaxed), 0);
        assert_eq!(hook.session_end.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn hook_is_send_sync() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn LifecycleHook>();
    }

    #[test]
    fn session_lifecycle_events_fire() {
        let hook = Arc::new(CountingHook::new());

        hook.on_session_start("session-1");
        hook.on_session_start("session-1");
        assert_eq!(hook.session_start.load(Ordering::Relaxed), 2);

        // We can't easily construct a full RunOutput here, but we can verify
        // the session_end counter doesn't fire until called.
        assert_eq!(hook.session_end.load(Ordering::Relaxed), 0);
    }
}
