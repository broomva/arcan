//! Capability bitflags for sandboxed execution.
//!
//! `SandboxCapabilitySet` encodes what a sandbox is allowed to do as a compact
//! bitmask. The companion `from_policy` constructor bridges from the
//! string-pattern `PolicySet` used by the Agent OS policy engine.

use aios_protocol::PolicySet;
use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Compact bitmask of capabilities granted to a sandbox.
    ///
    /// Each bit maps to one permission axis. Providers advertise the set they
    /// support via `SandboxProvider::capabilities()`; the spec passed to
    /// `SandboxProvider::create()` carries the subset actually granted.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct SandboxCapabilitySet: u32 {
        /// Sandbox may read files from the filesystem.
        const FILESYSTEM_READ    = 0b0000_0001;
        /// Sandbox may write files to the filesystem.
        const FILESYSTEM_WRITE   = 0b0000_0010;
        /// Sandbox may initiate outbound network connections.
        const NETWORK_OUTBOUND   = 0b0000_0100;
        /// Sandbox may accept inbound network connections.
        const NETWORK_INBOUND    = 0b0000_1000;
        /// Sandbox state may be snapshotted and resumed.
        const PERSISTENCE        = 0b0001_0000;
        /// Sandbox may use a custom container/VM image.
        const CUSTOM_IMAGE       = 0b0010_0000;
        /// Sandbox may access GPU hardware.
        const GPU                = 0b0100_0000;
    }
}

impl Default for SandboxCapabilitySet {
    fn default() -> Self {
        Self::FILESYSTEM_READ
    }
}

impl SandboxCapabilitySet {
    /// Derive a capability set from a `PolicySet` by inspecting its
    /// `allow_capabilities` patterns.
    ///
    /// The mapping is intentionally conservative: a capability is enabled only
    /// when an explicit `allow_capabilities` pattern covers it. Patterns are
    /// matched by prefix:
    ///
    /// | Pattern prefix   | Enabled bit         |
    /// |------------------|---------------------|
    /// | `fs:read:`       | `FILESYSTEM_READ`   |
    /// | `fs:write:`      | `FILESYSTEM_WRITE`  |
    /// | `net:egress:`    | `NETWORK_OUTBOUND`  |
    /// | `net:ingress:`   | `NETWORK_INBOUND`   |
    /// | `sandbox:persist`| `PERSISTENCE`       |
    /// | `sandbox:image`  | `CUSTOM_IMAGE`      |
    /// | `sandbox:gpu`    | `GPU`               |
    pub fn from_policy(policy: &PolicySet) -> Self {
        let mut caps = Self::empty();
        for cap in &policy.allow_capabilities {
            let s = cap.as_str();
            if s.starts_with("fs:read:") {
                caps |= Self::FILESYSTEM_READ;
            }
            if s.starts_with("fs:write:") {
                caps |= Self::FILESYSTEM_WRITE;
            }
            if s.starts_with("net:egress:") {
                caps |= Self::NETWORK_OUTBOUND;
            }
            if s.starts_with("net:ingress:") {
                caps |= Self::NETWORK_INBOUND;
            }
            if s.starts_with("sandbox:persist") {
                caps |= Self::PERSISTENCE;
            }
            if s.starts_with("sandbox:image") {
                caps |= Self::CUSTOM_IMAGE;
            }
            if s.starts_with("sandbox:gpu") {
                caps |= Self::GPU;
            }
        }
        caps
    }
}

#[cfg(test)]
mod tests {
    use aios_protocol::{Capability, PolicySet};

    use super::*;

    fn make_policy(caps: &[&str]) -> PolicySet {
        PolicySet {
            allow_capabilities: caps.iter().map(|s| Capability::new(*s)).collect(),
            gate_capabilities: vec![],
            max_tool_runtime_secs: 30,
            max_events_per_turn: 10,
        }
    }

    #[test]
    fn empty_policy_yields_empty_caps() {
        let caps = SandboxCapabilitySet::from_policy(&make_policy(&[]));
        assert!(caps.is_empty());
    }

    #[test]
    fn fs_read_pattern_maps_correctly() {
        let caps = SandboxCapabilitySet::from_policy(&make_policy(&["fs:read:/session/**"]));
        assert!(caps.contains(SandboxCapabilitySet::FILESYSTEM_READ));
        assert!(!caps.contains(SandboxCapabilitySet::FILESYSTEM_WRITE));
    }

    #[test]
    fn net_egress_maps_to_outbound() {
        let caps = SandboxCapabilitySet::from_policy(&make_policy(&["net:egress:api.openai.com"]));
        assert!(caps.contains(SandboxCapabilitySet::NETWORK_OUTBOUND));
        assert!(!caps.contains(SandboxCapabilitySet::NETWORK_INBOUND));
    }

    #[test]
    fn multiple_capabilities_combine() {
        let caps = SandboxCapabilitySet::from_policy(&make_policy(&[
            "fs:read:/tmp/**",
            "fs:write:/tmp/**",
            "net:egress:*",
            "sandbox:persist",
        ]));
        assert!(caps.contains(
            SandboxCapabilitySet::FILESYSTEM_READ
                | SandboxCapabilitySet::FILESYSTEM_WRITE
                | SandboxCapabilitySet::NETWORK_OUTBOUND
                | SandboxCapabilitySet::PERSISTENCE
        ));
        assert!(!caps.contains(SandboxCapabilitySet::GPU));
    }

    #[test]
    fn serde_roundtrip() {
        let caps = SandboxCapabilitySet::FILESYSTEM_READ | SandboxCapabilitySet::NETWORK_OUTBOUND;
        let json = serde_json::to_string(&caps).unwrap();
        let back: SandboxCapabilitySet = serde_json::from_str(&json).unwrap();
        assert_eq!(caps, back);
    }

    #[test]
    fn default_is_filesystem_read() {
        assert_eq!(
            SandboxCapabilitySet::default(),
            SandboxCapabilitySet::FILESYSTEM_READ
        );
    }
}
