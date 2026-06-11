//! Loader for the on-disk anima identity produced by `life init`.
//!
//! `life init` (crates/cli/life-cli/src/init.rs, #1242) bootstraps
//! `.life/identity/` with two artifacts:
//!
//! - `soul.json` — schema-versioned soul document (flat `did` /
//!   `wallet` / `soul_hash` / `custody` fields plus the full nested
//!   `AgentSoul`), kept readable for shell inspection;
//! - `seed.local.bin` — the raw 32-byte master seed, written `0o600`.
//!
//! This module reads those artifacts back and reconstructs the
//! [`InProcessAnima`] custody handle so `arcan serve` can wire the
//! `AgentAttestationAdapter` (ergon-anima-adapter) with a STABLE agent
//! DID — the missing piece that kept workflow soul attestation on the
//! noop fallback (2026-06-10 harness Phase-2 closure note: a boot-time
//! `generate_dev()` key would mint a fresh DID every restart, making
//! the signed session boundaries unverifiable across runs).

use std::path::Path;
use std::sync::Arc;

use anima_identity::custody::AnimaCustody;
use anima_identity::{InProcessAnima, MasterSeed};
use anyhow::{Context, bail};

/// Read-side projection of `.life/identity/soul.json`.
///
/// Mirrors the writer schema in `life-cli`'s `SoulDocument` but keeps
/// only the fields the loader needs; unknown fields are ignored so
/// additive writer changes don't break boot.
#[derive(serde::Deserialize)]
struct SoulDocumentView {
    schema_version: u32,
    did: String,
    custody: CustodyView,
    soul: SoulView,
}

#[derive(serde::Deserialize)]
struct CustodyView {
    kind: String,
    #[serde(default = "default_seed_file")]
    seed_file: String,
}

fn default_seed_file() -> String {
    "seed.local.bin".to_owned()
}

#[derive(serde::Deserialize)]
struct SoulView {
    /// The agent's compressed auth public key (P-256 SEC1, 33 bytes),
    /// pinned at init time. The seed must still derive this exact key.
    root_public_key: Vec<u8>,
}

