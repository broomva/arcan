//! Mockito tests for the [`HypervisorFilesystemExt`] surface and `list`.
//!
//! These pin three behaviours the kernel relies on:
//! * `write_files` base64-encodes binary contents into the JSON envelope.
//! * `read_file` URL-encodes its `path` query parameter (verified by
//!   matching the literal request URI against the percent-encoded form).
//! * `list` decodes a `VmListResp` into the canonical `Vec<VmInfo>`.

use aios_protocol::hypervisor::{
    FileWrite, HypervisorBackend, HypervisorFilesystemExt, VmHandle, VmStatus,
};
use aios_protocol::ids::{AgentId, SessionId};
use arcan_provider_cube::CubeProvider;

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
async fn write_files_round_trips_base64_body() {
    let mut server = mockito::Server::new_async().await;
    // `0o644` == `420` decimal — the request body must contain the
    // numeric mode and the base64-encoded contents (`hello` → `aGVsbG8=`).
    let m = server
        .mock("POST", "/api/v1/vms/vm-1/files")
        .match_body(mockito::Matcher::PartialJsonString(
            r#"{"files":[{"path":"/tmp/x","mode":420,"content_b64":"aGVsbG8="}]}"#.into(),
        ))
        // Cube actually returns 204 with no body, but our `request` helper
        // decodes a `serde_json::Value` from the response — keeping the
        // mock at 200 with `{}` matches the decoder contract until we
        // teach the client to skip JSON decode on 204 (BRO-918 territory).
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body("{}")
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    provider
        .write_files(
            &fake_handle("vm-1"),
            vec![FileWrite {
                path: "/tmp/x".into(),
                content: b"hello".to_vec(),
                mode: 0o644,
            }],
        )
        .await
        .expect("write ok");

    m.assert_async().await;
}

#[tokio::test]
async fn read_file_url_encodes_path() {
    let mut server = mockito::Server::new_async().await;
    // Path contains a space and slashes — they must arrive percent-encoded.
    let m = server
        .mock("GET", "/api/v1/vms/vm-1/files?path=%2Ftmp%2Fhello%20world")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"content_b64":"aGVsbG8="}"#)
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let bytes = provider
        .read_file(&fake_handle("vm-1"), "/tmp/hello world")
        .await
        .expect("read ok");
    assert_eq!(bytes, b"hello".to_vec());
    m.assert_async().await;
}

#[tokio::test]
async fn list_decodes_vm_info() {
    let mut server = mockito::Server::new_async().await;
    let m = server
        .mock("GET", "/api/v1/vms")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            r#"{"vms":[
                {"id":"vm-1","status":{"state":"running"},"created_at":"2026-04-25T00:00:00Z"},
                {"id":"vm-2","status":{"state":"snapshotted"},"created_at":"2026-04-25T01:00:00Z"}
            ]}"#,
        )
        .expect(1)
        .create_async()
        .await;

    let provider = CubeProvider::new(server.url(), "test-token");
    let vms = provider.list().await.expect("list ok");

    assert_eq!(vms.len(), 2);
    assert_eq!(vms[0].vm_id.0, "vm-1");
    assert_eq!(vms[0].backend.0, "cube");
    assert!(matches!(vms[0].status, VmStatus::Running));
    assert_eq!(vms[1].vm_id.0, "vm-2");
    assert!(matches!(vms[1].status, VmStatus::Snapshotted));

    // Sanity check that the `name()` accessor is wired — the trait
    // bound check above already enforces the impl exists, but keeping
    // a runtime hit guards against accidental override drift.
    assert_eq!(provider.name(), "cube");
    m.assert_async().await;
}
