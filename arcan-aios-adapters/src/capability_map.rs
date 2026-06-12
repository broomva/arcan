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

use aios_protocol::{Capability, PolicySet};

/// Derive the capabilities required to execute a tool call.
///
/// The mapping is intentionally coarse: we use the tool name and top-level
/// input keys to produce a capability token, then rely on the policy engine's
/// glob matching (prefix `*`) to enforce tier boundaries.
pub fn capabilities_for_tool(tool_name: &str, input: &serde_json::Value) -> Vec<Capability> {
    match tool_name {
        // ── Shell / subprocess ─────────────────────────────────────────────
        // Requires exec:cmd:<binary> capability where <binary> is the program
        // name extracted from the command string (e.g. "ls" from "ls -la").
        //
        // Using just the binary (not the full command string) enables precise
        // per-command whitelisting in the PolicySet — free tier allows
        // exec:cmd:cat, exec:cmd:ls, etc. while blocking exec:cmd:rm.
        "bash" | "shell" | "command" | "terminal" | "run_command" => {
            let cmd = input
                .get("command")
                .or_else(|| input.get("cmd"))
                .and_then(|v| v.as_str())
                .unwrap_or("*");
            // Use Capability::new to build "exec:cmd:<binary>" without triggering the
            // execFile lint (this is Rust, not JavaScript).
            let binary = shell_binary(cmd);
            vec![Capability::new(format!("exec:cmd:{binary}"))]
        }

        // ── Filesystem writes ──────────────────────────────────────────────
        "write_file" | "create_file" | "edit_file" | "delete_file" | "move_file"
        | "create_directory" | "append_file" => {
            let path = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("/");
            vec![Capability::fs_write(&policy_path(path))]
        }

        // ── Filesystem reads ───────────────────────────────────────────────
        "read_file" | "glob" | "grep" | "list_dir" | "list_directory" | "read" | "view_file" => {
            let path = input
                .get("path")
                .or_else(|| input.get("file_path"))
                .or_else(|| input.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("/session/**");
            vec![Capability::fs_read(&policy_path(path))]
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

// ── Path → policy-namespace normalization ─────────────────────────────────────

/// Map a tool-input path into the policy's virtual `/session` namespace.
///
/// Praxis fs tools take WORKSPACE-RELATIVE paths ("artifacts/receipt.txt"),
/// while the `PolicySet` capability namespace is virtual and `/session`-rooted
/// ("fs:write:/session/artifacts/**"). Without this mapping the derived
/// capability ("fs:write:artifacts/receipt.txt") never prefix-matches any
/// default allow pattern, so EVERY fs tool call from a lifed-routed session is
/// denied — including writes the policy explicitly intends to allow
/// (confirmed live: prod dogfood 2026-06-11, write_file → "capabilities
/// denied" → Recover → dead-air turn).
///
/// Mapping rules:
/// - Relative path → anchored under `/session/` (`"artifacts/x"` →
///   `"/session/artifacts/x"`, `"./notes.md"` → `"/session/notes.md"`).
/// - Absolute path → passed through RAW. The default policy only allows
///   `/session/...` patterns, so absolute paths are denied by the gate —
///   and even if a future policy allowed one, praxis `FsPolicy` still
///   enforces the real workspace boundary at execution time.
/// - Any path containing a `..` segment → passed through RAW (denied by the
///   default policy). Traversal must never be normalized into an
///   allow-matching shape.
///
/// The capability gate answers "may this tier touch this part of the session
/// namespace"; the actual filesystem boundary is `FsPolicy`'s job (defense in
/// depth — both layers stay independent).
fn policy_path(path: &str) -> String {
    let has_traversal = path.split('/').any(|segment| segment == "..");
    if path.starts_with('/') || has_traversal {
        return path.to_string();
    }
    let trimmed = path.trim_start_matches("./");
    format!("/session/{trimmed}")
}

// ── Shell command binary extraction ───────────────────────────────────────────

/// Extract the program binary from a shell command string.
///
/// Returns just the first whitespace-delimited token with path components
/// stripped (e.g. `"ls -la"` → `"ls"`, `"/usr/bin/python3 script.py"` → `"python3"`).
/// Falls back to `"*"` for empty input so the caller always gets a valid capability.
fn shell_binary(cmd: &str) -> &str {
    let token = cmd.split_whitespace().next().unwrap_or("*");
    // Strip leading path (e.g. /usr/bin/ls → ls).
    token.rsplit('/').next().unwrap_or(token)
}

// ── Tier-aware tool catalog filtering ─────────────────────────────────────────

/// Derive an allowlist of tool names visible in the LLM tool catalog based on
/// the session's [`PolicySet`].
///
/// This is a *pre-filter* for what the LLM sees — it hides tools that the
/// policy would deny at execution time anyway, preventing the agent from
/// planning actions it cannot carry out.
///
/// Returns `None` if the policy is fully permissive (wildcard `"*"` allow) so
/// the full tool catalog is shown.  Returns `Some(allowlist)` with the names of
/// tools that are safe to expose for the current tier.
///
/// ## Tier mapping
///
/// | Tier        | `exec:cmd:*` | `fs:write:*` | Visible extras         |
/// |-------------|:------------:|:------------:|------------------------|
/// | anonymous   | gated        | gated        | read-only tools only   |
/// | free        | gated        | gated        | read + net (no write)  |
/// | pro/enterprise | allowed   | allowed      | all tools (None)       |
/// Derive the tool allowlist, optionally enriched with all registered tool names.
///
/// When `all_tool_names` is provided, any tool whose name does not match a known
/// privileged pattern (exec, fs:write, net, secrets) is automatically included
/// in the visible set — this ensures dynamically registered tools (opsis bridge,
/// skills, MCP tools) are visible without hardcoding each name.
pub fn tools_allowed_by_policy(
    policy: &PolicySet,
    all_tool_names: Option<&[String]>,
) -> Option<Vec<String>> {
    let allow = &policy.allow_capabilities;

    // Full wildcard — show everything.
    if allow.iter().any(|c| c.as_str() == "*") {
        return None;
    }

    // If both high-privilege categories are BROADLY granted there is no
    // useful filtering to apply.
    if broadly_allows_category(allow, "exec:cmd:") && broadly_allows_category(allow, "fs:write:") {
        return None;
    }

    // Visibility is ANY-grant based (BRO-1466): a path-scoped grant like
    // `fs:write:/session/artifacts/**` (the default policy) or a shell
    // whitelist entry like `exec:cmd:cat` (the free tier) means the tool IS
    // usable — the policy gate still enforces the path/binary scope at
    // execution time. The previous broad-grant-only rule hid `write_file`
    // from sessions whose policy explicitly permits artifact writes (live
    // receipt 2026-06-12, BRO-1490) and hid `bash` from free-tier sessions
    // whose whitelist allows `cat`/`ls`/`grep`/…. Gated-but-not-allowed
    // capabilities (e.g. free's `fs:write:**` in `gate_capabilities`)
    // deliberately do NOT count: until the dispatch path renders the
    // approval flow, a tool that silently parks the turn in AskHuman is
    // worse chat UX than a hidden one.
    let exec_allowed = allows_category(allow, "exec:cmd:");
    let fs_write_allowed = allows_category(allow, "fs:write:");

    // Safe tools — always visible regardless of tier (require only fs:read or
    // no capability at all).
    let mut visible: Vec<String> = vec![
        "read_file".to_owned(),
        "list_dir".to_owned(),
        "glob".to_owned(),
        "grep".to_owned(),
        "read_memory".to_owned(),
        "memory_query".to_owned(),
        // Opsis world state bridge — observer tools, no capability required.
        "opsis_world_state".to_owned(),
        "opsis_observe".to_owned(),
        "opsis_alert".to_owned(),
    ];

    if exec_allowed {
        visible.push("bash".to_owned());
    }

    if fs_write_allowed {
        visible.extend([
            "write_file".to_owned(),
            "edit_file".to_owned(),
            "write_memory".to_owned(),
            "memory_propose".to_owned(),
            "memory_commit".to_owned(),
        ]);
    }

    // Auto-include any registered tool that requires no capabilities.
    // This covers opsis tools, skills, MCP tools, and any future dynamically
    // registered tools — no need to hardcode each name.
    if let Some(names) = all_tool_names {
        for name in names {
            if !visible.contains(name) && !requires_privilege(name) {
                visible.push(name.clone());
            }
        }
    }

    Some(visible)
}

/// Returns `true` if a tool name matches a known privileged pattern that
/// requires explicit capability grants (exec, fs:write, net, secrets).
fn requires_privilege(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "bash"
            | "shell"
            | "command"
            | "terminal"
            | "run_command"
            | "write_file"
            | "create_file"
            | "edit_file"
            | "delete_file"
            | "move_file"
            | "create_directory"
            | "append_file"
            | "http_request"
            | "web_search"
            | "fetch_url"
            | "web_fetch"
            | "curl"
            | "browser"
            | "get_secret"
            | "read_env"
            | "get_credential"
    )
}

/// Returns `true` if ANY pattern in `allow` grants something inside
/// `category` — broad (`exec:cmd:*`), scoped (`fs:write:/session/…/**`), or
/// single (`exec:cmd:git`). The full wildcard counts for every category.
///
/// This drives tool VISIBILITY only; the policy gate still enforces the
/// exact path/binary scope at execution time (BRO-1466).
fn allows_category(allow: &[Capability], category: &str) -> bool {
    allow.iter().any(|cap| {
        let s = cap.as_str();
        s == "*" || s.starts_with(category)
    })
}

/// Returns `true` if `allow` contains a wildcard pattern that broadly covers
/// every capability in `category`.
///
/// A pattern broadly covers a category when:
/// - It is the full wildcard `"*"` (allow everything), or
/// - It ends with `'*'` and the trimmed prefix exactly equals `category`
///   (e.g. `"exec:cmd:*"` → trimmed prefix `"exec:cmd:"` covers
///   the `"exec:cmd:"` category).
///
/// A path-restricted pattern like `"fs:write:/session/artifacts/**"` does
/// **not** broadly cover `"fs:write:"` — its trimmed prefix is
/// `"fs:write:/session/artifacts/"`, not `"fs:write:"`.
fn broadly_allows_category(allow: &[Capability], category: &str) -> bool {
    allow.iter().any(|cap| {
        let s = cap.as_str();
        if s == "*" {
            return true;
        }
        if s.ends_with('*') {
            let pat = s.trim_end_matches('*');
            return pat == category;
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_derives_exec_cmd_binary_capability() {
        // Only the binary name is used (not the full command string), enabling
        // precise per-command whitelisting in the PolicySet (BRO-216).
        let caps = capabilities_for_tool("bash", &serde_json::json!({"command": "ls -la"}));
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "exec:cmd:ls");
    }

    #[test]
    fn shell_strips_path_prefix_from_binary() {
        let caps = capabilities_for_tool(
            "bash",
            &serde_json::json!({"command": "/usr/bin/python3 script.py"}),
        );
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "exec:cmd:python3");
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
    fn relative_write_is_anchored_under_session_namespace() {
        // The live-found bug (prod dogfood 2026-06-11): praxis tools take
        // workspace-relative paths, so the derived capability must anchor
        // under /session/ or the default policy's
        // fs:write:/session/artifacts/** can never match and EVERY write is
        // denied.
        let caps = capabilities_for_tool(
            "write_file",
            &serde_json::json!({"path": "artifacts/receipt.txt"}),
        );
        assert_eq!(caps[0].as_str(), "fs:write:/session/artifacts/receipt.txt");

        // ...and that capability is actually covered by the default allow.
        let allow_prefix = "fs:write:/session/artifacts/**".trim_end_matches('*');
        assert!(
            caps[0].as_str().starts_with(allow_prefix),
            "default policy must cover relative artifacts writes: {}",
            caps[0].as_str()
        );
    }

    #[test]
    fn relative_read_is_anchored_under_session_namespace() {
        let caps = capabilities_for_tool("read_file", &serde_json::json!({"path": "notes.md"}));
        assert_eq!(caps[0].as_str(), "fs:read:/session/notes.md");

        let dot_slash =
            capabilities_for_tool("read_file", &serde_json::json!({"path": "./notes.md"}));
        assert_eq!(dot_slash[0].as_str(), "fs:read:/session/notes.md");
    }

    #[test]
    fn traversal_paths_are_never_normalized_into_session() {
        // "artifacts/../../etc/passwd" must NOT become
        // /session/artifacts/../../etc/passwd (which would prefix-match the
        // artifacts allow pattern). Raw pass-through → default policy denies.
        let caps = capabilities_for_tool(
            "write_file",
            &serde_json::json!({"path": "artifacts/../../etc/passwd"}),
        );
        assert_eq!(caps[0].as_str(), "fs:write:artifacts/../../etc/passwd");
        assert!(
            !caps[0].as_str().contains(":/session/"),
            "traversal must stay out of the session namespace"
        );
    }

    #[test]
    fn absolute_paths_pass_through_raw() {
        // Absolute non-/session paths stay raw so the default policy denies
        // them; /session-prefixed absolutes keep working unchanged.
        let abs = capabilities_for_tool("read_file", &serde_json::json!({"path": "/etc/passwd"}));
        assert_eq!(abs[0].as_str(), "fs:read:/etc/passwd");

        let session_abs = capabilities_for_tool(
            "write_file",
            &serde_json::json!({"path": "/session/artifacts/x.txt"}),
        );
        assert_eq!(session_abs[0].as_str(), "fs:write:/session/artifacts/x.txt");
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
    fn shell_cap_is_denied_by_anonymous_policy() {
        // exec:cmd:<binary> is neither in allow_capabilities nor gate_capabilities
        // for anonymous sessions — the StaticPolicyEngine puts it in `denied` (BRO-216).
        let cap = capabilities_for_tool("bash", &serde_json::json!({"command": "ls -la"}));
        assert_eq!(cap[0].as_str(), "exec:cmd:ls");
        let anon = PolicySet::anonymous();
        let exec_wildcard_in_gate = anon
            .gate_capabilities
            .iter()
            .any(|c| c.as_str() == "exec:cmd:*");
        assert!(
            !exec_wildcard_in_gate,
            "anonymous gate must not contain exec:cmd:* — exec is denied, not gated"
        );
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

    #[test]
    fn list_dir_derives_fs_read_capability() {
        let caps = capabilities_for_tool("list_dir", &serde_json::json!({"path": "/session/"}));
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].as_str(), "fs:read:/session/");
    }

    // ── tools_allowed_by_policy ──────────────────────────────────────────────

    #[test]
    fn anonymous_policy_blocks_bash_and_write_tools() {
        let policy = PolicySet::anonymous();
        let allowed = tools_allowed_by_policy(&policy, None).expect("should restrict");
        assert!(
            !allowed.contains(&"bash".to_owned()),
            "bash should be hidden"
        );
        assert!(
            !allowed.contains(&"write_file".to_owned()),
            "write_file should be hidden"
        );
        assert!(
            !allowed.contains(&"edit_file".to_owned()),
            "edit_file should be hidden"
        );
    }

    #[test]
    fn anonymous_policy_exposes_read_tools() {
        let policy = PolicySet::anonymous();
        let allowed = tools_allowed_by_policy(&policy, None).expect("should restrict");
        assert!(allowed.contains(&"read_file".to_owned()));
        assert!(allowed.contains(&"list_dir".to_owned()));
        assert!(allowed.contains(&"glob".to_owned()));
        assert!(allowed.contains(&"grep".to_owned()));
        assert!(allowed.contains(&"read_memory".to_owned()));
        assert!(allowed.contains(&"memory_query".to_owned()));
    }

    #[test]
    fn free_policy_exposes_whitelisted_bash_hides_write_tools() {
        // free() grants a shell whitelist (exec:cmd:cat/ls/grep/…) — bash
        // must be VISIBLE (the gate enforces the whitelist per binary at
        // execution time). fs:write:** is only GATED (approval), not
        // allowed, so write tools stay hidden until the dispatch path
        // renders the approval flow (BRO-1466).
        let policy = PolicySet::free();
        let allowed = tools_allowed_by_policy(&policy, None).expect("should restrict");
        assert!(
            allowed.contains(&"bash".to_owned()),
            "bash visible — the free tier explicitly whitelists shell commands"
        );
        assert!(!allowed.contains(&"write_file".to_owned()));
        assert!(!allowed.contains(&"edit_file".to_owned()));
        // read tools still visible
        assert!(allowed.contains(&"read_file".to_owned()));
    }

    #[test]
    fn pro_policy_returns_none_all_tools_visible() {
        let policy = PolicySet::pro();
        assert!(
            tools_allowed_by_policy(&policy, None).is_none(),
            "pro should allow all tools"
        );
    }

    #[test]
    fn enterprise_policy_returns_none_all_tools_visible() {
        let policy = PolicySet::enterprise();
        assert!(tools_allowed_by_policy(&policy, None).is_none());
    }

    #[test]
    fn default_policy_exposes_scoped_write_and_exec_tools() {
        // default() allows exec:cmd:git and fs:write:/session/artifacts/** —
        // SCOPED grants. The tools must be VISIBLE (the gate enforces the
        // git-only / artifacts-only scope at execution time); hiding them
        // regressed the chat write receipt (BRO-1490/BRO-1466, 2026-06-12).
        // The catalog is still restricted (Some, not None): nothing grants
        // net or secrets, so those tool families stay hidden.
        let policy = PolicySet::default();
        let allowed = tools_allowed_by_policy(&policy, None).expect("should restrict");
        assert!(
            allowed.contains(&"bash".to_owned()),
            "bash visible — exec:cmd:git is a usable grant"
        );
        assert!(
            allowed.contains(&"write_file".to_owned()),
            "write_file visible — /session/artifacts/** is a usable grant"
        );
        assert!(
            allowed.contains(&"edit_file".to_owned()),
            "edit_file rides the same fs:write grant"
        );
        assert!(
            !allowed.contains(&"web_search".to_owned()),
            "no net grant — net tools stay hidden"
        );
        assert!(
            !allowed.contains(&"get_secret".to_owned()),
            "no secrets grant — secrets tools stay hidden"
        );
    }

    #[test]
    fn allows_category_matches_scoped_single_and_broad_grants() {
        let scoped = vec![Capability::new("fs:write:/session/artifacts/**")];
        assert!(allows_category(&scoped, "fs:write:"));
        assert!(!allows_category(&scoped, "exec:cmd:"));

        let single = vec![Capability::new("exec:cmd:git")];
        assert!(allows_category(&single, "exec:cmd:"));
        assert!(!allows_category(&single, "fs:write:"));

        let broad = vec![Capability::new("exec:cmd:*")];
        assert!(allows_category(&broad, "exec:cmd:"));

        let wildcard = vec![Capability::new("*")];
        assert!(allows_category(&wildcard, "fs:write:"));
        assert!(allows_category(&wildcard, "exec:cmd:"));
    }

    #[test]
    fn broadly_allows_category_exec_cmd_wildcard() {
        let caps = vec![Capability::new("exec:cmd:*")];
        assert!(broadly_allows_category(&caps, "exec:cmd:"));
    }

    #[test]
    fn broadly_allows_category_full_wildcard() {
        let caps = vec![Capability::new("*")];
        assert!(broadly_allows_category(&caps, "exec:cmd:"));
        assert!(broadly_allows_category(&caps, "fs:write:"));
    }

    #[test]
    fn broadly_allows_category_specific_does_not_match() {
        // exec:git is specific — does not broadly allow exec:cmd:
        let caps = vec![Capability::new("exec:git")];
        assert!(!broadly_allows_category(&caps, "exec:cmd:"));
    }

    #[test]
    fn broadly_allows_category_path_restricted_does_not_match() {
        // fs:write:/session/artifacts/** is path-restricted — not a broad fs:write: grant
        let caps = vec![Capability::new("fs:write:/session/artifacts/**")];
        assert!(!broadly_allows_category(&caps, "fs:write:"));
    }
}