/// Load the custody handle from a `life init` identity directory.
///
/// Returns:
/// - `Ok(Some(custody))` — identity present, seed readable, and the
///   derived auth pubkey + DID match what `soul.json` pins;
/// - `Ok(None)` — the directory or `soul.json` is missing (identity
///   simply not initialized — callers fall back to noop attestation);
/// - `Err(..)` — identity present but unusable (corrupt soul document,
///   unsupported custody kind, missing/corrupt seed, pubkey/DID
///   drift). Configured-but-corrupt is a hard error, not a silent
///   fallback: noop-attesting under a corrupted identity would mask
///   key tampering.
pub fn load_custody_from_disk(
    identity_dir: &Path,
) -> anyhow::Result<Option<Arc<dyn AnimaCustody>>> {
    if !identity_dir.is_dir() {
        return Ok(None);
    }
    let soul_path = identity_dir.join("soul.json");
    if !soul_path.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&soul_path)
        .with_context(|| format!("failed to read {}", soul_path.display()))?;
    let doc: SoulDocumentView = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid soul document", soul_path.display()))?;

    if doc.schema_version != 1 {
        bail!(
            "{}: unsupported soul document schema_version {} (this arcan understands version 1)",
            soul_path.display(),
            doc.schema_version
        );
    }
    if doc.custody.kind != "in_process" {
        bail!(
            "{}: custody kind `{}` cannot be loaded by arcan serve yet — only `in_process` \
             (vault/tpm/webcrypto/hardware/soma custody wiring is a follow-up)",
            soul_path.display(),
            doc.custody.kind
        );
    }

    let seed_path = identity_dir.join(&doc.custody.seed_file);
    let seed_bytes = std::fs::read(&seed_path).with_context(|| {
        format!(
            "{} claims in_process custody but the seed at {} is unreadable — restore it from \
             backup, or remove soul.json and re-run `life init` (this regenerates the DID)",
            soul_path.display(),
            seed_path.display()
        )
    })?;
    let seed_arr: [u8; 32] = seed_bytes.try_into().map_err(|v: Vec<u8>| {
        anyhow::anyhow!(
            "{} is corrupt: {} bytes (expected 32)",
            seed_path.display(),
            v.len()
        )
    })?;

    let custody = InProcessAnima::from_seed_arc(MasterSeed::from_bytes(seed_arr))
        .context("failed to derive identity from master seed")?;

    // Sanity: the soul document pins the auth pubkey + DID at init
    // time; the seed must still derive the same key. A mismatch means
    // seed.local.bin and soul.json drifted apart (restored from
    // different backups, manual edits) — signing under a key the soul
    // doesn't vouch for would produce unverifiable attestations.
    if custody.auth_pubkey().as_slice() != doc.soul.root_public_key.as_slice() {
        bail!(
            "identity mismatch in {}: the seed derives an auth pubkey that does not match \
             soul.json's root_public_key — seed.local.bin and soul.json are out of sync",
            identity_dir.display()
        );
    }
    if custody.user_did() != doc.did {
        bail!(
            "identity mismatch in {}: the seed derives DID {} but soul.json pins {}",
            identity_dir.display(),
            custody.user_did(),
            doc.did
        );
    }

    Ok(Some(custody))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Deterministic test seed — mirrors the `deterministic_from_seed`
    /// pattern in `anima_identity::in_process`.
    const SEED: [u8; 32] = [7u8; 32];

    /// `unwrap_err` needs `Debug` on the Ok side, which
    /// `Arc<dyn AnimaCustody>` deliberately doesn't implement — match
    /// instead.
    fn load_err(dir: &Path) -> anyhow::Error {
        match load_custody_from_disk(dir) {
            Err(e) => e,
            Ok(_) => panic!("expected load_custody_from_disk to fail"),
        }
    }

    /// Write a well-formed identity dir (seed + soul.json) derived from
    /// `seed`, mirroring what `life init` produces. Returns the
    /// identity dir path.
    fn write_identity(root: &Path, seed: [u8; 32]) -> PathBuf {
        let identity_dir = root.join("identity");
        std::fs::create_dir_all(&identity_dir).unwrap();
        std::fs::write(identity_dir.join("seed.local.bin"), seed).unwrap();

        let custody = InProcessAnima::from_seed(MasterSeed::from_bytes(seed)).unwrap();
        let doc = soul_json(
            custody.user_did(),
            &custody.auth_pubkey(),
            "in_process",
            "seed.local.bin",
            1,
        );
        std::fs::write(identity_dir.join("soul.json"), doc).unwrap();
        identity_dir
    }

    fn soul_json(
        did: &str,
        pubkey: &[u8],
        custody_kind: &str,
        seed_file: &str,
        schema_version: u32,
    ) -> String {
        serde_json::json!({
            "schema_version": schema_version,
            "did": did,
            "wallet": { "address": "0x0000000000000000000000000000000000000000", "chain": "eip155:8453" },
            "soul_hash": "blake3:test",
            "custody": { "kind": custody_kind, "seed_file": seed_file },
            "soul": { "root_public_key": pubkey.to_vec() },
        })
        .to_string()
    }

    #[test]
    fn missing_dir_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_custody_from_disk(&tmp.path().join("does-not-exist")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn dir_without_soul_json_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_custody_from_disk(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn happy_path_loads_custody_with_pinned_did() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);

        let expected = InProcessAnima::from_seed(MasterSeed::from_bytes(SEED)).unwrap();
        let custody = load_custody_from_disk(&identity_dir)
            .unwrap()
            .expect("identity loads");
        assert_eq!(custody.user_did(), expected.user_did());
        assert_eq!(custody.auth_pubkey(), expected.auth_pubkey());

        // The handle signs — the whole point of loading it.
        let jws = custody.sign_jws(&serde_json::json!({"k": "v"})).unwrap();
        assert_eq!(jws.split('.').count(), 3);
    }

    /// The boot wiring contract end-to-end (sans daemon): a loaded
    /// custody handle feeds `AgentAttestationAdapter`, which attests a
    /// workflow session boundary under the on-disk identity's stable
    /// DID — exactly what `run_serve` wires via `with_soul_attester`.
    #[tokio::test]
    async fn loaded_custody_drives_agent_attestation_adapter() {
        use ergon_life_hooks::SoulAttester as _;

        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);

        let custody = load_custody_from_disk(&identity_dir)
            .unwrap()
            .expect("identity loads");
        let expected_did = custody.user_did().to_owned();

        let attester = ergon_anima_adapter::AgentAttestationAdapter::new(custody);
        assert_eq!(attester.agent_did(), expected_did, "stable DID from disk");

        let sid = ergon::SessionId::from_string("sid-boot-wiring");
        attester
            .sign_session_start(&sid, "greeter")
            .await
            .expect("session start attested");
        attester
            .sign_session_end(&sid, "greeter", true)
            .await
            .expect("session end attested");
    }

    #[test]
    fn corrupt_soul_json_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        std::fs::write(identity_dir.join("soul.json"), "{not json").unwrap();

        let err = load_err(&identity_dir);
        assert!(
            format!("{err:#}").contains("not a valid soul document"),
            "got: {err:#}"
        );
    }

    #[test]
    fn unsupported_schema_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        let custody = InProcessAnima::from_seed(MasterSeed::from_bytes(SEED)).unwrap();
        let doc = soul_json(
            custody.user_did(),
            &custody.auth_pubkey(),
            "in_process",
            "seed.local.bin",
            2,
        );
        std::fs::write(identity_dir.join("soul.json"), doc).unwrap();

        let err = load_err(&identity_dir);
        assert!(
            format!("{err:#}").contains("schema_version 2"),
            "got: {err:#}"
        );
    }

    #[test]
    fn non_in_process_custody_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        let custody = InProcessAnima::from_seed(MasterSeed::from_bytes(SEED)).unwrap();
        let doc = soul_json(
            custody.user_did(),
            &custody.auth_pubkey(),
            "vault",
            "seed.local.bin",
            1,
        );
        std::fs::write(identity_dir.join("soul.json"), doc).unwrap();

        let err = load_err(&identity_dir);
        assert!(format!("{err:#}").contains("`vault`"), "got: {err:#}");
    }

    #[test]
    fn missing_seed_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        std::fs::remove_file(identity_dir.join("seed.local.bin")).unwrap();

        let err = load_err(&identity_dir);
        assert!(format!("{err:#}").contains("unreadable"), "got: {err:#}");
    }

    #[test]
    fn corrupt_seed_wrong_length_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        std::fs::write(identity_dir.join("seed.local.bin"), [7u8; 16]).unwrap();

        let err = load_err(&identity_dir);
        assert!(
            format!("{err:#}").contains("16 bytes (expected 32)"),
            "got: {err:#}"
        );
    }

    #[test]
    fn pubkey_mismatch_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let identity_dir = write_identity(tmp.path(), SEED);
        // Swap the seed for a different one: soul.json still pins the
        // original pubkey/DID, so derivation no longer matches.
        std::fs::write(identity_dir.join("seed.local.bin"), [9u8; 32]).unwrap();

        let err = load_err(&identity_dir);
        assert!(format!("{err:#}").contains("out of sync"), "got: {err:#}");
    }
}
