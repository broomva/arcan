//! Wire types for CubeAPI v1.
//!
//! Mirrors the JSON shapes documented in
//! <https://github.com/TencentCloud/CubeSandbox> (v1 API reference).
//! Field names match the upstream spec verbatim — if upstream renames
//! a field, this module must follow.
//!
//! Every type here is private to the crate; `lib.rs` performs the
//! mapping into the public canonical types from
//! `aios_protocol::hypervisor`.

#![allow(dead_code)] // populated by Tasks 5–8; trait impl wires consumers.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Create VM ───────────────────────────────────────────────────────

/// `POST /api/v1/vms`.
#[derive(Debug, Serialize)]
pub(crate) struct CreateVmReq {
    pub vcpus: u32,
    pub memory_mb: u64,
    pub disk_mb: u64,
    pub timeout_secs: u64,
    pub runtime: RuntimeKind,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<MountReq>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, String>,
    pub network: NetworkPolicyReq,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RuntimeKind {
    Shell,
    Node { version: String },
    Python { version: String },
    Custom { image: String },
}

#[derive(Debug, Serialize)]
pub(crate) struct MountReq {
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum NetworkPolicyReq {
    Disabled,
    AllowList { cidrs: Vec<String> },
    Open,
}

/// Response body for `POST /api/v1/vms` and `GET /api/v1/vms/{id}`.
#[derive(Debug, Deserialize)]
pub(crate) struct VmResp {
    pub id: String,
    pub status: VmStatusResp,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum VmStatusResp {
    Starting,
    Running,
    Snapshotted,
    Stopping,
    Stopped,
    Failed { reason: String },
}

// ── Exec ────────────────────────────────────────────────────────────

/// `POST /api/v1/vms/{id}/exec`.
#[derive(Debug, Serialize)]
pub(crate) struct ExecReq {
    pub command: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin_b64: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExecResp {
    pub stdout_b64: String,
    pub stderr_b64: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

// ── Snapshot / restore ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct SnapshotResp {
    pub id: String,
    pub vm_id: String,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub name: Option<String>,
}

// ── List ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct VmListResp {
    pub vms: Vec<VmListItem>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VmListItem {
    pub id: String,
    pub status: VmStatusResp,
    pub created_at: DateTime<Utc>,
}

// ── Filesystem ──────────────────────────────────────────────────────

/// `POST /api/v1/vms/{id}/files` body (batch write).
#[derive(Debug, Serialize)]
pub(crate) struct WriteFilesReq {
    pub files: Vec<FileWriteEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct FileWriteEntry {
    pub path: String,
    pub mode: u32,
    /// Base64-encoded body — Cube's API takes binary as base64 in the JSON envelope.
    pub content_b64: String,
}

/// `GET /api/v1/vms/{id}/files?path=...` response.
#[derive(Debug, Deserialize)]
pub(crate) struct ReadFileResp {
    pub content_b64: String,
}

// ── Error envelope ──────────────────────────────────────────────────

/// Cube's standard error envelope: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiErrorEnvelope {
    pub error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiErrorBody {
    #[serde(default)]
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_vm_req_omits_empty_collections() {
        let req = CreateVmReq {
            vcpus: 2,
            memory_mb: 512,
            disk_mb: 2048,
            timeout_secs: 60,
            runtime: RuntimeKind::Shell,
            env: HashMap::new(),
            mounts: Vec::new(),
            tags: HashMap::new(),
            network: NetworkPolicyReq::Disabled,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("env"));
        assert!(!json.contains("mounts"));
        assert!(!json.contains("tags"));
    }

    #[test]
    fn vm_status_failed_round_trips_reason() {
        let json = r#"{"state": "failed", "reason": "oom"}"#;
        let parsed: VmStatusResp = serde_json::from_str(json).unwrap();
        assert!(matches!(parsed, VmStatusResp::Failed { ref reason } if reason == "oom"));
    }

    #[test]
    fn exec_resp_round_trips() {
        let json = r#"{"stdout_b64":"aGk=","stderr_b64":"","exit_code":0,"duration_ms":12}"#;
        let parsed: ExecResp = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.exit_code, 0);
        assert_eq!(parsed.duration_ms, 12);
    }

    #[test]
    fn api_error_envelope_decodes() {
        let json = r#"{"error":{"code":"VM_NOT_FOUND","message":"vm-1 missing"}}"#;
        let parsed: ApiErrorEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.error.code, "VM_NOT_FOUND");
        assert_eq!(parsed.error.message, "vm-1 missing");
    }
}
