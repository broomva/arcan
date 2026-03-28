//! Promotion Gate — policy-gated rule promotion with rollback.
//!
//! Phase 2.3 of the self-learning pipeline. Takes `RuleProposal` entries
//! produced by the consolidation job (Phase 2.2), validates them against
//! policy, optionally routes through the approval gate, and commits them
//! as active rules. Supports full rollback via `learning.reverted` events.
//!
//! Event types emitted (all `EventPayload::Custom` with `"learning."` prefix):
//! - `learning.promoted`  — proposal accepted, rule now active
//! - `learning.rejected`  — proposal explicitly rejected (with reason)
//! - `learning.reverted`  — previously promoted rule rolled back

use lago_core::event::{EventEnvelope, EventPayload, PolicyDecisionKind};
use lago_core::id::{BranchId, EventId, SessionId};
use lago_core::projection::Projection;
use lago_core::{Journal, LagoError, LagoResult};
use lago_policy::PolicyEngine;
use lago_store::BlobStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Event type constants ────────────────────────────────────────────

pub mod event_types {
    /// Emitted when a rule proposal is promoted to an active rule.
    pub const LEARNING_PROMOTED: &str = "learning.promoted";
    /// Emitted when a rule proposal is explicitly rejected.
    pub const LEARNING_REJECTED: &str = "learning.rejected";
    /// Emitted when a previously promoted rule is rolled back.
    pub const LEARNING_REVERTED: &str = "learning.reverted";
}

// ── Types ───────────────────────────────────────────────────────────

/// What a promoted rule does when it takes effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value")]
pub enum ProposalAction {
    /// Inject guidance text into the system prompt's Rules block.
    AddSystemPromptGuidance(String),
    /// Add a policy rule to the runtime policy engine.
    AddPolicyRule {
        rule_id: String,
        name: String,
        priority: u32,
        condition_json: serde_json::Value,
        decision: String,
        explanation: Option<String>,
    },
    /// Modify a tool's runtime configuration.
    ModifyToolConfig {
        tool_name: String,
        config_patch: serde_json::Value,
    },
}

/// A rule proposal produced by the consolidation job (Phase 2.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleProposal {
    /// Unique identifier for this proposal.
    pub proposal_id: String,
    /// Human-readable description of the proposed rule.
    pub description: String,
    /// The action to take if this proposal is promoted.
    pub action: ProposalAction,
    /// Confidence score from the consolidation job (0.0–1.0).
    pub confidence: f64,
    /// Source learning entries that led to this proposal.
    pub source_learning_ids: Vec<String>,
    /// Session where the proposal was generated.
    pub session_id: String,
}

/// Configuration for the promotion gate.
#[derive(Debug, Clone)]
pub struct PromotionConfig {
    /// If true, proposals below `auto_promote_confidence` require human approval.
    pub require_human_approval: bool,
    /// Confidence threshold for automatic promotion (default: 0.95).
    pub auto_promote_confidence: f64,
    /// Maximum number of active rules (safety cap, default: 50).
    pub max_active_rules: usize,
    /// Cooldown period after a revert before the same proposal can be re-promoted.
    pub cooldown_after_revert: Duration,
}

impl Default for PromotionConfig {
    fn default() -> Self {
        Self {
            require_human_approval: true,
            auto_promote_confidence: 0.95,
            max_active_rules: 50,
            cooldown_after_revert: Duration::from_secs(24 * 60 * 60),
        }
    }
}

/// A rule that has been promoted from a proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotedRule {
    /// Unique ID for this active rule (generated on promotion).
    pub rule_id: String,
    /// The proposal that was promoted.
    pub proposal_id: String,
    /// Human-readable rule text.
    pub rule_text: String,
    /// The action this rule enacts.
    pub action: ProposalAction,
    /// Journal sequence number at promotion time.
    pub promoted_at: u64,
    /// Whether this rule has been reverted.
    pub reverted: bool,
}

/// An event in the promotion audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionEvent {
    pub event_type: String,
    pub rule_id: Option<String>,
    pub proposal_id: String,
    pub reason: Option<String>,
    pub timestamp: u64,
    pub seq: u64,
}

