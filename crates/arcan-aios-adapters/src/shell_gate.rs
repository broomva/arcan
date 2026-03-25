//! Tier-specific shell execution policy helpers for Arcan (BRO-216).
//!
//! Shell/bash execution is the highest-risk capability in the Arcan runtime.
//! This module provides utilities for deriving the shell policy from a
//! session's [`PolicySet`] and validating commands against it.
//!
//! # Enforcement model
//!
//! Enforcement happens at **two complementary layers**:
//!
//! 1. **PolicySet / capability evaluation** (aios-protocol + aios-policy):
//!    `capabilities_for_tool("bash", input)` returns `exec:cmd:<binary>`.
//!    The `StaticPolicyEngine` evaluates this against the session's policy:
//!    - Anonymous: `exec:cmd:*` is absent from `allow_capabilities` and
//!      `gate_capabilities` → **immediately denied** (no approval ticket).
//!    - Free: only whitelisted binaries (`cat`, `ls`, `echo`, …) are in
//!      `allow_capabilities` → whitelisted commands allowed, others denied.
//!    - Pro/Enterprise: wildcard `"*"` in `allow_capabilities` → all allowed.
//!
//! 2. **Tier catalog filtering** (BRO-214): The `bash` tool is hidden from the
//!    LLM's tool list for anonymous/free tiers, preventing the model from
//!    planning shell-based actions it cannot execute.
//!
//! # Tier matrix
//!
//! | Tier        | Shell Access      | Mechanism                    |
//! |-------------|-------------------|------------------------------|
//! | Anonymous   | Blocked (denied)  | `exec:cmd:*` absent from policy |
//! | Free        | Whitelist only    | specific `exec:cmd:<binary>` allowed |
//! | Pro         | Full access       | wildcard `"*"` in policy     |
//! | Enterprise  | Full access       | wildcard `"*"` in policy     |

use aios_protocol::PolicySet;

/// Safe read-only shell commands allowed for the free tier.
///
/// These are restricted to non-destructive, read-only operations that cannot
/// exfiltrate credentials, modify system state, or escalate privileges.
pub const FREE_TIER_ALLOWED_COMMANDS: &[&str] = &[
    "cat", "echo", "find", "grep", "head", "jq", "ls", "python3", "sort", "tail", "wc",
];

/// Tier-specific shell execution policy derived from a [`PolicySet`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellPolicy {
    /// Shell execution is blocked entirely (anonymous tier).
    ///
    /// Any `bash` or `shell` tool call is immediately denied without
    /// creating an approval ticket.
    Blocked,
    /// Only whitelisted commands are allowed (free tier).
    ///
    /// The whitelist is the intersection of the free tier's `allow_capabilities`
    /// and a safe hard-coded set of read-only binaries.
    Whitelisted,
    /// Full shell access within the workspace (pro/enterprise).
    Unrestricted,
}

/// Derive the shell execution policy from a [`PolicySet`].
///
/// This mirrors the PolicySet evaluation performed by `StaticPolicyEngine`
/// at runtime, giving call sites a high-level view of what shell policy
/// applies without having to replicate the capability matching logic.
pub fn shell_policy_for(policy: &PolicySet) -> ShellPolicy {
    // Pro/Enterprise: wildcard allow → unrestricted.
    if policy.allow_capabilities.iter().any(|c| c.as_str() == "*") {
        return ShellPolicy::Unrestricted;
    }

    // Check whether exec:cmd:* is broadly allowed (e.g. exec:cmd:* wildcard).
    let exec_broadly_allowed = policy.allow_capabilities.iter().any(|c| {
        let s = c.as_str();
        s == "exec:cmd:*" || (s.starts_with("exec:cmd:") && s.ends_with('*'))
    });
    if exec_broadly_allowed {
        return ShellPolicy::Unrestricted;
    }

    // Check whether at least one specific exec:cmd:<binary> is allowed.
    let exec_any_allowed = policy
        .allow_capabilities
        .iter()
        .any(|c| c.as_str().starts_with("exec:cmd:"));
    if exec_any_allowed {
        return ShellPolicy::Whitelisted;
    }

    // No exec capabilities at all → blocked.
    ShellPolicy::Blocked
}

/// Validate a shell command against a [`ShellPolicy`].
///
/// Returns `Ok(())` if the command is permitted, or an error string describing
/// the violation.  The caller should convert the error to the appropriate
/// domain error type (e.g. `CoreError::Middleware`).
pub fn validate_shell_command(cmd: &str, policy: &ShellPolicy) -> Result<(), String> {
    match policy {
        ShellPolicy::Blocked => Err(format!(
            "shell execution blocked by tier policy: exec:cmd:{} denied",
            shell_binary(cmd)
        )),
        ShellPolicy::Unrestricted => Ok(()),
        ShellPolicy::Whitelisted => {
            let binary = shell_binary(cmd);
            if FREE_TIER_ALLOWED_COMMANDS.contains(&binary) {
                Ok(())
            } else {
                Err(format!(
                    "command '{}' not in free-tier shell allowlist",
                    binary
                ))
            }
        }
    }
}

