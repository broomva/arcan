//! Health reporting configuration for fleet agents.
//!
//! Defines thresholds and health snapshot data structures that map to
//! the Spaces `AgentHealthSnapshot` table and Autonomic's three-pillar model.

use crate::vertical::AgentVertical;
use serde::{Deserialize, Serialize};

/// Health thresholds for a fleet agent.
///
/// These map to the Autonomic controller's gating profile and
/// determine when an agent is considered healthy vs degraded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthThresholds {
    /// Maximum consecutive errors before degraded status.
    pub max_error_streak: u32,
    /// Maximum context pressure (0.0-1.0) before pausing.
    pub max_context_pressure: f64,
    /// Minimum budget remaining percentage before entering conserving mode.
    pub min_budget_remaining_pct: f64,
    /// Maximum uncertainty (0.0-1.0) before requesting human review.
    pub max_uncertainty: f64,
    /// Target tasks per hour for throughput monitoring.
    pub target_tasks_per_hour: u32,
}

/// Health snapshot for reporting to Spaces.
///
/// Maps directly to the `update_health_snapshot` reducer parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetHealthSnapshot {
    /// Agent vertical.
    pub vertical: AgentVertical,
    /// Current task progress (0.0-1.0).
    pub progress: f64,
    /// Current uncertainty level (0.0-1.0).
    pub uncertainty: f64,
    /// Consecutive error count.
    pub error_streak: u32,
    /// Context window pressure (0.0-1.0).
    pub context_pressure: f64,
    /// Budget remaining percentage (0.0-100.0).
    pub budget_remaining_pct: f64,
    /// Current economic operating mode.
    pub operating_mode: String,
    /// Whether the agent is currently online and accepting tasks.
    pub is_online: bool,
    /// Tasks completed in the current reporting period.
    pub tasks_completed: u32,
    /// Tasks failed in the current reporting period.
    pub tasks_failed: u32,
}

impl FleetHealthSnapshot {
    /// Check if the agent is healthy given its thresholds.
    pub fn is_healthy(&self, thresholds: &HealthThresholds) -> bool {
        self.is_online
            && self.error_streak <= thresholds.max_error_streak
            && self.context_pressure <= thresholds.max_context_pressure
            && self.budget_remaining_pct >= thresholds.min_budget_remaining_pct
            && self.uncertainty <= thresholds.max_uncertainty
    }

    /// Compute a health score (0-100) for the Spaces listing.
    pub fn health_score(&self, thresholds: &HealthThresholds) -> u32 {
        if !self.is_online {
            return 0;
        }

        let mut score: f64 = 100.0;

        // Error streak penalty: -10 per error, capped
        let error_penalty = (self.error_streak as f64 / thresholds.max_error_streak as f64) * 30.0;
        score -= error_penalty.min(30.0);

        // Context pressure penalty
        let pressure_penalty = (self.context_pressure / thresholds.max_context_pressure) * 20.0;
        score -= pressure_penalty.min(20.0);

        // Budget penalty
        let budget_ratio = self.budget_remaining_pct / 100.0;
        if budget_ratio < thresholds.min_budget_remaining_pct / 100.0 {
            score -= 25.0;
        }

        // Uncertainty penalty
        let uncertainty_penalty = (self.uncertainty / thresholds.max_uncertainty) * 25.0;
        score -= uncertainty_penalty.min(25.0);

        score.clamp(0.0, 100.0) as u32
    }
}

/// Default health thresholds per vertical.
pub fn thresholds_for(vertical: AgentVertical) -> HealthThresholds {
    match vertical {
        AgentVertical::Coding => HealthThresholds {
            max_error_streak: 3,
            max_context_pressure: 0.85,
            min_budget_remaining_pct: 15.0,
            max_uncertainty: 0.7,
            target_tasks_per_hour: 10,
        },
        AgentVertical::DataProcessing => HealthThresholds {
            max_error_streak: 2,
            max_context_pressure: 0.80,
            min_budget_remaining_pct: 20.0,
            max_uncertainty: 0.6,
            target_tasks_per_hour: 8,
        },
        AgentVertical::Support => HealthThresholds {
            max_error_streak: 5, // more tolerant — some tickets are ambiguous
            max_context_pressure: 0.90,
            min_budget_remaining_pct: 10.0,
            max_uncertainty: 0.8,      // higher tolerance for uncertainty
            target_tasks_per_hour: 20, // support is high-throughput, low-complexity
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy_snapshot(vertical: AgentVertical) -> FleetHealthSnapshot {
        FleetHealthSnapshot {
            vertical,
            progress: 0.5,
            uncertainty: 0.2,
            error_streak: 0,
            context_pressure: 0.3,
            budget_remaining_pct: 80.0,
            operating_mode: "sovereign".into(),
            is_online: true,
            tasks_completed: 10,
            tasks_failed: 0,
        }
    }

    #[test]
    fn healthy_agent_passes() {
        let snap = healthy_snapshot(AgentVertical::Coding);
        let thresh = thresholds_for(AgentVertical::Coding);
        assert!(snap.is_healthy(&thresh));
    }

    #[test]
    fn offline_agent_not_healthy() {
        let mut snap = healthy_snapshot(AgentVertical::Coding);
        snap.is_online = false;
        let thresh = thresholds_for(AgentVertical::Coding);
        assert!(!snap.is_healthy(&thresh));
    }

    #[test]
    fn error_streak_degrades_health() {
        let mut snap = healthy_snapshot(AgentVertical::Coding);
        snap.error_streak = 5;
        let thresh = thresholds_for(AgentVertical::Coding);
        assert!(!snap.is_healthy(&thresh));
    }

    #[test]
    fn healthy_agent_scores_high() {
        let snap = healthy_snapshot(AgentVertical::Coding);
        let thresh = thresholds_for(AgentVertical::Coding);
        let score = snap.health_score(&thresh);
        assert!(score >= 70, "expected >= 70, got {score}");
    }

    #[test]
    fn offline_agent_scores_zero() {
        let mut snap = healthy_snapshot(AgentVertical::Coding);
        snap.is_online = false;
        let thresh = thresholds_for(AgentVertical::Coding);
        assert_eq!(snap.health_score(&thresh), 0);
    }

    #[test]
    fn degraded_agent_scores_lower() {
        let mut snap = healthy_snapshot(AgentVertical::DataProcessing);
        snap.error_streak = 2;
        snap.uncertainty = 0.5;
        snap.context_pressure = 0.7;
        let thresh = thresholds_for(AgentVertical::DataProcessing);
        let score = snap.health_score(&thresh);
        assert!(score < 70, "expected < 70, got {score}");
    }

    #[test]
    fn support_higher_throughput_target() {
        let coding = thresholds_for(AgentVertical::Coding);
        let support = thresholds_for(AgentVertical::Support);
        assert!(support.target_tasks_per_hour > coding.target_tasks_per_hour);
    }
}