/// Error types specific to the promotion gate.
#[derive(Debug, thiserror::Error)]
pub enum PromotionError {
    #[error("proposal {0} not found")]
    ProposalNotFound(String),
    #[error("rule {0} not found")]
    RuleNotFound(String),
    #[error("rule {0} already reverted")]
    AlreadyReverted(String),
    #[error("active rule limit reached ({0})")]
    ActiveRuleLimitReached(usize),
    #[error("policy conflict: {0}")]
    PolicyConflict(String),
    #[error("proposal {0} is in cooldown after revert")]
    CooldownActive(String),
    #[error("requires human approval")]
    RequiresApproval,
    #[error("lago error: {0}")]
    Lago(#[from] LagoError),
}

// ── Active Rule Set Projection ──────────────────────────────────────

/// Projection that rebuilds the set of active promoted rules by folding
/// over `learning.promoted` and `learning.reverted` events.
#[derive(Debug, Default, Clone)]
pub struct ActiveRuleSet {
    rules: Vec<PromotedRule>,
    history: Vec<PromotionEvent>,
    /// Tracks reverted proposal IDs with their revert timestamp (micros).
    reverted_proposals: HashMap<String, u64>,
}

impl ActiveRuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all currently active (non-reverted) rules.
    pub fn active_rules(&self) -> Vec<&PromotedRule> {
        self.rules.iter().filter(|r| !r.reverted).collect()
    }

    /// Count of active (non-reverted) rules.
    pub fn active_count(&self) -> usize {
        self.rules.iter().filter(|r| !r.reverted).count()
    }

    /// Get the full promotion history (all events).
    pub fn history(&self) -> &[PromotionEvent] {
        &self.history
    }

    /// Check if a proposal was recently reverted (within cooldown).
    pub fn is_in_cooldown(&self, proposal_id: &str, cooldown: Duration, now_micros: u64) -> bool {
        if let Some(&revert_time) = self.reverted_proposals.get(proposal_id) {
            let cooldown_micros = cooldown.as_micros() as u64;
            return now_micros.saturating_sub(revert_time) < cooldown_micros;
        }
        false
    }

    /// Find a promoted rule by rule_id.
    pub fn find_rule(&self, rule_id: &str) -> Option<&PromotedRule> {
        self.rules.iter().find(|r| r.rule_id == rule_id)
    }

