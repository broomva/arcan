//! Conversions between the canonical `aios_protocol::hypervisor` types and
//! the private CubeAPI v1 wire types in [`crate::types`].
//!
//! The boundary between protocol and wire is intentionally narrow: every
//! function here is a pure, total mapping with no I/O, so it can be unit
//! tested without spinning up an HTTP layer.

use aios_protocol::hypervisor::{
    BackendId, ExecRequest, ExecResult, Mount, RuntimeHint, VmHandle, VmId, VmSpec, VmStatus,
};
use aios_protocol::ids::{AgentId, SessionId};
use aios_protocol::sandbox::NetworkPolicy;
use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::error::CubeError;
use crate::types::{
    CreateVmReq, ExecReq, ExecResp, MountReq, NetworkPolicyReq, RuntimeKind, VmResp, VmStatusResp,
};

/// Build a [`CreateVmReq`] from a canonical [`VmSpec`].
///
/// Cube's HTTP API exposes memory + disk as megabytes, while the canonical
/// vocabulary keeps everything in kilobytes for forward compatibility with
/// nano-VMs and lightweight WASM guests. The conversion divides by 1024 and
/// rounds toward zero — callers wanting a precise floor MUST pass already
/// MiB-aligned values.
pub(crate) fn create_vm_req_from_spec(spec: &VmSpec) -> CreateVmReq {
    CreateVmReq {
        vcpus: spec.resources.vcpus,
        memory_mb: spec.resources.memory_kb / 1024,
        disk_mb: spec.resources.disk_kb / 1024,
        timeout_secs: spec.resources.timeout_secs,
        runtime: runtime_from_hint(&spec.runtime_hint),
        env: spec.env.clone(),
        mounts: spec.mounts.iter().map(mount_to_req).collect(),
        tags: spec.labels.clone(),
        network: network_to_req(&spec.network_policy),
    }
}

/// Map a [`RuntimeHint`] into Cube's runtime selector.
///
/// `RuntimeHint` is `#[non_exhaustive]`; new variants are coerced to
/// [`RuntimeKind::Shell`] so a future kernel version that adds, say, a
/// `RuntimeHint::Bun` does not break Cube backends pinned to v0.3 — the
/// provider degrades gracefully instead of panicking.
pub(crate) fn runtime_from_hint(hint: &RuntimeHint) -> RuntimeKind {
    match hint {
        RuntimeHint::Shell => RuntimeKind::Shell,
        RuntimeHint::Node { version } => RuntimeKind::Node {
            version: version.clone(),
        },
        RuntimeHint::Python { version } => RuntimeKind::Python {
            version: version.clone(),
        },
        RuntimeHint::Custom { image } => RuntimeKind::Custom {
            image: image.clone(),
        },
        // `RuntimeHint` is `#[non_exhaustive]` — fall through gracefully
        // when a new variant lands upstream before the Cube backend has
        // been updated.
        _ => RuntimeKind::Shell,
    }
}

/// Map a canonical [`Mount`] into the Cube wire shape.
pub(crate) fn mount_to_req(m: &Mount) -> MountReq {
    MountReq {
        source: m.source.clone(),
        target: m.target.clone(),
        read_only: m.read_only,
    }
}

/// Map a canonical [`NetworkPolicy`] (from `aios_protocol::sandbox`) into
/// the Cube wire shape.
///
/// Cube exposes "no restrictions" as `Open` and "explicit allow-list" as
/// `AllowList { cidrs }`. The canonical vocabulary calls the open variant
/// `AllowAll` and threads the allow-list as `hosts` — the conversion
/// preserves the values verbatim because Cube treats CIDR strings and
/// host strings interchangeably (it parses each entry as either a CIDR
/// network or an FQDN).
pub(crate) fn network_to_req(p: &NetworkPolicy) -> NetworkPolicyReq {
    match p {
        NetworkPolicy::Disabled => NetworkPolicyReq::Disabled,
        NetworkPolicy::AllowAll => NetworkPolicyReq::Open,
        NetworkPolicy::AllowList { hosts } => NetworkPolicyReq::AllowList {
            cidrs: hosts.clone(),
        },
    }
}

