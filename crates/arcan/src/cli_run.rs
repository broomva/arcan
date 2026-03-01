//! Non-interactive CLI runner for Arcan.
//!
//! Sends a single message to the daemon, prints the response to stdout,
//! and exits with code 0 on success or 1 on error.

use std::io::Write;

use reqwest::Client;
use serde::Deserialize;

/// SSE event parsed from the stream.
#[cfg(test)]
pub(crate) struct SseEvent {
    pub(crate) data: String,
}

/// Subset of EventRecord fields we care about for CLI output.
#[derive(Debug, Deserialize)]
struct EventRecord {
    kind: EventKind,
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
/// The run endpoint is synchronous (returns after the tick completes),
/// so we fetch events via `GET /events` after the run finishes rather than
/// streaming an SSE connection that would never close.
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

    // POST the run — this blocks until the tick completes.
    let run_response = client
        .post(format!("{base_url}/sessions/{session_id}/runs"))
        .json(&serde_json::json!({ "objective": message }))
        .send()
        .await?;

    if !run_response.status().is_success() {
        let status = run_response.status();
        let body = run_response.text().await.unwrap_or_default();
        anyhow::bail!("Run request failed ({status}): {body}");
    }

    // Parse run response to get the event range for this run only.
    let run_result: serde_json::Value = run_response.json().await?;
    let last_sequence = run_result
        .get("last_sequence")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let events_emitted = run_result
        .get("events_emitted")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let from_sequence = last_sequence
        .saturating_sub(events_emitted)
        .saturating_add(1);

    // Fetch only events from this run (not previous runs).
    let events_response = client
        .get(format!(
            "{base_url}/sessions/{session_id}/events?from_sequence={from_sequence}&limit=10000"
        ))
        .send()
        .await?;

    if !events_response.status().is_success() {
        let status = events_response.status();
        let body = events_response.text().await.unwrap_or_default();
        anyhow::bail!("Events request failed ({status}): {body}");
    }

    let body = events_response.text().await?;

    if json_output {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{body}");
        return Ok(0);
    }

    // Parse the events list response.
    let events_list: serde_json::Value = serde_json::from_str(&body)?;
    let Some(events) = events_list.get("events").and_then(|v| v.as_array()) else {
        anyhow::bail!("Unexpected events response format");
    };

    let mut exit_code = 0;
    let mut stdout = std::io::stdout().lock();

    for event_value in events {
        let Ok(record) = serde_json::from_value::<EventRecord>(event_value.clone()) else {
            continue;
        };

        match record.kind {
            EventKind::TextDelta { ref delta } | EventKind::AssistantTextDelta { ref delta } => {
                let _ = write!(stdout, "{delta}");
                let _ = stdout.flush();
            }
            EventKind::Message { ref content } => {
                let _ = write!(stdout, "{content}");
                let _ = stdout.flush();
            }
            EventKind::ToolCallRequested { ref tool_name } => {
                let _ = writeln!(stdout, "\n[tool: {tool_name}] requested");
            }
            EventKind::ToolCallStarted { ref tool_name } => {
                let _ = writeln!(stdout, "[tool: {tool_name}] started");
            }
            EventKind::ToolCallCompleted { ref tool_name } => {
                let _ = writeln!(stdout, "[tool: {tool_name}] OK");
            }
            EventKind::ToolCallFailed {
                ref tool_name,
                ref error,
            } => {
                let _ = writeln!(stdout, "[tool: {tool_name}] ERROR: {error}");
            }
            EventKind::RunFinished {} => {
                let _ = writeln!(stdout);
                break;
            }
            EventKind::RunErrored { ref error } => {
                let _ = writeln!(stdout, "\nERROR: {error}");
                exit_code = 1;
                break;
            }
            EventKind::Other => {}
        }
    }

    Ok(exit_code)
}

/// Parse SSE events from raw text body.
#[cfg(test)]
pub(crate) fn parse_sse_events(body: &str) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut current_data = String::new();

    for line in body.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim_start();
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(data);
        } else if line.is_empty() && !current_data.is_empty() {
            events.push(SseEvent {
                data: std::mem::take(&mut current_data),
            });
        }
    }

    // Handle trailing event without final newline.
    if !current_data.is_empty() {
        events.push(SseEvent { data: current_data });
    }

    events
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
