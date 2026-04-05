//! Pre-approved MCP server registry with tier gates (BRO-226).
//!
//! Defines which MCP servers are accessible at each capability tier and
//! provides helpers used by `run_session` to gate skill-declared MCP
//! connections before the session kernel spawns them.
//!
//! ## Access model
//!
//! | Tier       | MCP access                                    |
//! |------------|-----------------------------------------------|
//! | Anonymous  | None — all MCP blocked                        |
//! | Free       | Pre-approved read-only servers                |
//! | Pro        | All pre-approved servers                      |
//! | Enterprise | Pre-approved + any custom tenant-registered   |
//!
//! Custom (not listed here) servers require `Enterprise` tier.

use crate::auth::Tier;

// ─── Registry entry ────────────────────────────────────────────────────────

/// A pre-approved MCP server entry with its minimum required tier.
#[derive(Debug, Clone)]
pub struct McpServerSpec {
    /// Server name as declared in SKILL.md `mcp_servers[].name`.
    pub name: &'static str,
    /// Human-readable description.
    pub description: &'static str,
    /// Minimum tier required to connect to this server.
    pub min_tier: Tier,
    /// Serialisable tier name for API responses.
    pub min_tier_name: &'static str,
}

// ─── Static registry ───────────────────────────────────────────────────────

/// Pre-approved MCP servers available on the shared Arcan instance.
///
/// Enterprise tenants can register additional private servers via the
/// `POST /tenant/mcp-servers` API (BRO-226).
pub static APPROVED_MCP_SERVERS: &[McpServerSpec] = &[
    // ── Free-tier (read-only, no auth required) ──
    McpServerSpec {
        name: "brave-search",
        description: "Brave Search read-only search API",
        min_tier: Tier::Free,
        min_tier_name: "free",
    },
    McpServerSpec {
        name: "perplexity",
        description: "Perplexity AI semantic search",
        min_tier: Tier::Free,
        min_tier_name: "free",
    },
    McpServerSpec {
        name: "context7",
        description: "Context7 library documentation retrieval",
        min_tier: Tier::Free,
        min_tier_name: "free",
    },
    McpServerSpec {
        name: "linear",
        description: "Linear issue tracker (read-only for free tier)",
        min_tier: Tier::Free,
        min_tier_name: "free",
    },
    McpServerSpec {
        name: "github",
        description: "GitHub repository access (read-only for free tier)",
        min_tier: Tier::Free,
        min_tier_name: "free",
    },
    // ── Pro-tier and above ──
    McpServerSpec {
        name: "neon",
        description: "Neon Postgres query (user's own database)",
        min_tier: Tier::Pro,
        min_tier_name: "pro",
    },
    McpServerSpec {
        name: "railway",
        description: "Railway deployment management (user's own account)",
        min_tier: Tier::Pro,
        min_tier_name: "pro",
    },
    McpServerSpec {
        name: "vercel",
        description: "Vercel deployment management (user's own account)",
        min_tier: Tier::Pro,
        min_tier_name: "pro",
    },
    McpServerSpec {
        name: "slack",
        description: "Slack workspace integration",
        min_tier: Tier::Pro,
        min_tier_name: "pro",
    },
];

// ─── Tier helpers ─────────────────────────────────────────────────────────

fn tier_rank(tier: &Tier) -> u8 {
    match tier {
        Tier::Anonymous => 0,
        Tier::Free => 1,
        Tier::Pro => 2,
        Tier::Enterprise => 3,
    }
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Returns `true` if the named MCP server is accessible for the given tier.
///
/// Rules:
/// - `Anonymous` → always `false`
/// - `Enterprise` → `true` for any server (including custom/unlisted)
/// - `Free` / `Pro` → `true` only if the server is in `APPROVED_MCP_SERVERS`
///   and the tier rank meets the server's `min_tier`
pub fn is_mcp_server_allowed(server_name: &str, tier: &Tier) -> bool {
    match tier {
        Tier::Anonymous => false,
        Tier::Enterprise => true,
        Tier::Free | Tier::Pro => APPROVED_MCP_SERVERS
            .iter()
            .any(|spec| spec.name == server_name && tier_rank(tier) >= tier_rank(&spec.min_tier)),
    }
}

/// Returns the slice of pre-approved servers accessible at `tier`.
pub fn allowed_servers_for_tier(tier: &Tier) -> Vec<&'static McpServerSpec> {
    match tier {
        Tier::Anonymous => vec![],
        Tier::Enterprise => APPROVED_MCP_SERVERS.iter().collect(),
        Tier::Free | Tier::Pro => APPROVED_MCP_SERVERS
            .iter()
            .filter(|spec| tier_rank(tier) >= tier_rank(&spec.min_tier))
            .collect(),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_blocks_all_mcp_servers() {
        for spec in APPROVED_MCP_SERVERS {
            assert!(
                !is_mcp_server_allowed(spec.name, &Tier::Anonymous),
                "{} should be blocked for anonymous",
                spec.name
            );
        }
    }

    #[test]
    fn enterprise_allows_any_server() {
        assert!(is_mcp_server_allowed("brave-search", &Tier::Enterprise));
        assert!(is_mcp_server_allowed(
            "custom-internal-crm",
            &Tier::Enterprise
        ));
        assert!(is_mcp_server_allowed("anything-goes", &Tier::Enterprise));
    }

    #[test]
    fn free_tier_allows_read_only_approved_servers() {
        assert!(is_mcp_server_allowed("brave-search", &Tier::Free));
        assert!(is_mcp_server_allowed("context7", &Tier::Free));
        assert!(is_mcp_server_allowed("linear", &Tier::Free));
        assert!(is_mcp_server_allowed("github", &Tier::Free));
    }

    #[test]
    fn free_tier_blocks_pro_servers() {
        assert!(!is_mcp_server_allowed("neon", &Tier::Free));
        assert!(!is_mcp_server_allowed("railway", &Tier::Free));
        assert!(!is_mcp_server_allowed("vercel", &Tier::Free));
    }

    #[test]
    fn free_tier_blocks_unapproved_custom_servers() {
        assert!(!is_mcp_server_allowed("internal-crm", &Tier::Free));
        assert!(!is_mcp_server_allowed("my-custom-server", &Tier::Free));
    }

    #[test]
    fn pro_tier_allows_all_approved_servers() {
        for spec in APPROVED_MCP_SERVERS {
            assert!(
                is_mcp_server_allowed(spec.name, &Tier::Pro),
                "{} should be allowed for pro",
                spec.name
            );
        }
    }

    #[test]
    fn allowed_servers_for_anonymous_is_empty() {
        assert!(allowed_servers_for_tier(&Tier::Anonymous).is_empty());
    }

    #[test]
    fn allowed_servers_for_free_is_subset_of_pro() {
        let free = allowed_servers_for_tier(&Tier::Free);
        let pro = allowed_servers_for_tier(&Tier::Pro);
        assert!(free.len() < pro.len());
        // Every free server must also be in pro
        for s in &free {
            assert!(pro.iter().any(|p| p.name == s.name));
        }
    }

    #[test]
    fn allowed_servers_for_enterprise_includes_all() {
        let enterprise = allowed_servers_for_tier(&Tier::Enterprise);
        assert_eq!(enterprise.len(), APPROVED_MCP_SERVERS.len());
    }
}
