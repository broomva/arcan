//! Nous tool observer — bridges Nous eval into the ArcanHarnessAdapter observer pattern.
//!
//! Runs Nous heuristic evaluators after each tool execution and logs scores via tracing.
//! Optionally persists eval scores to the Lago journal via `LivePublisher`.
//!
//! When an async judge provider is configured, `on_run_finished` runs LLM-as-judge
//! evaluators (PlanQuality, TaskCompletion, PlanAdherence) in a background task,
//! then publishes the EGRI outcome event.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use arcan_aios_adapters::tools::ToolHarnessObserver;
use async_trait::async_trait;
use lago_core::Journal;
use nous_api::ScoreStore;
use nous_core::events::NousEvent;
use nous_core::score::EvalResult;
use nous_core::{EvalContext, EvalHook, EvalScore, EvaluatorRegistry, NousEvaluator};
use nous_judge::JudgeProvider;
use nous_lago::LivePublisher;
use tracing::debug;

/// Observer that runs Nous evaluators after tool execution.
pub struct NousToolObserver {
    registry: EvaluatorRegistry,
    scores: Mutex<Vec<EvalScore>>,
    publisher: Option<LivePublisher>,
    score_store: Option<ScoreStore>,
    /// Optional async judge provider for LLM-as-judge evaluation on run completion.
    judge_provider: Option<Arc<dyn JudgeProvider>>,
}

impl NousToolObserver {
    #[allow(dead_code)]
    pub fn new(registry: EvaluatorRegistry) -> Self {
        Self {
            registry,
            scores: Mutex::new(Vec::new()),
            publisher: None,
            score_store: None,
            judge_provider: None,
        }
    }

    /// Create a new observer with Lago journal persistence via `LivePublisher`.
    #[allow(dead_code)]
    pub fn with_journal(
        registry: EvaluatorRegistry,
        journal: Arc<dyn Journal>,
        session_id: &str,
        agent_id: &str,
    ) -> Self {
        Self {
            registry,
            scores: Mutex::new(Vec::new()),
            publisher: Some(LivePublisher::new(journal, session_id, agent_id)),
            score_store: None,
            judge_provider: None,
        }
    }

    /// Create a fully-configured observer with journal, score store, and judge provider.
    pub fn with_full_config(
        registry: EvaluatorRegistry,
        journal: Arc<dyn Journal>,
        session_id: &str,
        agent_id: &str,
        judge_provider: Option<Arc<dyn JudgeProvider>>,
    ) -> Self {
        Self {
            registry,
            scores: Mutex::new(Vec::new()),
            publisher: Some(LivePublisher::new(journal, session_id, agent_id)),
            score_store: None,
            judge_provider,
        }
    }

    /// Attach a `ScoreStore` so scores are also recorded for the HTTP eval API.
    pub fn with_score_store(mut self, store: ScoreStore) -> Self {
        self.score_store = Some(store);
        self
    }

    /// Get the attached score store, if any.
    #[allow(dead_code)]
    pub fn score_store(&self) -> Option<&ScoreStore> {
        self.score_store.as_ref()
    }

    /// Get accumulated scores (for testing/debugging).
    #[allow(dead_code)]
    pub fn scores(&self) -> Vec<EvalScore> {
        self.scores.lock().expect("lock poisoned").clone()
    }

    /// Whether async judges are available.
    #[allow(dead_code)]
    pub fn has_judge_provider(&self) -> bool {
        self.judge_provider.is_some()
    }
}

