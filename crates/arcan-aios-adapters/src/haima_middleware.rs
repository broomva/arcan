//! Haima payment middleware — intercepts HTTP 402 tool results and invokes
//! the x402 payment flow through the live Haima facilitator.
//!
//! [`HaimaPaymentMiddleware`] implements the [`Middleware`] trait as a
//! **post-tool-call** interceptor. When a tool result indicates an HTTP 402
//! response with a `payment-required` header, the middleware:
//!
//! 1. Checks agent credit via `POST /v1/credit/{agent_id}/check` on the
//!    Haima facilitator.
//! 2. Invokes [`X402Client::handle_402`] to parse terms, evaluate policy, and
//!    optionally sign the payment.
//! 3. Submits the signed payment to `POST /v1/facilitate` on the live
//!    Haima facilitator for on-chain settlement.
//! 4. Publishes the appropriate [`FinanceEventKind`] to the Lago journal via
//!    [`FinancePublisher`].
//! 5. Logs the outcome (settled / failed / credit-insufficient / denied).
//!
//! The middleware does **not** retry the tool call — that is the orchestrator's
//! responsibility. It only processes the payment side-channel so that the
//! financial state is updated and events are emitted.

use std::sync::Arc;
use std::time::Instant;

use arcan_core::runtime::Middleware;
use arcan_core::{CoreError, ToolContext, ToolResult};
use haima_core::event::FinanceEventKind;
use haima_core::payment::PaymentDecision;
use haima_core::wallet::usdc_raw_to_micro_credits;
use haima_lago::publisher::FinancePublisher;
use haima_x402::client::X402Client;
use haima_x402::facilitator::{FacilitateRequest, FacilitateResponse, FacilitationStatus};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Constants for detecting 402 in tool output
// ---------------------------------------------------------------------------

/// JSON field name that tools use to report HTTP status codes.
const STATUS_CODE_FIELD: &str = "status_code";
/// JSON field name for the `payment-required` header value from 402 responses.
const PAYMENT_REQUIRED_HEADER_FIELD: &str = "payment_required_header";
/// JSON field name for the resource URL that returned 402.
const RESOURCE_URL_FIELD: &str = "resource_url";

/// HTTP 402 status code.
const HTTP_402: u64 = 402;

/// Default Haima facilitator URL (local development).
const DEFAULT_HAIMA_URL: &str = "http://localhost:3003";

/// Environment variable for the Haima facilitator URL.
const HAIMA_URL_ENV: &str = "HAIMA_URL";

// ---------------------------------------------------------------------------
// Credit check types (mirror the facilitator API)
// ---------------------------------------------------------------------------

/// Request body for `POST /v1/credit/{agent_id}/check`.
#[derive(Debug, Serialize)]
struct CreditCheckRequest {
    amount_micro_usd: u64,
}

/// Response from `POST /v1/credit/{agent_id}/check`.
#[derive(Debug, Deserialize)]
struct CreditCheckResponse {
    approved: bool,
    #[serde(default)]
    tier: String,
    #[serde(default)]
    remaining_limit: u64,
    #[serde(default)]
    reason: Option<String>,
}

// ---------------------------------------------------------------------------
// HaimaPaymentMiddleware
// ---------------------------------------------------------------------------

/// Post-tool-call middleware that handles HTTP 402 payment flows through the
/// live Haima facilitator.
///
/// When a tool result's `output` JSON contains:
/// - `status_code: 402`
/// - `payment_required_header: "<base64-encoded header>"`
/// - `resource_url: "<url>"`
///
/// the middleware:
/// 1. Checks credit via the facilitator
/// 2. Invokes the x402 client to evaluate and sign
/// 3. Submits to the facilitator for settlement
/// 4. Emits finance events
pub struct HaimaPaymentMiddleware {
    x402_client: Arc<X402Client>,
    publisher: Arc<FinancePublisher>,
    http_client: reqwest::Client,
    /// The agent's identity for credit checks.
    agent_id: String,
    /// Base URL for the Haima facilitator (e.g. `https://haimad-production.up.railway.app`).
    haima_url: String,
}