/// Encode a canonical [`ExecRequest`] into the Cube wire shape.
///
/// `stdin` is base64-encoded because the Cube envelope is JSON; raw bytes
/// would otherwise need lossy UTF-8 coercion.
pub(crate) fn exec_req_from(req: &ExecRequest) -> ExecReq {
    ExecReq {
        command: req.command.clone(),
        working_dir: req.working_dir.clone(),
        env: req.env.clone(),
        timeout_secs: req.timeout_secs,
        stdin_b64: req.stdin.as_ref().map(|s| STANDARD.encode(s)),
    }
}

/// Decode an [`ExecResp`] back into a canonical [`ExecResult`].
///
/// Returns [`CubeError::Decode`] when stdout/stderr are not valid base64 —
/// the kernel turns this into [`aios_protocol::hypervisor::BackendError::Transport`]
/// at the trait boundary, which matches Cube's actual failure mode (a
/// corrupt response was already a transport-level fault).
pub(crate) fn exec_result_from_resp(resp: ExecResp) -> Result<ExecResult, CubeError> {
    let stdout = STANDARD
        .decode(&resp.stdout_b64)
        .map_err(|e| CubeError::Decode(format!("stdout: {e}")))?;
    let stderr = STANDARD
        .decode(&resp.stderr_b64)
        .map_err(|e| CubeError::Decode(format!("stderr: {e}")))?;
    Ok(ExecResult {
        stdout,
        stderr,
        exit_code: resp.exit_code,
        duration_ms: resp.duration_ms,
    })
}

/// Build a canonical [`VmHandle`] from a [`VmResp`] plus the session +
/// agent attribution captured by the provider.
pub(crate) fn vm_handle_from_resp(
    resp: VmResp,
    backend_name: &'static str,
    session: &SessionId,
    agent: &AgentId,
) -> VmHandle {
    VmHandle {
        vm_id: VmId::from(resp.id),
        backend: BackendId::from(backend_name),
        session_id: session.clone(),
        agent_id: agent.clone(),
        status: status_from_resp(resp.status),
        created_at: resp.created_at,
        metadata: resp.metadata,
    }
}

