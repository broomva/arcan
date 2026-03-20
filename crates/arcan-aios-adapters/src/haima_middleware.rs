//! Haima payment middleware — intercepts HTTP 402 tool results and invokes
//! the x402 payment flow.
//!
//! [`HaimaPaymentMiddleware`] implements the [`Middleware`] trait as a
//! **post-tool-call** interceptor. When a tool result indicates an HTTP 402
//! response with a `payment-required` header, the middleware:
//!
//! 1. Invokes [`X402Client::handle_402`] to parse terms, evaluate policy, and
//!    optionally sign the payment.
//! 2. Publishes the appropriate [`FinanceEventKind`] to the Lago journal via
//!    [`FinancePublisher`].
//! 3. Logs the outcome (approved / requires-approval / denied).
//!
//! The middleware does **not** retry the tool call — that is the orchestrator's
//! responsibility. It only processes the payment side-channel so that the
//! financial state is updated and events are emitted.

use std::sync::Arc;

use arcan_core::runtime::Middleware;
use arcan_core::{CoreError, ToolContext, ToolResult};
use haima_core::event::FinanceEventKind;
use haima_core::payment::PaymentDecision;
use haima_lago::publisher::FinancePublisher;
use haima_x402::client::X402Client;
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

// ---------------------------------------------------------------------------
// HaimaPaymentMiddleware
// ---------------------------------------------------------------------------

/// Post-tool-call middleware that handles HTTP 402 payment flows.
///
/// When a tool result's `output` JSON contains:
/// - `status_code: 402`
/// - `payment_required_header: "<base64-encoded header>"`
/// - `resource_url: "<url>"`
///
/// the middleware invokes the x402 client to evaluate and optionally sign the
/// payment, then emits the corresponding finance event.
pub struct HaimaPaymentMiddleware {
    x402_client: Arc<X402Client>,
    publisher: Arc<FinancePublisher>,
}

impl HaimaPaymentMiddleware {
    /// Create a new payment middleware.
    pub fn new(x402_client: Arc<X402Client>, publisher: Arc<FinancePublisher>) -> Self {
        Self {
            x402_client,
            publisher,
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
            "detected HTTP 402 in tool result, invoking x402 payment flow"
        );

        // Spawn the async payment handling on the current runtime.
        // The middleware trait is sync, so we bridge to async via spawn_blocking
        // inverse: we spawn an async task and block on it.
        let x402_client = Arc::clone(&self.x402_client);
        let publisher = Arc::clone(&self.publisher);
        let resource_url = info.resource_url.clone();
        let payment_header = info.payment_required_header.clone();
        let session_id = context.session_id.clone();

        // Use tokio::task::block_in_place to allow blocking within an async
        // context, then use Handle::current() to run the async work.
        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async move {
                handle_payment_flow(
                    &x402_client,
                    &publisher,
                    &resource_url,
                    &payment_header,
                    &session_id,
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
// Payment flow
// ---------------------------------------------------------------------------

/// Process the x402 payment flow and emit finance events.
async fn handle_payment_flow(
    x402_client: &X402Client,
    publisher: &FinancePublisher,
    resource_url: &str,
    payment_required_header: &str,
    session_id: &str,
) {
    // Step 1: Invoke x402 client to parse, evaluate, and optionally sign.
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
    let micro_credits = amount_str.parse::<i64>().unwrap_or(0);

    // Step 2: Emit events based on the decision.
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
                "payment auto-approved and signed"
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
        let middleware =
            HaimaPaymentMiddleware::new(Arc::clone(&x402_client), Arc::clone(&publisher));
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

        // Should not error — the middleware processes the payment flow internally
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
}
