use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing;

/// Result of a heartbeat check.
#[derive(Debug, Clone)]
pub enum HeartbeatResult {
    /// Nothing to do — all checks passed.
    Ok,
    /// Observation threshold reached — compaction recommended.
    CompactionNeeded {
        session_id: String,
        observation_count: usize,
    },
    /// A session has been idle too long.
    SessionIdle { session_id: String, idle_secs: u64 },
}

/// A check function that runs on each heartbeat tick.
/// Returns a list of findings (empty = healthy).
pub trait HeartbeatCheck: Send + Sync {
    fn name(&self) -> &str;
    fn check(&self) -> Vec<HeartbeatResult>;
}

/// Configuration for the heartbeat scheduler.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// Interval between heartbeat ticks.
    pub interval: Duration,
    /// Whether the heartbeat scheduler is enabled.
    pub enabled: bool,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            enabled: true,
        }
    }
}

/// The heartbeat scheduler runs periodic checks in the background.
///
/// Design:
/// - Runs cheap deterministic checks first (no LLM calls)
/// - Only escalates to agent run if checks find something actionable
/// - Sends results on a channel for the server to act on
///
/// Checks include:
/// - Observation compaction thresholds
/// - Session idle timeouts
/// - Custom checks via HeartbeatCheck trait
pub struct HeartbeatScheduler {
    config: HeartbeatConfig,
    checks: Vec<Arc<dyn HeartbeatCheck>>,
}

impl HeartbeatScheduler {
    pub fn new(config: HeartbeatConfig) -> Self {
        Self {
            config,
            checks: Vec::new(),
        }
    }

    /// Register a heartbeat check.
    pub fn add_check(&mut self, check: Arc<dyn HeartbeatCheck>) {
        self.checks.push(check);
    }

    /// Start the heartbeat scheduler as a background task.
    ///
    /// Returns a receiver for heartbeat results. The scheduler runs
    /// until the returned sender is dropped (or the task is aborted).
    pub fn start(self) -> mpsc::Receiver<Vec<HeartbeatResult>> {
        let (tx, rx) = mpsc::channel(16);

        if !self.config.enabled {
            tracing::info!("Heartbeat scheduler disabled");
            return rx;
        }

        let tick_interval = self.config.interval;
        let checks = self.checks;

        tokio::spawn(async move {
            let mut ticker = interval(tick_interval);
            // Skip the immediate first tick
            ticker.tick().await;

            tracing::info!(
                interval_secs = tick_interval.as_secs(),
                check_count = checks.len(),
                "Heartbeat scheduler started"
            );

            loop {
                ticker.tick().await;

                let mut results = Vec::new();
                for check in &checks {
                    let findings = check.check();
                    if !findings.is_empty() {
                        tracing::debug!(
                            check = check.name(),
                            findings = findings.len(),
                            "Heartbeat check found issues"
                        );
                    }
                    results.extend(findings);
                }

                if results.is_empty() {
                    tracing::trace!("Heartbeat: all checks ok");
                } else {
                    tracing::info!(
                        findings = results.len(),
                        "Heartbeat: {} findings",
                        results.len()
                    );
                    if tx.send(results).await.is_err() {
                        tracing::info!("Heartbeat receiver dropped, shutting down scheduler");
                        break;
                    }
                }
            }
        });

        rx
    }
}

/// A simple observation compaction check.
///
/// Checks if any observer has accumulated observations beyond a threshold.
pub struct ObservationCompactionCheck {
    /// Function that returns (session_id, observation_count) for each active session.
    session_observations: Arc<dyn Fn() -> Vec<(String, usize)> + Send + Sync>,
    threshold: usize,
}

impl ObservationCompactionCheck {
    pub fn new(
        threshold: usize,
        session_observations: Arc<dyn Fn() -> Vec<(String, usize)> + Send + Sync>,
    ) -> Self {
        Self {
            session_observations,
            threshold,
        }
    }
}

impl HeartbeatCheck for ObservationCompactionCheck {
    fn name(&self) -> &str {
        "observation_compaction"
    }

