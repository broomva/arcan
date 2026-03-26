//! Remote Lago journal — implements `lago_core::Journal` over the Lago HTTP API.
//!
//! Used by arcan when `LAGO_URL` is set so that all events are stored in the
//! remote Lago daemon instead of a local `journal.redb` file.  This makes
//! session state durable across arcan redeploys without needing a Railway
//! persistent volume on the arcan service.
//!
//! # Endpoints used
//!
//! | Method | Path | Purpose |
//! |--------|------|---------|
//! | `PUT`  | `/v1/sessions/{id}` | Upsert a session |
//! | `GET`  | `/v1/sessions/{id}` | Get a session |
//! | `GET`  | `/v1/sessions` | List sessions |
//! | `POST` | `/v1/sessions/{id}/events` | Append one event |
//! | `GET`  | `/v1/sessions/{id}/events/read` | Batch-read events |
//! | `GET`  | `/v1/sessions/{id}/events/head` | Head sequence |
//! | `GET`  | `/v1/sessions/{id}/events` | Tail SSE stream (format=lago) |

use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use lago_core::error::{LagoError, LagoResult};
use lago_core::event::EventEnvelope;
use lago_core::id::{BranchId, EventId, SeqNo, SessionId};
use lago_core::journal::{EventQuery, EventStream, Journal};
use lago_core::session::Session;
use reqwest::Client;
use reqwest_eventsource::{Event, EventSource};
use serde::Deserialize;
use tracing::warn;

// ─── Helper types (mirror Lago API response shapes) ──────────────────────────

#[derive(Deserialize)]
struct AppendEventResponse {
    seq: SeqNo,
}

#[derive(Deserialize)]
struct HeadSeqResponse {
    seq: SeqNo,
}

// ─── RemoteLagoJournal ───────────────────────────────────────────────────────

/// A `Journal` implementation that proxies all operations to a remote Lago
/// daemon via its HTTP API.
#[derive(Clone)]
pub struct RemoteLagoJournal {
    client: Arc<Client>,
    base_url: String,
}

impl RemoteLagoJournal {
    /// Create a new remote journal pointing at `base_url`
    /// (e.g. `http://lagod.railway.internal:3001`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: Arc::new(Client::new()),
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v1{}", self.base_url, path)
    }

    async fn req_err(resp: reqwest::Response) -> LagoError {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        LagoError::Journal(format!("lago HTTP {status}: {body}"))
    }
}

// ─── Journal impl ────────────────────────────────────────────────────────────

type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

impl Journal for RemoteLagoJournal {
    // ── append ───────────────────────────────────────────────────────────────

