//! Mockito tests covering Cube → `BackendError` mapping.
//!
//! Three flavours: 404 on snapshot (→ `VmNotFound`), 404 on restore (→
//! `SnapshotNotFound`), and 5xx with retry budget exhausted (→ `Internal`
//! carrying the HTTP status). Together they pin the error-routing rules
//! the kernel relies on.

use aios_protocol::hypervisor::{
    BackendError, BackendSelector, HypervisorBackend, RuntimeHint, VmHandle, VmResources, VmSpec,
    VmStatus,
};
use aios_protocol::ids::{AgentId, SessionId};
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
async fn snapshot_404_maps_to_vm_not_found() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/v1/vms/vm-x/snapshot")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"VM_NOT_FOUND","message":"missing"}}"#)
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let err = provider
        .snapshot(&fake_handle("vm-x"))
        .await
        .expect_err("404 must surface");
    assert!(matches!(err, BackendError::VmNotFound(ref id) if id.0 == "vm-x"));
    m.assert_async().await;
}

#[tokio::test]
async fn restore_404_maps_to_snapshot_not_found() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("POST", "/api/v1/snapshots/snap-z/restore")
        .with_status(404)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"SNAPSHOT_NOT_FOUND","message":"missing"}}"#)
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let err = provider
        .restore(&"snap-z".into())
        .await
        .expect_err("404 must surface");
    assert!(matches!(err, BackendError::SnapshotNotFound(ref id) if id.0 == "snap-z"));
    m.assert_async().await;
}

#[tokio::test]
async fn create_5xx_maps_to_internal_with_status() {
    let mut server = mockito::Server::new_async().await;
    // Cube's retry budget is exactly one retry — both attempts hit 503,
    // so we expect two calls total.
    let m = server
        .mock("POST", "/api/v1/vms")
        .with_status(503)
        .with_header("content-type", "application/json")
        .with_body(r#"{"error":{"code":"POOL_EXHAUSTED","message":"no vmm"}}"#)
        .expect(2)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let err = provider
        .create(minimal_spec())
        .await
        .expect_err("503 must surface after retry");
    match err {
        BackendError::Internal(msg) => {
            assert!(
                msg.contains("503"),
                "expected status in message, got: {msg}"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
    m.assert_async().await;
}