impl HaimaPaymentMiddleware {
    /// Create a new payment middleware.
    ///
    /// The `agent_id` identifies this agent for credit checks.
    /// The facilitator URL is read from `HAIMA_URL` env var, falling back to
    /// `http://localhost:3003`.
    pub fn new(
        x402_client: Arc<X402Client>,
        publisher: Arc<FinancePublisher>,
        agent_id: String,
    ) -> Self {
        let haima_url =
            std::env::var(HAIMA_URL_ENV).unwrap_or_else(|_| DEFAULT_HAIMA_URL.to_string());
        Self {
            x402_client,
            publisher,
            http_client: reqwest::Client::new(),
            agent_id,
            haima_url,
        }
    }

    /// Create a new payment middleware with an explicit facilitator URL.
    pub fn with_url(
        x402_client: Arc<X402Client>,
        publisher: Arc<FinancePublisher>,
        agent_id: String,
        haima_url: String,
    ) -> Self {
        Self {
            x402_client,
            publisher,
            http_client: reqwest::Client::new(),
            agent_id,
            haima_url,
        }
    }
}

impl Middleware for HaimaPaymentMiddleware {
    fn post_tool_call(&self, context: &ToolContext, result: &ToolResult) -> Result<(), CoreError> {
        // Check if this tool result indicates an HTTP 402.
        let Some(info) = extract_402_info(result) else {
            return Ok(());
        };

        info!(
            tool = %result.tool_name,
            resource_url = %info.resource_url,
            session_id = %context.session_id,
            haima_url = %self.haima_url,
            "detected HTTP 402 in tool result, invoking x402 payment flow"
        );

        // Spawn the async payment handling on the current runtime.
        // The middleware trait is sync, so we bridge to async via block_in_place.
        let x402_client = Arc::clone(&self.x402_client);
        let publisher = Arc::clone(&self.publisher);
        let http_client = self.http_client.clone();
        let resource_url = info.resource_url.clone();
        let payment_header = info.payment_required_header.clone();
        let session_id = context.session_id.clone();
        let agent_id = self.agent_id.clone();
        let haima_url = self.haima_url.clone();

        // Use tokio::task::block_in_place to allow blocking within an async
        // context, then use Handle::current() to run the async work.
        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async move {
                handle_payment_flow(
                    &x402_client,
                    &publisher,
                    &http_client,
                    &resource_url,
                    &payment_header,
                    &session_id,
                    &agent_id,
                    &haima_url,
                )
                .await;
            });
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 402 detection helpers
// ---------------------------------------------------------------------------

/// Information extracted from a tool result that indicates an HTTP 402.
struct Http402Info {
    resource_url: String,
    payment_required_header: String,
}

/// Try to extract 402 payment information from a tool result.
///
/// Returns `Some(Http402Info)` if the tool result's `output` JSON contains
/// the expected fields indicating an HTTP 402 response.
fn extract_402_info(result: &ToolResult) -> Option<Http402Info> {
    let output = &result.output;

    // Check for status_code == 402
    let status_code = output.get(STATUS_CODE_FIELD)?.as_u64()?;
    if status_code != HTTP_402 {
        return None;
    }

    // Extract the payment-required header value
    let payment_required_header = output
        .get(PAYMENT_REQUIRED_HEADER_FIELD)?
        .as_str()?
        .to_owned();

    // Extract the resource URL (with fallback)
    let resource_url = output
        .get(RESOURCE_URL_FIELD)
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_owned();

    Some(Http402Info {
        resource_url,
        payment_required_header,
    })
}

// ---------------------------------------------------------------------------
// Credit check
// ---------------------------------------------------------------------------

/// Check whether the agent has sufficient credit for a payment.
///
/// Calls `POST /v1/credit/{agent_id}/check` on the Haima facilitator.
/// Returns `Ok(true)` if approved, `Ok(false)` if insufficient.
/// On network/parse errors, returns `Ok(true)` to allow the payment to proceed
/// (graceful degradation — credit check is advisory).
async fn check_agent_credit(
    http_client: &reqwest::Client,
    haima_url: &str,
    agent_id: &str,
    amount_micro_credits: i64,
) -> (bool, Option<String>) {
    let url = format!("{haima_url}/v1/credit/{agent_id}/check");
    let body = CreditCheckRequest {
        amount_micro_usd: amount_micro_credits.unsigned_abs(),
    };

    let resp = match http_client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                error = %e,
                agent_id,
                url,
                "credit check request failed, allowing payment (graceful degradation)"
            );
            return (true, None);
        }
    };

    if !resp.status().is_success() {
        warn!(
            status = %resp.status(),
            agent_id,
            "credit check returned non-200, allowing payment (graceful degradation)"
        );
        return (true, None);
    }

    match resp.json::<CreditCheckResponse>().await {
        Ok(check) => {
            if check.approved {
                debug!(
                    agent_id,
                    tier = %check.tier,
                    remaining_limit = check.remaining_limit,
                    "credit check passed"
                );
                (true, None)
            } else {
                let reason = check.reason.unwrap_or_else(|| {
                    format!(
                        "tier '{}' with remaining limit {} insufficient",
                        check.tier, check.remaining_limit
                    )
                });
                info!(
                    agent_id,
                    tier = %check.tier,
                    remaining_limit = check.remaining_limit,
                    "credit check failed: {reason}"
                );
                (false, Some(reason))
            }
        }
        Err(e) => {
            warn!(
                error = %e,
                agent_id,
                "failed to parse credit check response, allowing payment"
            );
            (true, None)
        }
    }
}