/// Extract the program binary from a shell command string.
///
/// Strips leading path components and trailing arguments:
/// - `"ls -la"` → `"ls"`
/// - `"/usr/bin/python3 script.py"` → `"python3"`
/// - `""` → `"*"`
fn shell_binary(cmd: &str) -> &str {
    let token = cmd.split_whitespace().next().unwrap_or("*");
    token.rsplit('/').next().unwrap_or(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── shell_policy_for ─────────────────────────────────────────────────────

    #[test]
    fn anonymous_policy_is_blocked() {
        assert_eq!(
            shell_policy_for(&PolicySet::anonymous()),
            ShellPolicy::Blocked
        );
    }

    #[test]
    fn free_policy_is_whitelisted() {
        assert_eq!(
            shell_policy_for(&PolicySet::free()),
            ShellPolicy::Whitelisted
        );
    }

    #[test]
    fn pro_policy_is_unrestricted() {
        assert_eq!(
            shell_policy_for(&PolicySet::pro()),
            ShellPolicy::Unrestricted
        );
    }

    #[test]
    fn enterprise_policy_is_unrestricted() {
        assert_eq!(
            shell_policy_for(&PolicySet::enterprise()),
            ShellPolicy::Unrestricted
        );
    }

    // ── validate_shell_command ───────────────────────────────────────────────

    #[test]
    fn blocked_policy_rejects_all_commands() {
        let policy = ShellPolicy::Blocked;
        assert!(validate_shell_command("ls", &policy).is_err());
        assert!(validate_shell_command("cat file.txt", &policy).is_err());
        assert!(validate_shell_command("rm -rf /", &policy).is_err());
    }

    #[test]
    fn unrestricted_policy_allows_all_commands() {
        let policy = ShellPolicy::Unrestricted;
        assert!(validate_shell_command("rm -rf /tmp", &policy).is_ok());
        assert!(validate_shell_command("sudo bash", &policy).is_ok());
        assert!(validate_shell_command("cat /etc/passwd", &policy).is_ok());
    }

    #[test]
    fn whitelisted_policy_allows_safe_commands() {
        let policy = ShellPolicy::Whitelisted;
        for cmd in FREE_TIER_ALLOWED_COMMANDS {
            assert!(
                validate_shell_command(cmd, &policy).is_ok(),
                "{cmd} must be allowed"
            );
        }
    }

    #[test]
    fn whitelisted_policy_rejects_unlisted_commands() {
        let policy = ShellPolicy::Whitelisted;
        assert!(validate_shell_command("rm -rf /", &policy).is_err());
        assert!(validate_shell_command("sudo bash", &policy).is_err());
        assert!(validate_shell_command("curl https://evil.com", &policy).is_err());
        assert!(validate_shell_command("chmod 777 /", &policy).is_err());
    }

    #[test]
    fn whitelisted_policy_allows_commands_with_args() {
        // Validation uses the binary name only, not the full command.
        let policy = ShellPolicy::Whitelisted;
        assert!(validate_shell_command("ls -la /tmp", &policy).is_ok());
        assert!(validate_shell_command("grep -r pattern /session", &policy).is_ok());
        assert!(validate_shell_command("python3 -c 'print(1)'", &policy).is_ok());
    }

    #[test]
    fn whitelisted_policy_strips_absolute_path() {
        let policy = ShellPolicy::Whitelisted;
        assert!(validate_shell_command("/bin/ls -la", &policy).is_ok());
        assert!(validate_shell_command("/usr/bin/grep -r foo /tmp", &policy).is_ok());
    }

    // ── shell_binary ─────────────────────────────────────────────────────────

    #[test]
    fn shell_binary_strips_args() {
        assert_eq!(shell_binary("ls -la"), "ls");
        assert_eq!(shell_binary("cat file.txt"), "cat");
    }

    #[test]
    fn shell_binary_strips_path() {
        assert_eq!(shell_binary("/usr/bin/python3 script.py"), "python3");
        assert_eq!(shell_binary("/bin/ls -la"), "ls");
    }

    #[test]
    fn shell_binary_empty_input() {
        assert_eq!(shell_binary(""), "*");
        assert_eq!(shell_binary("   "), "*");
    }

    #[test]
    fn shell_binary_bare_name() {
        assert_eq!(shell_binary("grep"), "grep");
    }

    // ── end-to-end: anonymous tier denies bash ────────────────────────────────

    #[test]
    fn anonymous_bash_call_is_denied() {
        let policy = shell_policy_for(&PolicySet::anonymous());
        let result = validate_shell_command("ls -la", &policy);
        assert!(result.is_err(), "anonymous bash must be denied");
        assert!(result.unwrap_err().contains("blocked"));
    }

    #[test]
    fn free_tier_ls_is_allowed() {
        let policy = shell_policy_for(&PolicySet::free());
        assert!(validate_shell_command("ls -la /session", &policy).is_ok());
    }

    #[test]
    fn free_tier_rm_is_denied() {
        let policy = shell_policy_for(&PolicySet::free());
        let result = validate_shell_command("rm -rf /tmp/x", &policy);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("allowlist"));
    }
}