    fn check(&self) -> Vec<HeartbeatResult> {
        let sessions = (self.session_observations)();
        sessions
            .into_iter()
            .filter(|(_, count)| *count >= self.threshold)
            .map(
                |(session_id, observation_count)| HeartbeatResult::CompactionNeeded {
                    session_id,
                    observation_count,
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_config_defaults() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.interval, Duration::from_secs(60));
        assert!(config.enabled);
    }

    #[test]
    fn observation_check_below_threshold() {
        let check = ObservationCompactionCheck::new(10, Arc::new(|| vec![("s1".to_string(), 5)]));
        let results = check.check();
        assert!(results.is_empty());
    }

    #[test]
    fn observation_check_above_threshold() {
        let check = ObservationCompactionCheck::new(
            10,
            Arc::new(|| vec![("s1".to_string(), 15), ("s2".to_string(), 3)]),
        );
        let results = check.check();
        assert_eq!(results.len(), 1);
        match &results[0] {
            HeartbeatResult::CompactionNeeded {
                session_id,
                observation_count,
            } => {
                assert_eq!(session_id, "s1");
                assert_eq!(*observation_count, 15);
            }
            other => panic!("Expected CompactionNeeded, got: {:?}", other),
        }
    }

    #[test]
    fn observation_check_multiple_sessions_above_threshold() {
        let check = ObservationCompactionCheck::new(
            5,
            Arc::new(|| {
                vec![
                    ("s1".to_string(), 10),
                    ("s2".to_string(), 7),
                    ("s3".to_string(), 2),
                ]
            }),
        );
        let results = check.check();
        assert_eq!(results.len(), 2); // s1 and s2 above threshold
    }

    #[test]
    fn scheduler_creation_with_checks() {
        let mut scheduler = HeartbeatScheduler::new(HeartbeatConfig::default());
        let check = ObservationCompactionCheck::new(10, Arc::new(|| vec![]));
        scheduler.add_check(Arc::new(check));
        // Just verifying it doesn't panic
    }

    #[tokio::test]
    async fn disabled_scheduler_returns_empty_receiver() {
        let scheduler = HeartbeatScheduler::new(HeartbeatConfig {
            interval: Duration::from_millis(10),
            enabled: false,
        });
        let mut rx = scheduler.start();

        // Should not receive anything (scheduler didn't start)
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn scheduler_fires_on_interval() {
        let mut scheduler = HeartbeatScheduler::new(HeartbeatConfig {
            interval: Duration::from_millis(50),
            enabled: true,
        });

        // Add a check that always finds something
        struct AlwaysFindsCheck;
        impl HeartbeatCheck for AlwaysFindsCheck {
            fn name(&self) -> &str {
                "always"
            }
            fn check(&self) -> Vec<HeartbeatResult> {
                vec![HeartbeatResult::Ok]
            }
        }

        scheduler.add_check(Arc::new(AlwaysFindsCheck));
        let mut rx = scheduler.start();

        // Wait for at least one tick
        let result = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await;
        assert!(
            result.is_ok(),
            "Should receive heartbeat result within timeout"
        );
        let findings = result.unwrap().unwrap();
        assert!(!findings.is_empty());
    }

    #[tokio::test]
    async fn scheduler_no_findings_doesnt_send() {
        let mut scheduler = HeartbeatScheduler::new(HeartbeatConfig {
            interval: Duration::from_millis(50),
            enabled: true,
        });

        // Check that returns empty (no findings)
        struct EmptyCheck;
        impl HeartbeatCheck for EmptyCheck {
            fn name(&self) -> &str {
                "empty"
            }
            fn check(&self) -> Vec<HeartbeatResult> {
                vec![]
            }
        }

        scheduler.add_check(Arc::new(EmptyCheck));
        let mut rx = scheduler.start();

        // Should NOT receive anything since checks found nothing
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            rx.try_recv().is_err(),
            "Should not receive when no findings"
        );
    }

    #[test]
    fn heartbeat_result_debug_format() {
        let result = HeartbeatResult::CompactionNeeded {
            session_id: "s1".to_string(),
            observation_count: 42,
        };
        let debug = format!("{result:?}");
        assert!(debug.contains("CompactionNeeded"));
        assert!(debug.contains("42"));
    }
}