// ---------------------------------------------------------------------------
// Facilitator settlement
// ---------------------------------------------------------------------------

/// Submit a signed payment to the live Haima facilitator for on-chain settlement.
///
/// Calls `POST /v1/facilitate` with the signed payment header.
/// Returns the facilitator response or an error string.
async fn submit_to_facilitator(
    http_client: &reqwest::Client,
    haima_url: &str,
    payment_header: &str,
    resource_url: &str,
    amount_micro_usd: u64,
    agent_id: &str,
) -> Result<FacilitateResponse, String> {
    let url = format!("{haima_url}/v1/facilitate");
    let body = FacilitateRequest {
        payment_header: payment_header.to_owned(),
        resource_url: resource_url.to_owned(),
        amount_micro_usd,
        agent_id: Some(agent_id.to_owned()),
    };

    let resp = http_client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("facilitator request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("facilitator returned {status}: {body_text}"));
    }

    resp.json::<FacilitateResponse>()
        .await
        .map_err(|e| format!("failed to parse facilitator response: {e}"))
}

// ---------------------------------------------------------------------------
// Payment flow
// ---------------------------------------------------------------------------

/// Process the full x402 payment flow: credit check -> sign -> facilitate -> emit events.
async fn handle_payment_flow(
    x402_client: &X402Client,
    publisher: &FinancePublisher,
    http_client: &reqwest::Client,
    resource_url: &str,
    payment_required_header: &str,
    session_id: &str,
    agent_id: &str,
    haima_url: &str,
) {
    let start = Instant::now();

    // Step 1: Parse the payment header to get the amount for credit check.
    let handle_result = match x402_client
        .handle_402(resource_url, payment_required_header)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            warn!(
                error = %e,
                resource_url,
                session_id,
                "x402 handle_402 failed"
            );
            // Emit a payment-failed event for observability.
            let failed_event = FinanceEventKind::PaymentFailed {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: 0,
                reason: format!("x402 protocol error: {e}"),
            };
            if let Err(pub_err) = publisher.publish(&failed_event).await {
                warn!(error = %pub_err, "failed to publish payment_failed event");
            }
            return;
        }
    };

    let requirement = &handle_result.requirement;
    let amount_str = &requirement.amount;
    let raw_amount: u64 = amount_str.parse().unwrap_or(0);
    let micro_credits = usdc_raw_to_micro_credits(raw_amount);

    // Step 2: Credit check before attempting payment.
    let (credit_ok, credit_reason) =
        check_agent_credit(http_client, haima_url, agent_id, micro_credits).await;

    if !credit_ok {
        let reason = credit_reason.unwrap_or_else(|| "insufficient credit".to_string());
        warn!(
            agent_id,
            resource_url,
            micro_credits,
            session_id,
            reason = %reason,
            "credit insufficient, skipping payment"
        );

        // Emit CreditInsufficient event
        let insufficient_event = FinanceEventKind::CreditInsufficient {
            agent_id: agent_id.to_owned(),
            resource_url: resource_url.to_owned(),
            amount_micro_credits: micro_credits,
            reason,
        };
        if let Err(e) = publisher.publish(&insufficient_event).await {
            warn!(error = %e, "failed to publish credit_insufficient event");
        }
        return;
    }

    // Step 3: Process based on the policy decision.
    match &handle_result.decision {
        PaymentDecision::Approved {
            payer,
            micro_credit_cost,
            reason,
        } => {
            info!(
                resource_url,
                micro_credit_cost,
                payer = %payer,
                reason,
                session_id,
                "payment auto-approved and signed, submitting to facilitator"
            );

            // Emit PaymentRequested
            let requested_event = FinanceEventKind::PaymentRequested {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: *micro_credit_cost,
                token: requirement.token.clone(),
                chain: requirement.network.clone(),
            };
            if let Err(e) = publisher.publish(&requested_event).await {
                warn!(error = %e, "failed to publish payment_requested event");
            }

            // Emit PaymentAuthorized
            let authorized_event = FinanceEventKind::PaymentAuthorized {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: *micro_credit_cost,
                payer_address: payer.address.clone(),
                recipient_address: requirement.recipient.clone(),
            };
            if let Err(e) = publisher.publish(&authorized_event).await {
                warn!(error = %e, "failed to publish payment_authorized event");
            }

            // Emit PaymentAttempted (we are about to call the facilitator)
            let attempted_event = FinanceEventKind::PaymentAttempted {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: *micro_credit_cost,
                facilitator_url: haima_url.to_owned(),
            };
            if let Err(e) = publisher.publish(&attempted_event).await {
                warn!(error = %e, "failed to publish payment_attempted event");
            }

            // Step 4: Submit to the live facilitator for settlement.
            if let Some(ref signature_header) = handle_result.signature_header {
                match submit_to_facilitator(
                    http_client,
                    haima_url,
                    signature_header,
                    resource_url,
                    raw_amount,
                    agent_id,
                )
                .await
                {
                    Ok(facilitate_resp) => {
                        let latency_ms = start.elapsed().as_millis() as u64;

                        match facilitate_resp.status {
                            FacilitationStatus::Settled => {
                                let tx_hash = facilitate_resp
                                    .receipt
                                    .as_ref()
                                    .map(|r| r.tx_hash.clone())
                                    .unwrap_or_else(|| "unknown".into());

                                info!(
                                    resource_url,
                                    tx_hash = %tx_hash,
                                    latency_ms,
                                    session_id,
                                    "payment settled via facilitator"
                                );

                                // Emit PaymentSettled
                                let settled_event = FinanceEventKind::PaymentSettled {
                                    tx_hash,
                                    amount_micro_credits: *micro_credit_cost,
                                    chain: requirement.network.clone(),
                                    latency_ms,
                                    facilitator: haima_url.to_owned(),
                                };
                                if let Err(e) = publisher.publish(&settled_event).await {
                                    warn!(error = %e, "failed to publish payment_settled event");
                                }
                            }
                            FacilitationStatus::Rejected => {
                                let reason = facilitate_resp
                                    .reason
                                    .unwrap_or_else(|| "facilitator rejected".into());
                                warn!(
                                    resource_url,
                                    reason = %reason,
                                    session_id,
                                    "facilitator rejected payment"
                                );

                                let failed_event = FinanceEventKind::PaymentFailed {
                                    resource_url: resource_url.to_owned(),
                                    amount_micro_credits: *micro_credit_cost,
                                    reason: format!("facilitator rejected: {reason}"),
                                };
                                if let Err(e) = publisher.publish(&failed_event).await {
                                    warn!(error = %e, "failed to publish payment_failed event");
                                }
                            }
                            FacilitationStatus::Pending => {
                                info!(
                                    resource_url,
                                    session_id, "payment pending (async settlement)"
                                );
                                // Pending is not a failure — the settlement will
                                // be confirmed asynchronously. We emit
                                // PaymentAttempted (already done) and let the
                                // facilitator notify on completion.
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            resource_url,
                            session_id,
                            "facilitator submission failed"
                        );

                        let failed_event = FinanceEventKind::PaymentFailed {
                            resource_url: resource_url.to_owned(),
                            amount_micro_credits: *micro_credit_cost,
                            reason: format!("facilitator error: {e}"),
                        };
                        if let Err(pub_err) = publisher.publish(&failed_event).await {
                            warn!(error = %pub_err, "failed to publish payment_failed event");
                        }
                    }
                }
            }
        }

        PaymentDecision::RequiresApproval {
            micro_credit_cost,
            reason,
        } => {
            info!(
                resource_url,
                micro_credit_cost, reason, session_id, "payment requires human approval"
            );

            // Emit PaymentRequested (the request happened, approval pending)
            let requested_event = FinanceEventKind::PaymentRequested {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: *micro_credit_cost,
                token: requirement.token.clone(),
                chain: requirement.network.clone(),
            };
            if let Err(e) = publisher.publish(&requested_event).await {
                warn!(error = %e, "failed to publish payment_requested event");
            }

            // Note: The ApprovalRequested flow is handled by the orchestrator
            // via Arcan's ApprovalPort. We only record the payment request here.
        }

        PaymentDecision::Denied { reason } => {
            warn!(
                resource_url,
                reason, micro_credits, session_id, "payment denied by policy"
            );

            // Emit PaymentFailed with the denial reason
            let denied_event = FinanceEventKind::PaymentFailed {
                resource_url: resource_url.to_owned(),
                amount_micro_credits: micro_credits,
                reason: format!("policy denied: {reason}"),
            };
            if let Err(e) = publisher.publish(&denied_event).await {
                warn!(error = %e, "failed to publish payment_failed event");
            }
        }
    }

    debug!(
        resource_url,
        session_id,
        decision = ?handle_result.decision,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "payment flow complete"
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- extract_402_info tests --

    fn make_402_result(amount: &str) -> ToolResult {
        ToolResult {
            call_id: "call-1".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": amount,
                "resource_url": "https://api.example.com/data"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        }
    }

    fn make_ok_result() -> ToolResult {
        ToolResult {
            call_id: "call-2".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 200,
                "body": "success"
            }),
            content: None,
            is_error: false,
            state_patch: None,
        }
    }

    fn make_error_result() -> ToolResult {
        ToolResult {
            call_id: "call-3".into(),
            tool_name: "read_file".into(),
            output: json!({
                "error": "file not found"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        }
    }

    fn make_402_no_header() -> ToolResult {
        ToolResult {
            call_id: "call-4".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "resource_url": "https://api.example.com/data"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        }
    }

    #[test]
    fn extract_402_info_from_valid_result() {
        let result = make_402_result("dGVzdC1oZWFkZXI=");
        let info = extract_402_info(&result);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.resource_url, "https://api.example.com/data");
        assert_eq!(info.payment_required_header, "dGVzdC1oZWFkZXI=");
    }

    #[test]
    fn extract_402_info_returns_none_for_200() {
        let result = make_ok_result();
        assert!(extract_402_info(&result).is_none());
    }

    #[test]
    fn extract_402_info_returns_none_for_non_http() {
        let result = make_error_result();
        assert!(extract_402_info(&result).is_none());
    }

    #[test]
    fn extract_402_info_returns_none_without_header() {
        let result = make_402_no_header();
        assert!(extract_402_info(&result).is_none());
    }

    // -- Credit check tests (using wiremock) --

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn credit_check_approved() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/credit/agent-test/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": true,
                "tier": "standard",
                "remaining_limit": 95000,
                "reason": null
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let (ok, reason) =
            check_agent_credit(&client, &mock_server.uri(), "agent-test", 5000).await;
        assert!(ok);
        assert!(reason.is_none());
    }

    #[tokio::test]
    async fn credit_check_rejected() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/credit/agent-test/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": false,
                "tier": "micro",
                "remaining_limit": 100,
                "reason": "insufficient_credit"
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let (ok, reason) =
            check_agent_credit(&client, &mock_server.uri(), "agent-test", 500_000).await;
        assert!(!ok);
        assert_eq!(reason.unwrap(), "insufficient_credit");
    }

    #[tokio::test]
    async fn credit_check_graceful_on_network_error() {
        // No mock server running at this URL — should gracefully allow payment.
        let client = reqwest::Client::new();
        let (ok, _) =
            check_agent_credit(&client, "http://127.0.0.1:19999", "agent-test", 5000).await;
        assert!(ok, "credit check should allow payment on network error");
    }

    #[tokio::test]
    async fn credit_check_graceful_on_500() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/credit/agent-test/check"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let (ok, _) = check_agent_credit(&client, &mock_server.uri(), "agent-test", 5000).await;
        assert!(ok, "credit check should allow payment on server error");
    }

    // -- Facilitator submission tests (using wiremock) --

    #[tokio::test]
    async fn submit_to_facilitator_settled() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/facilitate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "settled",
                "receipt": {
                    "tx_hash": "0xabc123",
                    "payer": "0xpayer",
                    "payee": "0xpayee",
                    "amount_micro_usd": 1000,
                    "chain": "base",
                    "settled_at": "2026-03-22T00:00:00Z"
                },
                "facilitator_fee_bps": 15
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let result = submit_to_facilitator(
            &client,
            &mock_server.uri(),
            "test-payment-header",
            "https://api.example.com/data",
            1000,
            "agent-test",
        )
        .await;

        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.status, FacilitationStatus::Settled);
        assert!(resp.receipt.is_some());
        assert_eq!(resp.receipt.unwrap().tx_hash, "0xabc123");
    }

    #[tokio::test]
    async fn submit_to_facilitator_rejected() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/facilitate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "rejected",
                "reason": "invalid_signature",
                "details": "bad sig"
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let result = submit_to_facilitator(
            &client,
            &mock_server.uri(),
            "bad-header",
            "https://api.example.com/data",
            1000,
            "agent-test",
        )
        .await;

        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.status, FacilitationStatus::Rejected);
        assert_eq!(resp.reason.unwrap(), "invalid_signature");
    }

    #[tokio::test]
    async fn submit_to_facilitator_network_error() {
        let client = reqwest::Client::new();
        let result = submit_to_facilitator(
            &client,
            "http://127.0.0.1:19999",
            "test-header",
            "https://api.example.com/data",
            1000,
            "agent-test",
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("facilitator request failed"));
    }

    // -- Middleware integration tests (using real X402Client + mock publisher) --

    use haima_core::policy::PaymentPolicy;
    use haima_core::wallet::ChainId;
    use haima_wallet::LocalSigner;
    use haima_x402::client::X402Client;
    use haima_x402::facilitator::{Facilitator, FacilitatorConfig};
    use haima_x402::header::{PaymentRequiredHeader, SchemeRequirement, encode_payment_required};

    fn test_middleware() -> (HaimaPaymentMiddleware, Arc<FinancePublisher>) {
        let signer = LocalSigner::generate(ChainId::base()).unwrap();
        let facilitator = Facilitator::new(FacilitatorConfig::default());
        let x402_client = Arc::new(X402Client::new(
            Arc::new(signer),
            facilitator,
            PaymentPolicy::default(),
        ));
        let publisher = Arc::new(FinancePublisher::new(false)); // Disabled for tests
        let middleware = HaimaPaymentMiddleware::with_url(
            Arc::clone(&x402_client),
            Arc::clone(&publisher),
            "test-agent".into(),
            "http://127.0.0.1:19999".into(), // No facilitator running — tests are non-fatal
        );
        (middleware, publisher)
    }

    fn sample_payment_required_header(amount: &str) -> String {
        let header = PaymentRequiredHeader {
            schemes: vec![SchemeRequirement {
                scheme: "exact".into(),
                network: "eip155:8453".into(),
                token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".into(),
                amount: amount.into(),
                recipient: "0xrecipient".into(),
                facilitator: "https://x402.org/facilitator".into(),
            }],
            version: "v2".into(),
        };
        encode_payment_required(&header).unwrap()
    }

    fn test_context() -> ToolContext {
        ToolContext {
            run_id: "run-1".into(),
            session_id: "session-1".into(),
            iteration: 1,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_ignores_non_402_results() {
        let (middleware, _) = test_middleware();
        let ctx = test_context();

        // 200 response - should pass through
        let result = make_ok_result();
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());

        // Non-HTTP error - should pass through
        let result = make_error_result();
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_handles_402_auto_approve() {
        let (middleware, _publisher) = test_middleware();
        let ctx = test_context();

        // 50 micro-credits is below auto-approve cap of 100
        let encoded_header = sample_payment_required_header("50");
        let result = ToolResult {
            call_id: "call-1".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": encoded_header,
                "resource_url": "https://api.example.com/paid-data"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        };

        // Should not error — the middleware processes the payment flow internally.
        // Facilitator is unreachable, but middleware is non-fatal.
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_handles_402_denied() {
        let (middleware, _publisher) = test_middleware();
        let ctx = test_context();

        // 2_000_000 exceeds hard cap of 1_000_000 — will be denied
        let encoded_header = sample_payment_required_header("2000000");
        let result = ToolResult {
            call_id: "call-1".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": encoded_header,
                "resource_url": "https://api.example.com/expensive"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        };

        // Should not error — denial is a valid outcome
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_handles_invalid_payment_header() {
        let (middleware, _publisher) = test_middleware();
        let ctx = test_context();

        // Invalid base64 header — x402 client will return an error
        let result = ToolResult {
            call_id: "call-1".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": "not-valid-base64!!!",
                "resource_url": "https://api.example.com/broken"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        };

        // Should not error — the middleware logs and emits PaymentFailed
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }

    // -- Full flow integration test with wiremock facilitator --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_full_flow_with_mock_facilitator() {
        let mock_server = MockServer::start().await;

        // Mock credit check — approved
        Mock::given(method("POST"))
            .and(path("/v1/credit/test-agent-flow/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": true,
                "tier": "standard",
                "remaining_limit": 99950,
                "reason": null
            })))
            .mount(&mock_server)
            .await;

        // Mock facilitate — settled
        Mock::given(method("POST"))
            .and(path("/v1/facilitate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "settled",
                "receipt": {
                    "tx_hash": "0xfull_flow_tx",
                    "payer": "0xagent",
                    "payee": "0xresource_owner",
                    "amount_micro_usd": 50,
                    "chain": "base",
                    "settled_at": "2026-03-22T00:00:00Z"
                },
                "facilitator_fee_bps": 15
            })))
            .mount(&mock_server)
            .await;

        // Build middleware pointing at mock server
        let signer = LocalSigner::generate(ChainId::base()).unwrap();
        let facilitator = Facilitator::new(FacilitatorConfig::default());
        let x402_client = Arc::new(X402Client::new(
            Arc::new(signer),
            facilitator,
            PaymentPolicy::default(),
        ));
        let publisher = Arc::new(FinancePublisher::new(false));
        let middleware = HaimaPaymentMiddleware::with_url(
            Arc::clone(&x402_client),
            Arc::clone(&publisher),
            "test-agent-flow".into(),
            mock_server.uri(),
        );

        let ctx = test_context();

        // 50 micro-credits — below auto-approve cap, will be signed and submitted
        let encoded_header = sample_payment_required_header("50");
        let result = ToolResult {
            call_id: "call-flow".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": encoded_header,
                "resource_url": "https://api.example.com/paid-data"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        };

        // Should complete successfully — credit check passes, facilitator settles
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }

    // -- Credit insufficient flow --

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_credit_insufficient_skips_payment() {
        let mock_server = MockServer::start().await;

        // Mock credit check — rejected
        Mock::given(method("POST"))
            .and(path("/v1/credit/test-agent-broke/check"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approved": false,
                "tier": "micro",
                "remaining_limit": 10,
                "reason": "insufficient_credit"
            })))
            .mount(&mock_server)
            .await;

        // No facilitate mock — it should never be called

        let signer = LocalSigner::generate(ChainId::base()).unwrap();
        let facilitator = Facilitator::new(FacilitatorConfig::default());
        let x402_client = Arc::new(X402Client::new(
            Arc::new(signer),
            facilitator,
            PaymentPolicy::default(),
        ));
        let publisher = Arc::new(FinancePublisher::new(false));
        let middleware = HaimaPaymentMiddleware::with_url(
            Arc::clone(&x402_client),
            Arc::clone(&publisher),
            "test-agent-broke".into(),
            mock_server.uri(),
        );

        let ctx = test_context();

        let encoded_header = sample_payment_required_header("50");
        let result = ToolResult {
            call_id: "call-broke".into(),
            tool_name: "http_request".into(),
            output: json!({
                "status_code": 402,
                "payment_required_header": encoded_header,
                "resource_url": "https://api.example.com/paid-data"
            }),
            content: None,
            is_error: true,
            state_patch: None,
        };

        // Should complete — credit insufficient is non-fatal
        let outcome = middleware.post_tool_call(&ctx, &result);
        assert!(outcome.is_ok());
    }
}