    /// Fold a single event into the projection.
    pub fn fold(&mut self, event: &EventEnvelope) {
        if let EventPayload::Custom {
            ref event_type,
            ref data,
        } = event.payload
        {
            match event_type.as_str() {
                event_types::LEARNING_PROMOTED => {
                    if let (Some(rule_id), Some(proposal_id), Some(rule_text)) = (
                        data.get("rule_id").and_then(|v| v.as_str()),
                        data.get("proposal_id").and_then(|v| v.as_str()),
                        data.get("rule_text").and_then(|v| v.as_str()),
                    ) {
                        let action: Option<ProposalAction> = data
                            .get("action")
                            .and_then(|v| serde_json::from_value(v.clone()).ok());

                        self.rules.push(PromotedRule {
                            rule_id: rule_id.to_string(),
                            proposal_id: proposal_id.to_string(),
                            rule_text: rule_text.to_string(),
                            action: action.unwrap_or(ProposalAction::AddSystemPromptGuidance(
                                rule_text.to_string(),
                            )),
                            promoted_at: event.seq,
                            reverted: false,
                        });

                        self.history.push(PromotionEvent {
                            event_type: event_types::LEARNING_PROMOTED.to_string(),
                            rule_id: Some(rule_id.to_string()),
                            proposal_id: proposal_id.to_string(),
                            reason: None,
                            timestamp: event.timestamp,
                            seq: event.seq,
                        });
                    }
                }
                event_types::LEARNING_REJECTED => {
                    if let Some(proposal_id) = data.get("proposal_id").and_then(|v| v.as_str()) {
                        self.history.push(PromotionEvent {
                            event_type: event_types::LEARNING_REJECTED.to_string(),
                            rule_id: None,
                            proposal_id: proposal_id.to_string(),
                            reason: data
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            timestamp: event.timestamp,
                            seq: event.seq,
                        });
                    }
                }
                event_types::LEARNING_REVERTED => {
                    if let Some(rule_id) = data.get("rule_id").and_then(|v| v.as_str()) {
                        // Mark the rule as reverted
                        for rule in &mut self.rules {
                            if rule.rule_id == rule_id {
                                rule.reverted = true;
                                // Track revert for cooldown
                                self.reverted_proposals
                                    .insert(rule.proposal_id.clone(), event.timestamp);
                            }
                        }

                        self.history.push(PromotionEvent {
                            event_type: event_types::LEARNING_REVERTED.to_string(),
                            rule_id: Some(rule_id.to_string()),
                            proposal_id: data
                                .get("proposal_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            reason: data
                                .get("reason")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            timestamp: event.timestamp,
                            seq: event.seq,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

impl Projection for ActiveRuleSet {
    fn on_event(&mut self, event: &EventEnvelope) -> LagoResult<()> {
        self.fold(event);
        Ok(())
    }

    fn name(&self) -> &str {
        "arcan::active_rules"
    }
}

// ── Promotion Gate ──────────────────────────────────────────────────

/// Policy-gated promotion mechanism for rule proposals.
///
/// Takes proposals from the consolidation job, validates against policy,
/// optionally requires human approval, commits as active rules, and
/// supports full rollback.
pub struct PromotionGate {
    journal: Arc<dyn Journal>,
    blob_store: Arc<BlobStore>,
    policy: Arc<Mutex<PolicyEngine>>,
    config: PromotionConfig,
    session_id: SessionId,
    branch_id: BranchId,
    /// Local projection rebuilt from events.
    rule_set: Mutex<ActiveRuleSet>,
}

impl PromotionGate {
    pub fn new(
        journal: Arc<dyn Journal>,
        blob_store: Arc<BlobStore>,
        policy: Arc<Mutex<PolicyEngine>>,
        config: PromotionConfig,
        session_id: SessionId,
        branch_id: BranchId,
    ) -> Self {
        Self {
            journal,
            blob_store,
            policy,
            config,
            session_id,
            branch_id,
            rule_set: Mutex::new(ActiveRuleSet::new()),
        }
    }

    /// Rebuild the projection by replaying all learning.* events from the journal.
    pub async fn rebuild_projection(&self) -> Result<(), PromotionError> {
        let query = lago_core::journal::EventQuery::default().session(self.session_id.clone());
        let events = self.journal.read(query).await?;

        let mut rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;
        *rule_set = ActiveRuleSet::new();
        for event in &events {
            rule_set.fold(event);
        }
        Ok(())
    }

    /// Promote a rule proposal to an active rule.
    ///
    /// Checks policy compatibility, enforces limits, and optionally
    /// requires human approval based on confidence threshold.
    pub async fn promote(&self, proposal: &RuleProposal) -> Result<PromotedRule, PromotionError> {
        let now_micros = EventEnvelope::now_micros();

        // Check cooldown
        {
            let rule_set = self
                .rule_set
                .lock()
                .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

            if rule_set.is_in_cooldown(
                &proposal.proposal_id,
                self.config.cooldown_after_revert,
                now_micros,
            ) {
                return Err(PromotionError::CooldownActive(proposal.proposal_id.clone()));
            }

            // Check active rule limit
            if rule_set.active_count() >= self.config.max_active_rules {
                return Err(PromotionError::ActiveRuleLimitReached(
                    self.config.max_active_rules,
                ));
            }
        }

        // Policy conflict detection for AddPolicyRule actions
        if let ProposalAction::AddPolicyRule {
            ref condition_json,
            ref decision,
            ..
        } = proposal.action
        {
            self.check_policy_conflict(condition_json, decision)?;
        }

        // Approval routing
        let needs_approval = self.config.require_human_approval
            && proposal.confidence < self.config.auto_promote_confidence;
        if needs_approval {
            return Err(PromotionError::RequiresApproval);
        }

        // Generate rule ID and store rule content in blob store
        let rule_id = format!("rule-{}", uuid::Uuid::new_v4());
        let rule_content = serde_json::to_vec(proposal)
            .map_err(|e| LagoError::Internal(format!("serialization failed: {e}")))?;
        let _blob_hash = self.blob_store.put(&rule_content)?;

        // Build the promoted rule
        let promoted = PromotedRule {
            rule_id: rule_id.clone(),
            proposal_id: proposal.proposal_id.clone(),
            rule_text: proposal.description.clone(),
            action: proposal.action.clone(),
            promoted_at: 0, // Will be set by journal seq
            reverted: false,
        };

        // Emit learning.promoted event
        let event = build_learning_event(
            &self.session_id,
            &self.branch_id,
            event_types::LEARNING_PROMOTED,
            serde_json::json!({
                "rule_id": rule_id,
                "proposal_id": proposal.proposal_id,
                "rule_text": proposal.description,
                "action": proposal.action,
                "confidence": proposal.confidence,
                "source_learning_ids": proposal.source_learning_ids,
            }),
        );
        let seq = self.journal.append(event).await?;

        // Update local projection
        let mut rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

        let mut promoted_with_seq = promoted;
        promoted_with_seq.promoted_at = seq.into();
        rule_set.rules.push(promoted_with_seq.clone());
        rule_set.history.push(PromotionEvent {
            event_type: event_types::LEARNING_PROMOTED.to_string(),
            rule_id: Some(rule_id),
            proposal_id: proposal.proposal_id.clone(),
            reason: None,
            timestamp: now_micros,
            seq: seq.into(),
        });

        Ok(promoted_with_seq)
    }

    /// Reject a rule proposal with a reason.
    pub async fn reject(&self, proposal_id: &str, reason: &str) -> Result<(), PromotionError> {
        let now_micros = EventEnvelope::now_micros();

        let event = build_learning_event(
            &self.session_id,
            &self.branch_id,
            event_types::LEARNING_REJECTED,
            serde_json::json!({
                "proposal_id": proposal_id,
                "reason": reason,
            }),
        );
        let seq = self.journal.append(event).await?;

        // Update local projection
        let mut rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

        rule_set.history.push(PromotionEvent {
            event_type: event_types::LEARNING_REJECTED.to_string(),
            rule_id: None,
            proposal_id: proposal_id.to_string(),
            reason: Some(reason.to_string()),
            timestamp: now_micros,
            seq: seq.into(),
        });

        Ok(())
    }

    /// Revert a previously promoted rule.
    pub async fn revert(&self, rule_id: &str, reason: &str) -> Result<(), PromotionError> {
        let now_micros = EventEnvelope::now_micros();

        // Find the rule and get its proposal_id
        let proposal_id = {
            let rule_set = self
                .rule_set
                .lock()
                .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

            let rule = rule_set
                .find_rule(rule_id)
                .ok_or_else(|| PromotionError::RuleNotFound(rule_id.to_string()))?;

            if rule.reverted {
                return Err(PromotionError::AlreadyReverted(rule_id.to_string()));
            }

            rule.proposal_id.clone()
        };

        let event = build_learning_event(
            &self.session_id,
            &self.branch_id,
            event_types::LEARNING_REVERTED,
            serde_json::json!({
                "rule_id": rule_id,
                "proposal_id": proposal_id,
                "reason": reason,
            }),
        );
        let seq = self.journal.append(event).await?;

        // Update local projection
        let mut rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

        for rule in &mut rule_set.rules {
            if rule.rule_id == rule_id {
                rule.reverted = true;
                rule_set
                    .reverted_proposals
                    .insert(rule.proposal_id.clone(), now_micros);
            }
        }

        rule_set.history.push(PromotionEvent {
            event_type: event_types::LEARNING_REVERTED.to_string(),
            rule_id: Some(rule_id.to_string()),
            proposal_id,
            reason: Some(reason.to_string()),
            timestamp: now_micros,
            seq: seq.into(),
        });

        Ok(())
    }

    /// Get the current set of active (non-reverted) rules.
    pub fn active_rules(&self) -> Result<Vec<PromotedRule>, PromotionError> {
        let rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

        Ok(rule_set.active_rules().into_iter().cloned().collect())
    }

    /// Get the full promotion history (audit trail).
    pub fn promotion_history(&self) -> Result<Vec<PromotionEvent>, PromotionError> {
        let rule_set = self
            .rule_set
            .lock()
            .map_err(|e| LagoError::Internal(format!("lock poisoned: {e}")))?;

        Ok(rule_set.history().to_vec())
    }

    /// Check whether a proposed policy rule conflicts with existing rules.
    fn check_policy_conflict(
        &self,
        condition_json: &serde_json::Value,
        decision: &str,
    ) -> Result<(), PromotionError> {
        let policy = self
            .policy
            .lock()
            .map_err(|e| LagoError::Internal(format!("policy lock poisoned: {e}")))?;

        // Parse the proposed condition to check for direct contradictions
        let proposed_condition: Result<lago_policy::MatchCondition, _> =
            serde_json::from_value(condition_json.clone());

        let Ok(proposed) = proposed_condition else {
            return Err(PromotionError::PolicyConflict(
                "invalid condition format".to_string(),
            ));
        };

        // Check for contradictions: same condition with opposite decision
        let proposed_decision = match decision {
            "allow" | "Allow" => PolicyDecisionKind::Allow,
            "deny" | "Deny" => PolicyDecisionKind::Deny,
            "require_approval" | "RequireApproval" => PolicyDecisionKind::RequireApproval,
            other => {
                return Err(PromotionError::PolicyConflict(format!(
                    "unknown decision kind: {other}"
                )));
            }
        };

        // Simple conflict detection: if any existing rule has the same
        // serialized condition but opposite decision, flag it.
        for existing in policy.rules() {
            let existing_json = serde_json::to_value(&existing.condition).unwrap_or_default();
            let proposed_json = serde_json::to_value(&proposed).unwrap_or_default();

            if existing_json == proposed_json && existing.decision != proposed_decision {
                return Err(PromotionError::PolicyConflict(format!(
                    "contradicts existing rule '{}' ({}): same condition, different decision",
                    existing.name, existing.id
                )));
            }
        }

        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Build a Lago `EventEnvelope` for a learning event.
fn build_learning_event(
    session_id: &SessionId,
    branch_id: &BranchId,
    event_type: &str,
    data: serde_json::Value,
) -> EventEnvelope {
    EventEnvelope {
        event_id: EventId::new(),
        session_id: session_id.clone(),
        branch_id: branch_id.clone(),
        run_id: None,
        seq: 0, // Auto-assigned by journal
        timestamp: EventEnvelope::now_micros(),
        parent_id: None,
        payload: EventPayload::Custom {
            event_type: event_type.to_string(),
            data,
        },
        metadata: HashMap::new(),
        schema_version: 1,
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::id::*;

    fn make_custom_event(
        event_type: &str,
        data: serde_json::Value,
        seq: u64,
        timestamp: u64,
    ) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: SessionId::from_string("test"),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq,
            timestamp,
            parent_id: None,
            payload: EventPayload::Custom {
                event_type: event_type.to_string(),
                data,
            },
            metadata: HashMap::new(),
            schema_version: 1,
        }
    }

    fn sample_proposal(id: &str, confidence: f64) -> RuleProposal {
        RuleProposal {
            proposal_id: id.to_string(),
            description: format!("Test rule from proposal {id}"),
            action: ProposalAction::AddSystemPromptGuidance(
                "Always check file permissions before writing".to_string(),
            ),
            confidence,
            source_learning_ids: vec!["learn-1".to_string()],
            session_id: "sess-1".to_string(),
        }
    }

    fn sample_policy_proposal(id: &str, confidence: f64) -> RuleProposal {
        RuleProposal {
            proposal_id: id.to_string(),
            description: "Deny shell access".to_string(),
            action: ProposalAction::AddPolicyRule {
                rule_id: format!("pr-{id}"),
                name: "deny shell".to_string(),
                priority: 10,
                condition_json: serde_json::json!({"type": "ToolName", "value": "exec_shell"}),
                decision: "Deny".to_string(),
                explanation: Some("shell access denied by learning".to_string()),
            },
            confidence,
            source_learning_ids: vec!["learn-2".to_string()],
            session_id: "sess-1".to_string(),
        }
    }

    // ── Projection tests ────────────────────────────────────────────

    #[test]
    fn projection_promotes_rule() {
        let mut proj = ActiveRuleSet::new();

        proj.fold(&make_custom_event(
            event_types::LEARNING_PROMOTED,
            serde_json::json!({
                "rule_id": "r1",
                "proposal_id": "p1",
                "rule_text": "Always verify permissions",
                "action": {"type": "AddSystemPromptGuidance", "value": "Always verify permissions"},
            }),
            1,
            1000,
        ));

        assert_eq!(proj.active_count(), 1);
        let active = proj.active_rules();
        assert_eq!(active[0].rule_id, "r1");
        assert_eq!(active[0].proposal_id, "p1");
        assert!(!active[0].reverted);
    }

    #[test]
    fn projection_rejects_proposal() {
        let mut proj = ActiveRuleSet::new();

        proj.fold(&make_custom_event(
            event_types::LEARNING_REJECTED,
            serde_json::json!({
                "proposal_id": "p2",
                "reason": "too risky",
            }),
            1,
            1000,
        ));

        // Rejected proposals don't appear in active rules
        assert_eq!(proj.active_count(), 0);
        // But they show up in history
        assert_eq!(proj.history().len(), 1);
        assert_eq!(proj.history()[0].event_type, event_types::LEARNING_REJECTED);
        assert_eq!(proj.history()[0].reason.as_deref(), Some("too risky"));
    }

    #[test]
    fn projection_reverts_promoted_rule() {
        let mut proj = ActiveRuleSet::new();

        // Promote
        proj.fold(&make_custom_event(
            event_types::LEARNING_PROMOTED,
            serde_json::json!({
                "rule_id": "r1",
                "proposal_id": "p1",
                "rule_text": "Check perms",
                "action": {"type": "AddSystemPromptGuidance", "value": "Check perms"},
            }),
            1,
            1000,
        ));
        assert_eq!(proj.active_count(), 1);

        // Revert
        proj.fold(&make_custom_event(
            event_types::LEARNING_REVERTED,
            serde_json::json!({
                "rule_id": "r1",
                "proposal_id": "p1",
                "reason": "caused regressions",
            }),
            2,
            2000,
        ));

        assert_eq!(proj.active_count(), 0);
        assert_eq!(proj.history().len(), 2);
    }

    #[test]
    fn projection_cooldown_after_revert() {
        let mut proj = ActiveRuleSet::new();

        // Promote and revert
        proj.fold(&make_custom_event(
            event_types::LEARNING_PROMOTED,
            serde_json::json!({
                "rule_id": "r1",
                "proposal_id": "p1",
                "rule_text": "Check perms",
            }),
            1,
            1_000_000,
        ));
        proj.fold(&make_custom_event(
            event_types::LEARNING_REVERTED,
            serde_json::json!({
                "rule_id": "r1",
                "proposal_id": "p1",
                "reason": "bad rule",
            }),
            2,
            2_000_000,
        ));

        // Immediately after revert: still in cooldown
        let cooldown = Duration::from_secs(3600); // 1 hour
        assert!(proj.is_in_cooldown("p1", cooldown, 2_500_000));

        // After cooldown expires: no longer in cooldown
        let far_future = 2_000_000 + 3_600_000_001; // revert_time + 1h + 1μs
        assert!(!proj.is_in_cooldown("p1", cooldown, far_future));

        // Unrelated proposal: never in cooldown
        assert!(!proj.is_in_cooldown("p999", cooldown, 2_500_000));
    }

    #[test]
    fn projection_ignores_unrelated_events() {
        let mut proj = ActiveRuleSet::new();

        proj.fold(&make_custom_event(
            "skill.activated",
            serde_json::json!({"name": "test"}),
            1,
            1000,
        ));

        assert_eq!(proj.active_count(), 0);
        assert!(proj.history().is_empty());
    }

    #[test]
    fn projection_multiple_rules_active() {
        let mut proj = ActiveRuleSet::new();

        for i in 0..5 {
            proj.fold(&make_custom_event(
                event_types::LEARNING_PROMOTED,
                serde_json::json!({
                    "rule_id": format!("r{i}"),
                    "proposal_id": format!("p{i}"),
                    "rule_text": format!("Rule {i}"),
                }),
                i as u64,
                (i * 1000) as u64,
            ));
        }

        assert_eq!(proj.active_count(), 5);

        // Revert middle rule
        proj.fold(&make_custom_event(
            event_types::LEARNING_REVERTED,
            serde_json::json!({
                "rule_id": "r2",
                "proposal_id": "p2",
                "reason": "not useful",
            }),
            5,
            5000,
        ));

        assert_eq!(proj.active_count(), 4);
        assert!(proj.find_rule("r2").unwrap().reverted);
        assert!(!proj.find_rule("r3").unwrap().reverted);
    }

    #[test]
    fn projection_name() {
        let proj = ActiveRuleSet::new();
        assert_eq!(proj.name(), "arcan::active_rules");
    }

    // ── Serialization tests ─────────────────────────────────────────

    #[test]
    fn proposal_action_serialization_roundtrip() {
        let actions = vec![
            ProposalAction::AddSystemPromptGuidance("Check perms".to_string()),
            ProposalAction::AddPolicyRule {
                rule_id: "r1".to_string(),
                name: "deny shell".to_string(),
                priority: 10,
                condition_json: serde_json::json!({"type": "ToolName", "value": "exec_shell"}),
                decision: "Deny".to_string(),
                explanation: Some("no shell".to_string()),
            },
            ProposalAction::ModifyToolConfig {
                tool_name: "bash".to_string(),
                config_patch: serde_json::json!({"timeout_ms": 5000}),
            },
        ];

        for action in &actions {
            let json = serde_json::to_string(action).unwrap();
            let back: ProposalAction = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, action);
        }
    }

    #[test]
    fn rule_proposal_serialization() {
        let proposal = sample_proposal("p1", 0.85);
        let json = serde_json::to_string(&proposal).unwrap();
        let back: RuleProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.proposal_id, "p1");
        assert_eq!(back.confidence, 0.85);
    }

    // ── Gate integration tests (async, with real journal) ───────────

    async fn setup_gate(config: PromotionConfig) -> PromotionGate {
        let dir = tempfile::tempdir().unwrap();
        let journal =
            Arc::new(lago_journal::RedbJournal::open(dir.path().join("test.redb")).unwrap());
        let blob_store = Arc::new(BlobStore::from_path(dir.path().join("blobs")).unwrap());
        let policy = Arc::new(Mutex::new(PolicyEngine::new()));

        // Create session
        let session = lago_core::Session {
            session_id: SessionId::from_string("sess-1"),
            config: lago_core::session::SessionConfig::new("test"),
            created_at: 0,
            branches: vec![],
        };
        journal.put_session(session).await.unwrap();

        PromotionGate::new(
            journal,
            blob_store,
            policy,
            config,
            SessionId::from_string("sess-1"),
            BranchId::from_string("main"),
        )
    }

    #[tokio::test]
    async fn gate_promote_creates_active_rule() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: false,
            auto_promote_confidence: 0.5,
            ..Default::default()
        })
        .await;

        let proposal = sample_proposal("p1", 0.9);
        let promoted = gate.promote(&proposal).await.unwrap();

        assert_eq!(promoted.proposal_id, "p1");
        assert!(!promoted.reverted);

        let active = gate.active_rules().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].proposal_id, "p1");
    }

    #[tokio::test]
    async fn gate_reject_emits_event() {
        let gate = setup_gate(PromotionConfig::default()).await;

        gate.reject("p1", "not useful").await.unwrap();

        let history = gate.promotion_history().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].event_type, event_types::LEARNING_REJECTED);
        assert_eq!(history[0].reason.as_deref(), Some("not useful"));

        // No active rules
        assert!(gate.active_rules().unwrap().is_empty());
    }

    #[tokio::test]
    async fn gate_revert_removes_from_active() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: false,
            auto_promote_confidence: 0.5,
            ..Default::default()
        })
        .await;

        let proposal = sample_proposal("p1", 0.9);
        let promoted = gate.promote(&proposal).await.unwrap();

        // Revert it
        gate.revert(&promoted.rule_id, "caused regressions")
            .await
            .unwrap();

        assert!(gate.active_rules().unwrap().is_empty());

        let history = gate.promotion_history().unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].event_type, event_types::LEARNING_REVERTED);
    }

    #[tokio::test]
    async fn gate_auto_promote_above_threshold() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: true,
            auto_promote_confidence: 0.90,
            ..Default::default()
        })
        .await;

        // High confidence → auto-promote even with require_human_approval
        let proposal = sample_proposal("p1", 0.95);
        let result = gate.promote(&proposal).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn gate_requires_approval_below_threshold() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: true,
            auto_promote_confidence: 0.95,
            ..Default::default()
        })
        .await;

        // Low confidence + require_human_approval → RequiresApproval error
        let proposal = sample_proposal("p1", 0.80);
        let result = gate.promote(&proposal).await;
        assert!(matches!(result, Err(PromotionError::RequiresApproval)));
    }

    #[tokio::test]
    async fn gate_enforces_max_active_rules() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: false,
            auto_promote_confidence: 0.5,
            max_active_rules: 3,
            ..Default::default()
        })
        .await;

        // Fill up to the limit
        for i in 0..3 {
            let proposal = sample_proposal(&format!("p{i}"), 0.9);
            gate.promote(&proposal).await.unwrap();
        }

        // Fourth should fail
        let proposal = sample_proposal("p3", 0.9);
        let result = gate.promote(&proposal).await;
        assert!(matches!(
            result,
            Err(PromotionError::ActiveRuleLimitReached(3))
        ));
    }

    #[tokio::test]
    async fn gate_policy_conflict_detection() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: false,
            auto_promote_confidence: 0.5,
            ..Default::default()
        })
        .await;

        // Add an existing rule to the policy engine
        {
            let mut policy = gate.policy.lock().unwrap();
            policy.add_rule(lago_policy::Rule {
                id: "existing-1".to_string(),
                name: "allow shell".to_string(),
                priority: 10,
                condition: lago_policy::MatchCondition::ToolName("exec_shell".to_string()),
                decision: PolicyDecisionKind::Allow,
                explanation: None,
                required_sandbox: None,
            });
        }

        // Propose a contradicting rule (same condition, Deny vs Allow)
        let proposal = sample_policy_proposal("conflict-1", 0.99);
        let result = gate.promote(&proposal).await;
        assert!(matches!(result, Err(PromotionError::PolicyConflict(_))));
    }

    #[tokio::test]
    async fn gate_revert_nonexistent_rule_fails() {
        let gate = setup_gate(PromotionConfig::default()).await;

        let result = gate.revert("nonexistent", "test").await;
        assert!(matches!(result, Err(PromotionError::RuleNotFound(_))));
    }

    #[tokio::test]
    async fn gate_double_revert_fails() {
        let gate = setup_gate(PromotionConfig {
            require_human_approval: false,
            auto_promote_confidence: 0.5,
            ..Default::default()
        })
        .await;

        let proposal = sample_proposal("p1", 0.9);
        let promoted = gate.promote(&proposal).await.unwrap();

        gate.revert(&promoted.rule_id, "first revert")
            .await
            .unwrap();

        let result = gate.revert(&promoted.rule_id, "second revert").await;
        assert!(matches!(result, Err(PromotionError::AlreadyReverted(_))));
    }
}
