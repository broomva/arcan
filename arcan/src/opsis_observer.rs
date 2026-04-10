//! Opsis world state observer — forwards tool completions and run finishes
//! to the Opsis world state engine as ambient agent observations.

use std::sync::Arc;

use arcan_aios_adapters::tools::{RunCompletionContext, ToolHarnessObserver};
use arcan_opsis::OpsisClient;
use async_trait::async_trait;

/// Observer that pushes agent activity to Opsis as ambient world state events.
pub struct OpsisToolObserver {
    client: Arc<OpsisClient>,
}

impl OpsisToolObserver {
    pub fn new(client: Arc<OpsisClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ToolHarnessObserver for OpsisToolObserver {
    async fn post_execute(&self, session_id: String, tool_name: String, is_error: bool) {
        let insight = if is_error {
            format!("tool failed: {tool_name}")
        } else {
            format!("tool completed: {tool_name}")
        };

        let confidence = if is_error { 0.3 } else { 0.6 };
        let client = self.client.clone();

        // Fire-and-forget — don't block the tool pipeline.
        tokio::spawn(async move {
            if let Err(e) = client
                .observe(
                    insight,
                    confidence,
                    opsis_core::state::StateDomain::Technology,
                    None,
                )
                .await
            {
                tracing::debug!(
                    error = %e,
                    session = %session_id,
                    "opsis observer: failed to push tool event"
                );
            }
        });
    }

    async fn on_run_finished(&self, session_id: String, context: RunCompletionContext) {
        let insight = match &context.final_answer {
            Some(answer) => {
                let truncated = if answer.len() > 200 {
                    format!("{}...", &answer[..200])
                } else {
                    answer.clone()
                };
                format!("run completed: {truncated}")
            }
            None => "run completed (no answer)".into(),
        };

        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(e) = client
                .observe(
                    insight,
                    0.7,
                    opsis_core::state::StateDomain::Technology,
                    None,
                )
                .await
            {
                tracing::debug!(
                    error = %e,
                    session = %session_id,
                    "opsis observer: failed to push run completion"
                );
            }
        });
    }
}
