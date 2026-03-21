//! Nous tool observer — bridges Nous eval into the ArcanHarnessAdapter observer pattern.
//!
//! Runs Nous heuristic evaluators after each tool execution and logs scores via tracing.

use std::sync::Mutex;

use arcan_aios_adapters::tools::ToolHarnessObserver;
use async_trait::async_trait;
use nous_core::{EvalContext, EvalHook, EvalScore, EvaluatorRegistry};
use tracing::debug;

/// Observer that runs Nous evaluators after tool execution.
pub struct NousToolObserver {
    registry: EvaluatorRegistry,
    scores: Mutex<Vec<EvalScore>>,
}

impl NousToolObserver {
    pub fn new(registry: EvaluatorRegistry) -> Self {
        Self {
            registry,
            scores: Mutex::new(Vec::new()),
        }
    }

    /// Get accumulated scores (for testing/debugging).
    #[allow(dead_code)]
    pub fn scores(&self) -> Vec<EvalScore> {
        self.scores.lock().expect("lock poisoned").clone()
    }
}

#[async_trait]
impl ToolHarnessObserver for NousToolObserver {
    async fn post_execute(&self, session_id: String, tool_name: String) {
        let mut ctx = EvalContext::new(&session_id);
        ctx.tool_name = Some(tool_name);
        // Post-tool-call: run safety compliance and any PostToolCall evaluators.
        for evaluator in self.registry.evaluators_for(EvalHook::PostToolCall) {
            match evaluator.evaluate(&ctx) {
                Ok(scores) => {
                    for score in &scores {
                        debug!(
                            evaluator = score.evaluator,
                            value = score.value,
                            label = score.label.as_str(),
                            layer = %score.layer,
                            session_id = %session_id,
                            "nous eval score"
                        );
                    }
                    if let Ok(mut acc) = self.scores.lock() {
                        acc.extend(scores);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        evaluator = evaluator.name(),
                        error = %e,
                        "nous evaluator failed"
                    );
                }
            }
        }
    }
}
