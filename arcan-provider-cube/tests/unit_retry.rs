//! Mockito tests for the retry policy in `client.rs`.
//!
//! Two scenarios pin the contract documented at the top of `client.rs`:
//! * 502 is retryable — a single retry succeeds when the second attempt is 200.
//! * 400 is **not** retryable — exactly one HTTP call hits the server.
//!
//! Each mock asserts an exact call count via `expect()` so a phantom retry
//! would cause the test to fail rather than silently inflate latency in
//! production.

use aios_protocol::hypervisor::{
    BackendSelector, HypervisorBackend, RuntimeHint, VmResources, VmSpec,
};
use aios_protocol::sandbox::NetworkPolicy;
use arcan_provider_cube::CubeProvider;

fn minimal_spec() -> VmSpec {
    VmSpec {
        backend_selector: BackendSelector::Auto,
        resources: VmResources::default(),
        network_policy: NetworkPolicy::Disabled,
        mounts: Vec::new(),
        env: std::collections::HashMap::new(),
        runtime_hint: RuntimeHint::Shell,
        labels: std::collections::HashMap::new(),
    }
}

#[tokio::test]
async fn retries_once_on_502_then_succeeds() {
    let mut server = mockito::Server::new_async().await;

    // First match handles attempt #1 — registered first so mockito returns
    // it before any subsequent matchers.
    let m_fail = server
        .mock("POST", "/api/v1/vms")
        .with_status(502)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"BAD_GATEWAY","message":"flap"}}"#)
        .expect(1)
        .create_async()
        .await;
    let m_ok = server
        .mock("POST", "/api/v1/vms")
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"id":"vm-1","status":{"state":"starting"},"created_at":"2026-04-25T00:00:00Z","metadata":{}}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let handle = provider
        .create(minimal_spec())
        .await
        .expect("retry should succeed on second attempt");
    assert_eq!(handle.vm_id.0, "vm-1");

    m_fail.assert_async().await;
    m_ok.assert_async().await;
}

#[tokio::test]
async fn does_not_retry_on_400() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/v1/vms")
        .with_status(400)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"BAD_REQUEST","message":"bad spec"}}"#)
        .expect(1) // exactly one call — no retry
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let result = provider.create(minimal_spec()).await;
    assert!(
        result.is_err(),
        "400 must surface as an error, got: {result:?}"
    );

    m.assert_async().await;
}
