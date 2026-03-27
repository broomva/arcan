//! Haima task contracts for each agent vertical.
//!
//! Each contract defines outcome-based pricing, success criteria, and SLA.
//! These are registered with the Haima outcome engine at agent startup.

use crate::vertical::AgentVertical;
use haima_core::outcome::{RefundPolicy, SuccessCriterion, TaskComplexity, TaskContract, TaskType};

/// Return the primary Haima task contract for a vertical.
pub fn contract_for(vertical: AgentVertical) -> TaskContract {
    match vertical {
        AgentVertical::Coding => coding_contract(),
        AgentVertical::DataProcessing => data_contract(),
        AgentVertical::Support => support_contract(),
    }
}

/// Return all contracts for a vertical (primary + sub-task variants).
pub fn all_contracts_for(vertical: AgentVertical) -> Vec<TaskContract> {
    match vertical {
        AgentVertical::Coding => vec![
            coding_contract(),
            coding_bug_fix_contract(),
            coding_test_writing_contract(),
        ],
        AgentVertical::DataProcessing => vec![data_contract(), data_report_contract()],
        AgentVertical::Support => vec![support_contract()],
    }
}

// ---------------------------------------------------------------------------
// Coding contracts
// ---------------------------------------------------------------------------

/// Code review contract: $2 - $5 per PR.
fn coding_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-coding-review-v1".into(),
        task_type: TaskType::CodeReview,
        name: "Code Review (Fleet)".into(),
        price_floor_micro_credits: 2_000_000,   // $2.00
        price_ceiling_micro_credits: 5_000_000, // $5.00
        success_criteria: vec![
            SuccessCriterion::TestsPassed {
                scope: "unit".into(),
            },
            SuccessCriterion::Custom {
                description: "Code compiles without new warnings".into(),
            },
        ],
        refund_policy: RefundPolicy {
            sla_seconds: 7200, // 2 hours
            ..RefundPolicy::default()
        },
        min_trust_score: 0.3,
        custom_label: None,
        created_at: chrono::Utc::now(),
    }
}

/// Bug fix contract: $3 - $8 per fix (higher than review due to investigation).
fn coding_bug_fix_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-coding-bugfix-v1".into(),
        task_type: TaskType::CodeReview, // reuse CodeReview type
        name: "Bug Fix (Fleet)".into(),
        price_floor_micro_credits: 3_000_000,   // $3.00
        price_ceiling_micro_credits: 8_000_000, // $8.00
        success_criteria: vec![
            SuccessCriterion::TestsPassed {
                scope: "unit".into(),
            },
            SuccessCriterion::TestsPassed {
                scope: "integration".into(),
            },
            SuccessCriterion::Custom {
                description: "Bug reproduction test passes".into(),
            },
        ],
        refund_policy: RefundPolicy {
            sla_seconds: 3600, // 1 hour
            ..RefundPolicy::default()
        },
        min_trust_score: 0.5,
        custom_label: Some("bug_fix".into()),
        created_at: chrono::Utc::now(),
    }
}

/// Test writing contract: $1 - $4 per test suite.
fn coding_test_writing_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-coding-tests-v1".into(),
        task_type: TaskType::CodeReview,
        name: "Test Writing (Fleet)".into(),
        price_floor_micro_credits: 1_000_000,   // $1.00
        price_ceiling_micro_credits: 4_000_000, // $4.00
        success_criteria: vec![
            SuccessCriterion::TestsPassed {
                scope: "unit".into(),
            },
            SuccessCriterion::Custom {
                description: "New tests cover the specified functions".into(),
            },
        ],
        refund_policy: RefundPolicy {
            sla_seconds: 3600,
            ..RefundPolicy::default()
        },
        min_trust_score: 0.3,
        custom_label: Some("test_writing".into()),
        created_at: chrono::Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Data processing contracts
// ---------------------------------------------------------------------------

