//! Mockito-driven happy-path tests for `CubeProvider::create` / `exec` / `destroy`.
//!
//! These complement the unit tests baked into `lib.rs` by exercising the
//! request/response loop end-to-end through the `reqwest` client — every
//! mock asserts the expected number of HTTP calls so phantom retries
//! cannot hide.

use aios_protocol::hypervisor::{
    BackendSelector, ExecRequest, HypervisorBackend, RuntimeHint, VmHandle, VmResources, VmSpec,
    VmStatus,
};
use aios_protocol::ids::{AgentId, SessionId};
use aios_protocol::sandbox::NetworkPolicy;
use arcan_provider_cube::CubeProvider;

/// Minimal `VmSpec` good enough to exercise the create round-trip.
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

/// Synthetic `VmHandle` for routes that take an existing VM.
fn fake_handle(id: &str) -> VmHandle {
    VmHandle {
        vm_id: id.into(),
        backend: "cube".into(),
        session_id: SessionId::from_string("s"),
        agent_id: AgentId::from_string("a"),
        status: VmStatus::Running,
        created_at: chrono::Utc::now(),
        metadata: serde_json::Value::Null,
    }
}

#[tokio::test]
async fn create_round_trips_vm_handle() {
    let mut server = mockito::Server::new_async().await;
    let m = server
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
    let handle = provider.create(minimal_spec()).await.expect("create ok");

    assert_eq!(handle.vm_id.0, "vm-1");
    assert_eq!(handle.backend.0, "cube");
    m.assert_async().await;
}

#[tokio::test]
async fn exec_decodes_base64_stdout() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/v1/vms/vm-1/exec")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"stdout_b64":"aGVsbG8=","stderr_b64":"","exit_code":0,"duration_ms":7}"#)
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let out = provider
        .exec(&fake_handle("vm-1"), ExecRequest::shell("echo hello"))
        .await
        .expect("exec ok");

    assert_eq!(out.stdout, b"hello".to_vec());
    assert!(out.stderr.is_empty());
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.duration_ms, 7);
    m.assert_async().await;
}

#[tokio::test]
async fn destroy_treats_404_as_success() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("DELETE", "/api/v1/vms/vm-1")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"VM_NOT_FOUND","message":"already gone"}}"#)
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    provider
        .destroy(&fake_handle("vm-1"))
        .await
        .expect("destroy must be idempotent on 404");
    m.assert_async().await;
}