    fn append(&self, event: EventEnvelope) -> BoxFuture<'_, LagoResult<SeqNo>> {
        Box::pin(async move {
            let url = self.url(&format!("/sessions/{}/events", event.session_id));
            // Wrap in the shape the API expects: { "event": <envelope> }
            let body = serde_json::json!({ "event": event });
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(Self::req_err(resp).await);
            }
            let ar: AppendEventResponse = resp
                .json()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;
            Ok(ar.seq)
        })
    }

    // ── append_batch ─────────────────────────────────────────────────────────

    fn append_batch(&self, events: Vec<EventEnvelope>) -> BoxFuture<'_, LagoResult<SeqNo>> {
        Box::pin(async move {
            let mut last_seq = 0u64;
            for event in events {
                last_seq = self.append(event).await?;
            }
            Ok(last_seq)
        })
    }

    // ── read ─────────────────────────────────────────────────────────────────

    fn read(&self, query: EventQuery) -> BoxFuture<'_, LagoResult<Vec<EventEnvelope>>> {
        Box::pin(async move {
            let session_id = query
                .session_id
                .as_ref()
                .ok_or_else(|| LagoError::InvalidArgument("session_id required for read".into()))?;
            let branch = query
                .branch_id
                .as_ref()
                .map(|b| b.to_string())
                .unwrap_or_else(|| "main".to_string());
            let after_seq = query.after_seq.unwrap_or(0);

            let mut url = self.url(&format!("/sessions/{session_id}/events/read"));
            url.push_str(&format!("?branch={branch}&after_seq={after_seq}"));
            if let Some(limit) = query.limit {
                url.push_str(&format!("&limit={limit}"));
            }

            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(Self::req_err(resp).await);
            }
            resp.json::<Vec<EventEnvelope>>()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))
        })
    }

    // ── get_event ────────────────────────────────────────────────────────────
    //
    // Lago has no dedicated GET /events/{id} endpoint.  Return None — callers
    // that need this should use `read` with a seq range.

    fn get_event(&self, _event_id: &EventId) -> BoxFuture<'_, LagoResult<Option<EventEnvelope>>> {
        Box::pin(async { Ok(None) })
    }

    // ── head_seq ─────────────────────────────────────────────────────────────

    fn head_seq(
        &self,
        session_id: &SessionId,
        branch_id: &BranchId,
    ) -> BoxFuture<'_, LagoResult<SeqNo>> {
        let url = self.url(&format!(
            "/sessions/{session_id}/events/head?branch={branch_id}"
        ));
        Box::pin(async move {
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(0);
            }
            if !resp.status().is_success() {
                return Err(Self::req_err(resp).await);
            }
            let hr: HeadSeqResponse = resp
                .json()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;
            Ok(hr.seq)
        })
    }

    // ── stream ───────────────────────────────────────────────────────────────
    //
    // Connects to the Lago SSE endpoint with `format=lago` which emits raw
    // `EventEnvelope` JSON in each `event:` frame.  The stream tails
    // indefinitely (Lago sends keep-alive pings every 15 s).

    fn stream(
        &self,
        session_id: SessionId,
        branch_id: BranchId,
        after_seq: SeqNo,
    ) -> BoxFuture<'_, LagoResult<EventStream>> {
        let url = self.url(&format!(
            "/sessions/{session_id}/events?format=lago&after_seq={after_seq}&branch={branch_id}"
        ));
        let rb = self.client.get(&url);

        Box::pin(async move {
            let es = EventSource::new(rb)
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            let stream = es.filter_map(|item| async move {
                match item {
                    Ok(Event::Message(msg)) if msg.event == "event" => {
                        Some(
                            serde_json::from_str::<EventEnvelope>(&msg.data)
                                .map_err(LagoError::from),
                        )
                    }
                    Ok(Event::Message(msg)) if msg.event == "done" => None,
                    Ok(_) => None,
                    Err(e) => {
                        warn!(error = %e, "lago SSE stream error");
                        Some(Err(LagoError::Journal(e.to_string())))
                    }
                }
            });

            Ok(Box::pin(stream) as EventStream)
        })
    }

    // ── put_session ──────────────────────────────────────────────────────────

    fn put_session(&self, session: Session) -> BoxFuture<'_, LagoResult<()>> {
        let url = self.url(&format!("/sessions/{}", session.session_id));
        Box::pin(async move {
            let resp = self
                .client
                .put(&url)
                .json(&session)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
                return Ok(());
            }
            Err(Self::req_err(resp).await)
        })
    }

    // ── get_session ──────────────────────────────────────────────────────────

    fn get_session(&self, session_id: &SessionId) -> BoxFuture<'_, LagoResult<Option<Session>>> {
        let url = self.url(&format!("/sessions/{session_id}"));
        Box::pin(async move {
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if !resp.status().is_success() {
                return Err(Self::req_err(resp).await);
            }
            // Lago returns SessionResponse (subset of Session).  We reconstruct
            // a Session from the fields that are available.
            let raw: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;
            let session: Session =
                serde_json::from_value(raw).map_err(LagoError::from)?;
            Ok(Some(session))
        })
    }

    // ── list_sessions ────────────────────────────────────────────────────────

    fn list_sessions(&self) -> BoxFuture<'_, LagoResult<Vec<Session>>> {
        let url = self.url("/sessions");
        Box::pin(async move {
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(Self::req_err(resp).await);
            }
            let raw: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| LagoError::Journal(e.to_string()))?;
            let sessions: Vec<Session> =
                serde_json::from_value(raw).map_err(LagoError::from)?;
            Ok(sessions)
        })
    }
}