/// Data pipeline contract: $5 - $20 per pipeline run.
fn data_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-data-pipeline-v1".into(),
        task_type: TaskType::DataPipeline,
        name: "Data Pipeline (Fleet)".into(),
        price_floor_micro_credits: 5_000_000,    // $5.00
        price_ceiling_micro_credits: 20_000_000, // $20.00
        success_criteria: vec![
            SuccessCriterion::DataValidated {
                rule_id: "pipeline-output-schema".into(),
            },
            SuccessCriterion::Custom {
                description: "Output row count matches expected range".into(),
            },
        ],
        refund_policy: RefundPolicy {
            sla_seconds: 3600, // 1 hour
            ..RefundPolicy::default()
        },
        min_trust_score: 0.5,
        custom_label: None,
        created_at: chrono::Utc::now(),
    }
}

/// Report generation contract: $2 - $10 per report.
fn data_report_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-data-report-v1".into(),
        task_type: TaskType::DocumentGeneration,
        name: "Report Generation (Fleet)".into(),
        price_floor_micro_credits: 2_000_000,    // $2.00
        price_ceiling_micro_credits: 10_000_000, // $10.00
        success_criteria: vec![SuccessCriterion::DataValidated {
            rule_id: "document-schema".into(),
        }],
        refund_policy: RefundPolicy {
            sla_seconds: 1800, // 30 minutes
            ..RefundPolicy::default()
        },
        min_trust_score: 0.3,
        custom_label: None,
        created_at: chrono::Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Support contracts
// ---------------------------------------------------------------------------

/// Support ticket contract: $0.50 - $2.00 per ticket.
fn support_contract() -> TaskContract {
    TaskContract {
        contract_id: "fleet-support-ticket-v1".into(),
        task_type: TaskType::SupportTicket,
        name: "Support Ticket (Fleet)".into(),
        price_floor_micro_credits: 500_000,     // $0.50
        price_ceiling_micro_credits: 2_000_000, // $2.00
        success_criteria: vec![SuccessCriterion::Custom {
            description: "Customer confirmed resolution or ticket closed".into(),
        }],
        refund_policy: RefundPolicy {
            sla_seconds: 1800, // 30 minutes
            ..RefundPolicy::default()
        },
        min_trust_score: 0.3,
        custom_label: None,
        created_at: chrono::Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Pricing utilities
// ---------------------------------------------------------------------------

/// Estimate the price for a task given vertical, complexity, and trust score.
pub fn estimate_price(
    vertical: AgentVertical,
    complexity: TaskComplexity,
    trust_score: f64,
) -> i64 {
    contract_for(vertical).resolve_price(complexity, trust_score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_contracts_have_valid_ranges() {
        for vertical in [
            AgentVertical::Coding,
            AgentVertical::DataProcessing,
            AgentVertical::Support,
        ] {
            for contract in all_contracts_for(vertical) {
                assert!(
                    contract.price_floor_micro_credits > 0,
                    "{} has zero floor",
                    contract.contract_id
                );
                assert!(
                    contract.price_floor_micro_credits <= contract.price_ceiling_micro_credits,
                    "{} has floor > ceiling",
                    contract.contract_id
                );
                assert!(
                    !contract.success_criteria.is_empty(),
                    "{} has no criteria",
                    contract.contract_id
                );
            }
        }
    }

    #[test]
    fn coding_has_three_contracts() {
        assert_eq!(all_contracts_for(AgentVertical::Coding).len(), 3);
    }

    #[test]
    fn data_has_two_contracts() {
        assert_eq!(all_contracts_for(AgentVertical::DataProcessing).len(), 2);
    }

    #[test]
    fn support_has_one_contract() {
        assert_eq!(all_contracts_for(AgentVertical::Support).len(), 1);
    }

    #[test]
    fn estimate_price_within_range() {
        let price = estimate_price(AgentVertical::Coding, TaskComplexity::Standard, 0.5);
        assert!(price >= 2_000_000);
        assert!(price <= 5_000_000);
    }

    #[test]
    fn support_cheapest_vertical() {
        let support = estimate_price(AgentVertical::Support, TaskComplexity::Simple, 0.0);
        let coding = estimate_price(AgentVertical::Coding, TaskComplexity::Simple, 0.0);
        assert!(support < coding);
    }
}
