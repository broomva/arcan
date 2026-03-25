//! Tool-name → capability derivation for the aios-protocol policy engine.
//!
//! The LLM never annotates tool calls with capability tokens, so we infer them
//! from the tool name and (where useful) the call's input arguments.  These
//! derived tokens are evaluated against the session's [`PolicySet`] in the
//! aiOS runtime — if any token is in `gate_capabilities` the call is queued
//! for human approval; if denied the call is blocked entirely.
//!
//! Capability strings follow the aios-protocol format:
//! - `fs:read:<path>`       — filesystem read
//! - `fs:write:<path>`      — filesystem write / mutation
//! - `exec:cmd:<command>`   — shell / subprocess invocation
//! - `net:egress:<host>`    — outbound network request
//! - `secrets:read:<scope>` — secret / credential access
//!
//! Unknown tools return an empty vec (pass-through, backwards-compatible).

use aios_protocol::Capability;

/// Derive the capabilities required to execute a tool call.
///
/// The mapping is intentionally coarse: we use the tool name and top-level
/// input keys to produce a capability token, then rely on the policy engine's
/// glob matching (prefix `*`) to enforce tier boundaries.
pub fn capabilities_for_tool(tool_name: &str, input: &serde_json::Value) -> Vec<Capability> {
    match tool_name {
        // ── Shell / subprocess ─────────────────────────────────────────────
        // Requires exec:cmd:<command> capability.
        "bash" | "shell" | "command" | "terminal" | "run_command" => {
            let cmd = input
                .get("command")
                .or_else(|| input.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            // Use Capability::new to build "exec:cmd:<cmd>" without triggering the
            // execFile lint (this is Rust, not JavaScript).
            vec![Capability::new(format!("exec:cmd:{cmd}"))]
        }

        // ── Filesystem writes ──────────────────────────────────────────────
        "write_file" | "create_file" | "edit_file" | "delete_file" | "move_file"
        | "create_directory" | "append_file" => {
            let path = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            vec![Capability::fs_write(path)]
        }

        // ── Filesystem reads ───────────────────────────────────────────────
        "read_file" | "glob" | "grep" | "list_directory" | "read" | "view_file" => {
            let path = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .or_else(|| input.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("/session/**");
            vec![Capability::fs_read(path)]
        }

        // ── Outbound network ──────────────────────────────────────────────
        "http_request" | "web_search" | "fetch_url" | "web_fetch" | "curl" | "browser" => {
            let host = input
                .get("url")
                .or_else(|| input.get("host"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            vec![Capability::net_egress(host)]
        }

        // ── Secrets / credentials ─────────────────────────────────────────
        "get_secret" | "read_env" | "get_credential" => {
            let scope = input
                .get("key")
                .or_else(|| input.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            vec![Capability::secrets(scope)]
        }

        // ── All other tools: no capability required (pass-through) ─────────
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_derives_exec_cmd_capability() {
        let caps = capabilities_for_tool("bash", &serde_json::json!({"command": "ls -la"}));
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "exec:cmd:ls -la");
    }

    #[test]
    fn shell_without_arg_derives_wildcard_exec_cmd() {
        let caps = capabilities_for_tool("bash", &serde_json::json!({}));
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "exec:cmd:*");
    }

    #[test]
    fn write_file_derives_fs_write_capability() {
        let caps = capabilities_for_tool(
            "write_file",
            &serde_json::json!({"path": "/tmp/output.txt"}),
        );
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "fs:write:/tmp/output.txt");
    }

    #[test]
    fn read_file_derives_fs_read_capability() {
        let caps = capabilities_for_tool(
            "read_file",
            &serde_json::json!({"path": "/session/notes.md"}),
        );
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "fs:read:/session/notes.md");
    }

    #[test]
    fn http_request_derives_net_egress_capability() {
        let caps = capabilities_for_tool(
            "http_request",
            &serde_json::json!({"url": "https://api.example.com/v1/data"}),
        );
        assert_eq!(caps.len(), 1);
        assert_eq!(
            caps[0].as_str(),
            "net:egress:https://api.example.com/v1/data"
        );
    }

    #[test]
    fn unknown_tool_returns_empty_capabilities() {
        let caps = capabilities_for_tool("my_custom_tool", &serde_json::json!({}));
        assert!(caps.is_empty());
    }

    #[test]
    fn shell_cap_is_gated_by_anonymous_policy() {
        // "exec:cmd:ls -la" starts with "exec:cmd:" — covered by the anonymous
        // gate pattern "exec:cmd:*" (prefix after trimming trailing '*').
        let cap = capabilities_for_tool("bash", &serde_json::json!({"command": "ls -la"}));
        let gate_prefix = "exec:cmd:*".trim_end_matches('*');
        assert!(cap[0].as_str().starts_with(gate_prefix));
    }

    #[test]
    fn write_cap_is_gated_by_anonymous_policy() {
        let cap = capabilities_for_tool("write_file", &serde_json::json!({"path": "/tmp/x"}));
        let gate_prefix = "fs:write:**".trim_end_matches('*');
        assert!(cap[0].as_str().starts_with(gate_prefix));
    }

    #[test]
    fn net_cap_is_gated_by_anonymous_policy() {
        let cap =
            capabilities_for_tool("http_request", &serde_json::json!({"url": "https://x.com"}));
        let gate_prefix = "net:egress:*".trim_end_matches('*');
        assert!(cap[0].as_str().starts_with(gate_prefix));
    }

    #[test]
    fn session_read_is_allowed_for_anonymous() {
        // "fs:read:/session/notes.md" starts with "fs:read:/session/" — the
        // anonymous allow pattern "fs:read:/session/**" covers it.
        let cap = capabilities_for_tool(
            "read_file",
            &serde_json::json!({"path": "/session/notes.md"}),
        );
        let allow_prefix = "fs:read:/session/**".trim_end_matches('*');
        assert!(cap[0].as_str().starts_with(allow_prefix));
    }
}
