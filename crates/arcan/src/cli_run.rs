//! Non-interactive CLI runner for Arcan.
//!
//! Opens an SSE stream for real-time events, then POST /runs concurrently.
//! Text deltas, tool lifecycle, and errors render to stdout as they arrive.

use std::io::Write;

use futures_util::StreamExt;
use reqwest::Client;
use reqwest_eventsource::{Event as SseEvent, EventSource};
use serde::Deserialize;

/// Subset of EventRecord fields we care about for CLI output.
#[derive(Debug, Deserialize)]
struct EventRecord {
    kind: EventKind,
    #[serde(default)]
    sequence: u64,
}

/// Subset of EventKind variants for display purposes.
/// Uses the same `#[serde(tag = "type")]` format as the canonical EventKind.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum EventKind {
    TextDelta {
        delta: String,
    },
    AssistantTextDelta {
        delta: String,
    },
    Message {
        content: String,
    },
    ToolCallRequested {
        tool_name: String,
    },
    ToolCallStarted {
        tool_name: String,
    },
    ToolCallCompleted {
        tool_name: String,
    },
    ToolCallFailed {
        tool_name: String,
        error: String,
    },
    RunFinished {},
    RunErrored {
        error: String,
    },
    #[serde(other)]
    Other,
}

/// Run a single message against the daemon and print the response to stdout.
///
/// Opens an SSE stream at `/events/stream` before firing `/runs`, so events
/// (including ephemeral streaming text deltas) render in real time.
pub async fn run_cli(
    base_url: &str,
    session_id: &str,
    message: &str,
    json_output: bool,
) -> anyhow::Result<i32> {
    let client = Client::new();

    // Ensure session exists (creates if needed via POST /sessions).
    client
        .post(format!("{base_url}/sessions"))
        .json(&serde_json::json!({ "session_id": session_id }))
        .send()
        .await?;

    // Get the current event cursor so we only see new events.
    let events_response = client
        .get(format!(
            "{base_url}/sessions/{session_id}/events?limit=10000"
        ))
        .send()
        .await?;
    let cursor = if events_response.status().is_success() {
        let body: serde_json::Value = events_response.json().await?;
        body.get("events")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.last())
            .and_then(|e| e.get("sequence"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    } else {
        0
    };

    // Open SSE stream before firing the run.
    let stream_url =
        format!("{base_url}/sessions/{session_id}/events/stream?cursor={cursor}&format=canonical");
    let mut event_source = EventSource::get(&stream_url);

    // Fire the run concurrently (don't await completion before reading SSE).
    let run_url = format!("{base_url}/sessions/{session_id}/runs");
    let run_body = serde_json::json!({ "objective": message });
    let run_client = client.clone();
    let run_handle = tokio::spawn(async move {
        let result = run_client.post(&run_url).json(&run_body).send().await;
        match result {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                Err(anyhow::anyhow!("Run request failed ({status}): {body}"))
            }
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Run request failed: {e}")),
        }
    });

    let mut exit_code = 0;
    let mut stdout = std::io::stdout().lock();
    // Track whether we've received ephemeral streaming text (sequence 0).
    // If so, we skip persisted text events to avoid duplicate output.
    let mut saw_streaming_text = false;

    // Read SSE events as they arrive.
    while let Some(event) = event_source.next().await {
        match event {
            Ok(SseEvent::Open) => {}
            Ok(SseEvent::Message(msg)) => {
                if json_output {
                    let _ = writeln!(stdout, "{}", msg.data);
                    let _ = stdout.flush();
                }

                let Ok(record) = serde_json::from_str::<EventRecord>(&msg.data) else {
                    continue;
                };

                if !json_output {
                    match &record.kind {
                        EventKind::TextDelta { delta }
                        | EventKind::AssistantTextDelta { delta } => {
                            if record.sequence == 0 {
                                // Ephemeral streaming delta — display immediately.
                                saw_streaming_text = true;
                                let _ = write!(stdout, "{delta}");
                                let _ = stdout.flush();
                            } else if !saw_streaming_text {
                                // Persisted delta, but only show if we didn't
                                // already see streaming text.
                                let _ = write!(stdout, "{delta}");
                                let _ = stdout.flush();
                            }
                            // else: skip persisted duplicate
                        }
                        EventKind::Message { content } => {
                            if !saw_streaming_text {
                                let _ = write!(stdout, "{content}");
                                let _ = stdout.flush();
                            }
                        }
                        EventKind::ToolCallRequested { tool_name } => {
                            let _ = writeln!(stdout, "\n[tool: {tool_name}] requested");
                        }
                        EventKind::ToolCallStarted { tool_name } => {
                            let _ = writeln!(stdout, "[tool: {tool_name}] started");
                        }
                        EventKind::ToolCallCompleted { tool_name } => {
                            let _ = writeln!(stdout, "[tool: {tool_name}] OK");
                        }
                        EventKind::ToolCallFailed { tool_name, error } => {
                            let _ = writeln!(stdout, "[tool: {tool_name}] ERROR: {error}");
                        }
                        EventKind::RunFinished {} => {
                            let _ = writeln!(stdout);
                            event_source.close();
                            break;
                        }
                        EventKind::RunErrored { error } => {
                            let _ = writeln!(stdout, "\nERROR: {error}");
                            exit_code = 1;
                            event_source.close();
                            break;
                        }
                        EventKind::Other => {}
                    }
                }

                // In JSON mode, also detect run termination events.
                if json_output {
                    match &record.kind {
                        EventKind::RunFinished {} => {
                            event_source.close();
                            break;
                        }
                        EventKind::RunErrored { .. } => {
                            exit_code = 1;
                            event_source.close();
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {
                // SSE connection closed or errored — stop reading.
                break;
            }
        }
    }

    // Ensure the run request completed (surface errors).
    if let Err(e) = run_handle.await? {
        // Only report run errors if we didn't already get a RunErrored event.
        if exit_code == 0 {
            let _ = writeln!(stdout, "\nERROR: {e}");
            exit_code = 1;
        }
    }

    Ok(exit_code)
}

/// Try to resolve a session via the daemon's HTTP API.
pub async fn resolve_session_via_api(base_url: &str) -> Option<String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let response = client
        .get(format!("{base_url}/sessions"))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let sessions: Vec<serde_json::Value> = response.json().await.ok()?;
    sessions
        .first()
        .and_then(|s| s.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from)
}
