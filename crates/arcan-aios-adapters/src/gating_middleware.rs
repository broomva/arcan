//! Autonomic gating middleware for the Arcan [`Orchestrator`] agent loop.
//!
//! [`AutonomicGatingMiddleware`] implements the [`Middleware`] trait from
//! `arcan-core` and enforces dynamic, per-tick limits derived from the
//! Autonomic gating profile:
//!
//! - **`max_tool_calls_per_tick`**: Blocks tool calls once the per-tick cap is reached.
//! - **`max_file_mutations_per_tick`**: Blocks file-writing tools once the mutation cap is reached.
//! - **`allow_network`**: Rejects network-facing tools when disabled.
//! - **`allow_shell`**: Rejects shell execution tools when disabled.
//! - **`allow_side_effects`**: Rejects all write/mutating tools when disabled.
//! - **OperatingMode transitions**: Logs mode changes with rationale.
//!
//! The middleware is advisory: if the gating profile is unavailable (no events
//! have been folded yet, or the HTTP call failed), all checks are skipped and
//! the inner orchestrator proceeds normally.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use aios_protocol::mode::OperatingMode;
use arcan_core::error::CoreError;
use arcan_core::protocol::ToolCall;
use arcan_core::runtime::{Middleware, ProviderRequest, ToolContext};
use tokio::sync::RwLock;

use crate::autonomic::LocalGatingProfile;

// ---------------------------------------------------------------------------
// Shared gating state (per-tick counters + cached profile)
// ---------------------------------------------------------------------------

/// Shared handle exposing the latest full gating profile from Autonomic.
pub type GatingProfileHandle = Arc<RwLock<Option<LocalGatingProfile>>>;

/// Per-tick counters enforced by the gating middleware.
///
/// Counters are reset at the start of each run (via `reset_counters`).
/// They use atomics for lock-free reads from the synchronous `Middleware` trait.
pub struct AutonomicGatingState {
    /// Number of tool calls executed in the current tick.
    tool_calls: AtomicU32,
    /// Number of file mutations in the current tick.
    file_mutations: AtomicU32,
    /// Cached gating profile from the last Autonomic evaluation.
    profile: GatingProfileHandle,
    /// Last observed operating mode (for transition detection).
    last_mode: RwLock<OperatingMode>,
}

impl Default for AutonomicGatingState {
    fn default() -> Self {
        Self::new()
    }
}

impl AutonomicGatingState {
    pub fn new() -> Self {
        Self {
            tool_calls: AtomicU32::new(0),
            file_mutations: AtomicU32::new(0),
            profile: Arc::new(RwLock::new(None)),
            last_mode: RwLock::new(OperatingMode::Execute),
        }
    }

    /// Reset per-tick counters. Called at the start of each agent run.
    pub fn reset_counters(&self) {
        self.tool_calls.store(0, Ordering::Relaxed);
        self.file_mutations.store(0, Ordering::Relaxed);
    }

    /// Update the cached gating profile.
    pub async fn update_profile(&self, profile: LocalGatingProfile) {
        // Log rationale if present.
        if !profile.rationale.is_empty() {
            tracing::info!(
                rationale = ?profile.rationale,
                mode_hint = ?infer_mode(&profile),
                "Autonomic gating rationale"
            );
        }

        // Detect mode transitions.
        let new_mode = infer_mode(&profile);
        let mut last = self.last_mode.write().await;
        if *last != new_mode {
            tracing::info!(
                from = ?*last,
                to = ?new_mode,
                "Autonomic operating mode transition"
            );
            *last = new_mode;
        }

        let mut handle = self.profile.write().await;
        *handle = Some(profile);
    }

    /// Get the current gating profile (blocking-safe clone).
    pub fn profile_snapshot(&self) -> Option<LocalGatingProfile> {
        // Use try_read to avoid blocking in synchronous context.
        self.profile.try_read().ok().and_then(|guard| guard.clone())
    }