#[async_trait]
impl ToolHarnessObserver for NousToolObserver {
    async fn post_execute(&self, session_id: String, tool_name: String, is_error: bool) {
        let mut ctx = EvalContext::new(&session_id);
        ctx.tool_name = Some(tool_name);
        ctx.tool_errored = Some(is_error);
        // Post-tool-call: run safety compliance and any PostToolCall evaluators.
        for evaluator in self.registry.evaluators_for(EvalHook::PostToolCall) {
            match evaluator.evaluate(&ctx) {
                Ok(scores) => {
                    let metrics = life_vigil::GenAiMetrics::new("arcan");
                    for score in &scores {
                        debug!(
                            evaluator = score.evaluator,
                            value = score.value,
                            label = score.label.as_str(),
                            layer = %score.layer,
                            session_id = %session_id,
                            "nous eval score"
                        );
                        // Emit OTel span event
                        life_vigil::spans::eval_event(
                            &score.evaluator,
                            score.value,
                            score.label.as_str(),
                            score.layer.label(),
                            match score.timing {
                                nous_core::EvalTiming::Inline => "inline",
                                nous_core::EvalTiming::Async => "async",
                            },
                        );
                        // Record eval metric
                        metrics.record_eval_execution(
                            &score.evaluator,
                            score.layer.label(),
                            score.value,
                        );

                        // Persist to Lago journal (fire-and-forget)
                        if let Some(ref publisher) = self.publisher
                            && let Err(e) = publisher.publish_score(score).await
                        {
                            tracing::warn!(
                                evaluator = score.evaluator,
                                error = %e,
                                "failed to publish eval score to Lago (non-fatal)"
                            );
                        }

                        // Record to in-memory ScoreStore (serves /nous HTTP eval API)
                        if let Some(ref store) = self.score_store {
                            store.record(score.clone());
                        }
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

    async fn on_run_finished(
        &self,
        session_id: String,
        objective: Option<String>,
        final_answer: Option<String>,
        assistant_messages: Option<String>,
    ) {
        let Some(ref provider) = self.judge_provider else {
            return;
        };

        tracing::info!(
            session_id = %session_id,
            "running async judge evaluators on run completion"
        );

        let start = Instant::now();

        // Build evaluation context with run metadata.
        let mut ctx = EvalContext::new(&session_id);
        if let Some(ref obj) = objective {
            ctx.metadata.insert("objective".into(), obj.clone());
        }
        if let Some(ref ans) = final_answer {
            ctx.metadata.insert("final_answer".into(), ans.clone());
        }
        if let Some(ref msgs) = assistant_messages {
            ctx.metadata
                .insert("assistant_messages".into(), msgs.clone());
        }

        // Create judge evaluators — each is moved into a spawn_blocking task.
        let judge_specs: Vec<(&str, Box<dyn NousEvaluator>)> = vec![
            (
                "plan_quality",
                Box::new(nous_judge::PlanQuality::new(provider.clone())),
            ),
            (
                "task_completion",
                Box::new(nous_judge::TaskCompletion::new(provider.clone())),
            ),
            (
                "plan_adherence",
                Box::new(nous_judge::PlanAdherence::new(provider.clone())),
            ),
        ];

        let mut all_scores: Vec<EvalScore> = Vec::new();
        let metrics = life_vigil::GenAiMetrics::new("arcan");

        for (eval_name, eval) in judge_specs {
            let eval_ctx = ctx.clone();
            let eval_name = eval_name.to_owned();

            // JudgeProvider::judge() is blocking (HTTP) — run in spawn_blocking.
            let result = tokio::task::spawn_blocking(move || eval.evaluate(&eval_ctx)).await;

            match result {
                Ok(Ok(scores)) => {
                    for score in &scores {
                        debug!(
                            evaluator = score.evaluator,
                            value = score.value,
                            label = score.label.as_str(),
                            layer = %score.layer,
                            session_id = %session_id,
                            timing = "async",
                            "async judge eval score"
                        );

                        // Emit OTel span event
                        life_vigil::spans::eval_event(
                            &score.evaluator,
                            score.value,
                            score.label.as_str(),
                            score.layer.label(),
                            "async",
                        );

                        // Record eval metric
                        metrics.record_eval_execution(
                            &score.evaluator,
                            score.layer.label(),
                            score.value,
                        );

                        // Persist to Lago journal
                        if let Some(ref publisher) = self.publisher
                            && let Err(e) = publisher.publish_score(score).await
                        {
                            tracing::warn!(
                                evaluator = score.evaluator,
                                error = %e,
                                "failed to publish async judge score to Lago (non-fatal)"
                            );
                        }

                        // Record to in-memory ScoreStore
                        if let Some(ref store) = self.score_store {
                            store.record(score.clone());
                        }
                    }
                    all_scores.extend(scores);
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        evaluator = %eval_name,
                        error = %e,
                        "async judge evaluator failed (non-fatal)"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        evaluator = %eval_name,
                        error = %e,
                        "async judge task panicked (non-fatal)"
                    );
                }
            }
        }

        let duration = start.elapsed();

        // Build EvalResult and publish EGRI outcome event.
        if !all_scores.is_empty() {
            let eval_result = EvalResult {
                evaluator: "async_judge".into(),
                scores: all_scores.clone(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                duration_ms: duration.as_millis() as u64,
            };

            // Convert to EGRI outcome and publish.
            let outcome =
                nous_core::egri::eval_result_to_trial_event(&eval_result, &session_id, None);

            let egri_event = NousEvent::EgriOutcome {
                session_id: session_id.clone(),
                trial_id: None,
                outcome,
            };

            if let Some(ref publisher) = self.publisher
                && let Err(e) = publisher.publish_event(egri_event).await
            {
                tracing::warn!(
                    error = %e,
                    "failed to publish EGRI outcome to Lago (non-fatal)"
                );
            }

            tracing::info!(
                session_id = %session_id,
                score_count = all_scores.len(),
                aggregate = eval_result.aggregate_score(),
                duration_ms = duration.as_millis(),
                "async judge evaluation complete — EGRI outcome published"
            );

            // Accumulate into observer scores.
            if let Ok(mut acc) = self.scores.lock() {
                acc.extend(all_scores);
            }
        } else {
            tracing::debug!(
                session_id = %session_id,
                "async judge evaluation produced no scores (insufficient context)"
            );
        }
    }
}