/// Map Cube's status enum onto the canonical [`VmStatus`].
///
/// Cube does not currently expose a `Hibernated` state — backends advertise
/// `BackendCapabilitySet::HIBERNATE` separately, so omitting the variant
/// here is correct (a Cube response cannot decode into `Hibernated`).
pub(crate) fn status_from_resp(s: VmStatusResp) -> VmStatus {
    match s {
        VmStatusResp::Starting => VmStatus::Starting,
        VmStatusResp::Running => VmStatus::Running,
        VmStatusResp::Snapshotted => VmStatus::Snapshotted,
        VmStatusResp::Stopping => VmStatus::Stopping,
        VmStatusResp::Stopped => VmStatus::Stopped,
        VmStatusResp::Failed { reason } => VmStatus::Failed { reason },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use aios_protocol::hypervisor::{BackendSelector, VmResources, VmSpec};

    use super::*;

    #[test]
    fn create_req_converts_kb_to_mb() {
        let spec = VmSpec {
            backend_selector: BackendSelector::Auto,
            resources: VmResources {
                vcpus: 4,
                memory_kb: 2 * 1024 * 1024,
                disk_kb: 8 * 1024 * 1024,
                timeout_secs: 120,
            },
            network_policy: NetworkPolicy::Disabled,
            mounts: Vec::new(),
            env: HashMap::new(),
            runtime_hint: RuntimeHint::Shell,
            labels: HashMap::new(),
        };
        let req = create_vm_req_from_spec(&spec);
        assert_eq!(req.vcpus, 4);
        assert_eq!(req.memory_mb, 2048);
        assert_eq!(req.disk_mb, 8192);
        assert_eq!(req.timeout_secs, 120);
    }

    #[test]
    fn runtime_hint_maps_to_runtime_kind_node() {
        let kind = runtime_from_hint(&RuntimeHint::Node {
            version: "20.11".into(),
        });
        let json = serde_json::to_string(&kind).unwrap();
        assert!(json.contains("\"kind\":\"node\""));
        assert!(json.contains("20.11"));
    }

    #[test]
    fn mount_round_trips_read_only_flag() {
        let m = Mount {
            source: "/host".into(),
            target: "/guest".into(),
            read_only: true,
        };
        let req = mount_to_req(&m);
        assert_eq!(req.source, "/host");
        assert_eq!(req.target, "/guest");
        assert!(req.read_only);
    }

    #[test]
    fn network_disabled_maps_to_disabled() {
        let req = network_to_req(&NetworkPolicy::Disabled);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"kind\":\"disabled\""));
    }

    #[test]
    fn network_allow_all_maps_to_open() {
        let req = network_to_req(&NetworkPolicy::AllowAll);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"kind\":\"open\""));
    }

    #[test]
    fn network_allow_list_threads_hosts_into_cidrs() {
        let req = network_to_req(&NetworkPolicy::AllowList {
            hosts: vec!["api.anthropic.com".into(), "10.0.0.0/8".into()],
        });
        match req {
            NetworkPolicyReq::AllowList { cidrs } => {
                assert_eq!(cidrs, vec!["api.anthropic.com", "10.0.0.0/8"]);
            }
            _ => panic!("expected AllowList"),
        }
    }

    #[test]
    fn exec_req_base64_encodes_stdin() {
        let req = ExecRequest {
            command: vec!["cat".into()],
            working_dir: None,
            env: HashMap::new(),
            timeout_secs: None,
            stdin: Some(b"hi".to_vec()),
        };
        let encoded = exec_req_from(&req);
        assert_eq!(encoded.stdin_b64.as_deref(), Some("aGk="));
    }

    #[test]
    fn exec_result_decodes_base64_stdout() {
        let resp = ExecResp {
            stdout_b64: "aGVsbG8=".into(),
            stderr_b64: String::new(),
            exit_code: 0,
            duration_ms: 12,
        };
        let result = exec_result_from_resp(resp).unwrap();
        assert_eq!(result.stdout, b"hello");
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.duration_ms, 12);
    }

    #[test]
    fn exec_result_rejects_invalid_base64() {
        let resp = ExecResp {
            stdout_b64: "@@not-base64@@".into(),
            stderr_b64: String::new(),
            exit_code: 0,
            duration_ms: 0,
        };
        assert!(matches!(
            exec_result_from_resp(resp),
            Err(CubeError::Decode(_))
        ));
    }

    #[test]
    fn vm_handle_carries_session_and_agent_attribution() {
        let resp = VmResp {
            id: "vm-1".into(),
            status: VmStatusResp::Running,
            created_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        };
        let session = SessionId::from_string("sess-1");
        let agent = AgentId::from_string("agent-1");
        let handle = vm_handle_from_resp(resp, "cube", &session, &agent);
        assert_eq!(handle.vm_id.0, "vm-1");
        assert_eq!(handle.backend.0, "cube");
        assert_eq!(handle.session_id.as_str(), "sess-1");
        assert_eq!(handle.agent_id.as_str(), "agent-1");
        assert!(matches!(handle.status, VmStatus::Running));
    }

    #[test]
    fn status_failed_preserves_reason() {
        let mapped = status_from_resp(VmStatusResp::Failed {
            reason: "oom".into(),
        });
        match mapped {
            VmStatus::Failed { reason } => assert_eq!(reason, "oom"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }
}