    fn increment_tool_calls(&self) -> u32 {
        self.tool_calls.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn increment_file_mutations(&self) -> u32 {
        self.file_mutations.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Read the current tool call count (useful for diagnostics).
    pub fn current_tool_calls(&self) -> u32 {
        self.tool_calls.load(Ordering::Relaxed)
    }
}

/// Infer the OperatingMode from the gating profile.
///
/// This is a heuristic based on the profile's constraints:
/// - `allow_side_effects = false` → Recover (most restrictive)
/// - Both shell and network disabled → Explore (read-only gathering)
/// - Shell disabled or file mutations heavily restricted → Verify (guarded)
/// - Low tool call budget (≤ 2) → Verify
/// - Otherwise → Execute (normal productive mode)
fn infer_mode(profile: &LocalGatingProfile) -> OperatingMode {
    if !profile.operational.allow_side_effects {
        return OperatingMode::Recover;
    }
    if !profile.operational.allow_shell && !profile.operational.allow_network {
        return OperatingMode::Explore;
    }
    if !profile.operational.allow_shell
        || profile.operational.max_file_mutations_per_tick <= 1
        || profile.operational.max_tool_calls_per_tick <= 2
    {
        return OperatingMode::Verify;
    }
    OperatingMode::Execute
}

// ---------------------------------------------------------------------------
// Middleware implementation
// ---------------------------------------------------------------------------

/// Middleware that enforces Autonomic gating decisions in the Arcan
/// [`Orchestrator`] agent loop.
///
/// Wire this into the orchestrator's middleware stack to get dynamic,
/// per-tick enforcement of tool call limits, file mutation caps, and
/// network/shell restrictions based on the Autonomic homeostasis controller.
pub struct AutonomicGatingMiddleware {
    state: Arc<AutonomicGatingState>,
}

impl AutonomicGatingMiddleware {
    pub fn new(state: Arc<AutonomicGatingState>) -> Self {
        Self { state }
    }
}

impl Middleware for AutonomicGatingMiddleware {
    fn before_model_call(&self, _request: &ProviderRequest) -> Result<(), CoreError> {
        // Reset counters at the start of each iteration? No — counters are per-tick,
        // and a tick may span multiple iterations. The caller resets via `reset_counters`.
        Ok(())
    }

    fn pre_tool_call(&self, _context: &ToolContext, call: &ToolCall) -> Result<(), CoreError> {
        let Some(profile) = self.state.profile_snapshot() else {
            // No profile available — advisory fallthrough.
            return Ok(());
        };

        let operational = &profile.operational;

        // 1. Check allow_side_effects (nuclear gate).
        if !operational.allow_side_effects && is_side_effect_tool(&call.tool_name) {
            return Err(CoreError::Middleware(format!(
                "Autonomic: side effects disabled, blocking tool '{}'",
                call.tool_name
            )));
        }

        // 2. Check allow_shell.
        if !operational.allow_shell && is_shell_tool(&call.tool_name) {
            return Err(CoreError::Middleware(format!(
                "Autonomic: shell execution disabled, blocking tool '{}'",
                call.tool_name
            )));
        }

        // 3. Check allow_network.
        if !operational.allow_network && is_network_tool(&call.tool_name) {
            return Err(CoreError::Middleware(format!(
                "Autonomic: network access disabled, blocking tool '{}'",
                call.tool_name
            )));
        }

        // 4. Check max_tool_calls_per_tick.
        let count = self.state.increment_tool_calls();
        if count > operational.max_tool_calls_per_tick {
            return Err(CoreError::Middleware(format!(
                "Autonomic: tool call limit exceeded ({}/{} per tick)",
                count, operational.max_tool_calls_per_tick
            )));
        }

        // 5. Check max_file_mutations_per_tick (only for write tools).
        if is_file_mutation_tool(&call.tool_name) {
            let mutations = self.state.increment_file_mutations();
            if mutations > operational.max_file_mutations_per_tick {
                return Err(CoreError::Middleware(format!(
                    "Autonomic: file mutation limit exceeded ({}/{} per tick)",
                    mutations, operational.max_file_mutations_per_tick
                )));
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tool classification helpers
// ---------------------------------------------------------------------------

/// Returns true if the tool produces side effects (writes, deletes, executes).
fn is_side_effect_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file"
            | "edit_file"
            | "bash"
            | "shell"
            | "delete_file"
            | "create_directory"
            | "move_file"
            | "memory_propose"
            | "memory_commit"
    )
}

/// Returns true if the tool executes shell commands.
fn is_shell_tool(name: &str) -> bool {
    matches!(name, "bash" | "shell" | "exec" | "command")
}

/// Returns true if the tool performs network operations.
fn is_network_tool(name: &str) -> bool {
    matches!(name, "http_request" | "web_search" | "fetch_url" | "curl")
}

/// Returns true if the tool mutates files on disk.
fn is_file_mutation_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file" | "edit_file" | "delete_file" | "create_directory" | "move_file"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autonomic::{EconomicGates, EconomicMode, ModelTier};
    use aios_protocol::GatingProfile;
    use aios_protocol::event::RiskLevel;

    fn permissive_profile() -> LocalGatingProfile {
        LocalGatingProfile {
            operational: GatingProfile::default(),
            economic: EconomicGates {
                economic_mode: EconomicMode::Sovereign,
                max_tokens_next_turn: None,
                preferred_model: None,
                allow_expensive_tools: true,
                allow_replication: true,
            },
            rationale: vec![],
        }
    }

    fn restrictive_profile() -> LocalGatingProfile {
        LocalGatingProfile {
            operational: GatingProfile {
                allow_side_effects: false,
                require_approval_for_risk: RiskLevel::Low,
                max_tool_calls_per_tick: 2,
                max_file_mutations_per_tick: 0,
                allow_network: false,
                allow_shell: false,
            },
            economic: EconomicGates {
                economic_mode: EconomicMode::Hibernate,
                max_tokens_next_turn: Some(100),
                preferred_model: Some(ModelTier::Budget),
                allow_expensive_tools: false,
                allow_replication: false,
            },
            rationale: vec!["balance depleted".into()],
        }
    }

    fn limited_profile() -> LocalGatingProfile {
        LocalGatingProfile {
            operational: GatingProfile {
                allow_side_effects: true,
                require_approval_for_risk: RiskLevel::Medium,
                max_tool_calls_per_tick: 3,
                max_file_mutations_per_tick: 1,
                allow_network: true,
                allow_shell: false,
            },
            economic: EconomicGates {
                economic_mode: EconomicMode::Conserving,
                max_tokens_next_turn: Some(2000),
                preferred_model: Some(ModelTier::Standard),
                allow_expensive_tools: true,
                allow_replication: false,
            },
            rationale: vec!["approaching burn limit".into()],
        }
    }

    fn make_context() -> ToolContext {
        ToolContext {
            run_id: "r1".into(),
            session_id: "s1".into(),
            iteration: 1,
        }
    }

    fn make_call(name: &str) -> ToolCall {
        ToolCall {
            call_id: "c1".into(),
            tool_name: name.into(),
            input: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn allows_all_when_no_profile() {
        let state = Arc::new(AutonomicGatingState::new());
        let mw = AutonomicGatingMiddleware::new(state);
        let ctx = make_context();
        let call = make_call("write_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_ok());
    }

    #[tokio::test]
    async fn blocks_side_effects_when_disabled() {
        let state = Arc::new(AutonomicGatingState::new());
        state.update_profile(restrictive_profile()).await;
        let mw = AutonomicGatingMiddleware::new(state);
        let ctx = make_context();

        // Write tool should be blocked.
        let call = make_call("write_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_err());

        // Read tool should be allowed.
        let call = make_call("read_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_ok());
    }

    #[tokio::test]
    async fn blocks_shell_when_disabled() {
        let state = Arc::new(AutonomicGatingState::new());
        state.update_profile(limited_profile()).await;
        let mw = AutonomicGatingMiddleware::new(state);
        let ctx = make_context();
        let call = make_call("bash");
        assert!(mw.pre_tool_call(&ctx, &call).is_err());
    }

    #[tokio::test]
    async fn enforces_tool_call_limit() {
        let state = Arc::new(AutonomicGatingState::new());
        state.update_profile(limited_profile()).await; // max 3 tool calls
        let mw = AutonomicGatingMiddleware::new(state.clone());
        let ctx = make_context();

        // Calls 1-3 should succeed (read_file is not side-effect).
        for _ in 0..3 {
            let call = make_call("read_file");
            assert!(mw.pre_tool_call(&ctx, &call).is_ok());
        }

        // Call 4 should be blocked.
        let call = make_call("read_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_err());
    }

    #[tokio::test]
    async fn enforces_file_mutation_limit() {
        let state = Arc::new(AutonomicGatingState::new());
        state.update_profile(limited_profile()).await; // max 1 file mutation
        let mw = AutonomicGatingMiddleware::new(state);
        let ctx = make_context();

        // First write should succeed.
        let call = make_call("write_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_ok());

        // Second write should be blocked.
        let call = make_call("edit_file");
        assert!(mw.pre_tool_call(&ctx, &call).is_err());
    }

    #[tokio::test]
    async fn reset_counters_works() {
        let state = Arc::new(AutonomicGatingState::new());
        state.update_profile(limited_profile()).await; // max 3 tool calls
        let mw = AutonomicGatingMiddleware::new(state.clone());
        let ctx = make_context();

        // Exhaust the limit.
        for _ in 0..3 {
            let call = make_call("read_file");
            mw.pre_tool_call(&ctx, &call).unwrap();
        }

        // Should be blocked.
        assert!(mw.pre_tool_call(&ctx, &make_call("read_file")).is_err());

        // Reset counters.
        state.reset_counters();

        // Should succeed again.
        assert!(mw.pre_tool_call(&ctx, &make_call("read_file")).is_ok());
    }

    #[tokio::test]
    async fn mode_transition_detection() {
        let state = Arc::new(AutonomicGatingState::new());

        // Start with permissive profile (Execute mode).
        state.update_profile(permissive_profile()).await;
        {
            let mode = state.last_mode.read().await;
            assert_eq!(*mode, OperatingMode::Execute);
        }

        // Switch to restrictive profile (Recover mode).
        state.update_profile(restrictive_profile()).await;
        {
            let mode = state.last_mode.read().await;
            assert_eq!(*mode, OperatingMode::Recover);
        }
    }

    #[test]
    fn tool_classification() {
        assert!(is_side_effect_tool("write_file"));
        assert!(is_side_effect_tool("bash"));
        assert!(!is_side_effect_tool("read_file"));
        assert!(!is_side_effect_tool("glob"));

        assert!(is_shell_tool("bash"));
        assert!(!is_shell_tool("write_file"));

        assert!(is_network_tool("http_request"));
        assert!(!is_network_tool("read_file"));

        assert!(is_file_mutation_tool("write_file"));
        assert!(is_file_mutation_tool("edit_file"));
        assert!(!is_file_mutation_tool("bash"));
        assert!(!is_file_mutation_tool("read_file"));
    }

    #[test]
    fn infer_mode_from_profile() {
        assert_eq!(infer_mode(&permissive_profile()), OperatingMode::Execute);
        assert_eq!(infer_mode(&restrictive_profile()), OperatingMode::Recover);
        assert_eq!(infer_mode(&limited_profile()), OperatingMode::Verify);
    }
}
